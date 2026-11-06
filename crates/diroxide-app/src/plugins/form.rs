//! A plugin's form (`ui/form`, alpha 4's host-pieces lane, R30's commit
//! 21) as the app's own dialog: a list of setting rows in the Pack and
//! Access dialogs' shape, so no new dialog shape.
//!
//! - **The first row is the default button's**, its words first and the
//!   form's summary after ("Commit: 2 files staged on main"); Enter on it
//!   answers the form. Every other row is a field, its label first and its
//!   value after: "Message: empty", "Sign-off: off", "Password: hidden",
//!   a choice's current choice, a path's text.
//! - **Enter on a field changes it:** a text or path field in the app's
//!   prompt (a path with place completion); a multiline field as a list of
//!   its lines, each typed in the prompt, since the app has no editor of
//!   several lines (the smallest equivalent it has); a choice through a
//!   list of its choices; a checkbox toggles (Space too); a secret in the
//!   masked prompt.
//! - **A secret is masked everywhere:** its row says "hidden" or "empty";
//!   its text is never in a message, a log, a history or a debug line.
//! - **A form about something destructive** names No or Cancel as its
//!   default button: then the first row cancels, and an "OK" row after the
//!   fields answers the form (the shape has no words for that second
//!   button; the host-pieces lane's report asks for them).
//! - The dialog's title is the form's, the plugin's name last (answer
//!   90); the field's own dialogs carry the name too.

use diroxide_core::names::display_name;
use diroxide_core::{ListModel, ListRow, Secret};
use diroxide_plugins::ask::{FieldValue, Form, FormAnswer, FormField};
use serde_json::Value;

use super::CallId;

/// The most characters of a value a row shows; the rest is "...".
const SHOWN_CHARS: usize = 200;

/// One field's value while its form is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldState {
    /// A `text` or `path` field's text.
    Text(String),
    /// A `multiline` field's lines.
    Lines(Vec<String>),
    /// A `choice` field's choice.
    Choice(String),
    /// A `checkbox` field.
    Checked(bool),
    /// A `secret` field: masked everywhere.
    Secret(Secret),
}

/// A plugin's form as its dialog holds it while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormState {
    /// The plugin's call, for the answer.
    pub call: CallId,
    /// The plugin's name, last in its dialogs' titles.
    pub plugin: String,
    /// The form as read.
    pub form: Form,
    /// Each field's value, in the fields' order.
    pub values: Vec<FieldState>,
}

/// A text without control characters, as one line of a prompt holds it.
fn one_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// A value as a row shows it: hidden characters as question marks, cut
/// at [`SHOWN_CHARS`].
fn shown(text: &str) -> String {
    let text = display_name(text);
    if text.chars().count() <= SHOWN_CHARS {
        return text.into_owned();
    }
    let kept: String = text.chars().take(SHOWN_CHARS).collect();
    format!("{kept}...")
}

/// The lines of a multiline default: a text split at its line breaks, or
/// a list of texts.
fn default_lines(default: Option<&Value>) -> Vec<String> {
    match default {
        Some(Value::String(text)) if !text.is_empty() => text.lines().map(one_line).collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(one_line)
            .collect(),
        _ => Vec::new(),
    }
}

