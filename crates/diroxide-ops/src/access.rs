//! Permissions, attributes and times as a job (plan section 5.4;
//! research/permissions.md 6 and 7; research/alpha2-plan.md 3.7 and
//! commit 27).
//!
//! An [`AttributeEdit`] says what changes (the Unix mode by bits or
//! exactly, Windows attributes, the modified, created and accessed times)
//! and where ([`Recursion`], listed first in the plan's rows). The plan
//! says the result in words first, the octal second: "Owner can read and
//! write, group can read, others can read (644)"; several entries are
//! grouped by the change ("12 files: group gains read"). In a change that
//! goes inside folders, a folder whose class gains read gains enter too, a
//! file gains run only where someone could already run it, and no file
//! gains setuid. Links are never changed: a selected link is refused, a
//! link inside is skipped. Each step carries the attributes the plan saw,
//! so an entry that changed since is skipped, never overwritten; the
//! inverse the provider returns goes into the undo record.

use diroxide_core::format::count;
use diroxide_core::{Attributes, EntryKind, Timestamp, UtcOffset};

/// The Windows read-only attribute.
pub const READ_ONLY: u32 = 0x1;
/// The Windows hidden attribute.
pub const HIDDEN: u32 = 0x2;
/// The Windows system attribute.
pub const SYSTEM: u32 = 0x4;
/// The Windows archive attribute, "marked for backup".
pub const ARCHIVE: u32 = 0x20;

const SETUID: u32 = 0o4000;
const STICKY: u32 = 0o1000;

/// Which entries a change applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Recursion {
    /// The selected entries only.
    #[default]
    ThisItem,
    /// The selected entries and everything inside them.
    FilesAndFolders,
    /// The selected entries and the files inside them.
    FilesOnly,
    /// The selected entries and the folders inside them.
    FoldersOnly,
}

impl Recursion {
    /// The choice in words: "this item only", "files and folders inside".
    pub const fn words(self) -> &'static str {
        match self {
            Recursion::ThisItem => "this item only",
            Recursion::FilesAndFolders => "files and folders inside",
            Recursion::FilesOnly => "only files inside",
            Recursion::FoldersOnly => "only folders inside",
        }
    }

    /// The plan's first row: "Apply to: files and folders inside".
    pub fn row(self) -> String {
        format!("Apply to: {}", self.words())
    }

    /// Whether the change goes inside folders.
    pub const fn goes_inside(self) -> bool {
        !matches!(self, Recursion::ThisItem)
    }

    /// Whether an entry inside a selected folder is changed.
    pub(crate) fn takes(self, kind: EntryKind) -> bool {
        match self {
            Recursion::ThisItem => false,
            Recursion::FilesAndFolders => true,
            Recursion::FilesOnly => kind != EntryKind::Folder,
            Recursion::FoldersOnly => kind == EntryKind::Folder,
        }
    }
}

/// A change of a Unix mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeChange {
    /// Turn these bits on and these off, the rest as they are: the
    /// dialog's rows.
    Bits {
        /// Bits turned on.
        on: u32,
        /// Bits turned off.
        off: u32,
    },
    /// Set exactly, files and folders each their own mode: a preset such
    /// as "Owner edits, everyone reads" (644 and 755).
    Exact {
        /// The mode of files.
        files: u32,
        /// The mode of folders.
        folders: u32,
    },
}

impl ModeChange {
    /// The new mode of an entry whose mode is `current`. `inside` is true
    /// for an entry inside a selected folder, where the X convention
    /// holds: a folder whose class gains read gains enter, a file gains
    /// run only where someone could already run it, and no file gains
    /// setuid.
    pub fn apply(self, current: u32, kind: EntryKind, inside: bool) -> u32 {
        let folder = kind == EntryKind::Folder;
        match self {
            ModeChange::Exact { files, folders } => (if folder { folders } else { files }) & 0o7777,
            ModeChange::Bits { on, off } => {
                let mut on = on & 0o7777;
                if inside {
                    if folder {
                        for shift in [6, 3, 0] {
                            if on & (0o4 << shift) != 0 {
                                on |= 0o1 << shift;
                            }
                        }
                    } else {
                        if current & 0o111 == 0 {
                            on &= !0o111;
                        }
                        on &= !SETUID;
                    }
                }
                ((current & !off) | on) & 0o7777
            }
        }
    }
}

