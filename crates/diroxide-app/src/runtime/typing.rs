//! The readers of the background type check (alpha 4, lane TA, R30
//! commit 29): the first [`HEAD_BYTES`] of each file in the order the app
//! gave, typed on the reader's thread (a type's tests cost about 45
//! microseconds a file in spike F1, too much for the input thread), the
//! answers sent in batches: a small first batch for the rows on screen,
//! then up to [`BATCH`] at a time or every [`BATCH_WAIT`].
//!
//! Four readers read a disk of this computer at once and one a share
//! ([`crate::typing::LOCAL_READERS`], [`crate::typing::SHARE_READERS`]).
//! Each panel's checks are counted; a newer check of the panel, or a stop,
//! moves the count on, and a reader of an older one leaves at its next
//! file, its unsent answers dropped.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use diroxide_core::{Clock, Place, Side};
use diroxide_filetype::{HEAD_BYTES, TypeRegistry};
use diroxide_vfs::{OpContext, Registry, VolumeKind};

use super::Shared;
use crate::app::AppEvent;
use crate::typing::{LOCAL_READERS, SHARE_READERS, TypeItem, check_item};

/// The most answers in one batch after the first.
pub(super) const BATCH: usize = 512;

/// The longest a reader keeps answers before it sends them.
pub(super) const BATCH_WAIT: Duration = Duration::from_millis(200);

/// Each panel's count of checks.
#[derive(Debug, Default)]
pub(super) struct TypeReaders {
    counts: [Arc<AtomicU64>; 2],
}

impl TypeReaders {
    /// A new check of a panel: its number, which stops the readers of the
    /// check before.
    pub(super) fn next(&self, side: Side) -> u64 {
        self.counts[side.index()].fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The count a reader compares its check's number with.
    pub(super) fn current(&self, side: Side) -> Arc<AtomicU64> {
        Arc::clone(&self.counts[side.index()])
    }
}

/// One check: what its readers share.
pub(super) struct TypeJob {
    /// The panel.
    pub side: Side,
    /// The panel's entries the items belong to.
    pub listing: u64,
    /// This check's number.
    pub generation: u64,
    /// The panel's count: while it equals `generation`, the check runs.
    pub current: Arc<AtomicU64>,
    /// The folder.
    pub folder: Place,
    /// The files, in the order to read them.
    pub items: Vec<TypeItem>,
    /// The next item to take.
    pub next: AtomicUsize,
    /// How many first items are on screen.
    pub urgent: usize,
    /// The app's types.
    pub types: Arc<TypeRegistry>,
    /// The app's clock.
    pub clock: Arc<dyn Clock>,
    /// Where the files are read from.
    pub registry: Arc<Registry>,
}

impl TypeJob {
    fn running(&self) -> bool {
        self.current.load(Ordering::SeqCst) == self.generation
    }
}

/// Runs a check: decides how many readers the folder's disk takes, and
/// runs them to the end or until a newer check stops them.
pub(super) fn run(shared: &Arc<Shared>, job: &TypeJob) {
    if !job.running() {
        return;
    }
    let readers = readers_for(&job.registry, &job.folder).min(job.items.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..readers {
            scope.spawn(|| read(shared, job, readers));
        }
    });
}

/// How many readers a folder's disk takes: one on a share, four on a disk
/// of this computer.
fn readers_for(registry: &Registry, folder: &Place) -> usize {
    let text = folder.to_string();
    if text.starts_with(r"\\") || text.starts_with("//") {
        return SHARE_READERS;
    }
    let ctx = OpContext::new();
    match registry
        .resolve(folder)
        .and_then(|provider| provider.volume(folder, &ctx))
    {
        Ok(info) if info.kind == VolumeKind::Network => SHARE_READERS,
        _ => LOCAL_READERS,
    }
}

