// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::doc::package::tests::zip_of;
use crate::doc::{self, DocRow, ReadOptions, Reader, RowKind};

/// An Excel workbook of sheets, each a name and its rows of cells: a
/// cell that parses as a number is a number, any other an inline string.
/// `dimension` writes each sheet's used range, as Excel does.
pub(crate) fn xlsx(sheets: &[(&str, Vec<Vec<String>>)], dimension: bool) -> Vec<u8> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut overrides = String::new();
    let mut workbook_sheets = String::new();
    let mut rels = String::new();
    for (n, (name, rows)) in sheets.iter().enumerate() {
        let id = n + 1;
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{id}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        workbook_sheets.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{id}" r:id="rId{id}"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{id}.xml"/>"#
        ));
        let mut xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        );
        let width = rows.iter().map(Vec::len).max().unwrap_or(1);
        if dimension && !rows.is_empty() {
            xml.push_str(&format!(
                r#"<dimension ref="A1:{}{}"/>"#,
                column(width - 1),
                rows.len()
            ));
        }
        xml.push_str("<sheetData>");
        for (r, row) in rows.iter().enumerate() {
            xml.push_str(&format!(r#"<row r="{}">"#, r + 1));
            for (c, cell) in row.iter().enumerate() {
                let at = format!("{}{}", column(c), r + 1);
                if cell.parse::<f64>().is_ok() {
                    xml.push_str(&format!(r#"<c r="{at}"><v>{cell}</v></c>"#));
                } else if !cell.is_empty() {
                    xml.push_str(&format!(
                        r#"<c r="{at}" t="inlineStr"><is><t>{cell}</t></is></c>"#
                    ));
                }
            }
            xml.push_str("</row>");
        }
        xml.push_str("</sheetData></worksheet>");
        parts.push((format!("xl/worksheets/sheet{id}.xml"), xml));
    }
    let mut members: Vec<(String, String)> = vec![
        (
            "[Content_Types].xml".to_owned(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>{overrides}</Types>"#
            ),
        ),
        (
            "_rels/.rels".to_owned(),
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_owned(),
        ),
        (
            "xl/workbook.xml".to_owned(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{workbook_sheets}</sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".to_owned(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
            ),
        ),
    ];
    members.extend(parts);
    let refs: Vec<(&str, &[u8])> = members
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_bytes()))
        .collect();
    zip_of(&refs)
}

