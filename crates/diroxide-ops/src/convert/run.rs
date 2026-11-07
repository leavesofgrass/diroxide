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
use diroxide_vfs::{
    OpContext, Provider, RunLimits, Split, ToolRequest, ToolRunError, ToolStream, VfsError,
};

use super::placeholders::{Fill, dash_guarded, expand};
use super::plan::{ConvertItem, ConvertPlan, said, stem_of};
use super::report::ConvertReport;
use super::staging::{staging_name, token};
use super::tools::{self, FfmpegProgress};
use crate::engine::{Conflict, Engine, ItemError, JobControl, JobSink};
use crate::progress::JobProgress;
use crate::undo::LeftAs;

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
            let mut report = Shared::lock(&shared.report);
            match outcome {
                Outcome::Made(output, entry) => {
                    let entry = *entry;
                    report.made.push((item.source.clone(), output.clone()));
                    report.job.done.push(output.clone());
                    report.job.undo.created.push(output.clone());
                    report.job.undo.left.push(LeftAs::of(&output, &entry));
                }
                Outcome::Skipped(reason) => report.job.skipped.push((item.source.clone(), reason)),
                Outcome::Failed(cause) => {
                    report.job.failed.push((item.source.clone(), cause));
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
                    kind: std::io::ErrorKind::Other,
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

    /// One file: the input on this computer, the tool run to a staging
    /// name beside the output, and the staging file renamed in.
    fn one(&self, shared: &Shared<'_>, item: &ConvertItem) -> Outcome {
        let plan = shared.plan;
        let recipe = &plan.recipe;
        let ctx = self.context(&shared.control.cancel);
        if item.through_spool {
            return Outcome::Failed("it needs the spool, which this run lacks".to_owned());
        }
        let Some(input) = local(&item.source) else {
            return Outcome::Failed("it is not on this computer".to_owned());
        };
        let Some(folder) = item.output.parent() else {
            return Outcome::Failed("its output has no folder".to_owned());
        };
        let Some(out_folder) = local(&folder) else {
            return Outcome::Failed("the target is not on this computer".to_owned());
        };
        let provider = match self.registry().resolve(&folder) {
            Ok(provider) => provider,
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let (staging, staging_place) =
            match fresh_staging(&*provider, &folder, &out_folder, &recipe.to, &ctx) {
                Some(found) => found,
                None => return Outcome::Failed("no staging name was free".to_owned()),
            };
        let ran = self.run_tool(shared, item, &input, &staging, &out_folder);
        // The staging file is removed with a fresh context: the job's own
        // is cancelled when Cancel is why it goes.
        let failed = |cause: String| {
            let _ = provider.remove(&staging_place, &OpContext::new());
            Outcome::Failed(cause)
        };
        match ran {
            Ran::Cancelled => {
                let _ = provider.remove(&staging_place, &OpContext::new());
                Outcome::Cancelled
            }
            Ran::Failed(cause) => failed(cause),
            Ran::Done => match provider.stat(&staging_place, &ctx) {
                Ok(entry) if entry.size.unwrap_or(0) > 0 => {
                    self.commit(shared, item, &*provider, &staging_place, &folder, &ctx)
                }
                _ => failed(format!("{} made no file", plan.tool_name)),
            },
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

    /// The input's length in microseconds, from ffprobe, for FFmpeg's
    /// percent; `None` when ffprobe is not found or cannot say.
    fn media_length(&self, input: &Path, cancel: &diroxide_vfs::CancelToken) -> Option<u64> {
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
    fn commit(
        &self,
        shared: &Shared<'_>,
        item: &ConvertItem,
        provider: &dyn Provider,
        staging: &Place,
        folder: &Place,
        ctx: &OpContext,
    ) -> Outcome {
        let mut target = item.output.clone();
        let mut asked = !item.ask;
        for _ in 0..8 {
            if !asked && provider.stat(&target, ctx).is_ok() {
                asked = true;
                match self.ask(shared, item, provider, staging, &target, folder, ctx) {
                    Answer::Use(place) => target = place,
                    Answer::Stop(outcome) => {
                        let _ = provider.remove(staging, &OpContext::new());
                        return outcome;
                    }
                }
            }
            match provider.rename(staging, &target, ctx) {
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
                        let _ = provider.remove(staging, &OpContext::new());
                        return Outcome::Skipped("output exists".to_owned());
                    }
                    match keep_both_place(provider, folder, &target, ctx) {
                        Some(place) => target = place,
                        None => break,
                    }
                }
                Err(error) => {
                    let _ = provider.remove(staging, &OpContext::new());
                    return Outcome::Failed(error.to_string());
                }
            }
        }
        let _ = provider.remove(staging, &OpContext::new());
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
        staging: &Place,
        target: &Place,
        folder: &Place,
        ctx: &OpContext,
    ) -> Answer {
        let keep = keep_both_place(provider, folder, target, ctx);
        let mut settled = Shared::lock(&shared.asking);
        let action = match settled.clone() {
            Some(action) => action,
            None => {
                let new = provider
                    .stat(staging, ctx)
                    .unwrap_or_else(|_| item.entry.clone());
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
                    "Conversions never replace a file: kept both".to_owned(),
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
