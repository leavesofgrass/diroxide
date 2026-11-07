//! The quick view's render worker: one thread for the quick view's life,
//! started on its first render, that reads the previewed file and only
//! that file, at most the cap, and turns it into a document view. A
//! request whose generation is no longer the newest ([`LatestOnly`]) is
//! dropped before it reads; a newer move cancels the read in flight
//! through its token. Markdown and carta run under the preview crate's
//! guard, so a hostile file is cut off at the deadline.
//!
//! Alpha 4 (R30 commit 43, lane VL's readers): a file goes to the first
//! of the preview crate's previewers that takes it, by its checked type
//! first and its name when the type is not known. Pictures are said in
//! words from their headers and archives listed, at once; sound, video
//! and a PDF's text go through ffprobe and pdftotext, the tool-backed
//! previewers, after the wait only when auto-preview is on, and always on
//! the Preview command; otherwise the quick view says the preview waits
//! for it ("Preview on request: song.mp3, MP3 audio, 4.1 MB"). The Office
//! and OpenDocument packages and the old Excel workbooks are read whole
//! through the container read path with its caps
//! (`quick_view.max_document_mb`); RTF goes to its reader before the
//! sniff that names its signature binary; everything else is read as
//! before, at most `max_kib`, by the reader its type names.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::{FileType, TypeRegistry};
use diroxide_preview::doc::package::ContainerCaps;
use diroxide_preview::doc::{self, DocError, DocKind, DocView, ReadOptions};
use diroxide_preview::encoding::{Choice, Encoding, SNIFF_BYTES, binary_kind, choose, choose_head};
use diroxide_preview::{LatestOnly, Preview, PreviewError, PreviewRequest, Previews};
use diroxide_vfs::{CancelToken, OpContext, Registry, ToolRunner, VfsError};

/// The bytes read between two checks of the cancel token.
const CHUNK: usize = 64 * 1024;

/// The most rows a typed previewer gives the quick view.
const MAX_ROWS: usize = 10_000;

/// The previewers that run a helper program: ffprobe and pdftotext.
const TOOL_BACKED: [&str; 2] = ["media", "pdf"];

/// The previewers the quick view hands a file to before its own reading.
const TYPED: [&str; 4] = ["image", "archive", "media", "pdf"];

/// What the worker needs, shared with its thread.
pub(super) struct Shared {
    pub registry: Registry,
    pub types: TypeRegistry,
    pub latest: Arc<LatestOnly>,
    pub read: ReadOptions,
    pub max_bytes: u64,
    /// The container read path's caps, its container cap from
    /// `quick_view.max_document_mb`.
    pub caps: ContainerCaps,
    /// The preview crate's previewers, with the app's shared cache.
    pub previews: Previews,
    /// The runner ffprobe and pdftotext go through.
    pub tools: Arc<dyn ToolRunner>,
    /// Whether the tool-backed previewers run after the wait
    /// (auto-preview).
    pub auto: bool,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("max_bytes", &self.max_bytes)
            .field("auto", &self.auto)
            .finish_non_exhaustive()
    }
}

/// One render to do.
#[derive(Debug)]
pub(super) struct Request {
    pub generation: u64,
    pub place: Place,
    pub entry: Entry,
    /// Ask the provider for the entry's size again: the file changed.
    pub refresh: bool,
    pub cancel: CancelToken,
    /// The encoding the owner chose for this file (lane TA).
    pub encoding: Option<&'static Encoding>,
    /// Run the tool-backed previewers whatever auto-preview says: the
    /// Preview command asked.
    pub with_tools: bool,
}

/// What a render made.
#[derive(Debug)]
pub(super) enum Rendered {
    /// The rows, and why the format's reader could not read it when the
    /// text is shown instead. The view is boxed, so the answer stays
    /// small (VL's request 3).
    Document {
        view: Box<DocView>,
        error: Option<DocError>,
        /// Whether the cap stopped the read short of the file's end.
        capped: bool,
        /// What the automatic choice finds for the bytes' encoding, for
        /// the encoding chooser (lane TA).
        choice: Option<Choice>,
        /// A typed previewer's own line, said in place of the view's:
        /// "Preview: photo.png, PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB",
        /// "No preview: song.mp3, ffprobe not found".
        line: Option<String>,
    },
    /// Nothing to show for this kind of file yet: the details in words
    /// ("PNG image, 2.0 MB").
    NoPreview(String),
    /// A tool-backed preview waiting for the Preview command, since
    /// auto-preview is off: the details in words ("MP3 audio, 4.1 MB").
    OnRequest(String),
    /// The format's reader refused it: a package past its caps, or one
    /// that is not what its name says.
    NotRead(DocError),
    /// The provider could not read it.
    Failed(VfsError),
    /// A newer request replaced it.
    Superseded,
}

