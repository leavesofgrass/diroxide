//! The conversion's run (R28 5.3, steps 4 to 7): one file at a time, four
//! at once for pictures; each output made at a staging name in its
//! destination folder and renamed to its real name only when the tool
//! succeeded, so nothing half-made is ever visible and a hot folder
//! watching the destination never sees it; progress in words, with
//! FFmpeg's own `-progress` for the file it is on; cancel stops the tool
//! with every child it started and removes the staging file.
//!
//! The rename never replaces: a name taken since the plan is answered by
//! the policy (keep both, skip, or the question with a person there), and
//! an answer to replace keeps both instead and says so.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use diroxide_core::job::ErrorPolicy;
use diroxide_core::{
    ConflictAction, ConflictKind, ConflictPolicy, Entry, ErrorAction, JobKind, JobState, Place,
};
use diroxide_vfs::names::unused_name;
use diroxide_vfs::runner::seconds_words;
use diroxide_vfs::{
    CancelToken, OpContext, Provider, RunLimits, Split, ToolRequest, ToolRunError, ToolStream,
    VfsError, WriteMode,
};

use super::carta::{CartaEnd, CartaJob};
use super::placeholders::{Fill, dash_guarded, expand};
use super::plan::{ConvertItem, ConvertPlan, extension_of, said, stem_of};
use super::report::ConvertReport;
use super::staging::{spool_input_name, staging_name, token};
use super::tools::{self, FfmpegProgress};
use crate::engine::{Conflict, Engine, ItemError, JobControl, JobSink};
use crate::journal::JournalWriter;
use crate::progress::JobProgress;
use crate::undo::{LeftAs, UndoRecord};

/// How much output a tool may write on its two streams before it is
/// stopped: FFmpeg's progress for a ten-hour film stays well under it.
pub const OUTPUT_CAP: usize = 64 * 1024 * 1024;

/// How many files run at once: four for pictures, one for the rest
/// (media tools use several cores themselves).
pub fn parallel(tool: &str) -> usize {
    if tool == "magick" { 4 } else { 1 }
}

/// How many lines of a tool's standard error a failure keeps.
const STDERR_LINES: usize = 4;

/// What one file came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Made, at this place, as the run left it.
    Made(Place, Box<Entry>),
    /// Skipped, with the reason.
    Skipped(String),
    /// Failed, with the cause in words.
    Failed(String),
    /// The job was cancelled.
    Cancelled,
}

/// What the workers share.
struct Shared<'a> {
    plan: &'a ConvertPlan,
    control: &'a JobControl,
    sink: &'a dyn JobSink,
    report: Mutex<ConvertReport>,
    progress: Mutex<JobProgress>,
    next: AtomicUsize,
    halted: AtomicBool,
    /// One question at a time, and the answer that settles the rest.
    asking: Mutex<Option<ConflictAction>>,
    /// Error classes answered "Skip all like this".
    skip_all: AtomicBool,
    /// The undo journal, when the engine keeps one: a step per file, so
    /// Undo works after a restart and an interrupted run is said.
    journal: Mutex<Option<JournalWriter>>,
}

impl Shared<'_> {
    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Says the progress with the file at hand.
    fn say(&self, update: impl FnOnce(&mut JobProgress)) {
        let snapshot = {
            let mut progress = Self::lock(&self.progress);
            update(&mut progress);
            progress.clone()
        };
        self.sink.progress(&snapshot);
    }

    /// Appends a step to the undo journal: what it made.
    fn journal_step(&self, step: u64, outcome: &str, created: Vec<Place>) {
        if let Some(journal) = Self::lock(&self.journal).as_mut() {
            let delta = UndoRecord {
                created,
                ..UndoRecord::default()
            };
            let _ = journal.step(step, outcome, &delta);
        }
    }

    /// Notes in the undo journal which staging file belongs to which
    /// output, so a crash's leftover is said at the next start.
    fn journal_writing(&self, output: &Place, staging: &Place) {
        if let Some(journal) = Self::lock(&self.journal).as_mut() {
            let _ = journal.writing(output, staging);
        }
    }

    fn stopped(&self) -> bool {
        self.halted.load(Ordering::SeqCst) || self.control.cancel.is_cancelled()
    }
}

