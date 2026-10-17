//! The message catalog: the sentences the contract fixes, in one place, so
//! every frontend says them the same way and the Braille walk test reads
//! them all. Each puts its meaning in its first 40 cells.
//!
//! A sentence that names a key takes the key as an argument, from the live
//! keymap, never as fixed text: a rebound key would make it lie.
//!
//! Later briefs add their sentences where they are made (the conflict and
//! progress lines in `diroxide-ops`, the menu and palette names in the
//! app's command catalog); these are the safety sentences of plan section
//! 5.12 and the secret prompt's words (research/encryption.md).

use crate::format::count;
use crate::message::Message;

/// The loud failure after a trash call whose post-check could not find the
/// item in the trash: the item is gone, and not where the user can restore
/// it (spikes four and six).
pub const TRASH_NOT_CONFIRMED: &str = "Deleted but not found in the trash.";

/// The description line under every permanent-delete confirmation, the
/// honest sentence of research/encryption.md.
pub const PERMANENT_DELETE_NOTE: &str =
    "Deleted data may stay on the disk. diroxide cannot erase it for good.";

/// Refused before running a program or a shell in a folder of 259
/// characters or more (spike four).
pub const PROGRAM_TOO_DEEP: &str = "Cannot start a program in a folder this deep";

/// Refused before opening a file with the system opener at depth (spike
/// four); a crate's own error text is never shown.
pub const OPEN_TOO_DEEP: &str = "Cannot open a file this deep";

/// Said when the parent's watch reports the shown folder removed or
/// renamed (spike three); the panel then goes up.
pub const FOLDER_GONE: &str = "Folder is gone";

/// The masked prompt's Show toggle, turned on.
pub const SECRET_SHOWN: &str =
    "Showing the password. Anyone who can see or hear your screen can read it.";

/// The masked prompt's Show toggle, turned off.
pub const SECRET_HIDDEN: &str = "Hidden.";

/// A new secret typed twice, differently.
pub const SECRETS_DIFFER: &str = "The two passwords differ. Nothing was changed. Try again.";

/// A new secret left empty.
pub const SECRET_EMPTY: &str = "A password cannot be empty.";

/// The question before a permanent delete: "Delete 3 items for good? y or
/// n". The listing follows it; the answer starts on No.
pub fn delete_for_good(items: u64) -> Message {
    Message::question(&format!(
        "Delete {} for good",
        count(items, "item", "items")
    ))
}

/// The question where the place has no trash (a removable or network
/// drive on Windows): "This drive has no trash. Delete 3 items for good? y
/// or n". The answer starts on No.
pub fn no_trash_here(items: u64) -> Message {
    Message::question(&format!(
        "This drive has no trash. Delete {} for good",
        count(items, "item", "items")
    ))
}

/// The question where a path is too long for the trash (260 UTF-16 units
/// or more on Windows): "Path is too long for the trash. Delete 1 item for
/// good? y or n".
pub fn too_long_for_trash(items: u64) -> Message {
    Message::question(&format!(
        "Path is too long for the trash. Delete {} for good",
        count(items, "item", "items")
    ))
}

/// The question where the trash would copy an item to another drive (a
/// Linux drive whose own trash cannot be made): "Trashing copies 3.2 GB to
/// another drive. Go on? y or n". The answer starts on No; No leaves the
/// item where it is.
pub fn trash_copies_across(bytes: u64) -> Message {
    Message::question(&format!(
        "Trashing copies {} to another drive. Go on",
        crate::format::human_size(bytes)
    ))
}

/// After a trash job: "Moved 3 items to the trash. They can be restored."
pub fn moved_to_trash(items: u64) -> Message {
    let they = if items == 1 { "It" } else { "They" };
    Message::done(format!(
        "Moved {} to the trash. {they} can be restored.",
        count(items, "item", "items")
    ))
}

/// A folder that cannot be watched (a network drive, a watch that failed
/// to start): "Folder not watched. Reload with Ctrl+R.", with the reload
/// key from the live keymap.
pub fn folder_not_watched(reload_key: &str) -> Message {
    if reload_key.is_empty() {
        Message::status("Folder not watched. Reload from the View menu.")
    } else {
        Message::status(format!("Folder not watched. Reload with {reload_key}."))
    }
}

