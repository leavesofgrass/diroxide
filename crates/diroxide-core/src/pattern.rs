//! Patterns: the first rung of the filter language (R21), used by select
//! by pattern, the name filter and the quick search in alpha.1. The
//! language of alpha.3 grows from this subset, so every text valid here
//! keeps its meaning there.
//!
//! - **Words that contain:** `report 2026` matches names that contain both
//!   words, in any order.
//! - **Globs:** a term with `*`, `?` or `[` matches the whole name: `*.pdf`,
//!   `IMG_????.jpg`, `[a-c]*`.
//! - **Several terms,** separated by commas (or semicolons), match when any
//!   one does: `*.pdf, *.docx`.
//! - **`not`** before a term excludes what it matches: `*.pdf, not draft*`.
//!   A pattern of only `not` terms matches everything else.
//! - **A trailing slash** limits a term to folders: `src/`, or `*/` for
//!   every folder.
//! - **`re:`** makes the whole rest one regular expression, matched
//!   anywhere in the name: `re:^IMG_\d{4}`. `not re:...` excludes.
//!
//! Case is ignored by default. Errors say what needs care in words, each
//! line inside 40 cells: "Pattern needs care: unclosed bracket".

use std::fmt;

use regex::{Regex, RegexBuilder};
use thiserror::Error;

use crate::error::Explain;
use crate::sort::NameCase;

/// The most memory a compiled regular expression may take.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// The deepest nesting a regular expression may have.
const REGEX_NEST_LIMIT: u32 = 64;

/// Why a pattern could not be read. The display is the cause in lowercase
/// words; [`PatternError::line`] is the 40-cell message.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PatternError {
    /// Nothing but spaces, commas and semicolons.
    #[error("the pattern is empty")]
    Empty,
    /// A `[` with no `]` after it.
    #[error("a bracket in \"{0}\" is not closed")]
    UnclosedBracket(String),
    /// The text after `re:` is not a regular expression this program reads.
    #[error("the regular expression could not be read")]
    BadRegex,
    /// The regular expression is too large or too deep to run quickly.
    #[error("the regular expression is too complex")]
    TooComplex,
}

impl PatternError {
    /// The message, the meaning first and inside 40 cells: "Pattern needs
    /// care: unclosed bracket".
    pub fn line(&self) -> &'static str {
        match self {
            PatternError::Empty => "Pattern is empty",
            PatternError::UnclosedBracket(_) => "Pattern needs care: unclosed bracket",
            PatternError::BadRegex => "Pattern needs care: bad expression",
            PatternError::TooComplex => "Pattern needs care: too complex",
        }
    }
}

