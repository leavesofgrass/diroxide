//! Say what differs when old and new sound the same (R18, the finding no
//! tool has): spaces, tabs, case, line endings, hidden characters and
//! Unicode forms are invisible to speech, so "Was: total due. Now: total
//! due." is useless. The row says "trailing spaces added" or "hidden
//! character U+200B added" instead.
//!
//! The checks run in R18's order and stop at the first that explains the
//! whole difference: the line ending, space at the end, tabs against
//! spaces, the amount of space, case, hidden or direction characters, the
//! Unicode form, and last characters that sound alike (a no-break space,
//! a curly quote, a Cyrillic letter that looks Latin). Hidden characters
//! are told by the same classes the entry names use
//! (`diroxide_core::names`), with the tab left out, since a tab in a line
//! is ordinary space.

use std::fmt;

use diroxide_core::names::{CharClass, classify};

use crate::forms::same_in_another_form;
use crate::lines::LineEnding;

/// Whether something was added, removed or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Delta {
    /// Present only in the after line.
    Added,
    /// Present only in the before line.
    Removed,
    /// Present in both, differently.
    Changed,
}

impl Delta {
    const fn word(self) -> &'static str {
        match self {
            Delta::Added => "added",
            Delta::Removed => "removed",
            Delta::Changed => "changed",
        }
    }
}

/// Why two versions of a line that differ would sound the same, or nearly.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Only the line ending differs.
    LineEnding {
        /// The before line's ending.
        before: LineEnding,
        /// The after line's ending.
        after: LineEnding,
    },
    /// Only space at the end of the line differs.
    TrailingSpace(Delta),
    /// Tabs became spaces, or spaces became tabs.
    Tabs {
        /// Whether tabs became spaces (else spaces became tabs).
        to_spaces: bool,
        /// How many tabs.
        tabs: usize,
        /// How many spaces each tab stands for, when that is one number.
        spaces_each: Option<usize>,
    },
    /// Only the amount of space differs.
    SpaceAmount {
        /// Whether only the space at the start of the line differs.
        indentation: bool,
    },
    /// Only upper and lower case differ.
    Case,
    /// Hidden or direction characters were added or removed.
    Hidden {
        /// Hidden characters only in the after line, in order.
        added: Vec<char>,
        /// Hidden characters only in the before line, in order.
        removed: Vec<char>,
    },
    /// The same text in another Unicode form (composed against decomposed).
    UnicodeForm,
    /// Characters a reader may not tell apart by ear: the first pair that
    /// differs, and how many more pairs differ.
    SoundsAlike {
        /// The before line's character.
        before: char,
        /// The after line's character.
        after: char,
        /// How many more pairs of characters differ.
        more: usize,
    },
}

/// A character as plan section 5.12 spells it: "U+200B".
pub fn code_point(c: char) -> String {
    format!("U+{:04X}", u32::from(c))
}

fn code_points(chars: &[char]) -> String {
    const SHOWN: usize = 3;
    let shown: Vec<String> = chars.iter().take(SHOWN).map(|&c| code_point(c)).collect();
    let mut text = shown.join(", ");
    if chars.len() > SHOWN {
        text.push_str(&format!(" and {} more", chars.len() - SHOWN));
    }
    text
}

