//! Selection on the filter language (W3a, plan commit 36): select and
//! unselect by pattern with the count said before anything changes, by
//! kind, by size and by date, the same extension, a range, and the next
//! rows; restore selection takes back any of them.
//!
//! What is said, one line a change: "12 entries will be selected. No is
//! the default. y or n", then "12 selected"; "Selected 3 folders. 3
//! selected."; "Range started at report.pdf"; "Marked 5, report.pdf to
//! notes.txt. 5 selected."

use std::collections::HashSet;
use std::sync::Arc;

use diroxide_core::filter::{GROUPS, kind_word};
use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, Effect, EntryKind, Importance, ListModel, ListRow, Message, MessageKind,
    Move, Side,
};

use super::{App, Purpose};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::state;

/// What selection keeps between commands: where each panel's range
/// started.
#[derive(Clone, Debug, Default)]
pub(super) struct SelectState {
    range: [Option<String>; 2],
}

/// A selection command of this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    ByKind,
    BySize,
    ByDate,
    SameExtension,
    Range,
    NextRows,
}

const ROUTES: [(CommandId, Do); 6] = [
    (ids::SELECT_BY_KIND, Do::ByKind),
    (ids::SELECT_BY_SIZE, Do::BySize),
    (ids::SELECT_BY_DATE, Do::ByDate),
    (ids::SELECT_SAME_EXTENSION, Do::SameExtension),
    (ids::SELECT_RANGE, Do::Range),
    (ids::SELECT_NEXT_ROWS, Do::NextRows),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

/// The kinds Select by kind offers, each a filter and its words: the
/// entry kinds first, then the file-type groups.
fn kind_choices() -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> = [
        (EntryKind::Folder, "Folders"),
        (EntryKind::File, "Files"),
        (EntryKind::Link, "Links"),
        (EntryKind::Archive, "Archives"),
    ]
    .into_iter()
    .map(|(kind, words)| (format!("kind {}", kind_word(kind)), words.to_owned()))
    .collect();
    for group in &GROUPS {
        let mut words = group.name.to_owned();
        if let Some(first) = words.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        choices.push((format!("group {}", group.name), format!("{words}, group")));
    }
    choices
}