/// The words of a tool's standard error worth saying: its first
/// non-empty line, at most 200 characters (FFmpeg and ImageMagick put the
/// cause first).
fn cause_from(stderr: &[String]) -> Option<String> {
    let first = stderr.iter().map(|l| l.trim()).find(|l| !l.is_empty())?;
    Some(first.chars().take(200).collect())
}

/// A local folder's path.
fn local(place: &Place) -> Option<PathBuf> {
    place.local_path().map(PathBuf::from)
}

impl Engine {
    /// Runs a conversion's plan to its end, or until cancelled. Blocking:
    /// the app runs it on a worker thread. The report's undo record names
    /// every output made, so Undo trashes them.
    pub fn run_convert(
        &self,
        plan: &ConvertPlan,
        control: &JobControl,
        sink: &dyn JobSink,
    ) -> ConvertReport {
        let mut report = ConvertReport::new(plan.job, &plan.recipe.name, &plan.tool_name);
        report.refused = plan.refused.clone();
        report.job.skipped = plan.skipped.clone();
        report.job.undo.origin = plan.origin.clone();
        let mut progress = JobProgress::start(
            JobKind::Convert,
            plan.work.len() as u64,
            Some(plan.bytes).filter(|b| *b > 0),
        );
        progress.state = JobState::Running;
        let shared = Shared {
            plan,
            control,
            sink,
            report: Mutex::new(report),
            progress: Mutex::new(progress),
            next: AtomicUsize::new(0),
            halted: AtomicBool::new(false),
            asking: Mutex::new(None),
            skip_all: AtomicBool::new(false),
            journal: Mutex::new(self.journal().and_then(|journal| {
                journal
                    .start(
                        plan.job,
                        JobKind::Convert,
                        &plan.origin,
                        plan.work.len() as u64,
                    )
                    .ok()
            })),
        };
        shared.say(|_| {});
        self.make_folders(&shared);
        let workers = parallel(&plan.recipe.tool).min(plan.work.len()).max(1);
        thread::scope(|scope| {
            for _ in 1..workers {
                scope.spawn(|| self.work(&shared));
            }
            self.work(&shared);
        });
        let cancelled = control.cancel.is_cancelled();
        shared.say(|p| {
            p.state = JobState::Finished;
            p.current = None;
        });
        let mut report = shared
            .report
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        report.job.cancelled = cancelled;
        if let Some(journal) = shared
            .journal
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            let how = if cancelled { "cancelled" } else { "finished" };
            let _ = journal.end(how, &report.job.undo.left);
        }
        report
    }

    /// Makes the folders a recursive run mirrors, parents first; each one
    /// made goes in the undo record, which removes it if it is empty.
    fn make_folders(&self, shared: &Shared<'_>) {
        let ctx = self.context(&shared.control.cancel);
        for folder in &shared.plan.folders {
            let Ok(provider) = self.registry().resolve(folder) else {
                continue;
            };
            if provider.stat(folder, &ctx).is_ok() {
                continue;
            }
            if provider.create_dir(folder, &ctx).is_ok() {
                Shared::lock(&shared.report)
                    .job
                    .undo
                    .created
                    .push(folder.clone());
                shared.journal_step(0, "folder made", vec![folder.clone()]);
            }
        }
    }

    /// One worker: takes the next file until none is left or the job
    /// stops.
    fn work(&self, shared: &Shared<'_>) {
        loop {
            if shared.stopped() {
                return;
            }
            let index = shared.next.fetch_add(1, Ordering::SeqCst);
            let Some(item) = shared.plan.work.get(index) else {
                return;
            };
            self.hold_while_paused(shared);
            if shared.stopped() {
                return;
            }
            shared.say(|p| p.current = Some(said(&item.source)));
            let outcome = self.one_with_retries(shared, item);
            let step = index as u64 + 1;
            let mut report = Shared::lock(&shared.report);
            match outcome {
                Outcome::Made(output, entry) => {
                    let entry = *entry;
                    report.made.push((item.source.clone(), output.clone()));
                    report.job.done.push(output.clone());
                    report.job.undo.created.push(output.clone());
                    report.job.undo.left.push(LeftAs::of(&output, &entry));
                    shared.journal_step(step, "done", vec![output]);
                }
                Outcome::Skipped(reason) => {
                    report.job.skipped.push((item.source.clone(), reason));
                    shared.journal_step(step, "skipped", Vec::new());
                }
                Outcome::Failed(cause) => {
                    report.job.failed.push((item.source.clone(), cause));
                    shared.journal_step(step, "failed", Vec::new());
                    if shared.plan.errors == ErrorPolicy::StopAtFirst {
                        report.job.stopped = true;
                        shared.halted.store(true, Ordering::SeqCst);
                    }
                }
                Outcome::Cancelled => {
                    shared.halted.store(true, Ordering::SeqCst);
                }
            }
            drop(report);
            let size = item.entry.size.unwrap_or(0);
            shared.say(|p| {
                p.items_done += 1;
                p.bytes_done = p.bytes_done.saturating_add(size);
            });
        }
    }

    /// Waits while the job is paused, saying so once.
    fn hold_while_paused(&self, shared: &Shared<'_>) {
        if !shared.control.is_paused() {
            return;
        }
        shared.say(|p| p.state = JobState::Paused);
        while shared.control.is_paused() && !shared.control.cancel.is_cancelled() {
            self.pacer().sleep(Duration::from_millis(50));
        }
        shared.say(|p| p.state = JobState::Running);
    }

    /// One file, asking about a failure when the policy asks: Retry runs
    /// it again, Skip and Skip all like this move on, Cancel stops.
    fn one_with_retries(&self, shared: &Shared<'_>, item: &ConvertItem) -> Outcome {
        let mut attempt = 1;
        loop {
            let outcome = self.one(shared, item);
            let Outcome::Failed(cause) = &outcome else {
                return outcome;
            };
            if shared.plan.errors != ErrorPolicy::Ask || shared.skip_all.load(Ordering::SeqCst) {
                return outcome;
            }
            let question = ItemError {
                kind: JobKind::Convert,
                place: item.source.clone(),
                error: VfsError::Io {
                    kind: io::ErrorKind::Other,
                    message: cause.clone(),
                },
                attempt,
            };
            let answer = {
                let _one = Shared::lock(&shared.asking);
                shared.say(|p| p.state = JobState::WaitingForAnswer);
                let answer = shared.sink.error(&question);
                shared.say(|p| p.state = JobState::Running);
                answer
            };
            match answer {
                ErrorAction::Retry => attempt += 1,
                ErrorAction::SkipAllLikeThis => {
                    shared.skip_all.store(true, Ordering::SeqCst);
                    return outcome;
                }
                ErrorAction::Cancel => {
                    shared.control.cancel.cancel();
                    return outcome;
                }
                _ => return outcome,
            }
        }
    }

    /// One file: the input on this computer (copied into the spool first
    /// when it is not, or when the tool would misread its name), the tool
    /// run to a staging name, and the output placed under its real name.
    fn one(&self, shared: &Shared<'_>, item: &ConvertItem) -> Outcome {
        let ctx = self.context(&shared.control.cancel);
        let spooled = if item.through_spool {
            match self.spool_in(shared, item, &ctx) {
                Ok(path) => Some(path),
                Err(outcome) => return outcome,
            }
        } else {
            None
        };
        let outcome = self.one_from(shared, item, spooled.as_deref(), &ctx);
        if let Some(path) = spooled {
            let _ = fs::remove_file(path);
        }
        outcome
    }

    /// Copies a source into the spool under a staging name with its own
    /// extension, so the tool reads a local file with a plain name.
    fn spool_in(
        &self,
        shared: &Shared<'_>,
        item: &ConvertItem,
        ctx: &OpContext,
    ) -> Result<PathBuf, Outcome> {
        let failed = |cause: String| Outcome::Failed(format!("it could not be copied in: {cause}"));
        let spool = shared
            .plan
            .spool
            .as_ref()
            .ok_or_else(|| failed("there is no spool folder".to_owned()))?;
        fs::create_dir_all(spool).map_err(|e| failed(VfsError::from(e).to_string()))?;
        let extension = item
            .source
            .name()
            .and_then(extension_of)
            .filter(|e| e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .unwrap_or_else(|| "in".to_owned());
        let path = (0..8)
            .map(|_| spool.join(spool_input_name(token(), &extension)))
            .find(|path| !path.exists())
            .ok_or_else(|| failed("no spool name was free".to_owned()))?;
        let copied = (|| -> Result<(), VfsError> {
            let provider = self.registry().resolve(&item.source)?;
            let mut reader = provider.read(&item.source, ctx)?;
            let mut file = fs::File::create_new(&path)?;
            copy_checked(&mut reader, &mut file, &shared.control.cancel)?;
            Ok(())
        })();
        match copied {
            Ok(()) => Ok(path),
            Err(error) => {
                let _ = fs::remove_file(&path);
                if error == VfsError::Cancelled {
                    Err(Outcome::Cancelled)
                } else {
                    Err(failed(error.to_string()))
                }
            }
        }
    }

    /// One file from a local input: staged beside its output when the
    /// destination is on this computer, in the spool when it is not.
    fn one_from(
        &self,
        shared: &Shared<'_>,
        item: &ConvertItem,
        spooled: Option<&Path>,
        ctx: &OpContext,
    ) -> Outcome {
        let plan = shared.plan;
        let recipe = &plan.recipe;
        let input = match spooled {
            Some(path) => path.to_path_buf(),
            None => match local(&item.source) {
                Some(path) => path,
                None => return Outcome::Failed("it is not on this computer".to_owned()),
            },
        };
        let Some(folder) = item.output.parent() else {
            return Outcome::Failed("its output has no folder".to_owned());
        };
        let provider = match self.registry().resolve(&folder) {
            Ok(provider) => provider,
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let (staging, staged, out_folder) = match local(&folder) {
            Some(out_folder) => {
                match fresh_staging(&*provider, &folder, &out_folder, &recipe.to, ctx) {
                    Some((path, place)) => (path, Staged::Beside(place), out_folder),
                    None => return Outcome::Failed("no staging name was free".to_owned()),
                }
            }
            None => {
                let Some(spool) = plan.spool.clone() else {
                    return Outcome::Failed("the target is not on this computer".to_owned());
                };
                if let Err(error) = fs::create_dir_all(&spool) {
                    return Outcome::Failed(VfsError::from(error).to_string());
                }
                match fresh_spool_name(&spool, &recipe.to) {
                    Some(path) => (path.clone(), Staged::Spool(path), spool),
                    None => return Outcome::Failed("no spool name was free".to_owned()),
                }
            }
        };
        if let Staged::Beside(place) = &staged {
            shared.journal_writing(&item.output, place);
        }
        let ran = self.run_tool(shared, item, &input, &staging, &out_folder);
        // A staging file is removed with a fresh context: the job's own is
        // cancelled when Cancel is why it goes.
        let discard = || match &staged {
            Staged::Beside(place) => {
                let _ = provider.remove(place, &OpContext::new());
            }
            Staged::Spool(path) => {
                let _ = fs::remove_file(path);
            }
        };
        match ran {
            Ran::Cancelled => {
                discard();
                Outcome::Cancelled
            }
            Ran::Failed(cause) => {
                discard();
                Outcome::Failed(cause)
            }
            Ran::Done => {
                let size = fs::metadata(&staging).map_or(0, |m| m.len());
                if size == 0 {
                    discard();
                    return Outcome::Failed(format!("{} made no file", plan.tool_name));
                }
                let outcome = self.commit(shared, item, &*provider, &staged, size, &folder, ctx);
                if let Staged::Spool(path) = &staged {
                    let _ = fs::remove_file(path);
                }
                outcome
            }
        }
    }

    /// Runs the recipe's tool for one file, writing to `staging`.
    fn run_tool(
        &self,
        shared: &Shared<'_>,
        item: &ConvertItem,
        input: &Path,
        staging: &Path,
        out_folder: &Path,
    ) -> Ran {
        let recipe = &shared.plan.recipe;
        let source_name = item.source.name().unwrap_or_default();
        let in_folder = input.parent().map(Path::to_path_buf).unwrap_or_default();
        let input_name = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut typed = OsString::new();
        if let Some(format) = &item.format {
            typed.push(format);
            typed.push(":");
        }
        typed.push(input.as_os_str());
        let mut fill = Fill::new();
        fill.set("input", input)
            .set("file", input)
            .set("typed_input", &typed)
            .set("output", staging)
            .set("stem", stem_of(source_name))
            .set("name", dash_guarded(&input_name))
            .set("dir", &in_folder)
            .set("dest", out_folder)
            .set("other", out_folder);
        let args = match expand(&recipe.args, &fill) {
            Ok(args) => args,
            Err(name) => return Ran::Failed(format!("{name} has no value here")),
        };
        if recipe.tool == "carta" {
            return self.run_carta(shared, &args);
        }
        let args = tools::hardened(&recipe.tool, args);
        let request = ToolRequest::new(recipe.tool.clone(), args)
            .with_limits(RunLimits::new(recipe.timeout, OUTPUT_CAP))
            .in_folder(in_folder)
            .split_by(Split::Lines);
        let length = if recipe.tool == "ffmpeg" {
            self.media_length(input, &shared.control.cancel)
        } else {
            None
        };
        let name = said(&item.source);
        let mut stderr: Vec<String> = Vec::new();
        let mut last_percent = None;
        let mut record = |stream: ToolStream, line: &str| {
            match stream {
                ToolStream::Stderr => {
                    if stderr.len() < STDERR_LINES && !line.trim().is_empty() {
                        stderr.push(line.to_owned());
                    }
                }
                ToolStream::Stdout => {
                    if let (Some(total), Some(FfmpegProgress::At(at))) =
                        (length, tools::ffmpeg_progress(line))
                    {
                        let percent = (at.min(total) * 100 / total.max(1)).min(100);
                        if last_percent != Some(percent) {
                            last_percent = Some(percent);
                            shared.say(|p| {
                                p.current = Some(format!("{name}: {percent} percent"));
                            });
                        }
                    }
                }
            }
            ControlFlow::Continue(())
        };
        match self
            .tools()
            .run_streaming(&request, &shared.control.cancel, &mut record)
        {
            Err(ToolRunError::Cancelled) => Ran::Cancelled,
            Err(error) => Ran::Failed(error.to_string()),
            Ok(_) if shared.control.cancel.is_cancelled() => Ran::Cancelled,
            Ok(output) if output.succeeded() => Ran::Done,
            Ok(output) => {
                let tool = &shared.plan.tool_name;
                let code = output.status.map_or_else(
                    || "was stopped".to_owned(),
                    |c| format!("ended with code {c}"),
                );
                Ran::Failed(match cause_from(&stderr) {
                    Some(said) => format!("{tool} {code}: {said}"),
                    None => format!("{tool} {code}"),
                })
            }
        }
    }

    /// Runs carta for one file, in process or in its child.
    fn run_carta(&self, shared: &Shared<'_>, args: &[OsString]) -> Ran {
        let job = match CartaJob::from_args(args) {
            Ok(job) => job,
            Err(cause) => return Ran::Failed(cause),
        };
        let limit = shared.plan.recipe.timeout;
        match shared.plan.carta.run(&job, limit, &shared.control.cancel) {
            CartaEnd::Done => Ran::Done,
            CartaEnd::Failed(cause) => Ran::Failed(cause),
            CartaEnd::Cancelled => Ran::Cancelled,
            CartaEnd::TimedOut => Ran::Failed(format!(
                "carta took longer than {} and was stopped",
                seconds_words(&limit.as_secs())
            )),
        }
    }

    /// The input's length in microseconds, from ffprobe, for FFmpeg's
    /// percent; `None` when ffprobe is not found or cannot say.
    fn media_length(&self, input: &Path, cancel: &CancelToken) -> Option<u64> {
        if !self.tools().has("ffprobe") {
            return None;
        }
        let request = ToolRequest::new("ffprobe", tools::ffprobe_length_args(input))
            .with_limits(RunLimits::new(Duration::from_secs(30), 64 * 1024));
        let output = self.tools().run(&request, cancel).ok()?;
        output
            .succeeded()
            .then(|| tools::ffprobe_length(&output.stdout))
            .flatten()
    }

    /// Renames the staging file to the output's name, never over a file:
    /// a name taken is answered by the policy.
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &self,
        shared: &Shared<'_>,
        item: &ConvertItem,
        provider: &dyn Provider,
        staged: &Staged,
        size: u64,
        folder: &Place,
        ctx: &OpContext,
    ) -> Outcome {
        let quiet = OpContext::new();
        let discard = || {
            if let Staged::Beside(place) = staged {
                let _ = provider.remove(place, &quiet);
            }
        };
        let mut target = item.output.clone();
        let mut asked = !item.ask;
        for _ in 0..8 {
            if !asked && provider.stat(&target, ctx).is_ok() {
                asked = true;
                let mut new = item.entry.clone();
                new.size = Some(size);
                match self.ask(shared, item, provider, new, &target, folder, ctx) {
                    Answer::Use(place) => target = place,
                    Answer::Stop(outcome) => {
                        discard();
                        return outcome;
                    }
                }
            }
            let placed = match staged {
                Staged::Beside(place) => provider.rename(place, &target, ctx),
                Staged::Spool(path) => write_from_spool(provider, path, &target, ctx),
            };
            match placed {
                Ok(()) => {
                    let entry = provider.stat(&target, ctx).unwrap_or_else(|_| {
                        Entry::new(
                            target.name().unwrap_or_default(),
                            diroxide_core::EntryKind::File,
                        )
                    });
                    return Outcome::Made(target, Box::new(entry));
                }
                Err(VfsError::AlreadyExists) => {
                    if shared.plan.conflicts == ConflictPolicy::Skip {
                        discard();
                        return Outcome::Skipped("output exists".to_owned());
                    }
                    match keep_both_place(provider, folder, &target, ctx) {
                        Some(place) => target = place,
                        None => break,
                    }
                }
                Err(error) => {
                    discard();
                    return Outcome::Failed(error.to_string());
                }
            }
        }
        discard();
        Outcome::Failed("no free name was found for its output".to_owned())
    }

    /// The question about a taken name, with a person there: skip, keep
    /// both, a new name, or cancel. An answer to replace keeps both, since
    /// a conversion never replaces a file, and the report says so.
    #[allow(clippy::too_many_arguments)]
    fn ask(
        &self,
        shared: &Shared<'_>,
        item: &ConvertItem,
        provider: &dyn Provider,
        new: Entry,
        target: &Place,
        folder: &Place,
        ctx: &OpContext,
    ) -> Answer {
        let keep = keep_both_place(provider, folder, target, ctx);
        let mut settled = Shared::lock(&shared.asking);
        let action = match settled.clone() {
            Some(action) => action,
            None => {
                let old = provider
                    .stat(target, ctx)
                    .unwrap_or_else(|_| item.entry.clone());
                let conflict = Conflict {
                    kind: ConflictKind::FileOverFile,
                    source: item.source.clone(),
                    target: target.clone(),
                    source_entry: new,
                    target_entry: old,
                    keep_both_name: keep
                        .as_ref()
                        .and_then(|p| p.name().map(str::to_owned))
                        .unwrap_or_default(),
                };
                shared.say(|p| p.state = JobState::WaitingForAnswer);
                let answer = shared.sink.conflict(&conflict);
                shared.say(|p| p.state = JobState::Running);
                if answer.apply_to_rest {
                    *settled = Some(answer.action.clone());
                }
                answer.action
            }
        };
        drop(settled);
        match action {
            ConflictAction::Skip => Answer::Stop(Outcome::Skipped("output exists".to_owned())),
            ConflictAction::Cancel => {
                shared.control.cancel.cancel();
                Answer::Stop(Outcome::Cancelled)
            }
            ConflictAction::Rename(name) if !name.trim().is_empty() => {
                match folder.join(name.trim()) {
                    Ok(place) if provider.stat(&place, ctx).is_err() => Answer::Use(place),
                    _ => keep.map_or_else(
                        || Answer::Stop(Outcome::Failed("no free name was found".to_owned())),
                        Answer::Use,
                    ),
                }
            }
            ConflictAction::Replace | ConflictAction::ReplaceIfNewer => {
                Shared::lock(&shared.report).job.warnings.push((
                    item.source.clone(),
                    "kept both, since a conversion never replaces a file".to_owned(),
                ));
                keep.map_or_else(
                    || Answer::Stop(Outcome::Failed("no free name was found".to_owned())),
                    Answer::Use,
                )
            }
            _ => keep.map_or_else(
                || Answer::Stop(Outcome::Failed("no free name was found".to_owned())),
                Answer::Use,
            ),
        }
    }
}

