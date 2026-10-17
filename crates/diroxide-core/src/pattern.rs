//! Patterns: the first rung of the filter language (R21), used by select
//! by pattern, the name filter and the quick search in alpha.1, and the
//! building blocks the whole language ([`crate::filter`]) compiles its
//! name, path and content terms with.
//!
//! The first rung, [`Pattern`]:
//!
//! - **Words that contain:** `report 2026` matches names that contain both
//!   words, in any order.
//! - **Globs:** a term with `*`, `?` or `[` matches the whole name: `*.pdf`,
//!   `IMG_????.jpg`, `[a-c]*`.
//! - **Several terms,** separated by commas (or semicolons), match when any
//!   one does: `*.pdf, *.docx`.
//! - **`not`** before a term excludes what it matches: `*.pdf, not draft*`.
//!   A pattern of only `not` terms matches everything else.
//! - **A slash at either end** limits a term to folders: `src/`, `/build`,
//!   or `*/` for every folder.
//! - **`re:`** makes the whole rest one regular expression, matched
//!   anywhere in the name: `re:^IMG_\d{4}`. `not re:...` excludes.
//!
//! Case is ignored by default. Errors say what needs care in words, each
//! line inside 40 cells: "Pattern needs care: unclosed bracket".
//!
//! The building blocks:
//!
//! - [`Glob`]: a glob for a name, where `*` and `?` match any characters,
//!   or for a path, where they never cross a folder separator and `**`
//!   does.
//! - [`compile_regex`]: a regular expression with the limits set (at most
//!   [`MAX_PATTERN_CHARS`] characters, a size and a nesting limit), and
//!   look-around and backreferences refused in words before the regex
//!   crate sees them; [`regex_reason`] says why the crate refused one.
//! - [`CaseRule`]: smart case, the habit of ripgrep, fd and vim: case
//!   matters only when the text has a capital letter.
//! - [`folder_rule`]: the slash at either end of a name pattern.

use std::borrow::Cow;
use std::fmt;

use regex::{Regex, RegexBuilder};
use thiserror::Error;

use crate::error::Explain;
use crate::sort::NameCase;

/// The most memory a compiled regular expression may take.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// The deepest nesting a regular expression may have.
const REGEX_NEST_LIMIT: u32 = 64;

/// The longest regular expression a filter term may hold, in characters.
/// Longer ones are refused ("Pattern needs care: too long"), because
/// counted repetition can make a short-looking pattern huge.
pub const MAX_PATTERN_CHARS: usize = 1_000;

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
    /// The regular expression is longer than [`MAX_PATTERN_CHARS`].
    #[error("the regular expression is longer than 1,000 characters")]
    TooLong,
    /// The regular expression looks ahead or behind, as in `(?=...)` or
    /// `(?<=...)`, which the regex engine cannot do.
    #[error("the regular expression looks ahead or behind")]
    LookAround,
    /// The regular expression repeats text it matched earlier, as in
    /// `(.)\1`, which the regex engine cannot do.
    #[error("the regular expression repeats an earlier match")]
    Backreference,
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
            PatternError::TooLong => "Pattern needs care: too long",
            PatternError::LookAround => "Pattern needs care: look-around",
            PatternError::Backreference => "Pattern needs care: backreference",
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
            PatternError::TooLong => "Use at most 1,000 characters.".to_owned(),
            PatternError::LookAround => LOOK_AROUND_ADVICE.to_owned(),
            PatternError::Backreference => BACKREFERENCE_ADVICE.to_owned(),
        }
    }
}

/// How to do without look-around, in words (`research/bulk-rename.md`).
pub const LOOK_AROUND_ADVICE: &str =
    "Match the text around it too: for a v before a digit, write v\\d.";

/// How to do without a backreference, in words.
pub const BACKREFERENCE_ADVICE: &str = "List the cases instead, such as (aa|bb|cc).";

/// How case decides a match: the `filter_case` setting, and the `case` and
/// `anycase` words of the filter language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaseRule {
    /// Case matters only when the text has a capital letter (the default,
    /// as in ripgrep, fd and vim).
    #[default]
    Smart,
    /// Case never matters.
    Ignore,
    /// Case always matters.
    Match,
}

