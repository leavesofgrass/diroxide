//! Spreadsheets and delimited text as rows (alpha 4, R30 commit 38; R28
//! part 2.2): each sheet a level-1 heading named as the sheet is, its
//! used range a table whose first row is the header row, the cells the
//! values the file last saved (never a formula recomputed). At most
//! [`ContainerCaps::max_table_rows`] rows (10,000) and
//! [`ContainerCaps::max_columns`] columns (200) of a sheet are read, then
//! "and 4,200 more rows" and "and 12 more columns"; a file over
//! [`ContainerCaps::max_sheet_bytes`] (50 MB) is refused before it is
//! loaded. XLSX and XLSB are read cell by cell and stop at the cap; XLS
//! and ODS are read through `calamine`'s whole range, held by the file
//! cap. CSV and TSV go to carta (answer 119), cut to the same caps first
//! by a quote-aware pass ([`cut_delimited`]), so carta never holds more.

use std::io::Cursor;

use calamine::{Data, DataRef, Reader as _};

use super::package::ContainerCaps;
use super::{DocBuilder, DocError, DocKind, DocView};
use crate::words::{clean_text, count_words};

/// The most characters kept of a cell.
pub const MAX_CELL: usize = 1_000;

/// The most sheets of one workbook read.
pub const MAX_SHEETS: usize = 256;

/// A spreadsheet format `calamine` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SheetFormat {
    /// An Excel workbook (a package).
    Xlsx,
    /// An Excel binary workbook (a package).
    Xlsb,
    /// An Excel 97 workbook (a compound file, not a package).
    Xls,
    /// An OpenDocument spreadsheet (a package).
    Ods,
}

impl SheetFormat {
    /// The format's name as said: "XLSX".
    pub const fn name(self) -> &'static str {
        match self {
            SheetFormat::Xlsx => "XLSX",
            SheetFormat::Xlsb => "XLSB",
            SheetFormat::Xls => "XLS",
            SheetFormat::Ods => "ODS",
        }
    }

    /// Whether the format is a zip package, checked by the container read
    /// path before it is read.
    pub const fn is_package(self) -> bool {
        !matches!(self, SheetFormat::Xls)
    }

    /// The format a checked type id says: `xlsx`, `xls`, `ods`.
    pub fn from_type(type_id: &str) -> Option<Self> {
        Some(match type_id {
            "xlsx" => SheetFormat::Xlsx,
            "xls" => SheetFormat::Xls,
            "ods" => SheetFormat::Ods,
            _ => return None,
        })
    }

    /// The format a file name's extension says.
    pub fn from_name(name: &str) -> Option<Self> {
        let (_, ext) = name.rsplit_once('.')?;
        Some(match ext.to_ascii_lowercase().as_str() {
            "xlsx" | "xlsm" | "xltx" | "xltm" => SheetFormat::Xlsx,
            "xlsb" => SheetFormat::Xlsb,
            "xls" | "xlt" => SheetFormat::Xls,
            "ods" | "ots" => SheetFormat::Ods,
            _ => return None,
        })
    }
}

/// A number as a sheet shows it: whole numbers without a point, others
/// to at most ten significant digits, trailing zeros dropped.
pub fn number_words(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{value:.0}");
    }
    let digits = 10 - (value.abs().log10().floor() as i32 + 1).clamp(-5, 10);
    let shown = format!("{value:.*}", digits.max(0) as usize);
    let shown = shown.trim_end_matches('0').trim_end_matches('.');
    shown.to_owned()
}

fn datetime_words(value: &calamine::ExcelDateTime) -> String {
    if value.is_duration() {
        let total = (value.as_f64() * 86_400.0).round() as i64;
        let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
        return format!("{h}:{m:02}:{s:02}");
    }
    let (y, mo, d, h, mi, s, _) = value.to_ymd_hms_milli();
    match (h, mi, s) {
        (0, 0, 0) => format!("{y:04}-{mo:02}-{d:02}"),
        (h, mi, 0) => format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}"),
        (h, mi, s) => format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"),
    }
}

/// A cell's value in words, cleaned like a file name.
pub fn cell_words(value: &Data) -> String {
    match value {
        Data::Int(n) => n.to_string(),
        Data::Float(f) => number_words(*f),
        Data::String(s) => clean_text(s, MAX_CELL),
        Data::Bool(true) => "TRUE".to_owned(),
        Data::Bool(false) => "FALSE".to_owned(),
        Data::DateTime(d) => datetime_words(d),
        Data::DateTimeIso(s) | Data::DurationIso(s) => clean_text(s, MAX_CELL),
        Data::Error(e) => e.to_string(),
        Data::Empty => String::new(),
    }
}

