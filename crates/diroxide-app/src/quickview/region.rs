//! The preview region (commit 53): what Tab enters when the quick view is
//! on, the shown document as a list in the list model every other list
//! uses, so the hardware cursor and the Braille line land on the focused
//! row. Every row says its meaning first ("Installation, heading 2"), a
//! continued row "continued: " first, as the viewer's do.
//!
//! The moves, each a plain method returning what to say (research/
//! alpha2-plan.md part 3.6): by line, page, Home and End (the row is
//! read by the screen reader under the cursor, so the words are shown,
//! not said: Low); by heading with its level said ("Heading 2:
//! Installation, 3 of 12"); by paragraph; by table, then by cell with its
//! header said ("Age: 36, row 2 of 5, column 2"); the links as a list
//! ("Link 2 of 5: docs, to docs/index.md") opened through the safe opener
//! rule (web and mail addresses only, asked first, never a file); find
//! with the count said ("Match 2 of 7 in the preview"); the previous and
//! next file without leaving the region (P and N, as in Double
//! Commander).

use std::ops::Range;

use diroxide_core::names::spell_out;
use diroxide_core::{ListModel, ListRow, Message, Move};
use diroxide_preview::doc::{DocView, RowKind};

use super::{QuickView, Showing};
use crate::viewer::CONTINUED;

/// Something the quick view asks the app to do for it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuickViewRequest {
    /// Open a web or mail address with the system's browser or mail
    /// program, after the user said yes.
    OpenAddress(String),
    /// Put the active panel's cursor on this entry, by name: the region
    /// went to the previous or next file.
    FocusEntry(String),
    /// Hand the file to textweaver: the program and its arguments, with
    /// no shell (commit 54).
    RunProgram {
        /// The program found by the tools registry.
        program: std::path::PathBuf,
        /// Its arguments.
        args: Vec<std::ffi::OsString>,
    },
}

/// What a command said, and what it asks the app to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// What to say, in order.
    pub said: Vec<Message>,
    /// What the app does next, if anything.
    pub request: Option<QuickViewRequest>,
}

impl From<Vec<Message>> for Outcome {
    fn from(said: Vec<Message>) -> Self {
        Self {
            said,
            request: None,
        }
    }
}

/// Where a link may go from the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A web address: `http://` or `https://`.
    Web(String),
    /// A mail address: `mailto:`.
    Mail(String),
    /// Anything else, refused: a file, a relative path, `file:`,
    /// `javascript:`, `data:`.
    Refused(String),
}

impl LinkTarget {
    /// The safe opener rule: web and mail addresses only, whole and
    /// plain (no spaces, no hidden characters), never a file.
    pub fn of(target: &str) -> Self {
        let plain = !target.is_empty()
            && !target
                .chars()
                .any(|c| c.is_whitespace() || diroxide_core::names::classify(c).is_hidden());
        let lower = target.to_ascii_lowercase();
        let web = ["https://", "http://"]
            .iter()
            .any(|scheme| lower.starts_with(scheme) && target.len() > scheme.len());
        let mail = lower.starts_with("mailto:") && target.len() > "mailto:".len();
        match (plain, web, mail) {
            (true, true, _) => LinkTarget::Web(target.to_owned()),
            (true, _, true) => LinkTarget::Mail(target.to_owned()),
            _ => LinkTarget::Refused(target.to_owned()),
        }
    }
}

/// One row of the region: a row of the document, or a part of one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    row: usize,
    part: usize,
    range: Range<usize>,
}

/// The region's state over the shown document.
#[derive(Clone, Debug)]
pub(super) struct Region {
    list: ListModel,
    parts: Vec<Part>,
    /// The cell the cell moves are on: table, row in it, column.
    cell: Option<(usize, usize, usize)>,
    /// The last find's matches, as region rows, and the one landed on.
    found: Vec<usize>,
    found_at: Option<usize>,
    /// A link asked about, waiting for y or n.
    asking: Option<String>,
    /// The link the last link move landed on.
    link_at: Option<usize>,
    width: usize,
}

impl Default for Region {
    fn default() -> Self {
        let mut list = ListModel::new("Preview");
        list.set_empty_text("Nothing shown");
        Self {
            list,
            parts: Vec::new(),
            cell: None,
            found: Vec::new(),
            found_at: None,
            asking: None,
            link_at: None,
            width: 0,
        }
    }
}

