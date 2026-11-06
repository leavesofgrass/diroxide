//! DOCX, EPUB, ODT, HTML and CSV through carta 0.0.13 and the
//! HTML-fragment path (alpha 4, R30 commit 37; spike C2, answer 119):
//! twelve samples each, made here in memory, keep their headings, their
//! tables with the header row, and their lists' nesting.

use std::io::{Cursor, Write};

use diroxide_preview::doc::package::ContainerCaps;
use diroxide_preview::doc::{self, CartaFormat, DocKind, DocView, ReadOptions, Reader, RowKind};

/// What one sample holds: sections (each a level-1 and a level-2
/// heading), a table of `rows` by `columns` with its header row, and a
/// list nested `depth` deep.
#[derive(Clone, Copy, Debug)]
struct Shape {
    sections: usize,
    rows: usize,
    columns: usize,
    depth: usize,
}

fn shapes() -> Vec<Shape> {
    (0..12)
        .map(|n| Shape {
            sections: 1 + n % 3,
            rows: 2 + n % 4,
            columns: 2 + n % 3,
            depth: 1 + n % 3,
        })
        .collect()
}

fn zip_of(members: &[(&str, String)], stored_first: bool) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (n, (name, data)) in members.iter().enumerate() {
        let method = if stored_first && n == 0 {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        out.start_file(*name, options).unwrap();
        out.write_all(data.as_bytes()).unwrap();
    }
    out.finish().unwrap().into_inner()
}

const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";

fn docx(shape: Shape) -> Vec<u8> {
    let mut body = String::new();
    for s in 0..shape.sections {
        body.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Section {s}</w:t></w:r></w:p>\
             <w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>Part {s}</w:t></w:r></w:p>\
             <w:p><w:r><w:t>Crows gather at dusk.</w:t></w:r></w:p>"
        ));
    }
    body.push_str("<w:tbl><w:tblGrid>");
    for _ in 0..shape.columns {
        body.push_str("<w:gridCol w:w=\"2000\"/>");
    }
    body.push_str("</w:tblGrid>");
    for r in 0..shape.rows {
        body.push_str("<w:tr>");
        if r == 0 {
            body.push_str("<w:trPr><w:tblHeader/></w:trPr>");
        }
        for c in 0..shape.columns {
            let text = if r == 0 {
                format!("Head {c}")
            } else {
                format!("Cell {r} {c}")
            };
            body.push_str(&format!(
                "<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"
            ));
        }
        body.push_str("</w:tr>");
    }
    body.push_str("</w:tbl>");
    for d in 0..shape.depth {
        body.push_str(&format!(
            "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{d}\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr>\
             <w:r><w:t>Item at level {d}</w:t></w:r></w:p>"
        ));
    }
    let document =
        format!("<?xml version=\"1.0\"?><w:document {W}><w:body>{body}</w:body></w:document>");
    let mut styles = format!("<?xml version=\"1.0\"?><w:styles {W}>");
    for level in 1..=2 {
        styles.push_str(&format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{level}\"><w:name w:val=\"heading {level}\"/></w:style>"
        ));
    }
    styles.push_str("</w:styles>");
    let mut numbering =
        format!("<?xml version=\"1.0\"?><w:numbering {W}><w:abstractNum w:abstractNumId=\"0\">");
    for level in 0..3 {
        numbering.push_str(&format!(
            "<w:lvl w:ilvl=\"{level}\"><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"*\"/></w:lvl>"
        ));
    }
    numbering.push_str(
        "</w:abstractNum><w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num></w:numbering>",
    );
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    zip_of(
        &[
            ("[Content_Types].xml", types.to_owned()),
            ("_rels/.rels", rels.to_owned()),
            ("word/document.xml", document),
            ("word/styles.xml", styles),
            ("word/numbering.xml", numbering),
        ],
        false,
    )
}

