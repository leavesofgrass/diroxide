//! The type model of alpha.7, in diroxide's words (R28 4.2 and 4.3; the
//! corrected plan 5.6): file types form a tree, and a file is tested in
//! three stages, in one order:
//!
//! 1. **Signature** ([`TypeStage::Signature`], accuracy 3): bytes at fixed
//!    positions in the first [`HEAD_BYTES`], `??` for any byte. A
//!    signature of fewer than [`MIN_FIXED_BYTES`] fixed bytes counts only
//!    when the type's condition or name passes too, since two bytes such
//!    as gzip's `1F 8B` start plain text files by chance.
//! 2. **Condition** ([`TypeStage::Condition`], accuracy 2): one filter
//!    expression over the entry and its first bytes (`content:re:"^#!"`).
//!    `content` reads the first bytes as text, invalid bytes replaced, so
//!    a condition can look for text markers (`#!`, `<?xml`, `webm`) but
//!    never for bytes that are not text; those belong in a signature.
//! 3. **Name** ([`TypeStage::Name`], accuracy 1): an extension or a name
//!    pattern.
//!
//! Per type, the first stage that passes is its hit. The walk starts at
//! the types with no parent and opens a type's children only when the
//! type was hit, has no tests at all (a heading over a family), or could
//! not be decided because its contents were not read; so a child refines
//! its parent (DOCX under zip, a camera's raw file under TIFF), and a
//! file named `report.docx` reads as a Word document at once, before its
//! first bytes are read.
//!
//! Among the types hit, a child always beats its parent (DOCX, a child of
//! zip by its name, beats zip's signature), and otherwise the higher
//! priority wins, then the higher accuracy, then the earlier type.
//! Priorities are never added. A folder is never tested; the content
//! stages are skipped for links, special files, empty files, online-only
//! files and folders the owner marked "don't check", where the condition
//! (without contents) and the name still run.

use std::fmt;

use diroxide_core::filter::{ContentQuery, Filter, FilterError, Lookup, Truth, Unknown};
use diroxide_core::pattern::Glob;
use diroxide_core::sort::NameCase;
use diroxide_core::{Clock, Entry, EntryKind, Pattern, PatternError};

use crate::{FileType, HEAD_BYTES};

/// The fewest fixed bytes a signature needs to count on its own; a
/// shorter one needs the type's condition or name as well (R28 4.6).
pub const MIN_FIXED_BYTES: usize = 3;

/// How a type was found, the order the stages run in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TypeStage {
    /// By a signature in the first bytes.
    Signature,
    /// By the extended condition.
    Condition,
    /// By the name: an extension or a pattern.
    Name,
}

impl TypeStage {
    /// The stages, in the order they run.
    pub const ORDER: [TypeStage; 3] = [TypeStage::Signature, TypeStage::Condition, TypeStage::Name];

    /// How sure a hit is: 3 for a signature, 2 for a condition, 1 for a
    /// name; the tie-break after the priority.
    pub const fn accuracy(self) -> u8 {
        match self {
            TypeStage::Signature => 3,
            TypeStage::Condition => 2,
            TypeStage::Name => 1,
        }
    }

    /// The stage in words, for the details' "Read by" row: "by its
    /// contents", "by its name".
    pub const fn words(self) -> &'static str {
        match self {
            TypeStage::Signature => "by its contents",
            TypeStage::Condition => "by a condition",
            TypeStage::Name => "by its name",
        }
    }
}

/// Why a signature's text could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureError(String);

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SignatureError {}

/// Bytes at a position in a file's first bytes, each a byte or any byte.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    /// Where the bytes start.
    pub at: usize,
    /// The bytes; `None` is any byte (`??`).
    pub bytes: Vec<Option<u8>>,
}

