//! Reading typed values out of a TOML table, field by field, with every
//! problem named by its field and given a fix: the same house style as
//! the manifest reader, for the keys the script family adds (a script's
//! header, and a manifest-only command's run keys).

use toml::{Table, Value};

use crate::manifest::{Limit, Problem, Severity, parse_duration};

/// Collects problems, each with the label of its field.
pub(crate) struct Reader<'a> {
    table: &'a Table,
    label: &'a dyn Fn(&str) -> String,
    pub(crate) problems: Vec<Problem>,
}

impl<'a> Reader<'a> {
    /// A reader over `table`; `label` names a key's field in a problem
    /// ("timeout (line 4)" in a header, "command[0].timeout" in a
    /// manifest).
    pub(crate) fn new(table: &'a Table, label: &'a dyn Fn(&str) -> String) -> Self {
        Self {
            table,
            label,
            problems: Vec::new(),
        }
    }

    pub(crate) fn field(&self, key: &str) -> String {
        (self.label)(key)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&'a Value> {
        self.table.get(key)
    }

    pub(crate) fn error(&mut self, key: &str, message: &str, fix: &str) {
        let field = self.field(key);
        self.problems.push(Problem {
            severity: Severity::Error,
            field,
            message: message.to_owned(),
            fix: fix.to_owned(),
        });
    }

    pub(crate) fn warn(&mut self, key: &str, message: &str, fix: &str) {
        let field = self.field(key);
        self.problems.push(Problem {
            severity: Severity::Warning,
            field,
            message: message.to_owned(),
            fix: fix.to_owned(),
        });
    }

    pub(crate) fn string(&mut self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::String(s) => Some(s.clone()),
            other => {
                let words = type_words(other);
                self.error(
                    key,
                    &format!("is {words}, not text"),
                    "Put the value in quotes.",
                );
                None
            }
        }
    }

    pub(crate) fn word(&mut self, key: &str, allowed: &[&str]) -> Option<String> {
        let value = self.string(key)?;
        if allowed.contains(&value.as_str()) {
            Some(value)
        } else {
            self.error(
                key,
                &format!("\"{value}\" is not a known value"),
                &format!("Use one of: {}.", allowed.join(", ")),
            );
            None
        }
    }

    pub(crate) fn boolean(&mut self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Boolean(b) => Some(*b),
            other => {
                let words = type_words(other);
                self.error(
                    key,
                    &format!("is {words}, not true or false"),
                    "Write true or false, without quotes.",
                );
                None
            }
        }
    }

    pub(crate) fn integer(&mut self, key: &str, low: i64, high: i64) -> Option<i64> {
        match self.get(key)? {
            Value::Integer(n) if (low..=high).contains(n) => Some(*n),
            Value::Integer(n) => {
                let n = *n;
                self.error(
                    key,
                    &format!("{n} is out of range"),
                    &format!("Use a whole number from {low} to {high}."),
                );
                None
            }
            other => {
                let words = type_words(other);
                self.error(
                    key,
                    &format!("is {words}, not a whole number"),
                    "Write a whole number, without quotes.",
                );
                None
            }
        }
    }

    pub(crate) fn strings(&mut self, key: &str) -> Option<Vec<String>> {
        match self.get(key)? {
            Value::Array(items) => {
                let mut out = Vec::new();
                let mut bad = false;
                for item in items {
                    match item {
                        Value::String(s) => out.push(s.clone()),
                        _ => bad = true,
                    }
                }
                if bad {
                    self.error(
                        key,
                        "holds an item that is not text",
                        "Put each item in quotes.",
                    );
                }
                Some(out)
            }
            Value::String(s) => {
                let fix = format!("Write it as a list: [\"{}\"].", s.replace('"', "'"));
                self.error(key, "is text, not a list", &fix);
                None
            }
            other => {
                let words = type_words(other);
                self.error(
                    key,
                    &format!("is {words}, not a list"),
                    "Write it as a list of quoted items.",
                );
                None
            }
        }
    }

    pub(crate) fn duration(&mut self, key: &str) -> Option<Limit> {
        let text = self.string(key)?;
        match parse_duration(&text) {
            Ok(limit) => Some(limit),
            Err(why) => {
                self.error(
                    key,
                    &format!("\"{text}\" {why}"),
                    "Write a duration such as \"30s\" or \"5m\", or \"none\".",
                );
                None
            }
        }
    }

    pub(crate) fn table(&mut self, key: &str) -> Option<&'a Table> {
        match self.get(key)? {
            Value::Table(t) => Some(t),
            other => {
                let words = type_words(other);
                self.error(
                    key,
                    &format!("is {words}, not a table"),
                    "Write it as an inline table: { 8 = \"words\" }.",
                );
                None
            }
        }
    }
}

/// A value's kind in words, for a problem's message.
pub(crate) fn type_words(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "text",
        Value::Integer(_) => "a number",
        Value::Float(_) => "a decimal number",
        Value::Boolean(_) => "true or false",
        Value::Datetime(_) => "a date",
        Value::Array(_) => "a list",
        Value::Table(_) => "a table",
    }
}
