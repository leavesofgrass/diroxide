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
use diroxide_core::place::PathStyle;
use diroxide_core::{Attributes, EntryKind, Place, Timestamp, UtcOffset};

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

// Change owner (R30's commit 79; R29 4.3; R11 sections 7 and 8): the
// words of the plan, its question, its refusals and its failures. Unix
// only; Windows shows the owner and hands off to its own Security dialog.

/// Unix and macOS folders whose owner and group are never changed, nor
/// anything inside them (R11 section 7): the system's own.
const OWNER_PROTECTED_UNIX: [&str; 18] = [
    "/Applications",
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/lib",
    "/lib32",
    "/lib64",
    "/proc",
    "/root",
    "/run",
    "/sbin",
    "/sys",
    "/usr",
    "/var",
    "/System",
    "/Library",
    "/private",
];

/// Where a system's temporary files live, inside which owners may change:
/// macOS keeps each user's under `/var/folders` (and `/private/var/folders`,
/// the same place), which the protected `/var` and `/private` would cover.
/// The roots themselves stay protected.
const OWNER_UNPROTECTED_INSIDE_UNIX: [&str; 6] = [
    "/tmp",
    "/var/tmp",
    "/var/folders",
    "/private/tmp",
    "/private/var/tmp",
    "/private/var/folders",
];

/// Whether a place is inside one of the per-user temporary roots (not the
/// root itself): a user's own files live there, however system-like the
/// folder above looks, so neither the owner rules nor the task rules
/// treat them as system folders.
pub(crate) fn inside_user_temp(place: &Place) -> bool {
    OWNER_UNPROTECTED_INSIDE_UNIX
        .iter()
        .filter_map(|path| Place::parse_with(path, PathStyle::Unix).ok())
        .any(|root| place.is_within(&root) && !root.is_within(place))
}

/// Folders protected as themselves only, not what is inside: the
/// folders that hold every user's home.
const OWNER_PROTECTED_ITSELF: [(&str, PathStyle); 3] = [
    ("/home", PathStyle::Unix),
    ("/Users", PathStyle::Unix),
    (r"C:\Users", PathStyle::Windows),
];

/// Windows folders whose owner is never changed, nor anything inside.
const OWNER_PROTECTED_WINDOWS: [&str; 3] = [
    r"C:\Windows",
    r"C:\Program Files",
    r"C:\Program Files (x86)",
];

/// Why the owner and group of a place are never changed: the top of a
/// drive, or a system folder or anything inside one. `None` for any
/// other place.
pub fn owner_protection(place: &Place) -> Option<&'static str> {
    place.local_path()?;
    if place.is_root() {
        return Some("the top of a drive is protected");
    }
    if inside_user_temp(place) {
        return None;
    }
    let within = |list: &[&str], style: PathStyle| {
        list.iter()
            .filter_map(|path| Place::parse_with(path, style).ok())
            .any(|protected| place.is_within(&protected))
    };
    let itself = OWNER_PROTECTED_ITSELF
        .iter()
        .filter_map(|(path, style)| Place::parse_with(path, *style).ok())
        .any(|protected| place.is_within(&protected) && protected.is_within(place));
    // System Volume Information, at the top of any drive.
    let volume_information = place.parent().is_some_and(|parent| parent.is_root())
        && place
            .name()
            .is_some_and(|name| name.eq_ignore_ascii_case("System Volume Information"));
    (within(&OWNER_PROTECTED_UNIX, PathStyle::Unix)
        || within(&OWNER_PROTECTED_WINDOWS, PathStyle::Windows)
        || itself
        || volume_information)
        .then_some("system folders are protected")
}

/// What an owner change changes, for its words: the owner, the group, or
/// both, by the names given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerWords<'a> {
    /// The new owner's name or number.
    pub owner: Option<&'a str>,
    /// The new group's name or number.
    pub group: Option<&'a str>,
}

