//! The provider walk: folders listed through `Provider::list` by a few
//! threads sharing one queue, breadth first, for archives, remote and
//! plugin places, and for any search that follows links or enters
//! archives. A slow share holds this walk's threads and nothing else.
//!
//! The boundary rule: a link to a folder is followed only when its target
//! lies inside the place the search started in, judged on the real paths
//! (a local link is resolved by the system, a remote one by its text), and
//! never back into a folder it is in. Every link not followed is counted
//! with why. A folder seen before (a hard-linked folder, a bind mount) is
//! not entered again; a folder on another drive, a snapshot folder, a
//! marked place and a folder past the depth limit are not entered, each
//! counted.

use std::collections::{HashSet, VecDeque};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_vfs::{Provider, Registry, VfsError};

use super::ignores::{IgnoreLevel, is_ignored};
use super::{Candidate, Location, Visitor, WalkPlan, WalkStats};

/// Where a folder really is, for the loop checks of followed links.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Real {
    /// Not tracked: links are not followed.
    Untracked,
    /// A local folder's real path, as the system resolves it.
    Path(PathBuf),
    /// A remote folder's place, resolved by its text.
    Place(Place),
}

impl Real {
    fn child(&self, name: &str) -> Real {
        match self {
            Real::Untracked => Real::Untracked,
            Real::Path(path) => Real::Path(path.join(name)),
            Real::Place(place) => place.join(name).map_or(Real::Untracked, Real::Place),
        }
    }

    /// Whether this folder lies inside `outer`, or is it.
    fn is_within(&self, outer: &Real) -> bool {
        match (self, outer) {
            (Real::Path(inner), Real::Path(outer)) => inner.starts_with(outer),
            (Real::Place(inner), Real::Place(outer)) => inner.is_within(outer),
            _ => false,
        }
    }
}

/// A folder waiting to be listed.
struct Job {
    place: Place,
    provider: Arc<dyn Provider>,
    depth: u32,
    /// Its names below the start, for the ignore files.
    names: Vec<String>,
    ignores: Option<Arc<IgnoreLevel>>,
    real: Real,
}

struct Queue {
    jobs: VecDeque<Job>,
    busy: usize,
}

