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
use diroxide_core::{EntryKind, ListRow, Side};
use diroxide_theme::PanelRole;

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
    /// What the row is, for its colors and shape: a folder, an archive, a
    /// link, a broken link, a hidden entry, or plain text (the terminal's
    /// rule, `crates/diroxide-tui/src/view.rs`). The row says it in words
    /// too; the role is the extra.
    pub role: PanelRole,
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
            role: PanelRole::Text,
        }
    }

    /// The same row drawn in `role`.
    #[must_use]
    pub fn with_role(mut self, role: PanelRole) -> Self {
        self.role = role;
        self
    }
}

/// What a panel's row is, for its colors: the folder's entry at that
/// place, a plugin list's rows plain text whatever their names.
pub fn row_role(app: &App, side: Side, index: usize, row: &ListRow) -> PanelRole {
    let panel = app.panel(side);
    if panel.plugin_list().is_some() {
        return PanelRole::Text;
    }
    match panel.entries().get(index).filter(|e| e.name == row.id) {
        Some(e) if e.kind == EntryKind::Folder => PanelRole::Folder,
        Some(e) if e.kind == EntryKind::Archive => PanelRole::Archive,
        Some(e) if e.kind == EntryKind::Link => match &e.link {
            Some(link) if link.points_to.is_none() => PanelRole::BrokenLink,
            _ => PanelRole::Link,
        },
        Some(e) if e.hidden => PanelRole::Hidden,
        _ => PanelRole::Text,
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

/// Whether `side` shows the quick view's preview region: the other panel
/// while the quick view is on, as in the terminal.
pub fn shows_preview(app: &App, side: Side) -> bool {
    app.quick_view_on() && side != app.active_side()
}

/// The preview region in the other panel's place (R27's commit 23): the
/// same list box as a panel, named "Right panel, Preview, notes.md", its
/// options the region's rows (a heading, a paragraph, a table's row) and
/// its focus the region's; before anything is shown, the line that says
/// how to go in ("Tab goes into the preview").
fn preview_view(app: &App, side: Side, top: usize, page: usize) -> Option<PanelView> {
    let quick = app.quick_view()?;
    let list = quick.region();
    let title = display_name(&quick.region_title()).into_owned();
    let total = list.len();
    let focus = list.focus_index();
    let top = scroll_top(top, focus, page, total);
    let range = window_range(top, page, total);
    let in_name = states_in_name();
    let rows = list.rows()[range.clone()]
        .iter()
        .map(|row| RowView::of(row, in_name))
        .collect();
    let mut hasher = DefaultHasher::new();
    "preview".hash(&mut hasher);
    title.hash(&mut hasher);
    let empty = if list.empty_text().is_empty() {
        app.preview_placeholder()
    } else {
        display_name(list.empty_text()).into_owned()
    };
    Some(PanelView {
        side,
        name: format!("{} panel, {title}", side_word(side)),
        header: title,
        generation: hasher.finish(),
        total,
        top,
        first: range.start,
        rows,
        focus,
        active: app.in_preview(),
        empty,
    })
}

/// A panel's view with the rows from `top` drawn and `page` rows visible.
pub fn panel_view(app: &App, side: Side, top: usize, page: usize) -> PanelView {
    if shows_preview(app, side)
        && let Some(view) = preview_view(app, side, top, page)
    {
        return view;
    }
    let panel = app.panel(side);
    let list = panel.shown_list();
    let total = list.len();
    let focus = list.focus_index();
    let top = scroll_top(top, focus, page, total);
    let range = window_range(top, page, total);
    let in_name = states_in_name();
    let rows = list.rows()[range.clone()]
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            RowView::of(row, in_name).with_role(row_role(app, side, range.start + offset, row))
        })
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
                role: PanelRole::Text,
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

/// What a dialog's focus is on: a text field, a list, or a
/// confirmation's buttons with its listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlView {
    /// A text field, with its candidates.
    Field(FieldView),
    /// A list to choose from (the app moves its focus), or lines to read
    /// (the window moves it: [`DialogView::local_list`]).
    List(ListView),
    /// A yes-or-no question: No and Yes as buttons, No focused, and the
    /// listing of what it touches as a list to read, with its count.
    Confirm(ConfirmView),
}

