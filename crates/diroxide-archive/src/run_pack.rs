//! The pack runner (plan commits 32 to 35, research/archives.md 8.5 and
//! 8.6): the new archive is written under a temporary name beside its
//! destination (`.diroxide-` and the job's number and `.tmp`) through the
//! provider's committing writer, finished, read back and checked against
//! what was written ("Checked 300 of 300 entries, all good."), and only
//! then renamed into place. A cancel or a failure at any step removes the
//! temporary archive and leaves an old file of the same name untouched; a
//! replace sends the old one to the trash only after the new one passed
//! its test.
//!
//! Progress counts the bytes read from the sources, in the engine's
//! words: "Packing 30 percent, 2 minutes left".

use std::io::{self, Read, SeekFrom, Write};
use std::sync::Mutex;

use diroxide_core::{Entry, EntryKind, JobKind, JobState, Place, Secret, Timestamp};
use diroxide_ops::{JobControl, JobReport};
use diroxide_vfs::{OpContext, Provider, ReadSeek, Registry, TrashCheck, VfsError, WriteMode};

use crate::format::ArchiveFormat;
use crate::guard::Allowances;
use crate::job::{JobEnv, Landing, Progress, Settler};
use crate::pack::{PACK_TEMP_PREFIX, PACK_TEMP_SUFFIX, PackPlan};
use crate::secrets::JobSecrets;
use crate::test_archive::{Written, check_written};
use crate::write::{Contents, ItemKind, PackFailure, Records, Settings, Watch, cancelled};

/// What a pack did: the engine's report (its undo record holds the new
/// archive, and the old one when it went to the trash), the lines to say,
/// and where the archive landed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackOutcome {
    /// The report.
    pub report: JobReport,
    /// The summary first, then the check: "Pack finished: 300 done, 2
    /// skipped.", "Checked 300 of 300 entries, all good."
    pub lines: Vec<String>,
    /// Where the new archive landed, when it did.
    pub archive: Option<Place>,
}

/// The sources' bytes, read through their providers.
struct FromProviders<'a> {
    registry: &'a Registry,
    places: Vec<Place>,
    ctx: &'a OpContext,
}

impl Contents for FromProviders<'_> {
    fn open(&self, n: usize) -> Result<Box<dyn Read + Send + '_>, VfsError> {
        let place = self.places.get(n).ok_or(VfsError::NotFound)?;
        let provider = self.registry.resolve(place)?;
        provider.read(place, self.ctx)
    }
}

/// The job's watch over the bytes going in: pause and cancel between
/// reads, the progress in the engine's words.
struct Meter<'a> {
    progress: Mutex<Progress<'a>>,
    control: &'a JobControl,
}

impl Meter<'_> {
    fn with<T>(&self, f: impl FnOnce(&mut Progress<'_>) -> T) -> T {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut progress)
    }
}

impl Watch for Meter<'_> {
    fn check(&self) -> io::Result<()> {
        let control = self.control;
        if self.with(|progress| progress.hold(control)) {
            Ok(())
        } else {
            Err(cancelled())
        }
    }

    fn bytes(&self, bytes: u64) {
        self.with(|progress| progress.bytes(bytes));
    }

    fn member(&self, name: &str) {
        self.with(|progress| {
            progress.current(name);
            progress.item_done();
        });
    }
}

/// Notes a failure, or the cancel it is.
fn failed(report: &mut JobReport, place: &Place, error: VfsError) {
    if error == VfsError::Cancelled {
        report.cancelled = true;
    } else {
        report.failed.push((place.clone(), error.to_string()));
    }
}

/// A temporary name beside the destination that nothing holds yet.
fn free_temp(provider: &dyn Provider, plan: &PackPlan, ctx: &OpContext) -> Result<Place, VfsError> {
    let folder = plan.destination.parent().ok_or(VfsError::NotFound)?;
    for attempt in 0..64u32 {
        let name = if attempt == 0 {
            format!("{PACK_TEMP_PREFIX}{}{PACK_TEMP_SUFFIX}", plan.job.get())
        } else {
            format!(
                "{PACK_TEMP_PREFIX}{}-{attempt}{PACK_TEMP_SUFFIX}",
                plan.job.get()
            )
        };
        let place = folder.join(&name)?;
        match provider.stat(&place, ctx) {
            Err(VfsError::NotFound) => return Ok(place),
            Ok(_) => {}
            Err(error) => return Err(error),
        }
    }
    Err(VfsError::AlreadyExists)
}