/// What the walk's threads share.
struct Shared<'a> {
    plan: &'a WalkPlan<'a>,
    registry: &'a Registry,
    queue: Mutex<Queue>,
    ready: Condvar,
    stop: AtomicBool,
    /// Folders entered, by volume and inode where the provider gives them.
    seen: Mutex<HashSet<(u64, u64)>>,
    /// Folders entered through a link, by where they really are.
    targets: Mutex<HashSet<Real>>,
    root_volume: Option<u64>,
    root_real: Real,
    stats: Mutex<WalkStats>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared<'_> {
    /// The next folder, or `None` when there is no more work or the walk
    /// stops. Waits while other threads may still add folders.
    fn next(&self) -> Option<Job> {
        let mut queue = lock(&self.queue);
        loop {
            if self.stop.load(Ordering::Relaxed) || self.plan.control.is_cancelled() {
                return None;
            }
            if let Some(job) = queue.jobs.pop_front() {
                queue.busy += 1;
                return Some(job);
            }
            if queue.busy == 0 {
                return None;
            }
            queue = self
                .ready
                .wait_timeout(queue, Duration::from_millis(20))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// A folder is done; its subfolders join the queue.
    fn done(&self, children: Vec<Job>) {
        let mut queue = lock(&self.queue);
        queue.jobs.extend(children);
        queue.busy = queue.busy.saturating_sub(1);
        self.ready.notify_all();
    }

    fn halt(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.ready.notify_all();
    }
}

/// Walks a start through its provider with `plan.threads` threads (at
/// least one), handing every entry to a visitor made by `make` on each.
pub(crate) fn walk<V, F>(
    plan: &WalkPlan<'_>,
    provider: Arc<dyn Provider>,
    registry: &Registry,
    make: &F,
) -> WalkStats
where
    F: Fn() -> V + Sync,
    V: Visitor,
{
    let ctx = plan.control.op_context();
    let root = &plan.root.place;
    let root_entry = match provider.stat(root, &ctx) {
        Ok(entry) => entry,
        Err(VfsError::Cancelled) => {
            return WalkStats {
                stopped: true,
                ..WalkStats::default()
            };
        }
        Err(_) => {
            return WalkStats {
                unreadable: 1,
                ..WalkStats::default()
            };
        }
    };
    if root_entry.kind != EntryKind::Folder && root_entry.kind != EntryKind::Archive {
        let mut stats = WalkStats {
            entries: 1,
            ..WalkStats::default()
        };
        let candidate = Candidate {
            entry: root_entry,
            depth: 0,
            location: Location::Place(root.clone(), provider),
        };
        stats.stopped = make().visit(candidate).is_break();
        return stats;
    }
    let (place, provider) = if root_entry.kind == EntryKind::Archive {
        let inside = root.clone().into_archive();
        match registry.resolve(&inside) {
            Ok(archive) => (inside, archive),
            Err(_) => {
                return WalkStats {
                    unreadable: 1,
                    ..WalkStats::default()
                };
            }
        }
    } else {
        (root.clone(), provider)
    };
    let root_real = if !plan.follow_links {
        Real::Untracked
    } else if let Some(path) = plan.root.path.as_ref() {
        std::fs::canonicalize(path).map_or(Real::Untracked, Real::Path)
    } else {
        Real::Place(place.clone())
    };
    let shared = Shared {
        plan,
        registry,
        queue: Mutex::new(Queue {
            jobs: VecDeque::from([Job {
                place,
                provider,
                depth: 0,
                names: Vec::new(),
                ignores: None,
                real: root_real.clone(),
            }]),
            busy: 0,
        }),
        ready: Condvar::new(),
        stop: AtomicBool::new(false),
        seen: Mutex::new(
            root_entry
                .volume
                .zip(root_entry.inode)
                .into_iter()
                .collect(),
        ),
        targets: Mutex::new(HashSet::from([root_real.clone()])),
        root_volume: root_entry.volume,
        root_real,
        stats: Mutex::new(WalkStats::default()),
    };
    let threads = plan.threads.max(1);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| work(&shared, make()));
        }
    });
    let mut stats = shared
        .stats
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    stats.skips.ignore_files = plan.ignore_files;
    if plan.control.is_cancelled() {
        stats.stopped = true;
    }
    stats
}

/// One thread of the walk.
fn work<V: Visitor>(shared: &Shared<'_>, mut visitor: V) {
    let plan = shared.plan;
    let ctx = plan.control.op_context();
    let mut stats = WalkStats::default();
    'jobs: while let Some(job) = shared.next() {
        if plan.control.checkpoint() {
            shared.done(Vec::new());
            break;
        }
        let entries = match job.provider.list(&job.place, &ctx) {
            Ok(entries) => entries,
            Err(VfsError::Cancelled) => {
                shared.done(Vec::new());
                break;
            }
            Err(_) => {
                stats.unreadable += 1;
                shared.done(Vec::new());
                continue;
            }
        };
        stats.folders += 1;
        let ignores = if plan.ignore_files && job.provider.scheme() != "archive" {
            IgnoreLevel::read(
                job.ignores.clone(),
                job.names.len(),
                &job.place,
                &entries,
                &*job.provider,
                &ctx,
            )
        } else {
            None
        };
        let mut children = Vec::new();
        for entry in entries {
            if plan.control.checkpoint() {
                stats.stopped = true;
                shared.done(children);
                break 'jobs;
            }
            let names = if plan.ignore_files {
                let mut names = job.names.clone();
                names.push(entry.name.clone());
                names
            } else {
                Vec::new()
            };
            let is_folder = entry.kind == EntryKind::Folder;
            if plan.ignore_files && is_ignored(ignores.as_deref(), &names, is_folder) {
                continue;
            }
            if entry.hidden && !plan.hidden {
                stats.skips.hidden += 1;
                continue;
            }
            if is_folder && plan.skip_names.contains(&entry.name) {
                stats.skips.snapshots += 1;
                continue;
            }
            let Ok(place) = job.place.join(&entry.name) else {
                stats.skips.odd_names += 1;
                continue;
            };
            if is_folder && plan.skip_places.iter().any(|p| place.is_within(p)) {
                stats.skips.marked += 1;
                continue;
            }
            let depth = job.depth + 1;
            let next = match entry.kind {
                EntryKind::Folder => enter_folder(shared, &job, &entry, &place, depth, &mut stats),
                EntryKind::Link => enter_link(shared, &job, &entry, &place, depth, &mut stats),
                EntryKind::Archive if plan.archives => {
                    enter_archive(shared, &place, depth, &mut stats)
                }
                _ => None,
            };
            let next = next.map(|(place, provider, real)| Job {
                place,
                provider,
                depth,
                names,
                ignores: ignores.clone(),
                real,
            });
            stats.entries += 1;
            let candidate = Candidate {
                entry,
                depth,
                location: Location::Place(place, Arc::clone(&job.provider)),
            };
            if visitor.visit(candidate) == ControlFlow::Break(()) {
                stats.stopped = true;
                shared.halt();
                shared.done(children);
                break 'jobs;
            }
            children.extend(next);
        }
        shared.done(children);
    }
    lock(&shared.stats).add(&stats);
}