impl App {
    /// Runs a selection command of this module; `None` when the command
    /// is another's.
    pub(super) fn selecting_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        let side = self.active;
        Some(match action {
            Do::ByKind => self.select_kind_list(side),
            Do::BySize => self.pattern_prompt_with(side, true, "size over "),
            Do::ByDate => self.pattern_prompt_with(side, true, "modified within "),
            Do::SameExtension => self.select_same_extension(side),
            Do::Range => self.mark_range(side),
            Do::NextRows => self.next_rows_prompt(side),
        })
    }

    /// The select or unselect prompt with text in it, in the filter
    /// language, its words completed by Tab.
    pub(super) fn pattern_prompt_with(
        &mut self,
        side: Side,
        select: bool,
        text: &str,
    ) -> Vec<Effect> {
        let label = if select {
            "Select by pattern"
        } else {
            "Unselect by pattern"
        };
        let field = EntryField::new(label, CompletionSource::Filter)
            .with_text(text)
            .with_history(state::keys::PATTERN);
        self.open_dialog(Dialog::edit(field), Purpose::Pattern { side, select })
    }

    /// The rows a pattern matches in a panel, or the sentence that says
    /// why it cannot be used.
    fn pattern_rows(&self, side: Side, text: &str) -> Result<Vec<String>, String> {
        let filter = match self.read_filter(text) {
            Ok(Some(filter)) => filter,
            Ok(None) => return Err("Type a pattern, such as *.txt".to_owned()),
            Err(error) => return Err(error.sentences()),
        };
        if let Some(problem) = filter.panel_problem() {
            return Err(problem.sentences());
        }
        if filter.reads_content() {
            return Err(
                "Selecting by contents is not built yet. Filter by contents, then select all."
                    .to_owned(),
            );
        }
        let clock = Arc::clone(&self.clock);
        Ok(self
            .panel(side)
            .matching_ids(&filter, Some(&self.organize.bookmarks), clock.as_ref()))
    }

    /// Enter in the select or unselect prompt: the count first, as a
    /// question that starts on No; nothing changes until y.
    pub(super) fn pattern_entered(&mut self, side: Side, select: bool, text: &str) -> Vec<Effect> {
        let rows = match self.pattern_rows(side, text) {
            Ok(rows) => rows,
            Err(line) => return vec![self.say(Message::error(line))],
        };
        self.remember(state::keys::PATTERN, text);
        let list = self.panel(side).list();
        let selected: HashSet<&str> = list.selected_ids().into_iter().collect();
        let changing: Vec<String> = rows
            .into_iter()
            .filter(|id| selected.contains(id.as_str()) != select)
            .collect();
        let mut effects = self.close_top();
        if changing.is_empty() {
            let line = if select {
                "No more entries match. Nothing changed.".to_owned()
            } else {
                "No selected entry matches. Nothing changed.".to_owned()
            };
            effects.push(self.say(Message::info(line)));
            return effects;
        }
        let verb = if select { "selected" } else { "unselected" };
        let question = format!(
            "{} will be {verb}. No is the default. y or n",
            count(changing.len() as u64, "entry", "entries")
        );
        effects.extend(self.open_dialog(
            Dialog::Confirm {
                question: Message::new(question, Importance::High, MessageKind::Question),
                listing: Vec::new(),
                on_yes: Command::new(ids::YES),
            },
            Purpose::SelectCount {
                side,
                select,
                ids: changing,
            },
        ));
        effects
    }

    /// Yes to the count question: the marks change, the ones before kept
    /// for Restore selection.
    pub(super) fn select_counted(
        &mut self,
        side: Side,
        select: bool,
        ids: Vec<String>,
    ) -> Vec<Effect> {
        self.keep_selection(side);
        let changing: HashSet<String> = ids.into_iter().collect();
        let list = self.panel_mut(side).list_mut();
        if select {
            list.select_matching(|row| changing.contains(&row.id));
        } else {
            let keep: HashSet<String> = list
                .selected_ids()
                .into_iter()
                .filter(|id| !changing.contains(*id))
                .map(str::to_owned)
                .collect();
            list.select_none();
            list.select_matching(|row| keep.contains(&row.id));
        }
        let total = list.selected_count();
        let line = format!("{total} selected");
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Select by kind: the kinds and the file-type groups as a list.
    fn select_kind_list(&mut self, side: Side) -> Vec<Effect> {
        let rows = kind_choices()
            .into_iter()
            .map(|(id, words)| ListRow::new(id, words))
            .collect();
        let mut list = ListModel::new("Select by kind").with_rows(rows);
        list.set_help("Up and Down choose, Enter selects that kind, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SELECT_BY_KIND),
            },
            Purpose::SelectKind(side),
        )
    }

    /// Enter on a kind: every entry of it is selected at once.
    pub(super) fn select_kind_chosen(&mut self, side: Side, text: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let rows = match self.pattern_rows(side, text) {
            Ok(rows) => rows,
            Err(line) => {
                effects.push(self.say(Message::error(line)));
                return effects;
            }
        };
        let words = kind_choices()
            .into_iter()
            .find(|(id, _)| id == text)
            .map(|(_, words)| words.split(',').next().unwrap_or_default().to_lowercase())
            .unwrap_or_else(|| text.to_owned());
        self.keep_selection(side);
        let wanted: HashSet<String> = rows.iter().cloned().collect();
        let list = self.panel_mut(side).list_mut();
        let before = list.selected_count();
        list.select_matching(|row| wanted.contains(&row.id));
        let total = list.selected_count();
        let line = format!("Selected {} {words}. {total} selected.", total - before);
        effects.push(self.redraw_panel(side));
        effects.push(self.say(Message::info(line)));
        effects
    }

    /// Select the same extension as the focused entry.
    fn select_same_extension(&mut self, side: Side) -> Vec<Effect> {
        let Some(entry) = self.panel(side).focused_entry() else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let extension = entry
            .extension()
            .map(str::to_lowercase)
            .filter(|_| !entry.is_folder_like());
        let Some(extension) = extension else {
            let text = format!("{} has no extension", entry.display_name());
            return vec![self.say(Message::status(text))];
        };
        self.keep_selection(side);
        let wanted: HashSet<String> = self
            .panel(side)
            .entries()
            .iter()
            .filter(|e| !e.is_folder_like())
            .filter(|e| e.extension().is_some_and(|x| x.to_lowercase() == extension))
            .map(|e| e.name.clone())
            .collect();
        let list = self.panel_mut(side).list_mut();
        let before = list.selected_count();
        list.select_matching(|row| wanted.contains(&row.id));
        let total = list.selected_count();
        let line = format!(
            "Selected {} with .{extension}. {total} selected.",
            total - before
        );
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Mark range: the first time it starts at the focused row; the
    /// second marks every row from there to the focus.
    fn mark_range(&mut self, side: Side) -> Vec<Effect> {
        let started = self.selecting.range[side.index()].take();
        let list = self.panel(side).list();
        let Some(focus) = list.focus_index() else {
            let empty = list.empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let start = started.and_then(|id| list.rows().iter().position(|r| r.id == id));
        let Some(start) = start else {
            let row = &list.rows()[focus];
            let (id, label) = (row.id.clone(), row.label.clone());
            self.selecting.range[side.index()] = Some(id);
            let again = match self.key(&ids::SELECT_RANGE) {
                key if key.is_empty() => "Mark range again ends it".to_owned(),
                key => format!("{key} again ends it"),
            };
            let line = format!("Range started at {label}. {again}.");
            return vec![self.say(Message::info(line))];
        };
        let (low, high) = (start.min(focus), start.max(focus));
        let rows = list.rows();
        let first = rows[low].label.clone();
        let last = rows[high].label.clone();
        let wanted: HashSet<String> = rows[low..=high].iter().map(|r| r.id.clone()).collect();
        self.keep_selection(side);
        let list = self.panel_mut(side).list_mut();
        list.select_matching(|row| wanted.contains(&row.id));
        let total = list.selected_count();
        let marked = high - low + 1;
        let line = if marked == 1 {
            format!("Marked {first}. {total} selected.")
        } else {
            format!("Marked {marked}, {first} to {last}. {total} selected.")
        };
        vec![self.redraw_panel(side), self.say(Message::info(line))]
    }

    /// Select the next rows: how many, typed.
    fn next_rows_prompt(&mut self, side: Side) -> Vec<Effect> {
        let field = EntryField::new("Select the next rows, how many", CompletionSource::None);
        self.open_dialog(Dialog::edit(field), Purpose::NextRows(side))
    }

    /// The count typed: that many rows from the focus are marked, and the
    /// focus moves to the row after them.
    pub(super) fn next_rows_typed(&mut self, side: Side, text: &str) -> Vec<Effect> {
        let Ok(wanted) = text.trim().parse::<usize>() else {
            return vec![self.say(Message::error("Type a number of rows, such as 5"))];
        };
        let mut effects = self.close_top();
        let list = self.panel(side).list();
        let Some(focus) = list.focus_index().filter(|_| wanted > 0) else {
            effects.push(self.say(Message::info("Nothing selected")));
            return effects;
        };
        let end = (focus + wanted).min(list.len());
        let ids: HashSet<String> = list.rows()[focus..end]
            .iter()
            .map(|r| r.id.clone())
            .collect();
        self.keep_selection(side);
        let list = self.panel_mut(side).list_mut();
        list.select_matching(|row| ids.contains(&row.id));
        list.move_focus(Move::To(end.min(list.len().saturating_sub(1))));
        let total = list.selected_count();
        let marked = end - focus;
        let line = format!(
            "Marked {}. {total} selected.",
            count(marked as u64, "row", "rows")
        );
        effects.push(self.redraw_panel(side));
        effects.push(self.say(Message::info(line)));
        effects
    }
}