/// A confirmation's listing: every path it touches, the count in its
/// name ("Listing, 6 lines"), read line by line with the window's own
/// focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmView {
    /// The listing, when the question lists anything.
    pub listing: Option<ListView>,
}

/// Which control of a dialog has the keyboard focus. The app moves the
/// focus inside its own lists and fields; the window moves it between a
/// confirmation's buttons and listing, through a text's lines and to a
/// secret's Show toggle, which the app does not model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DialogFocus {
    /// The shape's own control: the field, or the list.
    #[default]
    Control,
    /// A confirmation's No button, where every confirmation starts.
    No,
    /// A confirmation's Yes button.
    Yes,
    /// A confirmation's listing.
    Listing,
    /// A question's message, read line by line (a new server key's
    /// fingerprint in groups of four).
    Message,
    /// A secret's Show toggle.
    Show,
}

/// The window's own focus in the open dialog: which control, and the line
/// of a text to read or of a confirmation's listing. It starts again for
/// every dialog ([`LocalFocus::start`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalFocus {
    /// The dialog it is for.
    pub dialog: Option<diroxide_core::DialogId>,
    /// The focused control.
    pub on: DialogFocus,
    /// The focused line, counted from the first.
    pub line: usize,
}

impl LocalFocus {
    /// Where the focus starts in a dialog: on No for a confirmation (the
    /// safe answer), on the control for every other shape.
    pub fn start(id: diroxide_core::DialogId, dialog: &diroxide_app::Dialog) -> Self {
        let on = match dialog {
            diroxide_app::Dialog::Confirm { .. } => DialogFocus::No,
            _ => DialogFocus::Control,
        };
        Self {
            dialog: Some(id),
            on,
            line: 0,
        }
    }
}

/// A button in a dialog (a confirmation's No and Yes) or a toggle (a
/// secret's Show), as a focusable control named by its label, with its
/// key as its keyboard shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushView {
    /// Which control it is, for the focus and for what pressing does.
    pub id: DialogFocus,
    /// The name, said and drawn: "No".
    pub label: String,
    /// The key, written ("n"); `None` when nothing is bound.
    pub key: Option<String>,
    /// A toggle's state (`Some`), or `None` for a button.
    pub toggled: Option<bool>,
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
    /// What a screen reader reads after the name as the dialog opens: a
    /// question's situation after its first line ("The one in Backup:
    /// 2.1 KB, modified yesterday"). `None` when the name says it all.
    pub description: Option<String>,
    /// Lines drawn under the title, before the control: a question's
    /// situation, a list's help. Readable text, never focused.
    pub lines: Vec<String>,
    /// A question's message as a list to read line by line, when it has
    /// two lines or more after its first (a new server key: the host,
    /// the key's type, the fingerprint one part a line): "Message, 10
    /// lines". Tab moves between it and the choices; the focus starts on
    /// the choices, the safe answer first.
    pub message: Option<ListView>,
    /// The control the focus is on.
    pub control: ControlView,
    /// The buttons and toggles under the control, in their order.
    pub buttons: Vec<PushView>,
    /// Which control has the keyboard focus.
    pub focus: DialogFocus,
    /// Whether the window moves the list's focus itself (a text to read,
    /// a listing), since the app does not model it.
    pub local_list: bool,
}

impl DialogView {
    /// Whether `other` is the same dialog in the same shape, so the
    /// modal is changed in place rather than built again (and its focus
    /// kept).
    pub fn same_shape(&self, other: &Self) -> bool {
        let ids = |b: &[PushView]| b.iter().map(|p| p.id).collect::<Vec<_>>();
        self.id == other.id
            && self.lines == other.lines
            && self.description == other.description
            && self.message.is_some() == other.message.is_some()
            && ids(&self.buttons) == ids(&other.buttons)
            && match (&self.control, &other.control) {
                (ControlView::Field(_), ControlView::Field(_))
                | (ControlView::List(_), ControlView::List(_)) => true,
                (ControlView::Confirm(a), ControlView::Confirm(b)) => {
                    a.listing.is_some() == b.listing.is_some()
                }
                _ => false,
            }
    }

