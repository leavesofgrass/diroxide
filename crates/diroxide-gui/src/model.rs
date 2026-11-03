//! What the window shows, as plain data built from the app: each panel's
//! list box with its window of rows, the three lines, and the panels'
//! headers. The widgets draw this and the accessibility tree carries it;
//! nothing here needs the window, so the tests read it with no display.
//!
//! The rules (ADR-0007 with its November 2 status update; R26 section 5;
//! R27 sections 1 and 2, commits 9 and 10):
//!
//! - Each panel is one list box whose options are the rows of
//!   [`Panel::shown_list`](diroxide_app::Panel::shown_list): the folder's
//!   rows, or the plugin list while one is up.
//! - An option's name is the row's label and details
//!   ([`ListRow::name`]): on Windows and Linux "selected" and
//!   "unavailable" are the option's states, so a screen reader says each
//!   once; on macOS, where VoiceOver reads a list's options as static
//!   text, they stay in the name ([`ListRow::spoken`]).
//! - The list carries the real number of rows, and each option its
//!   absolute position, so "3 of 40" is never in the name and a window of
//!   rows still counts the whole list.
//! - Only a window of rows becomes nodes: the visible rows and two pages
//!   each way ([`window_range`]), so a folder of 100,000 entries is a few
//!   hundred nodes (spike S3).
//! - The drawn row says its states in words, as the terminal does: a
//!   marked row starts with an asterisk and says "selected"; color is
//!   never the only sign.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use diroxide_app::App;
use diroxide_app::panel::place_leaf;
use diroxide_core::format::count;
use diroxide_core::names::display_name;
use diroxide_core::{ListRow, Side};

/// Whether an option's states go in its name: on macOS, where VoiceOver
/// gets a list's options as static text with no states (R4 section 2.2);
/// elsewhere the platform says them.
pub const fn states_in_name() -> bool {
    cfg!(target_os = "macos")
}

/// One row of a panel's list box, as an option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowView {
    /// The row's stable id (an entry's real name): its node keeps its id
    /// while the row stays in the window.
    pub id: String,
    /// The option's accessible name: the label and details, and on macOS
    /// the states too.
    pub name: String,
    /// What is drawn: the row as the terminal prints it, an asterisk
    /// first on a marked row and the states in words.
    pub text: String,
    /// Marked (the option's selected state).
    pub selected: bool,
    /// Can be chosen (unavailable rows are listed and say so).
    pub enabled: bool,
}

impl RowView {
    /// The option for a row; `states_in_name` puts "selected" and
    /// "unavailable" in the name ([`states_in_name`]).
    pub fn of(row: &ListRow, states_in_name: bool) -> Self {
        let spoken = display_name(&row.spoken()).into_owned();
        let name = if states_in_name {
            spoken.clone()
        } else {
            display_name(&row.name()).into_owned()
        };
        let text = if row.selected {
            format!("* {spoken}")
        } else {
            spoken
        };
        Self {
            id: row.id.clone(),
            name,
            text,
            selected: row.selected,
            enabled: row.enabled,
        }
    }
}

/// A panel as its list box shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelView {
    /// Which panel.
    pub side: Side,
    /// The list box's name: "Left panel, Documents", or the plugin list's
    /// title for a plugin list ("Left panel, Changed files").
    pub name: String,
    /// The header drawn above the rows: the title line for the active
    /// panel, the place and count for the other ([`panel_header`]).
    pub header: String,
    /// Changes when the list is another list (another place, a plugin
    /// list up or gone), so the rows' node ids start again.
    pub generation: u64,
    /// The real number of rows.
    pub total: usize,
    /// The first row drawn (the scroll).
    pub top: usize,
    /// The first row in `rows`.
    pub first: usize,
    /// The window of rows that are nodes, from `first`.
    pub rows: Vec<RowView>,
    /// The focused row, counted from the list's start.
    pub focus: Option<usize>,
    /// Whether this is the active panel.
    pub active: bool,
    /// What an empty list says ("Empty folder", "Reading folder").
    pub empty: String,
}

impl PanelView {
    /// The row at an absolute position, when it is in the window.
    pub fn row(&self, index: usize) -> Option<&RowView> {
        index.checked_sub(self.first).and_then(|i| self.rows.get(i))
    }

    /// The focused row, when there is one.
    pub fn focused(&self) -> Option<&RowView> {
        self.focus.and_then(|i| self.row(i))
    }
}

/// The side in words with a capital: "Left", "Right".
pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Left => "Left",
        Side::Right => "Right",
    }
}