/// The cells a row of text gets at least when wrapped.
const MIN_ROOM: usize = 8;

/// The byte ranges `text` wraps into at `width` characters, after a
/// space where one fits; a word longer than a row is cut. An empty text
/// is one empty row; a width of 0 does not wrap.
fn wrap(text: &str, width: usize) -> Vec<Range<usize>> {
    if width == 0 || text.chars().count() <= width {
        return std::iter::once(0..text.len()).collect();
    }
    let room = width.max(MIN_ROOM);
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0;
    let mut after_space = None;
    for (at, c) in text.char_indices() {
        if used == room {
            let cut = after_space.filter(|&s| s > start).unwrap_or(at);
            rows.push(start..cut);
            start = cut;
            used = text[cut..at].chars().count();
            after_space = None;
        }
        used += 1;
        if c == ' ' {
            after_space = Some(at + 1);
        }
    }
    rows.push(start..text.len());
    rows
}

/// A row's label split at `width`: the first part at the full width, the
/// rest at the width less the "continued: " cue they start with.
fn split(label: &str, width: usize) -> Vec<Range<usize>> {
    let first = wrap(label, width);
    if first.len() <= 1 {
        return first;
    }
    let head = first[0].clone();
    let after = &label[head.end..];
    let tail = head.end + (after.len() - after.trim_start().len());
    let room = width
        .saturating_sub(CONTINUED.chars().count())
        .max(MIN_ROOM);
    let mut out = vec![head];
    out.extend(
        wrap(&label[tail..], room)
            .into_iter()
            .map(|r| tail + r.start..tail + r.end),
    );
    out
}

impl Region {
    /// The region over a view, `width` characters wide (0: no wrap), the
    /// focus on a document row.
    fn new(view: &DocView, line_numbers: bool, width: usize, focus_row: usize) -> Self {
        let mut parts = Vec::new();
        let mut rows = Vec::new();
        for n in 0..view.len() {
            let label = view.label(n, line_numbers);
            for (part, range) in split(&label, width).into_iter().enumerate() {
                let text = &label[range.clone()];
                let row = if part == 0 {
                    ListRow::new(n.to_string(), text.trim_end()).with_details(view.row_details(n))
                } else {
                    ListRow::new(
                        format!("{n}.{part}"),
                        format!("{CONTINUED}{}", text.trim_end()),
                    )
                };
                rows.push(row);
                parts.push(Part {
                    row: n,
                    part,
                    range,
                });
            }
        }
        let mut list = ListModel::new("Preview").with_rows(rows);
        list.set_empty_text("Empty preview");
        list.set_help(
            "Up and Down move by line; H and Shift+H by heading; P and N to the previous and next \
             file; T by table, then the arrows by cell; L lists the links; slash finds; Escape \
             returns to the panel.",
        );
        let mut region = Self {
            list,
            parts,
            width,
            ..Self::default()
        };
        region.focus_row(focus_row);
        region
    }

    /// The region row of a document row's first part.
    fn first_part(&self, row: usize) -> Option<usize> {
        self.parts.iter().position(|p| p.row == row)
    }

    /// The document row the focus is on.
    fn row(&self) -> Option<usize> {
        let at = self.list.focus_index()?;
        self.parts.get(at).map(|p| p.row)
    }

    fn focus_row(&mut self, row: usize) -> bool {
        match self.first_part(row) {
            Some(at) => {
                self.list.move_focus(Move::To(at));
                true
            }
            None => false,
        }
    }
}

/// The edge words when a move cannot go on.
fn edge(down: bool) -> Message {
    Message::status(if down {
        "Bottom of the preview"
    } else {
        "Top of the preview"
    })
}

impl QuickView {
    /// Builds the region over what is shown: the focus on a document row,
    /// kept when the same file renders again.
    pub(super) fn rebuild_region(&mut self, keep_row: bool) {
        let focus = if keep_row {
            self.region.row().unwrap_or(0)
        } else {
            0
        };
        let width = self.region.width;
        self.region = match self.view() {
            Some(view) => Region::new(
                view,
                self.options.settings.line_numbers,
                if self.options.viewer.wrap { width } else { 0 },
                focus.min(view.len().saturating_sub(1)),
            ),
            None => Region {
                width,
                ..Region::default()
            },
        };
    }

