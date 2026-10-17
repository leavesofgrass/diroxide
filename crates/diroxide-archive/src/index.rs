//! The index of one archive (research/archives.md 5.2): one compact record
//! per member path, built once and cached, with parents synthesized, the
//! last copy of a duplicate winning (as extraction would write it), case
//! and composed-or-decomposed twins flagged, a file or link that shares a
//! name with a folder flagged (the folder wins), junk hidden rather than
//! dropped, and names refused with their reasons counted. Entering a
//! folder lists only its children, in archive order.
//!
//! A record holds the leaf name in one shared arena and a parent number,
//! so a million members cost about a hundred bytes each. Listing a folder
//! builds rows for at most [`MAX_LISTED`] children (spike M1: a flat
//! million rows alone take 413 MiB), and says so in words.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher, RandomState};
use std::sync::{Arc, Mutex};

use diroxide_core::format::{grouped, human_size};
use diroxide_core::{Attributes, Entry, EntryKind, Link, Timestamp};
use diroxide_vfs::names::{NameIssue, NameRules, check_name_with};

use crate::guard::{LimitHit, checked_total, refuse};
use crate::limits::{Limits, limit_words};
use crate::names::{NameFlags, NameRefusal, clash_key, has_archive_marker, is_junk, member_path};

/// The most children one listing returns: spike M1's cap.
pub const MAX_LISTED: usize = 500_000;

/// The line said when a folder holds more than [`MAX_LISTED`] entries:
/// "Over 500,000 entries: listing the first 500,000".
pub fn capped_line() -> String {
    let cap = grouped(MAX_LISTED as u64);
    format!("Over {cap} entries: listing the first {cap}")
}

/// The record number of the archive's root, which has no record.
pub const ROOT: u32 = u32::MAX;

/// What a member is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordKind {
    /// A folder, stored or synthesized.
    Folder,
    /// A file.
    File,
    /// A symbolic link; its target is in the index, never followed.
    Link,
    /// A hard link, device, pipe or anything else; listed, never read.
    Special,
}

/// Where a member's data is, for its format's reader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Locator {
    /// No data: a folder, or a member whose data was dropped.
    #[default]
    None,
    /// A zip member, by the zip crate's index.
    Zip(u32),
    /// A 7z member, by its file number.
    SevenZ(u32),
    /// A tar member: the offset of its data in the (decompressed) tar
    /// stream.
    Tar(u64),
    /// The whole decompressed stream: the one member of a plain xz.
    Whole,
}

/// Facts about a member that its row says in words.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RecordFlags(u16);

