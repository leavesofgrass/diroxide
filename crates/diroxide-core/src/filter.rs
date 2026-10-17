//! The filter language (R21, plan question 71): one grammar for the
//! panel filter, select by pattern, search, saved searches and the
//! tidy-up finders. It grows from the first rung in [`crate::pattern`],
//! so a plain word, a glob, a comma list, `re:` and a slash at either end
//! keep their meaning.
//!
//! **Shape.** Tests joined by `and`, `or` and `not`, with brackets. A
//! space between tests is and. Mixing and with or at one level without
//! brackets is an error that says so: no implicit precedence, because
//! Everything, Windows Search and programming languages disagree on it.
//! The pipe and the ampersand are refused with the word to use. The
//! words of the language ignore case (`AND` works).
//!
//! **Names.** A bare word matches names that contain it; a word with
//! `*`, `?` or `[` is a glob for the whole name. A comma joins values of
//! one test (`*.txt, *.md`, `ext pdf, docx`). A slash at either end means
//! folders (`build/`). `re:` reads a regular expression. Smart case: case
//! matters only when the text has a capital letter, and `case` or
//! `anycase` before a test decides it. Double quotes hold spaces and
//! operators, with a doubled quote for a quote; a name that starts with a
//! bracket or an exclamation mark, or is a word of the language, must be
//! quoted.
//!
//! **Fields.** `name`, `path`, `ext` (or `extension`), `kind`, `type`,
//! `group`, `label`, `owner`, `usergroup`, `size`, `depth`, `modified`,
//! `created`, `accessed`, `changed` and `content`, each with an optional
//! colon (`ext:pdf`) and comparison words or signs: `size over 100 MB`,
//! `size>100mb`, `modified within 3 days`, `modified last week`. KB, MB,
//! GB and TB are powers of 1,000; KiB, MiB, GiB and TiB powers of 1,024;
//! a bare k, m or g is refused. Dates are written year first or spelled.
//!
//! **Yes-or-no words.** `hidden`, `read-only`, `executable`, `system`,
//! `empty`, `broken`, `online-only`, `needs-care`, `selected`,
//! `bookmarked`, `labeled`.
//!
//! **What changed from the first rung.** A comma binds tighter than a
//! space, so `report 2026, *.pdf` is report and either 2026 or a PDF;
//! brackets group, so a name that starts with one is quoted; signs such
//! as `=` are operators outside quotes.

mod complete;
mod dates;
mod error;
mod eval;
mod lint;
mod parse;
mod readback;
mod token;
mod tree;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod truth_tests;

use crate::clock::{Clock, Weekday};
use crate::pattern::CaseRule;

pub use complete::{Candidate, CompletionContext, Completions, complete};
pub use dates::{
    CalendarDate, DateContext, DateProblem, DateValue, DateWord, Duration, Period, Precision,
    TimeUnit, month_from_name, parse_duration, parse_iso,
};
pub use error::{
    Expected, FilterError, KINDS_SENTENCE, Keyword, PROBLEM_START, Problem, UNITS_SENTENCE,
};
pub use eval::{ContentQuery, Evaluator, Lookup, NoLookup, Tally, Truth, Unknown};
pub use lint::{Lint, LintContext, LintKind};
pub use readback::ReadBack;
pub use tree::{
    Amount, ContentOp, ContentTest, DateOp, DateTest, Decimal, Expr, Field, FieldShape, Flag,
    KIND_WORDS, KindTest, NumberOp, NumberTest, SizeUnit, Spot, Test, TextOp, TextTest,
    kind_from_word, kind_word,
};

/// What a filter is read with: the `filter_case` and `week_starts_on`
/// settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FilterOptions {
    /// How case decides a match when a test does not say.
    pub case: CaseRule,
    /// The first day of the week, for this week and last week.
    pub week_start: Weekday,
}

impl Default for FilterOptions {
    /// Smart case, and weeks that start on Sunday.
    fn default() -> Self {
        Self {
            case: CaseRule::Smart,
            week_start: Weekday::Sunday,
        }
    }
}

/// A filter, read and checked: the text as typed, its tree, and its
/// patterns compiled once.
#[derive(Clone, Debug)]
pub struct Filter {
    text: String,
    expr: Expr,
    options: FilterOptions,
    plan: eval::Node,
}

