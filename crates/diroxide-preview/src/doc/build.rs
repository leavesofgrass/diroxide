//! Building a [`DocView`]: the readers walk their source and call these,
//! and the whitespace rules are kept here once. Inline text collapses its
//! whitespace to one space; a space or a break waits for the next text,
//! so a row never starts or ends with a space and no empty row is kept;
//! rows written as they are (code, plain lines) keep their text.

use super::{
    CELL_SEPARATOR, CodeBlock, DocKind, DocLink, DocRow, DocTable, DocView, RowKind, SEPARATOR_WORD,
};
use crate::highlight::{Symbol, SymbolKind};

/// The row being written.
#[derive(Debug)]
struct Open {
    kind: RowKind,
    text: String,
    block: usize,
    continued: bool,
}

/// A link waiting for its text.
#[derive(Debug)]
struct OpenLink {
    target: String,
    /// Where its text starts in the open row, once some was written.
    start: Option<usize>,
}

/// A table being written.
#[derive(Debug)]
struct OpenTable {
    first_row: usize,
    block: usize,
    header: Option<usize>,
    cells: Vec<Vec<String>>,
    /// The cells of the row being written, as byte ranges of its text.
    row_cells: Vec<(usize, usize)>,
    /// Whether the row being written is the header row.
    row_header: bool,
    /// Where the open cell's text starts.
    cell_start: Option<usize>,
}

/// Builds a [`DocView`]. See the module documentation.
#[derive(Debug)]
pub struct DocBuilder {
    view: DocView,
    open: Option<Open>,
    /// A space waits for the next text in the open row.
    space: bool,
    /// A line break waits: the next text continues the block in a new row.
    line: bool,
    next_block: usize,
    link: Option<OpenLink>,
    table: Option<OpenTable>,
}

impl DocBuilder {
    /// A builder for a view of a kind.
    pub fn new(kind: DocKind) -> Self {
        Self {
            view: DocView::empty(kind),
            open: None,
            space: false,
            line: false,
            next_block: 0,
            link: None,
            table: None,
        }
    }

    /// Sets the title, from front matter or metadata.
    pub fn set_title(&mut self, title: impl Into<String>) {
        let title = collapse(&title.into());
        if !title.is_empty() {
            self.view.title = Some(title);
        }
    }

    /// Whether a title is set.
    pub fn has_title(&self) -> bool {
        self.view.title.is_some()
    }

