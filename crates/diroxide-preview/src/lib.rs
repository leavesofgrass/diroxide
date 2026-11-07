//! Previews, text first, because the owner cannot see a thumbnail and text
//! is what a Braille line shows. The details view says what a file is in
//! words ("PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB"); text shows as text;
//! anything else can show as hex. A preview never blocks, and a newer
//! request cancels an older one ([`LatestOnly`]).
//!
//! Phase 0: text and hex previewers over the provider, and a details line
//! from the entry and its type. The alpha.2 contract step adds the token
//! classes and their per-line runs ([`classes`]), the entry's detail rows
//! on every preview ([`detail_rows`]), and serialization, so a preview
//! can cross a process boundary (a plugin's previewer, `diro serve`).
//! Wave 2 brings the viewer's modules (W2d): [`encoding`], carried from
//! textweaver, which decides a file's encoding and tells text from
//! binary; [`hex`], the one hex row function every place that shows bytes
//! uses; [`highlight`], syntect as a tokenizer into the token classes,
//! with its caps, its worker and its deadline, the symbols and the
//! validity lines. The quick view's [`doc`] modules (W2e): the row-based
//! document view the quick view moves through, and its readers. Alpha 4
//! (R30 commits 33 to 40) adds the typed previewers in front of the text
//! previewer: [`images`], pictures in words from their headers alone;
//! [`documents`], every format the quick view's [`doc`] readers read
//! (Markdown, carta's formats, SVG drawings), as rows; [`media`], sound
//! and video in words through ffprobe; [`archive`], an archive's members
//! counted and listed, never extracted; [`pdf`], a PDF's properties and
//! its first pages' text through pdftotext.

pub mod archive;
pub mod classes;
pub mod doc;
pub mod documents;
pub mod encoding;
pub mod fuzzing;
pub mod hex;
pub mod highlight;
pub mod images;
pub mod media;
pub mod pdf;
pub mod words;

use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicU64, Ordering};

use diroxide_core::format::human_size;
use diroxide_core::{DetailGroup, DetailRow, Entry, EntryKind, Explain, Place};
use diroxide_filetype::FileType;
use diroxide_vfs::{OpContext, ReadSeek, Registry, VfsError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use classes::{ClassRun, TokenClass};

/// How many bytes a text or hex preview reads at most.
pub const MAX_PREVIEW_BYTES: usize = 64 * 1024;

/// What kind of preview this is.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewKind {
    /// Lines of text.
    Text,
    /// Lines of hex.
    Hex,
    /// A folder's summary.
    Folder,
    /// Only the details line.
    Details,
    /// A picture in words: the sentence, then its rows ([`images`]).
    Image,
    /// A document read into rows ([`documents`]); the view travels in
    /// [`Preview::document`].
    Document,
    /// Sound or moving pictures in words, through ffprobe ([`media`]).
    Media,
    /// An archive's members, counted and listed ([`archive`]).
    Listing,
}

/// A preview: a details line, then lines to read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    /// What kind of preview.
    pub kind: PreviewKind,
    /// What the entry is, in words, meaning first.
    pub summary: String,
    /// The lines to show and read.
    pub lines: Vec<String>,
    /// The facts about the entry, as detail rows in their groups' order:
    /// what alpha.2's item information and alpha.7's details view list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<DetailRow>,
    /// For each line of [`Preview::lines`], its token class runs, when the
    /// preview was highlighted; empty when it was not (plain text, hex,
    /// screen reader mode with highlighting off, a file over the caps).
    /// When present it has one entry per line, an empty one for a line
    /// with no runs ([`Preview::runs_of`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<Vec<ClassRun>>,
    /// The document's rows with their outline, tables and links, for a
    /// document preview, so the quick view moves by heading and table
    /// (alpha 4); `None` for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<doc::DocView>,
}

impl Preview {
    /// A preview of a kind with its summary and nothing else.
    pub fn new(kind: PreviewKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
            lines: Vec::new(),
            details: Vec::new(),
            runs: Vec::new(),
            document: None,
        }
    }

    /// The runs of one line: empty when the preview is not highlighted or
    /// the line has none.
    pub fn runs_of(&self, line: usize) -> &[ClassRun] {
        self.runs.get(line).map_or(&[], Vec::as_slice)
    }

    /// Whether the preview carries token classes.
    pub fn is_highlighted(&self) -> bool {
        !self.runs.is_empty()
    }

    /// Whether the runs match the lines: none at all, or one list per line
    /// whose runs fit it ([`classes::runs_fit`]).
    pub fn runs_fit(&self) -> bool {
        self.runs.is_empty()
            || (self.runs.len() == self.lines.len()
                && self
                    .lines
                    .iter()
                    .zip(&self.runs)
                    .all(|(line, runs)| classes::runs_fit(line, runs)))
    }
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
    /// The file is past what a preview reads where its place cannot be
    /// read at random.
    #[error("it is too large to preview here")]
    TooLarge,
}