/// What a sheet's table was cut to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cut {
    rows: usize,
    columns: usize,
}

/// Writes one sheet: its name as a heading, its rows as a table (the
/// first the header row), then what was cut.
fn write_sheet(
    b: &mut DocBuilder,
    name: &str,
    rows: Vec<Vec<String>>,
    total_rows: usize,
    total_columns: usize,
) -> Cut {
    b.heading(1);
    b.text(&clean_text(name, 200));
    b.end();
    let shown = rows.len();
    let widest = rows.iter().map(Vec::len).max().unwrap_or(0);
    if shown > 0 {
        b.start_table();
        for (n, row) in rows.into_iter().enumerate() {
            b.start_table_row(n == 0);
            for cell in row {
                b.start_cell();
                b.text(&cell);
                b.end_cell();
            }
            b.end_table_row();
        }
        b.end_table();
    } else {
        b.paragraph();
        b.text("Empty sheet");
        b.end();
    }
    let cut = Cut {
        rows: total_rows.saturating_sub(shown),
        columns: total_columns.saturating_sub(widest),
    };
    if cut.rows > 0 {
        b.paragraph();
        b.text(&super::html::more_rows(cut.rows));
        b.end();
    }
    if cut.columns > 0 {
        b.paragraph();
        b.text(&more_columns(cut.columns));
        b.end();
    }
    cut
}

/// "and 12 more columns".
pub fn more_columns(n: usize) -> String {
    if n == 1 {
        "and 1 more column".to_owned()
    } else {
        format!(
            "and {} more columns",
            diroxide_core::format::grouped(n as u64)
        )
    }
}

/// Rows from a cell stream: cells arrive row by row, sparse; gaps are
/// blank. Stops at the caps and gives what it kept, the rows and columns
/// it saw (counting on past the row cap only when `count_on`).
fn rows_from_cells<'a, E>(
    mut next: impl FnMut() -> Result<Option<calamine::Cell<DataRef<'a>>>, E>,
    start: (u32, u32),
    caps: &ContainerCaps,
    count_on: bool,
) -> (Vec<Vec<String>>, usize, usize) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut seen_rows = 0usize;
    let mut seen_columns = 0usize;
    while let Ok(Some(cell)) = next() {
        let (r, c) = cell.get_position();
        let (r, c) = (
            r.saturating_sub(start.0) as usize,
            c.saturating_sub(start.1) as usize,
        );
        seen_rows = seen_rows.max(r + 1);
        seen_columns = seen_columns.max(c + 1);
        if r >= caps.max_table_rows {
            if count_on {
                continue;
            }
            break;
        }
        if c >= caps.max_columns {
            continue;
        }
        while rows.len() <= r {
            rows.push(Vec::new());
        }
        let row = &mut rows[r];
        while row.len() < c {
            row.push(String::new());
        }
        let value: Data = cell.get_value().clone().into();
        row.push(cell_words(&value));
    }
    (rows, seen_rows, seen_columns)
}

/// Rows from a whole range, cut to the caps.
fn rows_from_range(range: &calamine::Range<Data>, caps: &ContainerCaps) -> Vec<Vec<String>> {
    range
        .rows()
        .take(caps.max_table_rows)
        .map(|row| {
            let mut cells: Vec<String> =
                row.iter().take(caps.max_columns).map(cell_words).collect();
            while cells.last().is_some_and(String::is_empty) {
                cells.pop();
            }
            cells
        })
        .collect()
}

/// The sheet count and rows said after the kind: "3 sheets, 1,204 rows".
fn said(sheets: usize, rows: usize) -> String {
    format!(
        "{}, {}",
        count_words(sheets as u64, "sheet", "sheets"),
        count_words(rows as u64, "row", "rows")
    )
}