    /// The controls Tab moves through, in order: a confirmation's No, Yes
    /// and listing; a secret's field and Show toggle; else the one
    /// control.
    pub fn tab_order(&self) -> Vec<DialogFocus> {
        let mut order = Vec::new();
        match &self.control {
            ControlView::Confirm(confirm) => {
                order.extend(self.buttons.iter().map(|b| b.id));
                if confirm.listing.is_some() {
                    order.push(DialogFocus::Listing);
                }
            }
            _ => {
                order.push(DialogFocus::Control);
                if self.message.is_some() {
                    order.push(DialogFocus::Message);
                }
                order.extend(self.buttons.iter().map(|b| b.id));
            }
        }
        order
    }

    /// Whether `line` is what the dialog says by itself when the focus
    /// moves into it (its name, then the focused control), so the app's
    /// own line on opening it would say the same twice: the question, a
    /// list's title with its focused row, a field's label with its text,
    /// a password's prompt. The app's line is then left unsaid (the
    /// settled double: the dialog says it, the live region does not).
    pub fn says(&self, line: &str) -> bool {
        let line = display_name(line);
        !self.title.is_empty() && line.starts_with(self.title.as_str())
    }
}

/// The rows a dialog's list shows at a time, for its window.
pub const DIALOG_PAGE: usize = 12;

/// A confirmation's listing as a list to read, named with its count.
pub fn listing_view(listing: &[String], line: usize) -> Option<ListView> {
    (!listing.is_empty()).then(|| {
        ListView::lines(
            format!("Listing, {}", count(listing.len() as u64, "line", "lines")),
            listing,
            line,
        )
    })
}

/// A confirmation's two buttons, No first: each named by its word, with
/// the key the keymap gives the answer.
pub fn confirm_buttons(app: &App) -> Vec<PushView> {
    use diroxide_app::commands::ids;
    [
        (DialogFocus::No, "No", ids::NO),
        (DialogFocus::Yes, "Yes", ids::YES),
    ]
    .into_iter()
    .map(|(id, label, command)| {
        let key = app.keymap().key_text(&command);
        PushView {
            id,
            label: label.to_owned(),
            key: (!key.is_empty()).then_some(key),
            toggled: None,
        }
    })
    .collect()
}

/// The key that turns a secret's Show toggle, as the app reads it.
pub const SHOW_KEY: &str = "Alt+S";

/// A secret's Show toggle: "Show the password", on or off, with its key.
pub fn show_toggle(request: &diroxide_core::SecretRequest, shown: bool) -> PushView {
    PushView {
        id: DialogFocus::Show,
        label: format!("Show the {}", request.kind.word().to_lowercase()),
        key: Some(SHOW_KEY.to_owned()),
        toggled: Some(shown),
    }
}

/// The top dialog as the modal host shows it, with the focus where it
/// starts; see [`dialog_view_at`].
pub fn dialog_view(app: &App) -> Option<DialogView> {
    let (id, dialog) = app.dialog()?;
    dialog_view_at(app, &LocalFocus::start(id, dialog))
}