/// A column's letters: 0 is A, 26 is AA.
fn column(mut n: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

fn grid(rows: usize, columns: usize) -> Vec<Vec<String>> {
    (0..rows)
        .map(|r| {
            (0..columns)
                .map(|c| {
                    if r == 0 {
                        format!("Head{c}")
                    } else {
                        (r * 1000 + c).to_string()
                    }
                })
                .collect()
        })
        .collect()
}

fn ods(name: &str, rows: &[&[&str]]) -> Vec<u8> {
    let mut table = format!(r#"<table:table table:name="{name}">"#);
    for row in rows {
        table.push_str("<table:table-row>");
        for cell in *row {
            if cell.parse::<f64>().is_ok() {
                table.push_str(&format!(
                    r#"<table:table-cell office:value-type="float" office:value="{cell}"><text:p>{cell}</text:p></table:table-cell>"#
                ));
            } else {
                table.push_str(&format!(
                    r#"<table:table-cell office:value-type="string"><text:p>{cell}</text:p></table:table-cell>"#
                ));
            }
        }
        table.push_str("</table:table-row>");
    }
    table.push_str("</table:table>");
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" office:version="1.3"><office:body><office:spreadsheet>{table}</office:spreadsheet></office:body></office:document-content>"#
    );
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#;
    zip_of(&[
        (
            "mimetype",
            b"application/vnd.oasis.opendocument.spreadsheet",
        ),
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("content.xml", content.as_bytes()),
    ])
}

fn seekable(reader: Reader, bytes: Vec<u8>) -> Result<DocView, DocError> {
    doc::read_seekable(
        reader,
        "book",
        &mut Cursor::new(bytes),
        &ReadOptions::default(),
        &ContainerCaps::default(),
    )
}

fn shown(view: &DocView) -> Vec<String> {
    view.rows().iter().map(DocRow::shown).collect()
}

#[test]
fn each_sheet_is_a_heading_and_its_first_row_the_header() {
    let book = xlsx(
        &[
            ("Budget", grid(3, 3)),
            (
                "Notes",
                vec![
                    vec!["Who".to_owned(), "Said".to_owned()],
                    vec!["Ada Example".to_owned(), "Crows remember faces".to_owned()],
                ],
            ),
        ],
        true,
    );
    let view = seekable(Reader::Sheet(SheetFormat::Xlsx), book).unwrap();
    assert_eq!(view.kind, DocKind::Format("XLSX".to_owned()));
    assert_eq!(view.summary_words(), "2 sheets, 5 rows");
    assert_eq!(view.outline().len(), 2);
    assert_eq!(
        view.heading_words(1).as_deref(),
        Some("Heading 1: Notes, 2 of 2")
    );
    let budget = &view.tables()[0];
    assert_eq!(budget.header, Some(0));
    assert_eq!(budget.cells[0], ["Head0", "Head1", "Head2"]);
    assert_eq!(budget.cell_words(2, 1), "Head1: 2001, row 3 of 3, column 2");
    assert_eq!(
        view.tables()[1].cell_words(1, 1),
        "Said: Crows remember faces, row 2 of 2, column 2"
    );
}

#[test]
fn a_sheet_is_cut_at_ten_thousand_rows_and_two_hundred_columns() {
    let tall = xlsx(&[("Rows", grid(14_200, 2))], true);
    let view = seekable(Reader::Sheet(SheetFormat::Xlsx), tall).unwrap();
    assert_eq!(view.tables()[0].row_count(), 10_000);
    assert!(shown(&view).contains(&"and 4,200 more rows".to_owned()));
    assert_eq!(view.summary_words(), "1 sheet, 14,200 rows");
    // Without a used range in the file, the rows past the cap are counted.
    let tall = xlsx(&[("Rows", grid(10_050, 2))], false);
    let view = seekable(Reader::Sheet(SheetFormat::Xlsx), tall).unwrap();
    assert!(shown(&view).contains(&"and 50 more rows".to_owned()));
    let wide = xlsx(&[("Wide", grid(2, 212))], true);
    let view = seekable(Reader::Sheet(SheetFormat::Xlsx), wide).unwrap();
    assert_eq!(view.tables()[0].column_count(), 200);
    assert!(shown(&view).contains(&"and 12 more columns".to_owned()));
}

#[test]
fn an_opendocument_sheet_reads_by_its_range() {
    let book = ods(
        "Counts",
        &[&["Bird", "Seen"], &["Crow", "12"], &["Jay", "3.5"]],
    );
    let view = seekable(Reader::Sheet(SheetFormat::Ods), book).unwrap();
    assert_eq!(view.kind, DocKind::Format("ODS".to_owned()));
    assert_eq!(
        shown(&view),
        ["Counts", "Bird | Seen", "Crow | 12", "Jay | 3.5"]
    );
    assert!(matches!(
        view.rows()[1].kind,
        RowKind::TableRow { header: true, .. }
    ));
}

#[test]
fn damaged_and_oversized_books_are_refused_in_words() {
    assert_eq!(
        seekable(Reader::Sheet(SheetFormat::Xls), b"not a workbook".to_vec()).unwrap_err(),
        DocError::NotValid("XLS".to_owned())
    );
    let caps = ContainerCaps {
        max_sheet_bytes: 10,
        ..ContainerCaps::default()
    };
    assert_eq!(
        read(SheetFormat::Xlsx, vec![0; 11], &caps).unwrap_err(),
        DocError::TooLarge
    );
    assert_eq!(
        Reader::for_name("data.xlsx"),
        Reader::Sheet(SheetFormat::Xlsx)
    );
    assert_eq!(Reader::for_name("old.XLS"), Reader::Sheet(SheetFormat::Xls));
    assert!(Reader::for_name("data.ods").is_package());
}

#[test]
fn numbers_and_dates_read_as_a_sheet_shows_them() {
    assert_eq!(number_words(1234.0), "1234");
    assert_eq!(number_words(0.1 + 0.2), "0.3");
    assert_eq!(number_words(12.345_678_901_234), "12.3456789");
    assert_eq!(number_words(-0.000_125), "-0.000125");
    let date = calamine::ExcelDateTime::new(46_158.0, calamine::ExcelDateTimeType::DateTime, false);
    assert_eq!(cell_words(&Data::DateTime(date)), "2026-05-16");
    let time = calamine::ExcelDateTime::new(46_158.5, calamine::ExcelDateTimeType::DateTime, false);
    assert_eq!(cell_words(&Data::DateTime(time)), "2026-05-16 12:00");
    assert_eq!(cell_words(&Data::Bool(true)), "TRUE");
    assert_eq!(
        cell_words(&Data::String("a\u{202E}b\nc".to_owned())),
        "a?b c"
    );
}

#[test]
fn delimited_text_is_cut_by_records_not_by_lines() {
    let text = "Name,Note\r\n\"Crow, common\",\"says\n caw\"\r\nJay,blue\r\n";
    let cut = cut_delimited(text, b',', 10, 10);
    assert_eq!(cut.rows, 3);
    assert_eq!((cut.more_rows, cut.more_columns), (0, 0));
    assert_eq!(
        cut.text,
        "Name,Note\n\"Crow, common\",\"says\n caw\"\nJay,blue\n"
    );
    let cut = cut_delimited(text, b',', 2, 1);
    assert_eq!(cut.text, "Name\n\"Crow, common\"\n");
    assert_eq!((cut.rows, cut.more_rows, cut.more_columns), (3, 1, 1));
    let tsv = cut_delimited("a\tb\tc\nd\te\tf", b'\t', 5, 2);
    assert_eq!(tsv.text, "a\tb\nd\te");
    assert_eq!(tsv.more_columns, 1);
}

#[test]
fn a_long_csv_is_cut_before_carta_reads_it() {
    let mut text = String::from("Bird,Count\n");
    for n in 0..10_049 {
        text.push_str(&format!("crow {n},{n}\n"));
    }
    let view = doc::read("birds.csv", text.into_bytes(), &ReadOptions::default()).unwrap();
    assert_eq!(view.tables()[0].row_count(), 10_000);
    assert_eq!(
        shown(&view).last().map(String::as_str),
        Some("and 50 more rows")
    );
    assert_eq!(view.summary_words(), "10,050 rows");
}
