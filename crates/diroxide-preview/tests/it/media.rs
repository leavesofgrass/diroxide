//! The ffprobe previewer against the real ffprobe, when it is installed:
//! a second of silence the test writes as a WAV file is probed, never
//! played. Without ffprobe the test says so and passes.

use std::sync::Arc;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::{LatestOnly, PreviewKind, PreviewRequest, Previews};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::{OpContext, Registry};

use crate::images::scratch;

/// One second of 8 kHz, 16-bit mono silence as a WAV file.
fn silence() -> Vec<u8> {
    let data = vec![0u8; 16_000];
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8_000u32.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

#[test]
fn the_real_ffprobe_reads_a_file_the_test_wrote() {
    let tools = ToolRegistry::builtin();
    if !tools.has("ffprobe") {
        println!("skipped: ffprobe not found");
        return;
    }
    let folder = scratch();
    let path = folder.path().join("silence.wav");
    std::fs::write(&path, silence()).unwrap();
    let latest = LatestOnly::new();
    let request = PreviewRequest {
        place: Place::parse(&path.display().to_string()).unwrap(),
        entry: Entry::new("silence.wav", EntryKind::File).with_size(16_044),
        generation: latest.next(),
        max_lines: 20,
    };
    let types = TypeRegistry::builtin();
    let ctx = OpContext::new().with_tools(Arc::new(tools));
    let preview = Previews::builtin()
        .render(
            &request,
            types.get("wav"),
            &latest,
            &Registry::with_local(),
            &ctx,
        )
        .unwrap();
    assert_eq!(preview.kind, PreviewKind::Media);
    assert_eq!(preview.summary, "WAV, 1 sec, 128 kbps, mono");
    assert!(
        preview
            .lines
            .contains(&"Audio: PCM, mono, 8 kHz".to_owned()),
        "{:?}",
        preview.lines
    );
}