    /// The region as a list, for the frontend to draw: the focused row is
    /// where the hardware cursor goes, at its first cell.
    pub fn region(&self) -> &ListModel {
        &self.region.list
    }

    /// The columns the region is drawn in: rows wrap at this width when
    /// the viewer's `wrap` setting is on (0: no wrap).
    pub fn set_width(&mut self, width: usize) {
        if self.region.width != width {
            self.region.width = width;
            self.rebuild_region(true);
        }
    }

    /// The region's title, the file's name after "Preview": "Preview,
    /// quarter.md".
    pub fn region_title(&self) -> String {
        match &self.shown {
            Some(shown) => format!("Preview, {}", shown.name),
            None => "Preview".to_owned(),
        }
    }

    /// Tab entered the region: its title, then the row under the cursor
    /// with its place last ("Quarter, heading 1, 1 of 48"), or why there
    /// is nothing to read.
    pub fn enter(&mut self) -> Vec<Message> {
        let mut said = vec![Message::info(self.region_title())];
        match self.shown.as_ref().map(|s| &s.showing) {
            Some(Showing::Document(_)) => {
                said.push(Message::info(self.region.list.announce_focus()))
            }
            Some(Showing::NoPreview(words)) => {
                let name = self.shown.as_ref().map_or("", |s| s.name.as_str());
                said.push(Message::info(format!("No preview: {name}, {words}")));
            }
            Some(Showing::Unreadable(text)) => said.push(Message::info(text.clone())),
            None if self.on => said.push(Message::info("Nothing shown yet")),
            None => said.push(Message::info("Quick view off")),
        }
        said
    }

    /// Say Status in the region: the file and the line, with the reader's
    /// notes after.
    pub fn region_status(&self) -> Vec<Message> {
        let Some(shown) = self.shown.as_ref() else {
            return vec![Message::info(self.region_title())];
        };
        let mut said = vec![Message::info(format!(
            "Preview, {}, {}",
            shown.name,
            self.region.list.position_text()
        ))];
        if let Some(view) = self.view() {
            said.extend(view.notes().iter().map(Message::info));
        }
        said
    }

    fn said_focus(&self) -> Vec<Message> {
        vec![Message::status(self.region.list.announce_focus())]
    }

    /// Moves by line or page, or to the top or bottom (Up, Down, Page Up,
    /// Page Down, Home, End): the row is shown, the edge said.
    pub fn region_move(&mut self, movement: Move) -> Vec<Message> {
        self.region.cell = None;
        if self.region.list.move_focus(movement) {
            return self.said_focus();
        }
        vec![edge(!matches!(
            movement,
            Move::Up | Move::PageUp | Move::Home
        ))]
    }

    /// The next heading below the cursor, its level said first: "Heading
    /// 2: Installation, 3 of 12".
    pub fn next_heading(&mut self) -> Vec<Message> {
        self.heading(true)
    }

    /// The previous heading above the cursor.
    pub fn previous_heading(&mut self) -> Vec<Message> {
        self.heading(false)
    }

    fn heading(&mut self, down: bool) -> Vec<Message> {
        let Some(view) = self.view() else {
            return vec![edge(down)];
        };
        let here = self.region.row().unwrap_or(0);
        let found = if down {
            view.next_heading(here)
        } else {
            view.previous_heading(here)
        };
        let Some(index) = found else {
            return vec![Message::status(if down {
                "No heading below"
            } else {
                "No heading above"
            })];
        };
        let row = view.outline()[index].line;
        let words = view.heading_words(index).unwrap_or_default();
        self.region.cell = None;
        self.region.focus_row(row);
        vec![Message::info(words)]
    }

    /// The next paragraph (block) below the cursor: a table or a code
    /// block is one.
    pub fn next_paragraph(&mut self) -> Vec<Message> {
        let Some(view) = self.view() else {
            return vec![edge(true)];
        };
        let here = self.region.row().unwrap_or(0);
        match view.next_block(here) {
            Some(row) => {
                self.region.cell = None;
                self.region.focus_row(row);
                self.said_focus()
            }
            None => vec![edge(true)],
        }
    }

