//! Entries: what is in a place. A provider fills them in; the app turns them
//! into rows.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::names;

/// What an entry is.
///
/// An entry kind this version does not know, from a plugin or a newer
/// journal, reads as [`EntryKind::Special`]: listed, and acted on by
/// nothing that needs a file or a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// A folder.
    Folder,
    /// A regular file.
    File,
    /// A symbolic link, junction or shortcut; see [`Entry::link`].
    Link,
    /// A file that is an archive the archive provider can enter.
    Archive,
    /// A row that is not a file: a commit in the git plugin's log, a
    /// partition of a disk image, a plugin's own item. Enter runs the
    /// command its lister names; file commands say they do not apply.
    Item,
    /// A device, pipe, socket, anything else, or a kind this version does
    /// not know.
    #[serde(other)]
    Special,
}

impl EntryKind {
    /// The kind in words, for rows and messages: "folder", "file", "link",
    /// "archive", "item", "special file".
    pub const fn word(self) -> &'static str {
        match self {
            EntryKind::Folder => "folder",
            EntryKind::File => "file",
            EntryKind::Link => "link",
            EntryKind::Archive => "archive",
            EntryKind::Item => "item",
            EntryKind::Special => "special file",
        }
    }

    /// Whether this is a file system entry at all; an [`EntryKind::Item`]
    /// is not, so copy, move, delete and the rest do not apply to it.
    pub const fn is_file_system(self) -> bool {
        !matches!(self, EntryKind::Item)
    }
}

/// Where a link points.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Link {
    /// The target as stored in the link, not resolved.
    pub target: String,
    /// What the target is, or `None` when the link is broken or the target
    /// was not checked.
    pub points_to: Option<EntryKind>,
}

/// Permissions and attributes, as the provider's platform has them.
///
/// `Unix(v)` is the low twelve bits of the mode: the nine permission bits
/// and the setuid, setgid and sticky bits; a provider writes all twelve.
/// `Windows(v)` is the full attribute value; a provider keeps the bits it
/// cannot set unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Attributes {
    /// A Unix mode: permission bits and the set-id and sticky bits.
    Unix(u32),
    /// Windows file attribute flags.
    Windows(u32),
}

/// The Windows attribute bits and their words, in the order said.
const WINDOWS_WORDS: [(u32, &str); 11] = [
    (0x1, "read-only"),
    (0x2, "hidden"),
    (0x4, "system"),
    (0x20, "marked for backup"),
    (0x200, "sparse"),
    (0x400, "reparse point"),
    (0x800, "compressed"),
    (0x1000, "offline"),
    (0x4000, "encrypted"),
    (0x40000, "online only"),
    (0x400000, "online only"),
];

/// The archive attribute, which many files carry; short forms leave it out.
const WINDOWS_ARCHIVE: u32 = 0x20;
/// The read-only attribute.
const WINDOWS_READ_ONLY: u32 = 0x1;

const SETUID: u32 = 0o4000;
const SETGID: u32 = 0o2000;
const STICKY: u32 = 0o1000;

/// The three classes of a Unix mode, from the owner outward.
const CLASSES: [(&str, u32); 3] = [("owner", 6), ("group", 3), ("others", 0)];

impl Attributes {
    /// The attributes in text: `rwxr-xr-x` for a Unix mode, with `s`, `S`,
    /// `t` and `T` in the execute places as `ls` writes them (so 4755 reads
    /// `rwsr-xr-x`); words such as "read-only, hidden" for Windows, or
    /// "normal" when no flag is set.
    pub fn describe(self) -> String {
        match self {
            Attributes::Unix(mode) => {
                let mut text = String::with_capacity(9);
                for (class, (_, shift)) in CLASSES.iter().enumerate() {
                    let bits = (mode >> shift) & 0o7;
                    text.push(if bits & 0o4 != 0 { 'r' } else { '-' });
                    text.push(if bits & 0o2 != 0 { 'w' } else { '-' });
                    let special = match class {
                        0 => mode & SETUID != 0,
                        1 => mode & SETGID != 0,
                        _ => mode & STICKY != 0,
                    };
                    let run = bits & 0o1 != 0;
                    let letter = if class == 2 { 't' } else { 's' };
                    text.push(match (special, run) {
                        (true, true) => letter,
                        (true, false) => letter.to_ascii_uppercase(),
                        (false, true) => 'x',
                        (false, false) => '-',
                    });
                }
                text
            }
            Attributes::Windows(flags) => windows_words(flags, true),
        }
    }