/// One reader: takes the next file until none is left or the check is
/// stopped.
fn read(shared: &Arc<Shared>, job: &TypeJob, readers: usize) {
    let ctx = OpContext::new();
    let mut batch = Vec::new();
    let mut since = Instant::now();
    let mut limit = job.urgent.div_ceil(readers).max(1);
    loop {
        if !job.running() {
            return;
        }
        let i = job.next.fetch_add(1, Ordering::SeqCst);
        let Some(item) = job.items.get(i) else {
            break;
        };
        let head = job
            .folder
            .join(&item.name)
            .ok()
            .and_then(|place| first_bytes(&job.registry, &place, &ctx));
        batch.push(check_item(
            &job.types,
            item,
            head.as_deref(),
            job.clock.as_ref(),
        ));
        if batch.len() >= limit || since.elapsed() >= BATCH_WAIT {
            if !job.running() {
                return;
            }
            send(shared, job, std::mem::take(&mut batch));
            since = Instant::now();
            limit = BATCH;
        }
    }
    if !batch.is_empty() && job.running() {
        send(shared, job, batch);
    }
}

fn send(shared: &Arc<Shared>, job: &TypeJob, answers: Vec<crate::typing::TypeAnswer>) {
    shared.send(AppEvent::TypesChecked {
        side: job.side,
        listing: job.listing,
        answers,
    });
}

/// The first bytes of a file, at most [`HEAD_BYTES`]; `None` when it
/// could not be read.
pub(super) fn first_bytes(registry: &Registry, place: &Place, ctx: &OpContext) -> Option<Vec<u8>> {
    let provider = registry.resolve(place).ok()?;
    let mut reader = provider.read(place, ctx).ok()?;
    let mut head = vec![0; HEAD_BYTES];
    let mut got = 0;
    while got < HEAD_BYTES {
        match reader.read(&mut head[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    head.truncate(got);
    Some(head)
}

#[cfg(test)]
mod tests {
    use diroxide_core::{Entry, EntryKind, SystemClock};
    use diroxide_ops::{FakeProvider, Latency};

    use super::*;
    use crate::request::{Request, SharedClock};
    use crate::runtime::Runtime;

    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    /// Lane TA, commit 29: a newer check of a panel (a scroll that moved
    /// the focus a page away) stops the readers of the one before; a
    /// share gets one reader, a disk of this computer four.
    #[test]
    fn a_newer_check_stops_the_readers_of_the_one_before() {
        let fake = FakeProvider::new();
        let folder = p("/work");
        fake.add_folder(&folder);
        let mut items = Vec::new();
        for i in 0..200 {
            let name = format!("f{i:03}");
            fake.add_file(&folder.join(&name).unwrap(), b"#!/bin/sh\necho hi\n");
            items.push(TypeItem::of(
                i,
                &Entry::new(name, EntryKind::File).with_size(18),
            ));
        }
        fake.switches_mut().latency = Latency {
            per_call: Duration::from_millis(5),
            ..Latency::default()
        };
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        let registry = Arc::new(registry);
        assert_eq!(readers_for(&registry, &folder), LOCAL_READERS);
        let mut runtime = Runtime::new(Arc::clone(&registry));
        let types = Arc::new(TypeRegistry::builtin());
        let clock = SharedClock(Arc::new(SystemClock::utc()));
        let ask = |listing: u64, items: Vec<TypeItem>| Request::CheckTypes {
            side: Side::Left,
            listing,
            folder: folder.clone(),
            items,
            urgent: 4,
            types: Arc::clone(&types),
            clock: clock.clone(),
        };
        runtime.request(ask(1, items.clone()));
        runtime.request(ask(2, items[197..].to_vec()));
        let (mut first, mut second) = (0, Vec::new());
        loop {
            let events = runtime.settle();
            if events.is_empty() {
                break;
            }
            for event in events {
                if let AppEvent::TypesChecked {
                    listing, answers, ..
                } = event
                {
                    if listing == 1 {
                        first += answers.len();
                    } else {
                        second.extend(answers);
                    }
                }
            }
        }
        assert_eq!(second.len(), 3);
        assert!(first < 200, "the first check read {first} of 200");
        for answer in &second {
            let (index, stage) = answer.found.unwrap();
            assert_eq!(stage, diroxide_filetype::TypeStage::Condition);
            assert_eq!(types.by_index(index).unwrap().id, "shell");
        }
        fake.switches_mut().volume_kind = Some(VolumeKind::Network);
        assert_eq!(readers_for(&registry, &folder), SHARE_READERS);
    }
}
