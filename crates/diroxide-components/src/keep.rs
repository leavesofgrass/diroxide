//! Which members of a helper's archive are unpacked, and under what name
//! (alpha 4, R28 1.3; spike T1).
//!
//! A publisher's archive holds more than the program needs: ImageMagick's
//! portable build carries eight copies of one 32 MB program, FFmpeg's a
//! player, headers and documentation. A file's `keep` patterns name the
//! members its component is made of; the rest are not unpacked and not
//! said. A few names a publisher makes are not plain (`exiftool(-k).exe`),
//! and a file's `rename` list gives each a plain one before the name rule
//! is applied; everything else still has to be plain.
//!
//! A pattern is matched against a member's path after `strip`, part by
//! part, without regard to ASCII case: `*` stands for any run of
//! characters within one part, `**` for any number of whole parts.
//! `bin/*` keeps the files directly in `bin`; `exiftool_files/**` keeps a
//! folder with everything under it; `LICENSE*` keeps `LICENSE.txt` at the
//! top. A pattern holds only the characters of a plain name and `*`.

/// The most patterns one file may give.
pub const MAX_PATTERNS: usize = 32;

/// The most renames one file may give.
pub const MAX_RENAMES: usize = 32;

/// The most extra programs one file may name.
pub const MAX_PROGRAMS: usize = 16;

/// The longest pattern or member path a list may give, in bytes.
pub const MAX_PATTERN_BYTES: usize = 240;

/// The most `**` parts one pattern may hold, which keeps matching quick
/// whatever a list says.
pub const MAX_ANY_FOLDERS: usize = 2;

/// Whether `pattern` is one a list may give: parts joined by single `/`,
/// each `**` or made of letters, digits, `-`, `_`, `.` and `*`, never `.`
/// or `..`; at most [`MAX_ANY_FOLDERS`] `**` parts and
/// [`MAX_PATTERN_BYTES`] bytes.
pub fn is_keep_pattern(pattern: &str) -> bool {
    !pattern.is_empty()
        && pattern.len() <= MAX_PATTERN_BYTES
        && pattern.split('/').filter(|part| *part == "**").count() <= MAX_ANY_FOLDERS
        && pattern.split('/').all(|part| {
            part == "**"
                || (!part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part.contains("**")
                    && part.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'*')
                    }))
        })
}

/// Whether a member at `path` (after `strip`, parts joined by `/`) is
/// kept by `patterns`: always when there are none.
pub fn keeps(patterns: &[String], path: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let parts: Vec<&str> = path.split('/').collect();
    patterns.iter().any(|pattern| {
        let wanted: Vec<&str> = pattern.split('/').collect();
        match_parts(&wanted, &parts)
    })
}

fn match_parts(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| match_parts(rest, &path[skip..])),
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(part, more)| match_part(first, part) && match_parts(rest, more)),
    }
}

/// One part against one name: `*` is any run of characters, the rest
/// match without regard to ASCII case.
fn match_part(pattern: &str, name: &str) -> bool {
    let pattern = pattern.as_bytes();
    let name = name.as_bytes();
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, n));
            p += 1;
        } else if p < pattern.len() && pattern[p].eq_ignore_ascii_case(&name[n]) {
            p += 1;
            n += 1;
        } else if let Some((at, from)) = star {
            p = at + 1;
            n = from + 1;
            star = Some((at, from + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|b| *b == b'*')
}

/// Whether `from` is a member path a rename may name: not empty, at most
/// [`MAX_PATTERN_BYTES`] bytes, no control character, no backslash, no
/// leading `/`, no part that is empty, `.` or `..`. It need not be plain:
/// that is what a rename is for.
pub fn is_rename_source(from: &str) -> bool {
    !from.is_empty()
        && from.len() <= MAX_PATTERN_BYTES
        && !from.chars().any(|c| c.is_control() || c == '\\')
        && from
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// The new path `rename` gives the member at `path`, if any.
pub fn renamed<'a>(rename: &'a [(String, String)], path: &str) -> Option<&'a str> {
    rename
        .iter()
        .find(|(from, _)| from == path)
        .map(|(_, to)| to.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(patterns: &[&str], path: &str) -> bool {
        let patterns: Vec<String> = patterns.iter().map(|p| (*p).to_owned()).collect();
        keeps(&patterns, path)
    }

    #[test]
    fn patterns_keep_what_they_name() {
        assert!(kept(&[], "anything/at/all"));
        assert!(kept(&["bin/*"], "bin/ffmpeg.exe"));
        assert!(!kept(&["bin/*"], "bin/sub/x.dll"));
        assert!(!kept(&["bin/*"], "doc/ffmpeg.html"));
        assert!(kept(&["bin/*.dll"], "bin/avcodec-62.dll"));
        assert!(!kept(&["bin/*.dll"], "bin/ffplay.exe"));
        assert!(kept(&["LICENSE*"], "LICENSE.txt"));
        assert!(kept(&["LICENSE*"], "license"));
        assert!(kept(&["exiftool*.exe"], "exiftool(-k).exe"));
        assert!(kept(
            &["exiftool_files/**"],
            "exiftool_files/lib/Image/ExifTool.pm"
        ));
        assert!(kept(&["exiftool_files/**"], "exiftool_files/perl.exe"));
        assert!(!kept(&["exiftool_files/**"], "README.txt"));
        assert!(kept(&["**/*.xml"], "magick/config/policy.xml"));
        assert!(kept(&["**/*.xml"], "policy.xml"));
        assert!(kept(&["magick.exe", "*.xml"], "colors.xml"));
        assert!(!kept(&["magick.exe", "*.xml"], "compare.exe"));
        assert!(kept(&["a*b*c"], "aXXbYYc"));
        assert!(!kept(&["a*b*c"], "aXXbYY"));
        assert!(kept(&["*"], "x"));
    }

    #[test]
    fn only_plain_patterns_are_given() {
        for good in ["bin/*", "**/*.xml", "LICENSE*", "a/**/b/**", "x.y-z_1"] {
            assert!(is_keep_pattern(good), "{good}");
        }
        for bad in [
            "",
            "/bin",
            "bin/",
            "a//b",
            "../x",
            "./x",
            "a b",
            "a(1)",
            "a\\b",
            "a/***/b",
            "**/**/**",
            "caf\u{e9}",
        ] {
            assert!(!is_keep_pattern(bad), "{bad:?}");
        }
        assert!(!is_keep_pattern(&"a".repeat(MAX_PATTERN_BYTES + 1)));
    }

    #[test]
    fn a_rename_names_a_member_and_gives_a_new_path() {
        let list = vec![("exiftool(-k).exe".to_owned(), "exiftool.exe".to_owned())];
        assert_eq!(renamed(&list, "exiftool(-k).exe"), Some("exiftool.exe"));
        assert_eq!(renamed(&list, "exiftool.exe"), None);
        assert!(is_rename_source("exiftool(-k).exe"));
        assert!(is_rename_source("dir/a b.txt"));
        for bad in ["", "/x", "a/../b", "a\\b", "a\u{7}b", "a//b"] {
            assert!(!is_rename_source(bad), "{bad:?}");
        }
    }

    /// A pattern with the most `**` parts against a deep path stays quick.
    #[test]
    fn matching_stays_quick() {
        let deep = vec!["d"; 64].join("/");
        let patterns = vec!["**/x/**/y".to_owned(); MAX_PATTERNS];
        let started = std::time::Instant::now();
        assert!(!keeps(&patterns, &deep));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
