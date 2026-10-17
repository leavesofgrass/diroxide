//! The quick view's document: a slim, row-based view of a file, read by a
//! screen reader and a Braille line one row at a time, in the list model
//! every other list uses (research/alpha2-plan.md part 1.7, plan section
//! 5.9). Not textweaver's document model: rows, an outline of headings
//! with their levels, tables as row ranges with a header row and their
//! cells, links with their targets, and code blocks with their
//! languages, which is all the quick view moves by.
//!
//! What it holds:
//!
//! - **Rows** ([`DocRow`]), each one line of meaning: a heading, a
//!   paragraph (or one line of it after a hard break), a list item with
//!   its marker in words, a table row with its cells joined by `" | "`, a
//!   line of code, a line of a quote, a separator. Each row belongs to a
//!   block ([`DocRow::block`]), the unit a paragraph move skips by: a
//!   paragraph, a heading, a list item, a whole table, a whole code block.
//! - **The outline** ([`DocView::outline`]): the headings as the
//!   highlight module's [`Symbol`]s, so the viewer's outline (commit 45)
//!   and the quick view's share one index; a heading symbol's line is its
//!   row here.
//! - **Tables** ([`DocTable`]), **links** ([`DocLink`]) and **code
//!   blocks** ([`CodeBlock`]), each pointing at its rows.
//!
//! The whitespace rules, textweaver's builder's (relicensed under answer
//! 41, the rules and not the code): inline whitespace collapses to one
//! space and is dropped at a row's start and end; breaks wait for the next
//! text, so no row is empty or doubled; code and plain text keep their
//! lines as written. Readers build a view through [`DocBuilder`].
//!
//! Every word a move says is made here, meaning first and inside 40 cells
//! where the names allow: "Heading 2: Installation, 3 of 12", "Table 1 of
//! 2, 5 rows, 3 columns", "Age: 36, row 2 of 5, column 2", "Link 2 of 5:
//! docs, to docs/index.md".

mod build;
pub mod callout;
pub mod markdown;
#[cfg(test)]
mod tests;

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::classes::ClassRun;
use crate::highlight::{Language, Symbol, SymbolKind};

pub use build::DocBuilder;

/// What the words between a table row's cells are.
pub const CELL_SEPARATOR: &str = " | ";

/// What a separator row (a horizontal rule) says.
pub const SEPARATOR_WORD: &str = "Separator";

/// What a document was read from, for the one-line announcement.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocKind {
    /// Built by hand or by a reader not named here.
    Document,
    /// Markdown.
    Markdown,
    /// A format carta reads, by its name as said: "Org",
    /// "reStructuredText".
    Carta(String),
    /// Code in a language on the highlight allow-list.
    Code(Language),
    /// Plain text.
    Text,
    /// Bytes shown as hex.
    Hex,
}

impl DocKind {
    /// The kind in words, for the announcement: "Org", "Rust"; `None` for
    /// Markdown, plain text and a hand-built document, which say their
    /// counts alone.
    pub fn word(&self) -> Option<&str> {
        match self {
            DocKind::Carta(name) => Some(name),
            DocKind::Code(language) => Some(language.name()),
            DocKind::Hex => Some("binary"),
            DocKind::Document | DocKind::Markdown | DocKind::Text => None,
        }
    }
}

/// What a row is.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RowKind {
    /// A heading, its level 1 to 6.
    Heading(u8),
    /// A paragraph, or one line of one after a hard break.
    Paragraph,
    /// A list item, `depth` 1 for the outermost list, with its marker in
    /// words: "1." for an ordered item, "checked" or "not checked" for a
    /// task; `None` for a plain bullet.
    ListItem {
        /// How deep the list is, 1 for the outermost.
        depth: u8,
        /// The marker in words.
        marker: Option<String>,
    },
    /// A row of a table: the table's index in [`DocView::tables`] and the
    /// row's index in the table.
    TableRow {
        /// The table.
        table: usize,
        /// The row within the table, 0 for the first.
        row: usize,
        /// Whether it is the header row.
        header: bool,
    },
    /// A line of a code block: the block's index in
    /// [`DocView::code_blocks`].
    Code {
        /// The code block.
        block: usize,
    },
    /// A line of a quote, with its label when it is a callout or an alert
    /// ("tip", "note"), whose type its text already says first.
    Quote {
        /// The callout's or alert's type as written, lowercase.
        label: Option<String>,
    },
    /// A line of a plain text file, as written.
    Line,
    /// A row of hex.
    Hex,
    /// A horizontal rule, said "Separator".
    Separator,
    /// A note the reader added in its own words: an embed or an include
    /// left out, the footnotes' heading.
    Note,
}

