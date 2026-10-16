//! The screen as lines of runs, built from the app's state: the one place
//! that decides what each line says and where the hardware cursor goes.
//! Both the drawn frame and [`crate::render`]'s plain lines come from it,
//! so they never drift apart.
//!
//! From the top: the title line (the active place first; hidden in compact
//! mode), the body (the panels in their layout, or the list dialog on top),
//! the minibuffer when a text field is open (the prompt, then its
//! candidates), the key bar (sighted mode only), the status line and the
//! jobs line.
//!
//! The rules (ADR-0007 as amended; R4 section 3.2; research/alpha1-plan.md
//! part three):
//!
//! - The cursor sits at the focused row's first cell in every state: the
//!   panel's focused row, the "Empty folder" or "Reading folder" line, a
//!   dialog's focused choice, a confirmation's question, a prompt's
//!   insertion point, a candidate in the list. With `cursor_fallback` on
//!   it is parked on the status line instead (a prompt keeps its
//!   insertion point, since that is where typing goes).
//! - Each row is one text run, meaning first: "report.pdf, PDF, 2.1 MB";
//!   a selected row starts with an asterisk and says "selected" after its
//!   name. Sighted rows keep two spaces where the asterisk goes, so names
//!   line up; screen-reader rows spend no cells on it.
//! - Only display names are drawn: every hidden character becomes a
//!   question mark before it reaches the terminal.
//! - In screen-reader mode the inactive panel is drawn as it was when it
//!   was last drawn, until its place changes, it becomes active, or Say
//!   Other Panel asks for it; a change in the background (a copy ending in
//!   the other panel, a watch) never redraws it.
//! - In screen-reader mode a candidate list is drawn only when the focus
//!   is in it; while typing, one line says the count.
//! - Rows scroll only as far as the focus needs, so moving inside the
//!   visible rows never rewrites them.

use std::collections::HashMap;

use diroxide_app::config::Layout;
use diroxide_app::panel::place_title;
use diroxide_app::{App, Dialog};
use diroxide_core::format::count;
use diroxide_core::names::display_name;
use diroxide_core::{DialogId, EntryKind, ListModel, ListRow, Place, Side};
use diroxide_theme::PanelRole;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::style::Styles;

/// Text drawn from a cell onward, at most `width` cells, in a role's
/// style; `fill` paints the whole width in the style first (a highlight
/// bar).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub x: u16,
    pub width: u16,
    pub text: String,
    pub role: PanelRole,
    pub fill: bool,
}

impl Run {
    fn new(x: u16, width: u16, text: impl Into<String>, role: PanelRole) -> Self {
        Self {
            x,
            width,
            text: text.into(),
            role,
            fill: false,
        }
    }

    fn filled(mut self) -> Self {
        self.fill = true;
        self
    }
}

/// A line: its runs, left to right.
pub(crate) type Line = Vec<Run>;

/// How a frame is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub width: u16,
    pub height: u16,
    pub screen_reader: bool,
    pub borders: bool,
    pub compact: bool,
    pub cursor_fallback: bool,
    pub status_blank: bool,
    pub secret_silent: bool,
}

/// What lies where: the frame's lines and the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct View {
    pub width: u16,
    pub height: u16,
    pub lines: Vec<Line>,
    /// The cursor: column, line.
    pub cursor: (u16, u16),
    /// The rows each panel shows, for Page Up and Page Down.
    pub page_rows: [usize; 2],
}

/// Where a list's scroll is kept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ScrollKey {
    Panel(Side),
    Dialog(DialogId),
    Candidates(DialogId),
}

/// A panel as last drawn while inactive, in screen-reader mode.
#[derive(Clone, Debug)]
struct Frozen {
    place: Place,
    complete: bool,
    area: (u16, u16, u16),
    lines: Vec<Line>,
}