impl OwnerWords<'_> {
    /// "change the owner of", or "change the group of" when only the
    /// group changes.
    fn verb(&self) -> &'static str {
        if self.owner.is_some() {
            "change the owner of"
        } else {
            "change the group of"
        }
    }

    /// A plan line's change: "owner becomes ada", "group becomes staff",
    /// "owner becomes ada, group becomes staff".
    pub fn change(&self) -> String {
        match (self.owner, self.group) {
            (Some(owner), Some(group)) => format!("owner becomes {owner}, group becomes {group}"),
            (Some(owner), None) => format!("owner becomes {owner}"),
            (None, Some(group)) => format!("group becomes {group}"),
            (None, None) => "no change".to_owned(),
        }
    }

    /// The question the app asks, starting on No: "Change owner of 3
    /// items to root?"; "Change group of 1 item to staff?"; "Change owner
    /// of 3 items to root, group to wheel?".
    pub fn question(&self, items: u64) -> String {
        let items = count(items, "item", "items");
        match (self.owner, self.group) {
            (Some(owner), Some(group)) => {
                format!("Change owner of {items} to {owner}, group to {group}?")
            }
            (Some(owner), None) => format!("Change owner of {items} to {owner}?"),
            (None, Some(group)) => format!("Change group of {items} to {group}?"),
            (None, None) => format!("Change nothing about {items}?"),
        }
    }

    /// The refusal for a protected place: "Could not change the owner of
    /// /usr: system folders are protected. Use the system's own tools."
    pub fn protected(&self, name: &str, why: &str) -> String {
        format!(
            "Could not {} {name}: {why}. Use the system's own tools.",
            self.verb()
        )
    }

    /// The refusal when only an administrator could: "Could not change the
    /// owner of a.txt: only an administrator can. Start diro as
    /// administrator, then try again."
    pub fn administrator_only(&self, name: &str) -> String {
        format!(
            "Could not {} {name}: only an administrator can. Start diro as administrator, then try again.",
            self.verb()
        )
    }

    /// The refusal for a group the caller is not in: "Could not change the
    /// group of a.txt: you are not in group staff. Choose one of your
    /// groups."
    pub fn not_your_group(name: &str, group: &str) -> String {
        format!(
            "Could not change the group of {name}: you are not in group {group}. Choose one of your groups."
        )
    }

    /// The refusal where the place keeps no owners, or on Windows, which
    /// changes owners in its own Security dialog.
    pub fn not_here(&self, name: &str, windows: bool) -> String {
        if windows {
            format!(
                "Could not {} {name}: Windows does it in its Security dialog. Open the Security dialog instead.",
                self.verb()
            )
        } else {
            let what = if self.owner.is_some() {
                "owners"
            } else {
                "groups"
            };
            format!(
                "Could not {} {name}: this place keeps no {what}.",
                self.verb()
            )
        }
    }

    /// The refusal for a name this computer does not know: "Could not
    /// change the owner of a.txt: there is no user named bob here. Type a
    /// user name or a number."
    pub fn unknown(&self, name: &str, who: &str, group: bool) -> String {
        let (kind, verb) = if group {
            ("group", "change the group of")
        } else {
            ("user", "change the owner of")
        };
        format!(
            "Could not {verb} {name}: there is no {kind} named {who} here. Type a {kind} name or a number."
        )
    }

    /// A failure in the job's report, in words: an owner change the system
    /// refused needs an administrator; a group change it refused needs one
    /// of the caller's groups (or the item's owner); anything else says
    /// the provider's own cause.
    pub fn failure(
        &self,
        name: &str,
        error: &diroxide_vfs::VfsError,
        in_group: Option<bool>,
    ) -> String {
        use diroxide_vfs::VfsError;
        let refused = matches!(
            error,
            VfsError::PermissionDenied | VfsError::NeedsPrivilege { .. }
        );
        match (refused, self.owner, self.group, in_group) {
            (true, Some(_), _, _) => self.administrator_only(name),
            (true, None, Some(group), Some(false)) => Self::not_your_group(name, group),
            (true, None, Some(_), _) => format!(
                "Could not change the group of {name}: only its owner can. Ask the owner, or start diro as administrator."
            ),
            _ => format!("Could not {} {name}: {error}.", self.verb()),
        }
    }
}

/// The line an undo plan says when putting owners back needs an
/// administrator, and the caller is not one: any old owner, or an old
/// group the caller is not in. `None` when the undo needs nothing more.
pub fn undo_owner_line(
    inverses: &[(Place, diroxide_vfs::AttributeChange)],
    caller: &diroxide_vfs::access::Caller,
) -> Option<&'static str> {
    if caller.administrator {
        return None;
    }
    let needs = inverses.iter().any(|(_, inverse)| {
        inverse.owner.is_some()
            || inverse.group.as_ref().is_some_and(|group| {
                group
                    .id
                    .parse::<u32>()
                    .ok()
                    .and_then(|gid| caller.in_group(gid))
                    != Some(true)
            })
    });
    needs.then_some(UNDO_NEEDS_ADMINISTRATOR)
}

