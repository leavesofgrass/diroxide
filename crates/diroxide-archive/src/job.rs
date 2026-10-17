//! What the runners share (pack, unpack and test): the job's
//! surroundings, its progress in the engine's words, pause and cancel,
//! the questions a limit asks while a job runs, where a file lands when
//! its name is taken, and the committing write that replaces nothing
//! until the new bytes are whole.
//!
//! The runners live here, beside the provider, and speak the engine's
//! types (research/archives.md 8.1): a [`JobSink`] for progress, conflicts
//! and errors, a [`JobControl`] for cancel and pause, a [`JobReport`] with
//! the undo record of what the job made. The order is the engine's:
//! create, verify, then destroy.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::{
    ConflictAction, ConflictKind, ConflictPolicy, Entry, EntryKind, JobId, JobKind, JobState,
    Place, catalog,
};
use diroxide_ops::{Conflict, Engine, JobControl, JobProgress, JobReport, JobSink, Pacer};
use diroxide_vfs::names::unused_name;
use diroxide_vfs::{
    AttributeChange, NoSecrets, OpContext, Provider, SecretSource, TrashCheck, VfsError, WriteMode,
};

use crate::guard::{Question, QuestionKind};
use crate::provider::ArchiveProvider;

/// Asks a limit's question while a job runs ("Expands to 40 GB from 40
/// KB. Unpack? y or n"), on the job's thread, blocking until the user
/// answers, as a conflict question does. The answer starts on No.
pub trait ArchiveQuestions: Send + Sync {
    /// Asks; true for yes.
    fn ask(&self, question: &Question) -> bool;
}

/// Answers every question No: for jobs that must not ask.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoQuestions;

impl ArchiveQuestions for NoQuestions {
    fn ask(&self, _question: &Question) -> bool {
        false
    }
}

/// What a runner needs around it: the engine (its registry, options and
/// clock), the archive provider, the job's id, its control and sink, whom
/// to ask a limit's question, and whom to ask for a password.
#[derive(Clone)]
pub struct JobEnv<'a> {
    /// The engine: the registry every place is reached through, the
    /// options (Keep dates, the old file to the trash on a replace), the
    /// clock.
    pub engine: &'a Engine,
    /// The archive provider.
    pub archives: &'a ArchiveProvider,
    /// The job's id, for its report and undo record.
    pub job: JobId,
    /// Cancel and pause.
    pub control: &'a JobControl,
    /// Progress, conflicts and errors.
    pub sink: &'a dyn JobSink,
    /// A limit's questions while the job runs.
    pub questions: &'a dyn ArchiveQuestions,
    /// The masked prompt, asked for a password when the first locked
    /// member is reached.
    pub secrets: Arc<dyn SecretSource>,
}

impl std::fmt::Debug for JobEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobEnv")
            .field("job", &self.job)
            .finish_non_exhaustive()
    }
}

impl<'a> JobEnv<'a> {
    /// The surroundings of a job that asks no question (every one answered
    /// No) and has no prompt for passwords.
    pub fn new(
        engine: &'a Engine,
        archives: &'a ArchiveProvider,
        job: JobId,
        control: &'a JobControl,
        sink: &'a dyn JobSink,
    ) -> Self {
        Self {
            engine,
            archives,
            job,
            control,
            sink,
            questions: &NoQuestions,
            secrets: Arc::new(NoSecrets),
        }
    }

    /// The same, asking a limit's questions of `questions`.
    pub fn with_questions(mut self, questions: &'a dyn ArchiveQuestions) -> Self {
        self.questions = questions;
        self
    }

    /// The same, asking for passwords of `secrets` (the masked prompt).
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretSource>) -> Self {
        self.secrets = secrets;
        self
    }

    /// A context for the job's provider calls: the control's cancel, no
    /// progress (the runner reports its own), these secrets.
    pub(crate) fn context(&self, secrets: Arc<dyn SecretSource>) -> OpContext {
        OpContext::new()
            .with_cancel(self.control.cancel.clone())
            .with_secrets(secrets)
    }

    /// The provider of a place.
    pub(crate) fn provider(&self, place: &Place) -> Result<Arc<dyn Provider>, VfsError> {
        self.engine.registry().resolve(place)
    }
}

/// A job's progress in the engine's words ("Unpacking 30 percent, 2
/// minutes left"): told to the sink at a change of state at once, and
/// otherwise at most four times a second (the app says it at its own
/// interval). The estimate waits for five seconds and five percent.
pub(crate) struct Progress<'a> {
    sink: &'a dyn JobSink,
    pacer: &'a dyn Pacer,
    progress: JobProgress,
    started: Duration,
    last: Duration,
    paused: bool,
}

/// The least time between two progress reports of one state.
const REPORT_EVERY: Duration = Duration::from_millis(250);

