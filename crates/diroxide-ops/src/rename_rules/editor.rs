//! The second door: the names edited in the owner's editor. The plan's
//! new names go out as a text, one a line in the given order, and come
//! back as row choices against the same plan, so the same preview, the
//! same confirmation and the same job follow. A line removed never
//! deletes a file and a line added never makes one: a changed count is
//! refused in words and nothing changes.

use super::plan::{Choices, RenamePlan};

/// Why a text of names cannot go out or come back.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditorRefused {
    /// Some names hold a line break, a tab or another control
    /// character, which a line of text cannot show.
    #[error("Cannot edit: {count} hold a line break or tab")]
    CannotShow {
        /// How many names, in words: "2 names".
        count: String,
        /// The names, spelled out.
        names: Vec<String>,
    },
    /// The text has more or fewer lines than the plan has rows.
    #[error("Names not read: {lines} for {rows}")]
    Count {
        /// The lines, in words: "5 lines".
        lines: String,
        /// The rows, in words: "6 files".
        rows: String,
    },
    /// A line is empty, so a name would be empty.
    #[error("Names not read: line {0} is empty")]
    Empty(usize),
}

impl EditorRefused {
    /// The refusal in one line, meaning first.
    pub fn line(&self) -> String {
        self.to_string()
    }

    /// What to do about it, in a sentence.
    pub fn next_step(&self) -> String {
        match self {
            EditorRefused::CannotShow { .. } => {
                "Rename those with rules, or skip them first.".to_owned()
            }
            EditorRefused::Count { .. } => {
                "Keep one line for each file, in the same order, and try again.".to_owned()
            }
            EditorRefused::Empty(_) => "Type a name on that line, or skip the row.".to_owned(),
        }
    }
}

/// Whether a name can stand on a line of its own.
fn showable(name: &str) -> bool {
    !name.chars().any(char::is_control)
}

/// The plan's new names as a text for the editor: one a line, in the
/// given order, each line ending in a line break. A skipped row's line is
/// its old name. Refused when a name could not be shown on a line.
pub fn editor_text(plan: &RenamePlan) -> Result<String, EditorRefused> {
    let rows = plan.rows_in_order();
    let hidden: Vec<String> = rows
        .iter()
        .flat_map(|row| [&row.old, &row.new])
        .filter(|name| !showable(name))
        .map(|name| diroxide_core::names::spell_out(name).into_owned())
        .collect();
    if !hidden.is_empty() {
        return Err(EditorRefused::CannotShow {
            count: diroxide_core::format::count(hidden.len() as u64, "name", "names"),
            names: hidden,
        });
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(&row.new);
        text.push('\n');
    }
    Ok(text)
}

/// Reads the edited text back against the plan it was made from: one
/// line a row, in the same order, a carriage return before a line break
/// ignored and one final empty line allowed. Every line that differs from
/// the row's new name becomes the owner's own name for that row; a line
/// put back to the old name leaves the row unchanged; a skipped row whose
/// line changed is taken back. The result is the plan's choices with
/// these edits, to plan again with [`super::plan_rename_with`].
pub fn read_editor_text(plan: &RenamePlan, text: &str) -> Result<Choices, EditorRefused> {
    let rows = plan.rows_in_order();
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    let lines = match lines.split_last() {
        Some((&"", rest)) => rest.to_vec(),
        _ => lines,
    };
    if lines.len() != rows.len() {
        return Err(EditorRefused::Count {
            lines: diroxide_core::format::count(lines.len() as u64, "line", "lines"),
            rows: diroxide_core::format::count(rows.len() as u64, "file", "files"),
        });
    }
    if let Some(at) = lines.iter().position(|line| line.is_empty()) {
        return Err(EditorRefused::Empty(at + 1));
    }
    let mut choices = plan.choices().clone();
    for (row, line) in rows.into_iter().zip(lines) {
        if line == row.new {
            continue;
        }
        choices.skipped.remove(&row.index);
        choices.edit(row.index, line);
    }
    Ok(choices)
}
