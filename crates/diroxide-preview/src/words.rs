//! Words the previewers share. Text a file carries about itself (an
//! author's description, a title, a tag, a sheet's name) is hostile until
//! cleaned (R28 2.5's hostile-metadata risk): every such string goes
//! through [`clean_text`] before it is shown or spoken, the rule file
//! names follow.

use diroxide_core::names::display_name;

/// What a cut string ends with.
pub const CUT_SHORT: &str = ", cut short";

/// Text from a file, cleaned like a file name: runs of white space (line
/// breaks and tabs among them) become one space, the ends are trimmed,
/// every hidden character (a control, a format or direction character, a
/// private-use one) shows as a question mark, and at most `max_chars`
/// characters are kept, then ", cut short".
pub fn clean_text(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let shown = display_name(&collapsed);
    if shown.chars().count() <= max_chars {
        return shown.into_owned();
    }
    let mut cut: String = shown.chars().take(max_chars).collect();
    cut.truncate(cut.trim_end().len());
    cut.push_str(CUT_SHORT);
    cut
}

/// "1 frame", "24 frames", with thousands grouped.
pub fn count_words(n: u64, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{} {plural}", diroxide_core::format::grouped(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_text_is_cleaned_like_a_name() {
        assert_eq!(
            clean_text("  Ada\tExample\r\n at the harbor ", 80),
            "Ada Example at the harbor"
        );
        assert_eq!(clean_text("evil\u{202E}txt.exe", 80), "evil?txt.exe");
        assert_eq!(clean_text("\u{1b}[31mred", 80), "?[31mred");
        assert_eq!(clean_text("abcdef ghij", 7), "abcdef, cut short");
        assert_eq!(clean_text("   ", 7), "");
        assert_eq!(count_words(1, "frame", "frames"), "1 frame");
        assert_eq!(count_words(1_203, "row", "rows"), "1,203 rows");
    }
}