/// Windows attributes turned on and off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowsChange {
    /// Attributes turned on.
    pub on: u32,
    /// Attributes turned off.
    pub off: u32,
}

impl WindowsChange {
    /// The new attributes of an entry whose attributes are `current`.
    pub fn apply(self, current: u32) -> u32 {
        (current & !self.off) | self.on
    }

    /// The change in words, one row each: "Turns on read-only", "Turns off
    /// hidden".
    pub fn rows(self) -> Vec<String> {
        let words = [
            (READ_ONLY, "read-only"),
            (HIDDEN, "hidden"),
            (SYSTEM, "system"),
            (ARCHIVE, "marked for backup"),
        ];
        let on = words
            .iter()
            .filter(|(bit, _)| self.on & bit != 0)
            .map(|(_, word)| format!("Turns on {word}"));
        let off = words
            .iter()
            .filter(|(bit, _)| self.off & bit != 0)
            .map(|(_, word)| format!("Turns off {word}"));
        on.chain(off).collect()
    }
}

/// What a permissions, attributes or times job changes, and where.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttributeEdit {
    /// Which entries: listed first in the plan.
    pub recursion: Recursion,
    /// The Unix mode.
    pub mode: Option<ModeChange>,
    /// The Windows attributes.
    pub windows: Option<WindowsChange>,
    /// A new modified time.
    pub modified: Option<Timestamp>,
    /// A new created time, where the place can set one.
    pub created: Option<Timestamp>,
    /// A new accessed time.
    pub accessed: Option<Timestamp>,
    /// The offset times are said in: the app's local offset.
    pub offset: UtcOffset,
}

impl AttributeEdit {
    /// An edit that changes nothing yet, applied as `recursion` says.
    pub fn new(recursion: Recursion) -> Self {
        Self {
            recursion,
            ..Self::default()
        }
    }

    /// The same edit changing the Unix mode.
    pub fn with_mode(mut self, mode: ModeChange) -> Self {
        self.mode = Some(mode);
        self
    }

    /// The same edit changing Windows attributes.
    pub fn with_windows(mut self, windows: WindowsChange) -> Self {
        self.windows = Some(windows);
        self
    }

    /// The same edit setting the modified time.
    pub fn with_modified(mut self, time: Timestamp) -> Self {
        self.modified = Some(time);
        self
    }

    /// The same edit setting the created time.
    pub fn with_created(mut self, time: Timestamp) -> Self {
        self.created = Some(time);
        self
    }

    /// The same edit setting the accessed time.
    pub fn with_accessed(mut self, time: Timestamp) -> Self {
        self.accessed = Some(time);
        self
    }

    /// The same edit saying times at this offset.
    pub fn at_offset(mut self, offset: UtcOffset) -> Self {
        self.offset = offset;
        self
    }

    /// Whether the edit changes nothing.
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.windows.is_none()
            && self.modified.is_none()
            && self.created.is_none()
            && self.accessed.is_none()
    }

    /// Whether it changes only times.
    pub fn times_only(&self) -> bool {
        self.mode.is_none() && self.windows.is_none()
    }
}

/// A mode in words first, the octal second: "Owner can read and write,
/// group can read, others can read (644)". The special bits follow the
/// classes ("Runs as its owner").
pub fn mode_line(mode: u32, kind: EntryKind) -> String {
    let attributes = Attributes::Unix(mode & 0o7777);
    let words = attributes.words(kind);
    format!("{} ({})", words.trim_end_matches('.'), attributes.octal())
}

/// The rights of a class's bits, in words.
fn rights(bits: u32, folder: bool) -> Vec<&'static str> {
    let names: [(u32, &str); 3] = if folder {
        [(0o4, "list"), (0o2, "change"), (0o1, "enter")]
    } else {
        [(0o4, "read"), (0o2, "write"), (0o1, "run")]
    };
    names
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| *name)
        .collect()
}