impl Signature {
    /// Reads a signature written as hexadecimal pairs and `??`, spaces
    /// between them allowed (`"FF D8 FF"`, `"52 49 46 46 ?? ?? ?? ?? 57
    /// 45 42 50"`). It must hold at least one fixed byte and end within
    /// the first [`HEAD_BYTES`].
    pub fn parse(at: usize, hex: &str) -> Result<Self, SignatureError> {
        let compact: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.is_empty() || !compact.len().is_multiple_of(2) || !compact.is_ascii() {
            return Err(SignatureError(format!(
                "\"{hex}\" is not whole pairs of hexadecimal digits"
            )));
        }
        let mut bytes = Vec::with_capacity(compact.len() / 2);
        for pair in compact.as_bytes().chunks(2) {
            let pair = std::str::from_utf8(pair).unwrap_or("");
            if pair == "??" {
                bytes.push(None);
            } else {
                let byte = u8::from_str_radix(pair, 16).map_err(|_| {
                    SignatureError(format!("\"{pair}\" in \"{hex}\" is not a hexadecimal byte"))
                })?;
                bytes.push(Some(byte));
            }
        }
        if bytes.iter().all(Option::is_none) {
            return Err(SignatureError(format!(
                "\"{hex}\" has no fixed byte, so every file would match"
            )));
        }
        if at + bytes.len() > HEAD_BYTES {
            return Err(SignatureError(format!(
                "\"{hex}\" at {at} ends past the first {HEAD_BYTES} bytes"
            )));
        }
        Ok(Self { at, bytes })
    }

    /// A signature of fixed bytes.
    pub fn fixed(at: usize, bytes: &[u8]) -> Self {
        Self {
            at,
            bytes: bytes.iter().copied().map(Some).collect(),
        }
    }

    /// How many of its bytes are fixed.
    pub fn fixed_count(&self) -> usize {
        self.bytes.iter().filter(|b| b.is_some()).count()
    }

    /// Whether it counts on its own, without the type's condition or name:
    /// at least [`MIN_FIXED_BYTES`] fixed bytes.
    pub fn stands_alone(&self) -> bool {
        self.fixed_count() >= MIN_FIXED_BYTES
    }

    /// Whether a file's first bytes match: the file reaches the last
    /// fixed byte, and every fixed byte is there.
    pub fn matches(&self, head: &[u8]) -> bool {
        self.bytes
            .iter()
            .enumerate()
            .all(|(i, wanted)| match wanted {
                None => true,
                Some(byte) => head.get(self.at + i) == Some(byte),
            })
    }
}

/// A type's extended condition: one expression of the shared filter
/// language, tested on the entry and its first bytes.
///
/// `content` means the first bytes, read as text with invalid bytes
/// replaced (spike F3): `content:re:"^#!"` finds a script's first line,
/// `content:"webm"` finds the word inside a Matroska header. A byte that
/// is not text (`FF D8`) cannot be written here; a signature with `??`
/// says it instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    filter: Filter,
}

impl Condition {
    /// Reads a condition; a mistake is said as the filter language says
    /// it.
    pub fn parse(text: &str) -> Result<Self, FilterError> {
        Filter::parse(text).map(|filter| Self { filter })
    }

    /// The condition as written.
    pub fn text(&self) -> &str {
        self.filter.text()
    }

    /// Whether the entry passes, `content` read over its first bytes as
    /// text; a content test with no bytes read is unknown.
    pub fn test(&self, entry: &Entry, head: Option<&[u8]>, clock: &dyn Clock) -> Truth {
        let lookup = HeadLookup(head);
        self.filter.evaluator(clock).test(entry, &lookup)
    }
}

/// A lookup whose content is a file's first bytes.
struct HeadLookup<'a>(Option<&'a [u8]>);

impl Lookup for HeadLookup<'_> {
    fn content(&self, _entry: &Entry, query: &ContentQuery) -> Truth {
        match self.0 {
            Some(head) => Truth::of(query.is_match(&String::from_utf8_lossy(head))),
            None => Truth::Unknown(Unknown::ContentNotRead),
        }
    }
}

/// A name pattern of a type or a group: a glob over the whole name
/// (`*.tar.gz`), several separated by commas (`*.tar.gz, *.tgz`), or
/// `re:` and one regular expression over the name
/// (`re:^Screenshot_\d{8}_\d{6}\.png$`). Case is ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePattern {
    text: String,
    form: PatternForm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PatternForm {
    Globs(Vec<Glob>),
    Regex(Pattern),
}

impl NamePattern {
    /// Reads a pattern; a mistake is said in the pattern language's words
    /// ("Pattern needs care: unclosed bracket").
    pub fn parse(text: &str) -> Result<Self, PatternError> {
        let trimmed = text.trim();
        let form = if trimmed.starts_with("re:") {
            PatternForm::Regex(Pattern::parse_with(trimmed, NameCase::Ignore)?)
        } else {
            let globs = trimmed
                .split(',')
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(|g| Glob::new(g, NameCase::Ignore))
                .collect::<Result<Vec<_>, _>>()?;
            if globs.is_empty() {
                return Err(PatternError::Empty);
            }
            PatternForm::Globs(globs)
        };
        Ok(Self {
            text: text.to_owned(),
            form,
        })
    }

