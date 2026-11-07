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
use crate::metadata::{MetaAsk, MetaNote, Verified};
use diroxide_meta::checksum::list::{
    ChecksumList, MAX_LIST_BYTES, list_row, parse_list, sidecars_for,
};

/// The largest file Verify checksum reads to see whether its own lines
/// are a list, when no list sits beside it: 1 MiB.
const LIST_PROBE_BYTES: u64 = 1024 * 1024;

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
        MetaAsk::Checksums { job, .. }
        | MetaAsk::Verify { job, .. }
        | MetaAsk::Strip { job, .. }
        | MetaAsk::RemoveMark { job, .. } => Some(*job),
        MetaAsk::SaveList { .. }
        | MetaAsk::Check { .. }
        | MetaAsk::ReadMark { .. }
        | MetaAsk::RestoreMark { .. } => None,
    }
}

/// Carries out one ask on the worker it runs on; `tools` checks a
/// checksum list's signature (lane AE).
pub(super) fn run(
    shared: &Arc<Shared>,
    registry: &Registry,
    tools: &dyn diroxide_vfs::ToolRunner,
    ask: MetaAsk,
    cancel: CancelToken,
) {
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
        MetaAsk::Verify { job, place, list } => MetaNote::Verified {
            outcome: verify(
                shared,
                registry,
                tools,
                job,
                &place,
                list.as_deref(),
                &cancel,
            ),
            job,
            place,
        },
        MetaAsk::Strip { job, places } => strip(shared, registry, job, &places, &cancel),
        MetaAsk::Check { places, tools } => MetaNote::Checked {
            reports: check(registry, &places, &tools, &cancel),
        },
        MetaAsk::ReadMark { place } => MetaNote::Mark {
            result: with_provider(registry, &place, |provider, ctx| {
                diroxide_meta::origin::read_mark(provider, &place, ctx)
                    .map(|mark| mark.map(|m| m.field().line()))
            }),
            place,
        },
        MetaAsk::RemoveMark { job, place } => MetaNote::MarkRemoved {
            result: with_provider(registry, &place, |provider, ctx| {
                diroxide_meta::origin::remove_internet_mark(provider, &place, ctx)
            }),
            job,
            place,
        },
        MetaAsk::RestoreMark { place, removal } => MetaNote::MarkRestored {
            result: with_provider(registry, &place, |provider, ctx| {
                diroxide_meta::origin::restore_internet_mark(provider, &place, &removal, ctx)
            }),
            place,
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

/// Runs `work` with the place's provider and a plain context; the error
/// in words.
fn with_provider<T>(
    registry: &Registry,
    place: &Place,
    work: impl FnOnce(&dyn diroxide_vfs::Provider, &OpContext) -> Result<T, diroxide_meta::MetaError>,
) -> Result<T, String> {
    let provider = registry.resolve(place).map_err(|e| e.to_string())?;
    work(provider.as_ref(), &OpContext::new()).map_err(|e| e.to_string())
}

/// Strip metadata into a copy, picture by picture: the copy beside it as
/// `NAME-clean.EXT`, or the first numbered name no entry has (nor an
/// earlier copy of this job), never over a file; the originals stay.
fn strip(
    shared: &Arc<Shared>,
    registry: &Registry,
    job: JobId,
    places: &[Place],
    cancel: &CancelToken,
) -> MetaNote {
    let ctx = OpContext::new().with_cancel(cancel.clone());
    let mut taken: std::collections::HashMap<Place, Vec<String>> = Default::default();
    let mut results = Vec::new();
    let mut cancelled = false;
    for (done, place) in places.iter().enumerate() {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let name = place.name().unwrap_or_default().to_owned();
        shared.send(AppEvent::JobProgress {
            job,
            progress: JobProgress {
                kind: JobKind::StripMetadata,
                state: JobState::Running,
                items_done: done as u64,
                items_total: places.len() as u64,
                bytes_done: 0,
                bytes_total: None,
                bytes_per_second: None,
                seconds_left: None,
                current: Some(name.clone()),
                waits_for: None,
            },
        });
        let result =
            (|| -> Result<(Place, diroxide_meta::strip::StripReport), diroxide_meta::MetaError> {
                let folder = place
                    .parent()
                    .ok_or_else(|| diroxide_meta::MetaError::unreadable("it has no folder"))?;
                let provider = registry.resolve(place)?;
                if !taken.contains_key(&folder) {
                    let names = provider
                        .list(&folder, &ctx)?
                        .into_iter()
                        .map(|e| e.name)
                        .collect();
                    taken.insert(folder.clone(), names);
                }
                let names = taken.entry(folder.clone()).or_default();
                let wanted = diroxide_meta::strip::clean_copy_name(&name);
                let free = if names.iter().any(|n| n.eq_ignore_ascii_case(&wanted)) {
                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                    diroxide_vfs::names::unused_name(&wanted, &refs).ok_or_else(|| {
                        diroxide_meta::MetaError::unreadable("no free name is left")
                    })?
                } else {
                    wanted
                };
                let target = folder
                    .join(&free)
                    .map_err(|e| diroxide_meta::MetaError::unreadable(e.to_string()))?;
                let report =
                    diroxide_meta::strip::strip_into_copy(provider.as_ref(), place, &target, &ctx)?;
                names.push(free);
                Ok((target, report))
            })();
        match result {
            Err(diroxide_meta::MetaError::Place(VfsError::Cancelled)) => {
                cancelled = true;
                break;
            }
            other => results.push((place.clone(), other.map_err(|e| e.to_string()))),
        }
    }
    MetaNote::Stripped {
        job,
        results,
        cancelled,
    }
}

/// Check before sharing: each file's private details, read by the
/// details' readers under their guard; read only.
fn check(
    registry: &Registry,
    places: &[Place],
    tools: &crate::request::SharedTools,
    cancel: &CancelToken,
) -> Vec<diroxide_meta::privacy::PrivacyReport> {
    let pipeline = crate::details::pipeline(tools);
    let ctx = OpContext::new()
        .with_cancel(cancel.clone())
        .with_tools(Arc::clone(&tools.0));
    places
        .iter()
        .map(|place| {
            let name = place.name().unwrap_or_default();
            let provider = match registry.resolve(place) {
                Ok(provider) => provider,
                Err(error) => {
                    return diroxide_meta::privacy::report_of(name, &[], vec![error.to_string()]);
                }
            };
            let entry = provider.stat(place, &ctx).unwrap_or_else(|_| {
                diroxide_core::Entry::new(name, diroxide_core::EntryKind::File)
            });
            let request = diroxide_meta::ReadRequest {
                place,
                entry: &entry,
                type_id: None,
                group: None,
            };
            diroxide_meta::privacy::check(
                &pipeline,
                provider.as_ref(),
                &request,
                &diroxide_meta::ReadLimits::default(),
                &ctx,
            )
        })
        .collect()
}

/// Verify checksum: the list chosen, the file itself when it is a list,
/// or the list beside the file in R29's order (`NAME.sha256`, `.sha512`,
/// `.md5`, `.blake3`, then `SHA256SUMS`, `SHA512SUMS`, `MD5SUMS`,
/// `B3SUMS` and the same ending `.txt`); for a file, only its own line is
/// checked. Every file is read again; nothing is written.
fn verify(
    shared: &Arc<Shared>,
    registry: &Registry,
    tools: &dyn diroxide_vfs::ToolRunner,
    job: JobId,
    place: &Place,
    chosen: Option<&str>,
    cancel: &CancelToken,
) -> Result<Verified, String> {
    let folder = place
        .parent()
        .ok_or_else(|| "the file has no folder".to_owned())?;
    let name = place.name().unwrap_or_default().to_owned();
    let provider = registry.resolve(place).map_err(|e| e.to_string())?;
    let plain = OpContext::new().with_cancel(cancel.clone());
    let names: Vec<String> = provider
        .list(&folder, &plain)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|e| e.name)
        .collect();
    let found = |wanted: &str| {
        names
            .iter()
            .find(|n| n.as_str() == wanted)
            .or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case(wanted)))
            .cloned()
    };
    let (list_name, only) = match chosen {
        Some(list) => (list.to_owned(), Some(name.clone())),
        None if crate::metadata::is_list_name(&name) => (name.clone(), None),
        // A small file whose own lines are all a list's, whatever its name
        // ("checksums.txt"), is checked as one.
        None if provider
            .stat(place, &plain)
            .is_ok_and(|e| e.size.is_some_and(|s| s <= LIST_PROBE_BYTES))
            && read_list(provider.as_ref(), &folder, &name, &plain)
                .is_ok_and(|l| !l.entries.is_empty() && l.problems.is_empty()) =>
        {
            (name.clone(), None)
        }
        None => {
            let lists: Vec<String> = sidecars_for(&name)
                .iter()
                .filter_map(|candidate| found(candidate))
                .collect();
            match lists.as_slice() {
                [] => return Ok(Verified::NoList),
                [one] => (one.clone(), Some(name.clone())),
                many => {
                    let rows = many
                        .iter()
                        .map(|list| {
                            let parsed = read_list(provider.as_ref(), &folder, list, &plain)
                                .unwrap_or_default();
                            (list.clone(), list_row(list, &parsed))
                        })
                        .collect();
                    return Ok(Verified::Choose(rows));
                }
            }
        }
    };
    let mut parsed = read_list(provider.as_ref(), &folder, &list_name, &plain)?;
    if let Some(only) = &only {
        parsed
            .entries
            .retain(|entry| entry.parts() == [only.as_str()]);
        parsed.problems.clear();
        if parsed.entries.is_empty() {
            return Ok(Verified::NoLine(list_name));
        }
    }
    let sink = Arc::new(JobSink {
        shared: Arc::clone(shared),
        job,
        kind: JobKind::Verify,
        base: AtomicU64::new(0),
        total: None,
        last: Mutex::new(None),
    });
    let ctx = OpContext::new()
        .with_cancel(cancel.clone())
        .with_progress(sink as Arc<dyn ProgressSink>);
    match diroxide_meta::checksum::verify::verify_parsed(
        provider.as_ref(),
        &folder,
        &list_name,
        parsed,
        &ctx,
    ) {
        // The chain (lane AE, R29 3.5 step 6): the list's own signature
        // beside it, checked through gpg or minisign.
        Ok(report) => Ok(Verified::Report {
            report: Box::new(report),
            signature: list_signature(tools, &folder, &list_name, &names, cancel),
        }),
        Err(diroxide_meta::MetaError::Place(VfsError::Cancelled)) => Ok(Verified::Cancelled),
        Err(error) => Err(error.to_string()),
    }
}