/// The list box's name: the panel, then what it shows.
pub fn panel_name(app: &App, side: Side) -> String {
    let panel = app.panel(side);
    let shows = match panel.plugin_list() {
        Some(shown) => display_name(shown.list().title()).into_owned(),
        None => place_leaf(panel.loading_place().unwrap_or(panel.place())),
    };
    format!("{} panel, {shows}", side_word(side))
}

/// The other panel's title, from its own place and count (R27's C2: the
/// app's title line covers only the active panel): "Backup, right panel,
/// 1 item, in D:\work", with ", loading" while it lists. A plugin list
/// says its title and its rows: "Changed files, right panel, 2 rows".
pub fn inactive_title(app: &App, side: Side) -> String {
    let panel = app.panel(side);
    let words = side.word();
    if let Some(shown) = panel.plugin_list() {
        let list = shown.list();
        return format!(
            "{}, {words} panel, {}",
            display_name(list.title()),
            count(list.len() as u64, "row", "rows")
        );
    }
    let place = panel.loading_place().unwrap_or(panel.place());
    let mut text = format!("{}, {words} panel", place_leaf(place));
    if panel.is_loading() {
        text.push_str(", loading");
    } else {
        text.push_str(&format!(
            ", {}",
            count(panel.list().len() as u64, "item", "items")
        ));
    }
    if let Some(parent) = place.parent() {
        let parent = diroxide_core::place::strip_extended_prefix(&parent.to_string()).into_owned();
        text.push_str(&format!(", in {}", display_name(&parent)));
    }
    text
}

/// The header drawn above a panel's rows: the app's title line for the
/// active panel (the whole line, never cut), [`inactive_title`] for the
/// other. While the viewer or the quick view owns the title line, the
/// active panel says its own place too.
pub fn panel_header(app: &App, side: Side) -> String {
    let owns_title = app.viewer().is_none() && !app.in_preview();
    if side == app.active_side() && owns_title {
        app.title_line()
    } else {
        inactive_title(app, side)
    }
}

/// A number that changes when a panel shows another list: its place, or a
/// plugin list over it.
pub fn generation(app: &App, side: Side) -> u64 {
    let panel = app.panel(side);
    let mut hasher = DefaultHasher::new();
    panel.place().to_string().hash(&mut hasher);
    if let Some(shown) = panel.plugin_list() {
        shown.plugin_id().hash(&mut hasher);
        shown.id().hash(&mut hasher);
    }
    hasher.finish()
}

/// The first row drawn, moved from the last only as far as the focus
/// needs, so moving inside the visible rows never scrolls (the terminal's
/// rule).
pub fn scroll_top(previous: usize, focus: Option<usize>, page: usize, len: usize) -> usize {
    let page = page.max(1);
    let mut top = previous.min(len.saturating_sub(page));
    if let Some(focus) = focus {
        if focus < top {
            top = focus;
        } else if focus >= top + page {
            top = focus + 1 - page;
        }
    }
    top
}

/// The rows that are nodes: the visible ones from `top` and two pages
/// each way, inside the list.
pub fn window_range(top: usize, page: usize, len: usize) -> Range<usize> {
    let page = page.max(1);
    let start = top.saturating_sub(2 * page);
    let end = top.saturating_add(3 * page).min(len);
    start.min(end)..end
}

/// A panel's view with the rows from `top` drawn and `page` rows visible.
pub fn panel_view(app: &App, side: Side, top: usize, page: usize) -> PanelView {
    let panel = app.panel(side);
    let list = panel.shown_list();
    let total = list.len();
    let focus = list.focus_index();
    let top = scroll_top(top, focus, page, total);
    let range = window_range(top, page, total);
    let in_name = states_in_name();
    let rows = list.rows()[range.clone()]
        .iter()
        .map(|row| RowView::of(row, in_name))
        .collect();
    let empty = if panel.is_loading() && panel.plugin_list().is_none() {
        "Reading folder".to_owned()
    } else {
        display_name(list.empty_text()).into_owned()
    };
    PanelView {
        side,
        name: panel_name(app, side),
        header: panel_header(app, side),
        generation: generation(app, side),
        total,
        top,
        first: range.start,
        rows,
        focus,
        active: side == app.active_side(),
        empty,
    }
}

/// A list inside a dialog: its name, its window of rows and its focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListView {
    /// The list box's name.
    pub name: String,
    /// The first row in `rows`.
    pub first: usize,
    /// The window of rows that are nodes.
    pub rows: Vec<RowView>,
    /// The real number of rows.
    pub total: usize,
    /// The focused row, counted from the list's start.
    pub focus: Option<usize>,
}

