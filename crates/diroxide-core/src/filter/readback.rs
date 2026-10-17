//! Reading a filter back: one canonical sentence of field words and
//! comparison words, units written out, dates spelled, `match case` where
//! case matters, and brackets only where the strict and-or rule needs
//! them. Typed forms never matter: `size>1mb` and `size over 1 MB` read
//! back the same. The sentence parses back to the same tree.

use crate::sort::NameCase;

use super::FilterOptions;
use super::dates::{DateValue, Precision};
use super::parse::default_op;
use super::token::split_head;
use super::tree::{ContentOp, DateOp, Expr, Field, Flag, NumberOp, Test, TextOp, kind_word};

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
    pub(crate) out: String,
}

impl Writer<'_> {
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
            | DateOp::OnOrBefore(value) => self.date_value(value),
            DateOp::Between(low, high) => {
                self.date_value(low);
                self.out.push_str(" to ");
                self.date_value(high);
            }
            DateOp::Within(duration) | DateOp::OlderThan(duration) => {
                self.out.push_str(&duration.to_string());
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

/// The canonical sentence of a tree.
pub(crate) fn sentence(expr: &Expr, options: &FilterOptions) -> String {
    let mut writer = Writer {
        options,
        out: String::new(),
    };
    writer.expr(expr);
    writer.out
}