impl CaseRule {
    /// Every rule, in the order the Settings list offers them.
    pub const ALL: [CaseRule; 3] = [CaseRule::Smart, CaseRule::Ignore, CaseRule::Match];

    /// The rule's word, as the settings file writes it: "smart", "ignore",
    /// "match".
    pub const fn word(self) -> &'static str {
        match self {
            CaseRule::Smart => "smart",
            CaseRule::Ignore => "ignore",
            CaseRule::Match => "match",
        }
    }

    /// The rule a word names, ignoring case.
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|rule| rule.word().eq_ignore_ascii_case(word))
    }

    /// The case a plain word, a glob or a whole name gets under this rule:
    /// under [`CaseRule::Smart`], respected when the text has a capital.
    pub fn resolve(self, text: &str) -> NameCase {
        match self {
            CaseRule::Smart if has_capital(text) => NameCase::Respect,
            CaseRule::Smart | CaseRule::Ignore => NameCase::Ignore,
            CaseRule::Match => NameCase::Respect,
        }
    }

    /// The case a regular expression gets under this rule. Under
    /// [`CaseRule::Smart`] only a capital the pattern matches counts, so
    /// `\D`, `\p{Greek}` and the `U` flag do not make it case-sensitive.
    pub fn resolve_regex(self, source: &str) -> NameCase {
        match self {
            CaseRule::Smart if regex_has_capital(source) => NameCase::Respect,
            CaseRule::Smart | CaseRule::Ignore => NameCase::Ignore,
            CaseRule::Match => NameCase::Respect,
        }
    }
}

/// Whether text has a capital letter, as smart case reads it.
pub fn has_capital(text: &str) -> bool {
    text.chars().any(char::is_uppercase)
}

/// Whether a regular expression has a capital letter it matches: escapes,
/// property names, group names and flags are skipped.
fn regex_has_capital(source: &str) -> bool {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                // An escape: skip the escaped letter, and a property's name.
                i += 1;
                if matches!(chars.get(i), Some('p' | 'P')) && chars.get(i + 1) == Some(&'{') {
                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }
                }
            }
            '(' if chars.get(i + 1) == Some(&'?') => {
                // A group's flags or name: skip to the colon, the closing
                // bracket or the end of the name.
                i += 2;
                if chars.get(i) == Some(&'P') {
                    i += 1;
                }
                if chars.get(i) == Some(&'<') {
                    while i < chars.len() && chars[i] != '>' {
                        i += 1;
                    }
                } else {
                    while i < chars.len() && chars[i] != ':' && chars[i] != ')' {
                        i += 1;
                    }
                }
            }
            c if c.is_uppercase() => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Whether text is a glob: it holds `*`, `?` or `[`.
pub fn is_glob(text: &str) -> bool {
    text.contains(['*', '?', '['])
}

/// The slash rule of a name pattern: a slash at either end limits it to
/// folders, as gitignore's trailing slash does (`build/`, `/build`).
/// Returns the pattern without its slashes, and whether it had one.
pub fn folder_rule(text: &str) -> (&str, bool) {
    let trimmed = text.trim_end_matches('/');
    let trailing = trimmed.len() != text.len();
    let stripped = trimmed.trim_start_matches('/');
    let leading = stripped.len() != trimmed.len();
    (stripped, trailing || leading)
}

/// Where a glob is matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlobScope {
    /// Against one name: `*` and `?` match any characters.
    Name,
    /// Against a whole path: `*` and `?` never cross a folder separator,
    /// `**` crosses any number of folders, and `/` and `\` are the same
    /// separator.
    Path,
}

/// A compiled glob: `*` any run, `?` one character, `[...]` a class
/// (`[!...]` or `[^...]` negated; a `]` first in the class is itself),
/// everything else literal; it matches the whole text.
#[derive(Clone, Debug)]
pub struct Glob {
    text: String,
    case: NameCase,
    scope: GlobScope,
    regex: Regex,
}

impl PartialEq for Glob {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.case == other.case && self.scope == other.scope
    }
}

impl Eq for Glob {}

impl fmt::Display for Glob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Glob {
    /// A glob matched against one name.
    pub fn new(text: &str, case: NameCase) -> Result<Self, PatternError> {
        Self::with_scope(text, case, GlobScope::Name)
    }

