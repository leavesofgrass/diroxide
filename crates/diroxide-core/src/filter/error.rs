//! Why a filter could not be read, said in two sentences: the first
//! starts "Filter problem: " (16 cells) and carries the meaning in cells
//! 17 to 40; the second names the word and what is expected next, and is
//! said once, after the typing pause.
//!
//! The same expected-next list feeds the error and Tab completion, so the
//! two never disagree (plan section 5.19).

use std::fmt;
use std::ops::Range;

use crate::error::Explain;
#[cfg(test)]
use crate::format::BRAILLE_CELLS;
use crate::pattern::{BACKREFERENCE_ADVICE, LOOK_AROUND_ADVICE};

use super::tree::Field;

/// The start of every first sentence.
pub const PROBLEM_START: &str = "Filter problem: ";

/// The units, as the unknown-unit sentence lists them.
pub const UNITS_SENTENCE: &str = "Units: B, KB, MB, GB, TB, KiB, MiB, GiB, TiB.";

/// The kinds, as the unknown-kind sentence lists them.
pub const KINDS_SENTENCE: &str = "Kinds: folder, file, link, archive, special, item.";

/// A word of the language that joins or negates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Keyword {
    /// `and`.
    And,
    /// `or`.
    Or,
    /// `not`, or `!` before a term.
    Not,
}

impl Keyword {
    /// The word.
    pub const fn word(self) -> &'static str {
        match self {
            Keyword::And => "and",
            Keyword::Or => "or",
            Keyword::Not => "not",
        }
    }
}

/// What could come next at a place in the text: the list the error names
/// and Tab completes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Expected {
    /// The start of a test: a word or pattern, a field, a yes-or-no word,
    /// `not` or a bracket.
    Term,
    /// `and` or `or`; once a level has one, only that one, since mixing
    /// them needs brackets.
    Joiner(Option<Keyword>),
    /// A closing bracket.
    CloseBracket,
    /// One of a field's comparison words.
    Comparison(Field),
    /// A field's value: a number, a date or date word, a kind, a group, a
    /// type, a label.
    Value(Field),
    /// A size unit after a number.
    SizeUnit,
    /// A time unit after a number: days, weeks.
    TimeUnit,
    /// Week, month or year, after `this` or `last`.
    Period,
    /// The `and` (or `to`) between two ends of a range.
    RangeJoiner,
    /// `match case` or `any case`, after the comma that ends a name, path
    /// or content value.
    CaseNote,
    /// `selected`, after `state`.
    Selected,
    /// The rest of a phrase: `least` or `most` after `at`.
    Words(&'static [&'static str]),
}

/// Why a filter could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Problem {
    /// Nothing but spaces.
    Empty,
    /// More than 10,000 characters.
    TooLong,
    /// Brackets nested more than 32 deep.
    TooDeep,
    /// A `"` with no closing `"`.
    UnclosedQuote,
    /// A `(` with no `)`.
    UnclosedBracket,
    /// A `)` with no `(`.
    ExtraBracket,
    /// `()` with nothing inside.
    EmptyBrackets,
    /// A pipe outside quotes: the word is or.
    Pipe,
    /// An ampersand outside quotes: the word is and.
    Ampersand,
    /// `and`, `or` or `not` at the end, or before a closing bracket.
    NothingAfter(Keyword),
    /// `and` or `or` at the start.
    NothingBefore(Keyword),
    /// `and` and `or` at one level without brackets.
    MixedAndOr,
    /// A word used as a field that is not one.
    UnknownField {
        /// The word.
        word: String,
        /// The nearest field, when one is close.
        nearest: Option<Field>,
    },
    /// A field with nothing usable after it.
    NeedsValue(Field),
    /// A size with a unit that is not one.
    UnknownUnit(String),
    /// A bare k, m, g or t, which could mean 1,000 or 1,024.
    BareUnit(char),
    /// A unit after a depth.
    DepthHasNoUnit,
    /// A number too large to hold.
    NumberTooLarge,
    /// A range with no `and` (or `to`) between its ends.
    RangeNeedsAnd,
    /// A date that could not be read.
    DateNotUnderstood,
    /// A date written with the year last.
    YearFirst,
    /// A month or day that does not exist.
    NoSuchDay,
    /// A number with no time unit after within or older than.
    NeedsTimeUnit,
    /// A duration with no within or older than.
    WithinOrOlder,
    /// within or older than with no duration.
    NeedsDuration,
    /// A regular expression the regex crate refused.
    BadRegex(String),
    /// A regular expression that looks ahead or behind.
    LookAround,
    /// A regular expression that repeats an earlier match.
    Backreference,
    /// A pattern longer than 1,000 characters.
    PatternTooLong,
    /// A pattern too large to run quickly.
    PatternTooComplex,
    /// A glob with a `[` that is not closed.
    GlobBracket,
    /// A comma after a test that takes no list.
    CommaNotHere,
    /// `case` or `anycase` before a test of something that has no case.
    CaseNeedsText,
    /// A kind word that is not one.
    UnknownKind(String),
    /// A group name that is not one.
    UnknownGroup {
        /// The word.
        word: String,
        /// The nearest group, when one is close.
        nearest: Option<&'static str>,
    },
    /// `state` with anything but selected.
    StateIsSelected,
    /// A depth test in a one-folder panel.
    DepthInPanel,
    /// A comparison sign with no field before it.
    SignNeedsField(&'static str),
}

