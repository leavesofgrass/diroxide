//! The PDF previewer against the real pdftotext, when it is installed:
//! a one-page PDF the test writes, its text and its properties read.
//! Without pdftotext the test says so and passes.

use std::sync::Arc;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::{LatestOnly, PreviewKind, PreviewRequest, Previews};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::{OpContext, Registry};

use crate::images::scratch;

/// A one-page PDF saying `line`, titled, with a true cross-reference
/// table.
fn one_page(line: &str, title: &str) -> Vec<u8> {
    let content = format!("BT /F1 18 Tf 72 700 Td ({line}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Title ({title}) >>"),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (n, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", n + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

#[test]
fn the_real_pdftotext_reads_a_page_the_test_wrote() {
    let tools = ToolRegistry::builtin();
    if !tools.has("pdftotext") {
        println!("skipped: pdftotext not found");
        return;
    }
    let folder = scratch();
    let path = folder.path().join("report.pdf");
    let bytes = one_page("Crows remember faces", "Example Report");
    std::fs::write(&path, &bytes).unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: Place::parse(&path.display().to_string()).unwrap(),
        entry: Entry::new("report.pdf", EntryKind::File).with_size(bytes.len() as u64),
        generation: latest.next(),
        max_lines: 20,
    };
    let types = TypeRegistry::builtin();
    let ctx = OpContext::new().with_tools(Arc::new(tools));
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("pdf"),
            &latest,
            &Registry::with_local(),
            &ctx,
        )
        .unwrap();
    assert_eq!(preview.kind, PreviewKind::Document);
    assert_eq!(preview.summary, "PDF, 1 page, Example Report");
    assert_eq!(
        preview.lines[..2],
        ["Page 1", "Crows remember faces"],
        "{:?}",
        preview.lines
    );
}
