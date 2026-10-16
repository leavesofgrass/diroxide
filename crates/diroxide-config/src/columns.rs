//! The column kinds a panel row can say after its name, as the `columns`
//! setting names them, with each cell's words (research/permissions.md
//! section 6.10): the permissions in words first, then the Unix forms for
//! those who read them.
//!
//! A cell that has nothing to say is empty and left out of the row; the
//! row never says "unknown" for a fact the provider did not give.

use std::fmt;

use diroxide_core::clock::{self, Clock};
use diroxide_core::format::human_size;
use diroxide_core::{Entry, EntryKind, Timestamp};
use serde::{Deserialize, Serialize};

/// A detail said after an entry's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    /// The file type: "PDF", "folder".
    Type,
    /// The size in words: "2.1 MB".
    Size,
    /// "modified Tuesday".
    Modified,
    /// "created October 3".
    Created,
    /// "accessed today, 2:05 PM".
    Accessed,
    /// "changed October 3": when the entry's facts last changed.
    Changed,
    /// The permissions in words, rights first: "read: everyone; write:
    /// owner", or Windows attributes in words.
    Permissions,
    /// The Unix mode as `ls` writes it, ten characters: `-rw-r--r--`.
    Mode,
    /// The Unix mode as a number: `644`, `4755`.
    Octal,
    /// The owner's name.
    Owner,
    /// The group's name.
    Group,
    /// Windows attributes in words: "read-only, hidden", or "normal".
    Attributes,
    /// Where a link points: "to report.pdf".
    Link,
    /// The extension.
    Extension,
}

impl ColumnKind {
    /// Every kind, in the order the reference lists them.
    pub const ALL: [ColumnKind; 14] = [
        ColumnKind::Type,
        ColumnKind::Size,
        ColumnKind::Modified,
        ColumnKind::Created,
        ColumnKind::Accessed,
        ColumnKind::Changed,
        ColumnKind::Permissions,
        ColumnKind::Mode,
        ColumnKind::Octal,
        ColumnKind::Owner,
        ColumnKind::Group,
        ColumnKind::Attributes,
        ColumnKind::Link,
        ColumnKind::Extension,
    ];

    /// The name in `config.toml`.
    pub const fn name(self) -> &'static str {
        match self {
            ColumnKind::Type => "type",
            ColumnKind::Size => "size",
            ColumnKind::Modified => "modified",
            ColumnKind::Created => "created",
            ColumnKind::Accessed => "accessed",
            ColumnKind::Changed => "changed",
            ColumnKind::Permissions => "permissions",
            ColumnKind::Mode => "mode",
            ColumnKind::Octal => "octal",
            ColumnKind::Owner => "owner",
            ColumnKind::Group => "group",
            ColumnKind::Attributes => "attributes",
            ColumnKind::Link => "link",
            ColumnKind::Extension => "extension",
        }
    }

    /// The column in words, for the Settings list and a header.
    pub const fn label(self) -> &'static str {
        match self {
            ColumnKind::Type => "Type",
            ColumnKind::Size => "Size",
            ColumnKind::Modified => "Modified",
            ColumnKind::Created => "Created",
            ColumnKind::Accessed => "Accessed",
            ColumnKind::Changed => "Changed",
            ColumnKind::Permissions => "Permissions",
            ColumnKind::Mode => "Mode",
            ColumnKind::Octal => "Octal mode",
            ColumnKind::Owner => "Owner",
            ColumnKind::Group => "Group",
            ColumnKind::Attributes => "Attributes",
            ColumnKind::Link => "Link target",
            ColumnKind::Extension => "Extension",
        }
    }

    /// A kind from its name in `config.toml`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == name.trim())
    }

    /// The cell's words for an entry, or an empty string when the entry
    /// has nothing to say for this column.
    pub fn cell(self, entry: &Entry, clock: &dyn Clock) -> String {
        match self {
            ColumnKind::Type => entry
                .type_name
                .clone()
                .unwrap_or_else(|| entry.kind.word().to_owned()),
            ColumnKind::Size => match (entry.kind, entry.size) {
                (EntryKind::File | EntryKind::Archive, Some(size)) => human_size(size),
                _ => String::new(),
            },
            ColumnKind::Modified => time_cell("modified", entry.modified, clock),
            ColumnKind::Created => time_cell("created", entry.created, clock),
            ColumnKind::Accessed => time_cell("accessed", entry.accessed, clock),
            ColumnKind::Changed => time_cell("changed", entry.changed, clock),
            ColumnKind::Permissions => entry
                .attributes
                .map(|a| a.short_words(entry.kind))
                .unwrap_or_default(),
            ColumnKind::Mode => match entry.attributes {
                Some(attributes @ diroxide_core::Attributes::Unix(_)) => {
                    format!("{}{}", type_letter(entry), attributes.describe())
                }
                _ => String::new(),
            },
            ColumnKind::Octal => entry.attributes.map(|a| a.octal()).unwrap_or_default(),
            ColumnKind::Owner => entry.owner.clone().unwrap_or_default(),
            ColumnKind::Group => entry.group.clone().unwrap_or_default(),
            ColumnKind::Attributes => match entry.attributes {
                Some(attributes @ diroxide_core::Attributes::Windows(_)) => {
                    attributes.short_words(entry.kind)
                }
                _ => String::new(),
            },
            ColumnKind::Link => entry
                .link
                .as_ref()
                .map(|l| format!("to {}", l.target))
                .unwrap_or_default(),
            ColumnKind::Extension => entry.extension().unwrap_or_default().to_owned(),
        }
    }
}