    /// The mode as an octal number: "644", or four digits when a setuid,
    /// setgid or sticky bit is set ("4755"). Windows attributes have no
    /// octal form, so they give an empty string.
    pub fn octal(self) -> String {
        match self {
            Attributes::Unix(mode) => {
                let mode = mode & 0o7777;
                if mode > 0o777 {
                    format!("{mode:04o}")
                } else {
                    format!("{mode:03o}")
                }
            }
            Attributes::Windows(_) => String::new(),
        }
    }

    /// The class sentence, for the details view and Say Details: "Owner can
    /// read and write, group can read, others can read." A class with no
    /// access reads "others have no access"; special bits add a sentence
    /// ("Runs as its owner."). On a folder the rights are list, enter and
    /// change. Windows attributes are said in words, with read-only on a
    /// folder explained.
    pub fn words(self, kind: EntryKind) -> String {
        let folder = kind == EntryKind::Folder;
        match self {
            Attributes::Unix(mode) => {
                let mut clauses = Vec::with_capacity(3);
                for (i, (who, shift)) in CLASSES.iter().enumerate() {
                    let phrase = class_phrase((mode >> shift) & 0o7, folder);
                    let who = if i == 0 { "Owner" } else { who };
                    let verb = if phrase.starts_with("no ") {
                        if i == 2 { "have" } else { "has" }
                    } else {
                        "can"
                    };
                    clauses.push(format!("{who} {verb} {phrase}"));
                }
                let mut text = format!("{}.", clauses.join(", "));
                for special in special_clauses(mode, folder) {
                    let mut chars = special.chars();
                    if let Some(first) = chars.next() {
                        text.push(' ');
                        text.extend(first.to_uppercase());
                        text.push_str(chars.as_str());
                        text.push('.');
                    }
                }
                text
            }
            Attributes::Windows(flags) => {
                let words = windows_words(flags, true);
                if folder && flags & WINDOWS_READ_ONLY != 0 {
                    words.replacen(
                        "read-only",
                        "read-only (a mark only; Windows ignores it on folders)",
                        1,
                    )
                } else {
                    words
                }
            }
        }
    }

    /// The short form, for a column, the status line and plan lines:
    /// rights first, then who ("read: everyone; write: owner"). Rights that
    /// share an audience merge ("read and run: everyone"); clauses run from
    /// the widest audience to the narrowest; a file nobody can write adds
    /// "write: nobody"; special bits add a clause; no bits at all read "no
    /// access". Windows attributes are said in words without "marked for
    /// backup", which most files carry.
    pub fn short_words(self, kind: EntryKind) -> String {
        let folder = kind == EntryKind::Folder;
        match self {
            Attributes::Unix(mode) => short_unix(mode, folder),
            Attributes::Windows(flags) => windows_words(flags, false),
        }
    }
}

/// The rights a class has, as one phrase (permissions.md section 6.1).
fn class_phrase(bits: u32, folder: bool) -> &'static str {
    match (bits & 0o7, folder) {
        (0o4, false) => "read",
        (0o6, false) => "read and write",
        (0o5, false) => "read and run",
        (0o7, false) => "read, write and run",
        (0o1, false) => "run only",
        (0o3, false) => "write and run",
        (0o2, false) => "write only",
        (0o4, true) => "list names only",
        (0o6, true) => "list names only (changing needs enter)",
        (0o5, true) => "list and enter",
        (0o7, true) => "list, enter and change contents",
        (0o1, true) => "enter only, not list",
        (0o3, true) => "enter and change contents, not list",
        (0o2, true) => "no usable access (changing needs enter)",
        _ => "no access",
    }
}

/// The special bits as clauses, in lowercase words.
fn special_clauses(mode: u32, folder: bool) -> Vec<&'static str> {
    let mut clauses = Vec::new();
    if folder {
        if mode & SETGID != 0 {
            clauses.push("new items get its group");
        }
        if mode & STICKY != 0 {
            clauses.push("only owners delete inside");
        }
    } else {
        if mode & SETUID != 0 {
            clauses.push("runs as its owner");
        }
        if mode & SETGID != 0 {
            clauses.push("runs as its group");
        }
    }
    clauses
}

