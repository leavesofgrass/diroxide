//! Text helpers for the rules: characters as a reader perceives them,
//! case forms, the date format, the character classes, and the escape
//! that makes text literal in a pattern.

use diroxide_core::CivilTime;

use super::rule::{CaseForm, CharacterClass, End, Position};

/// Whether a character joins the one before it into one perceived
/// character: combining marks, variation selectors, skin-tone modifiers,
/// emoji tags and the character after a zero-width joiner.
///
/// This is a close reading of Unicode's extended grapheme clusters for
/// the scripts names use most, not the whole algorithm: the
/// `unicode-segmentation` crate is not among the workspace's
/// dependencies (a contract request in the lane's report).
fn extends(c: char) -> bool {
    matches!(u32::from(c),
        0x0300..=0x036F   // combining diacritical marks
        | 0x0483..=0x0489 // Cyrillic combining marks
        | 0x0591..=0x05BD // Hebrew points
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4 | 0x06E7 | 0x06E8 | 0x06EA..=0x06ED // Arabic marks
        | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957
        | 0x0962 | 0x0963 // Devanagari signs and vowel marks
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E // Thai marks
        | 0x1AB0..=0x1AFF // combining diacritical marks extended
        | 0x1DC0..=0x1DFF // combining diacritical marks supplement
        | 0x200C          // zero-width non-joiner
        | 0x200D          // zero-width joiner
        | 0x20D0..=0x20FF // combining marks for symbols
        | 0x3099 | 0x309A // kana voiced marks
        | 0xFE00..=0xFE0F // variation selectors
        | 0xFE20..=0xFE2F // combining half marks
        | 0x1F3FB..=0x1F3FF // skin-tone modifiers
        | 0xE0020..=0xE007F // emoji tags
        | 0xE0100..=0xE01EF // variation selectors supplement
    )
}

/// Whether a character is a regional indicator, two of which make a flag.
fn regional(c: char) -> bool {
    matches!(u32::from(c), 0x1F1E6..=0x1F1FF)
}

/// The text split into perceived characters: `e` with a combining acute
/// is one, a flag is one, a family joined by zero-width joiners is one.
pub(crate) fn graphemes(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut previous: Option<char> = None;
    let mut regional_run = 0usize;
    for (at, c) in text.char_indices() {
        let joins = match previous {
            None => false,
            Some(prev) => {
                extends(c)
                    || prev == '\u{200D}'
                    || (regional(c) && regional(prev) && regional_run % 2 == 1)
            }
        };
        if !joins && at > 0 {
            out.push(&text[start..at]);
            start = at;
        }
        regional_run = if regional(c) { regional_run + 1 } else { 0 };
        previous = Some(c);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The byte offset of a position in the text, counted in perceived
/// characters and clamped to the text.
pub(crate) fn offset_of(text: &str, at: Position) -> usize {
    let parts = graphemes(text);
    let index = match at.from {
        End::Start => at.offset.min(parts.len()),
        End::End => parts.len().saturating_sub(at.offset),
    };
    parts[..index].iter().map(|part| part.len()).sum()
}

/// The text with `count` perceived characters taken out at a position:
/// from the start, the ones after it; from the end, the ones before it.
/// A count of 0 takes all the rest in that direction.
pub(crate) fn remove_at(text: &str, at: Position, count: usize) -> String {
    let parts = graphemes(text);
    let len = parts.len();
    let (from, to) = match at.from {
        End::Start => {
            let from = at.offset.min(len);
            let to = if count == 0 {
                len
            } else {
                from.saturating_add(count).min(len)
            };
            (from, to)
        }
        End::End => {
            let to = len.saturating_sub(at.offset);
            let from = if count == 0 {
                0
            } else {
                to.saturating_sub(count)
            };
            (from, to)
        }
    };
    parts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < from || *i >= to)
        .map(|(_, part)| *part)
        .collect()
}

/// Whether a character starts a new word for title case.
fn splits_words(c: char) -> bool {
    c.is_whitespace() || matches!(c, '_' | '-' | '.')
}

/// The first letter of `word` capital and the rest small.
fn capitalize(word: &str, out: &mut String) {
    let mut chars = word.chars();
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
        out.push_str(&chars.as_str().to_lowercase());
    }
}

/// The text in a case form, with Unicode's full case mappings (`ß`
/// becomes `SS`) and no language's own rules (Turkish dotted and dotless
/// i are not applied).
pub(crate) fn change_case(text: &str, form: CaseForm) -> String {
    match form {
        CaseForm::Lower => text.to_lowercase(),
        CaseForm::Upper => text.to_uppercase(),
        CaseForm::Sentence => {
            let mut out = String::with_capacity(text.len());
            capitalize(text, &mut out);
            out
        }
        CaseForm::Title => {
            let mut out = String::with_capacity(text.len());
            let mut word_start = 0;
            for (at, c) in text.char_indices() {
                if splits_words(c) {
                    capitalize(&text[word_start..at], &mut out);
                    out.push(c);
                    word_start = at + c.len_utf8();
                }
            }
            capitalize(&text[word_start..], &mut out);
            out
        }
    }
}

