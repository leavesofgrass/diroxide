//! The metadata commands' workers (alpha 4, lane MA, R30 commits 54 to
//! 56): each runs `diroxide-meta`'s engine with a context whose cancel
//! token Cancel trips ([`super::Runtime`]'s `meta_runs`) and whose
//! progress becomes the job's progress line, at most four times a second.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_core::{JobId, JobKind, JobState, Place};
use diroxide_meta::checksum::{Algorithm, Digest};
use diroxide_ops::JobProgress;
use diroxide_vfs::{
    CancelToken, OpContext, ProgressSink, ProgressUpdate, Registry, VfsError, WriteMode,
};

use super::Shared;
use crate::app::AppEvent;
use crate::metadata::{MetaAsk, MetaNote};

/// The shortest time between two progress lines a worker sends.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Turns a worker's progress into its job's progress line: bytes of all
/// the files, the one at work named.
struct JobSink {
    shared: Arc<Shared>,
    job: JobId,
    kind: JobKind,
    /// The bytes of the files before the one at work.
    base: AtomicU64,
    /// The bytes of all the files, when known.
    total: Option<u64>,
    last: Mutex<Option<Instant>>,
}

impl ProgressSink for JobSink {
    fn report(&self, update: ProgressUpdate) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < PROGRESS_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        drop(last);
        let base = self.base.load(Ordering::SeqCst);
        self.shared.send(AppEvent::JobProgress {
            job: self.job,
            progress: JobProgress {
                kind: self.kind,
                state: JobState::Running,
                items_done: update.items_done,
                items_total: update.items_total.unwrap_or_default(),
                bytes_done: base + update.bytes_done,
                bytes_total: self.total.or(update.bytes_total),
                bytes_per_second: None,
                seconds_left: None,
                current: update.current,
                waits_for: None,
            },
        });
    }
}

/// The job an ask runs as, when it is one: its cancel token is kept for
/// Cancel until the answer.
pub(super) fn job_of(ask: &MetaAsk) -> Option<JobId> {
    match ask {
        MetaAsk::Checksums { job, .. } => Some(*job),
        MetaAsk::SaveList { .. } => None,
    }
}

/// Carries out one ask on the worker it runs on.
pub(super) fn run(shared: &Arc<Shared>, registry: &Registry, ask: MetaAsk, cancel: CancelToken) {
    let note = match ask {
        MetaAsk::Checksums {
            job,
            places,
            algorithms,
        } => checksums(shared, registry, job, &places, &algorithms, &cancel),
        MetaAsk::SaveList { folder, name, text } => MetaNote::Saved {
            result: save_list(registry, &folder, &name, &text),
            folder,
        },
    };
    shared.send(AppEvent::Meta(Box::new(note)));
}

/// Each file's checksums, every algorithm in one read.
fn checksums(
    shared: &Arc<Shared>,
    registry: &Registry,
    job: JobId,
    places: &[Place],
    algorithms: &[Algorithm],
    cancel: &CancelToken,
) -> MetaNote {
    let plain = OpContext::new().with_cancel(cancel.clone());
    let sizes: Vec<Option<u64>> = places
        .iter()
        .map(|place| {
            registry
                .resolve(place)
                .ok()
                .and_then(|p| p.stat(place, &plain).ok())
                .and_then(|e| e.size)
        })
        .collect();
    let sink = Arc::new(JobSink {
        shared: Arc::clone(shared),
        job,
        kind: JobKind::Checksum,
        base: AtomicU64::new(0),
        total: sizes.iter().copied().sum(),
        last: Mutex::new(None),
    });
    let ctx = OpContext::new()
        .with_cancel(cancel.clone())
        .with_progress(Arc::clone(&sink) as Arc<dyn ProgressSink>);
    let mut results: Vec<(Place, Result<Vec<Digest>, String>)> = Vec::new();
    let mut cancelled = false;
    for (place, size) in places.iter().zip(&sizes) {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let result = registry
            .resolve(place)
            .map_err(diroxide_meta::MetaError::from)
            .and_then(|provider| {
                diroxide_meta::checksum::compute(provider.as_ref(), place, algorithms, &ctx)
            });
        match result {
            Err(diroxide_meta::MetaError::Place(VfsError::Cancelled)) => {
                cancelled = true;
                break;
            }
            other => results.push((place.clone(), other.map_err(|e| e.to_string()))),
        }
        sink.base
            .fetch_add(size.unwrap_or_default(), Ordering::SeqCst);
    }
    MetaNote::Checksums {
        job,
        results,
        cancelled,
    }
}

/// Writes a list into a folder under its name, or the first numbered
/// name no entry has ("SHA256SUMS (2)"), through the folder's provider,
/// never over anything; the name it got.
fn save_list(
    registry: &Registry,
    folder: &Place,
    name: &str,
    text: &str,
) -> Result<String, String> {
    let ctx = OpContext::new();
    let provider = registry.resolve(folder).map_err(|e| e.to_string())?;
    let names: Vec<String> = provider
        .list(folder, &ctx)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|e| e.name)
        .collect();
    let taken: Vec<&str> = names.iter().map(String::as_str).collect();
    let free = if taken.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        diroxide_vfs::names::unused_name(name, &taken)
            .ok_or_else(|| format!("no free name for {name} is left"))?
    } else {
        name.to_owned()
    };
    let place = folder.join(&free).map_err(|e| e.to_string())?;
    let mut writer = provider
        .write(&place, WriteMode::CreateNew, &ctx)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut writer, text.as_bytes()).map_err(|e| e.to_string())?;
    writer.commit(true).map_err(|e| e.to_string())?;
    Ok(free)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::runtime::Runtime;

    #[test]
    fn a_list_is_saved_under_a_free_name_and_never_over_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SHA256SUMS"), b"old").unwrap();
        let folder = Place::parse(dir.path().to_str().unwrap()).unwrap();
        let registry = Registry::with_local();
        let first = save_list(&registry, &folder, "SHA256SUMS", "a  b\n").unwrap();
        assert_eq!(first, "SHA256SUMS (2)");
        assert_eq!(
            std::fs::read(dir.path().join("SHA256SUMS")).unwrap(),
            b"old"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("SHA256SUMS (2)")).unwrap(),
            "a  b\n"
        );
        let other = save_list(&registry, &folder, "B3SUMS", "x\n").unwrap();
        assert_eq!(other, "B3SUMS");
    }

    #[test]
    fn checksums_come_back_for_each_file_with_progress() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"abc").unwrap();
        let a = Place::parse(dir.path().join("a.txt").to_str().unwrap()).unwrap();
        let gone = Place::parse(dir.path().join("gone.txt").to_str().unwrap()).unwrap();
        let mut runtime = Runtime::new(Arc::new(Registry::with_local()));
        runtime.request(Request::Meta(Box::new(MetaAsk::Checksums {
            job: JobId::new(9),
            places: vec![a.clone(), gone.clone()],
            algorithms: vec![Algorithm::Sha256],
        })));
        let mut note = None;
        loop {
            let events = runtime.settle();
            if events.is_empty() {
                break;
            }
            for event in events {
                if let AppEvent::Meta(found) = event {
                    note = Some(*found);
                }
            }
        }
        let Some(MetaNote::Checksums {
            job,
            results,
            cancelled,
        }) = note
        else {
            panic!("no checksums");
        };
        assert_eq!(job, JobId::new(9));
        assert!(!cancelled);
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].1.as_ref().unwrap()[0].hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(results[1].1.is_err(), "{:?}", results[1]);
    }
}