    /// The pattern as written.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether a name matches.
    pub fn is_match(&self, name: &str) -> bool {
        match &self.form {
            PatternForm::Globs(globs) => globs.iter().any(|g| g.is_match(name)),
            PatternForm::Regex(pattern) => pattern.matches(name, false),
        }
    }
}

/// What detection has to go on for one entry.
#[derive(Clone, Copy, Debug)]
pub struct Probe<'a> {
    /// The entry, with its name and kind.
    pub entry: &'a Entry,
    /// Its first bytes, when read; `None` while they are not. Only the
    /// first [`HEAD_BYTES`] are looked at.
    pub head: Option<&'a [u8]>,
    /// The content stages are skipped: a link, a special or empty file,
    /// an online-only file, a folder in `check.skip`.
    pub skip_content: bool,
}

impl<'a> Probe<'a> {
    /// A probe for an entry and its first bytes, the content stages
    /// skipped where the entry's own facts say so ([`skips_content`]).
    pub fn new(entry: &'a Entry, head: Option<&'a [u8]>) -> Self {
        Self {
            entry,
            head,
            skip_content: skips_content(entry),
        }
    }

    /// The same probe, the content stages skipped also when `skip` is
    /// true: the entry's folder is in `check.skip`.
    pub fn skipping(mut self, skip: bool) -> Self {
        self.skip_content |= skip;
        self
    }

    /// The first bytes the content stages may read: none while skipped
    /// or unread, else at most [`HEAD_BYTES`].
    fn content(&self) -> Option<&'a [u8]> {
        if self.skip_content {
            return None;
        }
        self.head.map(|head| &head[..head.len().min(HEAD_BYTES)])
    }
}

/// Whether an entry's own facts keep its contents from being read for its
/// type: anything but a regular file or an archive (a link, a special
/// file, a plugin's item), an empty file, and an online-only file, whose
/// first bytes would bring the whole file down. A folder is never tested
/// at all. The background check asks this before it reads anything.
pub fn skips_content(entry: &Entry) -> bool {
    !matches!(entry.kind, EntryKind::File | EntryKind::Archive)
        || entry.size == Some(0)
        || diroxide_vfs::facts::is_online_only(entry)
}

/// What a row says when no type is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Special {
    /// A folder, never tested.
    Folder,
    /// The name says nothing and the first bytes are not read yet.
    Pending,
    /// No type takes it.
    Unknown,
}

impl Special {
    /// The words a row says: "folder", "type pending", "unknown".
    pub const fn words(self) -> &'static str {
        match self {
            Special::Folder => "folder",
            Special::Pending => "type pending",
            Special::Unknown => "unknown",
        }
    }
}

/// A type found, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The type's id.
    pub type_id: String,
    /// The stage that found it.
    pub stage: TypeStage,
    /// The type's place in the tree's order, the same as in the registry
    /// that made the tree; [`Entry::type_index`] holds it plus one.
    pub index: usize,
}

/// Why a set of types does not make a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// A type names a parent no type has.
    UnknownParent {
        /// The type.
        type_id: String,
        /// The parent it names.
        parent: String,
    },
    /// A type is its own ancestor.
    Cycle {
        /// A type on the cycle.
        type_id: String,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::UnknownParent { type_id, parent } => {
                write!(
                    f,
                    "Type {type_id} names the parent {parent}, which no type has."
                )
            }
            TreeError::Cycle { type_id } => {
                write!(f, "Type {type_id} is its own parent, through its parents.")
            }
        }
    }
}

impl std::error::Error for TreeError {}

/// How one type answered a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// It was hit, at this stage.
    Hit(TypeStage),
    /// Not hit, but its children are tested: it has no tests, or its
    /// contents were not read so it could not be decided.
    Open,
    /// Not hit; its children are not tested.
    Miss,
}

/// The types as a tree: each with its parent and children, the name
/// patterns compiled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeTree {
    types: Vec<FileType>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    patterns: Vec<Vec<NamePattern>>,
}