impl Problem {
    /// The first sentence's meaning, after "Filter problem: ".
    fn meaning(&self) -> String {
        match self {
            Problem::Empty => "the filter is empty".to_owned(),
            Problem::TooLong => "filter too long".to_owned(),
            Problem::TooDeep => "too many brackets".to_owned(),
            Problem::UnclosedQuote => "unclosed quote".to_owned(),
            Problem::UnclosedBracket => "unclosed bracket".to_owned(),
            Problem::ExtraBracket => "extra closing bracket".to_owned(),
            Problem::EmptyBrackets => "empty brackets".to_owned(),
            Problem::Pipe => "write or, not |".to_owned(),
            Problem::Ampersand => "write and, not &".to_owned(),
            Problem::NothingAfter(word) => format!("nothing after {}", word.word()),
            Problem::NothingBefore(word) => format!("nothing before {}", word.word()),
            Problem::MixedAndOr => "and mixed with or".to_owned(),
            Problem::UnknownField { word, .. } => format!("unknown field '{word}'"),
            Problem::NeedsValue(field) => format!("{} needs {}", field.word(), field.needs()),
            Problem::UnknownUnit(word) => format!("unknown unit '{word}'"),
            Problem::BareUnit(letter) => {
                let upper = letter.to_ascii_uppercase();
                format!("write {upper}B or {upper}iB")
            }
            Problem::DepthHasNoUnit => "depth has no unit".to_owned(),
            Problem::NumberTooLarge => "number too large".to_owned(),
            Problem::RangeNeedsAnd => "between needs and".to_owned(),
            Problem::DateNotUnderstood => "date not understood".to_owned(),
            Problem::YearFirst => "write the year first".to_owned(),
            Problem::NoSuchDay => "no such day".to_owned(),
            Problem::NeedsTimeUnit => "time unit needed".to_owned(),
            Problem::WithinOrOlder => "say within or older than".to_owned(),
            Problem::NeedsDuration => "duration needed".to_owned(),
            Problem::BadRegex(_) => "bad regular expression".to_owned(),
            Problem::LookAround => "look-around not allowed".to_owned(),
            Problem::Backreference => "backreference refused".to_owned(),
            Problem::PatternTooLong => "pattern too long".to_owned(),
            Problem::PatternTooComplex => "pattern too complex".to_owned(),
            Problem::GlobBracket => "unclosed [ in pattern".to_owned(),
            Problem::CommaNotHere => "comma not allowed here".to_owned(),
            Problem::CaseNeedsText => "case works on text".to_owned(),
            Problem::UnknownKind(word) => format!("unknown kind '{word}'"),
            Problem::UnknownGroup { word, .. } => format!("unknown group '{word}'"),
            Problem::StateIsSelected => "state means selected".to_owned(),
            Problem::DepthInPanel => "depth works in searches".to_owned(),
            Problem::SignNeedsField(sign) => format!("{sign} needs a field"),
        }
    }