/// One row: one line of meaning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocRow {
    /// What it is.
    pub kind: RowKind,
    /// Its text, without a list marker; a table row's cells joined by
    /// [`CELL_SEPARATOR`].
    pub text: String,
    /// The block it belongs to, the unit a paragraph move skips by.
    pub block: usize,
    /// Whether it continues the row above it in the same block (after a
    /// hard break, a list item's second paragraph), so its marker is not
    /// said again.
    pub continued: bool,
    /// Its token class runs, for code that was highlighted; empty
    /// otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<ClassRun>,
}

impl DocRow {
    /// A row of a kind with its text, in block 0, not continued, with no
    /// runs.
    pub fn new(kind: RowKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            block: 0,
            continued: false,
            runs: Vec::new(),
        }
    }

    /// The row as shown and read, meaning first: a list item's marker in
    /// words before its text ("1. first", "Checked: Buy milk"), anything
    /// else its text.
    pub fn shown(&self) -> String {
        match &self.kind {
            RowKind::ListItem {
                marker: Some(marker),
                ..
            } if !self.continued => {
                // A task's state in words first, after an ordered item's
                // number: "2. Checked: Walk the dog".
                for (state, word) in [("not checked", "Not checked"), ("checked", "Checked")] {
                    if let Some(number) = marker.strip_suffix(state) {
                        let number = number.trim();
                        return if number.is_empty() {
                            format!("{word}: {}", self.text)
                        } else {
                            format!("{number} {word}: {}", self.text)
                        };
                    }
                }
                format!("{marker} {}", self.text)
            }
            _ => self.text.clone(),
        }
    }

    /// What is said after the row, in words: "heading 2", "list item,
    /// level 2", "header row", "quote"; empty for a paragraph, a line or a
    /// continued row. The first row of a code block is said by
    /// [`DocView::row_details`], which knows the language.
    pub fn details(&self) -> Vec<String> {
        if self.continued {
            return Vec::new();
        }
        match &self.kind {
            RowKind::Heading(level) => vec![format!("heading {level}")],
            RowKind::ListItem { depth, .. } if *depth > 1 => {
                vec!["list item".to_owned(), format!("level {depth}")]
            }
            RowKind::ListItem { .. } => vec!["list item".to_owned()],
            RowKind::TableRow { header: true, .. } => vec!["header row".to_owned()],
            RowKind::Quote { label: None } => vec!["quote".to_owned()],
            _ => Vec::new(),
        }
    }
}

/// A table: its rows in the view and its cells.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocTable {
    /// Its rows in [`DocView::rows`].
    pub rows: Range<usize>,
    /// The header row's index in the table, when it has one (the first
    /// row, as the readers make tables).
    pub header: Option<usize>,
    /// Each row's cells, as written, trimmed.
    pub cells: Vec<Vec<String>>,
}

impl DocTable {
    /// How many rows it has, the header row among them.
    pub fn row_count(&self) -> usize {
        self.cells.len()
    }

    /// How many columns its widest row has.
    pub fn column_count(&self) -> usize {
        self.cells.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// A cell's text: `None` past the row's end.
    pub fn cell(&self, row: usize, column: usize) -> Option<&str> {
        self.cells.get(row)?.get(column).map(String::as_str)
    }

    /// A column's header, when the table has a header row and the cell is
    /// not empty.
    pub fn header_of(&self, column: usize) -> Option<&str> {
        self.cell(self.header?, column).filter(|t| !t.is_empty())
    }

    /// Where a cell's text is in its row's text, in bytes.
    pub fn cell_range(&self, row: usize, column: usize) -> Option<Range<usize>> {
        let cells = self.cells.get(row)?;
        let mut start = 0;
        for (n, cell) in cells.iter().enumerate() {
            if n == column {
                return Some(start..start + cell.len());
            }
            start += cell.len() + CELL_SEPARATOR.len();
        }
        None
    }

    /// A cell said with its header first, its position last: "Age: 36,
    /// row 2 of 5, column 2"; a header cell "Age, header, row 1 of 5,
    /// column 2"; an empty cell "Age: blank, ...". Rows and columns are
    /// said from 1, the header row counted.
    pub fn cell_words(&self, row: usize, column: usize) -> String {
        let text = self.cell(row, column).unwrap_or("");
        let text = if text.is_empty() { "blank" } else { text };
        let at = format!(
            "row {} of {}, column {}",
            row + 1,
            self.row_count(),
            column + 1
        );
        if self.header == Some(row) {
            return format!("{text}, header, {at}");
        }
        match self.header_of(column) {
            Some(header) => format!("{header}: {text}, {at}"),
            None => format!("{text}, {at}"),
        }
    }
}

/// A link: its text, its target and where it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocLink {
    /// The row it is on.
    pub row: usize,
    /// Where its text is in the row's text, in bytes.
    pub range: Range<usize>,
    /// Its text as shown; the target when it has no text of its own.
    pub text: String,
    /// Where it points, as written.
    pub target: String,
}