impl<'a> Progress<'a> {
    /// The progress of a job of `kind`, with its items and bytes, said as
    /// running at once.
    pub(crate) fn start(env: &JobEnv<'a>, kind: JobKind, items: u64, bytes: Option<u64>) -> Self {
        let pacer = env.engine.pacer();
        let mut progress = JobProgress::start(kind, items, bytes);
        progress.state = JobState::Running;
        let now = pacer.now();
        let this = Self {
            sink: env.sink,
            pacer,
            progress,
            started: now,
            last: now,
            paused: false,
        };
        this.sink.progress(&this.progress);
        this
    }

    /// Says a new state at once.
    pub(crate) fn state(&mut self, state: JobState) {
        self.progress.state = state;
        self.last = self.pacer.now();
        self.sink.progress(&self.progress);
    }

    /// The item at work.
    pub(crate) fn current(&mut self, name: &str) {
        self.progress.current = Some(name.to_owned());
        self.maybe_report();
    }

    /// One item finished.
    pub(crate) fn item_done(&mut self) {
        self.progress.items_done = self.progress.items_done.saturating_add(1);
        self.maybe_report();
    }

    /// Bytes finished.
    pub(crate) fn bytes(&mut self, bytes: u64) {
        self.progress.bytes_done = self.progress.bytes_done.saturating_add(bytes);
        self.maybe_report();
    }

    /// Bytes counted twice (an item tried again): taken back.
    pub(crate) fn unbytes(&mut self, bytes: u64) {
        self.progress.bytes_done = self.progress.bytes_done.saturating_sub(bytes);
    }

    fn maybe_report(&mut self) {
        let now = self.pacer.now();
        if now.saturating_sub(self.last) < REPORT_EVERY {
            return;
        }
        self.last = now;
        self.estimate(now);
        self.sink.progress(&self.progress);
    }

    fn estimate(&mut self, now: Duration) {
        let elapsed = now.saturating_sub(self.started).as_secs_f64();
        let (done, total) = match self.progress.bytes_total.filter(|&t| t > 0) {
            Some(total) => (self.progress.bytes_done, total),
            None => (self.progress.items_done, self.progress.items_total),
        };
        if elapsed > 0.0 && self.progress.bytes_done > 0 {
            self.progress.bytes_per_second =
                Some((self.progress.bytes_done as f64 / elapsed) as u64);
        }
        if elapsed < 5.0 || total == 0 || done.saturating_mul(20) < total {
            return;
        }
        let rate = done as f64 / elapsed;
        if rate > 0.0 {
            self.progress.seconds_left = Some((total.saturating_sub(done) as f64 / rate) as u64);
        }
    }

    /// Waits while the job is paused, saying so; false once it is
    /// cancelled.
    pub(crate) fn hold(&mut self, control: &JobControl) -> bool {
        while control.is_paused() && !control.cancel.is_cancelled() {
            if !self.paused {
                self.paused = true;
                self.state(JobState::Paused);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.paused {
            self.paused = false;
            self.state(JobState::Running);
        }
        !control.cancel.is_cancelled()
    }

    /// Says the job finished.
    pub(crate) fn finish(&mut self) {
        self.progress.seconds_left = None;
        self.state(JobState::Finished);
    }
}

/// Where a new file lands: free, over a file that is there (replaced at
/// the commit, the old one to the trash first where the place has one),
/// or nowhere, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Landing {
    /// The name is free.
    Free(Place),
    /// The name holds a file the new one replaces.
    Replace(Place),
    /// Left where it is, with the reason in words.
    Skip(String),
    /// The user cancelled the job at the question.
    Cancel,
}

/// The conflict answers of one job: the starting policy, and the answers
/// given for the rest of the job, by kind.
#[derive(Debug, Default)]
pub(crate) struct Settler {
    policy: ConflictPolicy,
    answers: HashMap<ConflictKind, ConflictAction>,
    time_tolerance: i64,
}

impl Settler {
    /// The answers of a job with this policy.
    pub(crate) fn new(policy: ConflictPolicy, time_tolerance: i64) -> Self {
        Self {
            policy,
            answers: HashMap::new(),
            time_tolerance,
        }
    }