/// A default as text: a text as it is, a number or a flag in words.
fn default_text(default: Option<&Value>) -> String {
    match default {
        Some(Value::String(text)) => one_line(text),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

impl FieldState {
    /// The value a field starts with, from its default.
    fn starting(field: &FormField) -> Self {
        let default = field.default.as_ref();
        match field.kind.as_str() {
            "multiline" => FieldState::Lines(default_lines(default)),
            "choice" => {
                let chosen = default
                    .and_then(Value::as_str)
                    .filter(|d| field.choices.iter().any(|c| c == d))
                    .or_else(|| field.choices.first().map(String::as_str))
                    .unwrap_or_default();
                FieldState::Choice(chosen.to_owned())
            }
            "checkbox" => FieldState::Checked(default.and_then(Value::as_bool).unwrap_or(false)),
            "secret" => FieldState::Secret(Secret::new(
                default
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )),
            _ => FieldState::Text(default_text(default)),
        }
    }

    /// The value in words, after the label: "empty", "on", "hidden", the
    /// text, or a multiline field's first line and its count.
    pub fn words(&self) -> String {
        match self {
            FieldState::Text(text) if text.is_empty() => "empty".to_owned(),
            FieldState::Text(text) | FieldState::Choice(text) => shown(text),
            FieldState::Lines(lines) => {
                let Some(first) = lines.iter().find(|l| !l.trim().is_empty()) else {
                    return "empty".to_owned();
                };
                if lines.len() == 1 {
                    shown(first)
                } else {
                    format!("{}, {} lines", shown(first), lines.len())
                }
            }
            FieldState::Checked(true) => "on".to_owned(),
            FieldState::Checked(false) => "off".to_owned(),
            FieldState::Secret(secret) if secret.is_empty() => "empty".to_owned(),
            FieldState::Secret(_) => "hidden".to_owned(),
        }
    }

    /// The value on the wire.
    fn value(&self) -> FieldValue {
        match self {
            FieldState::Text(text) => FieldValue::Text(text.clone()),
            FieldState::Lines(lines) => FieldValue::Text(lines.join("\n")),
            FieldState::Choice(choice) => FieldValue::Choice(choice.clone()),
            FieldState::Checked(on) => FieldValue::Checked(*on),
            FieldState::Secret(secret) => FieldValue::Secret(secret.clone()),
        }
    }
}

/// A row's id for a field.
pub fn field_row(index: usize) -> String {
    format!("field-{index}")
}

/// The field a row's id names.
pub fn row_field(row: &str) -> Option<usize> {
    row.strip_prefix("field-")?.parse().ok()
}

/// A row's id for a line of a multiline field.
pub fn line_row(index: usize) -> String {
    format!("line-{index}")
}

/// The line a row's id names.
pub fn row_line(row: &str) -> Option<usize> {
    row.strip_prefix("line-")?.parse().ok()
}

/// The row that adds a line to a multiline field.
pub const ADD_LINE: &str = "add";

/// The first row: the default button's.
pub const BUTTON: &str = "go";

/// The row that answers a form whose default button cancels.
pub const OK: &str = "ok";

impl FormState {
    /// A form as it opens, every field at its default.
    pub fn new(call: CallId, plugin: &str, form: Form) -> Self {
        let values = form.fields.iter().map(FieldState::starting).collect();
        Self {
            call,
            plugin: plugin.to_owned(),
            form,
            values,
        }
    }

    /// A field's label, meaning first, without a closing colon; its id
    /// when the plugin gave none.
    pub fn label(&self, field: usize) -> String {
        let Some(f) = self.form.fields.get(field) else {
            return String::new();
        };
        let label = f.label.trim().trim_end_matches(':').trim_end();
        if label.is_empty() {
            one_line(&f.id)
        } else {
            label.to_owned()
        }
    }

    /// A field's kind word.
    pub fn kind(&self, field: usize) -> &str {
        self.form.fields.get(field).map_or("", |f| f.kind.as_str())
    }

    /// A field's row: "Message: empty", "Sign-off: off".
    pub fn row_words(&self, field: usize) -> String {
        let value = self
            .values
            .get(field)
            .map_or_else(|| "empty".to_owned(), FieldState::words);
        format!("{}: {value}", self.label(field))
    }

    /// The default button's words, or "OK".
    pub fn button_words(&self) -> String {
        self.form
            .default_button
            .as_deref()
            .map(one_line)
            .map(|w| w.trim().to_owned())
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| "OK".to_owned())
    }

    /// Whether the default button cancels (No or Cancel: a form about
    /// something destructive).
    pub fn button_cancels(&self) -> bool {
        let words = self.button_words().to_lowercase();
        words == "no" || words == "cancel"
    }

    /// The dialog's title, the plugin's name last: "Commit (Git)".
    pub fn title(&self) -> &str {
        &self.form.title
    }

    /// A field's own dialog's title, the plugin's name last: "Message
    /// (Git)".
    pub fn field_title(&self, field: usize) -> String {
        format!("{} ({})", self.label(field), self.plugin)
    }

    /// The form's rows: the default button's, then one a field, then the
    /// OK row when the default button cancels.
    pub fn list(&self) -> ListModel {
        let first = match self.form.summary.as_deref().filter(|s| !s.is_empty()) {
            Some(summary) => format!("{}: {summary}", self.button_words()),
            None => self.button_words(),
        };
        let mut rows = vec![ListRow::new(BUTTON, first)];
        for field in 0..self.form.fields.len() {
            rows.push(ListRow::new(field_row(field), self.row_words(field)));
        }
        if self.button_cancels() {
            rows.push(ListRow::new(OK, "OK"));
        }
        let mut list = ListModel::new(self.form.title.clone()).with_rows(rows);
        list.set_help(if self.button_cancels() {
            "Up and Down move through the fields, Enter changes one, Enter on OK answers, Escape cancels."
        } else {
            "Up and Down move through the fields, Enter changes one, Enter on the first row answers, Escape cancels."
        });
        list
    }

    /// The row the focus starts on: the form's focus field, else the
    /// first row.
    pub fn focus_row(&self) -> String {
        self.form
            .focus
            .as_deref()
            .and_then(|id| self.form.fields.iter().position(|f| f.id == id))
            .map_or_else(|| BUTTON.to_owned(), field_row)
    }

    /// Whether every typed value fits its field's pattern; the first that
    /// does not, with the sentence to say.
    pub fn check(&self) -> Result<(), (usize, String)> {
        for (index, (field, value)) in self.form.fields.iter().zip(&self.values).enumerate() {
            if let FieldState::Text(text) = value {
                field.check_text(text).map_err(|why| (index, why))?;
            }
        }
        Ok(())
    }

    /// The answer: every field's value by its id.
    pub fn answer(&self) -> FormAnswer {
        FormAnswer::Submitted(
            self.form
                .fields
                .iter()
                .zip(&self.values)
                .map(|(field, value)| (field.id.clone(), value.value()))
                .collect(),
        )
    }

    /// A field's text, for its prompt: a text or path field's text.
    pub fn text(&self, field: usize) -> String {
        match self.values.get(field) {
            Some(FieldState::Text(text)) => text.clone(),
            _ => String::new(),
        }
    }

    /// Sets a field's value.
    pub fn set(&mut self, field: usize, value: FieldState) {
        if let Some(slot) = self.values.get_mut(field) {
            *slot = value;
        }
    }

    /// Turns a checkbox over; its new state, or `None` for another kind.
    pub fn toggle(&mut self, field: usize) -> Option<bool> {
        match self.values.get_mut(field) {
            Some(FieldState::Checked(on)) => {
                *on = !*on;
                Some(*on)
            }
            _ => None,
        }
    }

    /// A multiline field's lines.
    pub fn lines(&self, field: usize) -> Vec<String> {
        match self.values.get(field) {
            Some(FieldState::Lines(lines)) => lines.clone(),
            _ => Vec::new(),
        }
    }

    /// Sets one line of a multiline field, or adds it at the end.
    pub fn set_line(&mut self, field: usize, line: Option<usize>, text: &str) {
        if let Some(FieldState::Lines(lines)) = self.values.get_mut(field) {
            let text = one_line(text);
            match line.filter(|l| *l < lines.len()) {
                Some(at) => lines[at] = text,
                None => lines.push(text),
            }
        }
    }

    /// Removes one line of a multiline field; whether it was there.
    pub fn remove_line(&mut self, field: usize, line: usize) -> bool {
        match self.values.get_mut(field) {
            Some(FieldState::Lines(lines)) if line < lines.len() => {
                lines.remove(line);
                true
            }
            _ => false,
        }
    }

    /// A multiline field's lines as a list: "Line 1: Fix the parser", then
    /// "Add a line".
    pub fn lines_list(&self, field: usize) -> ListModel {
        let mut rows: Vec<ListRow> = self
            .lines(field)
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let words = if line.is_empty() {
                    "empty".to_owned()
                } else {
                    shown(line)
                };
                ListRow::new(line_row(i), format!("Line {}: {words}", i + 1))
            })
            .collect();
        rows.push(ListRow::new(ADD_LINE, "Add a line"));
        let mut list = ListModel::new(self.field_title(field)).with_rows(rows);
        list.set_help(
            "Up and Down move through the lines, Enter types one, Delete removes one, Escape goes back to the form.",
        );
        list
    }

    /// A choice field's choices as a list, the focus on its choice.
    pub fn choices_list(&self, field: usize) -> ListModel {
        let choices = self
            .form
            .fields
            .get(field)
            .map(|f| f.choices.clone())
            .unwrap_or_default();
        let rows = choices
            .iter()
            .enumerate()
            .map(|(i, c)| ListRow::new(format!("choice-{i}"), shown(c)))
            .collect();
        let mut list = ListModel::new(self.field_title(field)).with_rows(rows);
        if let Some(FieldState::Choice(now)) = self.values.get(field)
            && let Some(at) = choices.iter().position(|c| c == now)
        {
            list.focus_id(&format!("choice-{at}"));
        }
        list.set_help("Up and Down choose, Enter sets it, Escape keeps it.");
        list
    }

    /// The choice a row of [`Self::choices_list`] names.
    pub fn choice_of(&self, field: usize, row: &str) -> Option<String> {
        let at: usize = row.strip_prefix("choice-")?.parse().ok()?;
        self.form.fields.get(field)?.choices.get(at).cloned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn commit() -> FormState {
        let params = json!({"title": "Commit", "summary": "2 files staged on main",
            "fields": [
                {"id": "message", "label": "Message", "kind": "multiline"},
                {"id": "author", "label": "Author:", "kind": "text", "default": "Ada Example",
                 "pattern": "[A-Za-z ]+", "pattern_message": "Use letters and spaces only"},
                {"id": "branch", "label": "Branch", "kind": "choice", "choices": ["main", "dev"], "default": "dev"},
                {"id": "sign", "label": "Sign-off", "kind": "checkbox"},
                {"id": "where", "label": "Repository", "kind": "path"},
                {"id": "key", "label": "Password", "kind": "secret"}
            ],
            "focus": "message", "default_button": "Commit"});
        let form = diroxide_plugins::ask::read_form(params.as_object().unwrap(), "Git").unwrap();
        FormState::new(CallId::new(1), "Git", form)
    }

    /// The rows read label first, value after, each inside 40 cells; the
    /// first row is the button with the summary; the focus starts on the
    /// form's focus field.
    #[test]
    fn the_rows_say_label_then_value() {
        let state = commit();
        let list = state.list();
        let rows: Vec<&str> = list.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            rows,
            [
                "Commit: 2 files staged on main",
                "Message: empty",
                "Author: Ada Example",
                "Branch: dev",
                "Sign-off: off",
                "Repository: empty",
                "Password: empty",
            ]
        );
        for row in &rows {
            assert!(row.chars().count() <= 40, "{row}");
        }
        assert_eq!(list.title(), "Commit (Git)");
        assert_eq!(state.focus_row(), "field-0");
        assert_eq!(state.field_title(0), "Message (Git)");
    }

    /// A secret says only "hidden", in its row and its debug line; the
    /// answer carries it in its field alone.
    #[test]
    fn a_secret_is_hidden_and_answered_once() {
        let mut state = commit();
        state.set(5, FieldState::Secret(Secret::new("canary-91c2".to_owned())));
        assert_eq!(state.row_words(5), "Password: hidden");
        assert!(!format!("{state:?}").contains("canary"));
        assert!(
            !state
                .list()
                .rows()
                .iter()
                .any(|r| r.label.contains("canary"))
        );
        let wire = state.answer().to_value();
        assert_eq!(wire["values"]["key"], "canary-91c2");
    }

    /// Lines join with line feeds; a checkbox toggles; a pattern is checked
    /// at the answer; a choice is set from its list.
    #[test]
    fn values_change_and_answer_in_their_shapes() {
        let mut state = commit();
        state.set_line(0, None, "Fix the parser");
        state.set_line(0, None, "");
        state.set_line(0, None, "It read one byte too far.");
        assert_eq!(state.row_words(0), "Message: Fix the parser, 3 lines");
        let lines = state.lines_list(0);
        let words: Vec<&str> = lines.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            words,
            [
                "Line 1: Fix the parser",
                "Line 2: empty",
                "Line 3: It read one byte too far.",
                "Add a line"
            ]
        );
        assert!(state.remove_line(0, 2));
        assert_eq!(state.toggle(3), Some(true));
        assert_eq!(state.row_words(3), "Sign-off: on");
        assert_eq!(state.toggle(1), None);
        let choices = state.choices_list(2);
        assert_eq!(choices.focused().map(|r| r.label.as_str()), Some("dev"));
        assert_eq!(state.choice_of(2, "choice-0").as_deref(), Some("main"));
        state.set(1, FieldState::Text("Ada 2".to_owned()));
        assert_eq!(
            state.check(),
            Err((1, "Use letters and spaces only".to_owned()))
        );
        state.set(1, FieldState::Text("Ada Example".to_owned()));
        assert_eq!(state.check(), Ok(()));
        let wire = state.answer().to_value();
        assert_eq!(wire["values"]["message"], "Fix the parser\n");
        assert_eq!(wire["values"]["sign"], true);
        assert_eq!(wire["values"]["branch"], "dev");
        assert_eq!(wire["values"]["where"], "");
    }

    /// A destructive form's first row cancels and OK answers; a form with
    /// no focus starts on the first row; hidden characters show as
    /// question marks.
    #[test]
    fn a_destructive_form_starts_on_cancel() {
        let params = json!({"title": "Discard", "fields": [
            {"id": "why", "label": "Why", "kind": "text", "default": "x\u{202e}y"}
        ], "default_button": "Cancel"});
        let form = diroxide_plugins::ask::read_form(params.as_object().unwrap(), "Git").unwrap();
        let state = FormState::new(CallId::new(2), "Git", form);
        assert!(state.button_cancels());
        assert_eq!(state.focus_row(), BUTTON);
        let rows: Vec<String> = state
            .list()
            .rows()
            .iter()
            .map(|r| r.label.clone())
            .collect();
        assert_eq!(rows, ["Cancel", "Why: x?y", "OK"]);
    }
}
