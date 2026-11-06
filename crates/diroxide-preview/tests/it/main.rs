//! Integration tests of previews.

// Test helpers may unwrap: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod carta;
mod carta_paths;
mod documents;
mod encoding;
mod images;
mod markdown;
mod media;
mod text;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::{LatestOnly, PreviewKind, PreviewRequest, Previews};
use diroxide_vfs::{OpContext, Registry};

/// The slow tests' gate (`docs/dev/building.md`, "Slow tests"): a slow
/// test runs only when `DIRO_SLOW` is `1`, as CI and the full check set
/// it. Otherwise it says so in a line nextest shows and passes at once.
pub fn slow_test_skipped() -> bool {
    if std::env::var_os("DIRO_SLOW").is_some_and(|v| v == "1") {
        return false;
    }
    println!("skipped: slow, set DIRO_SLOW=1");
    true
}

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