/// Runs a pack plan: see the module documentation. `password` is the one
/// typed (twice) in the pack dialog for zip or 7z; it is used for the
/// writing and the test and dropped with the job.
pub fn run_pack(plan: &PackPlan, password: Option<&Secret>, env: &JobEnv<'_>) -> PackOutcome {
    let mut report = JobReport::new(env.job, JobKind::Pack);
    for (place, reason) in &plan.left_out {
        report.skipped.push((place.clone(), reason.clone()));
    }
    let finish = |report: JobReport, extra: Vec<String>, archive: Option<Place>| {
        let mut lines = vec![report.summary()];
        lines.extend(extra);
        PackOutcome {
            report,
            lines,
            archive,
        }
    };
    let provider = match env.provider(&plan.destination) {
        Ok(provider) => provider,
        Err(error) => {
            report
                .failed
                .push((plan.destination.clone(), error.to_string()));
            return finish(report, Vec::new(), None);
        }
    };
    let provider = provider.as_ref();
    let secrets = JobSecrets::knowing(env.secrets.clone(), password.cloned());
    let ctx = env.context(secrets);
    let items = plan.items.len() as u64;
    let meter = Meter {
        progress: Mutex::new(Progress::start(env, JobKind::Pack, items, Some(plan.bytes))),
        control: env.control,
    };
    let temp = match free_temp(provider, plan, &ctx) {
        Ok(temp) => temp,
        Err(error) => {
            failed(&mut report, &plan.destination, error);
            meter.with(|progress| progress.finish());
            return finish(report, Vec::new(), None);
        }
    };
    let records = Records::new(plan.items.len());
    let settings = Settings {
        format: plan.format,
        level: plan.level,
        solid: plan.solid,
        block_bytes: plan.block_bytes,
        password: password.map(Secret::expose),
        encrypt_names: plan.encrypt_names,
        threads: plan.threads,
        keep_owner: plan.keep_owner,
    };
    let contents = FromProviders {
        registry: env.engine.registry(),
        places: plan.items.iter().map(|item| item.source.clone()).collect(),
        ctx: &ctx,
    };
    let written = write_temp(
        provider, &temp, plan, &contents, &settings, &meter, &records, &ctx,
    );
    if let Err(failure) = written {
        match failure {
            PackFailure::Cancelled => report.cancelled = true,
            PackFailure::Source(n, error) => {
                let place = plan
                    .items
                    .get(n)
                    .map_or_else(|| plan.destination.clone(), |i| i.source.clone());
                failed(&mut report, &place, error);
            }
            PackFailure::Write(error) => failed(&mut report, &plan.destination, error),
        }
        meter.with(|progress| progress.finish());
        return finish(report, Vec::new(), None);
    }
    let mut extra = Vec::new();
    if plan.test_after {
        meter.with(|progress| progress.state(JobState::Verifying));
        let expected: Vec<Written> = plan
            .write_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == ItemKind::File)
            .filter_map(|(n, item)| {
                records.get(n).map(|(size, crc32)| Written {
                    path: item.name.clone(),
                    size,
                    crc32,
                })
            })
            .collect();
        let checked = provider
            .open_seekable(&temp, &ctx)
            .map_err(|e| e.to_string())
            .and_then(|bytes: Box<dyn ReadSeek>| {
                env.archives
                    .open_bytes(bytes, &plan.destination, &ctx, Allowances::all())
                    .map_err(|e| format!("the new archive could not be read back: {e}"))
            })
            .and_then(|opened| check_written(env.archives, &opened, &expected, &ctx));
        match checked {
            Ok(line) => extra.push(line),
            Err(problem) => {
                let _ = provider.remove(&temp, &OpContext::new());
                report.failed.push((plan.destination.clone(), problem));
                meter.with(|progress| progress.finish());
                return finish(report, Vec::new(), None);
            }
        }
    }
    let landed = land(provider, &temp, plan, env, &ctx, &mut report);
    meter.with(|progress| progress.finish());
    match landed {
        Some(place) => {
            for item in &plan.items {
                report.done.push(item.source.clone());
            }
            report.undo.created.push(place.clone());
            finish(report, extra, Some(place))
        }
        None => {
            let _ = provider.remove(&temp, &OpContext::new());
            finish(report, Vec::new(), None)
        }
    }
}

/// Writes the archive under its temporary name and commits it there.
#[allow(clippy::too_many_arguments)]
fn write_temp(
    provider: &dyn Provider,
    temp: &Place,
    plan: &PackPlan,
    contents: &dyn Contents,
    settings: &Settings<'_>,
    watch: &dyn Watch,
    records: &Records,
    ctx: &OpContext,
) -> Result<(), PackFailure> {
    if plan.format == ArchiveFormat::SevenZ {
        return write_temp_7z(
            provider, temp, plan, contents, settings, watch, records, ctx,
        );
    }
    let writer = provider
        .write(temp, WriteMode::CreateNew, ctx)
        .map_err(PackFailure::Write)?;
    let writer = crate::write::write_archive(
        writer,
        &plan.write_items,
        contents,
        settings,
        watch,
        records,
    )?;
    writer.commit(true).map_err(PackFailure::Write)
}

