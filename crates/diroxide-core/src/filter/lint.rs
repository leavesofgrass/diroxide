//! Lints: filters that read, but probably do not say what was meant.
//! Each is said once, after the typing pause, its meaning in the first 40
//! cells: "Both patterns must match one name. Use a comma or or."
//!
//! - Two globs a name cannot match at once, joined by a space or and
//!   (`*.txt *.md`).
//! - A word that is nearly a field, before a value (`sise 10 MB`).
//! - Tests that can never all hold (`kind folder and kind file`, `size
//!   over 10 MB and size under 1 MB`, `hidden and not hidden`).
//! - A label no entry has (`label urgnt`), with the nearest one.
//! - A range written high to low, which reads as low to high.

use crate::entry::EntryKind;
use crate::sort::NameCase;

use super::FilterOptions;
use super::dates::{DateContext, DateWord, TimeUnit};
use super::parse::{nearest, nearest_field};
use super::readback::clause;
use super::tree::{Amount, DateOp, Expr, Field, NumberOp, SizeUnit, Spot, Test, TextOp, TextTest};

/// What a lint is about.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LintKind {
    /// Two globs joined by and that one name cannot both match.
    BothPatterns,
    /// A word that is nearly a field, before a value.
    MaybeField {
        /// The word typed.
        word: String,
        /// The field it is near.
        field: Field,
    },
    /// Tests that can never all hold.
    Contradiction,
    /// A label that is not in the label list.
    NoSuchLabel {
        /// The label typed.
        word: String,
        /// The nearest label, when one is close.
        nearest: Option<String>,
    },
    /// A range written from high to low.
    BackwardRange,
}

/// A lint: the problem, the word it is at, and two sentences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lint {
    kind: LintKind,
    word: usize,
    line: String,
    advice: String,
}

impl Lint {
    fn new(kind: LintKind, word: usize, line: String, advice: String) -> Self {
        Self {
            kind,
            word,
            line,
            advice,
        }
    }

    /// What it is about.
    pub fn kind(&self) -> &LintKind {
        &self.kind
    }

    /// The word it is at, counted from 1.
    pub fn word(&self) -> usize {
        self.word
    }

    /// The first sentence, its meaning inside 40 cells.
    pub fn line(&self) -> &str {
        &self.line
    }

    /// The second sentence: what to do.
    pub fn advice(&self) -> &str {
        &self.advice
    }

    /// Both sentences, as said once after the pause.
    pub fn sentences(&self) -> String {
        format!("{} {}", self.line, self.advice)
    }
}

/// What the lints check against beyond the filter: the label list.
#[derive(Clone, Copy, Debug, Default)]
pub struct LintContext<'a> {
    /// The labels in use; empty when the label list is not known, which
    /// turns the label lint off.
    pub labels: &'a [String],
}

fn sentence_of(expr: &Expr, options: &FilterOptions) -> String {
    clause(expr, options)
}

/// A glob's literal start and end: `IMG_*.jpg` gives `IMG_` and `.jpg`.
fn literal_ends(glob: &str) -> (&str, &str) {
    let first = glob.find(['*', '?', '[']).unwrap_or(glob.len());
    let last = glob.rfind(['*', '?', ']']).map_or(0, |i| i + 1);
    (&glob[..first], &glob[last.max(first).min(glob.len())..])
}

/// Whether one name can match two globs: their literal starts and ends
/// must agree.
fn can_both_match(a: &TextTest, b: &TextTest) -> bool {
    let fold = a.case == NameCase::Ignore || b.case == NameCase::Ignore;
    let norm = |s: &str| {
        if fold { s.to_lowercase() } else { s.to_owned() }
    };
    let (a_start, a_end) = literal_ends(&a.value);
    let (b_start, b_end) = literal_ends(&b.value);
    let (a_start, a_end, b_start, b_end) = (norm(a_start), norm(a_end), norm(b_start), norm(b_end));
    let starts = a_start.starts_with(&b_start) || b_start.starts_with(&a_start);
    let ends = a_end.ends_with(&b_end) || b_end.ends_with(&a_end);
    starts && ends
}

fn name_glob(expr: &Expr) -> Option<(&TextTest, &Spot)> {
    match expr {
        Expr::Test {
            test: Test::Text(text),
            at,
        } if text.field == Field::Name
            && text.op == TextOp::Matches
            && !text.value.starts_with('/')
            && !text.value.ends_with('/') =>
        {
            Some((text, at))
        }
        _ => None,
    }
}

/// A test, or a test under one not, with its word.
fn plain(expr: &Expr) -> Option<(&Test, &Spot, bool)> {
    match expr {
        Expr::Test { test, at } => Some((test, at, false)),
        Expr::Not(inner) => match &**inner {
            Expr::Test { test, at } => Some((test, at, true)),
            _ => None,
        },
        _ => None,
    }
}