impl TypeTree {
    /// The tree of these types, in their order: every parent named must be
    /// a type, and no type may be its own ancestor. A name pattern that
    /// does not read is left out (the registry says why when it loads).
    pub fn new(types: Vec<FileType>) -> Result<Self, TreeError> {
        let mut parents = Vec::with_capacity(types.len());
        for t in &types {
            let parent = match &t.parent {
                None => None,
                Some(parent) => {
                    Some(types.iter().position(|p| &p.id == parent).ok_or_else(|| {
                        TreeError::UnknownParent {
                            type_id: t.id.clone(),
                            parent: parent.clone(),
                        }
                    })?)
                }
            };
            parents.push(parent);
        }
        for start in 0..types.len() {
            let mut at = parents[start];
            let mut steps = 0;
            while let Some(i) = at {
                steps += 1;
                if i == start || steps > types.len() {
                    return Err(TreeError::Cycle {
                        type_id: types[start].id.clone(),
                    });
                }
                at = parents[i];
            }
        }
        let mut children = vec![Vec::new(); types.len()];
        let mut roots = Vec::new();
        for (i, parent) in parents.iter().enumerate() {
            match parent {
                Some(p) => children[*p].push(i),
                None => roots.push(i),
            }
        }
        let patterns = types
            .iter()
            .map(|t| {
                t.patterns
                    .iter()
                    .filter_map(|p| NamePattern::parse(p).ok())
                    .collect()
            })
            .collect();
        Ok(Self {
            types,
            parents,
            children,
            roots,
            patterns,
        })
    }

    /// The types, in order.
    pub fn types(&self) -> &[FileType] {
        &self.types
    }

    /// A type's parent, by place in the order.
    pub fn parent_of(&self, index: usize) -> Option<usize> {
        self.parents.get(index).copied().flatten()
    }

    /// A type's children, by place in the order.
    pub fn children_of(&self, index: usize) -> &[usize] {
        self.children.get(index).map_or(&[], Vec::as_slice)
    }

    /// Whether `ancestor` is a parent of `index`, or a parent's parent.
    pub fn is_ancestor(&self, ancestor: usize, index: usize) -> bool {
        let mut at = self.parents[index];
        while let Some(i) = at {
            if i == ancestor {
                return true;
            }
            at = self.parents[i];
        }
        false
    }