/// A wrong password with tries left: "Wrong password for course.zip. Try
/// again, 2 of 3."
pub fn wrong_secret(subject: &str, attempt: u32, allowed: u32) -> Message {
    Message::warning(format!(
        "Wrong password for {subject}. Try again, {attempt} of {allowed}."
    ))
}

/// After the last wrong try: "Gave up on course.zip after 3 wrong
/// passwords. Skipped."
pub fn gave_up_on_secret(subject: &str, allowed: u32) -> Message {
    Message::warning(format!(
        "Gave up on {subject} after {allowed} wrong passwords. Skipped."
    ))
}

/// A job waiting for a password, said at once as a change of state:
/// "Waiting for the password for course.zip."
pub fn waiting_for_secret(subject: &str) -> Message {
    Message::warning(format!("Waiting for the password for {subject}."))
}

/// No password given: "No password given. Skipped 10 members of
/// course.zip."
pub fn no_secret_given(skipped: u64, subject: &str) -> Message {
    Message::info(format!(
        "No password given. Skipped {} of {subject}.",
        count(skipped, "member", "members")
    ))
}

/// Every fixed sentence, for the Braille walk test.
pub const FIXED: [&str; 9] = [
    TRASH_NOT_CONFIRMED,
    PERMANENT_DELETE_NOTE,
    PROGRAM_TOO_DEEP,
    OPEN_TOO_DEEP,
    FOLDER_GONE,
    SECRET_SHOWN,
    SECRET_HIDDEN,
    SECRETS_DIFFER,
    SECRET_EMPTY,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::within_cells;
    use crate::message::{Importance, MessageKind};

    #[test]
    fn the_safety_questions_start_on_no_and_say_the_fact_first() {
        let q = no_trash_here(3);
        assert_eq!(
            q.text,
            "This drive has no trash. Delete 3 items for good? y or n"
        );
        assert_eq!(q.kind, MessageKind::Question);
        assert!(within_cells(&q.text, "no trash"));
        assert_eq!(delete_for_good(1).text, "Delete 1 item for good? y or n");
        assert!(within_cells(
            &too_long_for_trash(1).text,
            "too long for the trash"
        ));
        assert_eq!(
            moved_to_trash(1).text,
            "Moved 1 item to the trash. It can be restored."
        );
        let copies = trash_copies_across(64_000_001);
        assert_eq!(
            copies.text,
            "Trashing copies 64.0 MB to another drive. Go on? y or n"
        );
        assert_eq!(copies.kind, MessageKind::Question);
        assert!(within_cells(&copies.text, "to another drive"));
    }

    #[test]
    fn keys_come_from_the_caller() {
        assert_eq!(
            folder_not_watched("Ctrl+R").text,
            "Folder not watched. Reload with Ctrl+R."
        );
        assert!(!folder_not_watched("").text.contains("Ctrl"));
        assert_eq!(folder_not_watched("F9").importance, Importance::Low);
    }

    #[test]
    fn every_fixed_line_says_its_meaning_in_40_cells() {
        let keys = [
            "Deleted but not found",
            "Deleted data may stay",
            "Cannot start a program",
            "Cannot open a file",
            "Folder is gone",
            "Showing the password",
            "Hidden",
            "The two passwords differ",
            "A password cannot be empty",
        ];
        for (line, key) in FIXED.iter().zip(keys) {
            assert!(within_cells(line, key), "{line}");
        }
    }

    #[test]
    fn the_secret_lines_read_as_r9_wrote_them() {
        assert_eq!(
            wrong_secret("course.zip", 2, 3).text,
            "Wrong password for course.zip. Try again, 2 of 3."
        );
        assert_eq!(
            gave_up_on_secret("course.zip", 3).text,
            "Gave up on course.zip after 3 wrong passwords. Skipped."
        );
        assert_eq!(
            waiting_for_secret("course.zip").text,
            "Waiting for the password for course.zip."
        );
        assert_eq!(
            no_secret_given(10, "course.zip").text,
            "No password given. Skipped 10 members of course.zip."
        );
    }
}
