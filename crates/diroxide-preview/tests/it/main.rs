//! Integration tests of previews.

mod encoding;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::{LatestOnly, PreviewKind, PreviewRequest, Previews};
use diroxide_vfs::{OpContext, Registry};

#[test]
fn a_manifest_previews_as_text_with_its_type() {
    let place = Place::parse(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .join("Cargo.toml")
        .unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place,
        entry: Entry::new("Cargo.toml", EntryKind::File).with_size(10),
        generation: latest.next(),
        max_lines: 1,
    };
    let types = TypeRegistry::builtin();
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("text"),
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        )
        .unwrap();
    assert_eq!(preview.kind, PreviewKind::Text);
    assert_eq!(preview.summary, "text file, 10 bytes");
}
