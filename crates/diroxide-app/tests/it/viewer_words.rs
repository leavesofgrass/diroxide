//! F3 on a file whose bytes are not text (lane VP, the owner's first
//! finding of the alpha 4 check): a PDF, an Office document and, in the
//! terminal, a picture open on the preview's words instead of hex, the
//! bytes one Hex command away; a plain binary file keeps hex. Every file
//! is made in memory and written under the test's own folder; no test
//! starts a real program (the fake tools answer pdftotext and pdftoppm).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use diroxide_app::Keymap;
use diroxide_app::app::picture_view::{IN_THE_WINDOW, PageAsk, PictureSource, page_hint_words};
use diroxide_app::commands::ids;
use diroxide_app::viewer::{ViewerMode, hex_hint, preview_title};
use diroxide_core::KeyColumn;
use diroxide_core::format::BRAILLE_CELLS;
use diroxide_vfs::Registry;

use crate::support::{FakeTools, Session, Setup, place, png_bytes, temp};

/// A PDF of pages, each with one line of text, with a title. Offsets are
/// counted, so the cross-reference table is true.
fn pdf(pages: &[&str], title: &str) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let page_ids: Vec<usize> = (0..pages.len()).map(|n| 5 + 2 * n).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_owned());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        page_ids
            .iter()
            .map(|id| format!("{id} 0 R"))
            .collect::<Vec<_>>()
            .join(" "),
        pages.len()
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());
    objects.push(format!("<< /Title ({title}) /Producer (diroxide tests) >>"));
    for (n, text) in pages.iter().enumerate() {
        let content = format!("BT /F1 18 Tf 72 700 Td ({text}) Tj ET");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            page_ids[n] + 1
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (n, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", n + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A zip of stored members, each with its CRC, the central directory and
/// its end.
fn stored_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let at = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    let count = (members.len() as u16).to_le_bytes();
    out.extend_from_slice(&count);
    out.extend_from_slice(&count);
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// A Word file of one heading and one paragraph.
fn docx() -> Vec<u8> {
    let w = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";
    let types = "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>";
    let rels = "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
    let document = format!(
        "<?xml version=\"1.0\"?><w:document {w}><w:body>\
         <w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Crows</w:t></w:r></w:p>\
         <w:p><w:r><w:t>They meet at dawn.</w:t></w:r></w:p>\
         </w:body></w:document>"
    );
    let styles = format!(
        "<?xml version=\"1.0\"?><w:styles {w}><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/></w:style></w:styles>"
    );
    stored_zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
    ])
}

/// A session on a folder of made files, with the keys of one column and
/// these fake tools.
fn open(column: KeyColumn, tools: Arc<FakeTools>) -> (tempfile::TempDir, Session) {
    let dir = temp();
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("photo.png"), png_bytes(4, 3)).unwrap();
    fs::write(
        work.join("report.pdf"),
        pdf(
            &["Crows at dawn", "Crows at noon", "Crows at dusk"],
            "Example Report",
        ),
    )
    .unwrap();
    fs::write(work.join("notes.docx"), docx()).unwrap();
    fs::write(work.join("blob.bin"), [0u8, 159, 146, 150, 0, 1, 2, 3]).unwrap();
    let session = Session::open_with_keymap(
        place(&work),
        place(dir.path()),
        Registry::with_local(),
        Setup {
            tools,
            ..Setup::default()
        },
        Keymap::defaults_for(column),
    );
    (dir, session)
}

/// The viewer's lines, from the first, as many as it has (at most 60).
fn lines(s: &mut Session) -> Vec<String> {
    let viewer = s.app.viewer_mut().expect("the viewer is open");
    (0..60).map_while(|n| viewer.line_text(n)).collect()
}

/// pdftotext's answer for the made PDF: three pages, a form feed after
/// each.
fn pdftotext_answer() -> &'static [u8] {
    b"Crows at dawn\n\x0cCrows at noon\n\x0cCrows at dusk\n\x0c"
}