    /// What to do, without the word number.
    fn advice(&self) -> String {
        match self {
            Problem::Empty => "Type a word, or a pattern such as *.pdf.".to_owned(),
            Problem::TooLong => "Use at most 10,000 characters.".to_owned(),
            Problem::TooDeep => "Brackets nest at most 32 deep.".to_owned(),
            Problem::UnclosedQuote => "Add a closing quote.".to_owned(),
            Problem::UnclosedBracket => "Add ) to close it.".to_owned(),
            Problem::ExtraBracket => "Remove it, or add ( before it.".to_owned(),
            Problem::EmptyBrackets => "Put a test inside, or remove them.".to_owned(),
            Problem::Pipe => "Quote it if | is part of a name.".to_owned(),
            Problem::Ampersand => "Quote it if & is part of a name.".to_owned(),
            Problem::NothingAfter(_) => "Add a test, such as *.txt.".to_owned(),
            Problem::NothingBefore(_) => "Start with a test, such as *.txt.".to_owned(),
            Problem::MixedAndOr => "Put brackets around one of them.".to_owned(),
            Problem::UnknownField { nearest, .. } => match nearest {
                Some(field) => format!("Did you mean {}?", field.word()),
                None => "Quote it to match it in names.".to_owned(),
            },
            Problem::NeedsValue(field) => example_for(*field).to_owned(),
            Problem::UnknownUnit(_) => UNITS_SENTENCE.to_owned(),
            Problem::BareUnit(letter) => {
                let upper = letter.to_ascii_uppercase();
                let (decimal, binary) = match upper {
                    'K' => ("1,000", "1,024"),
                    'M' => ("1,000,000", "1,048,576"),
                    'G' => ("1,000,000,000", "1,073,741,824"),
                    _ => ("1,000,000,000,000", "1,099,511,627,776"),
                };
                format!("{upper}B is {decimal} bytes; {upper}iB is {binary}.")
            }
            Problem::DepthHasNoUnit => "Try depth at most 2.".to_owned(),
            Problem::NumberTooLarge => "Use a larger unit, or a smaller number.".to_owned(),
            Problem::RangeNeedsAnd => "Try size between 1 MB and 5 MB.".to_owned(),
            Problem::DateNotUnderstood => "Try today, last week or 2026-10-01.".to_owned(),
            Problem::YearFirst => {
                "Dates are written year, month, day, such as 2026-10-11.".to_owned()
            }
            Problem::NoSuchDay => "Check the month and the day.".to_owned(),
            Problem::NeedsTimeUnit => "Try 3 days, 2 weeks or 1 year.".to_owned(),
            Problem::WithinOrOlder => "Try modified within 3 days.".to_owned(),
            Problem::NeedsDuration => "Try within 3 days or older than 1 year.".to_owned(),
            Problem::BadRegex(reason) => format!("The pattern is not valid: {reason}."),
            Problem::LookAround => LOOK_AROUND_ADVICE.to_owned(),
            Problem::Backreference => BACKREFERENCE_ADVICE.to_owned(),
            Problem::PatternTooLong => "Use at most 1,000 characters.".to_owned(),
            Problem::PatternTooComplex => "Use a shorter pattern.".to_owned(),
            Problem::GlobBracket => "Close it with ], or write [[] for [.".to_owned(),
            Problem::CommaNotHere => "Commas join names, extensions and labels; use or.".to_owned(),
            Problem::CaseNeedsText => "Put case before a name or content test.".to_owned(),
            Problem::UnknownKind(_) => KINDS_SENTENCE.to_owned(),
            Problem::UnknownGroup { nearest, .. } => match nearest {
                Some(group) => format!("Did you mean {group}?"),
                None => "Groups: images, music, video, documents, archives, code, text.".to_owned(),
            },
            Problem::StateIsSelected => "Write selected.".to_owned(),
            Problem::DepthInPanel => "A panel shows one folder; search to go deeper.".to_owned(),
            Problem::SignNeedsField(sign) => format!("Try size {sign} 10 MB."),
        }
    }

    /// The cause in lowercase words, for the "Could not ... : CAUSE" form.
    fn cause(&self) -> String {
        self.meaning()
    }
}

/// An example that shows a field with its value.
fn example_for(field: Field) -> &'static str {
    match field {
        Field::Name => "Try name report.",
        Field::Path => "Try path \"/work/\".",
        Field::Extension => "Try ext pdf.",
        Field::Kind => KINDS_SENTENCE,
        Field::Type => "Try type PDF.",
        Field::Group => "Try group images.",
        Field::Label => "Try label urgent.",
        Field::Owner => "Try owner ada.",
        Field::UserGroup => "Try usergroup staff.",
        Field::Size => "Try size over 10 MB.",
        Field::Depth => "Try depth at most 2.",
        Field::Modified | Field::Created | Field::Accessed | Field::Changed => {
            "Try today, last week or 2026-10-01."
        }
        Field::Content => "Try content:\"total due\".",
    }
}

/// A filter that could not be read: the problem, the word it is at, and
/// what could come there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterError {
    problem: Problem,
    word: usize,
    span: Range<usize>,
    expected: Vec<Expected>,
}

impl FilterError {
    pub(crate) fn new(
        problem: Problem,
        word: usize,
        span: Range<usize>,
        expected: Vec<Expected>,
    ) -> Self {
        Self {
            problem,
            word,
            span,
            expected,
        }
    }

    /// What is wrong.
    pub fn problem(&self) -> &Problem {
        &self.problem
    }

    /// The word it is at, counted from 1; one past the last word when the
    /// filter ends too soon.
    pub fn word(&self) -> usize {
        self.word
    }

    /// Its bytes in the text, empty at the end.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    /// What could come at that word.
    pub fn expected(&self) -> &[Expected] {
        &self.expected
    }

    /// The first sentence, the meaning inside 40 cells: "Filter problem:
    /// size needs a number".
    pub fn line(&self) -> String {
        format!("{PROBLEM_START}{}", self.problem.meaning())
    }

    /// Both sentences, as the status line says them after the pause.
    pub fn sentences(&self) -> String {
        format!("{}. {}", self.line(), self.next_step())
    }
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.problem.cause())
    }
}

impl std::error::Error for FilterError {}

impl Explain for FilterError {
    /// The second sentence: the word, then what to do. "Word 3. Try size
    /// over 10 MB."
    fn next_step(&self) -> String {
        format!("Word {}. {}", self.word, self.problem.advice())
    }
}

/// Whether a problem's first sentence keeps its meaning inside one line of
/// the Braille display, whatever word it quotes: the fixed words come
/// before any quoted word, and lines without one fit whole.
#[cfg(test)]
pub(crate) fn fits_one_line(line: &str) -> bool {
    match line.find('\'') {
        Some(quote) => line[..quote].chars().count() <= BRAILLE_CELLS,
        None => line.chars().count() <= BRAILLE_CELLS,
    }
}