impl fmt::Display for ColumnKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

fn time_cell(word: &str, time: Option<Timestamp>, clock: &dyn Clock) -> String {
    time.map(|t| format!("{word} {}", clock::describe_time(t, clock)))
        .unwrap_or_default()
}

/// The first letter of `ls -l`: `d` for a folder, `l` for a link, `-` for
/// the rest.
fn type_letter(entry: &Entry) -> char {
    match entry.kind {
        EntryKind::Folder => 'd',
        EntryKind::Link => 'l',
        _ => '-',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{Attributes, FixedClock, UtcOffset};

    fn clock() -> FixedClock {
        FixedClock::new(
            Timestamp::from_civil(2026, 10, 10, 15, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        )
    }

    #[test]
    fn names_read_back() {
        for kind in ColumnKind::ALL {
            assert_eq!(ColumnKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ColumnKind::from_name("colour"), None);
    }

    #[test]
    fn the_permission_columns_read_words_first() {
        let clock = clock();
        let mut file = Entry::new("run.sh", EntryKind::File);
        file.attributes = Some(Attributes::Unix(0o4755));
        assert_eq!(ColumnKind::Octal.cell(&file, &clock), "4755");
        assert_eq!(ColumnKind::Mode.cell(&file, &clock), "-rwsr-xr-x");
        let words = ColumnKind::Permissions.cell(&file, &clock);
        assert!(!words.is_empty() && !words.contains('-'), "{words}");
        let mut folder = Entry::new("Music", EntryKind::Folder);
        folder.attributes = Some(Attributes::Unix(0o755));
        assert_eq!(ColumnKind::Mode.cell(&folder, &clock), "drwxr-xr-x");
        let mut windows = Entry::new("a.txt", EntryKind::File);
        windows.attributes = Some(Attributes::Windows(0x1 | 0x2));
        assert_eq!(ColumnKind::Mode.cell(&windows, &clock), "");
        assert_eq!(ColumnKind::Octal.cell(&windows, &clock), "");
        assert_eq!(
            ColumnKind::Attributes.cell(&windows, &clock),
            "read-only, hidden"
        );
    }

    #[test]
    fn empty_facts_give_empty_cells() {
        let clock = clock();
        let bare = Entry::new("x", EntryKind::File);
        for kind in [
            ColumnKind::Size,
            ColumnKind::Modified,
            ColumnKind::Permissions,
            ColumnKind::Mode,
            ColumnKind::Owner,
            ColumnKind::Group,
            ColumnKind::Link,
        ] {
            assert_eq!(kind.cell(&bare, &clock), "", "{kind}");
        }
        assert_eq!(ColumnKind::Type.cell(&bare, &clock), "file");
    }
}