    /// The paragraph above the cursor's, or the start of its own.
    pub fn previous_paragraph(&mut self) -> Vec<Message> {
        let Some(view) = self.view() else {
            return vec![edge(false)];
        };
        let here = self.region.row().unwrap_or(0);
        let on_part = self
            .region
            .list
            .focus_index()
            .and_then(|at| self.region.parts.get(at));
        let start = view.block_start(here);
        let target = if start < here || on_part.is_some_and(|p| p.part > 0) {
            Some(start)
        } else {
            view.previous_block(here)
        };
        match target {
            Some(row) => {
                self.region.cell = None;
                self.region.focus_row(row);
                self.said_focus()
            }
            None => vec![edge(false)],
        }
    }

    /// The next table below the cursor: "Table 1 of 2, 5 rows, 3
    /// columns", the cell moves starting on its first cell.
    pub fn next_table(&mut self) -> Vec<Message> {
        self.table(true)
    }

    /// The previous table above the cursor.
    pub fn previous_table(&mut self) -> Vec<Message> {
        self.table(false)
    }

    fn table(&mut self, down: bool) -> Vec<Message> {
        let Some(view) = self.view() else {
            return vec![edge(down)];
        };
        let here = self.region.row().unwrap_or(0);
        let found = if down {
            view.next_table(here)
        } else {
            view.previous_table(here)
        };
        let Some(index) = found else {
            return vec![Message::status(if down {
                "No table below"
            } else {
                "No table above"
            })];
        };
        let row = view.tables()[index].rows.start;
        let words = view.table_words(index).unwrap_or_default();
        self.region.focus_row(row);
        self.region.cell = Some((index, 0, 0));
        vec![Message::info(words)]
    }

    /// The cell to the right in the table row under the cursor, said with
    /// its header: "Age: 36, row 2 of 5, column 2".
    pub fn cell_right(&mut self) -> Vec<Message> {
        self.cell_move(0, 1)
    }

    /// The cell to the left.
    pub fn cell_left(&mut self) -> Vec<Message> {
        self.cell_move(0, -1)
    }

    /// The cell below, in the next row of the table.
    pub fn cell_down(&mut self) -> Vec<Message> {
        self.cell_move(1, 0)
    }

    /// The cell above.
    pub fn cell_up(&mut self) -> Vec<Message> {
        self.cell_move(-1, 0)
    }

    fn cell_move(&mut self, rows: isize, columns: isize) -> Vec<Message> {
        let Some(view) = self.view() else {
            return vec![Message::status("Not in a table")];
        };
        let here = self.region.row().unwrap_or(0);
        let Some(RowKind::TableRow { table, row, .. }) = view.row(here).map(|r| r.kind.clone())
        else {
            return vec![Message::status("Not in a table")];
        };
        let grid = &view.tables()[table];
        let column = match self.region.cell {
            Some((t, r, c)) if t == table && r == row => c,
            _ => 0,
        };
        let new_row = row
            .checked_add_signed(rows)
            .filter(|&r| r < grid.row_count());
        let width = grid.cells.get(new_row.unwrap_or(row)).map_or(0, Vec::len);
        let new_column = column.checked_add_signed(columns).filter(|&c| c < width);
        let (Some(new_row), Some(new_column)) = (new_row, new_column) else {
            return vec![Message::status(match (rows, columns) {
                (0, 1) => "End of the row",
                (0, _) => "Start of the row",
                (1, _) => "Last row of the table",
                _ => "First row of the table",
            })];
        };
        let words = grid.cell_words(new_row, new_column);
        let doc_row = grid.rows.start + new_row;
        self.region.focus_row(doc_row);
        self.region.cell = Some((table, new_row, new_column));
        vec![Message::info(words)]
    }

    /// The links as a list, titled "Links: 5 in quarter.md", one row a
    /// link, its text first and its target after; each row's id is its
    /// index, for [`QuickView::link_go`] and [`QuickView::open_link`].
    pub fn links(&self) -> ListModel {
        let (links, name) = match (self.view(), self.shown.as_ref()) {
            (Some(view), Some(shown)) => (view.links(), shown.name.as_str()),
            _ => (&[][..], ""),
        };
        let title = format!("Links: {} in {name}", links.len());
        let rows = links
            .iter()
            .enumerate()
            .map(|(n, l)| {
                let row = ListRow::new(n.to_string(), l.text.clone());
                if l.text == l.target {
                    row
                } else {
                    row.with_details([format!("to {}", spell_out(&l.target))])
                }
            })
            .collect();
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_empty_text("No links");
        list.set_help("Enter goes to the link; O opens a web or mail address, asked first.");
        list
    }