impl DocLink {
    /// The link said in a list of `total`, its text first and its target
    /// last: "Link 2 of 5: docs, to docs/index.md". `index` is 0-based.
    pub fn words(&self, index: usize, total: usize) -> String {
        if self.text == self.target {
            format!("Link {} of {total}: {}", index + 1, self.text)
        } else {
            format!(
                "Link {} of {total}: {}, to {}",
                index + 1,
                self.text,
                self.target
            )
        }
    }
}

/// A code block: its rows and its language as written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBlock {
    /// Its rows in [`DocView::rows`].
    pub rows: Range<usize>,
    /// The language its fence names, as written ("rust", "sh").
    pub language: Option<String>,
}

impl CodeBlock {
    /// The language as said: the highlighter's name for one it knows
    /// ("Rust"), else as written.
    pub fn language_words(&self) -> Option<String> {
        let written = self.language.as_deref()?;
        Some(fence_language(written).map_or_else(|| written.to_owned(), |l| l.name().to_owned()))
    }
}

/// The language a code fence names, by the highlighter's name ("rust",
/// "shell") or a file extension ("rs", "sh", "py").
pub fn fence_language(written: &str) -> Option<Language> {
    let written = written.trim();
    Language::ALL
        .into_iter()
        .find(|l| l.name().eq_ignore_ascii_case(written))
        .or_else(|| Language::by_name(&format!("block.{}", written.to_ascii_lowercase())))
}

/// A match of a find: where it starts and how long it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocMatch {
    /// The row.
    pub row: usize,
    /// Where it is in the row's text, in bytes.
    pub start: usize,
    /// Its length in bytes.
    pub len: usize,
}

/// A document as rows. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocView {
    /// What it was read from.
    pub kind: DocKind,
    /// Its title, from front matter or metadata, when it has one.
    pub title: Option<String>,
    rows: Vec<DocRow>,
    outline: Vec<Symbol>,
    tables: Vec<DocTable>,
    links: Vec<DocLink>,
    code_blocks: Vec<CodeBlock>,
    notes: Vec<String>,
}

impl Default for DocView {
    fn default() -> Self {
        Self::empty(DocKind::Document)
    }
}