/// The top dialog as the modal host shows it with the window's own focus
/// `local` (which starts again when it is for another dialog), or
/// `None`.
pub fn dialog_view_at(app: &App, local: &LocalFocus) -> Option<DialogView> {
    use diroxide_app::Dialog;
    use diroxide_app::entry::CompletionSource;
    let (id, dialog) = app.dialog()?;
    let start;
    let local = if local.dialog == Some(id) {
        local
    } else {
        start = LocalFocus::start(id, dialog);
        &start
    };
    let title = display_name(&dialog.title()).into_owned();
    let silent = app.settings().secret_echo == diroxide_app::config::SecretEcho::Silent;
    let mut buttons = Vec::new();
    let mut description = None;
    let mut local_list = false;
    let mut message = None;
    let (lines, control) = match dialog {
        Dialog::Confirm { listing, .. } => {
            // The answer starts on No; y, n, Enter and Escape answer it,
            // and the buttons press as those keys do.
            buttons = confirm_buttons(app);
            local_list = true;
            (
                Vec::new(),
                ControlView::Confirm(ConfirmView {
                    listing: listing_view(listing, local.line),
                }),
            )
        }
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
        Dialog::Read { title, lines } => {
            // The app keeps no focus in a text to read; the window moves
            // through its lines itself.
            local_list = true;
            (
                Vec::new(),
                ControlView::List(ListView::lines(title.clone(), lines, local.line)),
            )
        }
        Dialog::Secret { request, shown, .. } => {
            // The Show toggle after the field: off by default, its key the
            // app's (Alt+S). While it is on, the field shows the text and
            // a screen reader reads it, as the app says when it is turned
            // on; while it is off, only the mask reaches the field, the
            // drawing and the tree.
            buttons = vec![show_toggle(request, *shown)];
            let shown_text = dialog.masked_text(silent).unwrap_or_default();
            (
                Vec::new(),
                ControlView::Field(FieldView {
                    label: request.title(),
                    cursor: shown_text.chars().count(),
                    text: shown_text,
                    secret: !*shown,
                    combo: false,
                    candidates: None,
                    in_list: false,
                }),
            )
        }
        Dialog::Question { lines, choices } => {
            // The first line names the dialog ("Name taken: report.pdf");
            // the rest of the situation is drawn under it and read after
            // the name as the dialog opens, before the safe choice.
            let rest: Vec<String> = lines.iter().skip(1).cloned().collect();
            if !rest.is_empty() {
                description = Some(display_name(&rest.join(" ")).into_owned());
            }
            // Two lines or more are a list to read line by line too, so a
            // Braille display shows one part of a fingerprint at a time.
            let drawn = if rest.len() >= 2 {
                message = Some(ListView::lines(
                    format!("Message, {}", count(rest.len() as u64, "line", "lines")),
                    &rest,
                    local.line,
                ));
                Vec::new()
            } else {
                rest
            };
            (
                drawn,
                ControlView::List(ListView::of(choices, choices.title(), DIALOG_PAGE)),
            )
        }
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
                // A field that completes, or that a list of candidates
                // goes with (the Keys list, a typed confirmation's
                // listing), is a combo box naming its list.
                combo: candidates.is_some()
                    || !matches!(
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
    // The focus is on a control the dialog has: a confirmation's listing
    // only when it lists something, else No.
    let mut view = DialogView {
        id,
        title,
        description,
        lines: lines
            .iter()
            .map(|line| display_name(line).into_owned())
            .collect(),
        message,
        control,
        buttons,
        focus: local.on,
        local_list,
    };
    if !view.tab_order().contains(&view.focus) {
        view.focus = view.tab_order().first().copied().unwrap_or_default();
    }
    Some(view)
}

/// The messages the app said in one step as the one announcement the
/// live region makes, as the terminal's status line joins the messages
/// between two frames: the lines in their order joined with a space, a
/// line the same as the one before it once, a job's start line dropped
/// when more came after it (a quick job's result), Low lines left out
/// (they are only shown). The announcement is as urgent as its most
/// urgent line. Each entry is a message and whether it is a job's start
/// line. Empty when nothing is to be said.
pub fn join_messages(said: &[(diroxide_core::Message, bool)]) -> Vec<diroxide_core::Message> {
    use diroxide_core::Importance;
    let spoken: Vec<&(diroxide_core::Message, bool)> = said
        .iter()
        .filter(|(m, _)| m.importance != Importance::Low && !m.text.is_empty())
        .collect();
    let Some(last) = spoken.len().checked_sub(1) else {
        return Vec::new();
    };
    let mut parts: Vec<&str> = Vec::new();
    let mut importance = Importance::Normal;
    for (i, (message, start)) in spoken.iter().enumerate() {
        if *start && i < last {
            continue;
        }
        if parts.last() == Some(&message.text.as_str()) {
            continue;
        }
        parts.push(&message.text);
        importance = importance.max(message.importance);
    }
    let kind = spoken[last].0.kind;
    vec![diroxide_core::Message::new(
        parts.join(" "),
        importance,
        kind,
    )]
}

/// One button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonView {
    /// What pressing it does, given back in the action: an index or a
    /// command's id.
    pub id: String,
    /// The name, said and shown: "Rename".
    pub label: String,
    /// The key, written ("F2"), shown before the label and given as the
    /// keyboard shortcut; `None` when nothing is bound.
    pub key: Option<String>,
    /// Whether the drawn text starts with the key ("F2 Rename"), as the
    /// key bar draws it.
    pub key_first: bool,
}

impl ButtonView {
    /// What is drawn.
    pub fn text(&self) -> String {
        match (&self.key, self.key_first) {
            (Some(key), true) => format!("{key} {}", self.label),
            _ => self.label.clone(),
        }
    }
}

/// The theme role a token class is drawn with (the theme's token roles,
/// ADR-0007's status update of October 17), or `None` for plain text. As
/// the terminal maps them (`crates/diroxide-tui/src/viewer.rs`,
/// `class_role`).
pub fn class_role(class: diroxide_app::viewer::TokenClass) -> Option<PanelRole> {
    use diroxide_app::settings::PanelRole as R;
    use diroxide_app::viewer::TokenClass as C;
    Some(match class {
        C::Plain => return None,
        C::Keyword => R::TokenKeyword,
        C::String => R::TokenString,
        C::Comment => R::TokenComment,
        C::Number => R::TokenNumber,
        C::Constant => R::TokenConstant,
        C::FunctionName => R::TokenFunction,
        C::TypeName => R::TokenType,
        C::Variable => R::TokenVariable,
        C::Operator => R::TokenOperator,
        C::Punctuation => R::TokenPunctuation,
        C::Heading => R::TokenHeading,
        C::Link => R::TokenLink,
        C::Emphasis => R::TokenEmphasis,
        C::Code => R::TokenCode,
        C::Tag => R::TokenTag,
        C::Attribute => R::TokenAttribute,
        C::Problem => R::TokenProblem,
        _ => return None,
    })
}

/// One row of the viewer, as an option of its list box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerRowView {
    /// The row's place, kept while it stays drawn: "LINE:PART".
    pub id: String,
    /// The text as drawn: the line number when they are on, or
    /// "continued: " on a continued part, then the text.
    pub text: String,
    /// The option's name: the text with hidden characters spelled out.
    pub name: String,
    /// The line, from 0.
    pub line: u64,
    /// The token runs, byte ranges of `text` with the theme role each is
    /// drawn with (none when highlighting is off).
    pub runs: Vec<(usize, usize, PanelRole)>,
    /// The search's matches in `text`, drawn marked.
    pub matches: Vec<(usize, usize)>,
}