#[test]
fn f3_on_a_pdf_opens_its_text_with_the_title_and_pages_first_and_hex_is_one_command_away() {
    let tools = Arc::new(FakeTools::default());
    tools.answer("pdftotext", pdftotext_answer());
    let (_dir, mut s) = open(KeyColumn::Terminal, tools.clone());
    s.focus("report.pdf");
    let said = s.key("f3");
    let title = preview_title("report.pdf");
    assert_eq!(title, "report.pdf, preview");
    let at = said.iter().position(|l| *l == title).expect("the title");
    assert_eq!(said[at + 1], "PDF, 3 pages, Example Report", "{said:?}");
    assert!(!said.iter().any(|l| l.contains("hex mode")), "{said:?}");
    assert_eq!(tools.ran(), ["pdftotext"]);
    let viewer = s.app.viewer().expect("the viewer is open");
    assert!(viewer.is_preview());
    assert_eq!(viewer.title(), "report.pdf, preview");
    assert_eq!(viewer.pages(), Some(3));
    let shown = lines(&mut s);
    assert_eq!(shown[0], "PDF, 3 pages, Example Report");
    assert!(shown.iter().any(|l| l == "Pages: 3"), "{shown:?}");
    assert!(
        shown.iter().any(|l| l == "Title: Example Report"),
        "{shown:?}"
    );
    let first = shown
        .iter()
        .position(|l| l == "Page 1")
        .unwrap_or_else(|| panic!("{shown:?}"));
    assert_eq!(shown[first + 1], "Crows at dawn");
    let last = shown.last().cloned().unwrap_or_default();
    assert!(last.ends_with(" shows the bytes in hex"), "{last}");
    // Hex: the bytes in hex mode, said; Hex again: the words, their title.
    let said = s.command(ids::VIEWER_HEX);
    assert_eq!(said, ["Hex mode: the bytes of report.pdf"]);
    let viewer = s.app.viewer().unwrap();
    assert!(!viewer.is_preview());
    assert_eq!(viewer.mode(), ViewerMode::Hex);
    let said = s.command(ids::VIEWER_HEX);
    assert_eq!(said, ["report.pdf, preview"]);
    assert!(s.app.viewer().unwrap().is_preview());
    // Reload reads the preview again.
    let said = s.command(ids::VIEWER_RELOAD);
    assert!(
        said.iter().any(|l| l == "Reloaded report.pdf, preview"),
        "{said:?}"
    );
    assert!(s.app.viewer().unwrap().is_preview());
    // Escape goes back to the row.
    s.key("escape");
    assert!(s.app.viewer().is_none());
    assert_eq!(s.focused(), "report.pdf");
}

#[test]
fn without_pdftotext_the_viewer_says_so_with_the_install_line_and_never_hex() {
    let (_dir, mut s) = open(KeyColumn::Terminal, Arc::new(FakeTools::default()));
    s.focus("report.pdf");
    let said = s.key("f3");
    let at = said
        .iter()
        .position(|l| l == "report.pdf, preview")
        .expect("the title");
    assert_eq!(said[at + 1], "No preview: pdftotext not found");
    assert!(said[at + 2].starts_with("Install: "), "{said:?}");
    assert!(!said.iter().any(|l| l.contains("hex mode")), "{said:?}");
    let shown = lines(&mut s);
    assert!(shown.iter().any(|l| l == "Pages: 3"), "{shown:?}");
}

#[test]
fn f3_on_a_word_document_opens_its_text() {
    let (_dir, mut s) = open(KeyColumn::Gui, Arc::new(FakeTools::default()));
    s.focus("notes.docx");
    let said = s.key("f3");
    let at = said
        .iter()
        .position(|l| l == "notes.docx, preview")
        .expect("the title");
    assert_eq!(said[at + 1], "DOCX, 1 heading", "{said:?}");
    let shown = lines(&mut s);
    assert!(shown.iter().any(|l| l == "Crows"), "{shown:?}");
    assert!(shown.iter().any(|l| l == "They meet at dawn."), "{shown:?}");
    assert!(s.app.take_picture_ask().is_none());
}

#[test]
fn a_plain_binary_file_keeps_hex() {
    let (_dir, mut s) = open(KeyColumn::Terminal, Arc::new(FakeTools::default()));
    s.focus("blob.bin");
    let said = s.key("f3");
    assert!(
        said.iter().any(|l| l == "blob.bin looks binary; hex mode"),
        "{said:?}"
    );
    let viewer = s.app.viewer().unwrap();
    assert!(!viewer.is_preview());
    assert_eq!(viewer.mode(), ViewerMode::Hex);
}