    /// A glob matched against a whole path.
    pub fn for_path(text: &str, case: NameCase) -> Result<Self, PatternError> {
        Self::with_scope(text, case, GlobScope::Path)
    }

    /// A glob for a name or a path.
    pub fn with_scope(text: &str, case: NameCase, scope: GlobScope) -> Result<Self, PatternError> {
        let source = match scope {
            GlobScope::Name => glob_to_regex(text)?,
            GlobScope::Path => path_glob_to_regex(&text.replace('\\', "/"))?,
        };
        Ok(Self {
            text: text.to_owned(),
            case,
            scope,
            regex: build_regex(&source, case)?,
        })
    }

    /// The glob as written.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where it is matched.
    pub fn scope(&self) -> GlobScope {
        self.scope
    }

    /// Whether the whole text matches. A path's backslashes count as
    /// slashes.
    pub fn is_match(&self, text: &str) -> bool {
        match self.scope {
            GlobScope::Name => self.regex.is_match(text),
            GlobScope::Path => self.regex.is_match(&forward_slashes(text)),
        }
    }

    /// The compiled regular expression, for callers that match many texts
    /// (a path glob's text must have forward slashes).
    pub fn regex(&self) -> &Regex {
        &self.regex
    }
}

/// A path with its backslashes as slashes, borrowed when it has none.
pub fn forward_slashes(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

/// Compiles a regular expression with diroxide's limits: at most
/// [`MAX_PATTERN_CHARS`] characters, a compiled size and a nesting limit,
/// and look-around and backreferences refused in words. Matching time is
/// then linear in the text, so a pattern can never hang a listing.
pub fn compile_regex(source: &str, case: NameCase) -> Result<Regex, PatternError> {
    if source.chars().count() > MAX_PATTERN_CHARS {
        return Err(PatternError::TooLong);
    }
    if let Some(problem) = unsupported_syntax(source) {
        return Err(problem);
    }
    build_regex(source, case)
}

/// Why the regex crate refused a pattern, in its own words with the
/// pointer lines removed ("unclosed group"), or `None` when it compiles.
/// The filter says it after "Filter problem: bad regular expression".
pub fn regex_reason(source: &str) -> Option<String> {
    match RegexBuilder::new(source)
        .size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(REGEX_NEST_LIMIT)
        .build()
    {
        Ok(_) => None,
        Err(regex::Error::CompiledTooBig(_)) => Some("it is too complex".to_owned()),
        Err(error) => {
            let text = error.to_string();
            let reason = text
                .lines()
                .rev()
                .find_map(|line| line.trim().strip_prefix("error:"))
                .map_or("it could not be read", str::trim);
            Some(reason.to_owned())
        }
    }
}

/// Look-around and backreferences, found before the regex crate sees
/// them so the message can say what they are (`research/bulk-rename.md`).
/// Escaped characters and bracketed classes are skipped.
fn unsupported_syntax(source: &str) -> Option<PatternError> {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut class_depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                match chars.get(i + 1) {
                    Some('1'..='9') if class_depth == 0 => {
                        return Some(PatternError::Backreference);
                    }
                    Some('k') if chars.get(i + 2) == Some(&'<') => {
                        return Some(PatternError::Backreference);
                    }
                    _ => {}
                }
                i += 1;
            }
            '[' => class_depth += 1,
            ']' if class_depth > 0 => class_depth -= 1,
            '(' if class_depth == 0 && chars.get(i + 1) == Some(&'?') => {
                let next = chars.get(i + 2);
                let after = chars.get(i + 3);
                if matches!(next, Some('=' | '!'))
                    || (next == Some(&'<') && matches!(after, Some('=' | '!')))
                {
                    return Some(PatternError::LookAround);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[derive(Clone, Debug)]
enum Matcher {
    /// Every word must be in the name; lowercase when case is ignored.
    Words(Vec<String>),
    /// A glob or a regular expression.
    Regex(Regex),
    /// Anything: a lone slash, "every folder".
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

/// A regular expression built with the size and nesting limits but no
/// length limit, for patterns diroxide writes itself (escaped text).
pub(crate) fn build_regex(source: &str, case: NameCase) -> Result<Regex, PatternError> {
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

/// A glob as an anchored regular expression for one name.
fn glob_to_regex(glob: &str) -> Result<String, PatternError> {
    glob_source(glob, GlobScope::Name)
}

/// A glob as an anchored regular expression for a path with forward
/// slashes: `*` and `?` stay inside one name, `**` crosses folders, and
/// `**/` also matches no folder at all.
fn path_glob_to_regex(glob: &str) -> Result<String, PatternError> {
    glob_source(glob, GlobScope::Path)
}

fn glob_source(glob: &str, scope: GlobScope) -> Result<String, PatternError> {
    let mut out = String::with_capacity(glob.len() + 8);
    out.push_str("^(?s:");
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if scope == GlobScope::Path && chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 2;
                } else {
                    out.push_str(".*");
                    i += 1;
                }
            }
            '*' if scope == GlobScope::Path => out.push_str("[^/]*"),
            '?' if scope == GlobScope::Path => out.push_str("[^/]"),
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
                        '\\' | '[' | ']' | '&' | '~' | '^' | '-'
                            if c != '-' || j == start || chars.get(j + 1) == Some(&']') =>
                        {
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
    let (rest, folders_only) = folder_rule(rest);
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(folders_only.then_some(Term {
            negate,
            folders_only,
            matcher: Matcher::Any,
        }));
    }
    let matcher = if is_glob(rest) {
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
            // The first rung has no length limit of its own, so a pattern
            // over the limit reads as too complex, as it always has.
            let regex = compile_regex(source, case).map_err(|error| match error {
                PatternError::TooLong => PatternError::TooComplex,
                other => other,
            })?;
            vec![Term {
                negate,
                folders_only: false,
                matcher: Matcher::Regex(regex),
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
    /// the terms with a slash at either end.
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

    #[test]
    fn a_leading_slash_also_means_folders() {
        assert!(folder_hits("/build", "build"));
        assert!(!hits("/build", "build"));
        assert_eq!(folder_rule("/build/"), ("build", true));
        assert_eq!(folder_rule("build"), ("build", false));
        assert_eq!(folder_rule("/"), ("", true));
    }

    #[test]
    fn every_limit_has_its_sentence_inside_40_cells() {
        let table = [
            (
                compile_regex(&"a".repeat(MAX_PATTERN_CHARS + 1), NameCase::Ignore),
                PatternError::TooLong,
            ),
            (
                compile_regex("x(?=y)", NameCase::Ignore),
                PatternError::LookAround,
            ),
            (
                compile_regex("x(?!y)", NameCase::Ignore),
                PatternError::LookAround,
            ),
            (
                compile_regex("(?<=x)y", NameCase::Ignore),
                PatternError::LookAround,
            ),
            (
                compile_regex("(?<!x)y", NameCase::Ignore),
                PatternError::LookAround,
            ),
            (
                compile_regex("(.)\\1", NameCase::Ignore),
                PatternError::Backreference,
            ),
            (
                compile_regex("(?<a>.)\\k<a>", NameCase::Ignore),
                PatternError::Backreference,
            ),
            (
                compile_regex("a{1000}{1000}", NameCase::Ignore),
                PatternError::TooComplex,
            ),
            (
                compile_regex("(a", NameCase::Ignore),
                PatternError::BadRegex,
            ),
        ];
        for (said, expected) in table {
            let error = said.unwrap_err();
            assert_eq!(error, expected);
            assert!(error.line().chars().count() <= BRAILLE_CELLS, "{error:?}");
            assert!(!error.next_step().is_empty());
            assert!(!error.to_string().is_empty());
        }
        // Escaped and bracketed look-alikes are plain text, and named
        // groups are not look-behind.
        for fine in ["\\(?=x", "[(?=]", "(?<year>\\d{4})", "(?P<y>a)", "\\\\1?"] {
            assert!(compile_regex(fine, NameCase::Ignore).is_ok(), "{fine}");
        }
        // A pattern of exactly the limit is fine.
        assert!(compile_regex(&"a".repeat(MAX_PATTERN_CHARS), NameCase::Ignore).is_ok());
    }

    #[test]
    fn the_regex_crate_says_why_in_its_own_words() {
        assert_eq!(regex_reason("(a").as_deref(), Some("unclosed group"));
        assert_eq!(regex_reason("a)").as_deref(), Some("unopened group"));
        assert_eq!(regex_reason("a{2,1}").map(|r| r.is_empty()), Some(false));
        assert_eq!(regex_reason("a+"), None);
        let reason = regex_reason("[z-a]").unwrap();
        assert!(!reason.contains('\n'), "{reason}");
    }

    #[test]
    fn smart_case_counts_only_capitals_that_match() {
        assert_eq!(CaseRule::Smart.resolve("report"), NameCase::Ignore);
        assert_eq!(CaseRule::Smart.resolve("Report"), NameCase::Respect);
        assert_eq!(CaseRule::Ignore.resolve("Report"), NameCase::Ignore);
        assert_eq!(CaseRule::Match.resolve("report"), NameCase::Respect);
        assert_eq!(CaseRule::Smart.resolve("ÉTÉ"), NameCase::Respect);
        for plain in ["\\D+", "\\p{Greek}", "(?U)a+", "(?P<Year>\\d+)", "(?<Y>x)"] {
            assert_eq!(
                CaseRule::Smart.resolve_regex(plain),
                NameCase::Ignore,
                "{plain}"
            );
        }
        assert_eq!(CaseRule::Smart.resolve_regex("^IMG_"), NameCase::Respect);
        assert_eq!(CaseRule::Smart.resolve_regex("\\.Rs$"), NameCase::Respect);
        assert_eq!(CaseRule::from_word("SMART"), Some(CaseRule::Smart));
        assert_eq!(CaseRule::from_word("loud"), None);
        for rule in CaseRule::ALL {
            assert_eq!(CaseRule::from_word(rule.word()), Some(rule));
        }
    }

    #[test]
    fn name_globs_match_the_whole_name() {
        let glob = Glob::new("*.TXT", NameCase::Ignore).unwrap();
        assert!(glob.is_match("notes.txt"));
        assert!(!glob.is_match("notes.txt.bak"));
        assert_eq!(glob.to_string(), "*.TXT");
        assert_eq!(glob.scope(), GlobScope::Name);
        let exact = Glob::new("*.TXT", NameCase::Respect).unwrap();
        assert!(!exact.is_match("notes.txt"));
        assert!(Glob::new("[a-", NameCase::Ignore).is_err());
        let dash = Glob::new("[a-]x", NameCase::Ignore).unwrap();
        assert!(dash.is_match("-x"));
        assert!(dash.is_match("ax"));
        let range = Glob::new("[0-9]", NameCase::Ignore).unwrap();
        assert!(range.is_match("7"));
        assert!(!range.is_match("-"));
        assert!(is_glob("IMG_????.jpg"));
        assert!(!is_glob("report"));
    }

    #[test]
    fn path_globs_stay_inside_one_folder_unless_doubled() {
        let one = Glob::for_path("/work/*.rs", NameCase::Ignore).unwrap();
        assert!(one.is_match("/work/main.rs"));
        assert!(!one.is_match("/work/src/main.rs"));
        let deep = Glob::for_path("/work/**/*.rs", NameCase::Ignore).unwrap();
        assert!(deep.is_match("/work/main.rs"));
        assert!(deep.is_match("/work/src/bin/main.rs"));
        assert!(!deep.is_match("/home/main.rs"));
        let any = Glob::for_path("**", NameCase::Ignore).unwrap();
        assert!(any.is_match("a/b/c"));
        let single = Glob::for_path("/work/?.rs", NameCase::Ignore).unwrap();
        assert!(single.is_match("/work/a.rs"));
        assert!(!single.is_match("/work//.rs"));
        // Backslashes are slashes, on both sides.
        let windows = Glob::for_path("D:\\work\\*.txt", NameCase::Ignore).unwrap();
        assert!(windows.is_match("D:\\work\\notes.txt"));
        assert!(windows.is_match("D:/work/notes.txt"));
        assert!(!windows.is_match("D:\\work\\old\\notes.txt"));
        assert_eq!(forward_slashes("a\\b"), "a/b");
    }
}