/// What a frame remembers for the next: scroll offsets, and in
/// screen-reader mode the inactive panel as last drawn.
#[derive(Clone, Debug, Default)]
pub(crate) struct Memory {
    /// Whether the inactive panel is frozen (the session in screen-reader
    /// mode); [`crate::render`] draws the current state.
    pub freeze: bool,
    scroll: HashMap<ScrollKey, (Option<Place>, usize)>,
    frozen: [Option<Frozen>; 2],
}

impl Memory {
    /// Forgets a panel's frozen drawing, so the next frame draws it as it
    /// is now (Say Other Panel, a layout change).
    pub fn thaw(&mut self, side: Side) {
        self.frozen[side.index()] = None;
    }

    /// Forgets both panels' frozen drawings.
    pub fn thaw_all(&mut self) {
        self.frozen = [None, None];
    }
}

/// The text as drawn: hidden characters as question marks (ADR-0002).
pub(crate) fn clean(text: &str) -> String {
    display_name(text).into_owned()
}

/// A text's width in terminal cells.
pub(crate) fn cells(text: &str) -> usize {
    Span::raw(text).width()
}

/// The first rows a list shows: the last start, moved only as far as the
/// focus needs.
fn window(previous: usize, focus: Option<usize>, height: usize, len: usize) -> usize {
    let height = height.max(1);
    let mut start = previous.min(len.saturating_sub(height));
    if let Some(focus) = focus {
        if focus < start {
            start = focus;
        } else if focus >= start + height {
            start = focus + 1 - height;
        }
    }
    start
}

impl Memory {
    fn start(
        &mut self,
        key: ScrollKey,
        place: Option<&Place>,
        focus: Option<usize>,
        height: usize,
        len: usize,
    ) -> usize {
        let previous = match self.scroll.get(&key) {
            Some((kept, start)) if kept.as_ref() == place => *start,
            _ => 0,
        };
        let start = window(previous, focus, height, len);
        self.scroll.insert(key, (place.cloned(), start));
        start
    }
}

/// A row as drawn: the asterisk first on a selected row (the row also says
/// "selected"), two spaces in its place on a sighted unselected row.
pub(crate) fn row_text(row: &ListRow, screen_reader: bool) -> String {
    let spoken = clean(&row.spoken());
    if row.selected {
        format!("* {spoken}")
    } else if screen_reader {
        spoken
    } else {
        format!("  {spoken}")
    }
}

