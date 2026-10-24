//! Free space asked with a short limit (R25, 3.4; lane A3, commit 90): a
//! remote place's free space comes from its plugin (`provider/volumes`,
//! `statvfs` on an SFTP server), and a slow server must never stall a plan
//! or Say Status. Every remote provider in the runtime's registry is
//! wrapped in [`TimedSpace`], which asks `space` and `volume` on a helper
//! thread and gives up after the limit with "not known", never zero. Its
//! answers are kept a few seconds, so the plan's room check and the line
//! said after it share one request.
//!
//! The sentences: "Not enough room on build:" with "Needs 4.0 GB, 1.2 GB
//! free." under it, and "Free space on build is not known." when the
//! server cannot say; the copy then goes on.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::{Entry, JobKind, JobRequest, Place};
use diroxide_ops::{Plan, Preflight};
use diroxide_vfs::provider::{
    AttributeChange, CopyHint, FileWriter, LinkRequest, ReadSeek, ResumableWrite, StreamingList,
    WatchSink, WriteMode,
};
use diroxide_vfs::{
    CancelToken, Capabilities, OpContext, Provider, Registry, SpaceInfo, TrashCheck, TrashGate,
    VfsError, VolumeInfo, WatchHandle,
};

/// How long a place may take to say its free space before it is "not
/// known".
pub const SPACE_LIMIT: Duration = Duration::from_secs(3);

/// How long an answer is kept for the next question about the same
/// volume.
const KEPT_FOR: Duration = Duration::from_secs(5);

/// How often a wait looks at the caller's cancel.
const TICK: Duration = Duration::from_millis(25);

/// Runs `ask` on a helper thread and waits at most `limit`: `None` when it
/// did not answer in time (its request is cancelled), the caller's own
/// cancel as an error.
pub(crate) fn within<T: Send + 'static>(
    limit: Duration,
    ctx: &OpContext,
    ask: impl FnOnce(&OpContext) -> Result<T, VfsError> + Send + 'static,
) -> Option<Result<T, VfsError>> {
    let child = CancelToken::new();
    let asked = ctx.clone().with_cancel(child.clone());
    let (sender, answers) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("space-check".to_owned())
        .spawn(move || {
            let _ = sender.send(ask(&asked));
        });
    if spawned.is_err() {
        return None;
    }
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            child.cancel();
            return None;
        }
        match answers.recv_timeout(left.min(TICK)) {
            Ok(answer) => return Some(answer),
            Err(RecvTimeoutError::Timeout) => {
                if let Err(error) = ctx.check() {
                    child.cancel();
                    return Some(Err(error));
                }
            }
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// A provider whose free space and volume are asked with a time limit;
/// everything else goes straight to the provider it wraps.
pub(crate) struct TimedSpace {
    inner: Arc<dyn Provider>,
    limit: Duration,
    kept: Mutex<HashMap<String, (Instant, SpaceInfo)>>,
}

impl TimedSpace {
    /// Wraps a provider.
    pub(crate) fn new(inner: Arc<dyn Provider>, limit: Duration) -> Self {
        Self {
            inner,
            limit,
            kept: Mutex::new(HashMap::new()),
        }
    }

    fn key(place: &Place) -> String {
        VolumeInfo::top_of(place).to_string()
    }
}

impl fmt::Debug for TimedSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimedSpace")
            .field("inner", &self.inner)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl Provider for TimedSpace {
    fn scheme(&self) -> &str {
        self.inner.scheme()
    }
    fn capabilities(&self, place: &Place) -> Capabilities {
        self.inner.capabilities(place)
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.inner.list(place, ctx)
    }
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.inner.stat(place, ctx)
    }
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.inner.read(place, ctx)
    }
    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.inner.open_seekable(place, ctx)
    }
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.inner.write(place, mode, ctx)
    }
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.create_dir(place, ctx)
    }
    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        self.inner.create_link(place, request, ctx)
    }
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.rename(from, to, ctx)
    }
    fn rename_replacing(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.rename_replacing(from, to, ctx)
    }
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.remove(place, ctx)
    }
    fn trash_gate(&self, place: &Place, ctx: &OpContext) -> Result<TrashGate, VfsError> {
        self.inner.trash_gate(place, ctx)
    }
    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        self.inner.trash(place, ctx)
    }
    fn restore(&self, original: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.restore(original, ctx)
    }
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.inner.set_attributes(place, change, ctx)
    }
    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        self.inner.copy_hint(from, to)
    }
    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.copy_within(from, to, ctx)
    }
    /// The volume, asked with the limit; a volume of unknown kind at the
    /// place's top when it does not answer in time.
    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        let inner = Arc::clone(&self.inner);
        let asked = place.clone();
        within(self.limit, ctx, move |ctx| inner.volume(&asked, ctx))
            .unwrap_or_else(|| Ok(VolumeInfo::unknown(VolumeInfo::top_of(place))))
    }
    /// The free space, asked with the limit and kept a few seconds; "not
    /// known" when it does not answer in time, never zero.
    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        let key = Self::key(place);
        let fresh = self
            .kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .filter(|(at, _)| at.elapsed() < KEPT_FOR)
            .map(|(_, space)| *space);
        if let Some(space) = fresh {
            return Ok(space);
        }
        let inner = Arc::clone(&self.inner);
        let asked = place.clone();
        let space = match within(self.limit, ctx, move |ctx| inner.space(&asked, ctx)) {
            Some(Ok(space)) => space,
            Some(Err(error)) if ctx.cancel.is_cancelled() => return Err(error),
            // A refusal or no answer in time: not known, never zero.
            Some(Err(_)) | None => SpaceInfo::default(),
        };
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, (Instant::now(), space));
        Ok(space)
    }
    fn watch(&self, place: &Place, sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        self.inner.watch(place, sink)
    }
    fn as_streaming(&self) -> Option<&dyn StreamingList> {
        self.inner.as_streaming()
    }
    fn as_resumable(&self) -> Option<&dyn ResumableWrite> {
        self.inner.as_resumable()
    }
}