impl RecordFlags {
    /// No facts.
    pub const NONE: Self = Self(0);
    /// Junk other systems leave: hidden.
    pub const JUNK: Self = Self(1);
    /// The path appears more than once; the last copy is shown.
    pub const DUPLICATE: Self = Self(1 << 1);
    /// Another name in the folder clashes on some drives.
    pub const CASE_CLASH: Self = Self(1 << 2);
    /// A file and a folder share this name; the folder is shown.
    pub const FILE_CLASH: Self = Self(1 << 3);
    /// A link and a folder share this name; the folder is shown.
    pub const LINK_CLASH: Self = Self(1 << 4);
    /// Encrypted: "locked".
    pub const ENCRYPTED: Self = Self(1 << 5);
    /// Encrypted with zip's weak ZipCrypto.
    pub const WEAK_CRYPTO: Self = Self(1 << 6);
    /// Its local and central records disagree: refused on read.
    pub const MISMATCH: Self = Self(1 << 7);
    /// Its headers state two sizes (tar's PAX against the header).
    pub const SIZES_DISAGREE: Self = Self(1 << 8);
    /// It cannot be read here (a method or a sparse layout).
    pub const UNREADABLE: Self = Self(1 << 9);
    /// A link whose target leaves the archive.
    pub const LINK_LEAVES: Self = Self(1 << 10);
    /// A parent made up for a member whose folder the archive lacks.
    pub const SYNTHESIZED: Self = Self(1 << 11);
    /// It stores a CRC-32 to check.
    pub const HAS_CRC: Self = Self(1 << 12);

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Sets the flags of `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Clears the flags of `other`.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// Both sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::fmt::Debug for RecordFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RecordFlags({:#06x})", self.0)
    }
}

/// One member, compact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    name_start: u32,
    name_len: u32,
    /// The parent folder's record, or [`ROOT`].
    pub parent: u32,
    /// What it is.
    pub kind: RecordKind,
    /// The facts its row says.
    pub flags: RecordFlags,
    /// What the name rules found.
    pub name_flags: NameFlags,
    /// The size it claims, uncompressed.
    pub size: u64,
    packed: u64,
    modified: i64,
    mode: u32,
    /// Its stored CRC-32 (see [`RecordFlags::HAS_CRC`]).
    pub crc32: u32,
    /// Where its data is.
    pub locator: Locator,
}

/// The stored value that means "not known", for the compact fields.
const UNKNOWN_U64: u64 = u64::MAX;
const UNKNOWN_I64: i64 = i64::MIN;
const UNKNOWN_U32: u32 = u32::MAX;

impl Record {
    /// The bytes it takes packed, when known.
    pub fn packed(&self) -> Option<u64> {
        (self.packed != UNKNOWN_U64).then_some(self.packed)
    }

    /// When it last changed, in seconds since the epoch, when known.
    pub fn modified(&self) -> Option<i64> {
        (self.modified != UNKNOWN_I64).then_some(self.modified)
    }

    /// Its Unix mode bits, when stored.
    pub fn mode(&self) -> Option<u32> {
        (self.mode != UNKNOWN_U32).then_some(self.mode)
    }
}

/// A member as the format reader hands it to the builder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    /// The name as stored, decoded to text (lossy for bytes that are not
    /// text, with [`Member::not_unicode`] set).
    pub name: String,
    /// Whether the stored name was not valid text.
    pub not_unicode: bool,
    /// What it is, by its header; a folder is also any name ending in a
    /// slash.
    pub kind: Option<RecordKind>,
    /// The size it claims.
    pub size: u64,
    /// The bytes it takes packed, if known.
    pub packed: Option<u64>,
    /// When it last changed, in seconds since the epoch.
    pub modified: Option<i64>,
    /// Its Unix mode bits.
    pub mode: Option<u32>,
    /// Its stored CRC-32.
    pub crc32: Option<u32>,
    /// A link's target, as stored.
    pub link_target: Option<String>,
    /// Where its data is.
    pub locator: Locator,
    /// Facts the format reader found (encrypted, a mismatch).
    pub flags: RecordFlags,
    /// A note the format reader adds to the details: "method 99 is not
    /// supported".
    pub note: Option<String>,
}

/// What an index says about its archive as a whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexSummary {
    /// Members listed (files, links and special entries).
    pub members: u64,
    /// Folders, stored or made up.
    pub folders: u64,
    /// The sum of the members' claimed sizes, or `None` on an overflow.
    pub total_size: Option<u64>,
    /// Names refused.
    pub refused: u64,
    /// The first refusals' lines: "Skipped ../x: name leaves the archive."
    pub refused_lines: Vec<String>,
    /// Paths that appear more than once.
    pub duplicates: u64,
    /// Names that clash on some drives.
    pub clashes: u64,
    /// Notes on the whole archive, said once on opening: "Weak encryption
    /// in a.zip (ZipCrypto)."
    pub notes: Vec<String>,
}

/// How many refusal lines an index keeps; the rest are counted.
const KEPT_REFUSALS: usize = 64;

/// The index of one archive. Built by [`IndexBuilder`], shared read-only.
#[derive(Debug, Default)]
pub struct Index {
    records: Vec<Record>,
    arena: String,
    /// Children grouped by parent: the children of folder `f` are
    /// `children[starts[f + 1]..starts[f + 2]]`, of the root
    /// `children[starts[0]..starts[1]]`.
    children: Vec<u32>,
    /// The same groups, each sorted by name, for finding a child.
    by_name: Vec<u32>,
    starts: Vec<u32>,
    lookup: HashMap<u64, u32>,
    collisions: HashMap<u64, Vec<u32>>,
    hasher: RandomState,
    links: HashMap<u32, String>,
    duplicates: HashMap<u32, u32>,
    clash_with: HashMap<u32, u32>,
    shown_from: HashMap<u32, String>,
    notes: HashMap<u32, String>,
    summary: IndexSummary,
}

fn key(hasher: &RandomState, parent: u32, name: &str) -> u64 {
    let mut state = hasher.build_hasher();
    parent.hash(&mut state);
    name.hash(&mut state);
    state.finish()
}

impl Index {
    /// The number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the archive holds nothing.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// One record.
    pub fn record(&self, id: u32) -> Option<&Record> {
        self.records.get(id as usize)
    }

    /// A record's leaf name.
    pub fn name(&self, id: u32) -> &str {
        self.records.get(id as usize).map_or("", |r| {
            let start = r.name_start as usize;
            self.arena
                .get(start..start + r.name_len as usize)
                .unwrap_or("")
        })
    }

    /// A record's whole member path, from the root.
    pub fn path(&self, id: u32) -> String {
        let mut parts = Vec::new();
        let mut current = id;
        while current != ROOT {
            parts.push(self.name(current));
            current = self
                .records
                .get(current as usize)
                .map_or(ROOT, |r| r.parent);
        }
        parts.reverse();
        parts.join("/")
    }

    /// The record of a child called `name` of `parent`.
    pub fn child(&self, parent: u32, name: &str) -> Option<u32> {
        if !self.by_name.is_empty() || (self.lookup.is_empty() && !self.records.is_empty()) {
            let slot = if parent == ROOT {
                0
            } else {
                parent as usize + 1
            };
            let (Some(&a), Some(&b)) = (self.starts.get(slot), self.starts.get(slot + 1)) else {
                return None;
            };
            let group = self.by_name.get(a as usize..b as usize)?;
            return group
                .binary_search_by(|&id| self.name(id).cmp(name))
                .ok()
                .map(|at| group[at]);
        }
        let k = key(&self.hasher, parent, name);
        let matches = |id: u32| {
            self.records
                .get(id as usize)
                .is_some_and(|r| r.parent == parent)
                && self.name(id) == name
        };
        if let Some(&id) = self.lookup.get(&k)
            && matches(id)
        {
            return Some(id);
        }
        self.collisions
            .get(&k)
            .and_then(|ids| ids.iter().copied().find(|&id| matches(id)))
    }

    /// The record at a member path (forward slashes, already safe), or
    /// [`ROOT`] for the empty path; `None` when nothing is there.
    pub fn find(&self, path: &str) -> Option<u32> {
        let mut current = ROOT;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            current = self.child(current, part)?;
        }
        Some(current)
    }

    /// The children of a folder (or of [`ROOT`]), in archive order.
    pub fn children(&self, folder: u32) -> &[u32] {
        let slot = if folder == ROOT {
            0
        } else {
            folder as usize + 1
        };
        match (self.starts.get(slot), self.starts.get(slot + 1)) {
            (Some(&a), Some(&b)) => self.children.get(a as usize..b as usize).unwrap_or(&[]),
            _ => &[],
        }
    }

    /// What the index says about the archive.
    pub fn summary(&self) -> &IndexSummary {
        &self.summary
    }

    /// A link's stored target.
    pub fn link_target(&self, id: u32) -> Option<&str> {
        self.links.get(&id).map(String::as_str)
    }

    /// How many times a duplicate path appears.
    pub fn times(&self, id: u32) -> u32 {
        self.duplicates.get(&id).copied().unwrap_or(1)
    }

    /// The name another member clashes with.
    pub fn clashes_with(&self, id: u32) -> Option<&str> {
        self.clash_with.get(&id).map(|&other| self.name(other))
    }

    /// The format reader's note on a member.
    pub fn note(&self, id: u32) -> Option<&str> {
        self.notes.get(&id).map(String::as_str)
    }

    /// About how much memory the index holds, in bytes.
    pub fn memory(&self) -> u64 {
        let records = self.records.capacity() * size_of::<Record>();
        let arrays =
            (self.children.capacity() + self.by_name.capacity() + self.starts.capacity()) * 4;
        let lookup = self.lookup.capacity() * 16;
        (records + arrays + lookup + self.arena.capacity()) as u64
    }

    /// The row's entry for a record: the leaf name, the kind, the size and
    /// times, and the facts in words in [`Entry::extra`] (see
    /// [`member_row`]).
    pub fn entry(&self, id: u32) -> Option<Entry> {
        let record = self.records.get(id as usize)?;
        let name = self.name(id);
        let kind = match record.kind {
            RecordKind::Folder => EntryKind::Folder,
            RecordKind::File if crate::format::ArchiveFormat::from_name(name).is_some() => {
                EntryKind::Archive
            }
            RecordKind::File => EntryKind::File,
            RecordKind::Link => EntryKind::Link,
            RecordKind::Special => EntryKind::Special,
        };
        let mut entry = Entry::new(name.to_owned(), kind);
        if record.kind != RecordKind::Folder {
            entry.size = Some(record.size);
        }
        entry.packed_size = record.packed();
        entry.modified = record.modified().map(|secs| Timestamp::from_unix(secs, 0));
        entry.attributes = record.mode().map(|mode| Attributes::Unix(mode & 0o7777));
        if record.flags.contains(RecordFlags::HAS_CRC) {
            entry.crc32 = Some(record.crc32);
        }
        entry.hidden = record.flags.contains(RecordFlags::JUNK);
        entry.name_not_unicode = record.name_flags.contains(NameFlags::NOT_UNICODE);
        if let Some(target) = self.links.get(&id) {
            entry.link = Some(Link {
                target: target.clone(),
                points_to: None,
            });
        }
        let care = self.care_words(id, record);
        entry.needs_care = entry.needs_care || !care.is_empty();
        if !care.is_empty() {
            entry.extra.insert(EXTRA_CARE.to_owned(), care.join(", "));
        }
        let notes = self.note_words(id, record);
        if !notes.is_empty() {
            entry.extra.insert(EXTRA_NOTE.to_owned(), notes.join(", "));
        }
        if let Some(original) = self.shown_from.get(&id) {
            entry.extra.insert(EXTRA_NAME.to_owned(), original.clone());
        }
        if let Some(note) = self.notes.get(&id) {
            entry.extra.insert(EXTRA_DETAIL.to_owned(), note.clone());
        }
        Some(entry)
    }

    /// Why a name needs care, in words, the path's flags first, then the
    /// leaf's own issues under Windows rules (an archive travels).
    fn care_words(&self, _id: u32, record: &Record) -> Vec<String> {
        let mut words: Vec<String> = record
            .name_flags
            .words()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let name = self.name(_id);
        let rules = NameRules {
            windows: true,
            max_length: None,
            length_in_utf16: true,
        };
        for issue in check_name_with(name, &[], &rules) {
            let said = match issue {
                NameIssue::HiddenCharacters
                    if record.name_flags.contains(NameFlags::HIDDEN)
                        || record.name_flags.contains(NameFlags::CONTROL) =>
                {
                    continue;
                }
                NameIssue::TrailingDotOrSpace if name.ends_with(' ') => {
                    "ends with a space".to_owned()
                }
                NameIssue::TrailingDotOrSpace => "ends with a dot".to_owned(),
                other => other.describe(),
            };
            if !words.contains(&said) {
                words.push(said);
            }
        }
        words
    }

    /// The other facts, in words, the most important first.
    fn note_words(&self, id: u32, record: &Record) -> Vec<String> {
        let flags = record.flags;
        let mut words = Vec::new();
        if flags.contains(RecordFlags::ENCRYPTED) {
            words.push("locked".to_owned());
        }
        if flags.contains(RecordFlags::WEAK_CRYPTO) {
            words.push("weak encryption".to_owned());
        }
        if flags.contains(RecordFlags::MISMATCH) {
            words.push("its two headers disagree".to_owned());
        }
        if flags.contains(RecordFlags::UNREADABLE) {
            words.push("cannot be read here".to_owned());
        }
        if flags.contains(RecordFlags::DUPLICATE) {
            let times = self.times(id);
            let said = match times {
                0..=2 => "appears twice".to_owned(),
                n => format!("appears {n} times"),
            };
            words.push(format!("{said}, last copy shown"));
        }
        if flags.contains(RecordFlags::LINK_CLASH) {
            words.push("link and folder share a name".to_owned());
        }
        if flags.contains(RecordFlags::FILE_CLASH) {
            words.push("file and folder share a name".to_owned());
        }
        if flags.contains(RecordFlags::CASE_CLASH)
            && let Some(other) = self.clashes_with(id)
        {
            let name = self.name(id);
            if name.to_lowercase() == other.to_lowercase() {
                words.push(format!("case clash with {other}"));
            } else {
                words.push(format!("clashes with {other} on some drives"));
            }
        }
        if record.kind == RecordKind::Link
            && let Some(target) = self.links.get(&id)
        {
            words.push(format!("link to {target}, not followed"));
        }
        if flags.contains(RecordFlags::SIZES_DISAGREE) {
            words.push("sizes disagree, the PAX size used".to_owned());
        }
        if flags.contains(RecordFlags::JUNK) {
            words.push("hidden".to_owned());
        }
        words
    }
}

/// The key in [`Entry::extra`] for why a name needs care: "drive removed,
/// ends with a space".
pub const EXTRA_CARE: &str = "archive.care";
/// The key for the other facts in words: "appears twice, last copy shown".
pub const EXTRA_NOTE: &str = "archive.note";
/// The key for the stored name, spelled out, when the shown one differs.
pub const EXTRA_NAME: &str = "archive.name";
/// The key for the format reader's note.
pub const EXTRA_DETAIL: &str = "archive.detail";

/// A member's row in words, the meaning first: its name, then why it
/// needs care, then its other facts, then its kind and size ("a.txt,
/// appears twice, last copy shown, file, 9 bytes"). The app builds its
/// own rows from the same entry; this is the reference the tests read.
pub fn member_row(entry: &Entry) -> String {
    let mut parts = vec![entry.display_name().into_owned()];
    if let Some(care) = entry.extra.get(EXTRA_CARE) {
        parts.push(format!("name needs care: {care}"));
    } else if entry.needs_care {
        parts.push("name needs care".to_owned());
    }
    if let Some(note) = entry.extra.get(EXTRA_NOTE) {
        parts.push(note.clone());
    }
    parts.push(entry.kind.word().to_owned());
    if let Some(size) = entry.size {
        parts.push(human_size(size));
    }
    parts.join(", ")
}

/// Builds an [`Index`] member by member, under the limits.
#[derive(Debug)]
pub struct IndexBuilder {
    limits: Limits,
    index: Index,
    members: usize,
}

impl IndexBuilder {
    /// An empty builder.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            index: Index::default(),
            members: 0,
        }
    }

    /// Adds a note on the whole archive.
    pub fn note(&mut self, note: String) {
        if !self.index.summary.notes.contains(&note) {
            self.index.summary.notes.push(note);
        }
    }

    /// The records so far.
    pub fn len(&self) -> usize {
        self.index.records.len()
    }

    /// Whether nothing was added yet.
    pub fn is_empty(&self) -> bool {
        self.index.records.is_empty()
    }

    fn push(&mut self, parent: u32, name: &str, kind: RecordKind) -> Result<u32, LimitHit> {
        let estimate = (self.index.records.len() as u64 + 1) * 120 + self.index.arena.len() as u64;
        if estimate > self.limits.max_index_bytes {
            return Err(refuse(
                "index memory",
                limit_words(self.limits.max_index_bytes),
            ));
        }
        let id = u32::try_from(self.index.records.len())
            .map_err(|_| refuse("entries", grouped(u64::from(u32::MAX))))?;
        let name_start = u32::try_from(self.index.arena.len())
            .map_err(|_| refuse("index memory", limit_words(self.limits.max_index_bytes)))?;
        self.index.arena.push_str(name);
        self.index.records.push(Record {
            name_start,
            name_len: name.len() as u32,
            parent,
            kind,
            flags: RecordFlags::NONE,
            name_flags: NameFlags::NONE,
            size: 0,
            packed: UNKNOWN_U64,
            modified: UNKNOWN_I64,
            mode: UNKNOWN_U32,
            crc32: 0,
            locator: Locator::None,
        });
        let k = key(&self.index.hasher, parent, name);
        match self.index.lookup.entry(k) {
            std::collections::hash_map::Entry::Occupied(_) => {
                self.index.collisions.entry(k).or_default().push(id);
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(id);
            }
        }
        Ok(id)
    }

    /// The folder `name` under `parent`, made up when missing; a file or
    /// link in its place becomes the folder, flagged.
    fn folder(&mut self, parent: u32, name: &str) -> Result<u32, LimitHit> {
        if let Some(id) = self.index.child(parent, name) {
            let record = &mut self.index.records[id as usize];
            match record.kind {
                RecordKind::Folder => {}
                RecordKind::Link => {
                    record.kind = RecordKind::Folder;
                    record.flags.insert(RecordFlags::LINK_CLASH);
                    record.locator = Locator::None;
                    self.index.links.remove(&id);
                }
                RecordKind::File | RecordKind::Special => {
                    record.kind = RecordKind::Folder;
                    record.flags.insert(RecordFlags::FILE_CLASH);
                    record.locator = Locator::None;
                }
            }
            return Ok(id);
        }
        let id = self.push(parent, name, RecordKind::Folder)?;
        let mut flags = RecordFlags::SYNTHESIZED;
        if is_junk(name) {
            flags.insert(RecordFlags::JUNK);
        }
        self.index.records[id as usize].flags = flags;
        Ok(id)
    }

    /// Adds one member. A refused name is counted and its line kept; a
    /// limit reached stops the build.
    pub fn add(&mut self, member: Member) -> Result<Option<u32>, LimitHit> {
        // The limit counts the archive's own entries; the folders made up
        // for them are bounded by the index memory.
        if self.members >= self.limits.max_entries {
            return Err(refuse("entries", grouped(self.limits.max_entries as u64)));
        }
        self.members += 1;
        let path = match member_path(&member.name, &self.limits) {
            Ok(path) => path,
            Err(refusal) => {
                self.refused(refusal, &member.name);
                return Ok(None);
            }
        };
        let is_folder = member.kind == Some(RecordKind::Folder)
            || member.name.ends_with('/')
            || member.name.ends_with('\\');
        let kind = if is_folder {
            RecordKind::Folder
        } else {
            member.kind.unwrap_or(RecordKind::File)
        };
        if let Some(target) = member.link_target.as_ref()
            && target.len() > self.limits.max_link_target_bytes
        {
            self.refused(
                NameRefusal::TooLong {
                    max: self.limits.max_link_target_bytes,
                },
                &member.name,
            );
            return Ok(None);
        }
        let mut flags = path.flags;
        if member.not_unicode {
            flags.insert(NameFlags::NOT_UNICODE);
        }
        let (folders, leaf) = match path.path.rsplit_once('/') {
            Some((folders, leaf)) => (Some(folders), leaf),
            None => (None, path.path.as_str()),
        };
        let mut parent = ROOT;
        if let Some(folders) = folders {
            for part in folders.split('/') {
                parent = self.folder(parent, part)?;
            }
        }
        let id = if kind == RecordKind::Folder {
            let id = self.folder(parent, leaf)?;
            let record = &mut self.index.records[id as usize];
            record.flags.remove(RecordFlags::SYNTHESIZED);
            id
        } else if let Some(id) = self.index.child(parent, leaf) {
            let existing = self.index.records[id as usize].kind;
            if existing == RecordKind::Folder {
                // A file or link where a folder is: the folder wins, and
                // the clash is said on its row.
                let record = &mut self.index.records[id as usize];
                record.flags.insert(if kind == RecordKind::Link {
                    RecordFlags::LINK_CLASH
                } else {
                    RecordFlags::FILE_CLASH
                });
                return Ok(Some(id));
            }
            // The same path again: the last copy wins, as extraction
            // would write it.
            let times = self.index.duplicates.entry(id).or_insert(1);
            *times += 1;
            self.index.summary.duplicates += u64::from(*times == 2);
            let record = &mut self.index.records[id as usize];
            record.kind = kind;
            record.flags.insert(RecordFlags::DUPLICATE);
            self.index.links.remove(&id);
            id
        } else {
            self.push(parent, leaf, kind)?
        };
        let record = &mut self.index.records[id as usize];
        record.name_flags = record.name_flags.union(flags);
        let keep = RecordFlags::DUPLICATE
            .union(RecordFlags::FILE_CLASH)
            .union(RecordFlags::LINK_CLASH)
            .union(RecordFlags::SYNTHESIZED);
        let kept = RecordFlags(record.flags.0 & keep.0);
        record.flags = kept.union(member.flags);
        if is_junk(&path.path) {
            record.flags.insert(RecordFlags::JUNK);
        }
        if kind != RecordKind::Folder {
            record.size = member.size;
            record.packed = member.packed.unwrap_or(UNKNOWN_U64);
            record.locator = member.locator;
            if let Some(crc) = member.crc32 {
                record.crc32 = crc;
                record.flags.insert(RecordFlags::HAS_CRC);
            }
        }
        if let Some(modified) = member.modified {
            record.modified = modified;
        }
        if let Some(mode) = member.mode {
            record.mode = mode.min(UNKNOWN_U32 - 1);
        }
        if kind == RecordKind::Link
            && let Some(target) = member.link_target
        {
            if link_leaves(&path.path, &target) {
                record.flags.insert(RecordFlags::LINK_LEAVES);
            }
            self.index.links.insert(id, target);
        }
        let shown = &path.path;
        if flags.contains(NameFlags::CONTROL)
            || flags.contains(NameFlags::NOT_UNICODE)
            || has_archive_marker(leaf)
        {
            let spelled = diroxide_core::names::spell_out(&member.name).into_owned();
            if spelled != *shown {
                self.index.shown_from.insert(id, spelled);
            }
        }
        if let Some(note) = member.note {
            self.index.notes.insert(id, note);
        }
        Ok(Some(id))
    }

    fn refused(&mut self, refusal: NameRefusal, raw: &str) {
        self.index.summary.refused += 1;
        if self.index.summary.refused_lines.len() < KEPT_REFUSALS {
            self.index.summary.refused_lines.push(refusal.line(raw));
        }
    }

    /// Finishes the index: children grouped by folder in archive order,
    /// twins flagged, the summary counted.
    pub fn finish(mut self) -> Index {
        let index = &mut self.index;
        let count = index.records.len();
        let mut starts = vec![0u32; count + 2];
        for record in &index.records {
            let slot = if record.parent == ROOT {
                0
            } else {
                record.parent as usize + 1
            };
            starts[slot + 1] += 1;
        }
        for i in 1..starts.len() {
            starts[i] += starts[i - 1];
        }
        let mut fill = starts.clone();
        let mut children = vec![0u32; count];
        for (id, record) in index.records.iter().enumerate() {
            let slot = if record.parent == ROOT {
                0
            } else {
                record.parent as usize + 1
            };
            children[fill[slot] as usize] = id as u32;
            fill[slot] += 1;
        }
        index.children = children;
        index.starts = starts;
        // Twins: names in one folder that one key folds together.
        let groups = count + 1;
        for group in 0..groups {
            let folder = if group == 0 { ROOT } else { (group - 1) as u32 };
            let ids: Vec<u32> = index.children(folder).to_vec();
            if ids.len() < 2 {
                continue;
            }
            // The first name of each folded key, by its hash; a true
            // collision of two different keys goes to the short list.
            let mut seen: HashMap<u64, u32> = HashMap::with_capacity(ids.len());
            let mut others: Vec<(u64, u32)> = Vec::new();
            for id in ids {
                let folded = clash_key(index.name(id));
                let hash = index.hasher.hash_one(&folded);
                let twin = match seen.get(&hash) {
                    None => {
                        seen.insert(hash, id);
                        None
                    }
                    Some(&first) if clash_key(index.name(first)) == folded => Some(first),
                    Some(_) => {
                        let found = others
                            .iter()
                            .find(|(h, other)| {
                                *h == hash && clash_key(index.name(*other)) == folded
                            })
                            .map(|(_, other)| *other);
                        if found.is_none() {
                            others.push((hash, id));
                        }
                        found
                    }
                };
                if let Some(first) = twin {
                    for (this, that) in [(first, id), (id, first)] {
                        index.records[this as usize]
                            .flags
                            .insert(RecordFlags::CASE_CLASH);
                        index.clash_with.entry(this).or_insert(that);
                    }
                    index.summary.clashes += 1;
                }
            }
        }
        // Children sorted by name within each group, so a lookup is a
        // binary search; the build's hash map is then dropped.
        let mut by_name = index.children.clone();
        for slot in 0..index.starts.len().saturating_sub(1) {
            let (a, b) = (index.starts[slot] as usize, index.starts[slot + 1] as usize);
            let arena = &index.arena;
            let records = &index.records;
            let name_of = |id: u32| {
                let r = &records[id as usize];
                let start = r.name_start as usize;
                arena.get(start..start + r.name_len as usize).unwrap_or("")
            };
            by_name[a..b].sort_unstable_by(|&x, &y| name_of(x).cmp(name_of(y)));
        }
        index.by_name = by_name;
        index.lookup = HashMap::new();
        index.collisions = HashMap::new();
        index.records.shrink_to_fit();
        index.arena.shrink_to_fit();
        let mut members = 0u64;
        let mut folders = 0u64;
        for record in &index.records {
            if record.kind == RecordKind::Folder {
                folders += 1;
            } else {
                members += 1;
            }
        }
        index.summary.members = members;
        index.summary.folders = folders;
        index.summary.total_size = checked_total(
            index
                .records
                .iter()
                .filter(|r| r.kind != RecordKind::Folder)
                .map(|r| r.size),
        );
        self.index
    }
}

/// Whether a link's stored target leaves the archive from where the link
/// sits: absolute, a drive, or more `..` than the folders above it.
pub fn link_leaves(link_path: &str, target: &str) -> bool {
    let unified = target.replace('\\', "/");
    if unified.starts_with('/') || unified.as_bytes().get(1) == Some(&b':') {
        return true;
    }
    let mut depth = link_path.matches('/').count() as i64;
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => depth += 1,
        }
    }
    false
}

/// What identifies a container for the cache: its place as text, its
/// size and its modified time. A change of either invalidates the index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContainerKey {
    /// The container's place, written out (a nested container's place
    /// names its outer archive too).
    pub place: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its modified time, in seconds and nanoseconds since the epoch.
    pub modified: Option<(i64, u32)>,
}

/// A small cache of indexes, the most recently used kept: at most
/// `capacity` of them and `budget` bytes of index memory (research
/// archives.md 5.2: eight, 256 MiB).
#[derive(Debug)]
pub struct IndexCache<T> {
    entries: Mutex<Vec<(ContainerKey, Arc<T>, u64)>>,
    capacity: usize,
    budget: u64,
}

impl<T> IndexCache<T> {
    /// A cache holding at most `capacity` entries and `budget` bytes.
    pub fn new(capacity: usize, budget: u64) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            capacity: capacity.max(1),
            budget,
        }
    }

    /// The entry for a key, marked as just used.
    pub fn get(&self, key: &ContainerKey) -> Option<Arc<T>> {
        let mut entries = self.entries.lock().ok()?;
        let at = entries.iter().position(|(k, _, _)| k == key)?;
        let entry = entries.remove(at);
        let value = entry.1.clone();
        entries.push(entry);
        Some(value)
    }

    /// Keeps an entry of `bytes` bytes, dropping the least recently used
    /// ones past the capacity or the budget, and any older entry for the
    /// same place (the container changed).
    pub fn put(&self, key: ContainerKey, value: Arc<T>, bytes: u64) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        entries.retain(|(k, _, _)| k.place != key.place);
        entries.push((key, value, bytes));
        while entries.len() > self.capacity
            || (entries.len() > 1 && entries.iter().map(|(_, _, b)| *b).sum::<u64>() > self.budget)
        {
            entries.remove(0);
        }
    }

    /// The newest entry for a place, whatever its size and time: for a
    /// quick answer that needs no stat (capabilities, a title).
    pub fn peek(&self, place: &str) -> Option<Arc<T>> {
        let entries = self.entries.lock().ok()?;
        entries
            .iter()
            .rev()
            .find(|(k, _, _)| k.place == place)
            .map(|(_, v, _)| v.clone())
    }

    /// Drops every entry whose place starts with `prefix`: the archive and
    /// everything nested in it.
    pub fn forget(&self, prefix: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|(k, _, _)| !k.place.starts_with(prefix));
        }
    }

    /// How many entries are kept.
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |e| e.len())
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Member {
        Member {
            name: name.to_owned(),
            size,
            kind: Some(RecordKind::File),
            locator: Locator::Whole,
            ..Member::default()
        }
    }

    fn build(members: Vec<Member>) -> Index {
        let mut builder = IndexBuilder::new(Limits::default());
        for member in members {
            builder.add(member).unwrap();
        }
        builder.finish()
    }

    fn names(index: &Index, folder: u32) -> Vec<String> {
        index
            .children(folder)
            .iter()
            .map(|&id| index.name(id).to_owned())
            .collect()
    }

    #[test]
    fn parents_are_made_up_and_a_folder_lists_only_its_children() {
        let index = build(vec![
            file("a/b/c.txt", 3),
            file("a/d.txt", 4),
            file("top.txt", 5),
        ]);
        assert_eq!(names(&index, ROOT), ["a", "top.txt"]);
        let a = index.find("a").unwrap();
        assert_eq!(names(&index, a), ["b", "d.txt"]);
        let b = index.find("a/b").unwrap();
        assert_eq!(names(&index, b), ["c.txt"]);
        assert!(
            index
                .record(a)
                .unwrap()
                .flags
                .contains(RecordFlags::SYNTHESIZED)
        );
        assert_eq!(index.path(index.find("a/b/c.txt").unwrap()), "a/b/c.txt");
        assert_eq!(index.find("a/nothing"), None);
        assert_eq!(index.summary().members, 3);
        assert_eq!(index.summary().folders, 2);
        assert_eq!(index.summary().total_size, Some(12));
    }

    #[test]
    fn the_last_copy_wins_and_is_said() {
        let mut second = file("dup.txt", 9);
        second.locator = Locator::Zip(7);
        let index = build(vec![file("dup.txt", 1), second]);
        let id = index.find("dup.txt").unwrap();
        assert_eq!(index.record(id).unwrap().locator, Locator::Zip(7));
        let row = member_row(&index.entry(id).unwrap());
        assert_eq!(
            row,
            "dup.txt, appears twice, last copy shown, file, 9 bytes"
        );
        assert_eq!(index.summary().duplicates, 1);
    }

    #[test]
    fn a_folder_wins_over_a_file_or_link_of_its_name() {
        let link = Member {
            name: "link".to_owned(),
            kind: Some(RecordKind::Link),
            link_target: Some("../../etc".to_owned()),
            ..Member::default()
        };
        let index = build(vec![
            link,
            file("link/passwd", 4),
            file("a", 1),
            file("a/b", 1),
        ]);
        let link = index.find("link").unwrap();
        assert_eq!(index.record(link).unwrap().kind, RecordKind::Folder);
        assert!(member_row(&index.entry(link).unwrap()).contains("link and folder share a name"));
        let a = index.find("a").unwrap();
        assert!(member_row(&index.entry(a).unwrap()).contains("file and folder share a name"));
    }

    #[test]
    fn twins_and_junk_are_flagged_not_dropped() {
        let index = build(vec![
            file("README", 1),
            file("readme", 1),
            file("caf\u{e9}.txt", 1),
            file("cafe\u{301}.txt", 1),
            file("__MACOSX/._a", 1),
            file("ok.txt", 1),
        ]);
        let readme = index.entry(index.find("README").unwrap()).unwrap();
        assert!(member_row(&readme).starts_with("README, case clash with readme"));
        let cafe = index.entry(index.find("caf\u{e9}.txt").unwrap()).unwrap();
        assert!(member_row(&cafe).contains("clashes with cafe\u{301}.txt on some drives"));
        let junk = index.find("__MACOSX").unwrap();
        let entry = index.entry(junk).unwrap();
        assert!(entry.hidden);
        let inner = index.entry(index.find("__MACOSX/._a").unwrap()).unwrap();
        assert!(inner.hidden);
        assert!(!index.entry(index.find("ok.txt").unwrap()).unwrap().hidden);
        assert_eq!(index.summary().clashes, 2);
    }

    #[test]
    fn refused_names_are_counted_with_their_lines() {
        let index = build(vec![file("../x", 1), file("ok.txt", 1)]);
        assert_eq!(index.summary().refused, 1);
        assert_eq!(
            index.summary().refused_lines,
            ["Skipped ../x: name leaves the archive."]
        );
        assert_eq!(names(&index, ROOT), ["ok.txt"]);
    }

    #[test]
    fn the_limits_stop_a_build() {
        let limits = Limits {
            max_entries: 3,
            ..Limits::default()
        };
        let mut builder = IndexBuilder::new(limits);
        builder.add(file("a", 1)).unwrap();
        builder.add(file("b", 1)).unwrap();
        builder.add(file("c", 1)).unwrap();
        let hit = builder.add(file("d", 1)).unwrap_err();
        assert_eq!(
            hit.into_error().to_string(),
            "it passes the limit for entries: 3"
        );
        let tiny = Limits {
            max_index_bytes: 200,
            ..Limits::default()
        };
        let mut builder = IndexBuilder::new(tiny);
        builder.add(file("a", 1)).unwrap();
        assert!(builder.add(file("b", 1)).is_err());
    }

    #[test]
    fn links_say_where_they_point() {
        assert!(link_leaves("up", "../../outside"));
        assert!(link_leaves("a/b", "/etc/passwd"));
        assert!(link_leaves("a/b", r"C:\x"));
        assert!(!link_leaves("a/b", "../c"));
        assert!(link_leaves("a/b", "../../c"));
        let index = build(vec![Member {
            name: "up".to_owned(),
            kind: Some(RecordKind::Link),
            link_target: Some("../../outside".to_owned()),
            ..Member::default()
        }]);
        let id = index.find("up").unwrap();
        assert!(
            index
                .record(id)
                .unwrap()
                .flags
                .contains(RecordFlags::LINK_LEAVES)
        );
        assert_eq!(
            member_row(&index.entry(id).unwrap()),
            "up, link to ../../outside, not followed, link, 0 bytes"
        );
    }

    #[test]
    fn names_that_need_care_say_why() {
        let index = build(vec![
            file("trailing ", 1),
            file("CON", 1),
            file("a.txt:b", 1),
            file(r"\\?\C:\x", 1),
            file("a\u{0}b.txt", 1),
        ]);
        let row = |path: &str| member_row(&index.entry(index.find(path).unwrap()).unwrap());
        assert!(row("trailing ").contains("name needs care: ends with a space"));
        assert!(row("CON").contains("name needs care: is a Windows device"));
        assert!(row("a.txt:b").contains("reserved character :"));
        assert!(row("x").contains("drive removed"));
        let nul = index.entry(index.find("a\u{fffd}b.txt").unwrap()).unwrap();
        assert!(member_row(&nul).contains("contains a control character"));
        assert_eq!(
            nul.extra.get(EXTRA_NAME).map(String::as_str),
            Some("a[U+0000]b.txt")
        );
    }

    #[test]
    fn listings_cap_in_words() {
        assert_eq!(
            capped_line(),
            "Over 500,000 entries: listing the first 500,000"
        );
    }

    #[test]
    fn the_cache_keeps_the_recent_and_forgets_changed_containers() {
        let cache: IndexCache<u32> = IndexCache::new(2, 1_000);
        let key = |place: &str, size: u64| ContainerKey {
            place: place.to_owned(),
            size,
            modified: None,
        };
        cache.put(key("/a.zip", 1), Arc::new(1), 10);
        cache.put(key("/b.zip", 1), Arc::new(2), 10);
        assert_eq!(cache.get(&key("/a.zip", 1)).as_deref(), Some(&1));
        cache.put(key("/c.zip", 1), Arc::new(3), 10);
        assert!(
            cache.get(&key("/b.zip", 1)).is_none(),
            "least recently used"
        );
        assert!(cache.get(&key("/a.zip", 2)).is_none(), "the size changed");
        cache.put(key("/a.zip", 2), Arc::new(4), 10);
        assert_eq!(cache.len(), 2);
        cache.put(key("/big.zip", 1), Arc::new(5), 2_000);
        assert_eq!(cache.len(), 1, "over the budget, only the newest stays");
        cache.forget("/big.zip");
        assert!(cache.is_empty());
    }
}