    /// Adds a note in the reader's own words, once.
    pub fn note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self.view.notes.contains(&note) {
            self.view.notes.push(note);
        }
    }

    /// Starts a block of a kind: the row written so far ends, and the next
    /// text starts the new block's first row.
    pub fn start(&mut self, kind: RowKind) {
        if self.in_cell() {
            self.space = true;
            return;
        }
        self.end();
        let block = self.take_block();
        self.open = Some(Open {
            kind,
            text: String::new(),
            block,
            continued: false,
        });
    }

    /// Starts a heading of a level, 1 to 6.
    pub fn heading(&mut self, level: u8) {
        self.start(RowKind::Heading(level.clamp(1, 6)));
    }

    /// Starts a paragraph.
    pub fn paragraph(&mut self) {
        self.start(RowKind::Paragraph);
    }

    /// Starts a list item, `depth` 1 for the outermost list, with its
    /// marker in words.
    pub fn list_item(&mut self, depth: u8, marker: Option<String>) {
        self.start(RowKind::ListItem {
            depth: depth.max(1),
            marker,
        });
    }

    /// Sets the marker of the list item being written: a task's
    /// "checked", after an ordered item's number ("1. checked").
    pub fn set_item_marker(&mut self, state: &str) {
        if let Some(Open {
            kind: RowKind::ListItem { marker, .. },
            ..
        }) = self.open.as_mut()
        {
            *marker = Some(match marker.take() {
                Some(number) if number.ends_with('.') => format!("{number} {state}"),
                _ => state.to_owned(),
            });
        }
    }

    /// Whether the row being written has text yet.
    pub fn row_has_text(&self) -> bool {
        self.open.as_ref().is_some_and(|o| !o.text.is_empty())
    }

    /// The kind of the row being written.
    pub fn open_kind(&self) -> Option<&RowKind> {
        self.open.as_ref().map(|o| &o.kind)
    }

    /// Inline text: whitespace runs collapse to one space, dropped at a
    /// row's start. Text with no row open starts a paragraph.
    pub fn text(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() {
                self.space = true;
            } else {
                self.push_char(c);
            }
        }
    }

    /// A space before the next text, unless the row has none yet.
    pub fn space(&mut self) {
        self.space = true;
    }

    /// A line break: the next text continues the block in a new row (a
    /// hard break, `<br>`). In a heading or a table cell it is a space.
    pub fn line_break(&mut self) {
        match self.open.as_ref().map(|o| &o.kind) {
            Some(RowKind::Heading(_) | RowKind::TableRow { .. }) => self.space = true,
            Some(_) if self.row_has_text() => self.line = true,
            _ => {}
        }
    }

    /// Ends the row and the block being written. An empty row is dropped.
    pub fn end(&mut self) {
        self.line = false;
        self.space = false;
        if self.in_cell() {
            return;
        }
        self.finish_row();
    }

    /// Starts a link to `target`; its text is what follows until
    /// [`DocBuilder::end_link`].
    pub fn start_link(&mut self, target: impl Into<String>) {
        self.end_link();
        self.link = Some(OpenLink {
            target: target.into(),
            start: None,
        });
    }

    /// Ends the open link. A link with no text gets its target as text.
    pub fn end_link(&mut self) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        if link.start.is_none() {
            let target = link.target.clone();
            self.link = Some(link);
            self.text(&target);
            match self.link.take() {
                Some(written) => link = written,
                None => return,
            }
        }
        let Some(start) = link.start else {
            return;
        };
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let end = open.text.len();
        if end <= start {
            return;
        }
        let text = open.text[start..end].trim().to_owned();
        if text.is_empty() {
            return;
        }
        let row = self.view.rows.len();
        self.view.links.push(DocLink {
            row,
            range: start..end,
            text,
            target: link.target.trim().to_owned(),
        });
    }

    /// Rows written as they are, one per line: a code block's lines, with
    /// its language as written. Blank lines at its ends are dropped; an
    /// empty block adds nothing.
    pub fn code_block(&mut self, language: Option<&str>, text: &str) {
        if self.in_cell() {
            self.text(text);
            return;
        }
        self.end();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let text = text.trim_matches('\n').trim_end();
        if text.trim().is_empty() {
            return;
        }
        let block = self.take_block();
        let index = self.view.code_blocks.len();
        let first = self.view.rows.len();
        for line in text.split('\n') {
            self.view.rows.push(DocRow {
                kind: RowKind::Code { block: index },
                text: line.trim_end().to_owned(),
                block,
                continued: false,
                runs: Vec::new(),
            });
        }
        let language = language
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned);
        self.view.code_blocks.push(CodeBlock {
            rows: first..self.view.rows.len(),
            language,
        });
    }

    /// One row written as it is, a block of its own: a plain text file's
    /// line, a hex row. An empty line is kept, since a text file's lines
    /// are its own.
    pub fn line(&mut self, kind: RowKind, text: &str) {
        if self.in_cell() {
            self.text(text);
            return;
        }
        self.end();
        let block = self.take_block();
        self.view.rows.push(DocRow {
            kind,
            text: text.to_owned(),
            block,
            continued: false,
            runs: Vec::new(),
        });
    }

    /// A horizontal rule: a row of its own that says "Separator". Two in a
    /// row, or one at the start, are one or none.
    pub fn separator(&mut self) {
        if self.in_cell() {
            return;
        }
        self.end();
        if self
            .view
            .rows
            .last()
            .is_none_or(|r| r.kind == RowKind::Separator)
        {
            return;
        }
        let block = self.take_block();
        self.view.rows.push(DocRow {
            kind: RowKind::Separator,
            text: SEPARATOR_WORD.to_owned(),
            block,
            continued: false,
            runs: Vec::new(),
        });
    }

    /// Starts a table.
    pub fn start_table(&mut self) {
        self.end_table();
        self.end();
        let block = self.take_block();
        self.table = Some(OpenTable {
            first_row: self.view.rows.len(),
            block,
            header: None,
            cells: Vec::new(),
            row_cells: Vec::new(),
            row_header: false,
            cell_start: None,
        });
    }

    /// Starts a table row; the header row when `header` (the first row
    /// only).
    pub fn start_table_row(&mut self, header: bool) {
        let Some(table) = self.table.as_mut() else {
            return;
        };
        if table.cell_start.is_some() || !table.row_cells.is_empty() {
            self.end_table_row();
        }
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let index = self.view.tables.len();
        let row = table.cells.len();
        let block = table.block;
        table.row_header = header && row == 0;
        table.row_cells.clear();
        self.open = Some(Open {
            kind: RowKind::TableRow {
                table: index,
                row,
                header: header && row == 0,
            },
            text: String::new(),
            block,
            continued: false,
        });
        self.space = false;
        self.line = false;
    }

    /// Starts a cell of the row being written.
    pub fn start_cell(&mut self) {
        if self.table.is_none() {
            return;
        }
        if self.open.is_none() {
            self.start_table_row(false);
        }
        self.end_cell();
        let has_cells = self.table.as_ref().is_some_and(|t| !t.row_cells.is_empty());
        if let Some(open) = self.open.as_mut()
            && has_cells
        {
            open.text.push_str(CELL_SEPARATOR);
        }
        let at = self.open.as_ref().map_or(0, |o| o.text.len());
        if let Some(table) = self.table.as_mut() {
            table.cell_start = Some(at);
        }
        self.space = false;
    }

    /// Ends the open cell.
    pub fn end_cell(&mut self) {
        self.end_link();
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let Some(start) = table.cell_start.take() else {
            return;
        };
        let end = self.open.as_ref().map_or(start, |o| o.text.len());
        table.row_cells.push((start, end));
        self.space = false;
    }

    /// Ends the table row being written.
    pub fn end_table_row(&mut self) {
        self.end_cell();
        let Some(table) = self.table.as_mut() else {
            return;
        };
        let Some(open) = self.open.take() else {
            return;
        };
        if table.row_cells.is_empty() {
            return;
        }
        let cells: Vec<String> = table
            .row_cells
            .iter()
            .map(|&(s, e)| open.text[s..e].to_owned())
            .collect();
        table.row_cells.clear();
        if table.row_header {
            table.header = Some(table.cells.len());
        }
        table.cells.push(cells);
        self.view.rows.push(DocRow {
            kind: open.kind,
            text: open.text,
            block: open.block,
            continued: false,
            runs: Vec::new(),
        });
    }

    /// Ends the table. A table with no rows adds nothing.
    pub fn end_table(&mut self) {
        if self.table.is_none() {
            return;
        }
        self.end_table_row();
        let Some(table) = self.table.take() else {
            return;
        };
        if table.cells.is_empty() {
            return;
        }
        self.view.tables.push(DocTable {
            rows: table.first_row..self.view.rows.len(),
            header: table.header,
            cells: table.cells,
        });
    }

    /// The view, every open row, link and table ended.
    pub fn finish(mut self) -> DocView {
        self.end_link();
        self.end_table();
        self.end();
        self.view
    }

    /// Whether a table cell is being written.
    fn in_cell(&self) -> bool {
        self.table.as_ref().is_some_and(|t| t.cell_start.is_some())
    }

    fn take_block(&mut self) -> usize {
        let block = self.next_block;
        self.next_block += 1;
        block
    }

    fn push_char(&mut self, c: char) {
        if self.line {
            self.line = false;
            self.space = false;
            self.continue_row();
        }
        if self.open.is_none() {
            self.paragraph();
        }
        let in_cell_start = self
            .table
            .as_ref()
            .and_then(|t| t.cell_start)
            .zip(self.open.as_ref())
            .is_some_and(|(start, o)| o.text.len() == start);
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if self.space && !open.text.is_empty() && !in_cell_start {
            open.text.push(' ');
        }
        self.space = false;
        if let Some(link) = self.link.as_mut()
            && link.start.is_none()
        {
            link.start = Some(open.text.len());
        }
        open.text.push(c);
    }

    /// Ends the open row and starts the next row of the same block and
    /// kind, marked continued.
    fn continue_row(&mut self) {
        // A link with no text yet carries over whole; one with text ends
        // here and goes on in the new row.
        let carried = match self.link.as_ref() {
            Some(link) if link.start.is_none() => self.link.take(),
            Some(link) => {
                let target = link.target.clone();
                self.end_link();
                Some(OpenLink {
                    target,
                    start: None,
                })
            }
            None => None,
        };
        let Some(open) = self.open.as_ref() else {
            self.link = carried;
            return;
        };
        let (kind, block) = (open.kind.clone(), open.block);
        self.finish_row();
        self.open = Some(Open {
            kind,
            text: String::new(),
            block,
            continued: true,
        });
        self.link = carried;
    }

    fn finish_row(&mut self) {
        self.end_link();
        if self.table.is_some()
            && matches!(
                self.open.as_ref().map(|o| &o.kind),
                Some(RowKind::TableRow { .. })
            )
        {
            self.end_table_row();
            return;
        }
        let Some(open) = self.open.take() else {
            return;
        };
        let text = open.text.trim_end().to_owned();
        if text.is_empty() {
            return;
        }
        let row = self.view.rows.len();
        if let RowKind::Heading(level) = open.kind {
            self.view
                .outline
                .push(Symbol::new(SymbolKind::Heading(level), text.clone(), row));
        }
        self.view.rows.push(DocRow {
            kind: open.kind,
            text,
            block: open.block,
            continued: open.continued,
            runs: Vec::new(),
        });
    }
}

/// Whitespace collapsed to single spaces, trimmed.
pub(crate) fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