impl Explain for PreviewError {
    fn next_step(&self) -> String {
        match self {
            PreviewError::Vfs(inner) => inner.next_step(),
            PreviewError::Superseded | PreviewError::NoPreviewer | PreviewError::TooLarge => {
                String::new()
            }
        }
    }
}

/// Makes previews of some kinds of entry.
pub trait Previewer: Send + Sync {
    /// The previewer's name, for Settings.
    fn name(&self) -> &str;

    /// The previewer's version: part of a cached preview's key, raised
    /// when what it says about a file changes.
    fn version(&self) -> u32 {
        1
    }

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

/// The facts every preview carries about its entry, as detail rows in
/// their groups' order (core's `details` module): the type ("Type: PDF
/// document"), the size in words with the exact bytes ("Size: 48 KB
/// (49,152 bytes)"), and a link's target with hidden characters spelled
/// out. The item information list shows them; a format's own reader adds
/// its rows after these (alpha.7).
pub fn detail_rows(entry: &Entry, file_type: Option<&FileType>) -> Vec<DetailRow> {
    let what = match (entry.kind, file_type) {
        (EntryKind::Folder, _) => "folder".to_owned(),
        (_, Some(t)) => t.name.clone(),
        (kind, None) => kind.word().to_owned(),
    };
    let mut rows = vec![DetailRow::new("type", "Type", what, DetailGroup::Identity)];
    if let Some(size) = entry.size.filter(|_| entry.kind != EntryKind::Folder) {
        rows.push(DetailRow::new(
            "size",
            "Size",
            size_words(size),
            DetailGroup::Size,
        ));
    }
    if let Some(link) = &entry.link {
        rows.push(DetailRow::new(
            "link.target",
            "Link to",
            diroxide_core::names::spell_out(&link.target),
            DetailGroup::Place,
        ));
    }
    diroxide_core::details::order_rows(&mut rows);
    rows
}

/// A size in words with its exact bytes after: "48 KB (49,152 bytes)", or
/// "512 bytes" when the words already are exact.
pub fn size_words(bytes: u64) -> String {
    let words = human_size(bytes);
    if bytes < 1000 {
        return words;
    }
    format!("{words} ({} bytes)", diroxide_core::format::grouped(bytes))
}

/// Hex lines: an offset, sixteen bytes, and the printable characters,
/// through the shared row function ([`hex::hex_row`]): "000000: 50 4b
/// 03 04 ... PK..".
pub fn hex_lines(bytes: &[u8], start_offset: u64) -> Vec<String> {
    let layout = hex::HexLayout::for_len(start_offset + bytes.len() as u64, 16);
    hex::hex_rows(bytes, start_offset, layout)
}

/// Opens an entry for random access: its provider's seekable read, or,
/// where the provider has none, at most `cap` bytes read into memory
/// ([`PreviewError::TooLarge`] past them).
pub fn open_source(
    request: &PreviewRequest,
    registry: &Registry,
    ctx: &OpContext,
    cap: u64,
) -> Result<Box<dyn ReadSeek>, PreviewError> {
    let provider = registry.resolve(&request.place)?;
    match provider.open_seekable(&request.place, ctx) {
        Ok(source) => Ok(source),
        Err(VfsError::Unsupported { .. }) => {
            let reader = provider.read(&request.place, ctx)?;
            let mut bytes = Vec::new();
            reader
                .take(cap.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(VfsError::from)?;
            ctx.check()?;
            if bytes.len() as u64 > cap {
                return Err(PreviewError::TooLarge);
            }
            Ok(Box::new(Cursor::new(bytes)))
        }
        Err(error) => Err(error.into()),
    }
}

/// A preview that is its rows: the summary, then every row in the order
/// of facts, the entry's own first, each said as a line ("Dimensions:
/// 1920 by 1080").
pub(crate) fn details_preview(
    kind: PreviewKind,
    summary: String,
    base: Vec<DetailRow>,
    more: impl IntoIterator<Item = DetailRow>,
) -> Preview {
    let mut details = base;
    details.extend(more);
    diroxide_core::details::order_rows(&mut details);
    Preview {
        kind,
        summary,
        lines: details.iter().map(DetailRow::spoken).collect(),
        details,
        runs: Vec::new(),
        document: None,
    }
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
            details: detail_rows(&request.entry, file_type),
            runs: Vec::new(),
            document: None,
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
            details: detail_rows(&request.entry, file_type),
            runs: Vec::new(),
            document: None,
        })
    }
}

