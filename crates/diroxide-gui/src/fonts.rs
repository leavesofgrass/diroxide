//! The interface fonts, compiled in: Atkinson Hyperlegible Next and Mono,
//! regular and bold (SIL Open Font License 1.1, `third_party/fonts`),
//! loaded straight into Parley's font collection so nothing is installed
//! on the system. Adapted from textweaver's
//! `crates/textweaver-xilem/src/fonts.rs` (the owner's own project).

use std::sync::Arc;

use masonry::peniko::Blob;

/// The font files, in the order they are registered.
pub const FILES: [&[u8]; 4] = [
    include_bytes!(
        "../../../third_party/fonts/atkinson-hyperlegible-next/AtkinsonHyperlegibleNext-Regular.ttf"
    ),
    include_bytes!(
        "../../../third_party/fonts/atkinson-hyperlegible-next/AtkinsonHyperlegibleNext-Bold.ttf"
    ),
    include_bytes!(
        "../../../third_party/fonts/atkinson-hyperlegible-mono/AtkinsonHyperlegibleMono-Regular.ttf"
    ),
    include_bytes!(
        "../../../third_party/fonts/atkinson-hyperlegible-mono/AtkinsonHyperlegibleMono-Bold.ttf"
    ),
];

/// Every compiled-in font, as blobs for Parley's font collection.
pub fn blobs() -> Vec<Blob<u8>> {
    FILES
        .iter()
        .map(|data| Blob::new(Arc::new(*data)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_font_file_is_a_true_type_font() {
        for data in FILES {
            assert!(data.len() > 10_000);
            assert_eq!(&data[..4], &[0, 1, 0, 0], "a TrueType font starts so");
        }
        assert_eq!(blobs().len(), 4);
    }
}
