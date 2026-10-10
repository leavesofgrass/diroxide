//! Previews, text first, because the owner cannot see a thumbnail and text
//! is what a Braille line shows. The details view says what a file is in
//! words ("PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB"); text shows as text;
//! anything else can show as hex. A preview never blocks, and a newer
//! request cancels an older one ([`LatestOnly`]).
//!
//! Phase 0: text and hex previewers over the provider, and a details line
//! from the entry and its type. Wave 5 (W5b) owns this crate and adds the
//! image, audio, video, document and archive previewers.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};

use diroxide_core::format::human_size;
use diroxide_core::{Entry, EntryKind, Explain, Place};
use diroxide_filetype::FileType;
use diroxide_vfs::{OpContext, Registry, VfsError};
use thiserror::Error;

/// How many bytes a text or hex preview reads at most.
pub const MAX_PREVIEW_BYTES: usize = 64 * 1024;

/// What kind of preview this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    /// Lines of text.
    Text,
    /// Lines of hex.
    Hex,
    /// A folder's summary.
    Folder,
    /// Only the details line.
    Details,
}

/// A preview: a details line, then lines to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// What kind of preview.
    pub kind: PreviewKind,
    /// What the entry is, in words, meaning first.
    pub summary: String,
    /// The lines to show and read.
    pub lines: Vec<String>,
}

/// A preview to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRequest {
    /// The entry's place.
    pub place: Place,
    /// The entry.
    pub entry: Entry,
    /// The request's generation, from [`LatestOnly::next`].
    pub generation: u64,
    /// The most lines wanted.
    pub max_lines: usize,
}

/// Why a preview could not be made.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PreviewError {
    /// The provider failed.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// A newer request replaced this one.
    #[error("a newer preview replaced it")]
    Superseded,
    /// No previewer takes this entry.
    #[error("there is no preview for this kind of file")]
    NoPreviewer,
}

impl Explain for PreviewError {
    fn next_step(&self) -> String {
        match self {
            PreviewError::Vfs(inner) => inner.next_step(),
            PreviewError::Superseded | PreviewError::NoPreviewer => String::new(),
        }
    }
}

/// Makes previews of some kinds of entry.
pub trait Previewer: Send + Sync {
    /// The previewer's name, for Settings.
    fn name(&self) -> &str;

    /// Whether it takes this entry.
    fn accepts(&self, request: &PreviewRequest, file_type: Option<&FileType>) -> bool;

    /// Makes the preview, checking the context's token between reads.
    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError>;
}

/// Hands out generations so that only the newest preview request is kept.
#[derive(Debug, Default)]
pub struct LatestOnly(AtomicU64);

impl LatestOnly {
    /// A new counter.
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// The next generation; every earlier one is now stale.
    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Whether a generation is still the newest.
    pub fn is_current(&self, generation: u64) -> bool {
        self.0.load(Ordering::Relaxed) == generation
    }
}

/// The details line for an entry: its type, then its size. "PDF document,
/// 2.1 MB"; "folder"; "file, 0 bytes".
pub fn details(entry: &Entry, file_type: Option<&FileType>) -> String {
    let what = match (entry.kind, file_type) {
        (EntryKind::Folder, _) => return "folder".to_owned(),
        (_, Some(t)) => t.name.clone(),
        (kind, None) => kind.word().to_owned(),
    };
    match entry.size {
        Some(size) => format!("{what}, {}", human_size(size)),
        None => what,
    }
}

/// Hex lines: an offset, sixteen bytes, and the printable characters.
/// "00000000  50 4b 03 04  PK..".
pub fn hex_lines(bytes: &[u8], start_offset: u64) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, chunk)| {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let text: String = chunk
                .iter()
                .map(|&b| {
                    if b.is_ascii_graphic() || b == b' ' {
                        char::from(b)
                    } else {
                        '.'
                    }
                })
                .collect();
            format!(
                "{:08x}  {}  {}",
                start_offset + i as u64 * 16,
                hex.join(" "),
                text
            )
        })
        .collect()
}