fn hidden_noun(chars: &[char]) -> &'static str {
    let direction = chars.iter().all(|&c| classify(c) == CharClass::Direction);
    match (direction, chars.len() == 1) {
        (true, true) => "direction character",
        (true, false) => "direction characters",
        (false, true) => "hidden character",
        (false, false) => "hidden characters",
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::LineEnding { before, after } => match (before, after) {
                (LineEnding::None, after) => write!(f, "line break {after} added at the end"),
                (before, LineEnding::None) => {
                    write!(f, "line break {before} removed at the end")
                }
                (before, after) => write!(f, "line ending changed from {before} to {after}"),
            },
            Reason::TrailingSpace(delta) => write!(f, "trailing spaces {}", delta.word()),
            Reason::Tabs {
                to_spaces,
                tabs,
                spaces_each,
            } => match (to_spaces, tabs, spaces_each) {
                (true, 1, Some(1)) => f.write_str("tab became 1 space"),
                (true, 1, Some(n)) => write!(f, "tab became {n} spaces"),
                (true, 1, None) => f.write_str("tab became spaces"),
                (true, n, Some(each)) => write!(f, "{n} tabs became {each} spaces each"),
                (true, n, None) => write!(f, "{n} tabs became spaces"),
                (false, 1, Some(n)) => write!(f, "{n} spaces became a tab"),
                (false, 1, None) => f.write_str("spaces became a tab"),
                (false, n, _) => write!(f, "spaces became {n} tabs"),
            },
            Reason::SpaceAmount { indentation: true } => f.write_str("indentation changed"),
            Reason::SpaceAmount { indentation: false } => f.write_str("amount of space changed"),
            Reason::Case => f.write_str("case only"),
            Reason::Hidden { added, removed } => match (added.is_empty(), removed.is_empty()) {
                (false, true) => write!(f, "{} {} added", hidden_noun(added), code_points(added)),
                (true, false) => write!(
                    f,
                    "{} {} removed",
                    hidden_noun(removed),
                    code_points(removed)
                ),
                _ => {
                    let all: Vec<char> = removed.iter().chain(added).copied().collect();
                    let noun = if hidden_noun(&all).starts_with("direction") {
                        "direction characters"
                    } else {
                        "hidden characters"
                    };
                    write!(
                        f,
                        "{noun} changed: {} removed, {} added",
                        code_points(removed),
                        code_points(added)
                    )
                }
            },
            Reason::UnicodeForm => f.write_str("same text in another Unicode form"),
            Reason::SoundsAlike {
                before,
                after,
                more,
            } => {
                write!(
                    f,
                    "sounds the same; {} became {}",
                    code_point(*before),
                    code_point(*after)
                )?;
                if *more > 0 {
                    write!(f, " and {more} more")?;
                }
                Ok(())
            }
        }
    }
}

/// Why two versions of a line sound the same, or `None` when they do not
/// (or do not differ at all). `before` and `after` are the lines without
/// their endings.
pub fn explain(
    before: &str,
    before_ending: LineEnding,
    after: &str,
    after_ending: LineEnding,
) -> Option<Reason> {
    if before == after {
        return (before_ending != after_ending).then_some(Reason::LineEnding {
            before: before_ending,
            after: after_ending,
        });
    }
    trailing(before, after)
        .or_else(|| tabs(before, after))
        .or_else(|| space_amount(before, after))
        .or_else(|| (before.to_lowercase() == after.to_lowercase()).then_some(Reason::Case))
        .or_else(|| hidden(before, after))
        .or_else(|| same_in_another_form(before, after).then_some(Reason::UnicodeForm))
        .or_else(|| sounds_alike(before, after))
}

fn trailing(before: &str, after: &str) -> Option<Reason> {
    let (b, a) = (before.trim_end(), after.trim_end());
    if b != a {
        return None;
    }
    let delta = match (b.len() == before.len(), a.len() == after.len()) {
        (true, false) => Delta::Added,
        (false, true) => Delta::Removed,
        _ => Delta::Changed,
    };
    Some(Reason::TrailingSpace(delta))
}

fn tabs(before: &str, after: &str) -> Option<Reason> {
    let strip = |s: &str| -> String { s.chars().filter(|&c| c != ' ' && c != '\t').collect() };
    if strip(before) != strip(after) {
        return None;
    }
    let count = |s: &str, c: char| s.chars().filter(|&x| x == c).count();
    let (tabs_before, tabs_after) = (count(before, '\t'), count(after, '\t'));
    if tabs_before == tabs_after {
        return None;
    }
    let (spaces_before, spaces_after) = (count(before, ' '), count(after, ' '));
    let to_spaces = tabs_before > tabs_after;
    let tabs = tabs_before.abs_diff(tabs_after);
    let spaces = spaces_before.abs_diff(spaces_after);
    let grew = if to_spaces {
        spaces_after > spaces_before
    } else {
        spaces_before > spaces_after
    };
    let spaces_each = (grew && spaces.is_multiple_of(tabs)).then_some(spaces / tabs);
    Some(Reason::Tabs {
        to_spaces,
        tabs,
        spaces_each,
    })
}