/// A number test's inclusive bounds.
fn number_bounds(op: &NumberOp) -> Option<(u64, u64)> {
    let value = |a: &Amount| a.value().unwrap_or(u64::MAX);
    Some(match op {
        NumberOp::Is(a) => (value(a), value(a)),
        NumberOp::IsNot(_) => return None,
        NumberOp::Over(a) => (value(a).checked_add(1)?, u64::MAX),
        NumberOp::Under(a) => (0, value(a).checked_sub(1)?),
        NumberOp::AtLeast(a) => (value(a), u64::MAX),
        NumberOp::AtMost(a) => (0, value(a)),
        NumberOp::Between(a, b) => {
            let (a, b) = (value(a), value(b));
            (a.min(b), a.max(b))
        }
    })
}

/// A date test's stretch of time, from start up to end.
fn date_bounds(op: &DateOp, dates: &DateContext) -> (Option<i64>, Option<i64>) {
    let p = |v| dates.period(v);
    let secs = |t: crate::clock::Timestamp| t.unix_seconds();
    match op {
        DateOp::On(v) => (Some(secs(p(v).start)), Some(secs(p(v).end))),
        DateOp::Before(v) => (None, Some(secs(p(v).start))),
        DateOp::Since(v) => (Some(secs(p(v).start)), None),
        DateOp::After(v) => (Some(secs(p(v).end)), None),
        DateOp::OnOrBefore(v) => (None, Some(secs(p(v).end))),
        DateOp::Between(a, b) => (
            Some(secs(p(a).start.min(p(b).start))),
            Some(secs(p(a).end.max(p(b).end))),
        ),
        DateOp::Within(d) => (Some(secs(dates.ago(d))), None),
        DateOp::OlderThan(d) => (None, Some(secs(dates.ago(d)))),
    }
}

/// Whether two tests, each perhaps under a not, can never both hold.
fn contradict(a: (&Test, bool), b: (&Test, bool), dates: &DateContext) -> bool {
    let ((ta, na), (tb, nb)) = (a, b);
    if na != nb && ta == tb {
        return true;
    }
    if na || nb {
        return false;
    }
    match (ta, tb) {
        (Test::Kind(x), Test::Kind(y)) => {
            let file_like = |k| matches!(k, EntryKind::File | EntryKind::Archive);
            x.is && y.is && x.kind != y.kind && !(file_like(x.kind) && file_like(y.kind))
        }
        (Test::Text(x), Test::Text(y)) => {
            let single = matches!(
                x.field,
                Field::Name | Field::Extension | Field::Type | Field::Group | Field::Owner
            );
            if !single || x.field != y.field || x.op != TextOp::Is || y.op != TextOp::Is {
                return false;
            }
            if x.case == NameCase::Ignore || y.case == NameCase::Ignore {
                x.value.to_lowercase() != y.value.to_lowercase()
            } else {
                x.value != y.value
            }
        }
        (Test::Number(x), Test::Number(y)) if x.field == y.field => {
            match (number_bounds(&x.op), number_bounds(&y.op)) {
                (Some((xl, xh)), Some((yl, yh))) => xh < yl || yh < xl,
                _ => false,
            }
        }
        (Test::Date(x), Test::Date(y)) if x.field == y.field => {
            let (xs, xe) = date_bounds(&x.op, dates);
            let (ys, ye) = date_bounds(&y.op, dates);
            let before = |end: Option<i64>, start: Option<i64>| match (end, start) {
                (Some(end), Some(start)) => end <= start,
                _ => false,
            };
            before(xe, ys) || before(ye, xs)
        }
        _ => false,
    }
}

struct Linter<'a> {
    options: &'a FilterOptions,
    context: &'a LintContext<'a>,
    dates: DateContext,
    found: Vec<Lint>,
}

