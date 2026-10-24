//! The list model: one list type for panels, search results, bookmarks,
//! jobs, menus and every chooser, so every list reads the same way.
//!
//! A list has a title, rows and a focus. Each row says its meaning first and
//! its position last: "report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40".
//! Letters jump to the next row that starts with them; Home and End go to
//! the ends; F1 says the list's help; Say Status says where the focus is.

use serde::{Deserialize, Serialize};

use crate::format::count;

/// One row of a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRow {
    /// A stable identity: an entry's real name, a command id. The focus
    /// follows it when the rows are replaced.
    pub id: String,
    /// The meaning, said first: a display name, a command name.
    pub label: String,
    /// What is said after the label, in order: type, size, date, key.
    pub details: Vec<String>,
    /// Whether the row is selected.
    pub selected: bool,
    /// Whether the row can be chosen. Unavailable rows are still listed and
    /// say so.
    pub enabled: bool,
}

impl ListRow {
    /// A row with an id and a label, no details, not selected, enabled.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            details: Vec::new(),
            selected: false,
            enabled: true,
        }
    }

    /// The same row with details added.
    pub fn with_details<I, S>(mut self, details: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.details.extend(details.into_iter().map(Into::into));
        self
    }

    /// The row as said, without its position: "report.pdf, selected, PDF,
    /// 2.1 MB". An unavailable row says so right after its label, before
    /// the details, so the meaning lands in the first cells: "Eject,
    /// unavailable, needs the Volume actions plugin".
    pub fn spoken(&self) -> String {
        let mut parts: Vec<&str> = vec![&self.label];
        if !self.enabled {
            parts.push("unavailable");
        }
        if self.selected {
            parts.push("selected");
        }
        parts.extend(
            self.details
                .iter()
                .map(String::as_str)
                .filter(|d| !d.is_empty()),
        );
        parts.join(", ")
    }
}

/// What replacing a list's rows did to the focus and the marks, so the app
/// can say it: "Focus moved: report.pdf is gone", "2 marks lost".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RowsReplaced {
    /// The focused row's id is gone, and the focus is now on another row.
    pub focus_moved: bool,
    /// How many marked rows are gone.
    pub marks_lost: usize,
}

/// A selection frozen at the moment of confirmation: the plan acts on
/// these ids, by id, never on whatever row later sits where they were.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrozenSelection {
    /// The ids acted on: the marked rows, or the focused row when nothing
    /// was marked.
    pub ids: Vec<String>,
    /// The focused row's id when frozen.
    pub focused_id: Option<String>,
    /// The focused row's label when frozen, for the note.
    pub focused_label: Option<String>,
    /// Whether the focused row is among the ids.
    pub focused_included: bool,
}

impl FrozenSelection {
    /// The plan's first line when the focused row is not part of the
    /// action, inside 40 cells where the name allows: "Not the focused
    /// row: report.pdf". `None` when it is part of it.
    pub fn focus_note(&self) -> Option<String> {
        if self.focused_included {
            return None;
        }
        self.focused_label
            .as_ref()
            .map(|label| format!("Not the focused row: {label}"))
    }
}

/// A move of the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Move {
    /// One row up.
    Up,
    /// One row down.
    Down,
    /// One page up.
    PageUp,
    /// One page down.
    PageDown,
    /// The first row.
    Home,
    /// The last row.
    End,
    /// A row by its index, clamped to the list.
    To(usize),
}

/// A titled list with a focus. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListModel {
    title: String,
    rows: Vec<ListRow>,
    focus: usize,
    page_size: usize,
    help: String,
    empty_text: String,
}

impl ListModel {
    /// The number of rows a page moves when no page size is set.
    pub const DEFAULT_PAGE_SIZE: usize = 10;