/// The HTML body EPUB and HTML samples share.
fn html_body(shape: Shape) -> String {
    let mut body = String::new();
    for s in 0..shape.sections {
        body.push_str(&format!(
            "<h1>Section {s}</h1><h2>Part {s}</h2><p>Crows gather at dusk.</p>"
        ));
    }
    body.push_str("<table><thead><tr>");
    for c in 0..shape.columns {
        body.push_str(&format!("<th>Head {c}</th>"));
    }
    body.push_str("</tr></thead><tbody>");
    for r in 1..shape.rows {
        body.push_str("<tr>");
        for c in 0..shape.columns {
            body.push_str(&format!("<td>Cell {r} {c}</td>"));
        }
        body.push_str("</tr>");
    }
    body.push_str("</tbody></table>");
    for d in 0..shape.depth {
        body.push_str(&format!("<ul><li>Item at level {d}"));
    }
    for _ in 0..shape.depth {
        body.push_str("</li></ul>");
    }
    body
}

fn html(shape: Shape) -> Vec<u8> {
    format!(
        "<!DOCTYPE html><html><head><title>Example Page</title><script>alert(1)</script></head><body>{}</body></html>",
        html_body(shape)
    )
    .into_bytes()
}

fn epub(shape: Shape) -> Vec<u8> {
    let chapter = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Chapter</title></head><body>{}</body></html>",
        html_body(shape)
    );
    let nav = r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><ol><li><a href="chapter1.xhtml">Chapter</a></li></ol></nav></body></html>"#;
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:uuid:00000000-0000-4000-8000-000000000000</dc:identifier><dc:title>Example Book</dc:title><dc:language>en</dc:language></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
    let container = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    zip_of(
        &[
            ("mimetype", "application/epub+zip".to_owned()),
            ("META-INF/container.xml", container.to_owned()),
            ("OEBPS/content.opf", opf.to_owned()),
            ("OEBPS/nav.xhtml", nav.to_owned()),
            ("OEBPS/chapter1.xhtml", chapter),
        ],
        true,
    )
}

fn odt(shape: Shape) -> Vec<u8> {
    let mut text = String::new();
    for s in 0..shape.sections {
        text.push_str(&format!(
            "<text:h text:outline-level=\"1\">Section {s}</text:h><text:h text:outline-level=\"2\">Part {s}</text:h><text:p>Crows gather at dusk.</text:p>"
        ));
    }
    text.push_str(&format!(
        "<table:table table:name=\"Counts\"><table:table-column table:number-columns-repeated=\"{}\"/><table:table-header-rows><table:table-row>",
        shape.columns
    ));
    for c in 0..shape.columns {
        text.push_str(&format!(
            "<table:table-cell><text:p>Head {c}</text:p></table:table-cell>"
        ));
    }
    text.push_str("</table:table-row></table:table-header-rows>");
    for r in 1..shape.rows {
        text.push_str("<table:table-row>");
        for c in 0..shape.columns {
            text.push_str(&format!(
                "<table:table-cell><text:p>Cell {r} {c}</text:p></table:table-cell>"
            ));
        }
        text.push_str("</table:table-row>");
    }
    text.push_str("</table:table>");
    for d in 0..shape.depth {
        text.push_str(&format!(
            "<text:list><text:list-item><text:p>Item at level {d}</text:p>"
        ));
    }
    for _ in 0..shape.depth {
        text.push_str("</text:list-item></text:list>");
    }
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" office:version=\"1.3\"><office:body><office:text>{text}</office:text></office:body></office:document-content>"
    );
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#;
    zip_of(
        &[
            (
                "mimetype",
                "application/vnd.oasis.opendocument.text".to_owned(),
            ),
            ("META-INF/manifest.xml", manifest.to_owned()),
            ("content.xml", content),
        ],
        true,
    )
}

