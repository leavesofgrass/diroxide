//! The word pass inside changed lines, and the change window for long
//! lines (R18, "Words and characters inside changed lines" and "A change
//! window for long lines").
//!
//! Words are as Ediff makes them: a run of letters, hyphens and
//! underscores is one word, a run of digits is one, a run of space is one,
//! a run of punctuation is one, and any other character stands alone, so
//! a hidden character is a word of its own. Combining marks stay with the
//! character before them. Only the standard library's character classes
//! are used, so `similar`'s Unicode feature stays off.

use std::ops::Range;
use std::time::Instant;

use diroxide_vfs::CancelToken;

use crate::engine;
use crate::error::DiffError;
use crate::forms::is_mark;
use crate::options::LineAlgorithm;

/// The longest line, in characters, the word pass takes on; a longer line
/// is "too long to compare by word" (R18's length cap).
pub const WORD_PASS_MAX_CHARS: usize = 10_000;

/// How many words the change window keeps on each side of a line's first
/// difference.
pub const WINDOW_WORDS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Word,
    Number,
    Space,
    Punctuation,
    Alone,
}

fn class_of(c: char) -> Class {
    if c.is_alphabetic() || c == '-' || c == '_' {
        Class::Word
    } else if c.is_numeric() {
        Class::Number
    } else if c.is_whitespace() {
        Class::Space
    } else if c.is_ascii_punctuation() {
        Class::Punctuation
    } else {
        Class::Alone
    }
}

/// The words of a line, as byte ranges, in order.
pub(crate) fn tokens(line: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut last: Option<Class> = None;
    for (at, c) in line.char_indices() {
        let end = at + c.len_utf8();
        if is_mark(c)
            && let Some(token) = out.last_mut()
        {
            token.end = end;
            continue;
        }
        let class = class_of(c);
        match (out.last_mut(), last) {
            (Some(token), Some(previous)) if previous == class && class != Class::Alone => {
                token.end = end;
            }
            _ => out.push(at..end),
        }
        last = Some(class);
    }
    out
}

fn is_space(line: &str, token: &Range<usize>) -> bool {
    line[token.clone()].chars().all(char::is_whitespace)
}

/// One stretch of a word pass: the same words in both lines, or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordSegment {
    /// Whether the words are the same.
    pub same: bool,
    /// The bytes of the before line it covers.
    pub before: Range<usize>,
    /// The bytes of the after line it covers.
    pub after: Range<usize>,
    before_tokens: Range<usize>,
    after_tokens: Range<usize>,
}

/// The words of two versions of a line, lined up: what the change window
/// says, and what a window frontend can emphasize for sighted readers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordDiff {
    segments: Vec<WordSegment>,
    before_tokens: Vec<Range<usize>>,
    after_tokens: Vec<Range<usize>>,
    too_long: bool,
}

impl WordDiff {
    /// The stretches, in order, covering both lines.
    pub fn segments(&self) -> &[WordSegment] {
        &self.segments
    }

    /// How many stretches differ: the changes within the line.
    pub fn differences(&self) -> usize {
        self.segments.iter().filter(|s| !s.same).count()
    }

    /// Whether a line was longer than [`WORD_PASS_MAX_CHARS`], so no word
    /// pass ran: "line too long to compare by word".
    pub fn too_long(&self) -> bool {
        self.too_long
    }
}

/// Lines up the words of two versions of a line, within the deadline and
/// honoring `cancel`.
pub fn word_diff(
    before: &str,
    after: &str,
    deadline: Option<Instant>,
    cancel: &CancelToken,
) -> Result<WordDiff, DiffError> {
    let too_long = |line: &str| line.chars().nth(WORD_PASS_MAX_CHARS).is_some();
    if too_long(before) || too_long(after) {
        return Ok(WordDiff {
            segments: Vec::new(),
            before_tokens: Vec::new(),
            after_tokens: Vec::new(),
            too_long: true,
        });
    }
    let before_tokens = tokens(before);
    let after_tokens = tokens(after);
    let mut ids = std::collections::HashMap::new();
    let mut id = |text: &str| -> u32 {
        let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        *ids.entry(text.to_owned()).or_insert(next)
    };
    let before_ids: Vec<u32> = before_tokens
        .iter()
        .map(|t| id(&before[t.clone()]))
        .collect();
    let after_ids: Vec<u32> = after_tokens.iter().map(|t| id(&after[t.clone()])).collect();
    let stretches = engine::align(
        &before_ids,
        &after_ids,
        LineAlgorithm::Myers,
        deadline,
        cancel,
    )
    .map_err(|_| DiffError::Cancelled)?;
    let bytes = |tokens: &[Range<usize>], range: &Range<usize>, line_len: usize| {
        let start = tokens.get(range.start).map_or(line_len, |t| t.start);
        let end = if range.is_empty() {
            start
        } else {
            tokens[range.end - 1].end
        };
        start..end
    };
    let segments = stretches
        .into_iter()
        .map(|s| WordSegment {
            same: s.same,
            before: bytes(&before_tokens, &s.before, before.len()),
            after: bytes(&after_tokens, &s.after, after.len()),
            before_tokens: s.before,
            after_tokens: s.after,
        })
        .collect();
    Ok(WordDiff {
        segments,
        before_tokens,
        after_tokens,
        too_long: false,
    })
}