impl ListView {
    /// A dialog's list from the app's list model: the rows around the
    /// focus (three pages of `page` rows each way), named `name`.
    pub fn of(list: &diroxide_core::ListModel, name: impl Into<String>, page: usize) -> Self {
        let total = list.len();
        let focus = list.focus_index();
        let top = scroll_top(0, focus, page, total);
        let range = window_range(top, page, total);
        let in_name = states_in_name();
        Self {
            name: name.into(),
            first: range.start,
            rows: list.rows()[range]
                .iter()
                .map(|row| RowView::of(row, in_name))
                .collect(),
            total,
            focus,
        }
    }

    /// A list of plain lines (a confirmation's answers, a text to read),
    /// the first focused.
    pub fn lines(name: impl Into<String>, lines: &[String], focus: usize) -> Self {
        let rows: Vec<RowView> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| RowView {
                id: i.to_string(),
                name: display_name(line).into_owned(),
                text: display_name(line).into_owned(),
                selected: false,
                enabled: true,
            })
            .collect();
        let total = rows.len();
        Self {
            name: name.into(),
            first: 0,
            rows,
            total,
            focus: (total > 0).then_some(focus.min(total - 1)),
        }
    }

    /// The row at an absolute position, when it is in the window.
    pub fn row(&self, index: usize) -> Option<&RowView> {
        index.checked_sub(self.first).and_then(|i| self.rows.get(i))
    }
}

/// A text field in a dialog, driven by the app's entry model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldView {
    /// The field's label, said first ("Rename report.pdf to").
    pub label: String,
    /// The text as shown: the typed text, or for a secret only its mask
    /// (one asterisk a character, or nothing). The secret itself never
    /// enters the view.
    pub text: String,
    /// The caret, in characters from the start.
    pub cursor: usize,
    /// A password field.
    pub secret: bool,
    /// A field with completion: a combo box whose list is the
    /// candidates.
    pub combo: bool,
    /// The candidates, while their list is open.
    pub candidates: Option<ListView>,
    /// Whether the focus is in the candidates rather than the field.
    pub in_list: bool,
}

/// What a dialog's focus is on: a text field, or a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlView {
    /// A text field, with its candidates.
    Field(FieldView),
    /// A list to choose from, or lines to read.
    List(ListView),
}

/// The open dialog as the modal host shows it: one of the seven shapes
/// as a name, lines to read and one control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogView {
    /// Which dialog: a new one is a new modal.
    pub id: diroxide_core::DialogId,
    /// The dialog's name, said when it opens: the question, the list's
    /// title, the field's label.
    pub title: String,
    /// Lines to read before the control: a question's situation, the
    /// paths a confirmation touches, a list's help.
    pub lines: Vec<String>,
    /// The control the focus is on.
    pub control: ControlView,
}

impl DialogView {
    /// Whether `other` is the same dialog in the same shape, so the
    /// modal is changed in place rather than built again (and its focus
    /// kept).
    pub fn same_shape(&self, other: &Self) -> bool {
        self.id == other.id
            && self.lines == other.lines
            && matches!(
                (&self.control, &other.control),
                (ControlView::Field(_), ControlView::Field(_))
                    | (ControlView::List(_), ControlView::List(_))
            )
    }
}

/// The rows a dialog's list shows at a time, for its window.
pub const DIALOG_PAGE: usize = 12;

