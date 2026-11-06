//! The image words through the previewers, on a file on disk.

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::{LatestOnly, PreviewKind, PreviewRequest, Previews};
use diroxide_vfs::{OpContext, Registry};
use image::ImageEncoder;

/// A scratch folder under the build's own folder, never the system's.
pub fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("preview-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

#[test]
fn a_picture_on_disk_previews_as_its_sentence_and_rows() {
    let folder = scratch();
    let path = folder.path().join("chart.png");
    let pixels = vec![0u8; 1920 * 1080 * 4];
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixels, 1920, 1080, image::ExtendedColorType::Rgba8)
        .unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: Place::parse(&path.display().to_string()).unwrap(),
        entry: Entry::new("chart.png", EntryKind::File).with_size(2_300_000),
        generation: latest.next(),
        max_lines: 40,
    };
    let types = TypeRegistry::builtin();
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("png"),
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        )
        .unwrap();
    assert_eq!(preview.kind, PreviewKind::Image);
    assert_eq!(preview.summary, "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB");
    assert_eq!(
        preview.lines,
        [
            "Type: PNG image",
            "Dimensions: 1920 by 1080",
            "Shape: landscape, about 16 to 9",
            "Color: full color, 8 bits, transparency",
            "Size: 2.3 MB (2,300,000 bytes)",
        ]
    );
    for line in &preview.lines {
        assert!(line.chars().count() <= 40, "{line}");
    }
    // Text named as a picture is said as a damaged picture, not shown.
    let fake = folder.path().join("fake.png");
    std::fs::write(&fake, b"just some words").unwrap();
    let request = PreviewRequest {
        place: Place::parse(&fake.display().to_string()).unwrap(),
        entry: Entry::new("fake.png", EntryKind::File).with_size(15),
        generation: latest.next(),
        max_lines: 40,
    };
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("png"),
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        )
        .unwrap();
    assert_eq!(preview.summary, "PNG, unreadable: not a picture");
}
