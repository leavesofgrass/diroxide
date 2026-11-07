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

/// The longest column key, so a header stays short.
pub const KEY_LIMIT: usize = 64;

/// The prefix of a regex column's key: `regex.episode` is the column the
/// `regex_columns` table's `episode` pattern fills.
pub const REGEX_PREFIX: &str = "regex.";

/// Whether `text` is a column key the app resolves (alpha 4, R30 commit
/// 52): a metadata field (`audio.artist`, `doc.pages`), the owner's own
/// (`custom.note`, `custom.project`), a regex column (`regex.episode`) or
/// a plugin's column (`diro-git.git`). Small letters, digits, `-` and `_`
/// in parts joined by dots, at least two parts, at most [`KEY_LIMIT`]
/// characters. Which keys a reader or a plugin gives is the app's to
/// say; a key nothing gives leaves its cells empty.
pub fn is_column_key(text: &str) -> bool {
    let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
    text.len() <= KEY_LIMIT
        && text.contains('.')
        && text
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(plain))
}

/// Whether `name` may name a regex column in `regex_columns`: small
/// letters, digits, `-` and `_`, at most 24 characters.
pub fn is_regex_column_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 24
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// One column of the `columns` setting: a built-in kind, or a key the
/// app resolves to a metadata field, the owner's own field, a regex
/// column or a plugin's column. Written as its name or its key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColumnSpec {
    /// A built-in column.
    Kind(ColumnKind),
    /// A column by its key: `audio.artist`.
    Key(String),
}

impl ColumnSpec {
    /// A column from its name or key in `config.toml`; `None` for a name
    /// no column has.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        ColumnKind::from_name(text)
            .map(ColumnSpec::Kind)
            .or_else(|| is_column_key(text).then(|| ColumnSpec::Key(text.to_owned())))
    }

    /// Its name or key, as `config.toml` writes it.
    pub fn name(&self) -> &str {
        match self {
            ColumnSpec::Kind(kind) => kind.name(),
            ColumnSpec::Key(key) => key,
        }
    }

    /// The built-in kind, when it is one.
    pub fn kind(&self) -> Option<ColumnKind> {
        match self {
            ColumnSpec::Kind(kind) => Some(*kind),
            ColumnSpec::Key(_) => None,
        }
    }

    /// The key, when it is not a built-in column.
    pub fn key(&self) -> Option<&str> {
        match self {
            ColumnSpec::Kind(_) => None,
            ColumnSpec::Key(key) => Some(key),
        }
    }
}

impl From<ColumnKind> for ColumnSpec {
    fn from(kind: ColumnKind) -> Self {
        ColumnSpec::Kind(kind)
    }
}

impl fmt::Display for ColumnSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for ColumnSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for ColumnSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        ColumnSpec::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("no column has the name {text}")))
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
    fn a_column_is_a_kind_or_a_key() {
        assert_eq!(
            ColumnSpec::parse(" size "),
            Some(ColumnSpec::Kind(ColumnKind::Size))
        );
        for key in [
            "audio.artist",
            "custom.project",
            "regex.episode",
            "diro-git.git",
            "a.b.c_d",
        ] {
            assert_eq!(ColumnSpec::parse(key), Some(ColumnSpec::Key(key.into())));
        }
        for bad in [
            "colour",
            "Audio.Artist",
            "audio.",
            ".x",
            "a..b",
            "a b.c",
            "",
        ] {
            assert_eq!(ColumnSpec::parse(bad), None, "{bad}");
        }
        assert!(!is_column_key(&format!("a.{}", "b".repeat(KEY_LIMIT))));
        // Written and read as its name, so a folder's view keeps it.
        let specs = vec![
            ColumnSpec::Kind(ColumnKind::Type),
            ColumnSpec::Key("audio.artist".into()),
        ];
        let json = toml::Value::try_from(&specs).unwrap();
        assert_eq!(json.to_string(), "[\"type\", \"audio.artist\"]");
        let back: Vec<ColumnSpec> = json.try_into().unwrap();
        assert_eq!(back, specs);
        assert!(is_regex_column_name("episode"));
        assert!(!is_regex_column_name("Episode"));
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
