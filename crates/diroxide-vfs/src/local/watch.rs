//! Watching folders (spike three; research/file-operations.md,
//! "Directory watching").
//!
//! - **One watcher for the app.** Every watched folder shares one `notify`
//!   watcher and one debounce thread. Its handler only sends to a channel
//!   and returns: a handler that blocks loses events silently.
//! - **The parent is watched too.** A folder's own watch never reports its
//!   own rename or removal on Windows; its parent's does. Either way the
//!   panel hears [`WatchEvent::Gone`] at once ("Folder is gone"), and the
//!   folder's watches are dropped, since their paths are stale.
//! - **Debounced.** Events wait for 200 ms of quiet, or one second at
//!   most; up to 50 in a window are passed on one by one, more become one
//!   [`WatchEvent::Rescan`]. On Windows the count is the only overflow
//!   signal there is: an overflowing buffer comes back empty, with no
//!   flag. During a job's burst the panel hears at most one Rescan a
//!   second. Unfinished `.diro-part-` names are never passed on.
//! - **Removable drives are polled**, every two seconds (slower when a
//!   look takes long), so no watch holds the drive and its eject keeps
//!   working until the owner's eject test says otherwise. **Network places
//!   are not watched**: `Ok(None)`, and the panel says "Folder not watched.
//!   Reload with Ctrl+R." A watch that fails to start answers the same.
//! - **Leaving a folder drops its watch at once**, when the handle drops.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use diroxide_core::Place;
use notify::event::{AccessKind, AccessMode, ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::VfsError;
use crate::local::volumes::{self, WatchMode};
use crate::names;
use crate::provider::{WatchEvent, WatchHandle, WatchSink};

/// The quiet that ends a window of events.
pub(crate) const QUIET: Duration = Duration::from_millis(200);

/// The longest a window of events is held.
pub(crate) const MAX_WINDOW: Duration = Duration::from_secs(1);

/// More events than this in one window become one Rescan.
pub(crate) const THRESHOLD: usize = 50;

/// How often a removable drive's folder is looked at.
pub(crate) const POLL_EVERY: Duration = Duration::from_secs(2);

/// How often the debounce thread wakes to close windows.
const TICK: Duration = Duration::from_millis(50);

type SharedSink = Arc<WatchSink>;

/// One panel's watch of one folder.
struct Subscription {
    /// The folder as the panel names it, for the events' places.
    place: Place,
    /// The folder as the watcher reports it.
    folder: PathBuf,
    /// The folder's parent, watched for the folder's own rename or
    /// removal.
    parent: Option<PathBuf>,
    sink: SharedSink,
    /// The folder went away; its watches are dropped.
    gone: bool,
    pending: Vec<WatchEvent>,
    overflow: bool,
    first: Option<Instant>,
    last: Option<Instant>,
}

/// The app's one watcher, its watched paths and the panels listening.
#[derive(Default)]
struct Hub {
    watcher: Option<RecommendedWatcher>,
    counts: HashMap<PathBuf, usize>,
    subscriptions: HashMap<u64, Subscription>,
    next: u64,
}

fn lock_hub() -> MutexGuard<'static, Hub> {
    static HUB: OnceLock<Mutex<Hub>> = OnceLock::new();
    let hub = HUB.get_or_init(|| Mutex::new(Hub::default()));
    match hub.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Hub {
    /// The watcher, made with the debounce thread the first time.
    fn watcher(&mut self) -> Option<&mut RecommendedWatcher> {
        if self.watcher.is_none() {
            let (sender, receiver): (Sender<notify::Result<Event>>, Receiver<_>) = mpsc::channel();
            let watcher = notify::recommended_watcher(sender).ok()?;
            std::thread::Builder::new()
                .name("diroxide-watch".to_owned())
                .spawn(move || debounce(&receiver))
                .ok()?;
            self.watcher = Some(watcher);
        }
        self.watcher.as_mut()
    }

    /// Watches a path (not its subfolders), counting the panels that need
    /// it.
    fn add(&mut self, path: &Path) -> bool {
        if let Some(count) = self.counts.get_mut(path) {
            *count += 1;
            return true;
        }
        let Some(watcher) = self.watcher() else {
            return false;
        };
        if watcher.watch(path, RecursiveMode::NonRecursive).is_err() {
            return false;
        }
        self.counts.insert(path.to_path_buf(), 1);
        true
    }

    /// Stops needing a path; the last panel to stop unwatches it.
    fn release(&mut self, path: &Path) {
        let Some(count) = self.counts.get_mut(path) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            self.counts.remove(path);
            if let Some(watcher) = self.watcher.as_mut() {
                let _ = watcher.unwatch(path);
            }
        }
    }

    /// Drops a subscription's native watches.
    fn release_subscription(&mut self, folder: &Path, parent: Option<&Path>) {
        self.release(folder);
        if let Some(parent) = parent {
            self.release(parent);
        }
    }
}