    /// Goes to a link's row, saying it: "Link 2 of 5: docs, to
    /// docs/index.md".
    pub fn link_go(&mut self, id: &str) -> Vec<Message> {
        let Some(view) = self.view() else {
            return Vec::new();
        };
        let Some((index, link)) = id
            .parse::<usize>()
            .ok()
            .and_then(|n| view.links().get(n).map(|l| (n, l.clone())))
        else {
            return Vec::new();
        };
        let words = link.words(index, view.links().len());
        self.region.focus_row(link.row);
        self.region.link_at = Some(index);
        vec![Message::info(spell_out(&words))]
    }

    /// The next link after the one last landed on, or the first at or
    /// below the cursor's row, said.
    pub fn next_link(&mut self) -> Vec<Message> {
        self.step_link(true)
    }

    /// The previous link before the one last landed on, or the last above
    /// the cursor's row.
    pub fn previous_link(&mut self) -> Vec<Message> {
        self.step_link(false)
    }

    fn step_link(&mut self, down: bool) -> Vec<Message> {
        let Some(view) = self.view() else {
            return vec![Message::status("No links")];
        };
        let here = self.region.row().unwrap_or(0);
        let links = view.links();
        let landed = self
            .region
            .link_at
            .filter(|&n| links.get(n).is_some_and(|l| l.row == here));
        let found = match (landed, down) {
            (Some(n), true) => (n + 1 < links.len()).then_some(n + 1),
            (Some(n), false) => n.checked_sub(1),
            (None, true) => links.iter().position(|l| l.row >= here),
            (None, false) => links.iter().rposition(|l| l.row < here),
        };
        match found {
            Some(n) => self.link_go(&n.to_string()),
            None => vec![Message::status(if down {
                "No link below"
            } else {
                "No link above"
            })],
        }
    }

    /// Asks before opening a link: "Open `https://example.org` in the
    /// browser? y or n", or "Open mail to ada@example.org? y or n". A
    /// link to a file, a relative path or any other address is refused,
    /// never opened: "Could not open docs/index.md: only web and mail
    /// links open from the preview."
    pub fn open_link(&mut self, id: &str) -> Vec<Message> {
        let Some(link) = self.view().and_then(|view| {
            id.parse::<usize>()
                .ok()
                .and_then(|n| view.links().get(n).cloned())
        }) else {
            return Vec::new();
        };
        match LinkTarget::of(&link.target) {
            LinkTarget::Web(url) => {
                self.region.asking = Some(url.clone());
                vec![Message::question(&format!(
                    "Open {} in the browser",
                    spell_out(&url)
                ))]
            }
            LinkTarget::Mail(url) => {
                self.region.asking = Some(url.clone());
                let to = &url["mailto:".len()..];
                vec![Message::question(&format!(
                    "Open mail to {}",
                    spell_out(to)
                ))]
            }
            LinkTarget::Refused(target) => vec![Message::could_not(
                "open",
                &spell_out(&target),
                "only web and mail links open from the preview",
                "",
            )],
        }
    }

    /// The answer to the link question: yes asks the app to open the
    /// address; no, or anything else, opens nothing ("Not opened").
    pub fn answer_link(&mut self, yes: bool) -> Outcome {
        match self.region.asking.take() {
            Some(url) if yes => Outcome {
                said: Vec::new(),
                request: Some(QuickViewRequest::OpenAddress(url)),
            },
            Some(_) => vec![Message::status("Not opened")].into(),
            None => Outcome::default(),
        }
    }

    /// Whether a link question waits for its answer.
    pub fn is_asking(&self) -> bool {
        self.region.asking.is_some()
    }

    /// Finds text in the preview, ignoring case, from the cursor on:
    /// "Match 2 of 7 in the preview", or "No match for zebra in the
    /// preview".
    pub fn find(&mut self, text: &str) -> Vec<Message> {
        let Some(view) = self.view() else {
            return Vec::new();
        };
        let matches = view.find(text);
        let rows: Vec<usize> = matches
            .iter()
            .filter_map(|m| self.part_of(m.row, m.start))
            .collect();
        self.region.found = rows;
        self.region.found_at = None;
        if self.region.found.is_empty() {
            return vec![Message::info(format!(
                "No match for {} in the preview",
                spell_out(text)
            ))];
        }
        let here = self.region.list.focus_index().unwrap_or(0);
        let index = self
            .region
            .found
            .iter()
            .position(|&r| r >= here)
            .unwrap_or(0);
        self.land_match(index, false)
    }