/// The registry with each remote provider wrapped in [`TimedSpace`];
/// local places and archives as they are.
pub(crate) fn timed(registry: &Registry, limit: Duration) -> Registry {
    let mut timed = registry.clone();
    for scheme in registry.schemes() {
        let Ok(place) = Place::parse(&format!("{scheme}://host/")) else {
            continue;
        };
        if let Ok(provider) = registry.resolve(&place) {
            timed.register(scheme, Arc::new(TimedSpace::new(provider, limit)));
        }
    }
    timed
}

/// The name a remote place's volume is said by: its host ("build"); a
/// local place's own words otherwise.
pub(crate) fn host_words(place: &Place) -> String {
    match place.remote() {
        Some(remote) => remote.host().to_owned(),
        None => diroxide_core::names::spell_out(&place.to_string()).into_owned(),
    }
}

/// A plan's room question in lines, a remote volume named by its host:
/// "Not enough room on build:", "Needs 4.0 GB, 1.2 GB free.", "Go on
/// anyway? y or n". Every other question as the engine words it.
pub(crate) fn question_lines(question: &Preflight) -> Vec<String> {
    let mut lines = question.lines();
    if let Preflight::NotEnoughRoom { drive, .. } = question
        && drive.remote().is_some()
        && let Some(first) = lines.first_mut()
    {
        *first = format!("Not enough room on {}:", host_words(drive));
    }
    lines
}

/// The line said before a plan's confirmation when a copy or a move to a
/// remote place could not learn its free space: "Free space on build is
/// not known."; `None` when it is known, the plan already asks about
/// room, or nothing is written there. The answer comes from the plan's
/// own question a moment ago, kept by [`TimedSpace`].
pub(crate) fn room_not_known(registry: &Registry, job: &JobRequest, plan: &Plan) -> Option<String> {
    if !matches!(job.kind, JobKind::Copy | JobKind::Move) {
        return None;
    }
    let destination = job.destination.as_ref()?;
    destination.remote()?;
    if !plan.facts.counted || plan.facts.bytes == 0 {
        return None;
    }
    let asks_room = plan
        .facts
        .questions
        .iter()
        .any(|q| matches!(q, Preflight::NotEnoughRoom { .. }));
    if asks_room {
        return None;
    }
    let provider = registry.resolve(destination).ok()?;
    let space = provider.space(destination, &OpContext::new()).ok()?;
    space
        .free_to_user
        .is_none()
        .then(|| format!("Free space on {} is not known.", host_words(destination)))
}

