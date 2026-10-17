//! The search itself: the walkers' candidates decided by the filter's
//! evaluator on the walking threads, the hits streamed to the sink on the
//! calling thread in arrival order.
//!
//! The walk runs on a thread of its own, so a cancel answers within a
//! second even when a read is blocked on a dead share: the search then
//! returns with the folders still being read said in its summary, and
//! the blocked thread is left to finish on its own. Otherwise every
//! thread has ended when the search returns.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use diroxide_core::filter::{
    ContentQuery, Evaluator, Field, Filter, Flag, Lookup, Test, Truth, Unknown,
};
use diroxide_core::{Clock, Entry, EntryKind};
use diroxide_vfs::{Registry, VfsError, VolumeKind, facts};

use crate::content::{Found, MOST_LINES, Reader};
use crate::hit::{ContentMatch, SearchHit};
use crate::query::{OnlineOnly, RowShape, SearchQuery};
use crate::sink::{SearchControl, SearchProgress, SearchSink};
use crate::summary::{SearchSummary, Skips};
use crate::walk::{self, Candidate, Live, Location, Root, Visitor, WalkPlan, WalkStats};
use crate::{SearchError, local_entry};

/// How long a cancelled search waits for its threads before it returns
/// and leaves a blocked read behind: the plan's "the stop key answering
/// within a second".
const GRACE: Duration = Duration::from_millis(900);

/// How often the sink hears the progress.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Hits waiting for the sink before the walkers wait for it.
const CHANNEL: usize = 4096;

/// What the walking thread tells the calling thread.
enum Message {
    Hit(Box<SearchHit>),
    Unknown(HashMap<Unknown, u64>),
    Walked(WalkStats),
    Failed,
}

/// The counts the progress line reads while the search runs.
#[derive(Default)]
struct Counts {
    live: Live,
    entries: AtomicU64,
    found: AtomicU64,
    files_read: AtomicU64,
    bytes_read: AtomicU64,
}

/// Whether the walk must read each entry's size and times: only when a
/// test needs them. Names, kinds, depths and the yes-or-no words that
/// come from the name do not.
fn needs_metadata(filter: Option<&Filter>) -> bool {
    let Some(filter) = filter else {
        return false;
    };
    filter.expr().tests().iter().any(|(test, _)| match test {
        Test::Text(text) => !matches!(
            text.field,
            Field::Name | Field::Path | Field::Extension | Field::Group | Field::Label
        ),
        Test::Kind(_) => false,
        Test::Number(number) => number.field != Field::Depth,
        Test::Flag(flag) => !matches!(
            flag,
            Flag::Hidden
                | Flag::NeedsCare
                | Flag::Selected
                | Flag::Bookmarked
                | Flag::Labeled
                | Flag::Broken
        ),
        _ => true,
    })
}

/// What reading one candidate's contents found and cost.
#[derive(Default)]
struct Read {
    /// The first content test that matched: its lines and their count.
    lines: Option<(Vec<ContentMatch>, u64)>,
    /// Whether an undecided answer is already counted as a skip (binary,
    /// online only, unreadable), so it is not counted twice.
    skipped: bool,
    files: u64,
    bytes: u64,
}

/// A content test as a key: its pattern, whether case is ignored, and
/// its byte limit.
type ReaderKey = (String, bool, Option<u64>);

/// What a walking thread keeps between candidates for content tests.
struct Contents<'a> {
    /// Each content test's reader, compiled once.
    readers: RefCell<HashMap<ReaderKey, Option<Arc<Reader>>>>,
    /// The thread's skip counts.
    skips: RefCell<Skips>,
    keep: usize,
    online: OnlineOnly,
    follow_links: bool,
    /// The start's real path, for links followed to files.
    root_real: Option<PathBuf>,
    control: &'a SearchControl,
}

/// The facts the evaluator asks of a candidate beyond its entry.
struct Facts<'a> {
    candidate: &'a Candidate,
    root: &'a Root,
    contents: &'a Contents<'a>,
    read: RefCell<Read>,
}

impl Facts<'_> {
    fn skip(&self, what: impl FnOnce(&mut Skips)) -> Truth {
        what(&mut self.contents.skips.borrow_mut());
        self.read.borrow_mut().skipped = true;
        Truth::Unknown(Unknown::ContentNotRead)
    }

    fn reader(&self, query: &ContentQuery) -> Option<Arc<Reader>> {
        let key = (
            query.regex().as_str().to_owned(),
            query.ignores_case(),
            query.limit(),
        );
        self.contents
            .readers
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| Reader::new(query).ok().map(Arc::new))
            .clone()
    }

    /// Whether a link's target may be read: only when links are followed
    /// and its real path lies inside the start.
    fn link_inside(&self) -> bool {
        if !self.contents.follow_links {
            return false;
        }
        let path = match &self.candidate.location {
            Location::Local(path) => Some(path.as_path()),
            Location::Place(place, _) => place.local_path().map(std::path::Path::new),
        };
        match (path, &self.contents.root_real) {
            (Some(path), Some(root)) => {
                std::fs::canonicalize(path).is_ok_and(|real| real.starts_with(root))
            }
            _ => false,
        }
    }
}