    /// Settles where `entry` (from `from`) goes when it is meant for `to`:
    /// the target if free; else the policy, the answer given for the rest
    /// of the job, or the user's answer through the sink. A file over a
    /// folder, or a folder over a file, is skipped, never resolved by
    /// deleting.
    pub(crate) fn settle(
        &mut self,
        provider: &dyn Provider,
        ctx: &OpContext,
        from: &Place,
        entry: &Entry,
        to: &Place,
        sink: &dyn JobSink,
    ) -> Result<Landing, VfsError> {
        let mut target = to.clone();
        for _ in 0..8 {
            let existing = match provider.stat(&target, ctx) {
                Ok(existing) => existing,
                Err(VfsError::NotFound) => return Ok(Landing::Free(target)),
                Err(error) => return Err(error),
            };
            let folder = target.parent().ok_or(VfsError::NotFound)?;
            let kind = match (entry.kind, existing.kind) {
                (EntryKind::Folder, EntryKind::Folder) => return Ok(Landing::Free(target)),
                (EntryKind::Folder, _) => ConflictKind::FolderOverFile,
                (_, EntryKind::Folder) => ConflictKind::FileOverFolder,
                _ if existing.hidden || is_read_only(&existing) => ConflictKind::ReadOnlyTarget,
                _ => ConflictKind::FileOverFile,
            };
            let keep_both = {
                let siblings: Vec<String> = provider
                    .list(&folder, ctx)
                    .map(|entries| entries.into_iter().map(|e| e.name).collect())
                    .unwrap_or_default();
                let siblings: Vec<&str> = siblings.iter().map(String::as_str).collect();
                unused_name(target.name().unwrap_or_default(), &siblings)
            };
            let clash = matches!(
                kind,
                ConflictKind::FileOverFolder | ConflictKind::FolderOverFile
            );
            let action = match self.answers.get(&kind) {
                Some(action) => action.clone(),
                None => match (self.policy, kind) {
                    (ConflictPolicy::Skip, _) => ConflictAction::Skip,
                    (ConflictPolicy::KeepBoth, _) if !clash => ConflictAction::KeepBoth,
                    (ConflictPolicy::Overwrite, ConflictKind::FileOverFile) => {
                        ConflictAction::Replace
                    }
                    (ConflictPolicy::Newer, ConflictKind::FileOverFile) => {
                        ConflictAction::ReplaceIfNewer
                    }
                    _ => {
                        let answer = sink.conflict(&Conflict {
                            kind,
                            source: from.clone(),
                            target: target.clone(),
                            source_entry: entry.clone(),
                            target_entry: existing.clone(),
                            keep_both_name: keep_both.clone().unwrap_or_default(),
                        });
                        if answer.apply_to_rest
                            && matches!(
                                answer.action,
                                ConflictAction::Skip
                                    | ConflictAction::Replace
                                    | ConflictAction::KeepBoth
                                    | ConflictAction::ReplaceIfNewer
                            )
                        {
                            self.answers.insert(kind, answer.action.clone());
                        }
                        answer.action
                    }
                },
            };
            if clash {
                return Ok(match action {
                    ConflictAction::Cancel => Landing::Cancel,
                    _ if kind == ConflictKind::FileOverFolder => {
                        Landing::Skip("a folder has its name here".to_owned())
                    }
                    _ => Landing::Skip("a file has its name here".to_owned()),
                });
            }
            let action = match action {
                ConflictAction::ReplaceIfNewer => match (entry.modified, existing.modified) {
                    (Some(new), Some(old)) if new > old.plus_seconds(self.time_tolerance) => {
                        ConflictAction::Replace
                    }
                    _ => return Ok(Landing::Skip("not newer than the one there".to_owned())),
                },
                other => other,
            };
            match action {
                ConflictAction::Skip | ConflictAction::ReplaceIfNewer => {
                    return Ok(Landing::Skip("the name is taken".to_owned()));
                }
                ConflictAction::Cancel => return Ok(Landing::Cancel),
                ConflictAction::Replace => return Ok(Landing::Replace(target)),
                ConflictAction::KeepBoth => {
                    let Some(name) = keep_both else {
                        return Ok(Landing::Skip(
                            "no free numbered name after 999 tries".to_owned(),
                        ));
                    };
                    return Ok(Landing::Free(folder.join(&name).map_err(VfsError::from)?));
                }
                ConflictAction::Rename(name) => {
                    if name.is_empty() {
                        return Ok(Landing::Skip("no new name was given".to_owned()));
                    }
                    target = folder.join(&name).map_err(VfsError::from)?;
                }
            }
        }
        Ok(Landing::Skip("the name is taken".to_owned()))
    }
}

/// Whether an entry is read-only.
fn is_read_only(entry: &Entry) -> bool {
    match entry.attributes {
        Some(diroxide_core::Attributes::Windows(bits)) => bits & 1 != 0,
        Some(diroxide_core::Attributes::Unix(mode)) => mode & 0o200 == 0,
        None => false,
    }
}

/// How a committing write ended: whether the file it replaced is in the
/// trash, and anything to say.
pub(crate) struct Written {
    /// Whether the old file went to the trash first (so Undo can bring it
    /// back).
    pub(crate) old_in_trash: bool,
    /// Something to say about the write.
    pub(crate) warning: Option<String>,
}

/// Writes `reader` to `target` through the provider's committing writer:
/// the bytes go to an unfinished name; when they are whole, the old file
/// (on a replace) goes to the trash where the place has one, then the
/// commit gives the target its new content. A failure or a cancel leaves
/// the target as it was. `times` are set on the unfinished file where the
/// writer can, so the file lands with its dates right.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_file(
    provider: &dyn Provider,
    target: &Place,
    replace: bool,
    keep_old: bool,
    reader: &mut dyn Read,
    times: &AttributeChange,
    ctx: &OpContext,
    progress: &mut Progress<'_>,
    control: &JobControl,
    chunk: usize,
) -> Result<Written, WriteFailure> {
    let mode = if replace {
        WriteMode::Overwrite
    } else {
        WriteMode::CreateNew
    };
    let mut writer = provider
        .write(target, mode, ctx)
        .map_err(WriteFailure::Write)?;
    let mut buffer = vec![0u8; chunk.max(4096)];
    let mut bytes = 0u64;
    let copied = (|| -> Result<(), WriteFailure> {
        loop {
            if !progress.hold(control) {
                return Err(WriteFailure::Write(VfsError::Cancelled));
            }
            let n = match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(WriteFailure::Read(e)),
            };
            writer
                .write_all(&buffer[..n])
                .map_err(|e| WriteFailure::Write(io_to_vfs(e)))?;
            bytes += n as u64;
            progress.bytes(n as u64);
        }
    })();
    let flushed = copied.and_then(|()| {
        writer
            .flush()
            .map_err(|e| WriteFailure::Write(io_to_vfs(e)))
    });
    if let Err(failure) = flushed {
        drop(writer);
        progress.unbytes(bytes);
        return Err(failure);
    }
    let set_early = !times.is_empty() && writer.set_attributes(times).is_ok();
    let mut warning = None;
    let mut old_in_trash = false;
    if replace && keep_old && provider.capabilities(target).trash {
        match provider.trash(target, ctx) {
            Ok(TrashCheck::InTrash) => old_in_trash = true,
            Ok(TrashCheck::NotChecked) => {
                old_in_trash = true;
                warning = Some("the old copy went to the trash; not checked".to_owned());
            }
            Ok(TrashCheck::GoneNotInTrash) => {
                warning = Some(catalog::TRASH_NOT_CONFIRMED.to_owned());
            }
            Ok(_) => {
                drop(writer);
                progress.unbytes(bytes);
                return Err(WriteFailure::Write(VfsError::Unsafe {
                    why: "the old copy is still there after the trash".to_owned(),
                }));
            }
            Err(error) => {
                drop(writer);
                progress.unbytes(bytes);
                return Err(WriteFailure::Write(error));
            }
        }
    } else if replace {
        warning = Some("replaced; this cannot be undone".to_owned());
    }
    if let Err(error) = writer.commit(replace) {
        progress.unbytes(bytes);
        if old_in_trash && provider.restore(target, &OpContext::new()).is_err() {
            return Err(WriteFailure::Write(VfsError::Unsafe {
                why: format!("{error}; the old copy is in the trash"),
            }));
        }
        return Err(WriteFailure::Write(error));
    }
    if !set_early && !times.is_empty() {
        let _ = provider.set_attributes(target, times, &OpContext::new());
    }
    Ok(Written {
        old_in_trash,
        warning,
    })
}

