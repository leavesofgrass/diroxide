//! The type model of alpha.7, in diroxide's words (R28 4.2 and 4.3; the
//! corrected plan 5.6): file types form a tree, and a file is tested in
//! three stages, in one order:
//!
//! 1. **Signature** ([`TypeStage::Signature`], accuracy 3): bytes at fixed
//!    positions in the first [`HEAD_BYTES`](crate::HEAD_BYTES), `??` for
//!    any byte.
//! 2. **Condition** ([`TypeStage::Condition`], accuracy 2): one filter
//!    expression over the entry and its first bytes (`content:re:"^#!"`).
//! 3. **Name** ([`TypeStage::Name`], accuracy 1): an extension or a name
//!    pattern.
//!
//! Per type, the first stage that passes is its hit. Among the types hit,
//! a child always beats its parent (DOCX, a child of zip by its name,
//! beats zip's signature), and otherwise the higher priority wins, then
//! the higher accuracy, then the earlier type. Priorities are never
//! added. A folder is never tested; the content stages are skipped for
//! links, special files, empty files, online-only files and folders the
//! owner marked "don't check", where the name stage still runs.
//!
//! The contract step of alpha 4 (P4) fixes these types with naive bodies:
//! [`TypeTree::detect`] tests every type, where the type model's lane
//! (commit 24) opens a type's children only under a hit or a type with no
//! tests, and adds the second test a signature of fewer than three fixed
//! bytes needs.

use std::fmt;

use diroxide_core::filter::{ContentQuery, Filter, FilterError, Lookup, Truth, Unknown};
use diroxide_core::pattern::Glob;
use diroxide_core::sort::NameCase;
use diroxide_core::{Clock, Entry};

use crate::{FileType, HEAD_BYTES};

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
        if compact.is_empty() || !compact.len().is_multiple_of(2) {
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
            Some(head) => Truth::of(query.regex().is_match(&String::from_utf8_lossy(head))),
            None => Truth::Unknown(Unknown::ContentNotRead),
        }
    }
}

/// What detection has to go on for one entry.
#[derive(Clone, Copy, Debug)]
pub struct Probe<'a> {
    /// The entry, with its name and kind.
    pub entry: &'a Entry,
    /// Its first bytes, when read; `None` while they are not.
    pub head: Option<&'a [u8]>,
    /// The content stages are skipped: a link, a special or empty file,
    /// an online-only file, a folder in `check.skip`.
    pub skip_content: bool,
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

/// The types as a tree: each with its parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeTree {
    types: Vec<FileType>,
    parents: Vec<Option<usize>>,
}

impl TypeTree {
    /// The tree of these types, in their order: every parent named must be
    /// a type, and no type may be its own ancestor.
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
        Ok(Self { types, parents })
    }

    /// The types, in order.
    pub fn types(&self) -> &[FileType] {
        &self.types
    }

    /// Whether `ancestor` is a parent of `index`, or a parent's parent.
    fn is_ancestor(&self, ancestor: usize, index: usize) -> bool {
        let mut at = self.parents[index];
        while let Some(i) = at {
            if i == ancestor {
                return true;
            }
            at = self.parents[i];
        }
        false
    }

    /// The first stage of a type that passes, in the stages' order.
    fn hit(&self, t: &FileType, probe: &Probe<'_>, clock: &dyn Clock) -> Option<TypeStage> {
        if !probe.skip_content
            && let Some(head) = probe.head
            && t.signatures.iter().any(|s| s.matches(head))
        {
            return Some(TypeStage::Signature);
        }
        if let Some(when) = &t.when {
            let head = if probe.skip_content { None } else { probe.head };
            if when.test(probe.entry, head, clock).is_yes() {
                return Some(TypeStage::Condition);
            }
        }
        let lower = probe.entry.name.to_lowercase();
        let by_extension = lower
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && t.extensions.iter().any(|e| e == ext));
        let by_pattern = t
            .patterns
            .iter()
            .any(|p| Glob::new(p, NameCase::Ignore).is_ok_and(|g| g.is_match(&probe.entry.name)));
        (by_extension || by_pattern).then_some(TypeStage::Name)
    }

    /// The entry's type: every type tested in the stages' order; among the
    /// hits a child beats its ancestors, then the higher priority, the
    /// higher accuracy and the earlier type win. A folder is never tested;
    /// with nothing found, [`Special::Pending`] while the first bytes are
    /// still to be read, else [`Special::Unknown`].
    pub fn detect(&self, probe: &Probe<'_>, clock: &dyn Clock) -> Result<Found, Special> {
        if probe.entry.is_folder_like() {
            return Err(Special::Folder);
        }
        let hits: Vec<(usize, TypeStage)> = self
            .types
            .iter()
            .enumerate()
            .filter_map(|(i, t)| self.hit(t, probe, clock).map(|stage| (i, stage)))
            .collect();
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
        ] {
            let error = Signature::parse(0, hex).unwrap_err().to_string();
            assert!(error.contains(why), "{hex}: {error}");
        }
        assert!(Signature::parse(62, "FF D8 FF").is_err(), "past 64 bytes");
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

    /// The `#!` condition, over the first bytes; skipped content leaves
    /// the name stage alone.
    #[test]
    fn a_condition_reads_the_first_bytes() {
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