impl Lookup for Facts<'_> {
    fn path(&self, _: &Entry) -> Option<String> {
        Some(self.candidate.path_text().into_owned())
    }

    fn depth(&self, _: &Entry) -> Option<u32> {
        Some(self.candidate.depth)
    }

    fn folder_empty(&self, _: &Entry) -> Option<bool> {
        match &self.candidate.location {
            Location::Local(path) => std::fs::read_dir(path)
                .ok()
                .map(|mut items| items.next().is_none()),
            Location::Place(place, provider) => provider
                .list(place, &self.contents.control.op_context())
                .ok()
                .map(|items| items.is_empty()),
        }
    }

    fn link_broken(&self, entry: &Entry) -> Option<bool> {
        entry.link.as_ref().map(|link| link.points_to.is_none())
    }

    fn content(&self, entry: &Entry, query: &ContentQuery) -> Truth {
        if entry.kind == EntryKind::Link && !self.link_inside() {
            return Truth::Unknown(Unknown::ContentNotRead);
        }
        if facts::is_online_only(entry) {
            let size = entry.size.unwrap_or(0);
            let refused = match self.contents.online {
                OnlineOnly::Skip => true,
                OnlineOnly::Read => self
                    .candidate
                    .place(self.root)
                    .is_none_or(|place| !self.contents.control.allows(&place, entry)),
            };
            if refused {
                return self.skip(|skips| {
                    skips.online_only += 1;
                    skips.online_only_bytes += size;
                });
            }
        }
        let Some(reader) = self.reader(query) else {
            return Truth::Unknown(Unknown::ContentNotRead);
        };
        let opened: Option<Box<dyn std::io::Read>> = match &self.candidate.location {
            Location::Local(path) => std::fs::File::open(path)
                .ok()
                .map(|file| Box::new(file) as Box<dyn std::io::Read>),
            Location::Place(place, provider) => provider
                .read(place, &self.contents.control.op_context())
                .ok()
                .map(|read| read as Box<dyn std::io::Read>),
        };
        let Some(file) = opened else {
            self.skip(|skips| skips.unreadable_files += 1);
            return Truth::Unknown(Unknown::Unreadable);
        };
        match reader.search(file, self.contents.keep) {
            Ok((found, bytes)) => {
                {
                    let mut read = self.read.borrow_mut();
                    read.files += 1;
                    read.bytes += bytes;
                }
                match found {
                    Found::Lines { lines, count } => {
                        self.read.borrow_mut().lines.get_or_insert((lines, count));
                        Truth::Yes
                    }
                    Found::Nothing => Truth::No,
                    Found::Binary => self.skip(|skips| skips.binary += 1),
                }
            }
            Err(_) => {
                self.skip(|skips| skips.unreadable_files += 1);
                Truth::Unknown(Unknown::Unreadable)
            }
        }
    }
}

/// Decides candidates on one walking thread and sends the hits.
struct Decide<'a> {
    evaluator: Option<&'a Evaluator>,
    root: &'a Root,
    counts: &'a Counts,
    cap: Option<u64>,
    rows: RowShape,
    contents: Contents<'a>,
    tx: SyncSender<Message>,
    unknown: HashMap<Unknown, u64>,
    odd_names: u64,
}

impl Drop for Decide<'_> {
    fn drop(&mut self) {
        let unknown = std::mem::take(&mut self.unknown);
        if !unknown.is_empty() {
            let _ = self.tx.send(Message::Unknown(unknown));
        }
        let mut skips = std::mem::take(&mut *self.contents.skips.borrow_mut());
        skips.odd_names = self.odd_names;
        let stats = WalkStats {
            skips,
            ..WalkStats::default()
        };
        if stats.skips != Skips::default() {
            let _ = self.tx.send(Message::Walked(stats));
        }
    }
}

impl Decide<'_> {
    /// Counts a hit, and sends it while under the row cap.
    fn send(&self, hit: SearchHit) -> ControlFlow<()> {
        let before = self.counts.found.fetch_add(1, Ordering::Relaxed);
        if self.cap.is_some_and(|cap| before >= cap) {
            return ControlFlow::Continue(());
        }
        match self.tx.send(Message::Hit(Box::new(hit))) {
            Ok(()) => ControlFlow::Continue(()),
            Err(_) => ControlFlow::Break(()),
        }
    }
}