/// The viewer as its list box shows it: the rows on screen, which is all
/// a 100,000-line file gives the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerView {
    /// The list box's name: "Viewer, report.txt" (", hex" in hex mode).
    pub name: String,
    /// Changes when another file or mode is shown, so the rows' node ids
    /// start again.
    pub generation: u64,
    /// The rows on screen.
    pub rows: Vec<ViewerRowView>,
    /// The cursor's row in `rows`.
    pub focus: Option<usize>,
    /// The file's lines, as far as they are counted (the list's size).
    pub lines: u64,
}

/// Whether the viewer draws token classes: the `highlighting` setting,
/// automatic meaning colored except in screen-reader mode, as in the
/// terminal (a screen reader hears classes through Say Token Kind and
/// Say Line by Kind).
pub fn colors_code(app: &App) -> bool {
    use diroxide_app::viewer::HighlightSetting;
    match app.settings().highlighting {
        HighlightSetting::On => true,
        HighlightSetting::Off => false,
        HighlightSetting::Auto => !app.screen_reader(),
    }
}

/// The viewer's rows for a list `columns` wide and `rows` high, after
/// telling the viewer that size; `None` when no viewer is open.
pub fn viewer_view(app: &mut App, columns: usize, rows: usize) -> Option<ViewerView> {
    use diroxide_app::viewer::ViewerMode;
    let colored = colors_code(app);
    let viewer = app.viewer_mut()?;
    viewer.set_size(columns.max(1), rows.max(1));
    let hex = viewer.mode() == ViewerMode::Hex;
    let name = if hex {
        format!("Viewer, {}, hex", display_name(viewer.name()))
    } else {
        format!("Viewer, {}", display_name(viewer.name()))
    };
    let mut hasher = DefaultHasher::new();
    viewer.place().to_string().hash(&mut hasher);
    hex.hash(&mut hasher);
    let generation = hasher.finish();
    let shown = viewer.rows();
    let mut out = Vec::with_capacity(shown.len());
    let mut focus = None;
    for (i, row) in shown.iter().enumerate() {
        if row.focused {
            focus = Some(i);
        }
        let runs = if colored && !hex {
            viewer
                .row_runs(row)
                .iter()
                .filter_map(|run| {
                    class_role(run.class).map(|role| (run.start as usize, run.end as usize, role))
                })
                .collect()
        } else {
            Vec::new()
        };
        let matches = viewer
            .match_ranges(&row.text)
            .into_iter()
            .map(|m| (m.start, m.end))
            .collect();
        out.push(ViewerRowView {
            id: format!("{}:{}", row.pos.line, row.pos.part),
            text: row.text.clone(),
            name: display_name(&row.text).into_owned(),
            line: row.pos.line,
            runs,
            matches,
        });
    }
    let lines = viewer
        .line_count()
        .unwrap_or_else(|| viewer.lines_known())
        .max(1);
    Some(ViewerView {
        name,
        generation,
        rows: out,
        focus,
        lines,
    })
}