    /// An empty list with a title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
            focus: 0,
            page_size: Self::DEFAULT_PAGE_SIZE,
            help: String::new(),
            empty_text: "Empty".to_owned(),
        }
    }

    /// The same list with rows, the focus on the first.
    pub fn with_rows(mut self, rows: Vec<ListRow>) -> Self {
        self.rows = rows;
        self.focus = 0;
        self
    }

    /// The title, said when the list gets focus.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Changes the title.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// The F1 help for this list, or an empty string.
    pub fn help(&self) -> &str {
        &self.help
    }

    /// Sets the F1 help.
    pub fn set_help(&mut self, help: impl Into<String>) {
        self.help = help.into();
    }

    /// What is said for an empty list, such as "Empty folder".
    pub fn empty_text(&self) -> &str {
        &self.empty_text
    }

    /// Sets what is said for an empty list.
    pub fn set_empty_text(&mut self, text: impl Into<String>) {
        self.empty_text = text.into();
    }

    /// The rows a page moves.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Sets the rows a page moves, at least one; frontends set it to the
    /// visible height.
    pub fn set_page_size(&mut self, rows: usize) {
        self.page_size = rows.max(1);
    }

    /// The rows.
    pub fn rows(&self) -> &[ListRow] {
        &self.rows
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Replaces the rows. The listed row must never lie (plan section
    /// 5.2): marks travel by row id, never by position, so a new row is
    /// marked when the row with its id was marked (or it comes marked); the
    /// focus stays on the row with the same id. When that id is gone, the
    /// focus keeps its index, clamped to the list, and the result says the
    /// focus moved to a different row, so the app announces it. The result
    /// also counts the marks whose rows are gone.
    pub fn set_rows(&mut self, mut rows: Vec<ListRow>) -> RowsReplaced {
        let focused_id = self.focused().map(|row| row.id.clone());
        let marked: std::collections::HashSet<String> = self
            .rows
            .drain(..)
            .filter(|row| row.selected)
            .map(|row| row.id)
            .collect();
        let mut kept = 0;
        for row in &mut rows {
            if marked.contains(&row.id) {
                row.selected = true;
                kept += 1;
            }
        }
        self.rows = rows;
        let focus_moved = match focused_id {
            Some(id) => match self.rows.iter().position(|row| row.id == id) {
                Some(index) => {
                    self.focus = index;
                    false
                }
                None => {
                    self.focus = self.focus.min(self.rows.len().saturating_sub(1));
                    true
                }
            },
            None => {
                self.focus = 0;
                false
            }
        };
        RowsReplaced {
            focus_moved,
            marks_lost: marked.len().saturating_sub(kept),
        }
    }

    /// The marked rows' ids frozen with the focus, as a confirmation shows
    /// them: the plan acts on exactly these ids, whatever happens to the
    /// list after (plan section 5.2). With no marks, the focused row alone
    /// is the selection.
    pub fn freeze(&self) -> FrozenSelection {
        let marked: Vec<String> = self
            .rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.id.clone())
            .collect();
        let focused = self
            .focused()
            .map(|row| (row.id.clone(), row.label.clone()));
        let ids = if marked.is_empty() {
            focused.iter().map(|(id, _)| id.clone()).collect()
        } else {
            marked
        };
        let (focused_id, focused_label) = focused.unzip();
        FrozenSelection {
            focused_included: focused_id.as_ref().is_none_or(|id| ids.contains(id)),
            ids,
            focused_id,
            focused_label,
        }
    }

    /// The focused row's index, or `None` when the list is empty.
    pub fn focus_index(&self) -> Option<usize> {
        (!self.rows.is_empty()).then_some(self.focus)
    }

    /// The focused row.
    pub fn focused(&self) -> Option<&ListRow> {
        self.rows.get(self.focus)
    }

    /// Puts the focus on the row with this id. Returns whether it was found.
    pub fn focus_id(&mut self, id: &str) -> bool {
        match self.rows.iter().position(|row| row.id == id) {
            Some(index) => {
                self.focus = index;
                true
            }
            None => false,
        }
    }

    /// Moves the focus. Returns whether it changed. Moves stop at the ends;
    /// they do not wrap.
    pub fn move_focus(&mut self, movement: Move) -> bool {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return false;
        };
        let target = match movement {
            Move::Up => self.focus.saturating_sub(1),
            Move::Down => (self.focus + 1).min(last),
            Move::PageUp => self.focus.saturating_sub(self.page_size),
            Move::PageDown => self.focus.saturating_add(self.page_size).min(last),
            Move::Home => 0,
            Move::End => last,
            Move::To(index) => index.min(last),
        };
        let changed = target != self.focus;
        self.focus = target;
        changed
    }

    /// Moves the focus to the next row after it whose label starts with
    /// `letter`, ignoring case, wrapping round. Returns whether one was
    /// found.
    pub fn letter_jump(&mut self, letter: char) -> bool {
        let wanted: String = letter.to_lowercase().collect();
        let n = self.rows.len();
        for step in 1..=n {
            let index = (self.focus + step) % n;
            if self.rows[index].label.to_lowercase().starts_with(&wanted) {
                self.focus = index;
                return true;
            }
        }
        false
    }

    /// Moves the focus to the first row, from the focused one on and
    /// wrapping round, whose label starts with `prefix`, ignoring case: the
    /// type-ahead search. Returns whether one was found.
    pub fn prefix_jump(&mut self, prefix: &str) -> bool {
        let wanted = prefix.to_lowercase();
        let n = self.rows.len();
        for step in 0..n {
            let index = (self.focus + step) % n;
            if self.rows[index].label.to_lowercase().starts_with(&wanted) {
                self.focus = index;
                return true;
            }
        }
        false
    }

    /// Toggles the focused row's selection. Returns the new state, or `None`
    /// when the list is empty.
    pub fn toggle_selected(&mut self) -> Option<bool> {
        let row = self.rows.get_mut(self.focus)?;
        row.selected = !row.selected;
        Some(row.selected)
    }

    /// Selects every row.
    pub fn select_all(&mut self) {
        self.rows.iter_mut().for_each(|row| row.selected = true);
    }

    /// Clears the selection.
    pub fn select_none(&mut self) {
        self.rows.iter_mut().for_each(|row| row.selected = false);
    }

    /// Inverts the selection.
    pub fn invert_selection(&mut self) {
        self.rows
            .iter_mut()
            .for_each(|row| row.selected = !row.selected);
    }

    /// Selects the rows that match, leaving the others as they are. Returns
    /// how many matched.
    pub fn select_matching(&mut self, matches: impl Fn(&ListRow) -> bool) -> usize {
        let mut matched = 0;
        for row in self.rows.iter_mut().filter(|row| matches(row)) {
            row.selected = true;
            matched += 1;
        }
        matched
    }

    /// The number of selected rows.
    pub fn selected_count(&self) -> usize {
        self.rows.iter().filter(|row| row.selected).count()
    }

    /// The ids of the selected rows, in order.
    pub fn selected_ids(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.id.as_str())
            .collect()
    }

    /// The focus's position: "3 of 40", or "0 of 0" when empty.
    pub fn position_text(&self) -> String {
        match self.focus_index() {
            Some(index) => format!("{} of {}", index + 1, self.rows.len()),
            None => "0 of 0".to_owned(),
        }
    }

    /// What is said when the focus lands on a row: the row, then its
    /// position last. An empty list says its empty text.
    pub fn announce_focus(&self) -> String {
        match self.focused() {
            Some(row) => format!("{}, {}", row.spoken(), self.position_text()),
            None => self.empty_text.clone(),
        }
    }

    /// Say Status: the title, the count, the selection and the focus, in
    /// that order: "Documents, 40 items, 2 selected, on report.pdf, 3 of 40".
    pub fn status(&self) -> String {
        let mut parts = vec![
            self.title.clone(),
            count(self.rows.len() as u64, "item", "items"),
        ];
        let selected = self.selected_count();
        if selected > 0 {
            parts.push(format!("{selected} selected"));
        }
        if let Some(row) = self.focused() {
            parts.push(format!("on {}", row.label));
            parts.push(self.position_text());
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(labels: &[&str]) -> ListModel {
        ListModel::new("Documents").with_rows(labels.iter().map(|l| ListRow::new(*l, *l)).collect())
    }

    #[test]
    fn focus_moves_stop_at_the_ends() {
        let mut l = list(&["a", "b", "c"]);
        assert!(!l.move_focus(Move::Up));
        assert!(l.move_focus(Move::Down));
        assert!(l.move_focus(Move::End));
        assert_eq!(l.focus_index(), Some(2));
        assert!(!l.move_focus(Move::Down));
        assert!(l.move_focus(Move::Home));
        l.set_page_size(2);
        l.move_focus(Move::PageDown);
        assert_eq!(l.focus_index(), Some(2));
        l.move_focus(Move::To(99));
        assert_eq!(l.focus_index(), Some(2));
    }

    #[test]
    fn an_empty_list_says_its_empty_text() {
        let mut l = ListModel::new("Downloads");
        l.set_empty_text("Empty folder");
        assert!(!l.move_focus(Move::Down));
        assert_eq!(l.focus_index(), None);
        assert_eq!(l.announce_focus(), "Empty folder");
        assert_eq!(l.position_text(), "0 of 0");
        assert_eq!(l.status(), "Downloads, no items");
        assert!(!l.letter_jump('a'));
        assert_eq!(l.toggle_selected(), None);
    }

    #[test]
    fn letters_jump_and_wrap() {
        let mut l = list(&["Alpha", "beta", "Bravo", "charlie"]);
        assert!(l.letter_jump('b'));
        assert_eq!(l.focused().unwrap().label, "beta");
        assert!(l.letter_jump('B'));
        assert_eq!(l.focused().unwrap().label, "Bravo");
        assert!(l.letter_jump('b'));
        assert_eq!(l.focused().unwrap().label, "beta");
        assert!(!l.letter_jump('z'));
        assert!(l.prefix_jump("CH"));
        assert_eq!(l.focused().unwrap().label, "charlie");
        assert!(l.prefix_jump("ch"));
        assert_eq!(l.focused().unwrap().label, "charlie");
    }

    #[test]
    fn the_position_is_said_last() {
        let mut l = ListModel::new("Documents").with_rows(vec![
            ListRow::new("a.txt", "a.txt"),
            ListRow::new("report.pdf", "report.pdf").with_details([
                "PDF",
                "2.1 MB",
                "modified Tuesday",
            ]),
        ]);
        l.move_focus(Move::Down);
        assert_eq!(
            l.announce_focus(),
            "report.pdf, PDF, 2.1 MB, modified Tuesday, 2 of 2"
        );
        l.toggle_selected();
        assert_eq!(
            l.announce_focus(),
            "report.pdf, selected, PDF, 2.1 MB, modified Tuesday, 2 of 2"
        );
        assert_eq!(
            l.status(),
            "Documents, 2 items, 1 selected, on report.pdf, 2 of 2"
        );
    }

    #[test]
    fn replacing_rows_keeps_the_focus_on_the_same_id() {
        let mut l = list(&["a", "b", "c"]);
        l.move_focus(Move::To(1));
        let replaced = l.set_rows(vec![
            ListRow::new("x", "x"),
            ListRow::new("a", "a"),
            ListRow::new("b", "b"),
        ]);
        assert_eq!(l.focused().unwrap().id, "b");
        assert!(!replaced.focus_moved);
        let replaced = l.set_rows(vec![ListRow::new("y", "y")]);
        assert_eq!(l.focus_index(), Some(0));
        assert!(replaced.focus_moved);
        l.set_rows(Vec::new());
        assert_eq!(l.focus_index(), None);
    }

    #[test]
    fn marks_travel_by_id_and_lost_ones_are_counted() {
        let mut l = list(&["a", "b", "c", "d"]);
        l.move_focus(Move::To(1));
        l.toggle_selected();
        l.move_focus(Move::To(3));
        l.toggle_selected();
        // A refresh: "a" is gone, so every row moves up one place.
        let replaced = l.set_rows(vec![
            ListRow::new("b", "b"),
            ListRow::new("c", "c"),
            ListRow::new("e", "e"),
        ]);
        assert_eq!(l.selected_ids(), vec!["b"]);
        assert_eq!(replaced.marks_lost, 1);
        assert!(replaced.focus_moved);
        assert_eq!(l.focused().unwrap().id, "e");
    }

    #[test]
    fn a_frozen_selection_says_when_the_focus_is_left_out() {
        let mut l = list(&["a.txt", "b.txt", "report.pdf"]);
        l.toggle_selected();
        l.move_focus(Move::To(2));
        let frozen = l.freeze();
        assert_eq!(frozen.ids, vec!["a.txt"]);
        assert!(!frozen.focused_included);
        assert_eq!(
            frozen.focus_note().as_deref(),
            Some("Not the focused row: report.pdf")
        );
        l.select_none();
        let alone = l.freeze();
        assert_eq!(alone.ids, vec!["report.pdf"]);
        assert_eq!(alone.focus_note(), None);
        assert_eq!(ListModel::new("x").freeze().ids, Vec::<String>::new());
    }

    #[test]
    fn selection_commands() {
        let mut l = list(&["a.txt", "b.pdf", "c.txt"]);
        assert_eq!(l.select_matching(|row| row.label.ends_with(".txt")), 2);
        assert_eq!(l.selected_ids(), vec!["a.txt", "c.txt"]);
        l.invert_selection();
        assert_eq!(l.selected_ids(), vec!["b.pdf"]);
        l.select_all();
        assert_eq!(l.selected_count(), 3);
        l.select_none();
        assert_eq!(l.selected_count(), 0);
        let mut disabled = ListRow::new("x", "Paste");
        disabled.enabled = false;
        assert_eq!(disabled.spoken(), "Paste, unavailable");
    }
}