/// The previewers, tried in order; the details line when none takes it.
pub struct Previews {
    previewers: Vec<Box<dyn Previewer>>,
}

impl Previews {
    /// The built-in previewers: the typed ones first, then text, then
    /// hex.
    pub fn builtin() -> Self {
        Self {
            previewers: vec![
                Box::new(images::ImagePreviewer::default()),
                Box::new(media::MediaPreviewer),
                Box::new(archive::ArchivePreviewer),
                Box::new(pdf::PdfPreviewer::default()),
                Box::new(documents::DocPreviewer::default()),
                Box::new(TextPreviewer),
                Box::new(HexPreviewer),
            ],
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
                details: detail_rows(&request.entry, file_type),
                ..Preview::new(
                    if request.entry.kind == EntryKind::Folder {
                        PreviewKind::Folder
                    } else {
                        PreviewKind::Details
                    },
                    details(&request.entry, file_type),
                )
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
    fn a_preview_carries_runs_and_rows_and_travels_as_json() {
        let entry = Entry::new("main.rs", EntryKind::File).with_size(49_152);
        let mut preview = Preview::new(PreviewKind::Text, "Rust source, 49.2 KB");
        preview.lines = vec!["fn main() {".to_owned(), "}".to_owned()];
        preview.runs = vec![
            vec![
                ClassRun::new(0, 2, TokenClass::Keyword),
                ClassRun::new(3, 7, TokenClass::FunctionName),
            ],
            Vec::new(),
        ];
        preview.details = detail_rows(&entry, None);
        assert!(preview.is_highlighted() && preview.runs_fit());
        assert_eq!(preview.runs_of(0).len(), 2);
        assert!(preview.runs_of(1).is_empty() && preview.runs_of(9).is_empty());
        let json = serde_json::to_string(&preview).unwrap();
        let back: Preview = serde_json::from_str(&json).unwrap();
        assert_eq!(back, preview);
        assert!(json.contains(r#""class":"function-name""#), "{json}");
        let plain = Preview::new(PreviewKind::Hex, "a.bin, shown as hex");
        let json = serde_json::to_string(&plain).unwrap();
        assert!(
            !json.contains("runs") && !json.contains("details"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Preview>(&json).unwrap(), plain);
        assert!(plain.runs_fit() && !plain.is_highlighted());
        preview.runs.pop();
        assert!(!preview.runs_fit(), "one list of runs per line");
    }

    #[test]
    fn every_preview_lists_the_entrys_facts_label_first() {
        let file = Entry::new("a.bin", EntryKind::File).with_size(49_152);
        let spoken: Vec<String> = detail_rows(&file, None)
            .iter()
            .map(DetailRow::spoken)
            .collect();
        assert_eq!(spoken, ["Type: file", "Size: 49.2 KB (49,152 bytes)"]);
        let folder = Entry::new("src", EntryKind::Folder).with_size(4_096);
        assert_eq!(detail_rows(&folder, None).len(), 1, "no size for a folder");
        let mut link = Entry::new("latest", EntryKind::Link);
        link.link = Some(diroxide_core::Link {
            target: "v2\u{202E}".to_owned(),
            points_to: None,
        });
        let rows = detail_rows(&link, None);
        assert_eq!(rows[1].spoken(), "Link to: v2[U+202E]");
        assert_eq!(size_words(512), "512 bytes");
        assert_eq!(size_words(1_234_567_890), "1.2 GB (1,234,567,890 bytes)");
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
        assert_eq!(preview.details[0].spoken(), "Type: file");
    }

    #[test]
    fn hex_lines_show_offsets_bytes_and_text() {
        let lines = hex_lines(b"PK\x03\x04hello", 0);
        let padding = " ".repeat(7 * 3);
        assert_eq!(
            lines,
            vec![format!(
                "000000: 50 4b 03 04 68 65 6c 6c 6f {padding}PK..hello"
            )]
        );
        let entry = Entry::new("a.bin", EntryKind::File).with_size(2_100_000);
        assert_eq!(details(&entry, None), "file, 2.1 MB");
    }
}
