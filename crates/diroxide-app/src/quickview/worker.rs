//! The quick view's render worker: one thread for the quick view's life,
//! started on its first render, that reads the previewed file and only
//! that file, at most the cap, and turns it into a document view. A
//! request whose generation is no longer the newest ([`LatestOnly`]) is
//! dropped before it reads; a newer move cancels the read in flight
//! through its token. Markdown and carta run under the preview crate's
//! guard, so a hostile file is cut off at the deadline.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::LatestOnly;
use diroxide_preview::doc::{self, DocError, DocView, ReadOptions};
use diroxide_preview::encoding::{SNIFF_BYTES, binary_kind};
use diroxide_vfs::{CancelToken, OpContext, Registry, VfsError};

/// The bytes read between two checks of the cancel token.
const CHUNK: usize = 64 * 1024;

/// What the worker needs, shared with its thread.
#[derive(Debug)]
pub(super) struct Shared {
    pub registry: Registry,
    pub types: TypeRegistry,
    pub latest: Arc<LatestOnly>,
    pub read: ReadOptions,
    pub max_bytes: u64,
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
}

/// What a render made.
#[derive(Debug)]
pub(super) enum Rendered {
    /// The rows, and why the format's reader could not read it when the
    /// text is shown instead.
    Document {
        view: DocView,
        error: Option<DocError>,
        /// Whether the cap stopped the read short of the file's end.
        capped: bool,
    },
    /// Nothing to show for this kind of file yet: the details in words
    /// ("PNG image, 2.0 MB").
    NoPreview(String),
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
                    let rendered = if shared.latest.is_current(request.generation) {
                        render(&shared, &request)
                    } else {
                        Rendered::Superseded
                    };
                    let answer = Answer {
                        generation: request.generation,
                        place: request.place,
                        entry: request.entry,
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

/// Reads the previewed file, and only it, at most the cap, and makes its
/// view; a folder, a link or an archive is said in words, and so is a
/// known kind of file that is not text (a picture, a PDF).
fn render(shared: &Shared, request: &Request) -> Rendered {
    let mut entry = request.entry.clone();
    let name = entry.name.clone();
    if !matches!(entry.kind, EntryKind::File) {
        return Rendered::NoPreview(diroxide_preview::details(&entry, None));
    }
    let ctx = OpContext::new().with_cancel(request.cancel.clone());
    let provider = match shared.registry.resolve(&request.place) {
        Ok(provider) => provider,
        Err(error) => return Rendered::Failed(error),
    };
    if request.refresh
        && let Ok(fresh) = provider.stat(&request.place, &ctx)
    {
        entry.size = fresh.size;
    }
    let reader = match provider.read(&request.place, &ctx) {
        Ok(reader) => reader,
        Err(VfsError::Cancelled) => return Rendered::Superseded,
        Err(error) => return Rendered::Failed(error),
    };
    let mut reader = reader.take(shared.max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    let mut sniffed = false;
    loop {
        if ctx.cancel.is_cancelled() || !shared.latest.is_current(request.generation) {
            return Rendered::Superseded;
        }
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Rendered::Failed(VfsError::from(error)),
        };
        bytes.extend_from_slice(&chunk[..n]);
        if !sniffed && bytes.len() >= SNIFF_BYTES.min(CHUNK) {
            sniffed = true;
            if let Some(words) = not_text(shared, &entry, &bytes) {
                return Rendered::NoPreview(words);
            }
        }
    }
    if !sniffed && let Some(words) = not_text(shared, &entry, &bytes) {
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
    let (view, error) = doc::read_or_text(&name, bytes, &shared.read);
    Rendered::Document {
        view,
        error,
        capped,
    }
}

/// The details in words when the bytes are not text and the name or the
/// bytes say a known kind of file the quick view has no previewer for
/// yet (a picture, a PDF, an archive): "PNG image, 2.0 MB". `None` for
/// text, and for bytes of no known kind, which show as hex.
fn not_text(shared: &Shared, entry: &Entry, head: &[u8]) -> Option<String> {
    binary_kind(head)?;
    let found = shared.types.detect(&entry.name, head)?;
    let file_type = shared.types.get(&found.type_id)?;
    if matches!(file_type.id.as_str(), "text" | "html") {
        return None;
    }
    Some(diroxide_preview::details(entry, Some(file_type)))
}