impl DocView {
    /// An empty view of a kind.
    pub fn empty(kind: DocKind) -> Self {
        Self {
            kind,
            title: None,
            rows: Vec::new(),
            outline: Vec::new(),
            tables: Vec::new(),
            links: Vec::new(),
            code_blocks: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// A builder for a view of a kind.
    pub fn builder(kind: DocKind) -> DocBuilder {
        DocBuilder::new(kind)
    }

    /// The rows, in order.
    pub fn rows(&self) -> &[DocRow] {
        &self.rows
    }

    /// One row.
    pub fn row(&self, row: usize) -> Option<&DocRow> {
        self.rows.get(row)
    }

    /// How many rows there are.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The headings, as symbols whose line is their row: the viewer's
    /// outline and the quick view's share this index.
    pub fn outline(&self) -> &[Symbol] {
        &self.outline
    }

    /// The tables, in order.
    pub fn tables(&self) -> &[DocTable] {
        &self.tables
    }

    /// The links, in order.
    pub fn links(&self) -> &[DocLink] {
        &self.links
    }

    /// The code blocks, in order.
    pub fn code_blocks(&self) -> &[CodeBlock] {
        &self.code_blocks
    }

    /// What the reader said about the file in its own words, once each:
    /// "An include was left out, so the file it names is not read."
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The same view with highlight runs on its rows: one list per row,
    /// from the highlighter. Lists past the last row are dropped.
    pub fn with_runs(mut self, runs: Vec<Vec<ClassRun>>) -> Self {
        for (row, runs) in self.rows.iter_mut().zip(runs) {
            row.runs = runs;
        }
        self
    }

    /// What is said after a row: [`DocRow::details`], and for the first
    /// row of a code block "code" with its language ("code, Rust").
    pub fn row_details(&self, row: usize) -> Vec<String> {
        let Some(r) = self.rows.get(row) else {
            return Vec::new();
        };
        if let RowKind::Code { block } = r.kind
            && let Some(code) = self.code_blocks.get(block)
            && code.rows.start == row
        {
            let mut said = vec!["code".to_owned()];
            said.extend(code.language_words());
            return said;
        }
        r.details()
    }

    /// The index in the outline of the heading at or above a row.
    pub fn heading_at(&self, row: usize) -> Option<usize> {
        self.outline.iter().rposition(|s| s.line <= row)
    }

    /// The index in the outline of the first heading below a row.
    pub fn next_heading(&self, row: usize) -> Option<usize> {
        self.outline.iter().position(|s| s.line > row)
    }

    /// The index in the outline of the last heading above a row.
    pub fn previous_heading(&self, row: usize) -> Option<usize> {
        self.outline.iter().rposition(|s| s.line < row)
    }

    /// A heading said with its level first and its place last: "Heading
    /// 2: Installation, 3 of 12". `index` is the outline's.
    pub fn heading_words(&self, index: usize) -> Option<String> {
        let symbol = self.outline.get(index)?;
        let SymbolKind::Heading(level) = symbol.kind else {
            return None;
        };
        Some(format!(
            "Heading {level}: {}, {} of {}",
            symbol.name,
            index + 1,
            self.outline.len()
        ))
    }

    /// The table a row is in.
    pub fn table_at(&self, row: usize) -> Option<usize> {
        self.tables.iter().position(|t| t.rows.contains(&row))
    }

    /// The first table that starts below a row.
    pub fn next_table(&self, row: usize) -> Option<usize> {
        self.tables.iter().position(|t| t.rows.start > row)
    }

    /// The last table that starts above a row, the table the row is in
    /// excepted.
    pub fn previous_table(&self, row: usize) -> Option<usize> {
        let here = self.table_at(row);
        self.tables
            .iter()
            .enumerate()
            .rev()
            .find(|(n, t)| t.rows.start < row && Some(*n) != here)
            .map(|(n, _)| n)
    }

    /// A table said with its place first: "Table 1 of 2, 5 rows, 3
    /// columns".
    pub fn table_words(&self, index: usize) -> Option<String> {
        let table = self.tables.get(index)?;
        Some(format!(
            "Table {} of {}, {}, {}",
            index + 1,
            self.tables.len(),
            counted(table.row_count(), "row", "rows"),
            counted(table.column_count(), "column", "columns")
        ))
    }

    /// The links on a row, by their index in [`DocView::links`].
    pub fn links_on(&self, row: usize) -> impl Iterator<Item = usize> + '_ {
        self.links
            .iter()
            .enumerate()
            .filter(move |(_, l)| l.row == row)
            .map(|(n, _)| n)
    }

    /// The first row of the next block below a row.
    pub fn next_block(&self, row: usize) -> Option<usize> {
        let block = self.rows.get(row)?.block;
        self.rows
            .iter()
            .enumerate()
            .skip(row + 1)
            .find(|(_, r)| r.block != block)
            .map(|(n, _)| n)
    }

    /// The first row of the block above the one a row is in.
    pub fn previous_block(&self, row: usize) -> Option<usize> {
        if row >= self.rows.len() {
            return None;
        }
        let start = self.block_start(row);
        (start > 0).then(|| self.block_start(start - 1))
    }

    /// The first row of the block a row is in.
    pub fn block_start(&self, row: usize) -> usize {
        let Some(block) = self.rows.get(row).map(|r| r.block) else {
            return row;
        };
        let mut start = row;
        while start > 0 && self.rows[start - 1].block == block {
            start -= 1;
        }
        start
    }

    /// Every match of `needle` in the rows' shown text, ignoring case, in
    /// order; an empty needle matches nothing.
    pub fn find(&self, needle: &str) -> Vec<DocMatch> {
        let needle = needle.to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut found = Vec::new();
        for (n, row) in self.rows.iter().enumerate() {
            let hay = row.shown().to_lowercase();
            let mut from = 0;
            while let Some(at) = hay[from..].find(&needle) {
                let start = from + at;
                found.push(DocMatch {
                    row: n,
                    start,
                    len: needle.len(),
                });
                from = start + needle.len().max(1);
                if from >= hay.len() {
                    break;
                }
            }
        }
        found
    }

    /// The facts the announcement says after the name and the kind:
    /// "12 headings" for a document with headings, else its rows ("1,204
    /// lines"), or "empty".
    pub fn summary_words(&self) -> String {
        if self.rows.is_empty() {
            return "empty".to_owned();
        }
        if !self.outline.is_empty() {
            return counted(self.outline.len(), "heading", "headings");
        }
        match self.kind {
            DocKind::Hex => "hex".to_owned(),
            DocKind::Code(_) | DocKind::Text => counted(self.rows.len(), "line", "lines"),
            _ => counted(self.rows.len(), "row", "rows"),
        }
    }
}

/// A count in words with its thousands grouped: "1 heading", "1,204
/// lines", "no links".
pub fn counted(n: usize, singular: &str, plural: &str) -> String {
    let n = n as u64;
    match n {
        0 => format!("no {plural}"),
        1 => format!("1 {singular}"),
        _ => format!("{} {plural}", diroxide_core::format::grouped(n)),
    }
}
