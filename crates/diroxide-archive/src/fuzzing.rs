//! The bodies of the `archive_*` fuzz targets (research/archives.md
//! section 12), as plain functions, so the libFuzzer entry points in
//! `fuzz/fuzz_targets/` and the ordinary tests call the same code: the
//! checked-in seeds and every crash found later replay on Windows and
//! macOS in normal CI, with no fuzzer. Each body panics only when a
//! property fails; bad input is an error, never a panic.

use diroxide_core::Place;
use diroxide_core::place::PathStyle;

use crate::limits::Limits;
use crate::names::{is_junk, member_path, normalize_member_name};

/// `archive_names`: the member-name rules, `Place` parsing with `!`,
/// `join` and `is_junk`, over each line of the input. The properties: a
/// result never holds `..`, a leading slash, a drive, NUL or a control
/// character; normalizing twice gives the same result; and a member place
/// that is printed and parsed again is equal to the original.
pub fn names(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    for line in text.split('\n').take(64) {
        let _ = is_junk(line);
        if let Some(once) = normalize_member_name(line) {
            assert_safe(&once);
            assert_eq!(normalize_member_name(&once).as_deref(), Some(once.as_str()));
        }
        let limits = Limits::default();
        if let Ok(once) = member_path(line, &limits) {
            assert_safe(&once.path);
            let twice = member_path(&once.path, &limits).map(|p| p.path);
            assert_eq!(twice.as_deref(), Ok(once.path.as_str()));
            round_trip(&once.path);
        }
    }
}

/// The safe-path properties.
fn assert_safe(path: &str) {
    assert!(!path.starts_with('/'), "a leading slash in {path:?}");
    assert!(!path.contains('\\'), "a backslash in {path:?}");
    assert!(
        !path.chars().any(char::is_control),
        "a control character in {path:?}"
    );
    for part in path.split('/') {
        assert!(
            !part.is_empty() && part != "." && part != "..",
            "a bad part in {path:?}"
        );
        let bytes = part.as_bytes();
        assert!(
            !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'),
            "a drive in {path:?}"
        );
    }
}

/// Joins each part of a safe member path under an archive's root, one by
/// one as a listing does, and checks the place prints and parses back
/// equal; a part `join` refuses (an archive marker) ends the walk.
fn round_trip(path: &str) {
    let Ok(container) = Place::parse_with("/srv/course.zip", PathStyle::Unix) else {
        return;
    };
    let mut place = container.into_archive();
    for part in path.split('/') {
        match place.join(part) {
            Ok(next) => place = next,
            Err(_) => return,
        }
        let printed = place.to_string();
        let parsed = Place::parse_with(&printed, PathStyle::Unix);
        assert_eq!(parsed.as_ref(), Ok(&place), "{printed:?}");
    }
}

/// The seeds of each target, generated, so the checked-in files under
/// `fixtures/archives/seeds/` are reproducible: a test compares them, and
/// writes them when `DIRO_WRITE_SEEDS=1` is set.
pub fn seeds() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let names_lines: &[&str] = &[
        "../x",
        "a/../../x",
        "/etc/passwd",
        r"C:\win\x.dll",
        r"\\?\C:\x",
        r"\\?\\\?\C:\x",
        r"/\\?\C:\x",
        r"\??\C:\x",
        r"\\.\C:\dev",
        r"\\?\UNC\srv\share\x",
        "a/C:/x",
        "a/.. /x",
        "a\u{0}b",
        "tab\there",
        "CON",
        "a.txt:b",
        "caf\u{e9}.txt",
        "cafe\u{301}.txt",
        "inner.zip!x.txt",
        "photo\u{202e}gpj.exe",
        "PROGRA~1/x",
        "__MACOSX/._a",
        "trailing ",
        "ok/name!.txt",
    ];
    vec![(
        "archive_names",
        "hostile-names.txt",
        names_lines.join("\n").into_bytes(),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_seeds_hold() {
        for (target, _, bytes) in seeds() {
            if target == "archive_names" {
                names(&bytes);
            }
        }
        names(b"");
        names(&[0xFF, 0xFE, b'\n', b'/', b'.', b'.']);
    }
}