/// Events to hand to sinks once the hub is unlocked.
type Deliveries = Vec<(SharedSink, Vec<WatchEvent>)>;

/// What a change to a direct child means, by the kind of event and where
/// in a rename pair the path stands.
fn child_event(kind: &EventKind, index: usize, place: Place, path: &Path) -> Option<WatchEvent> {
    let exists = || fs::symlink_metadata(path).is_ok();
    let event = match kind {
        EventKind::Create(_) => WatchEvent::Created(place),
        EventKind::Remove(_) => WatchEvent::Removed(place),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => WatchEvent::Removed(place),
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => WatchEvent::Created(place),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if index == 0 {
                WatchEvent::Removed(place)
            } else {
                WatchEvent::Created(place)
            }
        }
        EventKind::Modify(ModifyKind::Name(_)) => {
            if exists() {
                WatchEvent::Created(place)
            } else {
                WatchEvent::Removed(place)
            }
        }
        EventKind::Modify(_) => WatchEvent::Changed(place),
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => WatchEvent::Changed(place),
        EventKind::Access(_) => return None,
        EventKind::Any | EventKind::Other => {
            if exists() {
                WatchEvent::Changed(place)
            } else {
                WatchEvent::Removed(place)
            }
        }
    };
    Some(event)
}

/// Whether an event on the folder itself means it went away.
fn folder_went_away(kind: &EventKind, folder: &Path) -> bool {
    matches!(
        kind,
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    ) && fs::symlink_metadata(folder).is_err()
}

/// Sorts one raw event into the panels' windows, and hands a "gone" over
/// at once.
fn route(hub: &mut Hub, event: notify::Result<Event>, now: Instant, out: &mut Deliveries) {
    let event = match event {
        Ok(event) => event,
        Err(_) => {
            // The watcher failed: nothing it said can be trusted.
            for subscription in hub.subscriptions.values_mut() {
                subscription.overflow = true;
                subscription.first.get_or_insert(now);
                subscription.last = Some(now);
            }
            return;
        }
    };
    let mut gone = Vec::new();
    for (id, subscription) in &mut hub.subscriptions {
        if subscription.gone {
            continue;
        }
        let touches = |path: &Path| {
            path == subscription.folder || path.parent() == Some(subscription.folder.as_path())
        };
        if event.need_rescan() && (event.paths.is_empty() || event.paths.iter().any(|p| touches(p)))
        {
            subscription.overflow = true;
            subscription.first.get_or_insert(now);
            subscription.last = Some(now);
        }
        for (index, path) in event.paths.iter().enumerate() {
            if *path == subscription.folder {
                if folder_went_away(&event.kind, path) {
                    gone.push(*id);
                }
                continue;
            }
            if path.parent() != Some(subscription.folder.as_path()) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                subscription.overflow = true;
                continue;
            };
            if names::is_unfinished(name) {
                continue;
            }
            let Ok(place) = subscription.place.join(name) else {
                continue;
            };
            if let Some(change) = child_event(&event.kind, index, place, path) {
                subscription.pending.push(change);
                subscription.first.get_or_insert(now);
                subscription.last = Some(now);
            }
        }
    }
    for id in gone {
        let Some(subscription) = hub.subscriptions.get_mut(&id) else {
            continue;
        };
        subscription.gone = true;
        subscription.pending.clear();
        subscription.overflow = false;
        let sink = Arc::clone(&subscription.sink);
        let folder = subscription.folder.clone();
        let parent = subscription.parent.clone();
        hub.release_subscription(&folder, parent.as_deref());
        out.push((sink, vec![WatchEvent::Gone]));
    }
}