impl Visitor for Decide<'_> {
    fn visit(&mut self, candidate: Candidate) -> ControlFlow<()> {
        self.counts.entries.fetch_add(1, Ordering::Relaxed);
        let facts = Facts {
            candidate: &candidate,
            root: self.root,
            contents: &self.contents,
            read: RefCell::new(Read::default()),
        };
        let truth = match self.evaluator {
            Some(evaluator) => evaluator.test(&candidate.entry, &facts),
            None => Truth::Yes,
        };
        let read = facts.read.into_inner();
        if read.files > 0 {
            self.counts
                .files_read
                .fetch_add(read.files, Ordering::Relaxed);
            self.counts
                .bytes_read
                .fetch_add(read.bytes, Ordering::Relaxed);
        }
        match truth {
            Truth::Yes => {}
            Truth::No => return ControlFlow::Continue(()),
            Truth::Unknown(why) => {
                if !read.skipped {
                    *self.unknown.entry(why).or_default() += 1;
                }
                return ControlFlow::Continue(());
            }
        }
        let Some(place) = candidate.place(self.root) else {
            self.odd_names += 1;
            return ControlFlow::Continue(());
        };
        let mut entry = candidate.entry;
        if entry.modified.is_none()
            && let Location::Local(path) = &candidate.location
            && let Ok(metadata) = std::fs::symlink_metadata(path)
        {
            local_entry::fill(&mut entry, &metadata);
        }
        let hit = SearchHit::new(place, entry).with_depth(candidate.depth);
        match read.lines {
            None => self.send(hit),
            Some((lines, count)) => match self.rows {
                RowShape::PerFile => match lines.into_iter().next() {
                    Some(first) => self.send(hit.with_line(first, count.saturating_sub(1))),
                    None => self.send(hit),
                },
                RowShape::PerLine => {
                    for line in lines {
                        self.send(hit.clone().with_line(line, 0))?;
                    }
                    ControlFlow::Continue(())
                }
            },
        }
    }
}

/// One start of the walk, owned by the walking thread.
struct Start {
    root: Root,
    threads: usize,
}

/// The walk on its own thread: every start in turn, each candidate
/// decided where it is found.
fn spawn_walk(
    query: SearchQuery,
    starts: Vec<Start>,
    registry: Registry,
    evaluator: Option<Evaluator>,
    control: SearchControl,
    counts: Arc<Counts>,
    tx: SyncSender<Message>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("diroxide-search".to_owned())
        .spawn(move || {
            let scope = &query.scope;
            let hidden = query.searches_hidden();
            let max_depth = query.depth_limit();
            let metadata = needs_metadata(query.filter.as_ref());
            for start in &starts {
                if control.is_cancelled() {
                    break;
                }
                let plan = WalkPlan {
                    root: &start.root,
                    hidden,
                    ignore_files: scope.ignore_files,
                    follow_links: scope.follow_links,
                    one_file_system: scope.one_file_system,
                    archives: scope.archives,
                    max_depth,
                    threads: start.threads,
                    skip_names: &scope.skip_names,
                    skip_places: &scope.skip_places,
                    metadata,
                    control: &control,
                    live: &counts.live,
                };
                let root_real = start
                    .root
                    .path
                    .as_ref()
                    .filter(|_| scope.follow_links)
                    .and_then(|path| std::fs::canonicalize(path).ok());
                let make = || Decide {
                    evaluator: evaluator.as_ref(),
                    root: &start.root,
                    counts: &counts,
                    cap: scope.row_cap,
                    rows: query.rows,
                    contents: Contents {
                        readers: RefCell::new(HashMap::new()),
                        skips: RefCell::new(Skips::default()),
                        keep: match query.rows {
                            RowShape::PerFile => 1,
                            RowShape::PerLine => MOST_LINES,
                        },
                        online: scope.online_only,
                        follow_links: scope.follow_links,
                        root_real: root_real.clone(),
                        control: &control,
                    },
                    tx: tx.clone(),
                    unknown: HashMap::new(),
                    odd_names: 0,
                };
                let message = match walk::run(&plan, &registry, &make) {
                    Ok(stats) => Message::Walked(stats),
                    Err(_) => Message::Failed,
                };
                if tx.send(message).is_err() {
                    break;
                }
            }
        })
}

