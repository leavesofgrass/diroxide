//! Path-length limits of the shell-bound operations (spike four). The
//! standard library lists, copies, renames and removes at any depth; the
//! Windows shell does not, so these operations are refused before the
//! call, with the catalog's sentences:
//!
//! - the trash, at 260 UTF-16 units or more ([`crate::trash`]);
//! - running a program or a shell in a folder of 259 units or more
//!   ("Cannot start a program in a folder this deep"), and a program
//!   stored at that depth;
//! - the system opener at depth ("Cannot open a file this deep").
//!
//! Lengths are counted in UTF-16 units, as Windows counts them, on the
//! plain path (never with the `\\?\` prefix, which is not shown and not
//! passed to the shell).

use diroxide_core::Place;
use diroxide_core::place::strip_extended_prefix;

/// The longest working folder a program can start in, in UTF-16 units
/// (spike six: 258 started, 259 failed with "The directory name is
/// invalid").
pub const PROGRAM_FOLDER_MAX_UNITS: usize = 258;

/// The longest path the system opener is given, in UTF-16 units: the
/// shell's limit, as for the trash.
pub const OPENER_MAX_PATH_UNITS: usize = 259;

/// The length of a path in UTF-16 units, without any extended-length
/// prefix.
pub fn utf16_units(path: &str) -> usize {
    strip_extended_prefix(path).encode_utf16().count()
}

/// Whether a program or a shell can start with this folder as its working
/// folder. Always true off Windows.
pub fn can_start_program_in(folder: &Place) -> bool {
    !cfg!(windows) || utf16_units(&folder.to_string()) <= PROGRAM_FOLDER_MAX_UNITS
}

/// Whether the system opener can be given this file. Always true off
/// Windows.
pub fn can_open_with_system(file: &Place) -> bool {
    !cfg!(windows) || utf16_units(&file.to_string()) <= OPENER_MAX_PATH_UNITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_count_utf16_units_without_the_prefix() {
        assert_eq!(utf16_units(r"D:\abc"), 6);
        assert_eq!(utf16_units(r"\\?\D:\abc"), 6);
        assert_eq!(utf16_units("D:\\\u{1F980}"), 5);
    }

    #[test]
    fn deep_folders_are_refused_on_windows_only() {
        let base = if cfg!(windows) { "D:\\" } else { "/" };
        let shallow = Place::parse(&format!("{base}docs")).unwrap();
        assert!(can_start_program_in(&shallow));
        assert!(can_open_with_system(&shallow));
        let name = "x".repeat(250);
        let deep = Place::parse(&format!("{base}{name}"))
            .unwrap()
            .join(&"y".repeat(20))
            .unwrap();
        assert_eq!(can_start_program_in(&deep), !cfg!(windows));
        assert_eq!(can_open_with_system(&deep), !cfg!(windows));
    }
}