/// A finished render.
#[derive(Debug)]
pub(super) struct Answer {
    pub generation: u64,
    pub place: Place,
    pub entry: Entry,
    pub rendered: Rendered,
}

/// The worker thread and its two channels.
#[derive(Debug)]
pub(super) struct Worker {
    shared: Arc<Shared>,
    to: Option<Sender<Request>>,
    from: Option<Receiver<Answer>>,
    threads: Arc<AtomicUsize>,
}

/// Counts the worker thread while it lives.
struct Alive(Arc<AtomicUsize>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Worker {
    pub fn new(shared: Shared) -> Self {
        Self {
            shared: Arc::new(shared),
            to: None,
            from: None,
            threads: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// Replaces what the worker reads with; the thread starts again on
    /// the next request.
    pub fn reset(&mut self, shared: Shared) {
        self.to = None;
        self.from = None;
        self.shared = Arc::new(shared);
    }

    /// The worker threads alive: one once a render was asked for.
    pub fn threads(&self) -> usize {
        self.threads.load(Ordering::SeqCst)
    }

    /// Whether the thread is there to take requests.
    pub fn is_running(&self) -> bool {
        self.to.is_some()
    }

    /// Hands a request to the thread, starting it when it is not running.
    /// A request that cannot be handed over comes back as an answer.
    pub fn send(&mut self, request: Request) -> Option<Answer> {
        if self.to.is_none() && !self.start() {
            return Some(Answer {
                generation: request.generation,
                place: request.place,
                entry: request.entry,
                rendered: Rendered::Failed(VfsError::from(std::io::Error::other(
                    "the preview could not start",
                ))),
            });
        }
        let to = self.to.as_ref()?;
        match to.send(request) {
            Ok(()) => None,
            Err(mpsc::SendError(request)) => {
                self.to = None;
                self.from = None;
                Some(Answer {
                    generation: request.generation,
                    place: request.place,
                    entry: request.entry,
                    rendered: Rendered::Failed(VfsError::from(std::io::Error::other(
                        "the preview stopped",
                    ))),
                })
            }
        }
    }

    /// The answers ready now, in order.
    pub fn answers(&mut self) -> Vec<Answer> {
        let mut out = Vec::new();
        let Some(from) = self.from.as_ref() else {
            return out;
        };
        loop {
            match from.try_recv() {
                Ok(answer) => out.push(answer),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.to = None;
                    self.from = None;
                    break;
                }
            }
        }
        out
    }

    fn start(&mut self) -> bool {
        let (to, requests) = mpsc::channel::<Request>();
        let (answers, from) = mpsc::channel::<Answer>();
        let shared = Arc::clone(&self.shared);
        let threads = Arc::clone(&self.threads);
        threads.fetch_add(1, Ordering::SeqCst);
        let alive = Alive(Arc::clone(&threads));
        let spawned = thread::Builder::new()
            .name("diroxide-quick-view".to_owned())
            .spawn(move || {
                let _alive = alive;
                while let Ok(request) = requests.recv() {
                    let mut entry = request.entry.clone();
                    let rendered = if shared.latest.is_current(request.generation) {
                        render(&shared, &request, &mut entry)
                    } else {
                        Rendered::Superseded
                    };
                    let answer = Answer {
                        generation: request.generation,
                        place: request.place,
                        entry,
                        rendered,
                    };
                    if answers.send(answer).is_err() {
                        break;
                    }
                }
            });
        if spawned.is_err() {
            return false;
        }
        self.to = Some(to);
        self.from = Some(from);
        true
    }
}

/// The file's type: the one the background check settled on (lane TA),
/// else the one its name says.
fn file_type_of(shared: &Shared, entry: &Entry) -> Option<FileType> {
    shared
        .types
        .by_index(entry.type_index)
        .or_else(|| {
            shared
                .types
                .detect(&entry.name, &[])
                .and_then(|d| shared.types.get(&d.type_id))
        })
        .cloned()
}

/// Reads the previewed file, and only it, and makes its view: a typed
/// previewer's words or listing, a package read whole within its caps,
/// or the first `max_bytes` read by the reader its type names. A folder
/// or a link is said in words, and so is a known kind of file that is
/// not text and that no previewer takes. A changed file's size is asked
/// again, into `entry`.
fn render(shared: &Shared, request: &Request, entry: &mut Entry) -> Rendered {
    let name = entry.name.clone();
    if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
        return Rendered::NoPreview(diroxide_preview::details(entry, None));
    }
    let ctx = OpContext::new()
        .with_cancel(request.cancel.clone())
        .with_tools(Arc::clone(&shared.tools));
    let provider = match shared.registry.resolve(&request.place) {
        Ok(provider) => provider,
        Err(error) => return Rendered::Failed(error),
    };
    if request.refresh
        && let Ok(fresh) = provider.stat(&request.place, &ctx)
    {
        entry.size = fresh.size;
    }
    let file_type = file_type_of(shared, entry);
    let asked = PreviewRequest {
        place: request.place.clone(),
        entry: entry.clone(),
        generation: request.generation,
        max_lines: MAX_ROWS,
    };
    if request.encoding.is_none()
        && let Some(chosen) = shared.previews.chosen(&asked, file_type.as_ref())
        && TYPED.contains(&chosen)
    {
        if TOOL_BACKED.contains(&chosen) && !request.with_tools && !shared.auto {
            return Rendered::OnRequest(diroxide_preview::details(entry, file_type.as_ref()));
        }
        return typed(shared, &asked, file_type.as_ref(), &ctx);
    }
    if entry.kind == EntryKind::Archive {
        return Rendered::NoPreview(diroxide_preview::details(entry, file_type.as_ref()));
    }
    let reader = doc::Reader::for_type(file_type.as_ref(), &name);
    if reader.is_package() && request.encoding.is_none() {
        return package(shared, &asked, reader, &ctx);
    }
    let reader_box = match provider.read(&request.place, &ctx) {
        Ok(reader) => reader,
        Err(VfsError::Cancelled) => return Rendered::Superseded,
        Err(error) => return Rendered::Failed(error),
    };
    let mut source = reader_box.take(shared.max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    let mut sniffed = false;
    // RTF is text to its reader, though the sniff names its signature
    // binary (VL's note): it is never turned away by the sniff.
    let sniffs = request.encoding.is_none() && reader != doc::Reader::Rtf;
    loop {
        if ctx.cancel.is_cancelled() || !shared.latest.is_current(request.generation) {
            return Rendered::Superseded;
        }
        let n = match source.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Rendered::Failed(VfsError::from(error)),
        };
        bytes.extend_from_slice(&chunk[..n]);
        if !sniffed && sniffs && bytes.len() >= SNIFF_BYTES.min(CHUNK) {
            sniffed = true;
            if let Some(words) = not_text(shared, entry, &bytes) {
                return Rendered::NoPreview(words);
            }
        }
    }
    if !sniffed
        && sniffs
        && let Some(words) = not_text(shared, entry, &bytes)
    {
        return Rendered::NoPreview(words);
    }
    let capped = bytes.len() as u64 > shared.max_bytes;
    if capped {
        bytes.truncate(usize::try_from(shared.max_bytes).unwrap_or(usize::MAX));
        // A line cut by the cap is left out, so no half character or half
        // word is read.
        if let Some(end) = bytes.iter().rposition(|&b| b == b'\n') {
            bytes.truncate(end + 1);
        }
    }
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    let choice = Some(if capped || bytes.len() > SNIFF_BYTES {
        choose_head(head, None)
    } else {
        choose(head, None)
    });
    // An encoding the owner chose: the bytes are read in it, as UTF-8 for
    // the readers, its own byte-order mark left out.
    let bytes = match request.encoding {
        Some(encoding) => encoding
            .decode_with_bom_removal(&bytes)
            .0
            .into_owned()
            .into_bytes(),
        None => bytes,
    };
    let (view, error) = doc::read_as_or_text(reader, &name, bytes, &shared.read);
    Rendered::Document {
        view: Box::new(view),
        error,
        capped,
        choice,
        line: None,
    }
}

/// A file a typed previewer takes: its preview, made through the shared
/// cache with the tools in the context, as rows.
fn typed(
    shared: &Shared,
    asked: &PreviewRequest,
    file_type: Option<&FileType>,
    ctx: &OpContext,
) -> Rendered {
    match shared
        .previews
        .render(asked, file_type, &shared.latest, &shared.registry, ctx)
    {
        Ok(preview) => {
            let name = diroxide_core::names::display_name(&asked.entry.name).into_owned();
            let line = match preview.summary.strip_prefix("No preview: ") {
                Some(why) => format!("No preview: {name}, {why}"),
                None => format!("Preview: {name}, {}", preview.summary),
            };
            Rendered::Document {
                view: Box::new(view_of(preview)),
                error: None,
                capped: false,
                choice: None,
                line: Some(line),
            }
        }
        Err(PreviewError::Superseded | PreviewError::Vfs(VfsError::Cancelled)) => {
            Rendered::Superseded
        }
        Err(PreviewError::Vfs(error)) => Rendered::Failed(error),
        Err(error) => Rendered::NoPreview(error.to_string()),
    }
}

/// A typed preview as the quick view's rows: a PDF's pages as the
/// previewer read them, or each detail a row ("Dimensions: 1920 by
/// 1080"); its summary is what the one line says after the name
/// ("Preview: photo.png, PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB").
pub(super) fn view_of(preview: Preview) -> DocView {
    let mut view = match preview.document {
        Some(view) => view,
        None => {
            let mut b = DocView::builder(DocKind::Document);
            for line in &preview.lines {
                b.paragraph();
                b.text(line);
                b.end();
            }
            b.finish()
        }
    };
    view.kind = DocKind::Document;
    view.set_summary_words(preview.summary);
    view
}

/// A Word, EPUB or OpenDocument package, or a workbook, read whole
/// through the container read path within its caps.
fn package(
    shared: &Shared,
    asked: &PreviewRequest,
    reader: doc::Reader,
    ctx: &OpContext,
) -> Rendered {
    let mut source = match diroxide_preview::open_source(
        asked,
        &shared.registry,
        ctx,
        shared.caps.max_container,
    ) {
        Ok(source) => source,
        Err(PreviewError::Vfs(VfsError::Cancelled) | PreviewError::Superseded) => {
            return Rendered::Superseded;
        }
        Err(PreviewError::Vfs(error)) => return Rendered::Failed(error),
        Err(_) => return Rendered::NotRead(DocError::TooLarge),
    };
    if ctx.cancel.is_cancelled() || !shared.latest.is_current(asked.generation) {
        return Rendered::Superseded;
    }
    match doc::read_seekable(
        reader,
        &asked.entry.name,
        source.as_mut(),
        &shared.read,
        &shared.caps,
    ) {
        Ok(view) => Rendered::Document {
            view: Box::new(view),
            error: None,
            capped: false,
            choice: None,
            line: None,
        },
        Err(error) => Rendered::NotRead(error),
    }
}

/// The details in words when the bytes are not text and the name or the
/// bytes say a known kind of file the quick view has no previewer for
/// (a program, a font): "Windows program, 2.0 MB". `None` for text, and
/// for bytes of no known kind, which show as hex.
fn not_text(shared: &Shared, entry: &Entry, head: &[u8]) -> Option<String> {
    binary_kind(head)?;
    let found = shared.types.detect(&entry.name, head)?;
    let file_type = shared.types.get(&found.type_id)?;
    // A type in the text or code group is shown, as text or hex, however
    // its bytes look (lane TL's note): a Markdown file with a stray NUL
    // is still a Markdown file.
    if matches!(file_type.group.as_deref(), Some("text" | "code")) {
        return None;
    }
    Some(diroxide_preview::details(entry, Some(file_type)))
}
