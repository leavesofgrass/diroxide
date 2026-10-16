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

use diroxide_core::place::strip_extended_prefix;
use diroxide_core::{Place, catalog};

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

/// The refusal before a program or a shell starts, or `None` when it may:
/// "Cannot start a program in a folder this deep" when the working folder
/// is too deep, or when the program itself is stored deeper than the
/// shell can start it from (spike four).
pub fn program_refusal(folder: &Place, program: Option<&Place>) -> Option<&'static str> {
    let deep_folder = !can_start_program_in(folder);
    let deep_program = program.is_some_and(|program| !can_open_with_system(program));
    (deep_folder || deep_program).then_some(catalog::PROGRAM_TOO_DEEP)
}

/// The refusal before the system opener is given a file, or `None` when
/// it may: "Cannot open a file this deep". The opener's own error text is
/// never shown (spike four found it unreadable).
pub fn opener_refusal(file: &Place) -> Option<&'static str> {
    (!can_open_with_system(file)).then_some(catalog::OPEN_TOO_DEEP)
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

    #[test]
    fn refusals_use_the_catalog_sentences_at_the_measured_lengths() {
        let base = if cfg!(windows) { "D:\\" } else { "/" };
        // A working folder of 258 units starts; 259 does not (spike six).
        let at = |units: usize| Place::parse(&format!("{base}{}", "f".repeat(units - 3))).unwrap();
        if cfg!(windows) {
            assert_eq!(program_refusal(&at(258), None), None);
            assert_eq!(
                program_refusal(&at(259), None),
                Some("Cannot start a program in a folder this deep")
            );
            assert_eq!(opener_refusal(&at(259)), None);
            assert_eq!(
                opener_refusal(&at(260)),
                Some("Cannot open a file this deep")
            );
            assert_eq!(
                program_refusal(&at(10), Some(&at(260))),
                Some("Cannot start a program in a folder this deep")
            );
        } else {
            assert_eq!(program_refusal(&at(259), Some(&at(260))), None);
            assert_eq!(opener_refusal(&at(260)), None);
        }
    }
}