/// What a tool's run came to.
enum Ran {
    Done,
    Failed(String),
    Cancelled,
}

/// Where a tool's output waits until it is placed under its real name.
enum Staged {
    /// Beside the output, in its folder on this computer: renamed in.
    Beside(Place),
    /// In the spool, for a destination that is not on this computer:
    /// written to the output's place through its provider's committing
    /// writer, which never replaces a file.
    Spool(PathBuf),
}

/// What a conflict's answer means for the commit.
enum Answer {
    /// Rename the staging file to this place.
    Use(Place),
    /// End this file here.
    Stop(Outcome),
}

/// A staging name in the destination folder that nothing holds yet.
fn fresh_staging(
    provider: &dyn Provider,
    folder: &Place,
    local_folder: &Path,
    extension: &str,
    ctx: &OpContext,
) -> Option<(PathBuf, Place)> {
    (0..8).find_map(|_| {
        let name = staging_name(token(), extension);
        let place = folder.join(&name).ok()?;
        provider
            .stat(&place, ctx)
            .is_err()
            .then(|| (local_folder.join(&name), place))
    })
}

/// A staging name in the spool that nothing holds yet.
fn fresh_spool_name(spool: &Path, extension: &str) -> Option<PathBuf> {
    (0..8)
        .map(|_| spool.join(staging_name(token(), extension)))
        .find(|path| !path.exists())
}

