//! Making links, and changing where one points (plan sections 5.4 and
//! 5.12; research/permissions.md 10; research/file-operations.md, "Symbolic
//! links"; research/alpha2-plan.md 3.7, 4.14 and commit 26).
//!
//! - **Relative or absolute.** A link points at its target by the full
//!   path, or by the way from the link's own folder (`..\data\x.txt`), so a
//!   folder that holds both still works after it moves. A target on
//!   another drive cannot be reached that way, and the link is made
//!   absolute, said in the plan's note.
//! - **Refused before planning** where the place holds no links (a FAT or
//!   exFAT drive, an archive, a read-only place), in the capability's own
//!   words: "Cannot link here: this place holds no links".
//! - **The privilege, in words.** Windows makes a symbolic link only with
//!   Developer Mode or an administrator; the failure is said as
//!   [`NEEDS_DEVELOPER_MODE`], never as the system's error code, and a
//!   folder can be offered a junction instead ([`junction_question`]).
//! - **Changing a link's target** makes the new link beside the old under
//!   a temporary name, sends the old one to the trash, and gives the new
//!   one the name, as one step that Undo reverses (the old link comes back
//!   from the trash). The provider's rename never replaces, so the name is
//!   free for a moment between the trash and the rename; a rename that
//!   replaces is a contract change request.

use diroxide_core::{Message, Place};

/// The failure of a symbolic link on Windows without the privilege.
pub const NEEDS_DEVELOPER_MODE: &str = "Cannot link: needs developer mode.";

/// How a new link writes its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LinkForm {
    /// The full path: the link keeps working wherever it is moved, as long
    /// as the target stays.
    #[default]
    Absolute,
    /// The way from the link's folder: the link keeps working when a
    /// folder holding both moves.
    Relative,
}

impl LinkForm {
    /// The form in words, for the plan's rows: "relative", "absolute".
    pub const fn word(self) -> &'static str {
        match self {
            LinkForm::Absolute => "absolute",
            LinkForm::Relative => "relative",
        }
    }
}

/// The question after a folder link failed for the privilege: "Make a
/// junction instead? y or n". A junction needs no privilege, holds a
/// folder only, and points by the full path.
pub fn junction_question() -> Message {
    Message::question("Make a junction instead")
}

/// The top of a place and the names from it down.
fn names_of(place: &Place) -> (Place, Vec<String>) {
    let mut names = Vec::new();
    let mut current = place.clone();
    while let Some(parent) = current.parent() {
        names.push(current.name().unwrap_or_default().to_owned());
        current = parent;
    }
    names.reverse();
    (current, names)
}

/// The target of a link at `link`, written as the way from the link's
/// folder to `target`: `..\data\x.txt` on Windows, `../data/x.txt`
/// elsewhere. `None` when there is no such way: the two are on different
/// drives, or not local places.
pub fn relative_target(link: &Place, target: &Place) -> Option<String> {
    let (folder_path, target_path) = (
        link.parent()?.local_path()?.to_owned(),
        target.local_path()?,
    );
    let windows = !folder_path.starts_with('/');
    if windows == target_path.starts_with('/') {
        return None;
    }
    let (folder_top, folder) = names_of(&link.parent()?);
    let (target_top, target_names) = names_of(target);
    if !(folder_top.is_within(&target_top) && target_top.is_within(&folder_top)) {
        return None;
    }
    let same = |a: &String, b: &String| {
        if windows {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        }
    };
    let common = folder
        .iter()
        .zip(&target_names)
        .take_while(|(a, b)| same(a, b))
        .count();
    let mut parts: Vec<&str> = vec![".."; folder.len() - common];
    parts.extend(target_names[common..].iter().map(String::as_str));
    if parts.is_empty() {
        return Some(".".to_owned());
    }
    Some(parts.join(if windows { "\\" } else { "/" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::place::PathStyle;

    fn unix(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    fn windows(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Windows).unwrap()
    }

    #[test]
    fn a_relative_target_is_the_way_from_the_links_folder() {
        let table = [
            ("/proj/links/x", "/proj/data/x.txt", Some("../data/x.txt")),
            ("/proj/x", "/proj/data/x.txt", Some("data/x.txt")),
            ("/proj/a/b/x", "/proj/x.txt", Some("../../x.txt")),
            ("/proj/x", "/proj/y", Some("y")),
            ("/proj/x", "/proj", Some(".")),
            ("/x", "/", Some(".")),
        ];
        for (link, target, expected) in table {
            assert_eq!(
                relative_target(&unix(link), &unix(target)).as_deref(),
                expected,
                "{link} to {target}"
            );
        }
    }

    #[test]
    fn windows_paths_use_backslashes_ignore_case_and_stop_at_the_drive() {
        assert_eq!(
            relative_target(
                &windows(r"D:\Proj\links\x"),
                &windows(r"d:\proj\Data\x.txt")
            )
            .as_deref(),
            Some(r"..\Data\x.txt")
        );
        assert_eq!(
            relative_target(&windows(r"D:\proj\x"), &windows(r"E:\data\x.txt")),
            None,
            "another drive: no way there"
        );
    }

    #[test]
    fn the_privilege_and_the_junction_are_said_in_words() {
        assert!(within_cells(
            NEEDS_DEVELOPER_MODE,
            "Cannot link: needs developer mode."
        ));
        assert_eq!(junction_question().text, "Make a junction instead? y or n");
        assert_eq!(LinkForm::default(), LinkForm::Absolute);
        assert_eq!(LinkForm::Relative.word(), "relative");
    }
}