/// The most of a 7z's body held in memory before it goes to a body file
/// beside the destination.
const BODY_IN_MEMORY: usize = 64 << 20;

/// A 7z's body on its way, for a committing writer that cannot seek: in
/// memory while it is small, then in a body file beside the destination.
/// The 7z writer must put its start header in front once it knows it,
/// which such a writer cannot seek back to; the body is copied behind the
/// header into the temporary archive when it is whole. A writer that
/// seeks (`FileWriter::seek_to`, the local one) takes the 7z straight,
/// with no body file and no copy (P2b, W2b's request 1).
enum Spill<'p> {
    Memory {
        bytes: Vec<u8>,
        limit: usize,
        provider: &'p dyn Provider,
        place: Place,
        ctx: &'p OpContext,
    },
    File {
        writer: Box<dyn diroxide_vfs::FileWriter>,
        provider: &'p dyn Provider,
        place: Place,
        ctx: &'p OpContext,
    },
    Moved,
}

impl Write for Spill<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Spill::Memory { bytes, limit, .. } = self {
            if bytes.len() + buf.len() <= *limit {
                bytes.extend_from_slice(buf);
                return Ok(buf.len());
            }
            let Spill::Memory {
                bytes,
                provider,
                place,
                ctx,
                ..
            } = std::mem::replace(self, Spill::Moved)
            else {
                return Err(io::Error::other("the body went missing"));
            };
            let mut writer = provider
                .write(&place, WriteMode::CreateNew, ctx)
                .map_err(crate::guard::to_io)?;
            writer.write_all(&bytes)?;
            *self = Spill::File {
                writer,
                provider,
                place,
                ctx,
            };
        }
        match self {
            Spill::File { writer, .. } => writer.write(buf),
            _ => Err(io::Error::other("the body went missing")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Spill::File { writer, .. } => writer.flush(),
            _ => Ok(()),
        }
    }
}

impl Spill<'_> {
    /// Copies the body behind the header already written to `out`, and
    /// removes the body file, if there was one.
    fn finish_into(self, out: &mut dyn Write, watch: &dyn Watch) -> Result<(), PackFailure> {
        match self {
            Spill::Memory { bytes, .. } => out.write_all(&bytes).map_err(PackFailure::from_io),
            Spill::File {
                writer,
                provider,
                place,
                ctx,
            } => {
                let copied = writer
                    .commit(false)
                    .map_err(PackFailure::Write)
                    .and_then(|()| provider.read(&place, ctx).map_err(PackFailure::Write))
                    .and_then(|mut reader| {
                        let mut chunk = vec![0u8; 1 << 20];
                        loop {
                            watch.check().map_err(PackFailure::from_io)?;
                            let n = reader.read(&mut chunk).map_err(PackFailure::from_io)?;
                            if n == 0 {
                                return Ok(());
                            }
                            out.write_all(&chunk[..n]).map_err(PackFailure::from_io)?;
                        }
                    });
                // The body file is the job's own, done with either way.
                let _ = provider.remove(&place, &OpContext::new());
                copied
            }
            Spill::Moved => Err(PackFailure::Write(VfsError::NotFound)),
        }
    }
}

/// Writes a 7z under its temporary name: straight into the committing
/// writer when it seeks, else the body through [`Spill`], then the start
/// header and the body into the temporary archive.
#[allow(clippy::too_many_arguments)]
fn write_temp_7z(
    provider: &dyn Provider,
    temp: &Place,
    plan: &PackPlan,
    contents: &dyn Contents,
    settings: &Settings<'_>,
    watch: &dyn Watch,
    records: &Records,
    ctx: &OpContext,
) -> Result<(), PackFailure> {
    let mut out = provider
        .write(temp, WriteMode::CreateNew, ctx)
        .map_err(PackFailure::Write)?;
    if out.seek_to(SeekFrom::Current(0)).is_ok() {
        let crate::write_7z::Seeking(out) = crate::write_7z::write_7z(
            crate::write_7z::Seeking(out),
            &plan.write_items,
            contents,
            settings,
            watch,
            records,
        )?;
        return out.commit(true).map_err(PackFailure::Write);
    }
    let folder = temp
        .parent()
        .ok_or(PackFailure::Write(VfsError::NotFound))?;
    let body = folder
        .join(&format!(
            "{PACK_TEMP_PREFIX}{}-body{PACK_TEMP_SUFFIX}",
            plan.job.get()
        ))
        .map_err(|e| PackFailure::Write(e.into()))?;
    if provider.stat(&body, ctx).is_ok() {
        return Err(PackFailure::Write(VfsError::AlreadyExists));
    }
    let spill = Spill::Memory {
        bytes: Vec::new(),
        limit: BODY_IN_MEMORY,
        provider,
        place: body,
        ctx,
    };
    let holder = crate::write_7z::HeadHolder::new(spill);
    let holder = crate::write_7z::write_7z(
        holder,
        &plan.write_items,
        contents,
        settings,
        watch,
        records,
    )?;
    let (head, spill) = holder.into_parts();
    out.write_all(&head).map_err(PackFailure::from_io)?;
    spill.finish_into(&mut out, watch)?;
    out.commit(true).map_err(PackFailure::Write)
}

