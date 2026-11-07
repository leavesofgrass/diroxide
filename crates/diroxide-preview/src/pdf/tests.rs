// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use diroxide_vfs::{CancelToken, ToolOutput, ToolRunner};

use super::*;

/// A PDF of pages, each with one line of text, and an Info dictionary
/// with a title and an author when given. Offsets are counted, so the
/// cross-reference table is true.
pub(crate) fn pdf(pages: &[&str], title: Option<&str>, author: Option<&str>) -> Vec<u8> {
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
    let mut info = String::from("<<");
    if let Some(t) = title {
        info.push_str(&format!(" /Title ({t})"));
    }
    if let Some(a) = author {
        info.push_str(&format!(" /Author ({a})"));
    }
    info.push_str(" /Producer (diroxide tests) >>");
    objects.push(info);
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

#[test]
fn the_properties_come_from_the_metadata_reader() {
    let pages: Vec<String> = (1..=12).map(|n| format!("Page {n} of crows")).collect();
    let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
    let bytes = pdf(&refs, Some("Example Report"), Some("Ada Example"));
    let props = properties(&bytes).unwrap();
    assert_eq!(props.pages, 12);
    assert_eq!(props.version, "1.4");
    assert!(!props.encrypted);
    let line = props.sentence("PDF", Some(1_200_000));
    assert_eq!(line, "PDF, 12 pages, Example Report");
    assert_eq!(line.chars().count(), 29);
    let said: Vec<String> = props.fields().iter().map(Field::line).collect();
    assert_eq!(
        said,
        [
            "Title: Example Report",
            "Author: Ada Example",
            "Pages: 12",
            "Written by: diroxide tests",
            "PDF version: 1.4"
        ]
    );
    let untitled = properties(&pdf(&["one"], None, None)).unwrap();
    assert_eq!(
        untitled.sentence("PDF", Some(1_200_000)),
        "PDF, 1 page, 1.2 MB"
    );
    assert!(properties(b"%PDF-1.4 cut short").is_none());
    assert_eq!(NOT_FOUND.chars().count(), 31);
}

#[test]
fn the_text_is_a_heading_a_page_then_the_pages_not_read() {
    let text = "Crows remember\nfaces.\n\nThey gather.\n\u{c}Second page\n\u{c}Third\n\u{c}";
    let view = view_of(text, 12);
    let shown: Vec<String> = view.rows().iter().map(crate::doc::DocRow::shown).collect();
    assert_eq!(
        shown,
        [
            "Page 1",
            "Crows remember faces.",
            "They gather.",
            "Page 2",
            "Second page",
            "Page 3",
            "Third",
            "and 9 more pages"
        ]
    );
    assert_eq!(view.outline().len(), 3);
}

#[test]
fn the_arguments_are_a_list_for_three_pages() {
    assert_eq!(
        arguments("D:/docs/a report.pdf"),
        [
            "-f",
            "1",
            "-l",
            "3",
            "-enc",
            "UTF-8",
            "-q",
            "D:/docs/a report.pdf",
            "-"
        ]
    );
}

/// A runner that has pdftotext and streams fixed lines.
struct Fake {
    lines: Vec<String>,
    seen: Mutex<Vec<ToolRequest>>,
}

impl ToolRunner for Fake {
    fn has(&self, tool: &str) -> bool {
        tool == PDFTOTEXT
    }

    fn run(&self, _: &ToolRequest, _: &CancelToken) -> Result<ToolOutput, ToolRunError> {
        Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new()))
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        _cancel: &CancelToken,
        record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        self.seen.lock().unwrap().push(request.clone());
        for line in &self.lines {
            if record(ToolStream::Stdout, line).is_break() {
                break;
            }
        }
        Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new()))
    }
}

#[test]
fn the_text_stops_at_its_cap_and_a_missing_tool_is_said() {
    let fake = Arc::new(Fake {
        lines: vec!["x".repeat(1_000); 200],
        seen: Mutex::new(Vec::new()),
    });
    let ctx = OpContext::new().with_tools(fake.clone());
    let kept = text(&ctx, "D:/a.pdf").unwrap();
    assert!(kept.len() <= MAX_TEXT_BYTES);
    assert_eq!(kept.lines().count(), 65);
    assert_eq!(fake.seen.lock().unwrap()[0].limits, LIMITS);
    assert_eq!(
        text(&OpContext::new(), "D:/a.pdf"),
        Err(TextError::NotFound)
    );
}

#[test]
fn the_previewer_says_the_properties_without_the_tool() {
    use crate::{LatestOnly, Previews};
    use diroxide_core::{Entry, Place};
    let folder = tempfile::Builder::new()
        .prefix("pdf-")
        .tempdir_in(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .unwrap();
    let path = folder.path().join("report.pdf");
    std::fs::write(&path, pdf(&["one", "two"], Some("Example Report"), None)).unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: Place::parse(&path.display().to_string()).unwrap(),
        entry: Entry::new("report.pdf", EntryKind::File).with_size(900),
        generation: latest.next(),
        max_lines: 20,
    };
    let types = diroxide_filetype::TypeRegistry::builtin();
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("pdf"),
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        )
        .unwrap();
    assert_eq!(preview.summary, NOT_FOUND);
    assert!(preview.lines.contains(&"Title: Example Report".to_owned()));
    assert!(preview.lines.contains(&"Pages: 2".to_owned()));
    let fake = Arc::new(Fake {
        lines: vec!["Crows remember faces.".to_owned(), "\u{c}Two".to_owned()],
        seen: Mutex::new(Vec::new()),
    });
    let ctx = OpContext::new().with_tools(fake);
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("pdf"),
            &latest,
            &Registry::with_local(),
            &ctx,
        )
        .unwrap();
    assert_eq!(preview.summary, "PDF, 2 pages, Example Report");
    assert_eq!(preview.lines[..2], ["Page 1", "Crows remember faces."]);
}