/// The verdict on the list's own signature beside it (`SHA256SUMS.asc`,
/// `.sig` or `.minisig`), when one is there (lane AE).
fn list_signature(
    tools: &dyn diroxide_vfs::ToolRunner,
    folder: &Place,
    list: &str,
    names: &[String],
    cancel: &CancelToken,
) -> Option<Box<diroxide_ops::crypt::SignatureVerdict>> {
    use diroxide_ops::crypt::{SignatureCheck, sidecars_present, verify_signature};
    let siblings: Vec<&str> = names.iter().map(String::as_str).collect();
    let sidecar = sidecars_present(list, &siblings)
        .into_iter()
        .find(|sidecar| sidecar.signs == list)?;
    let check = SignatureCheck::detached(folder.join(&sidecar.name).ok()?, folder.join(list).ok()?);
    Some(Box::new(verify_signature(tools, &check, cancel)))
}

/// Reads and parses a list in a folder, at most 16 MiB.
fn read_list(
    provider: &dyn diroxide_vfs::Provider,
    folder: &Place,
    name: &str,
    ctx: &OpContext,
) -> Result<ChecksumList, String> {
    let place = folder.join(name).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(
            provider.read(&place, ctx).map_err(|e| e.to_string())?,
            MAX_LIST_BYTES as u64 + 1,
        ),
        &mut bytes,
    )
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_LIST_BYTES {
        return Err(format!("{name} is larger than 16 MiB"));
    }
    Ok(parse_list(&bytes, name))
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