/// Closes the windows that have been quiet for 200 ms or open for a
/// second: up to 50 events one by one, more as one Rescan.
fn flush(hub: &mut Hub, now: Instant, out: &mut Deliveries) {
    for subscription in hub.subscriptions.values_mut() {
        let (Some(first), Some(last)) = (subscription.first, subscription.last) else {
            continue;
        };
        if now.duration_since(last) < QUIET && now.duration_since(first) < MAX_WINDOW {
            continue;
        }
        let pending = std::mem::take(&mut subscription.pending);
        let events = if subscription.overflow || pending.len() > THRESHOLD {
            vec![WatchEvent::Rescan]
        } else {
            let mut unique: Vec<WatchEvent> = Vec::with_capacity(pending.len());
            for event in pending {
                if !unique.contains(&event) {
                    unique.push(event);
                }
            }
            unique
        };
        subscription.overflow = false;
        subscription.first = None;
        subscription.last = None;
        if !events.is_empty() {
            out.push((Arc::clone(&subscription.sink), events));
        }
    }
}

/// The debounce thread: sorts raw events into windows and closes them,
/// calling the sinks only after the hub is unlocked.
fn debounce(receiver: &Receiver<notify::Result<Event>>) {
    loop {
        let message = receiver.recv_timeout(TICK);
        let now = Instant::now();
        let mut out = Deliveries::new();
        {
            let mut hub = lock_hub();
            match message {
                Ok(event) => route(&mut hub, event, now, &mut out),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            flush(&mut hub, now, &mut out);
        }
        for (sink, events) in out {
            for event in events {
                sink(event);
            }
        }
    }
}

/// The path the watcher reports for a folder: canonical where the system
/// reports canonical paths (macOS's FSEvents), as given on Windows (where
/// canonicalizing would add the extended-length prefix).
fn reported_path(folder: &Path) -> PathBuf {
    if cfg!(windows) {
        folder.to_path_buf()
    } else {
        fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
    }
}

/// Starts a native watch, or `None` when it cannot start.
fn native(place: &Place, folder: &Path, sink: WatchSink) -> Option<WatchHandle> {
    let folder = reported_path(folder);
    let parent = folder.parent().map(Path::to_path_buf);
    let mut hub = lock_hub();
    if !hub.add(&folder) {
        return None;
    }
    // Without the parent's watch the folder is still watched; only its own
    // disappearance would go unheard.
    let parent = parent.filter(|parent| hub.add(parent));
    let id = hub.next;
    hub.next += 1;
    hub.subscriptions.insert(
        id,
        Subscription {
            place: place.clone(),
            folder,
            parent,
            sink: Arc::new(sink),
            gone: false,
            pending: Vec::new(),
            overflow: false,
            first: None,
            last: None,
        },
    );
    Some(WatchHandle::new(move || {
        let mut hub = lock_hub();
        if let Some(subscription) = hub.subscriptions.remove(&id)
            && !subscription.gone
        {
            hub.release_subscription(&subscription.folder, subscription.parent.as_deref());
        }
    }))
}

/// A cheap fingerprint of a folder's contents: how many entries, and their
/// names, sizes and modified times mixed in an order-free way. `None` when
/// the folder is gone.
fn fingerprint(folder: &Path) -> Option<(u64, u64)> {
    let reader = fs::read_dir(folder).ok()?;
    let mut count = 0u64;
    let mut mixed = 0u64;
    for item in reader.flatten() {
        count += 1;
        let mut hasher = DefaultHasher::new();
        item.file_name().hash(&mut hasher);
        if let Ok(metadata) = item.metadata() {
            metadata.len().hash(&mut hasher);
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .hash(&mut hasher);
        }
        mixed = mixed.wrapping_add(hasher.finish());
    }
    Some((count, mixed))
}

/// Polls a folder: a look every `every` (slower when a look takes long),
/// a Rescan when anything changed, Gone when the folder went away. No
/// handle on the folder is held between looks.
pub(crate) fn poll(folder: &Path, sink: WatchSink, every: Duration) -> Option<WatchHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let folder = folder.to_path_buf();
    let mut last = fingerprint(&folder)?;
    std::thread::Builder::new()
        .name("diroxide-poll".to_owned())
        .spawn(move || {
            let mut wait = every;
            loop {
                let until = Instant::now() + wait;
                while Instant::now() < until {
                    if stopped.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(TICK.min(every));
                }
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                let started = Instant::now();
                match fingerprint(&folder) {
                    None => {
                        sink(WatchEvent::Gone);
                        return;
                    }
                    Some(now) if now != last => {
                        last = now;
                        sink(WatchEvent::Rescan);
                    }
                    Some(_) => {}
                }
                wait = every
                    .max(started.elapsed() * 10)
                    .min(Duration::from_secs(30));
            }
        })
        .ok()?;
    Some(WatchHandle::new(move || {
        stop.store(true, Ordering::Relaxed)
    }))
}

