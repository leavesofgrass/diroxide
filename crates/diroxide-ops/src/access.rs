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

// The access details view in words (alpha.7, R30's commits 74 to 77):
// the rows a provider's access facts read as.
mod rows;
pub use rows::{
    FULL_CONTROL, LINK_HAS_NONE, MASK_HELP, MASK_NOTE, NO_PERMISSIONS, RightsWords, YOU_CAN_HELP,
    access_rows, acl_entry_line, applies_to, canonical, label_words, map_generic, mask_line,
    merged_entries, rights_phrase, special_words, windows_entry_line, windows_rights, you_can_row,
};

// The edit's types moved into core with P2b (W2c's request 3), so a job
// request carries them; they are re-exported here under their old paths.
pub use diroxide_core::access::{
    ARCHIVE, AttributeEdit, HIDDEN, ModeChange, READ_ONLY, Recursion, SYSTEM, WindowsChange,
};

const STICKY: u32 = 0o1000;

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

/// A time as the plan's rows and the time dialog say it, the weekday
/// first and computed from the date, never typed (the owner's rule on
/// dates): "Saturday, 2026-10-10 22:09".
pub fn time_words(time: Timestamp, offset: UtcOffset) -> String {
    let civil = time.to_civil(offset);
    format!(
        "{}, {:04}-{:02}-{:02} {:02}:{:02}",
        civil.weekday.name(),
        civil.year,
        civil.month,
        civil.day,
        civil.hour,
        civil.minute
    )
}

/// One of an entry's times, as the time dialog lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeKind {
    /// When its contents last changed: settable everywhere.
    Modified,
    /// When it was last read: settable everywhere, kept to the day on FAT.
    Accessed,
    /// When it was made: settable on Windows and macOS only.
    Created,
    /// When its details last changed (the Unix change time): the system
    /// sets it, never a program.
    Changed,
}

impl TimeKind {
    /// The label, said first: "Modified".
    pub const fn label(self) -> &'static str {
        match self {
            TimeKind::Modified => "Modified",
            TimeKind::Accessed => "Accessed",
            TimeKind::Created => "Created",
            TimeKind::Changed => "Changed",
        }
    }
}

/// The refusal when the change time is chosen to be set (R11 sections 7
/// and 8).
pub const CHANGE_TIME_REFUSAL: &str =
    "Could not set the change time: the system sets it. Set the modified time instead.";

/// The time dialog's row for one time: "Modified: Saturday, 2026-10-10
/// 22:09"; "Created: not settable on this system"; "Changed: set by the
/// system"; "Accessed: not known".
pub fn time_row(
    kind: TimeKind,
    time: Option<Timestamp>,
    offset: UtcOffset,
    settable: bool,
) -> String {
    let label = kind.label();
    if kind == TimeKind::Changed {
        return format!("{label}: set by the system");
    }
    if !settable {
        return format!("{label}: not settable on this system");
    }
    match time {
        Some(time) => format!("{label}: {}", time_words(time, offset)),
        None => format!("{label}: not known"),
    }
}

/// What the disk kept of a time that was set, read back after the job:
/// `None` when it kept the time asked for; "Modified time rounded by the
/// disk" when it kept one within two days of it (FAT keeps the modified
/// time to two seconds, the accessed time to the day, in local time);
/// "Modified time not kept by the disk" when it kept another or none.
pub fn rounded_line(kind: TimeKind, asked: Timestamp, kept: Option<Timestamp>) -> Option<String> {
    /// Two days: a day of rounding and a day of time zone.
    const NEAR: i64 = 2 * 24 * 60 * 60;
    let label = kind.label();
    match kept {
        Some(kept) if kept == asked => None,
        Some(kept) if (kept.unix_seconds() - asked.unix_seconds()).abs() <= NEAR => {
            Some(format!("{label} time rounded by the disk"))
        }
        _ => Some(format!("{label} time not kept by the disk")),
    }
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
        assert_eq!(
            time_words(time, UtcOffset::UTC),
            "Saturday, 2026-10-10 22:09"
        );
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

    #[test]
    fn times_say_their_weekday_from_the_date_and_what_the_disk_kept() {
        // The weekdays checked against Python's calendar on the machine.
        for (y, m, d, words) in [
            (2026, 9, 21, "Monday, 2026-09-21 15:13"),
            (2026, 11, 7, "Saturday, 2026-11-07 15:13"),
            (2000, 2, 29, "Tuesday, 2000-02-29 15:13"),
            (1969, 12, 31, "Wednesday, 1969-12-31 15:13"),
        ] {
            let time = Timestamp::from_civil(y, m, d, 15, 13, 0, UtcOffset::UTC).unwrap();
            assert_eq!(time_words(time, UtcOffset::UTC), words);
        }
        let time = Timestamp::from_civil(2026, 10, 10, 22, 9, 0, UtcOffset::UTC).unwrap();
        let row = time_row(TimeKind::Modified, Some(time), UtcOffset::UTC, true);
        assert_eq!(row, "Modified: Saturday, 2026-10-10 22:09");
        assert_eq!(row.len(), 36);
        let created = time_row(TimeKind::Created, None, UtcOffset::UTC, false);
        assert_eq!(created, "Created: not settable on this system");
        assert_eq!(
            time_row(TimeKind::Changed, Some(time), UtcOffset::UTC, false),
            "Changed: set by the system"
        );
        assert_eq!(
            time_row(TimeKind::Accessed, None, UtcOffset::UTC, true),
            "Accessed: not known"
        );
        assert!(within_cells(
            CHANGE_TIME_REFUSAL,
            "Could not set the change time"
        ));
        let rounded = Timestamp::from_unix(time.unix_seconds() + 1, 0);
        assert_eq!(rounded_line(TimeKind::Modified, time, Some(time)), None);
        let line = rounded_line(TimeKind::Modified, time, Some(rounded));
        assert_eq!(line.as_deref(), Some("Modified time rounded by the disk"));
        assert_eq!("Modified time rounded by the disk".len(), 33);
        let far = Timestamp::from_unix(time.unix_seconds() - 30 * 86_400, 0);
        assert_eq!(
            rounded_line(TimeKind::Accessed, time, Some(far)).as_deref(),
            Some("Accessed time not kept by the disk")
        );
        assert_eq!(
            rounded_line(TimeKind::Created, time, None).as_deref(),
            Some("Created time not kept by the disk")
        );
    }
}