impl Explain for PatternError {
    fn next_step(&self) -> String {
        match self {
            PatternError::Empty => "Type a word, or a pattern such as *.pdf.".to_owned(),
            PatternError::UnclosedBracket(_) => {
                "Close the bracket, or write [[] for a bracket itself.".to_owned()
            }
            PatternError::BadRegex => {
                "Check the expression after re:, or remove re: to match words.".to_owned()
            }
            PatternError::TooComplex => "Use a shorter expression.".to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
enum Matcher {
    /// Every word must be in the name; lowercase when case is ignored.
    Words(Vec<String>),
    /// A glob or a regular expression.
    Regex(Regex),
    /// Anything: a lone trailing slash, "every folder".
    Any,
}

#[derive(Clone, Debug)]
struct Term {
    negate: bool,
    folders_only: bool,
    matcher: Matcher,
}

impl Term {
    fn hit(&self, name: &str, is_folder: bool, ignore_case: bool) -> bool {
        if self.folders_only && !is_folder {
            return false;
        }
        match &self.matcher {
            Matcher::Any => true,
            Matcher::Regex(regex) => regex.is_match(name),
            Matcher::Words(words) => {
                if ignore_case {
                    let lower = name.to_lowercase();
                    words.iter().all(|w| lower.contains(w.as_str()))
                } else {
                    words.iter().all(|w| name.contains(w.as_str()))
                }
            }
        }
    }
}

/// A compiled pattern. See the module documentation for the language.
#[derive(Clone, Debug)]
pub struct Pattern {
    text: String,
    case: NameCase,
    terms: Vec<Term>,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.case == other.case
    }
}

impl Eq for Pattern {}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Strips a leading `not` word, ignoring case.
fn strip_not(text: &str) -> Option<&str> {
    let head = text.get(..4)?;
    (head.eq_ignore_ascii_case("not ")).then(|| text[4..].trim_start())
}

/// Strips a leading `re:`, ignoring case.
fn strip_re(text: &str) -> Option<&str> {
    let head = text.get(..3)?;
    head.eq_ignore_ascii_case("re:").then(|| &text[3..])
}

fn build_regex(source: &str, case: NameCase) -> Result<Regex, PatternError> {
    RegexBuilder::new(source)
        .case_insensitive(case == NameCase::Ignore)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(REGEX_NEST_LIMIT)
        .build()
        .map_err(|error| match error {
            regex::Error::CompiledTooBig(_) => PatternError::TooComplex,
            _ => PatternError::BadRegex,
        })
}

/// A glob as an anchored regular expression: `*` any run, `?` one
/// character, `[...]` a class (`[!...]` or `[^...]` negated; a `]` first in
/// the class is itself), everything else literal.
fn glob_to_regex(glob: &str) -> Result<String, PatternError> {
    let mut out = String::with_capacity(glob.len() + 8);
    out.push_str("^(?s:");
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '[' => {
                let mut j = i + 1;
                let mut class = String::from("[");
                if matches!(chars.get(j), Some('!' | '^')) {
                    class.push('^');
                    j += 1;
                }
                let start = j;
                let mut closed = false;
                while let Some(&c) = chars.get(j) {
                    if c == ']' && j > start {
                        closed = true;
                        break;
                    }
                    match c {
                        '\\' | '[' | ']' | '&' | '~' | '^' => {
                            class.push('\\');
                            class.push(c);
                        }
                        _ => class.push(c),
                    }
                    j += 1;
                }
                if !closed {
                    return Err(PatternError::UnclosedBracket(glob.to_owned()));
                }
                class.push(']');
                out.push_str(&class);
                i = j;
            }
            c => out.push_str(&regex::escape(c.encode_utf8(&mut [0; 4]))),
        }
        i += 1;
    }
    out.push_str(")$");
    Ok(out)
}

fn parse_term(text: &str, case: NameCase) -> Result<Option<Term>, PatternError> {
    let mut rest = text.trim();
    let negate = match strip_not(rest) {
        Some(after) => {
            rest = after;
            true
        }
        None => false,
    };
    let folders_only = rest.ends_with('/');
    let rest = rest.trim_end_matches('/').trim();
    if rest.is_empty() {
        return Ok(folders_only.then_some(Term {
            negate,
            folders_only,
            matcher: Matcher::Any,
        }));
    }
    let matcher = if rest.contains(['*', '?', '[']) {
        Matcher::Regex(build_regex(&glob_to_regex(rest)?, case)?)
    } else {
        let words = rest
            .split_whitespace()
            .map(|w| match case {
                NameCase::Ignore => w.to_lowercase(),
                NameCase::Respect => w.to_owned(),
            })
            .collect();
        Matcher::Words(words)
    };
    Ok(Some(Term {
        negate,
        folders_only,
        matcher,
    }))
}

impl Pattern {
    /// Reads a pattern, ignoring case.
    pub fn parse(text: &str) -> Result<Self, PatternError> {
        Self::parse_with(text, NameCase::Ignore)
    }

    /// Reads a pattern with a case rule.
    pub fn parse_with(text: &str, case: NameCase) -> Result<Self, PatternError> {
        let trimmed = text.trim();
        let (negate, after_not) = match strip_not(trimmed) {
            Some(after) => (true, after),
            None => (false, trimmed),
        };
        let terms = if let Some(source) = strip_re(after_not) {
            if source.trim().is_empty() {
                return Err(PatternError::Empty);
            }
            vec![Term {
                negate,
                folders_only: false,
                matcher: Matcher::Regex(build_regex(source, case)?),
            }]
        } else {
            let mut terms = Vec::new();
            for piece in trimmed.split([',', ';']) {
                if let Some(term) = parse_term(piece, case)? {
                    terms.push(term);
                }
            }
            terms
        };
        if terms.is_empty() {
            return Err(PatternError::Empty);
        }
        Ok(Self {
            text: trimmed.to_owned(),
            case,
            terms,
        })
    }