/// A panel row's role: the focus, a mark, then what the entry is.
fn row_role(app: &App, side: Side, index: usize, row: &ListRow, focused: bool) -> PanelRole {
    if focused {
        return PanelRole::FocusedRow;
    }
    if row.selected {
        return PanelRole::MarkedRow;
    }
    let entry = app
        .panel(side)
        .entries()
        .get(index)
        .filter(|e| e.name == row.id);
    match entry {
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

/// Lines of a region and the line and column of its focus, relative to
/// the region's top.
struct Region {
    lines: Vec<Line>,
    focus: Option<(u16, usize)>,
}

/// A panel in a region: its header line when asked for, then its rows.
#[allow(clippy::too_many_arguments)]
fn panel_region(
    app: &App,
    side: Side,
    x: u16,
    width: u16,
    height: usize,
    header: bool,
    active: bool,
    look: &Look,
    memory: &mut Memory,
) -> Region {
    let panel = app.panel(side);
    let shown = panel.loading_place().unwrap_or(panel.place()).clone();
    let frozen_ok = memory.freeze && !active;
    if frozen_ok
        && let Some(f) = &memory.frozen[side.index()]
        && f.complete
        && f.place == shown
        && f.area == (x, width, height as u16)
    {
        return Region {
            lines: f.lines.clone(),
            focus: None,
        };
    }
    let mut lines: Vec<Line> = Vec::new();
    if header && height > 0 {
        let mut text = clean(&place_title(&shown));
        if panel.is_loading() {
            text.push_str(", loading");
        }
        let role = if active {
            PanelRole::ActiveTitle
        } else {
            PanelRole::InactiveTitle
        };
        lines.push(vec![Run::new(x, width, text, role)]);
    }
    let rows = height.saturating_sub(lines.len());
    let list = panel.list();
    let mut focus = None;
    if rows > 0 {
        if list.is_empty() {
            let text = if panel.is_loading() {
                "Reading folder".to_owned()
            } else {
                clean(list.empty_text())
            };
            if active {
                focus = Some((x, lines.len()));
            }
            lines.push(vec![Run::new(x, width, text, PanelRole::Text)]);
        } else {
            let focused = list.focus_index();
            let start = memory.start(
                ScrollKey::Panel(side),
                Some(panel.place()),
                focused,
                rows,
                list.len(),
            );
            for (i, row) in list.rows().iter().enumerate().skip(start).take(rows) {
                let is_focus = active && Some(i) == focused;
                if is_focus {
                    focus = Some((x, lines.len()));
                }
                let role = row_role(app, side, i, row, is_focus);
                let mut run = Run::new(x, width, row_text(row, look.screen_reader), role);
                if is_focus {
                    run = run.filled();
                }
                lines.push(vec![run]);
            }
        }
    }
    if frozen_ok {
        let complete = panel.is_listed()
            && (!panel.is_loading()
                || panel.loading_how() == Some(diroxide_app::panel::Load::Refresh));
        memory.frozen[side.index()] = Some(Frozen {
            place: shown,
            complete,
            area: (x, width, height as u16),
            lines: lines.clone(),
        });
    }
    Region { lines, focus }
}

/// The panels in the layout the settings name.
fn panels_body(
    app: &App,
    look: &Look,
    memory: &mut Memory,
    width: u16,
    height: usize,
) -> (Region, [usize; 2]) {
    let active = app.active_side();
    memory.thaw(active);
    let layout = app.settings().layout;
    let mut pages = [height.max(1); 2];
    match layout {
        Layout::Single => {
            let region = panel_region(app, active, 0, width, height, false, true, look, memory);
            (region, pages)
        }
        Layout::SideBySide => {
            let left_w = width.saturating_sub(1) / 2;
            let right_x = left_w + 1;
            let right_w = width.saturating_sub(right_x);
            let left = panel_region(
                app,
                Side::Left,
                0,
                left_w,
                height,
                true,
                active == Side::Left,
                look,
                memory,
            );
            let right = panel_region(
                app,
                Side::Right,
                right_x,
                right_w,
                height,
                true,
                active == Side::Right,
                look,
                memory,
            );
            pages = [height.saturating_sub(1).max(1); 2];
            let mut lines: Vec<Line> = Vec::new();
            for i in 0..height {
                let mut line = left.lines.get(i).cloned().unwrap_or_default();
                if look.borders {
                    line.push(Run::new(left_w, 1, "|", PanelRole::InactiveTitle));
                }
                line.extend(right.lines.get(i).cloned().unwrap_or_default());
                lines.push(line);
            }
            let focus = if active == Side::Left {
                left.focus
            } else {
                right.focus
            };
            (Region { lines, focus }, pages)
        }
        Layout::Stacked => {
            let top_h = (height / 2).max(1).min(height);
            let bottom_h = height - top_h;
            let top = panel_region(
                app,
                Side::Left,
                0,
                width,
                top_h,
                true,
                active == Side::Left,
                look,
                memory,
            );
            let bottom = panel_region(
                app,
                Side::Right,
                0,
                width,
                bottom_h,
                true,
                active == Side::Right,
                look,
                memory,
            );
            pages = [
                top_h.saturating_sub(1).max(1),
                bottom_h.saturating_sub(1).max(1),
            ];
            let mut lines = top.lines;
            lines.resize(top_h, Vec::new());
            let focus = match active {
                Side::Left => top.focus,
                Side::Right => bottom.focus.map(|(x, line)| (x, line + top_h)),
            };
            lines.extend(bottom.lines);
            (Region { lines, focus }, pages)
        }
    }
}

/// A list's rows in a window of `height` lines from line `first`.
#[allow(clippy::too_many_arguments)]
fn list_rows(
    list: &ListModel,
    key: ScrollKey,
    width: u16,
    height: usize,
    show_focus: bool,
    role: PanelRole,
    memory: &mut Memory,
    lines: &mut Vec<Line>,
    focus: &mut Option<(u16, usize)>,
) {
    if height == 0 {
        return;
    }
    if list.is_empty() {
        if show_focus {
            *focus = Some((0, lines.len()));
        }
        lines.push(vec![Run::new(0, width, clean(list.empty_text()), role)]);
        return;
    }
    let focused = list.focus_index();
    let start = memory.start(key, None, focused, height, list.len());
    for (i, row) in list.rows().iter().enumerate().skip(start).take(height) {
        let is_focus = show_focus && Some(i) == focused;
        if is_focus {
            *focus = Some((0, lines.len()));
        }
        let text = clean(&row.spoken());
        let mut run = Run::new(
            0,
            width,
            text,
            if is_focus {
                PanelRole::FocusedRow
            } else {
                role
            },
        );
        if is_focus {
            run = run.filled();
        }
        lines.push(vec![run]);
    }
}

/// A list dialog covering the body: its title or question first.
fn dialog_body(
    id: DialogId,
    dialog: &Dialog,
    memory: &mut Memory,
    width: u16,
    height: usize,
) -> Region {
    let mut lines: Vec<Line> = Vec::new();
    let mut focus = None;
    match dialog {
        Dialog::Choose { list, .. } => {
            lines.push(vec![Run::new(
                0,
                width,
                clean(list.title()),
                PanelRole::ActiveTitle,
            )]);
            let rows = height.saturating_sub(1);
            list_rows(
                list,
                ScrollKey::Dialog(id),
                width,
                rows,
                true,
                PanelRole::Dialog,
                memory,
                &mut lines,
                &mut focus,
            );
            if focus.is_none() {
                focus = Some((0, 0));
            }
        }
        Dialog::Question {
            lines: said,
            choices,
        } => {
            let room = height.saturating_sub(choices.len().clamp(1, 3));
            for (i, line) in said.iter().take(room.max(1)).enumerate() {
                let role = if i == 0 {
                    PanelRole::ActiveTitle
                } else {
                    PanelRole::Dialog
                };
                lines.push(vec![Run::new(0, width, clean(line), role)]);
            }
            let rows = height.saturating_sub(lines.len());
            list_rows(
                choices,
                ScrollKey::Dialog(id),
                width,
                rows,
                true,
                PanelRole::Dialog,
                memory,
                &mut lines,
                &mut focus,
            );
            if focus.is_none() {
                focus = Some((0, 0));
            }
        }
        Dialog::Confirm {
            question, listing, ..
        } => {
            lines.push(vec![Run::new(
                0,
                width,
                clean(&question.text),
                PanelRole::DefaultButton,
            )]);
            focus = Some((0, 0));
            let room = height.saturating_sub(1);
            let shown = if listing.len() > room {
                room.saturating_sub(1)
            } else {
                listing.len()
            };
            for line in listing.iter().take(shown) {
                lines.push(vec![Run::new(0, width, clean(line), PanelRole::Dialog)]);
            }
            if listing.len() > shown && room > 0 {
                let more = match listing.len() - shown {
                    1 => "And 1 more line".to_owned(),
                    n => format!("And {n} more lines"),
                };
                lines.push(vec![Run::new(0, width, more, PanelRole::Dialog)]);
            }
        }
        Dialog::Read { title, lines: text } => {
            lines.push(vec![Run::new(
                0,
                width,
                clean(title),
                PanelRole::ActiveTitle,
            )]);
            focus = Some((0, 0));
            for line in text.iter().take(height.saturating_sub(1)) {
                lines.push(vec![Run::new(0, width, clean(line), PanelRole::Dialog)]);
            }
        }
        Dialog::Prompt { .. } | Dialog::Edit { .. } | Dialog::Secret { .. } => {}
    }
    lines.truncate(height);
    Region { lines, focus }
}

/// Whether a dialog lives in the minibuffer (a text field) rather than
/// over the body.
pub(crate) fn in_minibuffer(dialog: &Dialog) -> bool {
    matches!(
        dialog,
        Dialog::Edit { .. } | Dialog::Secret { .. } | Dialog::Prompt { .. }
    )
}

/// A prompt line and its insertion point's column: "Rename report.pdf to:
/// report.pdf". A label too long for the line is cut (it was said whole
/// when the prompt opened); a long text scrolls so the insertion point
/// stays on the line.
pub(crate) fn prompt_line(label: &str, text: &str, cursor: usize, width: u16) -> (String, u16) {
    let width = usize::from(width.max(2));
    let label = clean(label);
    let text = clean(text);
    let before: String = text.chars().take(cursor).collect();
    let after: String = text.chars().skip(cursor).collect();
    let mut head = if label.is_empty() {
        String::new()
    } else {
        format!("{label}: ")
    };
    if cells(&head) + cells(&before) < width {
        let column = cells(&head) + cells(&before);
        return (format!("{head}{before}{after}"), column as u16);
    }
    // Cut the label to half the line, then keep the end of the text
    // before the insertion point.
    let half = width / 2;
    if cells(&head) > half {
        let mut cut = String::new();
        for c in head.chars() {
            if cells(&cut) + cells(&c.to_string()) + 2 > half {
                break;
            }
            cut.push(c);
        }
        head = format!("{}: ", cut.trim_end_matches([':', ' ']));
    }
    let room = width.saturating_sub(cells(&head) + 1);
    let mut kept: Vec<char> = Vec::new();
    let mut used = 0;
    for c in before.chars().rev() {
        let w = cells(&c.to_string());
        if used + w > room {
            break;
        }
        used += w;
        kept.push(c);
    }
    kept.reverse();
    let kept: String = kept.into_iter().collect();
    let column = cells(&head) + used;
    (format!("{head}{kept}{after}"), column as u16)
}

/// The minibuffer: the prompt line and, below it, the candidates.
struct Minibuffer {
    lines: Vec<Line>,
    cursor: (u16, usize),
    /// Whether the cursor is in the candidate list (not the field).
    in_list: bool,
}

fn minibuffer(
    id: DialogId,
    dialog: &Dialog,
    look: &Look,
    memory: &mut Memory,
    width: u16,
    room: usize,
) -> Minibuffer {
    match dialog {
        Dialog::Edit {
            field,
            candidates,
            in_list,
        } => {
            let (line, column) = prompt_line(field.label(), field.text(), field.cursor(), width);
            let mut lines = vec![vec![Run::new(0, width, line, PanelRole::Dialog)]];
            let mut cursor = (column, 0);
            let mut entered = false;
            if let Some(list) = candidates {
                let rows = room.min(8);
                if look.screen_reader && !*in_list {
                    if rows > 0 {
                        let said = if list.is_empty() {
                            clean(list.empty_text())
                        } else if list.title().contains("Down arrow") {
                            clean(list.title())
                        } else {
                            format!(
                                "{}, Down arrow lists them",
                                count(list.len() as u64, "match", "matches")
                            )
                        };
                        lines.push(vec![Run::new(0, width, said, PanelRole::Dialog)]);
                    }
                } else {
                    let mut focus = None;
                    list_rows(
                        list,
                        ScrollKey::Candidates(id),
                        width,
                        rows,
                        *in_list,
                        PanelRole::Dialog,
                        memory,
                        &mut lines,
                        &mut focus,
                    );
                    if let Some((x, line)) = focus.filter(|_| *in_list) {
                        cursor = (x, line);
                        entered = true;
                    }
                }
            }
            Minibuffer {
                lines,
                cursor,
                in_list: entered,
            }
        }
        Dialog::Secret { request, .. } => {
            let masked = dialog.masked_text(look.secret_silent).unwrap_or_default();
            let prompt = request.prompt_line();
            let (line, column) = prompt_line("", &format!("{prompt}{masked}"), usize::MAX, width);
            Minibuffer {
                lines: vec![vec![Run::new(0, width, line, PanelRole::Dialog)]],
                cursor: (column, 0),
                in_list: false,
            }
        }
        Dialog::Prompt { label, text, .. } => {
            let (line, column) = prompt_line(label, text, text.chars().count(), width);
            Minibuffer {
                lines: vec![vec![Run::new(0, width, line, PanelRole::Dialog)]],
                cursor: (column, 0),
                in_list: false,
            }
        }
        _ => Minibuffer {
            lines: Vec::new(),
            cursor: (0, 0),
            in_list: false,
        },
    }
}

/// Builds the frame for the app's state.
pub(crate) fn build(app: &App, look: &Look, memory: &mut Memory) -> View {
    let width = look.width.max(20);
    let height = usize::from(look.height.max(5));
    let status = if look.status_blank {
        String::new()
    } else {
        clean(&app.status_line())
    };
    let jobs = clean(&app.jobs().line());
    let bar = if look.screen_reader || look.compact {
        String::new()
    } else {
        clean(&app.key_bar())
    };
    let mut show_title = !look.compact;
    let mut show_bar = !bar.is_empty();
    let top = app.dialog();
    let mini_dialog = top.filter(|(_, d)| in_minibuffer(d));
    let mini_base = usize::from(mini_dialog.is_some());
    let fixed = |title: bool, bar: bool| usize::from(title) + usize::from(bar) + 2 + mini_base;
    if fixed(show_title, show_bar) + 1 > height {
        show_bar = false;
    }
    if fixed(show_title, show_bar) + 1 > height {
        show_title = false;
    }
    let spare = height.saturating_sub(fixed(show_title, show_bar));
    // The candidates take what the body can spare, keeping three body
    // lines where the screen has them.
    let candidate_room = spare.saturating_sub(3.min(spare.saturating_sub(1)));
    let mini =
        mini_dialog.map(|(id, dialog)| minibuffer(id, dialog, look, memory, width, candidate_room));
    let mini_lines = mini.as_ref().map_or(0, |m| m.lines.len());
    let body_h = height.saturating_sub(fixed(show_title, show_bar) - mini_base + mini_lines);
    let covering = app.dialogs().iter().rev().find(|(_, d)| !in_minibuffer(d));
    let (body, pages) = match covering {
        Some((id, dialog)) => (
            dialog_body(*id, dialog, memory, width, body_h),
            [body_h.max(1); 2],
        ),
        None => panels_body(app, look, memory, width, body_h),
    };
    let mut lines: Vec<Line> = Vec::new();
    if show_title {
        lines.push(vec![Run::new(
            0,
            width,
            clean(&app.title_line_within(usize::from(width))),
            PanelRole::ActiveTitle,
        )]);
    }
    let body_top = lines.len();
    let mut body_lines = body.lines;
    body_lines.resize(body_h, Vec::new());
    lines.extend(body_lines);
    let mini_top = lines.len();
    if let Some(mini) = &mini {
        lines.extend(mini.lines.iter().cloned());
    }
    if show_bar {
        lines.push(vec![Run::new(0, width, bar, PanelRole::KeyBar)]);
    }
    let status_line = lines.len();
    lines.push(vec![
        Run::new(0, width, status, PanelRole::StatusLine).filled(),
    ]);
    let jobs_role = if app.jobs().is_empty() {
        PanelRole::JobsLine
    } else {
        PanelRole::Progress
    };
    lines.push(vec![Run::new(0, width, jobs, jobs_role).filled()]);
    lines.truncate(height);
    lines.resize(height, Vec::new());
    let parked = (0, status_line.min(height - 1) as u16);
    let cursor = match &mini {
        Some(m) if m.in_list && look.cursor_fallback => parked,
        Some(m) => (m.cursor.0, (mini_top + m.cursor.1).min(height - 1) as u16),
        None if look.cursor_fallback => parked,
        None => match body.focus {
            Some((x, line)) => (x, (body_top + line).min(height - 1) as u16),
            None => parked,
        },
    };
    View {
        width,
        height: height as u16,
        lines,
        cursor,
        page_rows: pages,
    }
}

/// Paints a view into a buffer of its size with the styles.
pub(crate) fn paint(view: &View, styles: &Styles, buffer: &mut Buffer) {
    let area = buffer.area;
    buffer.set_style(area, styles.get(PanelRole::Page));
    for (y, line) in view.lines.iter().enumerate() {
        let y = area.y + y as u16;
        if y >= area.bottom() {
            break;
        }
        for run in line {
            let x = area.x + run.x;
            if x >= area.right() {
                continue;
            }
            let width = run.width.min(area.right() - x);
            let style = styles.get(run.role);
            if run.fill {
                buffer.set_style(Rect::new(x, y, width, 1), style);
            }
            buffer.set_stringn(x, y, &run.text, usize::from(width), style);
        }
    }
}

/// The text of one buffer line as a reader gets it: each cell's symbol,
/// a wide character once, trailing spaces trimmed.
pub(crate) fn line_text(buffer: &Buffer, y: u16) -> String {
    let area = buffer.area;
    let mut text = String::new();
    let mut x = area.x;
    while x < area.right() {
        let symbol = buffer[(x, y)].symbol();
        text.push_str(symbol);
        x += cells(symbol).max(1) as u16;
    }
    text.trim_end().to_owned()
}

/// Whether a buffer line holds a character two cells wide, and the first
/// such cell.
pub(crate) fn first_wide(buffer: &Buffer, y: u16) -> Option<u16> {
    let area = buffer.area;
    (area.x..area.right()).find(|x| cells(buffer[(*x, y)].symbol()) > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_scroll_only_as_far_as_the_focus_needs() {
        assert_eq!(window(0, Some(5), 10, 40), 0);
        assert_eq!(window(0, Some(12), 10, 40), 3);
        assert_eq!(window(3, Some(11), 10, 40), 3);
        assert_eq!(window(3, Some(2), 10, 40), 2);
        assert_eq!(window(30, Some(39), 10, 40), 30);
        assert_eq!(window(35, None, 10, 12), 2);
    }

    #[test]
    fn a_long_prompt_keeps_the_insertion_point_on_the_line() {
        let (line, column) = prompt_line("Go to a path", "D:\\work", 7, 40);
        assert_eq!(line, "Go to a path: D:\\work");
        assert_eq!(column, 21);
        let long = "a".repeat(100);
        let (line, column) = prompt_line("Rename x to", &long, 100, 40);
        assert!(cells(&line) <= 40, "{line}");
        assert!(line.starts_with("Rename x to: "));
        assert!(usize::from(column) < 40);
        let (line, column) = prompt_line("Name", "日本語", 1, 40);
        assert_eq!(line, "Name: 日本語");
        assert_eq!(column, 8);
        let (line, _) = prompt_line("Name", "bad\u{1b}[2Jname", 0, 40);
        assert!(!line.contains('\u{1b}'));
    }

    #[test]
    fn a_selected_row_starts_with_its_asterisk() {
        let mut row = ListRow::new("a", "report.pdf").with_details(["PDF"]);
        assert_eq!(row_text(&row, true), "report.pdf, PDF");
        assert_eq!(row_text(&row, false), "  report.pdf, PDF");
        row.selected = true;
        assert_eq!(row_text(&row, true), "* report.pdf, selected, PDF");
        assert_eq!(row_text(&row, false), "* report.pdf, selected, PDF");
    }
}
