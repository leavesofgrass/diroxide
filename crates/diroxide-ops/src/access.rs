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
