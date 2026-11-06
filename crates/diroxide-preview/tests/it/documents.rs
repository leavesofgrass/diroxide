//! Documents through the previewers: the typed reader chosen by the
//! file's checked type, the rows in the preview.

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::{LatestOnly, PreviewKind, PreviewRequest, Previews};
use diroxide_vfs::{OpContext, Registry};

use crate::images::scratch;

/// The preview of a file of this name and content, checked as `type_id`.
pub fn preview_of(name: &str, content: &[u8], type_id: &str) -> diroxide_preview::Preview {
    let folder = scratch();
    let path = folder.path().join(name);
    std::fs::write(&path, content).unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: Place::parse(&path.display().to_string()).unwrap(),
        entry: Entry::new(name, EntryKind::File).with_size(content.len() as u64),
        generation: latest.next(),
        max_lines: 40,
    };
    let types = TypeRegistry::builtin();
    Previews::builtin()
        .render(
            &request,
            types.get(type_id),
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        )
        .unwrap()
}

#[test]
fn a_drawing_previews_as_its_reading() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512">
<title>Example Logo</title><desc>A crow on a lamp post.</desc>
<g><title>Crow</title></g><text>Portland</text></svg>"#;
    let preview = preview_of("logo.svg", svg, "svg");
    assert_eq!(preview.kind, PreviewKind::Document);
    assert_eq!(preview.summary, "SVG, 512 by 512, titled Example Logo");
    assert_eq!(
        preview.lines,
        ["Example Logo", "A crow on a lamp post.", "Crow", "Portland"]
    );
    let view = preview.document.unwrap();
    assert_eq!(view.outline().len(), 1);
    assert!(
        preview
            .details
            .iter()
            .any(|r| r.spoken() == "Contents: 1 heading")
    );
}

#[test]
fn markdown_previews_as_rows_and_a_broken_drawing_as_its_text() {
    let preview = preview_of(
        "notes.md",
        b"# Notes\n\nFirst.\n\n## More\n\nSecond.\n",
        "markdown",
    );
    assert_eq!(preview.summary, "Markdown, Notes, 2 headings");
    assert_eq!(preview.lines, ["Notes", "First.", "More", "Second."]);
    let broken = preview_of("broken.svg", b"<svg><title>cut", "svg");
    assert_eq!(broken.kind, PreviewKind::Document);
    assert!(
        broken
            .details
            .iter()
            .any(|r| r.spoken() == "Read as: text, not valid SVG")
    );
    assert_eq!(broken.lines, ["<svg><title>cut"]);
}
