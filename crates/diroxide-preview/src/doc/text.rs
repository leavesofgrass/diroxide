//! Plain text and code into the document view: one row a line, as
//! written, its language found by name and first line (the highlighter's
//! nine), and binary refused to hex. The highlight runs come from the
//! viewer's highlighter (W2d's [`Session`]): [`highlight`] starts the
//! worker on the rows and [`DocView::with_runs`] puts its classes on
//! them, so the quick view's code is heard and seen as the viewer's is.
//!
//! Every row is shown as the viewer shows a line: tabs to spaces at stops
//! of four, and every hidden character (controls, escapes, format,
//! direction and private-use characters) as a question mark, so a file
//! cannot move the cursor, color the screen or hide text from a reader.

use std::sync::Arc;

use diroxide_core::names::classify;

use super::{DocKind, DocView, RowKind};
use crate::encoding::{binary_kind, decode};
use crate::hex::{HexLayout, hex_rows};
use crate::highlight::{Caps, Job, Language, Refusal, Session};

/// Where tabs stop, in columns, as in the viewer.
pub const TAB_STOP: usize = 4;

/// The bytes a hex row shows in screen reader mode: eight, inside 40
/// cells, as the viewer's hex mode.
pub const HEX_ROW_SCREEN_READER: usize = 8;

/// The bytes a hex row shows otherwise.
pub const HEX_ROW: usize = 16;

/// Reads a file's bytes as plain text or code, or as hex when they are
/// not text. `name` finds the language; `hex_row` is the bytes a hex row
/// shows ([`HEX_ROW_SCREEN_READER`] or [`HEX_ROW`]).
pub fn read(name: &str, bytes: &[u8], hex_row: usize) -> DocView {
    if binary_kind(bytes).is_some() {
        return read_hex(bytes, hex_row);
    }
    let decoded = decode(bytes, None);
    let text = decoded.text.as_str();
    let first = text.lines().next().unwrap_or("");
    let kind = match Language::detect(name, first) {
        Some(language) => DocKind::Code(language),
        None => DocKind::Text,
    };
    let mut b = DocView::builder(kind);
    let body = text.strip_suffix('\n').unwrap_or(text);
    if !body.is_empty() {
        for line in body.split('\n') {
            b.line(RowKind::Line, line.strip_suffix('\r').unwrap_or(line));
        }
    }
    b.finish()
}

/// Reads text bytes as plain text, whatever the name says: the fallback
/// when a format's reader could not read a file. Bytes that are not text
/// are hex.
pub fn read_plain(bytes: &[u8], hex_row: usize) -> DocView {
    read("", bytes, hex_row)
}

/// Bytes as hex rows: an offset, the bytes, the printable characters.
pub fn read_hex(bytes: &[u8], hex_row: usize) -> DocView {
    let layout = HexLayout::for_len(bytes.len() as u64, hex_row.max(1));
    let mut b = DocView::builder(DocKind::Hex);
    for row in hex_rows(bytes, 0, layout) {
        b.line(RowKind::Hex, &row);
    }
    b.finish()
}

/// Starts highlighting a code view's rows on the session's worker, within
/// the caps; `bytes` is the file's size. `None` when the view is not code;
/// the refusal says why not, in words ("Highlighting off: over 512 KiB").
pub fn highlight(
    view: &DocView,
    session: &Session,
    caps: Caps,
    bytes: u64,
) -> Option<Result<Job, Refusal>> {
    let DocKind::Code(language) = view.kind else {
        return None;
    };
    let lines: Vec<String> = view.rows().iter().map(|r| r.text.clone()).collect();
    Some(session.start(language, Arc::new(lines), bytes, caps))
}

/// A line as shown: tabs to spaces at stops of [`TAB_STOP`], every hidden
/// character as a question mark.
pub fn neutralize(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut column = 0usize;
    for c in line.chars() {
        if c == '\t' {
            let spaces = TAB_STOP - column % TAB_STOP;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else if classify(c).is_hidden() {
            out.push('?');
            column += 1;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}

impl DocView {
    /// Whether each row is a line of the file, as for plain text and code,
    /// so it can be numbered.
    pub fn is_line_based(&self) -> bool {
        matches!(self.kind, DocKind::Text | DocKind::Code(_))
    }

    /// A row as drawn and read: its shown text, with its line number first
    /// ("12: fn main() {") when `line_numbers` is on and the view is a
    /// file's lines.
    pub fn label(&self, row: usize, line_numbers: bool) -> String {
        let Some(r) = self.row(row) else {
            return String::new();
        };
        if line_numbers && self.is_line_based() {
            format!(
                "{}: {}",
                diroxide_core::format::grouped(row as u64 + 1),
                r.shown()
            )
        } else {
            r.shown()
        }
    }

    /// The one line said when the preview is shown, meaning first and
    /// never the text itself: "Preview: report.md, 12 headings",
    /// "Preview: main.rs, Rust, 412 lines", "Preview: a.bin, binary,
    /// hex".
    pub fn preview_line(&self, name: &str) -> String {
        match self.kind.word() {
            Some(word) => format!("Preview: {name}, {word}, {}", self.summary_words()),
            None => format!("Preview: {name}, {}", self.summary_words()),
        }
    }
}