fn audience_word(classes: u32) -> &'static str {
    match classes {
        0b111 => "everyone",
        0b110 => "owner and group",
        0b101 => "owner and others",
        0b011 => "group and others",
        0b100 => "owner",
        0b010 => "group",
        0b001 => "others",
        _ => "nobody",
    }
}

fn join_words(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn short_unix(mode: u32, folder: bool) -> String {
    // Each right with its bit within a class, in the order said.
    let rights: [(&str, u32); 3] = if folder {
        [("list", 0o4), ("enter", 0o1), ("change", 0o2)]
    } else {
        [("read", 0o4), ("write", 0o2), ("run", 0o1)]
    };
    // For each right, the classes that have it, as three bits: owner,
    // group, others.
    let audience = |bit: u32| {
        CLASSES
            .iter()
            .enumerate()
            .filter(|(_, (_, shift))| (mode >> shift) & bit != 0)
            .fold(0, |all, (i, _)| all | (0b100 >> i))
    };
    let mut clauses: Vec<(u32, usize, Vec<&str>)> = Vec::new();
    for (index, (right, bit)) in rights.iter().enumerate() {
        let who = audience(*bit);
        if who == 0 {
            continue;
        }
        match clauses.iter_mut().find(|(w, _, _)| *w == who) {
            Some((_, _, names)) => names.push(right),
            None => clauses.push((who, index, vec![right])),
        }
    }
    clauses.sort_by(|a, b| b.0.count_ones().cmp(&a.0.count_ones()).then(a.1.cmp(&b.1)));
    let mut parts: Vec<String> = clauses
        .iter()
        .map(|(who, _, names)| format!("{}: {}", join_words(names), audience_word(*who)))
        .collect();
    let nobody_writes = audience(0o2) == 0;
    if parts.is_empty() && special_clauses(mode, folder).is_empty() {
        return "no access".to_owned();
    }
    if !folder && nobody_writes && !parts.is_empty() {
        parts.push("write: nobody".to_owned());
    }
    parts.extend(special_clauses(mode, folder).into_iter().map(str::to_owned));
    parts.join("; ")
}

fn windows_words(flags: u32, with_archive: bool) -> String {
    let mut words: Vec<&str> = Vec::new();
    for (bit, word) in WINDOWS_WORDS {
        if flags & bit != 0 && (with_archive || bit != WINDOWS_ARCHIVE) && !words.contains(&word) {
            words.push(word);
        }
    }
    if words.is_empty() {
        "normal".to_owned()
    } else {
        words.join(", ")
    }
}

/// One thing in a place. The real name is kept for operations; rows show
/// [`Entry::display_name`], with hidden characters replaced.
///
/// `None` in a field means "not available here, or not loaded".
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The real name, exactly as the provider gave it.
    pub name: String,
    /// What the entry is.
    pub kind: EntryKind,
    /// The size in bytes, when known.
    pub size: Option<u64>,
    /// When the contents last changed.
    pub modified: Option<Timestamp>,
    /// When the entry was last read.
    pub accessed: Option<Timestamp>,
    /// When the entry's status last changed: the Unix status-change time
    /// (ctime), or the Windows change time read from the handle. Never the
    /// creation time, which is [`Entry::created`]. It cannot be set.
    pub changed: Option<Timestamp>,
    /// When the entry was created (its birth time), where the platform and
    /// the file system keep one.
    #[serde(default)]
    pub created: Option<Timestamp>,
    /// Permissions or attributes.
    pub attributes: Option<Attributes>,
    /// The owner's user name.
    pub owner: Option<String>,
    /// The group name.
    pub group: Option<String>,
    /// Where a link points, for links.
    pub link: Option<Link>,
    /// The inode or file index, when the platform has one.
    pub inode: Option<u64>,
    /// The device or volume the entry is on, as the platform numbers it;
    /// with [`Entry::inode`] it identifies the entry. Recursive work stops
    /// where it changes (the boundary rule).
    #[serde(default)]
    pub volume: Option<u64>,
    /// The number of hard links.
    pub hard_links: Option<u64>,
    /// The blocks allocated.
    pub blocks: Option<u64>,
    /// Hidden by its name or its attributes.
    pub hidden: bool,
    /// The detected file type's short name, such as "PDF"; `None` until
    /// the type is checked.
    pub type_name: Option<String>,
    /// The user's labels on this entry.
    pub labels: Vec<String>,
    /// More facts as text, by key: a plugin's columns, the custom
    /// attribute column, an item row's fields. Shown and sorted by key
    /// (`SortKey::Extra`); never interpreted by the core.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
    /// The name is dangerous to type: hidden or reserved characters,
    /// trailing dots or spaces, a device name. Rows say "name needs care".
    pub needs_care: bool,
    /// The provider could not read the name as Unicode, so `name` is a
    /// lossy copy; operations on it are refused with a message.
    pub name_not_unicode: bool,
}