type Next = Option<(Place, Arc<dyn Provider>, Real)>;

fn enter_folder(
    shared: &Shared<'_>,
    job: &Job,
    entry: &Entry,
    place: &Place,
    depth: u32,
    stats: &mut WalkStats,
) -> Next {
    let plan = shared.plan;
    if plan.one_file_system
        && shared.root_volume.is_some()
        && entry.volume.is_some()
        && entry.volume != shared.root_volume
    {
        stats.skips.other_drives += 1;
        return None;
    }
    if !plan.enters(depth) {
        if plan.too_deep(depth) {
            stats.skips.too_deep += 1;
        }
        return None;
    }
    if let (Some(volume), Some(inode)) = (entry.volume, entry.inode)
        && !lock(&shared.seen).insert((volume, inode))
    {
        stats.skips.loops += 1;
        return None;
    }
    Some((
        place.clone(),
        Arc::clone(&job.provider),
        job.real.child(&entry.name),
    ))
}

fn enter_link(
    shared: &Shared<'_>,
    job: &Job,
    entry: &Entry,
    place: &Place,
    depth: u32,
    stats: &mut WalkStats,
) -> Next {
    let plan = shared.plan;
    let points_to_folder = entry
        .link
        .as_ref()
        .is_some_and(|link| link.points_to == Some(EntryKind::Folder));
    if !plan.follow_links {
        if points_to_folder {
            stats.skips.links_not_followed += 1;
        }
        return None;
    }
    let target = match (&shared.root_real, place.local_path()) {
        (Real::Path(_), Some(path)) => {
            let metadata = std::fs::metadata(path).ok()?;
            if !metadata.is_dir() {
                return None;
            }
            Real::Path(std::fs::canonicalize(path).ok()?)
        }
        (Real::Place(_), _) => {
            if !points_to_folder {
                return None;
            }
            let link = entry.link.as_ref()?;
            let parent = place.parent()?;
            Real::Place(Place::resolve(&link.target, &parent).ok()?)
        }
        _ => return None,
    };
    if !target.is_within(&shared.root_real) {
        stats.skips.links_outside += 1;
        return None;
    }
    if job.real.is_within(&target) || !lock(&shared.targets).insert(target.clone()) {
        stats.skips.loops += 1;
        return None;
    }
    if !plan.enters(depth) {
        if plan.too_deep(depth) {
            stats.skips.too_deep += 1;
        }
        return None;
    }
    Some((place.clone(), Arc::clone(&job.provider), target))
}

fn enter_archive(shared: &Shared<'_>, place: &Place, depth: u32, stats: &mut WalkStats) -> Next {
    if !shared.plan.enters(depth) {
        return None;
    }
    let inside = place.clone().into_archive();
    match shared.registry.resolve(&inside) {
        Ok(provider) => Some((inside, provider, Real::Untracked)),
        Err(_) => {
            stats.unreadable += 1;
            None
        }
    }
}
