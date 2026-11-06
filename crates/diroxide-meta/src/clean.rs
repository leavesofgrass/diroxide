//! Cleaning: every string a reader takes from a file is cleaned like a
//! file name before anyone sees or hears it (R12, "Cleaning"). A song
//! title can carry a terminal escape, a right-to-left override or a
//! private-use character; a PDF author can run to megabytes. After
//! cleaning, line breaks and tabs are spaces, every hidden character is a
//! question mark (`diroxide_core::names::display_name`, the rule names
//! already follow), runs of spaces are one, and a value longer than
//! [`MAX_VALUE_CHARS`] is cut with the words "(cut)" at its end.
//!
//! Keys are plain (R12, "Keys"): lowercase letters, digits, dot, dash and
//! underscore; [`plain_key`] makes any text one.

use diroxide_core::names::display_name;

/// The most characters a value keeps; longer values are cut, the words
/// "(cut)" said at the end.
pub const MAX_VALUE_CHARS: usize = 400;

/// The words that end a value that was cut.
pub const CUT: &str = " (cut)";

/// A value as it may be shown and said: one line, hidden characters as
/// question marks, spaces collapsed, trimmed, and cut at
/// [`MAX_VALUE_CHARS`].
pub fn clean_text(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if is_spacing(c) { ' ' } else { c })
        .collect();
    let shown = display_name(&spaced);
    let mut out = String::with_capacity(shown.len().min(MAX_VALUE_CHARS * 4));
    let mut count = 0;
    let mut space = false;
    for c in shown.trim().chars() {
        if c == ' ' {
            space = true;
            continue;
        }
        if space {
            if count + 1 >= MAX_VALUE_CHARS {
                out.push_str(CUT);
                return out;
            }
            out.push(' ');
            count += 1;
            space = false;
        }
        if count >= MAX_VALUE_CHARS {
            out.push_str(CUT);
            return out;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Characters read as a space: line and paragraph breaks, tabs, the NUL
/// that ends many stored strings, and every Unicode space.
fn is_spacing(c: char) -> bool {
    matches!(
        c,
        '\0' | '\t' | '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    ) || c.is_whitespace()
}

/// Whether a key is plain: lowercase ASCII letters, digits, dot, dash
/// and underscore, not empty, and not starting or ending with a dot.
pub fn is_plain_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('.')
        && !key.ends_with('.')
        && key.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_')
        })
}

/// Any text as a plain key part: ASCII letters lowercased, digits, dot,
/// dash and underscore kept, anything else an underscore; "unnamed" for
/// an empty result. ExifTool's group and tag names become
/// `exiftool.GROUP.TAG` this way.
pub fn plain_key(text: &str) -> String {
    let key: String = text
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if key.is_empty() {
        "unnamed".to_owned()
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_are_question_marks_and_breaks_are_spaces() {
        assert_eq!(clean_text("Night\u{1b}[31m Song"), "Night?[31m Song");
        assert_eq!(clean_text("abc\u{202E}def"), "abc?def");
        assert_eq!(
            clean_text("line one\r\nline two\tend"),
            "line one line two end"
        );
        assert_eq!(clean_text("  Ada   Example\0\0"), "Ada Example");
        assert_eq!(clean_text("\u{E000}"), "?");
        assert_eq!(clean_text(""), "");
        assert_eq!(clean_text(" \n\t "), "");
    }

    #[test]
    fn a_long_value_is_cut_and_says_so() {
        let long = "word ".repeat(500);
        let cleaned = clean_text(&long);
        assert!(cleaned.ends_with(CUT));
        assert!(cleaned.chars().count() <= MAX_VALUE_CHARS + CUT.chars().count());
        let exact = "x".repeat(MAX_VALUE_CHARS);
        assert_eq!(clean_text(&exact), exact);
        let over = "x".repeat(MAX_VALUE_CHARS + 1);
        assert!(clean_text(&over).ends_with(CUT));
    }

    #[test]
    fn keys_are_plain() {
        assert!(is_plain_key("image.width"));
        assert!(is_plain_key("exiftool.exif.make"));
        assert!(!is_plain_key("Image.Width"));
        assert!(!is_plain_key("a b"));
        assert!(!is_plain_key(".hidden"));
        assert!(!is_plain_key(""));
        assert_eq!(plain_key("XMP-dc"), "xmp-dc");
        assert_eq!(plain_key("GPS Latitude"), "gps_latitude");
        assert_eq!(plain_key("Ünïcode"), "_n_code");
        assert_eq!(plain_key(""), "unnamed");
    }
}