/// Reads a workbook's bytes into rows, every sheet within the caps.
/// Unguarded: the container read path runs it under the guard.
pub fn read(
    format: SheetFormat,
    bytes: Vec<u8>,
    caps: &ContainerCaps,
) -> Result<DocView, DocError> {
    if bytes.len() as u64 > caps.max_sheet_bytes {
        return Err(DocError::TooLarge);
    }
    let not_valid = || DocError::NotValid(format.name().to_owned());
    let mut b = DocBuilder::new(DocKind::Format(format.name().to_owned()));
    let mut total = 0usize;
    let mut sheets = 0usize;
    let source = Cursor::new(bytes);
    match format {
        SheetFormat::Xlsx => {
            let mut book: calamine::Xlsx<_> =
                calamine::open_workbook_from_rs(source).map_err(|_| not_valid())?;
            for name in book.sheet_names().into_iter().take(MAX_SHEETS) {
                let Ok(mut cells) = book.worksheet_cells_reader(&name) else {
                    continue;
                };
                let dims = cells.dimensions();
                let known = dims.end != (0, 0);
                let (rows, seen, columns) =
                    rows_from_cells(|| cells.next_cell(), dims.start, caps, !known);
                let rows_total = if known {
                    (dims.end.0 - dims.start.0 + 1) as usize
                } else {
                    seen
                };
                let columns_total = if known {
                    (dims.end.1 - dims.start.1 + 1) as usize
                } else {
                    columns
                };
                write_sheet(&mut b, &name, rows, rows_total, columns_total);
                total += rows_total;
                sheets += 1;
            }
        }
        SheetFormat::Xlsb => {
            let mut book: calamine::Xlsb<_> =
                calamine::open_workbook_from_rs(source).map_err(|_| not_valid())?;
            for name in book.sheet_names().into_iter().take(MAX_SHEETS) {
                let Ok(mut cells) = book.worksheet_cells_reader(&name) else {
                    continue;
                };
                let dims = cells.dimensions();
                let known = dims.end != (0, 0);
                let (rows, seen, columns) =
                    rows_from_cells(|| cells.next_cell(), dims.start, caps, !known);
                let rows_total = if known {
                    (dims.end.0 - dims.start.0 + 1) as usize
                } else {
                    seen
                };
                let columns_total = if known {
                    (dims.end.1 - dims.start.1 + 1) as usize
                } else {
                    columns
                };
                write_sheet(&mut b, &name, rows, rows_total, columns_total);
                total += rows_total;
                sheets += 1;
            }
        }
        SheetFormat::Xls => {
            let mut book: calamine::Xls<_> =
                calamine::open_workbook_from_rs(source).map_err(|_| not_valid())?;
            for name in book.sheet_names().into_iter().take(MAX_SHEETS) {
                let Ok(range) = book.worksheet_range(&name) else {
                    continue;
                };
                write_sheet(
                    &mut b,
                    &name,
                    rows_from_range(&range, caps),
                    range.height(),
                    range.width(),
                );
                total += range.height();
                sheets += 1;
            }
        }
        SheetFormat::Ods => {
            let mut book: calamine::Ods<_> =
                calamine::open_workbook_from_rs(source).map_err(|_| not_valid())?;
            for name in book.sheet_names().into_iter().take(MAX_SHEETS) {
                let Ok(range) = book.worksheet_range(&name) else {
                    continue;
                };
                write_sheet(
                    &mut b,
                    &name,
                    rows_from_range(&range, caps),
                    range.height(),
                    range.width(),
                );
                total += range.height();
                sheets += 1;
            }
        }
    }
    let mut view = b.finish();
    view.set_summary_words(said(sheets, total));
    Ok(view)
}

/// Delimited text cut to the caps before a reader sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delimited {
    /// The text kept: the first rows, each at most the column cap wide.
    pub text: String,
    /// How many records the whole text has.
    pub rows: usize,
    /// How many records were left out.
    pub more_rows: usize,
    /// How many fields the widest record had past the column cap.
    pub more_columns: usize,
}

/// Cuts delimited text (CSV with `b','`, TSV with `b'\t'`) to at most
/// `max_rows` records of at most `max_columns` fields, quote-aware: a
/// separator or a line break inside double quotes belongs to its field.
/// The records past the cap are counted, not kept.
pub fn cut_delimited(text: &str, separator: u8, max_rows: usize, max_columns: usize) -> Delimited {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len().min(1 << 20));
    let mut quoted = false;
    let mut field = 0usize;
    let mut widest = 0usize;
    let mut rows = 0usize;
    let mut record_has_bytes = false;
    let quotes = separator == b',';
    for &byte in bytes {
        let keep_row = rows < max_rows;
        if quotes && byte == b'"' {
            quoted = !quoted;
        }
        if !quoted && byte == separator {
            field += 1;
            widest = widest.max(field + 1);
            record_has_bytes = true;
            if keep_row && field < max_columns {
                out.push(byte);
            }
            continue;
        }
        if !quoted && byte == b'\n' {
            if keep_row {
                if out.last() == Some(&b'\r') {
                    out.pop();
                }
                out.push(b'\n');
            }
            rows += 1;
            field = 0;
            record_has_bytes = false;
            continue;
        }
        record_has_bytes = true;
        widest = widest.max(field + 1);
        if keep_row && field < max_columns {
            out.push(byte);
        }
    }
    if record_has_bytes {
        rows += 1;
    }
    Delimited {
        text: String::from_utf8_lossy(&out).into_owned(),
        rows,
        more_rows: rows.saturating_sub(max_rows),
        more_columns: widest.saturating_sub(max_columns),
    }
}

#[cfg(test)]
mod tests;