/// The top dialog as the modal host shows it, or `None`.
pub fn dialog_view(app: &App) -> Option<DialogView> {
    use diroxide_app::Dialog;
    use diroxide_app::entry::CompletionSource;
    let (id, dialog) = app.dialog()?;
    let title = display_name(&dialog.title()).into_owned();
    let silent = app.settings().secret_echo == diroxide_app::config::SecretEcho::Silent;
    let (lines, control) = match dialog {
        Dialog::Confirm { listing, .. } => (
            listing.clone(),
            // The answer starts on No; y, n, Enter and Escape answer it.
            ControlView::List(ListView::lines(
                "Answers",
                &["No, the default".to_owned(), "Yes".to_owned()],
                0,
            )),
        ),
        Dialog::Prompt { label, text, .. } => (
            Vec::new(),
            ControlView::Field(FieldView {
                label: label.clone(),
                text: text.clone(),
                cursor: text.chars().count(),
                secret: false,
                combo: false,
                candidates: None,
                in_list: false,
            }),
        ),
        Dialog::Choose { list, .. } => {
            let help = list.help();
            let lines = if help.is_empty() {
                Vec::new()
            } else {
                vec![help.to_owned()]
            };
            (
                lines,
                ControlView::List(ListView::of(list, list.title(), DIALOG_PAGE)),
            )
        }
        Dialog::Read { title, lines } => (
            Vec::new(),
            ControlView::List(ListView::lines(title.clone(), lines, 0)),
        ),
        Dialog::Secret { request, .. } => (
            Vec::new(),
            ControlView::Field(FieldView {
                label: request.title(),
                text: dialog.masked_text(silent).unwrap_or_default(),
                cursor: dialog
                    .masked_text(silent)
                    .map_or(0, |masked| masked.chars().count()),
                secret: true,
                combo: false,
                candidates: None,
                in_list: false,
            }),
        ),
        Dialog::Question { lines, choices } => (
            lines.clone(),
            ControlView::List(ListView::of(choices, choices.title(), DIALOG_PAGE)),
        ),
        Dialog::Edit {
            field,
            candidates,
            in_list,
        } => (
            Vec::new(),
            ControlView::Field(FieldView {
                label: field.label().to_owned(),
                text: field.text().to_owned(),
                cursor: field.cursor(),
                secret: false,
                combo: !matches!(
                    field.source(),
                    CompletionSource::None | CompletionSource::History
                ),
                candidates: candidates
                    .as_ref()
                    .map(|list| ListView::of(list, "Candidates", DIALOG_PAGE)),
                in_list: *in_list,
            }),
        ),
    };
    Some(DialogView {
        id,
        title,
        lines: lines
            .iter()
            .map(|line| display_name(line).into_owned())
            .collect(),
        control,
    })
}

/// The three lines under the panels and above them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// The title line, the active place first ("Documents, left panel, in
    /// D:\work").
    pub title: String,
    /// The status line: the last message, or the folder and its counts.
    pub status: String,
    /// The jobs line: "No jobs", or what runs.
    pub jobs: String,
}

/// The three lines as the app says them.
pub fn lines(app: &App) -> Lines {
    Lines {
        title: app.title_line(),
        status: app.status_line(),
        jobs: app.jobs().line(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_the_visible_rows_and_two_pages_each_way() {
        assert_eq!(window_range(0, 20, 100_000), 0..60);
        assert_eq!(window_range(1_000, 20, 100_000), 960..1_060);
        assert_eq!(window_range(99_980, 20, 100_000), 99_940..100_000);
        assert_eq!(window_range(0, 20, 5), 0..5);
        assert_eq!(window_range(0, 20, 0), 0..0);
    }

    #[test]
    fn the_scroll_moves_only_as_far_as_the_focus_needs() {
        assert_eq!(scroll_top(0, Some(5), 10, 100), 0);
        assert_eq!(scroll_top(0, Some(10), 10, 100), 1);
        assert_eq!(scroll_top(30, Some(29), 10, 100), 29);
        assert_eq!(scroll_top(95, Some(99), 10, 100), 90);
        assert_eq!(scroll_top(50, None, 10, 20), 10);
    }

    #[test]
    fn a_marked_row_says_its_state_once_on_windows_and_in_words_on_screen() {
        let mut row = ListRow::new("report.pdf", "report.pdf").with_details(["PDF", "2.1 MB"]);
        row.selected = true;
        let option = RowView::of(&row, false);
        assert_eq!(option.name, "report.pdf, PDF, 2.1 MB");
        assert_eq!(option.text, "* report.pdf, selected, PDF, 2.1 MB");
        assert!(option.selected);
        let mac = RowView::of(&row, true);
        assert_eq!(mac.name, "report.pdf, selected, PDF, 2.1 MB");
        let mut off = ListRow::new("eject", "Eject").with_details(["needs a plugin"]);
        off.enabled = false;
        let option = RowView::of(&off, false);
        assert_eq!(option.name, "Eject, needs a plugin");
        assert_eq!(option.text, "Eject, unavailable, needs a plugin");
        assert!(!option.enabled);
    }

    #[test]
    fn a_hidden_character_in_a_name_is_never_drawn_or_said() {
        let row = ListRow::new("bad\u{f03a}name", "bad\u{f03a}name");
        let option = RowView::of(&row, false);
        assert!(!option.name.contains('\u{f03a}'), "{}", option.name);
        assert!(!option.text.contains('\u{f03a}'), "{}", option.text);
    }
}