#[test]
fn in_the_terminal_a_picture_says_its_sentence_and_how_to_see_it() {
    let (_dir, mut s) = open(KeyColumn::Terminal, Arc::new(FakeTools::default()));
    s.focus("photo.png");
    let said = s.key("f3");
    let at = said
        .iter()
        .position(|l| l == "photo.png, preview")
        .expect("the title");
    assert!(said[at + 1].starts_with("PNG, 4 by 3"), "{said:?}");
    assert_eq!(said[at + 2], IN_THE_WINDOW);
    assert!(!said.iter().any(|l| l.contains("hex mode")), "{said:?}");
    assert!(s.app.take_picture_ask().is_none());
    let shown = lines(&mut s);
    assert!(
        shown.iter().any(|l| l.starts_with("Dimensions: ")),
        "{shown:?}"
    );
}

#[test]
fn in_the_window_enter_on_a_pdfs_words_asks_for_its_page_as_a_picture() {
    let tools = Arc::new(FakeTools::default());
    tools.answer("pdftotext", pdftotext_answer());
    let (_dir, mut s) = open(KeyColumn::Gui, tools);
    s.focus("report.pdf");
    let said = s.key("f3");
    assert!(said.contains(&page_hint_words("Enter")), "{said:?}");
    // The cursor on page 2's text, then Enter.
    let viewer = s.app.viewer_mut().unwrap();
    let line = (0..60)
        .find(|&n| viewer.line_text(n).as_deref() == Some("Crows at noon"))
        .unwrap();
    s.command(ids::VIEWER_JUMP);
    s.type_text(&(line + 1).to_string());
    s.key("enter");
    s.key("enter");
    let ask = s.app.take_picture_ask().expect("Enter asks for the page");
    assert_eq!(ask.from, PictureSource::Viewer);
    assert_eq!(
        ask.page,
        Some(PageAsk {
            page: 2,
            pages: Some(3)
        })
    );
}

#[test]
fn the_picture_helpers_run_imagemagick_and_pdftoppm_through_the_tools() {
    let png = png_bytes(2, 2);
    let tools = Arc::new(FakeTools::default());
    tools.answer("magick", &png);
    tools.answer("pdftoppm", &png);
    let (_dir, s) = open(KeyColumn::Gui, tools.clone());
    let helpers = s.app.picture_helpers();
    let path = Path::new("photo.heic");
    assert_eq!(helpers.magick_png(path, 1 << 20).unwrap(), png);
    assert_eq!(
        helpers
            .page_png(Path::new("report.pdf"), 2, 1 << 20)
            .unwrap(),
        png
    );
    assert_eq!(tools.ran(), ["magick", "pdftoppm"]);
    let runs = tools.runs.lock().unwrap().clone();
    let args: Vec<String> = runs[1]
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(&args[..4], ["-f", "2", "-l", "2"]);
    assert!(args.contains(&"-png".to_owned()), "{args:?}");
    // Without them: the words, then the install line.
    let (_dir, s) = open(KeyColumn::Gui, Arc::new(FakeTools::default()));
    let helpers = s.app.picture_helpers();
    let lines = helpers.magick_png(path, 1 << 20).unwrap_err();
    assert_eq!(lines[0], "Picture: not shown, needs ImageMagick");
    assert!(lines[1].starts_with("Install: "), "{lines:?}");
    let lines = helpers
        .page_png(Path::new("report.pdf"), 1, 1 << 20)
        .unwrap_err();
    assert_eq!(lines[0], "Page pictures need pdftoppm");
    assert!(lines[1].starts_with("Install: "), "{lines:?}");
}

#[test]
fn the_new_lines_fit_a_braille_line_with_their_meaning_first() {
    for line in [
        preview_title("Jimothy-at-night.png"),
        hex_hint("h"),
        hex_hint(""),
        diroxide_app::viewer::hex_face_line("report.pdf"),
        IN_THE_WINDOW.to_owned(),
        page_hint_words("Enter"),
        "No preview: pdftotext not found".to_owned(),
    ] {
        assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
    }
}