fn csv(shape: Shape, n: usize) -> Vec<u8> {
    let rows = shape.rows + n;
    let mut out = String::new();
    for r in 0..rows {
        let cells: Vec<String> = (0..shape.columns)
            .map(|c| {
                if r == 0 {
                    format!("Head {c}")
                } else if c == 0 {
                    format!("\"Cell {r}, quoted\"")
                } else {
                    format!("{}", r * 10 + c)
                }
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out.into_bytes()
}

fn deepest_list(view: &DocView) -> u8 {
    view.rows()
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::ListItem { depth, .. } => Some(depth),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

fn package(format: CartaFormat, bytes: Vec<u8>) -> DocView {
    let mut source = Cursor::new(bytes);
    doc::read_seekable(
        Reader::Carta(format),
        "sample",
        &mut source,
        &ReadOptions::default(),
        &ContainerCaps::default(),
    )
    .unwrap()
}

/// Checks one sample's view against its shape, collecting what differs.
fn check(name: &str, view: &DocView, shape: Shape, failed: &mut Vec<String>) {
    let mut wrong = Vec::new();
    if view.outline().len() != 2 * shape.sections {
        wrong.push(format!(
            "headings {} not {}",
            view.outline().len(),
            2 * shape.sections
        ));
    }
    match view.tables() {
        [table] => {
            if table.header != Some(0) {
                wrong.push(format!("header {:?}", table.header));
            }
            if (table.row_count(), table.column_count()) != (shape.rows, shape.columns) {
                wrong.push(format!(
                    "table {} by {} not {} by {}",
                    table.row_count(),
                    table.column_count(),
                    shape.rows,
                    shape.columns
                ));
            }
        }
        other => wrong.push(format!("{} tables", other.len())),
    }
    if usize::from(deepest_list(view)) != shape.depth {
        wrong.push(format!(
            "list depth {} not {}",
            deepest_list(view),
            shape.depth
        ));
    }
    if !wrong.is_empty() {
        failed.push(format!("{name}: {}", wrong.join("; ")));
    }
}

#[test]
fn twelve_samples_of_each_keep_headings_tables_and_list_nesting() {
    let mut failed = Vec::new();
    for (n, shape) in shapes().into_iter().enumerate() {
        let view = package(CartaFormat::Docx, docx(shape));
        assert_eq!(view.kind, DocKind::Carta("DOCX".to_owned()));
        check(&format!("docx {n}"), &view, shape, &mut failed);
        let view = package(CartaFormat::Epub, epub(shape));
        check(&format!("epub {n}"), &view, shape, &mut failed);
        let view = package(CartaFormat::Odt, odt(shape));
        check(&format!("odt {n}"), &view, shape, &mut failed);
        let view = doc::read("page.html", html(shape), &ReadOptions::default()).unwrap();
        assert_eq!(view.kind, DocKind::Carta("HTML".to_owned()));
        check(&format!("html {n}"), &view, shape, &mut failed);
        assert!(
            !view.rows().iter().any(|r| r.text.contains("alert")),
            "a script is never read"
        );
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn twelve_tables_of_comma_separated_values_keep_their_header_row() {
    for (n, shape) in shapes().into_iter().enumerate() {
        let view = doc::read("data.csv", csv(shape, n), &ReadOptions::default()).unwrap();
        assert_eq!(view.kind, DocKind::Carta("CSV".to_owned()));
        let table = &view.tables()[0];
        assert_eq!(table.header, Some(0), "csv {n}");
        assert_eq!(
            (table.row_count(), table.column_count()),
            (shape.rows + n, shape.columns),
            "csv {n}"
        );
        assert_eq!(table.cell(1, 0), Some("Cell 1, quoted"));
        assert_eq!(
            table.cell_words(1, 0),
            format!(
                "Head 0: Cell 1, quoted, row 2 of {}, column 1",
                shape.rows + n
            )
        );
    }
}

#[test]
fn a_package_whose_parts_overlap_never_reaches_carta() {
    let mut bytes = docx(shapes()[0]);
    let central: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(at, _)| at)
        .collect();
    let second = central[1];
    bytes[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
    let mut source = Cursor::new(bytes);
    let error = doc::read_seekable(
        Reader::Carta(CartaFormat::Docx),
        "bad.docx",
        &mut source,
        &ReadOptions::default(),
        &ContainerCaps::default(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "its parts overlap");
    assert_eq!(
        Reader::for_name("report.docx"),
        Reader::Carta(CartaFormat::Docx)
    );
    assert!(Reader::for_name("report.docx").is_package());
    assert!(!Reader::for_name("page.html").is_package());
}

#[test]
fn a_word_document_on_disk_previews_with_its_headings() {
    let preview = crate::documents::preview_of("report.docx", &docx(shapes()[1]), "docx");
    assert_eq!(preview.summary, "DOCX, 4 headings");
    let view = preview.document.unwrap();
    assert_eq!(
        view.heading_words(0).as_deref(),
        Some("Heading 1: Section 0, 1 of 4")
    );
}