/// Whether a character belongs to a class.
pub(crate) fn in_class(c: char, class: CharacterClass) -> bool {
    match class {
        CharacterClass::Digits => c.is_numeric(),
        CharacterClass::Spaces => c.is_whitespace(),
        CharacterClass::Punctuation => {
            !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() && !extends(c)
        }
        CharacterClass::NonAscii => !c.is_ascii(),
    }
}

/// The text with every character the regex crate reads as syntax made
/// literal, so plain-text finding runs through the same engine (and the
/// same case folding) as patterns.
pub(crate) fn escape_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '.'
                | '+'
                | '*'
                | '?'
                | '('
                | ')'
                | '|'
                | '['
                | ']'
                | '{'
                | '}'
                | '^'
                | '$'
                | '#'
                | '&'
                | '-'
                | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Replacement text with every dollar sign doubled, so the regex crate
/// writes it as it is.
pub(crate) fn literal_replacement(text: &str) -> String {
    text.replace('$', "$$")
}

/// The date format's letters, longest first so `YYYY` wins over `YY`.
const DATE_TOKENS: [&str; 7] = ["YYYY", "YY", "MM", "DD", "HH", "mm", "ss"];

/// Whether a date format holds at least one of its letters, so a typo
/// such as `{modified:yyyy}` is caught rather than written out.
pub(crate) fn date_format_is_valid(format: &str) -> bool {
    DATE_TOKENS.iter().any(|token| format.contains(token))
}

/// A time in a format of letters repeated for each part: `YYYY` the
/// year, `YY` its last two digits, `MM` the month, `DD` the day, `HH`
/// the hour of 24, `mm` minutes, `ss` seconds. Other characters are kept.
pub(crate) fn format_date(time: &CivilTime, format: &str) -> String {
    let mut out = String::with_capacity(format.len() + 4);
    let mut rest = format;
    'outer: while !rest.is_empty() {
        for token in DATE_TOKENS {
            if let Some(after) = rest.strip_prefix(token) {
                let text = match token {
                    "YYYY" => format!("{:04}", time.year),
                    "YY" => format!("{:02}", time.year.rem_euclid(100)),
                    "MM" => format!("{:02}", time.month),
                    "DD" => format!("{:02}", time.day),
                    "HH" => format!("{:02}", time.hour),
                    "mm" => format!("{:02}", time.minute),
                    _ => format!("{:02}", time.second),
                };
                out.push_str(&text);
                rest = after;
                continue 'outer;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

#[cfg(test)]
mod tests {
    use diroxide_core::{Timestamp, UtcOffset};

    use super::*;

    #[test]
    fn perceived_characters_keep_accents_flags_and_families_whole() {
        assert_eq!(graphemes("cafe\u{301}").len(), 4);
        assert_eq!(graphemes("\u{1F1FA}\u{1F1F8}\u{1F1EB}\u{1F1F7}").len(), 2);
        assert_eq!(
            graphemes("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}x").len(),
            2
        );
        assert_eq!(graphemes("").len(), 0);
    }

    #[test]
    fn removal_counts_from_either_end() {
        assert_eq!(remove_at("abcdef", Position::from_start(1), 2), "adef");
        assert_eq!(remove_at("abcdef", Position::from_end(0), 2), "abcd");
        assert_eq!(remove_at("abcdef", Position::from_end(1), 2), "abcf");
        assert_eq!(remove_at("abcdef", Position::from_start(4), 0), "abcd");
        assert_eq!(remove_at("abcdef", Position::from_end(2), 0), "ef");
        assert_eq!(remove_at("abc", Position::from_start(9), 2), "abc");
        assert_eq!(remove_at("cafe\u{301}s", Position::from_end(1), 1), "cafs");
    }

    #[test]
    fn case_forms_follow_the_words() {
        assert_eq!(
            change_case("o'brien_and-co.final", CaseForm::Title),
            "O'brien_And-Co.Final"
        );
        assert_eq!(
            change_case("hELLO wORLD", CaseForm::Sentence),
            "Hello world"
        );
        assert_eq!(change_case("tschüß", CaseForm::Upper), "TSCHÜSS");
        assert_eq!(
            change_case("  two  spaces", CaseForm::Title),
            "  Two  Spaces"
        );
    }

    #[test]
    fn dates_take_their_letters() {
        let time = Timestamp::from_unix(1_780_000_000, 0).to_civil(UtcOffset::UTC);
        assert_eq!(format_date(&time, "YYYY-MM-DD"), "2026-05-28");
        assert_eq!(format_date(&time, "YY.MM.DD HH-mm-ss"), "26.05.28 20-26-40");
        assert!(date_format_is_valid("YYYY"));
        assert!(!date_format_is_valid("yyyy"));
    }

    #[test]
    fn escaped_text_is_literal() {
        assert_eq!(escape_pattern("a.b(1)$"), "a\\.b\\(1\\)\\$");
        assert_eq!(literal_replacement("$1"), "$$1");
    }
}