/// Gives the tested archive its name: free, or as the conflict question
/// settles it. Returns where it landed, or `None` when it did not (the
/// temporary archive is then removed by the caller).
fn land(
    provider: &dyn Provider,
    temp: &Place,
    plan: &PackPlan,
    env: &JobEnv<'_>,
    ctx: &OpContext,
    report: &mut JobReport,
) -> Option<Place> {
    let mut entry = Entry::new(plan.archive_name(), EntryKind::File);
    entry.size = provider.stat(temp, ctx).ok().and_then(|e| e.size);
    entry.modified = Some(Timestamp::from_system_time(std::time::SystemTime::now()));
    let from = plan
        .sources
        .first()
        .cloned()
        .unwrap_or_else(|| temp.clone());
    let mut settler = Settler::new(plan.conflicts, env.engine.options().time_tolerance_seconds);
    let landing = match settler.settle(provider, ctx, &from, &entry, &plan.destination, env.sink) {
        Ok(landing) => landing,
        Err(error) => {
            failed(report, &plan.destination, error);
            return None;
        }
    };
    let target = match landing {
        Landing::Free(target) => target,
        Landing::Replace(target) => {
            let keep = env.engine.options().replace_keeps_old_in_trash
                && provider.capabilities(&target).trash;
            let replaced = if keep {
                match provider.trash(&target, ctx) {
                    Ok(TrashCheck::InTrash | TrashCheck::NotChecked) => {
                        report.undo.trashed.push(target.clone());
                        Ok(())
                    }
                    Ok(TrashCheck::GoneNotInTrash) => {
                        report.warnings.push((
                            target.clone(),
                            diroxide_core::catalog::TRASH_NOT_CONFIRMED.to_owned(),
                        ));
                        Ok(())
                    }
                    Ok(_) => Err("the old archive is still there after the trash".to_owned()),
                    Err(error) => Err(error.to_string()),
                }
            } else {
                report
                    .warnings
                    .push((target.clone(), "replaced; this cannot be undone".to_owned()));
                provider.remove(&target, ctx).map_err(|e| e.to_string())
            };
            if let Err(problem) = replaced {
                report.failed.push((target, problem));
                return None;
            }
            target
        }
        Landing::Skip(reason) => {
            report.skipped.push((plan.destination.clone(), reason));
            return None;
        }
        Landing::Cancel => {
            report.cancelled = true;
            return None;
        }
    };
    match provider.rename(temp, &target, ctx) {
        Ok(()) => Some(target),
        Err(error) => {
            if report.undo.trashed.last() == Some(&target) {
                let _ = provider.restore(&target, &OpContext::new());
                report.undo.trashed.pop();
            }
            report.failed.push((target, error.to_string()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::NoWatch;
    use diroxide_ops::FakeProvider;

    #[test]
    fn a_large_body_spills_to_a_file_and_comes_back() {
        let fake = FakeProvider::new();
        let folder = Place::parse(if cfg!(windows) { r"D:\out" } else { "/out" }).unwrap();
        fake.add_folder(&folder);
        let body = folder.join(".diroxide-9-body.tmp").unwrap();
        let ctx = OpContext::new();
        let mut spill = Spill::Memory {
            bytes: Vec::new(),
            limit: 10,
            provider: &fake,
            place: body.clone(),
            ctx: &ctx,
        };
        spill.write_all(b"0123456789").unwrap();
        assert!(matches!(spill, Spill::Memory { .. }));
        spill.write_all(b"abcdef").unwrap();
        assert!(matches!(spill, Spill::File { .. }));
        let mut out = Vec::new();
        out.extend_from_slice(b"HEAD");
        spill.finish_into(&mut out, &NoWatch).unwrap();
        assert_eq!(out, b"HEAD0123456789abcdef");
        assert!(!fake.exists(&body), "the body file is removed");
        assert!(fake.unfinished().is_empty());
    }
}
