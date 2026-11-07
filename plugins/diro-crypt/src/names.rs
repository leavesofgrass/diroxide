//! The names of what the plugin makes: `NAME.age` beside the file it
//! encrypts, `NAME` with `.age` removed beside the file it decrypts, and
//! a name that is taken numbered the way diroxide's copies are
//! ("report (2).pdf.age"), never written over.

use std::path::Path;

use diroxide_vfs::names::numbered_name;

/// The extension age files carry.
pub const AGE_EXTENSION: &str = ".age";

/// What a decrypted file is called when its name does not end in `.age`.
pub const DECRYPTED_EXTENSION: &str = ".decrypted";

/// The output's name for encrypting `name`: `report.pdf.age`.
pub fn encrypted_name(name: &str) -> String {
    format!("{name}{AGE_EXTENSION}")
}

/// The output's name for decrypting `name`: `report.pdf` from
/// `report.pdf.age` (any case of the extension), else the name with
/// `.decrypted` after it. A name that is only `.age` keeps a stem.
pub fn decrypted_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(AGE_EXTENSION) && name.len() > AGE_EXTENSION.len() {
        let stem = &name[..name.len() - AGE_EXTENSION.len()];
        if !stem.is_empty() && !stem.ends_with('.') {
            return stem.to_owned();
        }
    }
    format!("{name}{DECRYPTED_EXTENSION}")
}

/// The first name that is free in `folder`: `name` itself, then
/// `name (2)`, `name (3)`... with the extension kept after the number.
/// `None` after 999 tries.
pub fn free_name(folder: &Path, name: &str) -> Option<String> {
    free_name_by(name, |candidate| {
        std::fs::symlink_metadata(folder.join(candidate)).is_ok()
    })
}

/// [`free_name`] over a test of whether a name is taken.
pub fn free_name_by(name: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    if !taken(name) {
        return Some(name.to_owned());
    }
    (2..1000)
        .map(|n| numbered_name(name, n))
        .find(|candidate| !taken(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_are_named_beside_their_files() {
        assert_eq!(encrypted_name("report.pdf"), "report.pdf.age");
        assert_eq!(decrypted_name("report.pdf.age"), "report.pdf");
        assert_eq!(decrypted_name("REPORT.PDF.AGE"), "REPORT.PDF");
        assert_eq!(decrypted_name("backup"), "backup.decrypted");
        assert_eq!(decrypted_name(".age"), ".age.decrypted");
        assert_eq!(decrypted_name("a..age"), "a..age.decrypted");
    }

    #[test]
    fn a_taken_name_is_numbered_as_copies_are() {
        let taken = ["report.pdf.age", "report.pdf (2).age"];
        assert_eq!(
            free_name_by("report.pdf.age", |n| taken.contains(&n)).as_deref(),
            Some("report.pdf (3).age")
        );
        assert_eq!(
            free_name_by("notes.txt", |_| false).as_deref(),
            Some("notes.txt")
        );
        assert_eq!(free_name_by("x", |_| true), None);
    }
}