/// The undo line for owners put back: what it needs, first.
pub const UNDO_NEEDS_ADMINISTRATOR: &str = "Undo needs an administrator: it puts the owners back.";

/// What the Change owner command does at a place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OwnerCommand {
    /// The app's own dialog: the place keeps owners or groups (Unix,
    /// SFTP by number).
    Dialog,
    /// Windows: the owner is shown, and the command hands off to the
    /// system's own Security dialog.
    SecurityDialog,
    /// The place keeps no owners: the command is unavailable, with
    /// [`OWNER_NOT_HERE`] as its reason.
    NotHere,
}

/// Which Change owner the app offers at a place: its own dialog where
/// the capabilities say owners or groups can change, Windows' Security
/// dialog on the local disk of Windows, else none.
pub fn owner_command(capabilities: &diroxide_vfs::Capabilities, scheme: &str) -> OwnerCommand {
    let access = capabilities.access;
    if access.owner.is_possible() || access.group.is_possible() {
        OwnerCommand::Dialog
    } else if cfg!(windows) && scheme == "file" {
        OwnerCommand::SecurityDialog
    } else {
        OwnerCommand::NotHere
    }
}

/// Why Change owner is unavailable at a place that keeps no owners.
pub const OWNER_NOT_HERE: &str = "Change owner: this place keeps no owners";

/// The hand-off's line while Windows opens its dialog: "Opening the
/// Security dialog for report.pdf".
pub fn security_dialog_line(name: &str) -> String {
    format!("Opening the Security dialog for {name}")
}