/// Watches a folder the way its volume allows (see the module
/// documentation): natively, by polling, or not at all (`Ok(None)`).
pub(crate) fn watch(
    place: &Place,
    folder: &Path,
    sink: WatchSink,
) -> Result<Option<WatchHandle>, VfsError> {
    if !fs::metadata(folder).is_ok_and(|m| m.is_dir()) {
        return Ok(None);
    }
    let facts = volumes::facts_at(folder);
    Ok(match volumes::watch_mode(&facts) {
        WatchMode::None => None,
        WatchMode::Poll => poll(folder, sink, POLL_EVERY),
        WatchMode::Native => native(place, folder, sink),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

    fn collecting() -> (WatchSink, Receiver<WatchEvent>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let sink: WatchSink = Box::new(move |event| {
            if let Ok(sender) = sender.lock() {
                let _ = sender.send(event);
            }
        });
        (sink, receiver)
    }

    #[test]
    fn a_polled_folder_says_rescan_then_gone() {
        let scratch = crate::test_support::scratch();
        let folder = scratch.path().join("stick");
        fs::create_dir(&folder).unwrap();
        let (sink, events) = collecting();
        let handle = poll(&folder, sink, Duration::from_millis(100)).unwrap();
        fs::write(folder.join("new.txt"), b"n").unwrap();
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).unwrap(),
            WatchEvent::Rescan
        );
        fs::remove_file(folder.join("new.txt")).unwrap();
        fs::remove_dir(&folder).unwrap();
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).unwrap(),
            WatchEvent::Gone
        );
        drop(handle);
    }

    #[test]
    fn a_dropped_poll_stops() {
        let scratch = crate::test_support::scratch();
        let (sink, events) = collecting();
        let handle = poll(scratch.path(), sink, Duration::from_millis(50)).unwrap();
        drop(handle);
        std::thread::sleep(Duration::from_millis(150));
        fs::write(scratch.path().join("late.txt"), b"l").unwrap();
        assert!(events.recv_timeout(Duration::from_millis(400)).is_err());
    }

    #[test]
    fn children_events_read_by_kind() {
        let place = Place::parse(if cfg!(windows) { "D:\\w\\a" } else { "/w/a" }).unwrap();
        let path = Path::new("no-such-path-anywhere");
        let create = EventKind::Create(notify::event::CreateKind::File);
        assert_eq!(
            child_event(&create, 0, place.clone(), path),
            Some(WatchEvent::Created(place.clone()))
        );
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        assert_eq!(
            child_event(&both, 0, place.clone(), path),
            Some(WatchEvent::Removed(place.clone()))
        );
        assert_eq!(
            child_event(&both, 1, place.clone(), path),
            Some(WatchEvent::Created(place.clone()))
        );
        let read = EventKind::Access(AccessKind::Read);
        assert_eq!(child_event(&read, 0, place.clone(), path), None);
        let any = EventKind::Any;
        assert_eq!(
            child_event(&any, 0, place.clone(), path),
            Some(WatchEvent::Removed(place))
        );
    }
}