/// Every run of spaces and tabs as one space, and none at the end. Other
/// space characters, such as a no-break space, stay: swapping one in is
/// not more or less space but another character, which the last check
/// names.
fn collapsed(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_space = false;
    for c in line.trim_end_matches([' ', '\t']).chars() {
        if c == ' ' || c == '\t' {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

fn space_amount(before: &str, after: &str) -> Option<Reason> {
    if collapsed(before) != collapsed(after) {
        return None;
    }
    Some(Reason::SpaceAmount {
        indentation: before.trim_start() == after.trim_start(),
    })
}

fn is_hidden(c: char) -> bool {
    c != '\t' && classify(c).is_hidden()
}

fn hidden(before: &str, after: &str) -> Option<Reason> {
    let visible = |s: &str| -> String { s.chars().filter(|&c| !is_hidden(c)).collect() };
    if visible(before) != visible(after) {
        return None;
    }
    let mut removed: Vec<char> = before.chars().filter(|&c| is_hidden(c)).collect();
    let mut added = Vec::new();
    for c in after.chars().filter(|&c| is_hidden(c)) {
        match removed.iter().position(|&r| r == c) {
            Some(at) => {
                removed.remove(at);
            }
            None => added.push(c),
        }
    }
    if added.is_empty() && removed.is_empty() {
        // The same hidden characters, moved: still nothing to hear.
        let moved: Vec<char> = after.chars().filter(|&c| is_hidden(c)).collect();
        return Some(Reason::Hidden {
            added: moved.clone(),
            removed: moved,
        });
    }
    Some(Reason::Hidden { added, removed })
}

/// A character as a reader says it, near enough: every space as a space,
/// dashes as a hyphen, curly quotes as straight ones, full-width forms as
/// their ASCII letters, and the Cyrillic and Greek capitals and letters
/// that look Latin as the Latin ones.
fn spoken(c: char) -> char {
    match u32::from(c) {
        _ if c.is_whitespace() => ' ',
        0x2010..=0x2015 | 0x2212 | 0xFE58 | 0xFE63 => '-',
        0x2018..=0x201B | 0x2032 | 0x02BC => '\'',
        0x201C..=0x201F | 0x2033 => '"',
        code @ 0xFF01..=0xFF5E => char::from_u32(code - 0xFEE0).unwrap_or(c),
        0x0430 => 'a',
        0x0435 => 'e',
        0x043E | 0x03BF => 'o',
        0x0440 => 'p',
        0x0441 => 'c',
        0x0443 => 'y',
        0x0445 => 'x',
        0x0455 => 's',
        0x0456 => 'i',
        0x0458 => 'j',
        0x0501 => 'd',
        0x03BD => 'v',
        0x0410 | 0x0391 => 'A',
        0x0412 | 0x0392 => 'B',
        0x0415 | 0x0395 => 'E',
        0x041A | 0x039A => 'K',
        0x041C | 0x039C => 'M',
        0x041D | 0x0397 => 'H',
        0x041E | 0x039F => 'O',
        0x0420 | 0x03A1 => 'P',
        0x0421 => 'C',
        0x0422 | 0x03A4 => 'T',
        0x0425 | 0x03A7 => 'X',
        0x0405 => 'S',
        0x0406 | 0x0399 => 'I',
        0x0408 => 'J',
        0x0396 => 'Z',
        0x039D => 'N',
        0x03A5 => 'Y',
        _ => c,
    }
}

fn sounds_alike(before: &str, after: &str) -> Option<Reason> {
    let before: Vec<char> = before.chars().collect();
    let after: Vec<char> = after.chars().collect();
    if before.len() != after.len() {
        return None;
    }
    let mut pairs: Vec<(char, char)> = Vec::new();
    for (&b, &a) in before.iter().zip(&after) {
        if b == a {
            continue;
        }
        if spoken(b) != spoken(a) {
            return None;
        }
        if !pairs.contains(&(b, a)) {
            pairs.push((b, a));
        }
    }
    let &(before, after) = pairs.first()?;
    Some(Reason::SoundsAlike {
        before,
        after,
        more: pairs.len() - 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn why(before: &str, after: &str) -> Option<String> {
        explain(before, LineEnding::Lf, after, LineEnding::Lf).map(|r| r.to_string())
    }

    #[test]
    fn each_difference_that_sounds_the_same_is_named() {
        let table = [
            ("total due", "total due  ", "trailing spaces added"),
            ("total due \t", "total due", "trailing spaces removed"),
            ("total due ", "total due\t", "trailing spaces changed"),
            ("\tx = 1", "    x = 1", "tab became 4 spaces"),
            ("\t\tx", "        x", "2 tabs became 4 spaces each"),
            ("    x", "\tx", "4 spaces became a tab"),
            ("a  b", "a b", "amount of space changed"),
            ("  a b", "    a b", "indentation changed"),
            ("Total Due", "total due", "case only"),
            ("total", "to\u{200b}tal", "hidden character U+200B added"),
            (
                "a\u{200e}b\u{feff}",
                "ab",
                "hidden characters U+200E, U+FEFF removed",
            ),
            ("abc", "a\u{202e}bc", "direction character U+202E added"),
            (
                "a\u{200b}b",
                "a\u{200c}b",
                "hidden characters changed: U+200B removed, U+200C added",
            ),
            (
                "caf\u{e9}",
                "cafe\u{301}",
                "same text in another Unicode form",
            ),
            (
                "don't",
                "don\u{2019}t",
                "sounds the same; U+0027 became U+2019",
            ),
            (
                "a b c",
                "a\u{a0}b\u{a0}c",
                "sounds the same; U+0020 became U+00A0",
            ),
            (
                "paypal",
                "p\u{430}yp\u{430}l",
                "sounds the same; U+0061 became U+0430",
            ),
            (
                "a-b \"c\"",
                "a\u{2013}b \u{201c}c\u{201d}",
                "sounds the same; U+002D became U+2013 and 2 more",
            ),
        ];
        for (before, after, expected) in table {
            assert_eq!(why(before, after).as_deref(), Some(expected), "{before:?}");
        }
    }

    #[test]
    fn line_endings_alone_are_named() {
        let crlf = explain("x", LineEnding::CrLf, "x", LineEnding::Lf).unwrap();
        assert_eq!(crlf.to_string(), "line ending changed from CRLF to LF");
        let gone = explain("x", LineEnding::Lf, "x", LineEnding::None).unwrap();
        assert_eq!(gone.to_string(), "line break LF removed at the end");
        let added = explain("x", LineEnding::None, "x", LineEnding::CrLf).unwrap();
        assert_eq!(added.to_string(), "line break CRLF added at the end");
        assert_eq!(explain("x", LineEnding::Lf, "x", LineEnding::Lf), None);
    }

    #[test]
    fn real_changes_get_no_reason() {
        assert_eq!(why("total due", "total owed"), None);
        assert_eq!(why("ab", "a b"), None);
        assert_eq!(why("caf\u{e9}", "cafe\u{300}"), None);
        assert_eq!(why("Total due ", "total due"), None);
    }

    #[test]
    fn a_tab_is_space_not_a_hidden_character() {
        assert!(!is_hidden('\t'));
        assert!(is_hidden('\u{0}'));
        assert_eq!(
            why("a\u{0}b", "ab").as_deref(),
            Some("hidden character U+0000 removed")
        );
    }

    #[test]
    fn long_lists_of_code_points_are_cut() {
        let removed = ['\u{200b}'; 5];
        assert_eq!(
            Reason::Hidden {
                added: Vec::new(),
                removed: removed.to_vec()
            }
            .to_string(),
            "hidden characters U+200B, U+200B, U+200B and 2 more removed"
        );
    }
}