/// Copies a reader into a file a mebibyte at a time, stopping when the
/// job is cancelled.
fn copy_checked(
    reader: &mut dyn Read,
    file: &mut fs::File,
    cancel: &CancelToken,
) -> Result<(), VfsError> {
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        cancel.check()?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(file.flush()?);
        }
        file.write_all(&buffer[..read])?;
    }
}

/// Writes a spool file to a place through its provider's committing
/// writer: nothing reaches the place before the commit, and the commit
/// fails with `AlreadyExists` rather than replace a file.
fn write_from_spool(
    provider: &dyn Provider,
    path: &Path,
    target: &Place,
    ctx: &OpContext,
) -> Result<(), VfsError> {
    let mut file = fs::File::open(path)?;
    let mut writer = provider.write(target, WriteMode::CreateNew, ctx)?;
    io::copy(&mut file, &mut writer)?;
    writer.commit(true)
}

/// The keep-both place for a taken name: "photo (2).jpg", by the folder's
/// names now.
fn keep_both_place(
    provider: &dyn Provider,
    folder: &Place,
    target: &Place,
    ctx: &OpContext,
) -> Option<Place> {
    let names: Vec<String> = provider
        .list(folder, ctx)
        .ok()?
        .into_iter()
        .map(|e| e.name)
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let name = unused_name(target.name()?, &refs)?;
    folder.join(&name).ok()
}