    /// The next match after the one landed on, wrapping to the top.
    pub fn next_match(&mut self) -> Vec<Message> {
        self.step_match(true)
    }

    /// The match before, wrapping to the bottom.
    pub fn previous_match(&mut self) -> Vec<Message> {
        self.step_match(false)
    }

    fn step_match(&mut self, down: bool) -> Vec<Message> {
        let total = self.region.found.len();
        if total == 0 {
            return vec![Message::status("Nothing found yet")];
        }
        let at = self.region.found_at.unwrap_or(0);
        let (index, wrapped) = if down {
            if at + 1 < total {
                (at + 1, false)
            } else {
                (0, true)
            }
        } else if at > 0 {
            (at - 1, false)
        } else {
            (total - 1, true)
        };
        self.land_match(index, wrapped)
    }

    fn land_match(&mut self, index: usize, wrapped: bool) -> Vec<Message> {
        let total = self.region.found.len();
        let row = self.region.found[index];
        self.region.found_at = Some(index);
        self.region.list.move_focus(Move::To(row));
        let words = format!("Match {} of {total} in the preview", index + 1);
        vec![Message::info(if wrapped {
            format!("{words}, wrapped")
        } else {
            words
        })]
    }

    /// The region row that holds a byte of a document row's shown text.
    fn part_of(&self, row: usize, byte: usize) -> Option<usize> {
        let line_numbers =
            self.options.settings.line_numbers && self.view().is_some_and(DocView::is_line_based);
        let offset = if line_numbers {
            format!("{}: ", diroxide_core::format::grouped(row as u64 + 1)).len()
        } else {
            0
        };
        let at = byte + offset;
        self.region
            .parts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.row == row)
            .find(|(_, p)| at < p.range.end || p.range.is_empty())
            .or_else(|| {
                self.region
                    .parts
                    .iter()
                    .enumerate()
                    .rfind(|(_, p)| p.row == row)
            })
            .map(|(n, _)| n)
    }

    /// The entries the region's previous and next file step through, in
    /// the panel's order: the panel's files, set when the quick view is
    /// entered and when the panel's list changes.
    pub fn set_siblings(&mut self, siblings: Vec<(diroxide_core::Place, diroxide_core::Entry)>) {
        self.siblings = siblings
            .into_iter()
            .filter(|(_, e)| e.kind == diroxide_core::EntryKind::File)
            .collect();
    }

    /// The next file in the panel's order (N in the region), shown at
    /// once, without the wait: "report-2.md, 2 of 4", and the app moves
    /// the panel's cursor there. Its preview line follows when ready.
    pub fn next_file(&mut self, now: std::time::Instant) -> Outcome {
        self.step_file(true, now)
    }

    /// The previous file (P in the region).
    pub fn previous_file(&mut self, now: std::time::Instant) -> Outcome {
        self.step_file(false, now)
    }

    fn step_file(&mut self, forward: bool, now: std::time::Instant) -> Outcome {
        let total = self.siblings.len();
        let here = self
            .shown
            .as_ref()
            .map(|s| &s.place)
            .and_then(|place| self.siblings.iter().position(|(p, _)| p == place));
        let next = match (here, forward) {
            (Some(at), true) if at + 1 < total => at + 1,
            (Some(at), false) if at > 0 => at - 1,
            (None, _) if total > 0 => 0,
            _ => {
                return vec![Message::status(if forward {
                    "No next file"
                } else {
                    "No previous file"
                })]
                .into();
            }
        };
        let (place, entry) = self.siblings[next].clone();
        let name = diroxide_core::names::display_name(&entry.name).into_owned();
        let mut said = vec![Message::info(format!("{name}, {} of {total}", next + 1))];
        self.show_now(&place, &entry, now);
        said.extend(self.tick(now));
        Outcome {
            said,
            request: Some(QuickViewRequest::FocusEntry(entry.name.clone())),
        }
    }
}