impl Linter<'_> {
    fn push(&mut self, lint: Lint) {
        if !self.found.contains(&lint) {
            self.found.push(lint);
        }
    }

    fn walk(&mut self, expr: &Expr) {
        match expr {
            Expr::Test { test, at } => self.test(test, at),
            Expr::Not(inner) => self.walk(inner),
            Expr::Or(parts) => {
                for part in parts {
                    self.walk(part);
                }
            }
            Expr::And(parts) => {
                self.and(parts);
                for part in parts {
                    self.walk(part);
                }
            }
        }
    }

    fn and(&mut self, parts: &[Expr]) {
        for (i, a) in parts.iter().enumerate() {
            for b in &parts[i + 1..] {
                if let (Some((ga, at)), Some((gb, _))) = (name_glob(a), name_glob(b)) {
                    if !can_both_match(ga, gb) {
                        self.push(Lint::new(
                            LintKind::BothPatterns,
                            at.word(),
                            "Both patterns must match one name.".to_owned(),
                            "Use a comma or or.".to_owned(),
                        ));
                    }
                    continue;
                }
                // Two globs that can both match are no contradiction.
                let (Some((ta, at, na)), Some((tb, _, nb))) = (plain(a), plain(b)) else {
                    continue;
                };
                if contradict((ta, na), (tb, nb), &self.dates) {
                    let both = format!(
                        "{} and {}",
                        sentence_of(a, self.options),
                        sentence_of(b, self.options)
                    );
                    self.push(Lint::new(
                        LintKind::Contradiction,
                        at.word(),
                        "Nothing can match this filter.".to_owned(),
                        format!("{both} cannot both be true."),
                    ));
                }
            }
        }
    }

    fn test(&mut self, test: &Test, at: &Spot) {
        match test {
            Test::Text(text) if text.field == Field::Label && !self.context.labels.is_empty() => {
                let known = self
                    .context
                    .labels
                    .iter()
                    .any(|label| label.eq_ignore_ascii_case(&text.value));
                let exact = matches!(text.op, TextOp::Is | TextOp::IsNot);
                if known || !exact {
                    return;
                }
                let lower: Vec<String> = self
                    .context
                    .labels
                    .iter()
                    .map(|l| l.to_lowercase())
                    .collect();
                let close = nearest(&text.value, lower.iter().map(String::as_str)).and_then(|n| {
                    self.context
                        .labels
                        .iter()
                        .find(|l| l.to_lowercase() == n)
                        .cloned()
                });
                let advice = match &close {
                    Some(label) => format!("Did you mean {label}?"),
                    None => format!("Labels: {}.", self.context.labels.join(", ")),
                };
                self.push(Lint::new(
                    LintKind::NoSuchLabel {
                        word: text.value.clone(),
                        nearest: close,
                    },
                    at.word(),
                    format!("No label named {}.", text.value),
                    advice,
                ));
            }
            Test::Number(number) => {
                if let NumberOp::Between(low, high) = number.op
                    && low.value() > high.value()
                {
                    self.push(Lint::new(
                        LintKind::BackwardRange,
                        at.word(),
                        "The range runs from high to low.".to_owned(),
                        format!("It is read as from {high} to {low}."),
                    ));
                }
            }
            Test::Date(date) => {
                if let DateOp::Between(low, high) = date.op
                    && self.dates.period(&low).start > self.dates.period(&high).start
                {
                    self.push(Lint::new(
                        LintKind::BackwardRange,
                        at.word(),
                        "The range runs from late to early.".to_owned(),
                        "Both dates count, the earlier first.".to_owned(),
                    ));
                }
            }
            _ => {}
        }
    }

    /// A bare word nearly a field, before a value: `sise 10 MB`.
    fn maybe_fields(&mut self, expr: &Expr) {
        let tests = expr.tests();
        for (test, at) in &tests {
            let Test::Text(text) = test else { continue };
            if text.field != Field::Name || text.op != TextOp::Contains {
                continue;
            }
            let Some(field) = nearest_field(&text.value) else {
                continue;
            };
            let next = tests.iter().find(|(_, spot)| spot.word() == at.word() + 1);
            let value_like = next.is_some_and(|(next, _)| match next {
                Test::Text(t) if t.field == Field::Name => looks_like_value(&t.value),
                _ => false,
            });
            if value_like {
                self.push(Lint::new(
                    LintKind::MaybeField {
                        word: text.value.clone(),
                        field,
                    },
                    at.word(),
                    format!("No field named {}.", text.value),
                    format!("Did you mean {}?", field.word()),
                ));
            }
        }
    }
}

/// Whether a word after a near-field looks like its value: a number, a
/// comparison, a unit, or a date word.
fn looks_like_value(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_digit())
        || [
            "is", "over", "under", "at", "between", "within", "older", "before", "since", "after",
            "on",
        ]
        .iter()
        .any(|w| w.eq_ignore_ascii_case(word))
        || SizeUnit::from_word(word).is_some()
        || TimeUnit::from_word(word).is_some()
        || DateWord::from_one(word).is_some()
}

/// The lints of a tree, in the order of their words.
pub(crate) fn lints(
    expr: &Expr,
    options: &FilterOptions,
    context: &LintContext<'_>,
    dates: DateContext,
) -> Vec<Lint> {
    let mut linter = Linter {
        options,
        context,
        dates,
        found: Vec::new(),
    };
    linter.walk(expr);
    linter.maybe_fields(expr);
    linter.found.sort_by_key(Lint::word);
    linter.found
}