/// The neighborhood of a line's first difference, as a row says it:
/// up to [`WINDOW_WORDS`] words on each side, with "..." where text was
/// cut, and how many more changes the line holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeWindow {
    /// The before line's neighborhood.
    pub before: String,
    /// The after line's neighborhood.
    pub after: String,
    /// The changes in the line after the first.
    pub more: usize,
}

/// The change window of two versions of a line: the neighborhood of
/// their first difference. `None` when the word pass did not run or
/// found no difference.
pub fn change_window(before: &str, after: &str, diff: &WordDiff) -> Option<ChangeWindow> {
    let first = diff.segments.iter().find(|s| !s.same)?;
    Some(ChangeWindow {
        before: neighborhood(before, &diff.before_tokens, &first.before_tokens),
        after: neighborhood(after, &diff.after_tokens, &first.after_tokens),
        more: diff.differences().saturating_sub(1),
    })
}

/// The words of `range`, with up to [`WINDOW_WORDS`] words on each side.
fn neighborhood(line: &str, tokens: &[Range<usize>], range: &Range<usize>) -> String {
    let mut start = range.start;
    let mut words = 0;
    while start > 0 && words < WINDOW_WORDS {
        start -= 1;
        if !is_space(line, &tokens[start]) {
            words += 1;
        }
    }
    let mut end = range.end;
    words = 0;
    while end < tokens.len() && words < WINDOW_WORDS {
        if !is_space(line, &tokens[end]) {
            words += 1;
        }
        end += 1;
    }
    let from = tokens.get(start).map_or(line.len(), |t| t.start);
    let to = if end == 0 {
        from
    } else {
        tokens.get(end - 1).map_or(line.len(), |t| t.end).max(from)
    };
    let mut text = String::new();
    if !line[..from].trim().is_empty() {
        text.push_str("...");
    }
    text.push_str(line[from..to].trim());
    if !line[to..].trim().is_empty() {
        text.push_str("...");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<&str> {
        tokens(line).into_iter().map(|t| &line[t]).collect()
    }

    fn diff(before: &str, after: &str) -> WordDiff {
        word_diff(before, after, None, &CancelToken::new()).unwrap()
    }

    #[test]
    fn words_are_cut_as_ediff_cuts_them() {
        assert_eq!(
            words("total_due-2 = 40.5;  ok"),
            [
                "total_due-",
                "2",
                " ",
                "=",
                " ",
                "40",
                ".",
                "5",
                ";",
                "  ",
                "ok"
            ]
        );
        // A hidden character stands alone; a combining mark stays.
        assert_eq!(words("a\u{200b}b"), ["a", "\u{200b}", "b"]);
        assert_eq!(words("cafe\u{301} x"), ["cafe\u{301}", " ", "x"]);
        // Punctuation runs together; other symbols stand alone.
        assert_eq!(words("$$"), ["$$"]);
        assert_eq!(words("\u{a9}\u{a9}"), ["\u{a9}", "\u{a9}"]);
        assert!(words("").is_empty());
    }

    #[test]
    fn the_word_pass_finds_the_changed_words() {
        let d = diff("the total due today", "the total owed today");
        assert_eq!(d.differences(), 1);
        let changed = d.segments().iter().find(|s| !s.same).unwrap();
        assert_eq!(&"the total due today"[changed.before.clone()], "due");
        assert_eq!(&"the total owed today"[changed.after.clone()], "owed");
    }

    #[test]
    fn the_window_keeps_three_words_each_side() {
        let before = "one two three four five six seven eight nine ten eleven";
        let after = "one two three four five SIX seven eight nine ten eleven";
        let window = change_window(before, after, &diff(before, after)).unwrap();
        assert_eq!(window.before, "...three four five six seven eight nine...");
        assert_eq!(window.after, "...three four five SIX seven eight nine...");
        assert_eq!(window.more, 0);
    }

    #[test]
    fn the_window_says_how_many_more_changes_the_line_holds() {
        let before = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let after = "ALPHA beta gamma delta epsilon zeta eta theta iota kappa lambda MU";
        let window = change_window(before, after, &diff(before, after)).unwrap();
        assert_eq!(window.before, "alpha beta gamma delta...");
        assert_eq!(window.more, 1);
    }

    #[test]
    fn a_short_line_needs_no_cuts() {
        let window =
            change_window("total due", "total owed", &diff("total due", "total owed")).unwrap();
        assert_eq!(
            (window.before.as_str(), window.after.as_str()),
            ("total due", "total owed")
        );
    }

    #[test]
    fn an_insertion_shows_where_it_went() {
        let before = "one two three four five";
        let after = "one two three NEW four five";
        let window = change_window(before, after, &diff(before, after)).unwrap();
        assert_eq!(window.before, "one two three four five");
        assert_eq!(window.after, "one two three NEW four five");
    }

    #[test]
    fn very_long_lines_are_not_compared_by_word() {
        let long = "x ".repeat(WORD_PASS_MAX_CHARS);
        let d = diff(&long, "x");
        assert!(d.too_long());
        assert!(change_window(&long, "x", &d).is_none());
        assert!(change_window("same", "same", &diff("same", "same")).is_none());
    }
}