impl PartialEq for Filter {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.options == other.options
    }
}

impl Eq for Filter {}

impl Filter {
    /// Reads a filter with smart case and weeks from Sunday.
    pub fn parse(text: &str) -> Result<Self, FilterError> {
        Self::parse_with(text, &FilterOptions::default())
    }

    /// Reads a filter with the owner's settings.
    pub fn parse_with(text: &str, options: &FilterOptions) -> Result<Self, FilterError> {
        let trimmed = text.trim();
        let lead = text.len() - text.trim_start().len();
        let expr = parse::parse(trimmed, options).map_err(|error| shift(error, lead))?;
        // Every pattern compiled while reading, so this cannot fail; a
        // failure is still said in words rather than a panic.
        let plan = eval::compile(&expr, true).map_err(|error| {
            let problem = parse::pattern_problem(&error, trimmed);
            FilterError::new(problem, 1, lead..lead + trimmed.len(), Vec::new())
        })?;
        Ok(Self {
            text: trimmed.to_owned(),
            expr,
            options: *options,
            plan,
        })
    }

    /// The filter ready to test entries, its date words and durations
    /// read against the clock once.
    pub fn evaluator(&self, clock: &dyn Clock) -> Evaluator {
        self.evaluator_at(self.dates(clock))
    }

    /// The filter ready to test entries at a moment already read.
    pub fn evaluator_at(&self, dates: DateContext) -> Evaluator {
        Evaluator::new(&self.plan, &dates)
    }

    /// What probably was not meant, each said once after the typing
    /// pause: two globs one name cannot match, a word nearly a field,
    /// tests that never all hold, a label not in the list, a backward
    /// range.
    pub fn lints(&self, context: &LintContext<'_>, clock: &dyn Clock) -> Vec<Lint> {
        lint::lints(&self.expr, &self.options, context, self.dates(clock))
    }

    /// The problem with this filter in a one-folder panel, where a depth
    /// test means nothing: "Filter problem: depth works in searches".
    pub fn panel_problem(&self) -> Option<FilterError> {
        self.expr.tests().into_iter().find_map(|(test, at)| {
            (test.field() == Some(Field::Depth))
                .then(|| FilterError::new(Problem::DepthInPanel, at.word(), at.span(), Vec::new()))
        })
    }

    /// Whether a test reads files, so the panel asks before it starts and
    /// again for online-only files.
    pub fn reads_content(&self) -> bool {
        self.expr
            .tests()
            .iter()
            .any(|(test, _)| matches!(test, Test::Content(_)))
    }

    /// The text as typed, trimmed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The tree.
    pub fn expr(&self) -> &Expr {
        &self.expr
    }

    /// The settings it was read with.
    pub fn options(&self) -> &FilterOptions {
        &self.options
    }

    /// The canonical sentence, without the clock: date words are not
    /// followed by their real days. It reads back to the same tree.
    pub fn sentence(&self) -> String {
        readback::read_back(&self.expr, &self.options, None)
            .sentence()
            .to_owned()
    }

    /// Say filter, against the clock: the sentence with each date word's
    /// real days ("modified last week, October 4 to 10"), and the details
    /// line with exact byte counts and the moments durations reach back
    /// to. It reads back to the same tree.
    pub fn read_back(&self, clock: &dyn Clock) -> ReadBack {
        self.read_back_at(self.dates(clock))
    }

    /// Say filter at a moment already read.
    pub fn read_back_at(&self, dates: DateContext) -> ReadBack {
        readback::read_back(&self.expr, &self.options, Some(dates))
    }

    /// The clock read once, with this filter's first day of the week.
    pub fn dates(&self, clock: &dyn Clock) -> DateContext {
        DateContext::new(clock, self.options.week_start)
    }

    /// Whether a `hidden` test shows hidden entries, which a panel that
    /// hides them then lists for this filter; the read-back ends ",
    /// including hidden".
    pub fn shows_hidden(&self) -> bool {
        readback::shows_hidden(&self.expr)
    }
}

/// An error's place in the trimmed text, moved to the text as typed.
fn shift(error: FilterError, lead: usize) -> FilterError {
    let span = error.span();
    let moved = (span.start + lead)..(span.end + lead);
    FilterError::new(
        error.problem().clone(),
        error.word(),
        moved,
        error.expected().to_vec(),
    )
}
