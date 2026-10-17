//! Reading a filter back ("Say filter"): one canonical sentence of field
//! words and comparison words, units written out, dates spelled, `match
//! case` where case matters, `including hidden` when a `hidden` test
//! shows hidden entries, and brackets only where the strict and-or rule
//! needs them. Typed forms never matter: `size>1mb` and `size over 1 MB`
//! read back the same. The sentence parses back to the same tree; the
//! exact byte counts go on the details line.

use crate::sort::NameCase;

use super::FilterOptions;
use super::dates::{DateContext, DateValue, Duration, Precision};
use super::parse::default_op;
use super::token::split_head;
use super::tree::{
    Amount, ContentOp, DateOp, Expr, Field, Flag, NumberOp, SizeUnit, Test, TextOp, kind_word,
};

/// The note the read-back ends with when hidden entries are shown.
pub(crate) const HIDDEN_NOTE: &str = ", including hidden";

/// A filter read back: the sentence Say filter says and the title line
/// shows, and the details line with the exact byte counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadBack {
    sentence: String,
    details: Vec<String>,
}

impl ReadBack {
    /// The sentence, meaning first: "size over 100 MB and name matches
    /// *.log". It reads back to the same filter.
    pub fn sentence(&self) -> &str {
        &self.sentence
    }

    /// The details, each a short sentence: "100 MB is 100,000,000 bytes."
    pub fn details(&self) -> &[String] {
        &self.details
    }

    /// The details on one line, or an empty string when there are none.
    pub fn details_line(&self) -> String {
        self.details.join(" ")
    }

    /// The sentence up to its first and or or outside brackets: what one
    /// line of the Braille display must carry.
    pub fn first_clause(&self) -> &str {
        first_clause(&self.sentence)
    }
}

/// A sentence up to its first and or or outside brackets and quotes.
pub(crate) fn first_clause(sentence: &str) -> &str {
    let mut depth = 0usize;
    let mut quoted = false;
    for (i, c) in sentence.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            ' ' if !quoted && depth == 0 => {
                let rest = &sentence[i..];
                if rest.starts_with(" and ") || rest.starts_with(" or ") {
                    return &sentence[..i];
                }
            }
            _ => {}
        }
    }
    sentence
}

/// A whole number with thousands separators: "100,000,000".
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Whether a `hidden` test shows hidden entries: one under an even
/// number of nots.
pub(crate) fn shows_hidden(expr: &Expr) -> bool {
    fn walk(expr: &Expr, negated: bool) -> bool {
        match expr {
            Expr::Test {
                test: Test::Flag(Flag::Hidden),
                ..
            } => !negated,
            Expr::Test { .. } => false,
            Expr::Not(inner) => walk(inner, !negated),
            Expr::And(parts) | Expr::Or(parts) => parts.iter().any(|p| walk(p, negated)),
        }
    }
    walk(expr, false)
}

/// The words the parser reads as part of the language, which a value
/// must be quoted to be.
const KEYWORDS: [&str; 7] = ["and", "or", "not", "case", "anycase", "state", "regex"];

/// Whether a value must be quoted to read back as itself.
pub(crate) fn needs_quotes(value: &str) -> bool {
    value.is_empty()
        || value.chars().any(|c| {
            c.is_whitespace() || matches!(c, '"' | ',' | ';' | '|' | '&' | '<' | '>' | '=')
        })
        || value.starts_with(['(', ')', '!'])
        || value.ends_with(')')
        || KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(value))
        || Field::from_word(value).is_some()
        || Flag::from_word(value).is_some()
        || split_head(value).is_some()
}

/// A value in double quotes, its own quotes doubled.
pub(crate) fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Writes the sentence.
pub(crate) struct Writer<'a> {
    pub(crate) options: &'a FilterOptions,
    pub(crate) dates: Option<DateContext>,
    pub(crate) out: String,
    pub(crate) details: Vec<String>,
}

