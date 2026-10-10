//! Entries: what is in a place. A provider fills them in; the app turns them
//! into rows.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::names;

/// What an entry is.
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
    /// A device, pipe, socket or anything else.
    Special,
}

impl EntryKind {
    /// The kind in words, for rows and messages: "folder", "file", "link",
    /// "archive", "special file".
    pub const fn word(self) -> &'static str {
        match self {
            EntryKind::Folder => "folder",
            EntryKind::File => "file",
            EntryKind::Link => "link",
            EntryKind::Archive => "archive",
            EntryKind::Special => "special file",
        }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Attributes {
    /// A Unix mode: permission bits and the set-id and sticky bits.
    Unix(u32),
    /// Windows file attribute flags.
    Windows(u32),
}

impl Attributes {
    const WINDOWS_WORDS: [(u32, &'static str); 5] = [
        (0x1, "read-only"),
        (0x2, "hidden"),
        (0x4, "system"),
        (0x20, "archive"),
        (0x400, "reparse point"),
    ];

    /// The attributes in text: `rwxr-xr-x` for a Unix mode, words such as
    /// "read-only, hidden" for Windows, or "normal" when no flag is set.
    pub fn describe(self) -> String {
        match self {
            Attributes::Unix(mode) => {
                let bits = ['r', 'w', 'x'];
                (0..9)
                    .map(|i| {
                        if mode & (0o400 >> i) != 0 {
                            bits[i % 3]
                        } else {
                            '-'
                        }
                    })
                    .collect()
            }
            Attributes::Windows(flags) => {
                let words: Vec<&str> = Self::WINDOWS_WORDS
                    .iter()
                    .filter(|(bit, _)| flags & bit != 0)
                    .map(|&(_, word)| word)
                    .collect();
                if words.is_empty() {
                    "normal".to_owned()
                } else {
                    words.join(", ")
                }
            }
        }
    }
}

/// One thing in a place. The real name is kept for operations; rows show
/// [`Entry::display_name`], with hidden characters replaced.
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
    /// When the entry's status last changed (Unix) or when it was created
    /// (Windows).
    pub changed: Option<Timestamp>,
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
            attributes: None,
            owner: None,
            group: None,
            link: None,
            inode: None,
            hard_links: None,
            blocks: None,
            hidden: false,
            type_name: None,
            labels: Vec::new(),
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

    /// The name for rows and speech: hidden characters become `?`.
    pub fn display_name(&self) -> Cow<'_, str> {
        names::display_name(&self.name)
    }

    /// The extension after the last dot, as written; `None` for names with
    /// no dot, or only a leading one such as `.profile`.
    pub fn extension(&self) -> Option<&str> {
        match self.name.rfind('.') {
            Some(0) | None => None,
            Some(at) if at + 1 < self.name.len() => Some(&self.name[at + 1..]),
            Some(_) => None,
        }
    }

    /// Whether Enter goes into it: a folder, an archive, or a link to one.
    pub fn is_container(&self) -> bool {
        match self.kind {
            EntryKind::Folder | EntryKind::Archive => true,
            EntryKind::Link => matches!(
                self.link.as_ref().and_then(|link| link.points_to),
                Some(EntryKind::Folder | EntryKind::Archive)
            ),
            EntryKind::File | EntryKind::Special => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(Entry::new("a.zip", EntryKind::Archive).is_container());
    }

    #[test]
    fn attributes_in_words() {
        assert_eq!(Attributes::Unix(0o755).describe(), "rwxr-xr-x");
        assert_eq!(Attributes::Unix(0o640).describe(), "rw-r-----");
        assert_eq!(Attributes::Windows(0x3).describe(), "read-only, hidden");
        assert_eq!(Attributes::Windows(0).describe(), "normal");
    }
}