fn and_list(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A change of mode in words, the change first: "group gains write",
/// "others lose list and enter", "everyone gains run", "turns on run as
/// owner (setuid)". "no change" when the modes are the same.
pub fn change_words(before: u32, after: u32, kind: EntryKind) -> String {
    let folder = kind == EntryKind::Folder;
    let classes = [
        ("owner", 6, "gains", "loses"),
        ("group", 3, "gains", "loses"),
        ("others", 0, "gain", "lose"),
    ];
    let gained: Vec<u32> = classes
        .iter()
        .map(|(_, shift, _, _)| (after & !before) >> shift & 0o7)
        .collect();
    let lost: Vec<u32> = classes
        .iter()
        .map(|(_, shift, _, _)| (before & !after) >> shift & 0o7)
        .collect();
    let mut clauses = Vec::new();
    for (changes, gain) in [(&gained, true), (&lost, false)] {
        if changes.iter().all(|bits| *bits != 0 && *bits == changes[0]) {
            let verb = if gain { "gains" } else { "loses" };
            clauses.push(format!(
                "everyone {verb} {}",
                and_list(&rights(changes[0], folder))
            ));
            continue;
        }
        for ((who, _, gains, loses), bits) in classes.iter().zip(changes.iter()) {
            if *bits != 0 {
                let verb = if gain { gains } else { loses };
                clauses.push(format!("{who} {verb} {}", and_list(&rights(*bits, folder))));
            }
        }
    }
    let special = [
        (
            0o4000,
            if folder {
                "the setuid mark"
            } else {
                "run as owner (setuid)"
            },
        ),
        (
            0o2000,
            if folder {
                "new items get its group (setgid)"
            } else {
                "run as group (setgid)"
            },
        ),
        (STICKY, "only owners delete inside (sticky)"),
    ];
    for (bit, word) in special {
        if after & bit != 0 && before & bit == 0 {
            clauses.push(format!("turns on {word}"));
        }
        if before & bit != 0 && after & bit == 0 {
            clauses.push(format!("turns off {word}"));
        }
    }
    if clauses.is_empty() {
        "no change".to_owned()
    } else {
        clauses.join(", ")
    }
}

/// A time as the plan's rows say it: "2026-10-10 22:09".
pub fn time_words(time: Timestamp, offset: UtcOffset) -> String {
    let civil = time.to_civil(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        civil.year, civil.month, civil.day, civil.hour, civil.minute
    )
}

/// The rows of a group of entries with one change: "12 files: group gains
/// read", "1 folder: others lose list and enter".
pub fn group_row(entries: u64, kind: EntryKind, change: &str) -> String {
    let noun = if kind == EntryKind::Folder {
        count(entries, "folder", "folders")
    } else {
        count(entries, "file", "files")
    };
    format!("{noun}: {change}")
}

/// What an attribute plan knows beyond its steps: the edit, and its rows
/// in words, the recursion first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttributeFacts {
    /// The edit.
    pub edit: AttributeEdit,
    /// The rows: "Apply to: this item only", then the change in words.
    pub rows: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    /// The words-function table: each mode with its line, on a file or a
    /// folder, meaning first and the octal second.
    #[test]
    fn modes_are_said_in_words_first() {
        let table = [
            (
                0o644,
                EntryKind::File,
                "Owner can read and write, group can read, others can read (644)",
            ),
            (
                0o640,
                EntryKind::File,
                "Owner can read and write, group can read, others have no access (640)",
            ),
            (
                0o600,
                EntryKind::File,
                "Owner can read and write, group has no access, others have no access (600)",
            ),
            (
                0o755,
                EntryKind::Folder,
                "Owner can list, enter and change contents, group can list and enter, others can list and enter (755)",
            ),
            (
                0o000,
                EntryKind::File,
                "Owner has no access, group has no access, others have no access (000)",
            ),
            (
                0o4755,
                EntryKind::File,
                "Owner can read, write and run, group can read and run, others can read and run. Runs as its owner (4755)",
            ),
        ];
        for (mode, kind, line) in table {
            assert_eq!(mode_line(mode, kind), line, "{mode:o}");
        }
        assert!(within_cells(
            &mode_line(0o640, EntryKind::File),
            "Owner can read and write, group can read"
        ));
    }

    #[test]
    fn changes_are_said_change_first() {
        let table = [
            (0o644, 0o664, EntryKind::File, "group gains write"),
            (
                0o755,
                0o750,
                EntryKind::Folder,
                "others lose list and enter",
            ),
            (0o644, 0o755, EntryKind::File, "everyone gains run"),
            (
                0o666,
                0o644,
                EntryKind::File,
                "group loses write, others lose write",
            ),
            (
                0o755,
                0o4755,
                EntryKind::File,
                "turns on run as owner (setuid)",
            ),
            (
                0o1777,
                0o777,
                EntryKind::Folder,
                "turns off only owners delete inside (sticky)",
            ),
            (0o644, 0o644, EntryKind::File, "no change"),
            (0o600, 0o640, EntryKind::File, "group gains read"),
        ];
        for (before, after, kind, words) in table {
            assert_eq!(
                change_words(before, after, kind),
                words,
                "{before:o} to {after:o}"
            );
        }
        assert_eq!(
            group_row(12, EntryKind::File, "group gains read"),
            "12 files: group gains read"
        );
        assert_eq!(
            group_row(1, EntryKind::Folder, "others lose list and enter"),
            "1 folder: others lose list and enter"
        );
    }

    #[test]
    fn inside_folders_the_x_convention_holds() {
        let add_read = ModeChange::Bits { on: 0o044, off: 0 };
        assert_eq!(
            add_read.apply(0o700, EntryKind::Folder, true),
            0o755,
            "folders stay enterable"
        );
        assert_eq!(add_read.apply(0o600, EntryKind::File, true), 0o644);
        let add_run = ModeChange::Bits { on: 0o111, off: 0 };
        assert_eq!(
            add_run.apply(0o644, EntryKind::File, true),
            0o644,
            "files keep their run setting"
        );
        assert_eq!(add_run.apply(0o744, EntryKind::File, true), 0o755);
        assert_eq!(
            add_run.apply(0o644, EntryKind::File, false),
            0o755,
            "the item itself does"
        );
        let setuid = ModeChange::Bits { on: 0o4000, off: 0 };
        assert_eq!(
            setuid.apply(0o755, EntryKind::File, true),
            0o755,
            "no mass setuid"
        );
        assert_eq!(setuid.apply(0o755, EntryKind::File, false), 0o4755);
        let preset = ModeChange::Exact {
            files: 0o644,
            folders: 0o755,
        };
        assert_eq!(preset.apply(0o600, EntryKind::File, true), 0o644);
        assert_eq!(preset.apply(0o700, EntryKind::Folder, false), 0o755);
        assert_eq!(
            ModeChange::Bits { on: 0, off: 0o022 }.apply(0o2775, EntryKind::Folder, false),
            0o2755,
            "setgid stays"
        );
    }

    #[test]
    fn windows_changes_and_times_have_their_words() {
        let change = WindowsChange {
            on: HIDDEN,
            off: READ_ONLY,
        };
        assert_eq!(change.apply(READ_ONLY | ARCHIVE), HIDDEN | ARCHIVE);
        assert_eq!(
            change.rows(),
            vec!["Turns on hidden", "Turns off read-only"]
        );
        let time = Timestamp::from_civil(2026, 10, 10, 22, 9, 0, UtcOffset::UTC).unwrap();
        assert_eq!(time_words(time, UtcOffset::UTC), "2026-10-10 22:09");
        assert_eq!(
            Recursion::FilesAndFolders.row(),
            "Apply to: files and folders inside"
        );
        assert!(AttributeEdit::new(Recursion::ThisItem).is_empty());
        assert!(
            AttributeEdit::new(Recursion::ThisItem)
                .with_modified(time)
                .times_only()
        );
    }
}