/// A phrase with its first letter capital: "Last week".
fn capital(phrase: &str) -> String {
    let mut chars = phrase.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl Writer<'_> {
    fn detail(&mut self, detail: String) {
        if !self.details.contains(&detail) {
            self.details.push(detail);
        }
    }

    /// The real days of a date word, on the details line: "Last week is
    /// October 4 to 10."
    fn date_detail(&mut self, value: &DateValue) {
        if let (Some(dates), DateValue::Word(word)) = (self.dates, value) {
            let detail = format!("{} is {}.", capital(word.words()), dates.spelled(*word));
            self.detail(detail);
        }
    }

    /// When a duration ago was, on the details line: "3 days ago:
    /// October 8, 2026, 2:09 PM."
    fn duration_detail(&mut self, duration: &Duration) {
        if let Some(dates) = self.dates {
            let detail = format!("{duration} ago: {}.", dates.moment(dates.ago(duration)));
            self.detail(detail);
        }
    }

    /// Notes an amount's exact byte count for the details line.
    fn count(&mut self, amount: &Amount) {
        let (Some(unit), Some(bytes)) = (amount.unit, amount.value()) else {
            return;
        };
        if unit == SizeUnit::Bytes {
            return;
        }
        self.detail(format!("{amount} is {} bytes.", thousands(bytes)));
    }

    pub(crate) fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Test { test, .. } => self.test(test),
            Expr::Not(inner) => {
                self.out.push_str("not ");
                self.grouped(inner, matches!(**inner, Expr::And(_) | Expr::Or(_)));
            }
            Expr::And(parts) => {
                for (i, part) in parts.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(" and ");
                    }
                    self.grouped(part, matches!(part, Expr::Or(_)));
                }
            }
            Expr::Or(parts) => {
                for (i, part) in parts.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(" or ");
                        if self.short(part) {
                            continue;
                        }
                    }
                    self.grouped(part, matches!(part, Expr::And(_)));
                }
            }
        }
    }

    fn grouped(&mut self, expr: &Expr, bracket: bool) {
        if bracket {
            self.out.push('(');
            self.expr(expr);
            self.out.push(')');
        } else {
            self.expr(expr);
        }
    }

    /// A name test after the first in an or, as its bare word: "name
    /// matches *.txt or *.md".
    fn short(&mut self, part: &Expr) -> bool {
        let Expr::Test {
            test: Test::Text(text),
            ..
        } = part
        else {
            return false;
        };
        if text.field != Field::Name || text.op != default_op(Field::Name, &text.value) {
            return false;
        }
        self.value(&text.value);
        self.case_note(text.field, text.op == TextOp::Regex, &text.value, text.case);
        true
    }

    fn value(&mut self, value: &str) {
        if needs_quotes(value) {
            self.out.push_str(&quoted(value));
        } else {
            self.out.push_str(value);
        }
    }

    /// A regular expression after `regex`, which reads to the next space
    /// and leaves a final comma out.
    fn pattern(&mut self, value: &str) {
        if value.is_empty()
            || value.chars().any(char::is_whitespace)
            || value.starts_with('"')
            || value.ends_with(',')
        {
            self.out.push_str(&quoted(value));
        } else {
            self.out.push_str(value);
        }
    }

    /// ", match case" when case matters; ", any case" when a capital would
    /// have made it matter.
    fn case_note(&mut self, field: Field, regex: bool, value: &str, case: NameCase) {
        if !field.has_case() {
            return;
        }
        let natural = if regex {
            self.options.case.resolve_regex(value)
        } else {
            self.options.case.resolve(value)
        };
        match (case, natural) {
            (NameCase::Respect, _) => self.out.push_str(", match case"),
            (NameCase::Ignore, NameCase::Respect) => self.out.push_str(", any case"),
            (NameCase::Ignore, NameCase::Ignore) => {}
        }
    }

    fn test(&mut self, test: &Test) {
        match test {
            Test::Text(text) => {
                self.out.push_str(text.field.word());
                self.out.push(' ');
                self.out.push_str(text.op.words());
                self.out.push(' ');
                let regex = text.op == TextOp::Regex;
                if regex {
                    self.pattern(&text.value);
                } else {
                    self.value(&text.value);
                }
                self.case_note(text.field, regex, &text.value, text.case);
            }
            Test::Kind(kind) => {
                let is = if kind.is { "is" } else { "is not" };
                self.out
                    .push_str(&format!("kind {is} {}", kind_word(kind.kind)));
            }
            Test::Number(number) => {
                let field = number.field.word();
                match number.op {
                    NumberOp::Between(a, b) => {
                        self.count(&a);
                        self.count(&b);
                    }
                    NumberOp::Is(a)
                    | NumberOp::IsNot(a)
                    | NumberOp::Over(a)
                    | NumberOp::Under(a)
                    | NumberOp::AtLeast(a)
                    | NumberOp::AtMost(a) => self.count(&a),
                }
                let words = match number.op {
                    NumberOp::Is(a) => format!("{field} is {a}"),
                    NumberOp::IsNot(a) => format!("{field} is not {a}"),
                    NumberOp::Over(a) => format!("{field} over {a}"),
                    NumberOp::Under(a) => format!("{field} under {a}"),
                    NumberOp::AtLeast(a) => format!("{field} at least {a}"),
                    NumberOp::AtMost(a) => format!("{field} at most {a}"),
                    NumberOp::Between(a, b) => format!("{field} from {a} to {b}"),
                };
                self.out.push_str(&words);
            }
            Test::Date(date) => {
                self.out.push_str(date.field.word());
                self.out.push(' ');
                self.date_op(&date.op);
            }
            Test::Content(content) => {
                let regex = content.op == ContentOp::Regex;
                self.out.push_str(if regex {
                    "content regex "
                } else {
                    "content contains "
                });
                if regex {
                    self.pattern(&content.text);
                } else {
                    self.value(&content.text);
                }
                if let Some(limit) = content.limit {
                    self.count(&limit);
                    self.out.push_str(&format!(" first {limit}"));
                }
                self.case_note(Field::Content, regex, &content.text, content.case);
            }
            Test::Flag(flag) => self.out.push_str(flag.word()),
        }
    }

    fn date_op(&mut self, op: &DateOp) {
        match op {
            DateOp::On(DateValue::Word(_)) => {}
            DateOp::On(DateValue::Date(date)) => match date.precision() {
                Precision::Year | Precision::Month => self.out.push_str("in "),
                _ => self.out.push_str("on "),
            },
            DateOp::Before(_) => self.out.push_str("before "),
            DateOp::Since(_) => self.out.push_str("since "),
            DateOp::After(_) => self.out.push_str("after "),
            DateOp::OnOrBefore(_) => self.out.push_str("on or before "),
            DateOp::Between(..) => self.out.push_str("from "),
            DateOp::Within(_) => self.out.push_str("within "),
            DateOp::OlderThan(_) => self.out.push_str("older than "),
        }
        match op {
            DateOp::On(value)
            | DateOp::Before(value)
            | DateOp::Since(value)
            | DateOp::After(value)
            | DateOp::OnOrBefore(value) => {
                self.date_value(value);
                // The real days follow the word: "last week, October 4
                // to 10".
                if let (Some(dates), DateValue::Word(word)) = (self.dates, value) {
                    self.out.push_str(", ");
                    self.out.push_str(&dates.spelled(*word));
                }
            }
            DateOp::Between(low, high) => {
                self.date_value(low);
                self.out.push_str(" to ");
                self.date_value(high);
                self.date_detail(low);
                self.date_detail(high);
            }
            DateOp::Within(duration) | DateOp::OlderThan(duration) => {
                self.out.push_str(&duration.to_string());
                self.duration_detail(duration);
            }
        }
    }

    fn date_value(&mut self, value: &DateValue) {
        match value {
            DateValue::Word(word) => self.out.push_str(word.words()),
            DateValue::Date(date) => self.out.push_str(&date.spelled()),
        }
    }
}

/// A tree read back; with a date context, date words are followed by
/// their real days.
pub(crate) fn read_back(
    expr: &Expr,
    options: &FilterOptions,
    dates: Option<DateContext>,
) -> ReadBack {
    let mut writer = Writer {
        options,
        dates,
        out: String::new(),
        details: Vec::new(),
    };
    writer.expr(expr);
    if shows_hidden(expr) {
        writer.out.push_str(HIDDEN_NOTE);
    }
    ReadBack {
        sentence: writer.out,
        details: writer.details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_get_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_048_576), "1,048,576");
        assert_eq!(thousands(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn the_first_clause_stops_at_a_joiner_outside_brackets_and_quotes() {
        assert_eq!(first_clause("a and b"), "a");
        assert_eq!(first_clause("(a or b) and c"), "(a or b)");
        assert_eq!(
            first_clause("name contains \"x and y\" or z"),
            "name contains \"x and y\""
        );
        assert_eq!(first_clause("size over 1 MB"), "size over 1 MB");
    }
}