/// Why a committing write failed: reading the source (the I/O error as the
/// reader gave it, which may carry a limit's question or a password's
/// refusal), or the writing and the provider.
#[derive(Debug)]
pub(crate) enum WriteFailure {
    /// Reading the source failed.
    Read(io::Error),
    /// Writing, the trash or the commit failed.
    Write(VfsError),
}

/// The provider error an I/O error from a reader or a writer stands for.
pub(crate) fn io_to_vfs(error: io::Error) -> VfsError {
    crate::guard::from_io(error)
}

/// The questions a job has already answered No to: the rest of the job
/// is not asked again, and each like it is skipped.
#[derive(Debug, Default)]
pub(crate) struct Declined(HashSet<QuestionKind>);

impl Declined {
    /// Whether this kind was declined.
    pub(crate) fn contains(&self, kind: QuestionKind) -> bool {
        self.0.contains(&kind)
    }

    /// Notes a No.
    pub(crate) fn insert(&mut self, kind: QuestionKind) {
        self.0.insert(kind);
    }
}

/// The report's head line with the job's own facts after it, each line
/// a sentence: the summary first ("Unpack finished: 118 done, 2
/// skipped."), then what the job says beside it.
pub(crate) fn lines_of(report: &JobReport, extra: Vec<String>) -> Vec<String> {
    let mut lines = vec![report.summary()];
    lines.extend(extra);
    lines
}

/// The reason a question gave, in words, for the report: "it was not
/// unpacked: Expands to 40 GB from 40 KB".
pub(crate) fn declined_reason(question: &Question) -> String {
    format!("you said No to: {}", question.text)
}