/// The hand-off's failure, with what to do instead (Microsoft marks the
/// call that opens the dialog as one that may change or go away).
pub const SECURITY_DIALOG_FAILED: &str = "Could not open the Security dialog: Windows did not open it. In File Explorer, open Properties, then Security.";

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

    #[test]
    fn system_folders_and_drive_tops_keep_their_owners() {
        let unix = |text: &str| Place::parse_with(text, PathStyle::Unix).unwrap();
        let windows = |text: &str| Place::parse_with(text, PathStyle::Windows).unwrap();
        for (place, why) in [
            (unix("/"), Some("the top of a drive is protected")),
            (unix("/usr"), Some("system folders are protected")),
            (
                unix("/usr/local/bin/tool"),
                Some("system folders are protected"),
            ),
            (unix("/etc/hosts"), Some("system folders are protected")),
            (unix("/home"), Some("system folders are protected")),
            (unix("/home/ada/notes.txt"), None),
            (unix("/Users/ada"), None),
            (unix("/srv/www"), None),
            (
                unix("/var/log/syslog"),
                Some("system folders are protected"),
            ),
            (unix("/var/tmp"), Some("system folders are protected")),
            (unix("/var/tmp/build/a.out"), None),
            (
                unix("/var/folders/h5/t85nd63d71x27_8gl5gw3l_w0000gn/T/app-it/work/notes.txt"),
                None,
            ),
            (unix("/private/var/folders/h5/x/T/notes.txt"), None),
            (unix("/tmp/notes.txt"), None),
            (windows(r"D:\"), Some("the top of a drive is protected")),
            (
                windows(r"c:\windows\system32"),
                Some("system folders are protected"),
            ),
            (
                windows(r"C:\Program Files\App"),
                Some("system folders are protected"),
            ),
            (windows(r"C:\Users"), Some("system folders are protected")),
            (windows(r"C:\Users\ada"), None),
            (
                windows(r"E:\System Volume Information"),
                Some("system folders are protected"),
            ),
            (windows(r"D:\data"), None),
        ] {
            assert_eq!(owner_protection(&place), why, "{place}");
        }
    }

    #[test]
    fn owner_changes_are_said_meaning_first_within_40_cells() {
        let owner = OwnerWords {
            owner: Some("root"),
            group: None,
        };
        let group = OwnerWords {
            owner: None,
            group: Some("staff"),
        };
        let both = OwnerWords {
            owner: Some("root"),
            group: Some("wheel"),
        };
        assert_eq!(owner.question(3), "Change owner of 3 items to root?");
        assert_eq!("Change owner of 3 items to root?".len(), 32);
        assert_eq!(group.question(1), "Change group of 1 item to staff?");
        assert_eq!(
            both.question(3),
            "Change owner of 3 items to root, group to wheel?"
        );
        assert_eq!(
            group_row(3, EntryKind::File, &owner.change()),
            "3 files: owner becomes root"
        );
        assert_eq!(both.change(), "owner becomes root, group becomes wheel");
        let lines = [
            (
                owner.administrator_only("a.txt"),
                "Could not change the owner of a.txt",
            ),
            (
                OwnerWords::not_your_group("a.txt", "staff"),
                "Could not change the group of a.txt",
            ),
            (
                owner.protected("/usr", "system folders are protected"),
                "Could not change the owner of /usr",
            ),
            (
                owner.not_here("a.txt", true),
                "Could not change the owner of a.txt",
            ),
            (
                group.not_here("a.txt", false),
                "Could not change the group of a.txt",
            ),
            (
                owner.unknown("a.txt", "bob", false),
                "Could not change the owner of a.txt",
            ),
        ];
        for (line, meaning) in &lines {
            assert!(within_cells(line, meaning), "{line}");
        }
        assert_eq!(
            lines[0].0,
            "Could not change the owner of a.txt: only an administrator can. Start diro as administrator, then try again."
        );
        assert_eq!(
            lines[2].0,
            "Could not change the owner of /usr: system folders are protected. Use the system's own tools."
        );
        assert_eq!(
            lines[3].0,
            "Could not change the owner of a.txt: Windows does it in its Security dialog. Open the Security dialog instead."
        );
        use diroxide_vfs::VfsError;
        assert_eq!(
            owner.failure("a.txt", &VfsError::PermissionDenied, None),
            lines[0].0
        );
        assert_eq!(
            group.failure("a.txt", &VfsError::PermissionDenied, Some(false)),
            "Could not change the group of a.txt: you are not in group staff. Choose one of your groups."
        );
        assert_eq!(
            group.failure("a.txt", &VfsError::PermissionDenied, Some(true)),
            "Could not change the group of a.txt: only its owner can. Ask the owner, or start diro as administrator."
        );
        assert_eq!(
            owner.failure("a.txt", &VfsError::ReadOnlyFilesystem, None),
            "Could not change the owner of a.txt: the drive is read only."
        );
        assert!(within_cells(
            UNDO_NEEDS_ADMINISTRATOR,
            "Undo needs an administrator"
        ));
        use diroxide_vfs::access::Caller;
        let back = |owner: Option<&str>, group: Option<&str>| {
            let principal = |id: &str| diroxide_vfs::Principal {
                id: id.to_owned(),
                name: None,
            };
            vec![(
                Place::parse_with("/t/a.txt", PathStyle::Unix).unwrap(),
                diroxide_vfs::AttributeChange {
                    owner: owner.map(principal),
                    group: group.map(principal),
                    ..diroxide_vfs::AttributeChange::default()
                },
            )]
        };
        let me = Caller::user(1000, vec![1000, 27]);
        assert_eq!(undo_owner_line(&back(None, Some("27")), &me), None);
        assert_eq!(
            undo_owner_line(&back(None, Some("50")), &me),
            Some(UNDO_NEEDS_ADMINISTRATOR)
        );
        assert_eq!(
            undo_owner_line(&back(Some("0"), None), &me),
            Some(UNDO_NEEDS_ADMINISTRATOR)
        );
        assert_eq!(
            undo_owner_line(&back(Some("0"), None), &Caller::administrator()),
            None
        );
    }

    #[test]
    fn change_owner_is_a_dialog_a_hand_off_or_unavailable() {
        use diroxide_vfs::{Can, Capabilities};
        let mut unix = Capabilities::read_only();
        unix.access.owner = Can::WithAdministrator;
        unix.access.group = Can::Yes;
        assert_eq!(owner_command(&unix, "file"), OwnerCommand::Dialog);
        let none = Capabilities::read_only();
        assert_eq!(
            owner_command(&none, "file"),
            if cfg!(windows) {
                OwnerCommand::SecurityDialog
            } else {
                OwnerCommand::NotHere
            }
        );
        assert_eq!(owner_command(&none, "s3"), OwnerCommand::NotHere);
        assert_eq!(
            security_dialog_line("report.pdf"),
            "Opening the Security dialog for report.pdf"
        );
        assert!(within_cells(
            SECURITY_DIALOG_FAILED,
            "Could not open the Security dialog"
        ));
        assert_eq!(OWNER_NOT_HERE.len(), 40);
        assert!(within_cells(
            OWNER_NOT_HERE,
            "Change owner: this place keeps no"
        ));
    }
}