    /// The pattern as typed, trimmed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether a name matches: any term without `not` matches (or there
    /// are only `not` terms), and no `not` term does. `is_folder` decides
    /// the terms with a trailing slash.
    pub fn matches(&self, name: &str, is_folder: bool) -> bool {
        let ignore = self.case == NameCase::Ignore;
        let mut positives = self.terms.iter().filter(|t| !t.negate).peekable();
        let wanted =
            positives.peek().is_none() || positives.any(|t| t.hit(name, is_folder, ignore));
        wanted
            && !self
                .terms
                .iter()
                .filter(|t| t.negate)
                .any(|t| t.hit(name, is_folder, ignore))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::BRAILLE_CELLS;

    fn hits(pattern: &str, name: &str) -> bool {
        Pattern::parse(pattern).unwrap().matches(name, false)
    }

    fn folder_hits(pattern: &str, name: &str) -> bool {
        Pattern::parse(pattern).unwrap().matches(name, true)
    }

    #[test]
    fn thirty_patterns() {
        let table: [(&str, &str, bool, bool); 30] = [
            ("report", "Annual Report.pdf", false, true),
            ("report 2026", "2026 report.pdf", false, true),
            ("report 2026", "report 2025.pdf", false, false),
            ("*.pdf", "a.PDF", false, true),
            ("*.pdf", "a.pdf.bak", false, false),
            ("*.pdf, *.docx", "b.docx", false, true),
            ("*.pdf; *.docx", "c.txt", false, false),
            ("IMG_????.jpg", "IMG_0042.jpg", false, true),
            ("IMG_????.jpg", "IMG_42.jpg", false, false),
            ("[a-c]*", "beta", false, true),
            ("[a-c]*", "delta", false, false),
            ("[!a-c]*", "delta", false, true),
            ("[]]x", "]x", false, true),
            ("*.pdf, not draft*", "draft-1.pdf", false, false),
            ("*.pdf, not draft*", "final.pdf", false, true),
            ("not *.tmp", "notes.txt", false, true),
            ("not *.tmp", "x.tmp", false, false),
            ("src/", "src", true, true),
            ("src/", "src", false, false),
            ("*/", "anything", true, true),
            ("*/", "file.txt", false, false),
            ("/", "folder", true, true),
            ("re:^IMG_\\d{4}", "IMG_2026.jpg", false, true),
            ("re:^IMG_\\d{4}", "my IMG_2026.jpg", false, false),
            ("RE:\\.(jpe?g|png)$", "a.JPEG", false, true),
            ("not re:^\\.", ".profile", false, false),
            ("not re:^\\.", "profile", false, true),
            ("a+b (1).txt", "a+b (1).txt", false, true),
            ("*(1)*", "a (1).txt", false, true),
            ("dollar$", "dollar$sign", false, true),
        ];
        for (pattern, name, folder, expected) in table {
            let said = if folder {
                folder_hits(pattern, name)
            } else {
                hits(pattern, name)
            };
            assert_eq!(said, expected, "{pattern} against {name}");
        }
    }

    #[test]
    fn case_can_be_respected() {
        let pattern = Pattern::parse_with("*.PDF", NameCase::Respect).unwrap();
        assert!(pattern.matches("A.PDF", false));
        assert!(!pattern.matches("a.pdf", false));
    }

    #[test]
    fn mistakes_are_said_in_words_inside_40_cells() {
        let cases = [
            ("", PatternError::Empty),
            (" , ; ", PatternError::Empty),
            ("re:", PatternError::Empty),
            ("[abc", PatternError::UnclosedBracket("[abc".into())),
            ("re:(unclosed", PatternError::BadRegex),
        ];
        for (text, expected) in cases {
            let error = Pattern::parse(text).unwrap_err();
            assert_eq!(error, expected, "{text:?}");
            assert!(error.line().chars().count() <= BRAILLE_CELLS);
            assert!(!error.next_step().is_empty());
        }
        let huge = format!("re:{}", "(a|b)".repeat(20_000));
        assert!(matches!(
            Pattern::parse(&huge),
            Err(PatternError::TooComplex | PatternError::BadRegex)
        ));
    }

    #[test]
    fn the_text_is_kept_as_typed() {
        let pattern = Pattern::parse("  *.pdf, not draft*  ").unwrap();
        assert_eq!(pattern.to_string(), "*.pdf, not draft*");
        assert_eq!(pattern, Pattern::parse("*.pdf, not draft*").unwrap());
    }
}