fn read_head(
    request: &PreviewRequest,
    registry: &Registry,
    ctx: &OpContext,
) -> Result<Vec<u8>, PreviewError> {
    let provider = registry.resolve(&request.place)?;
    let reader = provider.read(&request.place, ctx)?;
    let mut bytes = Vec::new();
    reader
        .take(MAX_PREVIEW_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(VfsError::from)?;
    ctx.check()?;
    Ok(bytes)
}

/// Shows files as text when they look like text.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextPreviewer;

impl Previewer for TextPreviewer {
    fn name(&self) -> &str {
        "text"
    }

    fn accepts(&self, request: &PreviewRequest, _file_type: Option<&FileType>) -> bool {
        matches!(request.entry.kind, EntryKind::File)
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let bytes = read_head(request, registry, ctx)?;
        if bytes.contains(&0) {
            return HexPreviewer.render(request, file_type, registry, ctx);
        }
        let text = String::from_utf8_lossy(&bytes);
        Ok(Preview {
            kind: PreviewKind::Text,
            summary: details(&request.entry, file_type),
            lines: text
                .lines()
                .take(request.max_lines)
                .map(str::to_owned)
                .collect(),
        })
    }
}

/// Shows any file as hex.
#[derive(Clone, Copy, Debug, Default)]
pub struct HexPreviewer;

impl Previewer for HexPreviewer {
    fn name(&self) -> &str {
        "hex"
    }

    fn accepts(&self, request: &PreviewRequest, _file_type: Option<&FileType>) -> bool {
        matches!(request.entry.kind, EntryKind::File | EntryKind::Archive)
    }

    fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        let bytes = read_head(request, registry, ctx)?;
        let mut lines = hex_lines(&bytes, 0);
        lines.truncate(request.max_lines);
        Ok(Preview {
            kind: PreviewKind::Hex,
            summary: format!("{}, shown as hex", details(&request.entry, file_type)),
            lines,
        })
    }
}

/// The previewers, tried in order; the details line when none takes it.
pub struct Previews {
    previewers: Vec<Box<dyn Previewer>>,
}

impl Previews {
    /// The built-in previewers: text, then hex.
    pub fn builtin() -> Self {
        Self {
            previewers: vec![Box::new(TextPreviewer), Box::new(HexPreviewer)],
        }
    }

    /// Adds a previewer before the built-in ones, as a plugin's would be.
    pub fn add_first(&mut self, previewer: Box<dyn Previewer>) {
        self.previewers.insert(0, previewer);
    }

    /// Makes the preview with the first previewer that takes the entry; a
    /// stale generation gives [`PreviewError::Superseded`].
    pub fn render(
        &self,
        request: &PreviewRequest,
        file_type: Option<&FileType>,
        latest: &LatestOnly,
        registry: &Registry,
        ctx: &OpContext,
    ) -> Result<Preview, PreviewError> {
        if !latest.is_current(request.generation) {
            return Err(PreviewError::Superseded);
        }
        match self
            .previewers
            .iter()
            .find(|p| p.accepts(request, file_type))
        {
            Some(previewer) => previewer.render(request, file_type, registry, ctx),
            None => Ok(Preview {
                kind: if request.entry.kind == EntryKind::Folder {
                    PreviewKind::Folder
                } else {
                    PreviewKind::Details
                },
                summary: details(&request.entry, file_type),
                lines: Vec::new(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_for(name: &str, kind: EntryKind, generation: u64) -> PreviewRequest {
        let place = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let place = if kind == EntryKind::Folder {
            place
        } else {
            place.join(name).unwrap()
        };
        PreviewRequest {
            place,
            entry: Entry::new(name, kind),
            generation,
            max_lines: 3,
        }
    }

    #[test]
    fn text_files_preview_as_text() {
        let latest = LatestOnly::new();
        let request = request_for("Cargo.toml", EntryKind::File, latest.next());
        let preview = Previews::builtin()
            .render(
                &request,
                None,
                &latest,
                &Registry::with_local(),
                &OpContext::new(),
            )
            .unwrap();
        assert_eq!(preview.kind, PreviewKind::Text);
        assert_eq!(preview.lines[0], "[package]");
        assert_eq!(preview.lines.len(), 3);
    }

    #[test]
    fn an_older_request_is_superseded() {
        let latest = LatestOnly::new();
        let old = request_for("Cargo.toml", EntryKind::File, latest.next());
        latest.next();
        let result = Previews::builtin().render(
            &old,
            None,
            &latest,
            &Registry::with_local(),
            &OpContext::new(),
        );
        assert_eq!(result, Err(PreviewError::Superseded));
    }

    #[test]
    fn folders_get_a_details_line() {
        let latest = LatestOnly::new();
        let request = request_for("src", EntryKind::Folder, latest.next());
        let preview = Previews::builtin()
            .render(
                &request,
                None,
                &latest,
                &Registry::with_local(),
                &OpContext::new(),
            )
            .unwrap();
        assert_eq!(
            (preview.kind, preview.summary.as_str()),
            (PreviewKind::Folder, "folder")
        );
    }

    #[test]
    fn hex_lines_show_offsets_bytes_and_text() {
        let lines = hex_lines(b"PK\x03\x04hello", 0);
        assert_eq!(
            lines,
            vec!["00000000  50 4b 03 04 68 65 6c 6c 6f  PK..hello".to_owned()]
        );
        let entry = Entry::new("a.bin", EntryKind::File).with_size(2_100_000);
        assert_eq!(details(&entry, None), "file, 2.1 MB");
    }
}