    /// Whether the name stage takes the entry: its extension, or a name
    /// pattern. `lower` is the name in lowercase.
    fn name_hit(&self, i: usize, name: &str, lower: &str) -> bool {
        let t = &self.types[i];
        let by_extension = lower
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && t.extensions.iter().any(|e| e == ext));
        by_extension || self.patterns[i].iter().any(|p| p.is_match(name))
    }

    /// How a type answers: the first stage that passes, in the stages'
    /// order, or whether its children are still worth testing.
    fn outcome(&self, i: usize, probe: &Probe<'_>, lower: &str, clock: &dyn Clock) -> Outcome {
        let t = &self.types[i];
        let content = probe.content();
        let condition = t.when.as_ref().map(|w| w.test(probe.entry, content, clock));
        let condition_yes = condition.is_some_and(Truth::is_yes);
        let name = self.name_hit(i, &probe.entry.name, lower);
        if let Some(head) = content
            && t.signatures
                .iter()
                .any(|s| s.matches(head) && (s.stands_alone() || condition_yes || name))
        {
            return Outcome::Hit(TypeStage::Signature);
        }
        if condition_yes {
            return Outcome::Hit(TypeStage::Condition);
        }
        if name {
            return Outcome::Hit(TypeStage::Name);
        }
        let no_tests = t.signatures.is_empty()
            && t.when.is_none()
            && t.extensions.is_empty()
            && self.patterns[i].is_empty();
        let undecided = (!t.signatures.is_empty() && content.is_none())
            || matches!(condition, Some(Truth::Unknown(_)));
        if no_tests || undecided {
            Outcome::Open
        } else {
            Outcome::Miss
        }
    }

    /// Walks the tree from `i`, collecting the hits.
    fn walk(
        &self,
        i: usize,
        probe: &Probe<'_>,
        lower: &str,
        clock: &dyn Clock,
        hits: &mut Vec<(usize, TypeStage)>,
    ) {
        let open = match self.outcome(i, probe, lower, clock) {
            Outcome::Hit(stage) => {
                hits.push((i, stage));
                true
            }
            Outcome::Open => true,
            Outcome::Miss => false,
        };
        if open {
            for &child in &self.children[i] {
                self.walk(child, probe, lower, clock, hits);
            }
        }
    }

    /// The entry's type. The walk starts at the types with no parent and
    /// opens a type's children only under a hit, a type with no tests, or
    /// a type its unread contents left undecided; among the hits a child
    /// beats its ancestors, then the higher priority, the higher accuracy
    /// and the earlier type win. A folder is never tested, nor a plugin's
    /// item; with nothing found, [`Special::Pending`] while the first
    /// bytes are still to be read, else [`Special::Unknown`].
    pub fn detect(&self, probe: &Probe<'_>, clock: &dyn Clock) -> Result<Found, Special> {
        if probe.entry.is_folder_like() {
            return Err(Special::Folder);
        }
        if probe.entry.kind == EntryKind::Item {
            return Err(Special::Unknown);
        }
        let lower = probe.entry.name.to_lowercase();
        let mut hits = Vec::new();
        for &root in &self.roots {
            self.walk(root, probe, &lower, clock, &mut hits);
        }
        let best = hits
            .iter()
            .filter(|(i, _)| !hits.iter().any(|(j, _)| self.is_ancestor(*i, *j)))
            .max_by(|(a, sa), (b, sb)| {
                (self.types[*a].priority, sa.accuracy())
                    .cmp(&(self.types[*b].priority, sb.accuracy()))
                    .then(b.cmp(a))
            });
        match best {
            Some((i, stage)) => Ok(Found {
                type_id: self.types[*i].id.clone(),
                stage: *stage,
                index: *i,
            }),
            None if probe.head.is_none() && !probe.skip_content => Err(Special::Pending),
            None => Err(Special::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use diroxide_core::{EntryKind, FixedClock, Timestamp, UtcOffset};

    use super::*;

    fn clock() -> FixedClock {
        let now = Timestamp::from_civil(2026, 11, 5, 22, 0, 0, UtcOffset::UTC).unwrap();
        FixedClock::new(now, UtcOffset::UTC)
    }

    fn kind(id: &str, extensions: &[&str], priority: i32) -> FileType {
        let mut t = FileType::new(id, id, id, extensions, &[]);
        t.priority = priority;
        t
    }

    fn found(tree: &TypeTree, name: &str, head: Option<&[u8]>) -> Result<Found, Special> {
        let entry = Entry::new(name, EntryKind::File);
        let probe = Probe {
            entry: &entry,
            head,
            skip_content: false,
        };
        tree.detect(&probe, &clock())
    }

    fn id(result: Result<Found, Special>) -> String {
        match result {
            Ok(found) => found.type_id,
            Err(special) => special.words().to_owned(),
        }
    }

    #[test]
    fn signatures_read_hex_with_any_bytes_and_match_the_head() {
        let webp = Signature::parse(0, "52 49 46 46 ?? ?? ?? ?? 57 45 42 50").unwrap();
        assert_eq!(webp.fixed_count(), 8);
        assert!(webp.matches(b"RIFF\x10\x00\x00\x00WEBPVP8 "));
        assert!(!webp.matches(b"RIFF\x10\x00\x00\x00WAVE"));
        assert!(
            !webp.matches(b"RIFF"),
            "the file must reach the last fixed byte"
        );
        for (hex, why) in [
            ("F", "is not whole pairs"),
            ("ZZ", "is not a hexadecimal byte"),
            ("?? ??", "has no fixed byte"),
            ("é1", "is not whole pairs"),
        ] {
            let error = Signature::parse(0, hex).unwrap_err().to_string();
            assert!(error.contains(why), "{hex}: {error}");
        }
        assert!(Signature::parse(62, "FF D8 FF").is_err(), "past 64 bytes");
        assert!(Signature::parse(61, "FF D8 FF").is_ok(), "ends at 64");
    }

    /// The container pattern: DOCX is a child of zip by its name, and a
    /// child beats its parent's signature; otherwise the higher number
    /// wins; a folder is never tested; a name that says nothing waits for
    /// its first bytes.
    #[test]
    fn a_child_beats_its_parent_and_the_higher_number_wins() {
        let mut zip = kind("zip", &["zip"], 0);
        zip.signatures = vec![Signature::fixed(0, b"PK\x03\x04")];
        let mut docx = kind("docx", &["docx"], 0);
        docx.parent = Some("zip".to_owned());
        let low = kind("text", &["txt"], 0);
        let high = kind("notes", &["txt"], 5);
        let tree = TypeTree::new(vec![zip, docx, low, high]).unwrap();
        let head: &[u8] = b"PK\x03\x04rest";
        let hit = found(&tree, "report.docx", Some(head)).unwrap();
        assert_eq!((hit.type_id.as_str(), hit.stage), ("docx", TypeStage::Name));
        assert_eq!(hit.index, 1);
        let hit = found(&tree, "bundle.bin", Some(head)).unwrap();
        assert_eq!(
            (hit.type_id.as_str(), hit.stage),
            ("zip", TypeStage::Signature)
        );
        assert_eq!(found(&tree, "a.txt", None).unwrap().type_id, "notes");
        assert_eq!(found(&tree, "data", None), Err(Special::Pending));
        assert_eq!(found(&tree, "data", Some(b"plain")), Err(Special::Unknown));
        let folder = Entry::new("docs", EntryKind::Folder);
        let probe = Probe {
            entry: &folder,
            head: None,
            skip_content: false,
        };
        assert_eq!(tree.detect(&probe, &clock()), Err(Special::Folder));
        assert_eq!(Special::Pending.words(), "type pending");
    }

    /// The walk: a child is tested only under a hit, a heading with no
    /// tests, or a parent its unread contents left undecided. A file
    /// named `.docx` reads as DOCX before its bytes are read, and after,
    /// when they are a zip; when they are not, zip's child is not
    /// tested, and the bytes decide.
    #[test]
    fn children_open_under_a_hit_a_heading_or_unread_contents() {
        let mut zip = kind("zip", &["zip"], 0);
        zip.signatures = vec![Signature::fixed(0, b"PK\x03\x04")];
        let mut docx = kind("docx", &["docx"], 0);
        docx.parent = Some("zip".to_owned());
        let mut pdf = kind("pdf", &["pdf"], 0);
        pdf.signatures = vec![Signature::fixed(0, b"%PDF-")];
        let pictures = kind("pictures", &[], 0);
        let mut png = kind("png", &["png"], 0);
        png.parent = Some("pictures".to_owned());
        let tree = TypeTree::new(vec![zip, docx, pdf, pictures, png]).unwrap();
        assert_eq!(id(found(&tree, "report.docx", None)), "docx");
        assert_eq!(id(found(&tree, "report.docx", Some(b"PK\x03\x04"))), "docx");
        assert_eq!(id(found(&tree, "report.docx", Some(b"%PDF-1.7"))), "pdf");
        assert_eq!(id(found(&tree, "report.docx", Some(b"plain"))), "unknown");
        assert_eq!(id(found(&tree, "photo.png", Some(b"plain"))), "png");
        let empty = Entry::new("report.docx", EntryKind::File).with_size(0);
        assert_eq!(id(tree.detect(&Probe::new(&empty, None), &clock())), "docx");
    }

    /// A signature of fewer than three fixed bytes needs the name or the
    /// condition too, so gzip's `1F 8B` does not take a text file.
    #[test]
    fn a_short_signature_needs_a_second_test() {
        let mut gzip = kind("gzip", &["gz"], 0);
        gzip.signatures = vec![Signature::fixed(0, b"\x1F\x8B")];
        let tree = TypeTree::new(vec![gzip]).unwrap();
        let head: &[u8] = b"\x1F\x8B\x08\x00";
        assert_eq!(id(found(&tree, "notes", Some(head))), "unknown");
        let hit = found(&tree, "notes.gz", Some(head)).unwrap();
        assert_eq!(hit.stage, TypeStage::Signature);
        assert!(!Signature::fixed(0, b"\x1F\x8B").stands_alone());
        assert!(Signature::fixed(0, b"BZh").stands_alone());
    }

    /// Spike F3: `content` in a type's condition is the first bytes read
    /// as text. A script's `#!` line and a word in a binary header are
    /// found; a byte that is not text is not, so it belongs in a
    /// signature. Skipped content leaves the name stage alone, and only
    /// the first 64 bytes are read.
    #[test]
    fn a_condition_reads_the_first_bytes_as_text() {
        let mut script = kind("shell-script", &[], 5);
        script.when = Some(Condition::parse(r#"content:re:"^#!""#).unwrap());
        let tree = TypeTree::new(vec![script, kind("text", &["txt"], 0)]).unwrap();
        let hit = found(&tree, "run", Some(b"#!/bin/sh\necho hi\n")).unwrap();
        assert_eq!(
            (hit.type_id.as_str(), hit.stage),
            ("shell-script", TypeStage::Condition)
        );
        let entry = Entry::new("run", EntryKind::File);
        let skipped = Probe {
            entry: &entry,
            head: Some(b"#!/bin/sh\n"),
            skip_content: true,
        };
        assert_eq!(tree.detect(&skipped, &clock()), Err(Special::Unknown));
        assert_eq!(TypeStage::ORDER.map(TypeStage::accuracy), [3, 2, 1]);

        let webm_head: &[u8] =
            b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81\x01\x42\xF7\x81\x01\x42\xF2\x81\x04\x42\xF3\x81\x08\x42\x82\x84webm\x42\x87";
        let word = Condition::parse(r#"content:"webm""#).unwrap();
        assert!(word.test(&entry, Some(webm_head), &clock()).is_yes());
        let jpeg_marker = Condition::parse(r#"content:re:"^\xFF\xD8""#).unwrap();
        assert!(
            !jpeg_marker
                .test(&entry, Some(b"\xFF\xD8\xFF\xE0"), &clock())
                .is_yes(),
            "a byte that is not text is never found by a condition"
        );
        assert_eq!(
            word.test(&entry, None, &clock()),
            Truth::Unknown(Unknown::ContentNotRead)
        );
        let mut late = vec![b' '; 70];
        late[66..70].copy_from_slice(b"webm");
        let mut far = kind("far", &[], 0);
        far.when = Some(word);
        let tree = TypeTree::new(vec![far]).unwrap();
        assert_eq!(id(found(&tree, "clip", Some(&late))), "unknown");
    }

    #[test]
    fn name_patterns_take_globs_lists_and_expressions() {
        let tar = NamePattern::parse("*.tar.gz, *.tgz").unwrap();
        assert!(tar.is_match("Backup.TAR.GZ"));
        assert!(tar.is_match("x.tgz"));
        assert!(!tar.is_match("x.gz"));
        let shot = NamePattern::parse(r"re:^Screenshot_\d{8}_\d{6}\.png$").unwrap();
        assert!(shot.is_match("Screenshot_20261106_041500.png"));
        assert!(!shot.is_match("Screenshot_2026.png"));
        assert!(NamePattern::parse("[a").is_err());
        assert!(NamePattern::parse(" , ").is_err());
        let mut gz = kind("tar-gz", &[], 0);
        gz.patterns = vec!["*.tar.gz, *.tgz".to_owned()];
        let tree = TypeTree::new(vec![gz]).unwrap();
        assert_eq!(
            found(&tree, "a.tar.gz", None).unwrap().stage,
            TypeStage::Name
        );
    }

    #[test]
    fn links_special_empty_and_online_only_files_skip_content() {
        let file = Entry::new("a.png", EntryKind::File).with_size(10);
        assert!(!skips_content(&file));
        assert!(skips_content(&Entry::new("a.png", EntryKind::Link)));
        assert!(skips_content(&Entry::new("fifo", EntryKind::Special)));
        assert!(skips_content(&file.clone().with_size(0)));
        let online = file
            .clone()
            .with_extra(diroxide_vfs::facts::ONLINE_ONLY_KEY, "yes");
        assert!(skips_content(&online));
        assert!(!Probe::new(&file, None).skip_content);
        assert!(Probe::new(&file, None).skipping(true).skip_content);
        let item = Entry::new("Fix the build.c", EntryKind::Item);
        let tree = TypeTree::new(vec![kind("c", &["c"], 0)]).unwrap();
        assert_eq!(
            tree.detect(&Probe::new(&item, None), &clock()),
            Err(Special::Unknown)
        );
    }

    #[test]
    fn a_tree_refuses_an_unknown_parent_and_a_cycle() {
        let mut a = kind("a", &[], 0);
        a.parent = Some("b".to_owned());
        let error = TypeTree::new(vec![a.clone()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Type a names the parent b, which no type has."
        );
        let mut b = kind("b", &[], 0);
        b.parent = Some("a".to_owned());
        assert!(matches!(
            TypeTree::new(vec![a, b]),
            Err(TreeError::Cycle { .. })
        ));
    }
}
