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
