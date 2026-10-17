//! The search itself: the walkers' candidates decided by the filter's
//! evaluator on the walking threads, the hits streamed to the sink on the
//! calling thread in arrival order.
//!
//! The walk runs on a thread of its own, so a cancel answers within a
//! second even when a read is blocked on a dead share: the search then
//! returns with the folders still being read said in its summary, and
//! the blocked thread is left to finish on its own. Otherwise every
//! thread has ended when the search returns.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use diroxide_core::filter::{
    ContentQuery, Evaluator, Field, Filter, Flag, Lookup, Test, Truth, Unknown,
};
use diroxide_core::{Clock, Entry, EntryKind};
use diroxide_vfs::{Registry, VfsError, VolumeKind};

use crate::hit::SearchHit;
use crate::query::SearchQuery;
use crate::sink::{SearchControl, SearchProgress, SearchSink};
use crate::summary::SearchSummary;
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

/// The facts the evaluator asks of a candidate beyond its entry.
struct Facts<'a> {
    candidate: &'a Candidate,
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
                .list(place, &diroxide_vfs::OpContext::new())
                .ok()
                .map(|items| items.is_empty()),
        }
    }

    fn link_broken(&self, entry: &Entry) -> Option<bool> {
        entry.link.as_ref().map(|link| link.points_to.is_none())
    }

    fn content(&self, _: &Entry, _: &ContentQuery) -> Truth {
        Truth::Unknown(Unknown::ContentNotRead)
    }
}

/// Decides candidates on one walking thread and sends the hits.
struct Decide<'a> {
    evaluator: Option<&'a Evaluator>,
    root: &'a Root,
    counts: &'a Counts,
    cap: Option<u64>,
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
        if self.odd_names > 0 {
            let mut stats = WalkStats::default();
            stats.skips.odd_names = self.odd_names;
            let _ = self.tx.send(Message::Walked(stats));
        }
    }
}

impl Visitor for Decide<'_> {
    fn visit(&mut self, candidate: Candidate) -> ControlFlow<()> {
        self.counts.entries.fetch_add(1, Ordering::Relaxed);
        let truth = match self.evaluator {
            Some(evaluator) => evaluator.test(
                &candidate.entry,
                &Facts {
                    candidate: &candidate,
                },
            ),
            None => Truth::Yes,
        };
        match truth {
            Truth::Yes => {}
            Truth::No => return ControlFlow::Continue(()),
            Truth::Unknown(why) => {
                *self.unknown.entry(why).or_default() += 1;
                return ControlFlow::Continue(());
            }
        }
        let Some(place) = candidate.place(self.root) else {
            self.odd_names += 1;
            return ControlFlow::Continue(());
        };
        let before = self.counts.found.fetch_add(1, Ordering::Relaxed);
        if self.cap.is_some_and(|cap| before >= cap) {
            return ControlFlow::Continue(());
        }
        let mut entry = candidate.entry;
        if entry.modified.is_none()
            && let Location::Local(path) = &candidate.location
            && let Ok(metadata) = std::fs::symlink_metadata(path)
        {
            local_entry::fill(&mut entry, &metadata);
        }
        let hit = SearchHit::new(place, entry).with_depth(candidate.depth);
        match self.tx.send(Message::Hit(Box::new(hit))) {
            Ok(()) => ControlFlow::Continue(()),
            Err(_) => ControlFlow::Break(()),
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
                let make = || Decide {
                    evaluator: evaluator.as_ref(),
                    root: &start.root,
                    counts: &counts,
                    cap: scope.row_cap,
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
                files_read: 0,
                bytes_read: 0,
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