impl Entry {
    /// An entry with a name and a kind, and nothing else known yet.
    pub fn new(name: impl Into<String>, kind: EntryKind) -> Self {
        let name = name.into();
        let needs_care = names::has_hidden(&name);
        Self {
            name,
            kind,
            size: None,
            modified: None,
            accessed: None,
            changed: None,
            created: None,
            attributes: None,
            owner: None,
            group: None,
            link: None,
            inode: None,
            volume: None,
            hard_links: None,
            blocks: None,
            hidden: false,
            type_name: None,
            labels: Vec::new(),
            extra: BTreeMap::new(),
            needs_care,
            name_not_unicode: false,
        }
    }

    /// The same entry with a size.
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// The same entry with a modification time.
    pub fn with_modified(mut self, modified: Timestamp) -> Self {
        self.modified = Some(modified);
        self
    }

    /// The same entry with one more fact in [`Entry::extra`].
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// The name for rows and speech: hidden characters become `?`.
    pub fn display_name(&self) -> Cow<'_, str> {
        names::display_name(&self.name)
    }

    /// The extension after the last dot, as written, without the dot;
    /// `None` for names with no dot, or only a leading one such as
    /// `.profile`. For the stem and extension together, and for compound
    /// extensions such as `.tar.gz`, see [`names::split_extension`].
    pub fn extension(&self) -> Option<&str> {
        let (_, extension) = names::split_extension(&self.name);
        extension.strip_prefix('.').filter(|e| !e.is_empty())
    }

    /// Whether Enter goes into it: a folder, an archive, or a link to one.
    pub fn is_container(&self) -> bool {
        match self.kind {
            EntryKind::Folder | EntryKind::Archive => true,
            EntryKind::Link => matches!(
                self.link.as_ref().and_then(|link| link.points_to),
                Some(EntryKind::Folder | EntryKind::Archive)
            ),
            EntryKind::File | EntryKind::Item | EntryKind::Special => false,
        }
    }

    /// Whether it sorts with the folders: a folder, or a link to one.
    pub fn is_folder_like(&self) -> bool {
        match self.kind {
            EntryKind::Folder => true,
            EntryKind::Link => matches!(
                self.link.as_ref().and_then(|link| link.points_to),
                Some(EntryKind::Folder)
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::within_cells;

    #[test]
    fn extensions() {
        assert_eq!(
            Entry::new("report.pdf", EntryKind::File).extension(),
            Some("pdf")
        );
        assert_eq!(
            Entry::new("a.tar.gz", EntryKind::File).extension(),
            Some("gz")
        );
        assert_eq!(Entry::new(".profile", EntryKind::File).extension(), None);
        assert_eq!(Entry::new("trailing.", EntryKind::File).extension(), None);
        assert_eq!(Entry::new("Makefile", EntryKind::File).extension(), None);
    }

    #[test]
    fn hidden_characters_mark_the_name() {
        let entry = Entry::new("C\u{F03A}", EntryKind::Folder);
        assert!(entry.needs_care);
        assert_eq!(entry.display_name(), "C?");
        assert_eq!(entry.name, "C\u{F03A}");
    }

    #[test]
    fn containers() {
        let mut link = Entry::new("l", EntryKind::Link);
        assert!(!link.is_container());
        link.link = Some(Link {
            target: "/x".into(),
            points_to: Some(EntryKind::Folder),
        });
        assert!(link.is_container());
        assert!(link.is_folder_like());
        assert!(Entry::new("a.zip", EntryKind::Archive).is_container());
        assert!(!Entry::new("a.zip", EntryKind::Archive).is_folder_like());
        assert!(!Entry::new("abc123", EntryKind::Item).is_container());
        assert!(!EntryKind::Item.is_file_system());
    }

    #[test]
    fn attributes_in_words() {
        assert_eq!(Attributes::Unix(0o755).describe(), "rwxr-xr-x");
        assert_eq!(Attributes::Unix(0o640).describe(), "rw-r-----");
        assert_eq!(Attributes::Unix(0o4755).describe(), "rwsr-xr-x");
        assert_eq!(Attributes::Unix(0o1777).describe(), "rwxrwxrwt");
        assert_eq!(Attributes::Unix(0o2644).describe(), "rw-r-Sr--");
        assert_eq!(Attributes::Windows(0x3).describe(), "read-only, hidden");
        assert_eq!(Attributes::Windows(0).describe(), "normal");
        assert_eq!(
            Attributes::Windows(0x20 | 0x800).describe(),
            "marked for backup, compressed"
        );
        assert_eq!(
            Attributes::Windows(0x40000 | 0x400000).describe(),
            "online only"
        );
    }

    #[test]
    fn octal_has_four_digits_only_for_special_bits() {
        assert_eq!(Attributes::Unix(0o644).octal(), "644");
        assert_eq!(Attributes::Unix(0o4755).octal(), "4755");
        assert_eq!(Attributes::Unix(0o100_644).octal(), "644");
        assert_eq!(Attributes::Unix(0).octal(), "000");
        assert_eq!(Attributes::Windows(0x1).octal(), "");
    }

    #[test]
    fn short_forms_fit_one_line() {
        let file = |mode| Attributes::Unix(mode).short_words(EntryKind::File);
        let folder = |mode| Attributes::Unix(mode).short_words(EntryKind::Folder);
        let table = [
            (file(0o600), "read and write: owner"),
            (file(0o644), "read: everyone; write: owner"),
            (file(0o664), "read: everyone; write: owner and group"),
            (file(0o755), "read and run: everyone; write: owner"),
            (folder(0o755), "list and enter: everyone; change: owner"),
            (folder(0o700), "list, enter and change: owner"),
            (file(0o666), "read and write: everyone"),
            (file(0o444), "read: everyone; write: nobody"),
            (file(0o000), "no access"),
            (
                file(0o4755),
                "read and run: everyone; write: owner; runs as its owner",
            ),
            (
                folder(0o1777),
                "list, enter and change: everyone; only owners delete inside",
            ),
        ];
        for (said, expected) in table {
            assert_eq!(said, expected);
        }
        for mode in [0o600, 0o644, 0o664, 0o755, 0o666, 0o444, 0o000] {
            assert!(file(mode).chars().count() <= 40, "{mode:o}");
        }
        assert!(folder(0o755).chars().count() <= 40);
    }

    #[test]
    fn class_sentences_say_each_class() {
        assert_eq!(
            Attributes::Unix(0o644).words(EntryKind::File),
            "Owner can read and write, group can read, others can read."
        );
        assert_eq!(
            Attributes::Unix(0o750).words(EntryKind::Folder),
            "Owner can list, enter and change contents, group can list and enter, others have no access."
        );
        assert_eq!(
            Attributes::Unix(0o4755).words(EntryKind::File),
            "Owner can read, write and run, group can read and run, others can read and run. Runs as its owner."
        );
        assert!(within_cells(
            &Attributes::Unix(0o600).words(EntryKind::File),
            "Owner can read and write"
        ));
    }

    #[test]
    fn windows_folders_explain_read_only() {
        assert_eq!(
            Attributes::Windows(0x1).words(EntryKind::Folder),
            "read-only (a mark only; Windows ignores it on folders)"
        );
        assert_eq!(
            Attributes::Windows(0x21).short_words(EntryKind::File),
            "read-only"
        );
        assert_eq!(
            Attributes::Windows(0x20).short_words(EntryKind::File),
            "normal"
        );
    }

    #[test]
    fn unknown_kinds_read_as_special_and_extra_round_trips() {
        let kind: EntryKind = serde_json::from_str("\"commit\"").unwrap();
        assert_eq!(kind, EntryKind::Special);
        let item: EntryKind = serde_json::from_str("\"item\"").unwrap();
        assert_eq!(item, EntryKind::Item);
        let entry = Entry::new("abc123", EntryKind::Item).with_extra("git.author", "Ada Example");
        let text = serde_json::to_string(&entry).unwrap();
        let back: Entry = serde_json::from_str(&text).unwrap();
        assert_eq!(
            back.extra.get("git.author").map(String::as_str),
            Some("Ada Example")
        );
    }
}