/// The key bar's name, as a toolbar.
pub const KEY_BAR_NAME: &str = "Key bar";

/// The key bar's buttons: the shown bank's buttons that have a key, the
/// key first as the bar draws it ("F2 Rename"), each named by its label
/// with the key as its shortcut; none when the `key_bar` setting is off.
/// The id is the button's place in the bank.
pub fn key_bar_buttons(app: &App) -> Vec<ButtonView> {
    if !app.settings().key_bar {
        return Vec::new();
    }
    let Some(bank) = app.shown_bank() else {
        return Vec::new();
    };
    bank.buttons
        .iter()
        .enumerate()
        .filter_map(|(i, button)| {
            let key = app.keymap().key_text(&button.commands.first()?.id);
            (!key.is_empty()).then(|| ButtonView {
                id: i.to_string(),
                label: button.label.clone(),
                key: Some(key),
                key_first: true,
            })
        })
        .collect()
}

/// A panel's sort buttons' toolbar name: "Sort the left panel".
pub fn sort_row_name(side: Side) -> String {
    format!("Sort the {} panel", side.word())
}

/// A panel's sort buttons, above its rows: by name, by size, by date
/// modified, each named for what it does with its key as its shortcut.
/// The id is the command's.
pub fn sort_buttons(app: &App) -> Vec<ButtonView> {
    use diroxide_app::commands::ids;
    [
        ("Sort by name", ids::SORT_NAME),
        ("Sort by size", ids::SORT_SIZE),
        ("Sort by date", ids::SORT_MODIFIED),
    ]
    .into_iter()
    .map(|(label, id)| {
        let key = app.keymap().key_text(&id);
        ButtonView {
            id: id.to_string(),
            label: label.to_owned(),
            key: (!key.is_empty()).then_some(key),
            key_first: false,
        }
    })
    .collect()
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
    fn a_steps_lines_are_one_announcement_in_their_order() {
        use diroxide_core::{Importance, Message};
        let said = [
            (Message::info("Copy started: 1 file to Backup"), true),
            (Message::status("Reading 40 percent"), false),
            (Message::info("Copy finished: 1 done"), false),
            (Message::info("Copy finished: 1 done"), false),
            (Message::error("Could not list Backup: gone. Go up."), false),
        ];
        let joined = join_messages(&said);
        assert_eq!(joined.len(), 1);
        assert_eq!(
            joined[0].text,
            "Copy finished: 1 done Could not list Backup: gone. Go up."
        );
        assert_eq!(joined[0].importance, Importance::High);
        // A start line alone is said.
        let alone = join_messages(&said[..1]);
        assert_eq!(alone[0].text, "Copy started: 1 file to Backup");
        // Low only: nothing is said.
        assert!(join_messages(&said[1..2]).is_empty());
        assert!(join_messages(&[]).is_empty());
    }

    #[test]
    fn a_hidden_character_in_a_name_is_never_drawn_or_said() {
        let row = ListRow::new("bad\u{f03a}name", "bad\u{f03a}name");
        let option = RowView::of(&row, false);
        assert!(!option.name.contains('\u{f03a}'), "{}", option.name);
        assert!(!option.text.contains('\u{f03a}'), "{}", option.text);
    }
}
