//! The second door: the names edited in the owner's editor. The plan's
//! new names go out as a text and come back as row choices against the
//! same plan, so the same preview, the same confirmation and the same
//! job follow. A line removed never deletes a file and a line added never
//! makes one: a missing or changed line is refused in words and nothing
//! changes.
//!
//! Two forms (`research/bulk-rename.md`):
//!
//! - **Keyed** (the default): `new name (was old name)`, one line a file.
//!   The editable text comes first and the old name is the key, on the
//!   same line, so lines may be reordered and the old name is there to
//!   compare with: `trip-01.jpg (was IMG_0001.jpg)`.
//! - **Plain**: one new name a line, in the given order, with a strict
//!   line count (ranger's way).

use std::collections::HashMap;

use diroxide_core::format::count;
use diroxide_core::names::spell_out;

use super::plan::{Choices, RenamePlan, RenameRow};

/// The editor text's form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EditorFormat {
    /// `new name (was old name)`, one line a file, in any order.
    #[default]
    Keyed,
    /// One new name a line, in the given order.
    Plain,
}

/// The words between a new name and its key.
const WAS: &str = " (was ";

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
    /// The text has more or fewer lines than the plan has rows (plain
    /// form).
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
    /// A line's `(was ...)` names no file of the plan, or one already
    /// named on an earlier line (keyed form).
    #[error("Names not read: line {0} has no known (was ...)")]
    UnknownKey(usize),
    /// Some files of the plan have no line (keyed form): "2 files have
    /// no line".
    #[error("Names not read: {0}")]
    Missing(String),
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
            EditorRefused::UnknownKey(_) => {
                "Keep each line's (was old name) as it was, and try again.".to_owned()
            }
            EditorRefused::Missing(_) => {
                "Put back a line for each file; to keep a name, leave its line as it is.".to_owned()
            }
        }
    }
}

/// Whether a name can stand on a line of its own.
fn showable(name: &str) -> bool {
    !name.chars().any(char::is_control)
}

/// The plan's new names as a text for the editor, in the given order,
/// each line ending in a line break. A skipped row's new name is its old
/// name. Refused when a name could not be shown on a line.
pub fn editor_text(plan: &RenamePlan, format: EditorFormat) -> Result<String, EditorRefused> {
    let rows = plan.rows();
    let hidden: Vec<String> = rows
        .iter()
        .flat_map(|row| [&row.old, &row.new])
        .filter(|name| !showable(name))
        .map(|name| spell_out(name).into_owned())
        .collect();
    if !hidden.is_empty() {
        return Err(EditorRefused::CannotShow {
            count: count(hidden.len() as u64, "name", "names"),
            names: hidden,
        });
    }
    let mut text = String::new();
    for row in rows {
        text.push_str(&row.new);
        if format == EditorFormat::Keyed {
            text.push_str(WAS);
            text.push_str(&row.old);
            text.push(')');
        }
        text.push('\n');
    }
    Ok(text)
}

/// The text's lines, a carriage return before a line break ignored and
/// one final empty line allowed.
fn lines_of(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    match lines.split_last() {
        Some((&"", rest)) => rest.to_vec(),
        _ => lines,
    }
}

/// Reads the edited text back against the plan it was made from. Every
/// line whose name differs from its row's new name becomes the owner's
/// own name for that row; a name put back to the old one leaves the row
/// unchanged; a skipped row whose name changed is taken back. The result
/// is the plan's choices with these edits, to plan again with
/// [`super::plan_rename_with`].
pub fn read_editor_text(
    plan: &RenamePlan,
    text: &str,
    format: EditorFormat,
) -> Result<Choices, EditorRefused> {
    let rows = plan.rows();
    let lines = lines_of(text);
    let named: Vec<(&RenameRow, &str)> = match format {
        EditorFormat::Plain => {
            if lines.len() != rows.len() {
                return Err(EditorRefused::Count {
                    lines: count(lines.len() as u64, "line", "lines"),
                    rows: count(rows.len() as u64, "file", "files"),
                });
            }
            if let Some(at) = lines.iter().position(|line| line.is_empty()) {
                return Err(EditorRefused::Empty(at + 1));
            }
            rows.iter().zip(lines).collect()
        }
        EditorFormat::Keyed => keyed(rows, &lines)?,
    };
    let mut choices = plan.choices().clone();
    for (row, name) in named {
        if name == row.new {
            continue;
        }
        choices.skipped.remove(&row.index);
        choices.edit(row.index, name);
    }
    Ok(choices)
}

/// Pairs keyed lines with their rows: the key is the last `(was ...)` at
/// the end of the line that names a row not yet claimed, so a new name
/// that itself holds " (was " is still read right. Old names repeated
/// across folders are claimed in the given order.
fn keyed<'p, 't>(
    rows: &'p [RenameRow],
    lines: &[&'t str],
) -> Result<Vec<(&'p RenameRow, &'t str)>, EditorRefused> {
    let mut unclaimed: HashMap<&str, Vec<&RenameRow>> = HashMap::new();
    for row in rows.iter().rev() {
        unclaimed.entry(row.old.as_str()).or_default().push(row);
    }
    let mut named = Vec::with_capacity(lines.len());
    for (at, line) in lines.iter().enumerate() {
        let number = at + 1;
        let Some(body) = line.strip_suffix(')') else {
            return Err(EditorRefused::UnknownKey(number));
        };
        let found = body.rmatch_indices(WAS).find_map(|(split, _)| {
            let key = &body[split + WAS.len()..];
            unclaimed
                .get(key)
                .filter(|rows| !rows.is_empty())
                .map(|_| (split, key))
        });
        let Some((split, key)) = found else {
            return Err(EditorRefused::UnknownKey(number));
        };
        let name = &body[..split];
        if name.is_empty() {
            return Err(EditorRefused::Empty(number));
        }
        let Some(row) = unclaimed.get_mut(key).and_then(Vec::pop) else {
            return Err(EditorRefused::UnknownKey(number));
        };
        named.push((row, name));
    }
    let missing: usize = unclaimed.values().map(Vec::len).sum();
    if missing > 0 {
        let words = match missing {
            1 => "1 file has no line".to_owned(),
            n => format!("{n} files have no line"),
        };
        return Err(EditorRefused::Missing(words));
    }
    Ok(named)
}