/// Searches, handing each hit to `sink` as it is found, in arrival order,
/// and returns the summary. Date words in the expression are read against
/// `clock` once, when the search starts. `control` stops, holds and
/// resumes it from any thread; a hit the sink answers with `Break` stops
/// it too.
///
/// A start that cannot be read is an error when it is the only one, and a
/// folder counted unreadable otherwise.
pub fn run_search(
    query: &SearchQuery,
    registry: &Registry,
    clock: &dyn Clock,
    control: &SearchControl,
    sink: &mut dyn SearchSink,
) -> Result<SearchSummary, SearchError> {
    let started = Instant::now();
    let places = &query.scope.places;
    if places.is_empty() {
        return Err(SearchError::NoPlaces);
    }
    let ctx = control.op_context();
    let mut summary = SearchSummary {
        row_cap: query.scope.row_cap,
        ..SearchSummary::default()
    };
    let mut starts = Vec::new();
    let mut network = false;
    for place in places {
        let provider = registry.resolve(place)?;
        let entry = match provider.stat(place, &ctx) {
            Ok(entry) => entry,
            Err(VfsError::Cancelled) => {
                summary.cancelled = true;
                return Ok(summary);
            }
            Err(error) if places.len() == 1 => return Err(error.into()),
            Err(_) => {
                summary.unreadable += 1;
                continue;
            }
        };
        let volume = provider.volume(place, &ctx).ok().map(|v| v.kind);
        network |= volume == Some(VolumeKind::Network) || place.remote().is_some();
        let root = Root::new(place.clone());
        let local = root.path.is_some() && !query.scope.follow_links && !query.scope.archives;
        let archive = place.archive().is_some() || entry.kind == EntryKind::Archive;
        let threads = query
            .scope
            .threads
            .unwrap_or_else(|| walk::default_threads(local, volume, archive));
        starts.push(Start { root, threads });
    }
    let evaluator = query.filter.as_ref().map(|f| f.evaluator(clock));
    let counts = Arc::new(Counts::default());
    let (tx, rx) = sync_channel(CHANNEL);
    let handle = spawn_walk(
        query.clone(),
        starts,
        registry.clone(),
        evaluator,
        control.clone(),
        Arc::clone(&counts),
        tx,
    )
    .map_err(|error| SearchError::Vfs(VfsError::from(error)))?;
    let ended = deliver(&rx, &counts, control, network, started, sink, &mut summary);
    summary.hits = counts.found.load(Ordering::Relaxed);
    summary.entries = counts.entries.load(Ordering::Relaxed);
    summary.files_read = counts.files_read.load(Ordering::Relaxed);
    summary.bytes_read = counts.bytes_read.load(Ordering::Relaxed);
    summary.cancelled |= control.is_cancelled();
    if ended {
        // Every message is in; the thread is finishing.
        let _ = handle.join();
    } else {
        summary.still_reading = counts.live.listing.load(Ordering::Relaxed).max(1);
        summary.folders = summary
            .folders
            .max(counts.live.folders.load(Ordering::Relaxed));
    }
    summary.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(summary)
}

/// Takes the walking thread's messages until it ends, or until a cancel
/// has waited its grace. Returns whether the thread ended.
fn deliver(
    rx: &Receiver<Message>,
    counts: &Counts,
    control: &SearchControl,
    network: bool,
    started: Instant,
    sink: &mut dyn SearchSink,
    summary: &mut SearchSummary,
) -> bool {
    let mut last_progress = Instant::now();
    let mut cancelled_at: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(Message::Hit(hit)) => {
                summary.shown += 1;
                if sink.found(*hit).is_break() {
                    control.cancel();
                }
            }
            Ok(Message::Unknown(unknown)) => {
                for (why, n) in unknown {
                    summary.add_unknown(why, n);
                }
            }
            Ok(Message::Walked(stats)) => {
                summary.folders += stats.folders;
                summary.unreadable += stats.unreadable;
                summary.skipped.add(&stats.skips);
            }
            Ok(Message::Failed) => summary.unreadable += 1,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return true,
        }
        if last_progress.elapsed() >= PROGRESS_EVERY {
            last_progress = Instant::now();
            sink.progress(&SearchProgress {
                found: counts.found.load(Ordering::Relaxed),
                folders: counts.live.folders.load(Ordering::Relaxed),
                entries: counts.entries.load(Ordering::Relaxed),
                files_read: counts.files_read.load(Ordering::Relaxed),
                bytes_read: counts.bytes_read.load(Ordering::Relaxed),
                elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                paused: control.is_paused(),
                network,
            });
        }
        if control.is_cancelled() {
            let at = *cancelled_at.get_or_insert_with(Instant::now);
            if at.elapsed() >= GRACE {
                return false;
            }
        }
    }
}
