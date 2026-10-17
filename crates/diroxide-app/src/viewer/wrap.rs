//! Wrapping a line into rows: at the screen's width or a fixed width such
//! as 40 for a Braille line, after a space where one fits, so a row never
//! starts in the middle of a word unless the word is longer than the row.
//! Every row after the first says "continued" first.

use std::ops::Range;

/// The words a continued row starts with.
pub const CONTINUED: &str = "continued: ";

/// The fewest cells a row's text gets, whatever the width and prefix.
const MIN_ROOM: usize = 8;

/// The cells a character takes in a terminal: two for the wide East
/// Asian ranges and most emoji, none for combining marks, one otherwise.
/// An approximation of the Unicode width tables, close enough to wrap.
pub(crate) fn char_cells(c: char) -> usize {
    let code = u32::from(c);
    if matches!(code, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F) {
        return 0;
    }
    let wide = matches!(code,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD
    );
    if wide { 2 } else { 1 }
}

/// The cells a text takes.
pub(crate) fn cells(text: &str) -> usize {
    text.chars().map(char_cells).sum()
}

/// The byte ranges of `text` that make its rows: the first row has
/// `first` cells, the others `rest`. An empty text is one empty row.
pub(crate) fn wrap(text: &str, first: usize, rest: usize) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut room = first.max(MIN_ROOM);
    let mut used = 0usize;
    // The byte after the last space in the current row, where a break
    // keeps words whole.
    let mut after_space: Option<usize> = None;
    let mut used_at_space = 0usize;
    for (at, c) in text.char_indices() {
        let w = char_cells(c);
        if used + w > room && at > start {
            let cut = match after_space {
                Some(space) if space > start && space <= at => space,
                _ => at,
            };
            rows.push(start..cut);
            used = if cut == at { 0 } else { used - used_at_space };
            start = cut;
            room = rest.max(MIN_ROOM);
            after_space = None;
        }
        used += w;
        if c == ' ' {
            after_space = Some(at + 1);
            used_at_space = used;
        }
    }
    rows.push(start..text.len());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str, first: usize, rest: usize) -> Vec<&str> {
        wrap(text, first, rest)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn rows_break_after_a_space_and_keep_words_whole() {
        let text = "the quick brown fox jumps over the lazy dog";
        assert_eq!(
            pieces(text, 16, 12),
            ["the quick brown ", "fox jumps ", "over the ", "lazy dog"]
        );
        assert_eq!(pieces("short", 40, 29), ["short"]);
        assert_eq!(pieces("", 40, 29), [""]);
        assert_eq!(
            pieces("abcdefghijklmnopqrstuvwxyz", 10, 10),
            ["abcdefghij", "klmnopqrst", "uvwxyz"],
            "a word longer than the row is cut"
        );
        let joined: String = pieces(text, 16, 12).concat();
        assert_eq!(joined, text, "nothing is lost");
    }

    #[test]
    fn wide_characters_take_two_cells() {
        assert_eq!(cells("日本語"), 6);
        assert_eq!(cells("e\u{301}"), 1);
        assert_eq!(pieces("日本語日本語", 8, 8), ["日本語日", "本語"]);
    }
}