/// A place's free space in words for Say Status: "412 GB free of 931 GB",
/// "412 GB free", or "free space not known".
pub(crate) fn room_words(space: Option<&SpaceInfo>) -> String {
    use diroxide_core::format::human_size;
    match space.and_then(|s| s.free_to_user.map(|free| (free, s.total))) {
        Some((free, Some(total))) => {
            format!("{} free of {}", human_size(free), human_size(total))
        }
        Some((free, None)) => format!("{} free", human_size(free)),
        None => "free space not known".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider whose free space takes a while.
    #[derive(Debug)]
    struct Slow(Duration);

    impl Provider for Slow {
        fn scheme(&self) -> &str {
            "slow"
        }
        fn capabilities(&self, _place: &Place) -> Capabilities {
            Capabilities::read_only()
        }
        fn list(&self, _place: &Place, _ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
            Ok(Vec::new())
        }
        fn stat(&self, _place: &Place, _ctx: &OpContext) -> Result<Entry, VfsError> {
            Err(VfsError::NotFound)
        }
        fn read(&self, _place: &Place, _ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
            Err(VfsError::NotFound)
        }
        fn write(
            &self,
            _place: &Place,
            _mode: WriteMode,
            _ctx: &OpContext,
        ) -> Result<Box<dyn FileWriter>, VfsError> {
            Err(VfsError::NotFound)
        }
        fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
            Ok(())
        }
        fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
            Ok(())
        }
        fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
            Ok(())
        }
        fn set_attributes(
            &self,
            _place: &Place,
            _change: &AttributeChange,
            _ctx: &OpContext,
        ) -> Result<AttributeChange, VfsError> {
            Ok(AttributeChange::default())
        }
        fn space(&self, _place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
            let until = Instant::now() + self.0;
            while Instant::now() < until {
                ctx.check()?;
                thread::sleep(Duration::from_millis(5));
            }
            Ok(SpaceInfo {
                free_to_user: Some(1000),
                total: Some(2000),
                max_file_size: None,
            })
        }
    }

    #[test]
    fn a_slow_place_is_not_known_after_the_limit_and_never_zero() {
        let slow = TimedSpace::new(
            Arc::new(Slow(Duration::from_secs(5))),
            Duration::from_millis(50),
        );
        let place = Place::parse("slow://host/a").unwrap();
        let started = Instant::now();
        let space = slow.space(&place, &OpContext::new()).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(space.free_to_user, None);
        assert_eq!(room_words(Some(&space)), "free space not known");
        let volume = slow.volume(&place, &OpContext::new()).unwrap();
        assert_eq!(volume.root, Place::parse("slow://host/").unwrap());
    }

    #[test]
    fn a_quick_place_answers_and_is_kept_a_moment() {
        let quick = TimedSpace::new(Arc::new(Slow(Duration::ZERO)), Duration::from_secs(2));
        let place = Place::parse("slow://host/a").unwrap();
        let space = quick.space(&place, &OpContext::new()).unwrap();
        assert_eq!(space.free_to_user, Some(1000));
        assert_eq!(room_words(Some(&space)), "1.0 KB free of 2.0 KB");
        assert_eq!(quick.kept.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_room_question_names_a_remote_volume_by_its_host() {
        let room = Preflight::NotEnoughRoom {
            drive: Place::parse("sftp://ada@build:2222/").unwrap(),
            needed: 4_000_000_000,
            free: 1_200_000_000,
        };
        let lines = question_lines(&room);
        assert_eq!(lines[0], "Not enough room on build:");
        assert!(lines[1].starts_with("Needs "), "{lines:?}");
        assert!(lines[2].ends_with("y or n"), "{lines:?}");
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn every_remote_provider_is_timed_and_local_stays_as_it_is() {
        let mut registry = Registry::with_local();
        registry.register("slow", Arc::new(Slow(Duration::ZERO)));
        let timed = timed(&registry, SPACE_LIMIT);
        let slow = timed.resolve(&Place::parse("slow://h/").unwrap()).unwrap();
        assert!(format!("{slow:?}").starts_with("TimedSpace"));
        let here = if cfg!(windows) { "C:\\" } else { "/" };
        let local = timed.resolve(&Place::parse(here).unwrap()).unwrap();
        assert_eq!(local.scheme(), "file");
        assert!(!format!("{local:?}").starts_with("TimedSpace"));
    }
}
