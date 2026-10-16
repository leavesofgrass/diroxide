//! The engine: plans jobs and runs plans.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use diroxide_core::{ConflictPolicy, JobId, JobKind, JobRequest, Message, Place};
use diroxide_vfs::{CancelToken, OpContext, Registry, VfsError, WriteMode};

use crate::error::OpsError;
use crate::plan::{Plan, Step};
use crate::progress::JobProgress;
use crate::undo::UndoRecord;

/// A name that is taken, and the question to ask about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// What was being copied or moved.
    pub source: Place,
    /// The place that already exists.
    pub target: Place,
}

impl Conflict {
    /// The question, meaning first and ending "y or n": "report.pdf exists
    /// in D:\backup. Replace it? y or n". A No answer skips; the dialog
    /// also offers keep both and rename.
    pub fn question(&self) -> Message {
        let name = self.target.name().unwrap_or("The entry");
        let folder = self
            .target
            .parent()
            .map_or_else(String::new, |p| format!(" in {p}"));
        Message::question(&format!("{name} exists{folder}. Replace it?"))
    }
}

/// Receives a running job's progress and answers its conflict question.
/// Called on the job's thread.
pub trait JobSink: Send + Sync {
    /// Takes a progress update; the app says at most two a second.
    fn progress(&self, progress: &JobProgress);

    /// Answers a conflict, blocking until the user does. The answer applies
    /// to the rest of the job; answering `Ask` again counts as `Skip`.
    fn conflict(&self, conflict: &Conflict) -> ConflictPolicy;
}

/// Cancels or pauses a running job. Cloning shares the control.
#[derive(Clone, Debug, Default)]
pub struct JobControl {
    /// Cancels the job between steps.
    pub cancel: CancelToken,
    paused: Arc<AtomicBool>,
}

impl JobControl {
    /// A control that is neither cancelled nor paused.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pauses the job after its current step.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    /// Lets a paused job go on.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    /// Whether the job is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

/// What a job did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobReport {
    /// The job.
    pub job: JobId,
    /// What kind of job it was.
    pub kind: JobKind,
    /// The places acted on.
    pub done: Vec<Place>,
    /// The places skipped, with the reason in words.
    pub skipped: Vec<(Place, String)>,
    /// The places that failed, with the cause in words.
    pub failed: Vec<(Place, String)>,
    /// Whether the job was cancelled before its end.
    pub cancelled: bool,
    /// What the Undo command can reverse.
    pub undo: UndoRecord,
}

impl JobReport {
    /// An empty report for a job.
    pub fn new(job: JobId, kind: JobKind) -> Self {
        Self {
            job,
            kind,
            done: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            cancelled: false,
            undo: UndoRecord {
                job: Some(job),
                ..UndoRecord::default()
            },
        }
    }

    /// The report in one line, the outcome first: "Copy finished: 3 done,
    /// 1 skipped, 1 failed." or "Copy cancelled: 2 done."
    pub fn summary(&self) -> String {
        let verb = self.kind.verb();
        let mut chars = verb.chars();
        let verb: String = match chars.next() {
            Some(c) => c.to_uppercase().chain(chars).collect(),
            None => String::new(),
        };
        let state = if self.cancelled {
            "cancelled"
        } else {
            "finished"
        };
        let mut parts = vec![format!("{} done", self.done.len())];
        if !self.skipped.is_empty() {
            parts.push(format!("{} skipped", self.skipped.len()));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        format!("{verb} {state}: {}.", parts.join(", "))
    }
}

enum Outcome {
    Done,
    Skipped(String),
}

/// The job engine. Planning is quick; running blocks, so the app runs it
/// on a worker thread.
#[derive(Clone, Debug)]
pub struct Engine {
    registry: Arc<Registry>,
}

impl Engine {
    /// An engine that reaches places through this registry.
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The registry the engine uses.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Computes what a job will do, touching nothing. Phase 0: one step per
    /// source, without walking into folders.
    pub fn plan(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        let named = |source: &Place| {
            source
                .name()
                .map(str::to_owned)
                .ok_or(OpsError::NothingSelected)
        };
        let new_name = || request.new_name.clone().ok_or(OpsError::NoName);
        let destination = || request.destination.clone().ok_or(OpsError::NoDestination);
        if request.sources.is_empty() && request.kind != JobKind::MakeFolder {
            return Err(OpsError::NothingSelected);
        }
        let mut steps = Vec::new();
        match request.kind {
            JobKind::Copy | JobKind::Move => {
                let folder = destination()?;
                for from in &request.sources {
                    let to = folder.join(&named(from)?).map_err(VfsError::from)?;
                    let from = from.clone();
                    steps.push(if request.kind == JobKind::Copy {
                        Step::Copy { from, to }
                    } else {
                        Step::Move { from, to }
                    });
                }
            }
            JobKind::Rename | JobKind::Duplicate => {
                let from = request.sources.first().ok_or(OpsError::NothingSelected)?;
                let parent = from.parent().ok_or(OpsError::NothingSelected)?;
                let to = parent.join(&new_name()?).map_err(VfsError::from)?;
                let from = from.clone();
                steps.push(if request.kind == JobKind::Rename {
                    Step::Rename { from, to }
                } else {
                    Step::Copy { from, to }
                });
            }
            JobKind::MakeFolder => {
                let place = destination()?.join(&new_name()?).map_err(VfsError::from)?;
                steps.push(Step::MakeFolder { place });
            }
            JobKind::Trash => steps.extend(request.sources.iter().map(|place| Step::Trash {
                place: place.clone(),
            })),
            JobKind::DeletePermanently => {
                steps.extend(request.sources.iter().map(|place| Step::Remove {
                    place: place.clone(),
                }))
            }
            other => return Err(OpsError::NotBuilt(other.verb().to_owned())),
        }
        Ok(Plan {
            job: request.id,
            kind: request.kind,
            steps,
            conflicts: request.conflicts,
        })
    }

    /// Runs a plan to its end, or until cancelled, reporting progress and
    /// asking about conflicts through the sink.
    pub fn run(&self, plan: &Plan, control: &JobControl, sink: &dyn JobSink) -> JobReport {
        let mut report = JobReport::new(plan.job, plan.kind);
        let mut policy = plan.conflicts;
        let mut progress = JobProgress::start(plan.kind, plan.steps.len() as u64, None);
        let ctx = OpContext::new().with_cancel(control.cancel.clone());
        for step in &plan.steps {
            while control.is_paused() && !control.cancel.is_cancelled() {
                thread::sleep(Duration::from_millis(50));
            }
            if control.cancel.is_cancelled() {
                report.cancelled = true;
                break;
            }
            let subject = step.subject().clone();
            progress.current = subject.name().map(str::to_owned);
            sink.progress(&progress);
            match self.run_step(step, &mut policy, sink, &ctx) {
                Ok(Outcome::Done) => {
                    match step {
                        Step::Move { from, to } | Step::Rename { from, to } => {
                            report.undo.moves.push((from.clone(), to.clone()));
                        }
                        Step::Trash { place } => report.undo.trashed.push(place.clone()),
                        _ => {}
                    }
                    report.done.push(subject);
                }
                Ok(Outcome::Skipped(reason)) => report.skipped.push((subject, reason)),
                Err(OpsError::Vfs(VfsError::Cancelled)) => {
                    report.cancelled = true;
                    break;
                }
                Err(error) => report.failed.push((subject, error.to_string())),
            }
            progress.items_done += 1;
        }
        progress.current = None;
        sink.progress(&progress);
        report
    }

    fn run_step(
        &self,
        step: &Step,
        policy: &mut ConflictPolicy,
        sink: &dyn JobSink,
        ctx: &OpContext,
    ) -> Result<Outcome, OpsError> {
        match step {
            Step::Copy { from, to } => {
                let source = self.registry.resolve(from)?;
                if source.stat(from, ctx)?.is_container() {
                    return Err(OpsError::NotBuilt("folder copy".to_owned()));
                }
                let Some((to, mode)) = self.settle_conflict(from, to, policy, sink, ctx)? else {
                    return Ok(Outcome::Skipped("the name is taken".to_owned()));
                };
                let target = self.registry.resolve(&to)?;
                let mut reader = source.read(from, ctx)?;
                let mut writer = target.write(&to, mode, ctx)?;
                io::copy(&mut reader, &mut writer).map_err(VfsError::from)?;
                Ok(Outcome::Done)
            }
            Step::Move { from, to } | Step::Rename { from, to } => {
                let source = self.registry.resolve(from)?;
                if source.scheme() != self.registry.resolve(to)?.scheme() {
                    return Err(OpsError::NotBuilt(
                        "moving between kinds of place".to_owned(),
                    ));
                }
                let Some((to, _)) = self.settle_conflict(from, to, policy, sink, ctx)? else {
                    return Ok(Outcome::Skipped("the name is taken".to_owned()));
                };
                source.rename(from, &to, ctx)?;
                Ok(Outcome::Done)
            }
            Step::MakeFolder { place } => {
                self.registry.resolve(place)?.create_dir(place, ctx)?;
                Ok(Outcome::Done)
            }
            Step::Trash { place } => {
                self.registry.resolve(place)?.trash(place, ctx)?;
                Ok(Outcome::Done)
            }
            Step::Restore { .. } => Err(OpsError::NotBuilt("restore".to_owned())),
            Step::Remove { place } => {
                self.registry.resolve(place)?.remove(place, ctx)?;
                Ok(Outcome::Done)
            }
            Step::SetAttributes { place, change } => {
                self.registry
                    .resolve(place)?
                    .set_attributes(place, change, ctx)?;
                Ok(Outcome::Done)
            }
        }
    }

    /// Where to write, and how, given the conflict policy; `None` to skip.
    fn settle_conflict(
        &self,
        from: &Place,
        to: &Place,
        policy: &mut ConflictPolicy,
        sink: &dyn JobSink,
        ctx: &OpContext,
    ) -> Result<Option<(Place, WriteMode)>, OpsError> {
        let target = self.registry.resolve(to)?;
        match target.stat(to, ctx) {
            Err(VfsError::NotFound) => return Ok(Some((to.clone(), WriteMode::CreateNew))),
            Err(other) => return Err(other.into()),
            Ok(_) => {}
        }
        if *policy == ConflictPolicy::Ask {
            let answer = sink.conflict(&Conflict {
                source: from.clone(),
                target: to.clone(),
            });
            *policy = if answer == ConflictPolicy::Ask {
                ConflictPolicy::Skip
            } else {
                answer
            };
        }
        match *policy {
            ConflictPolicy::Ask | ConflictPolicy::Skip => Ok(None),
            ConflictPolicy::Overwrite => Ok(Some((to.clone(), WriteMode::Overwrite))),
            ConflictPolicy::Newer => {
                let source = self.registry.resolve(from)?.stat(from, ctx)?.modified;
                let existing = target.stat(to, ctx)?.modified;
                Ok((source > existing).then(|| (to.clone(), WriteMode::Overwrite)))
            }
            ConflictPolicy::KeepBoth | ConflictPolicy::Rename => {
                let folder = to.parent().ok_or(OpsError::NoDestination)?;
                let name = to.name().ok_or(OpsError::NoName)?;
                for n in 2..1000 {
                    let candidate = folder
                        .join(&numbered_name(name, n))
                        .map_err(VfsError::from)?;
                    if matches!(target.stat(&candidate, ctx), Err(VfsError::NotFound)) {
                        return Ok(Some((candidate, WriteMode::CreateNew)));
                    }
                }
                Ok(None)
            }
        }
    }
}

/// A numbered name for keeping both: "report (2).pdf", "Makefile (2)",
/// ".profile (2)".
pub fn numbered_name(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(at) if at > 0 => format!("{} ({n}){}", &name[..at], &name[at..]),
        _ => format!("{name} ({n})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;
    use std::sync::Mutex;

    fn p(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl JobSink for Recorder {
        fn progress(&self, progress: &JobProgress) {
            self.0.lock().unwrap().push(progress.describe());
        }

        fn conflict(&self, _conflict: &Conflict) -> ConflictPolicy {
            ConflictPolicy::Skip
        }
    }

    #[test]
    fn copy_plans_one_step_per_source() {
        let engine = Engine::new(Arc::new(Registry::new()));
        let request =
            JobRequest::new(JobId::new(7), JobKind::Copy, vec![p("/a/x"), p("/a/y")]).to(p("/b"));
        let plan = engine.plan(&request).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(
            plan.steps[1],
            Step::Copy {
                from: p("/a/y"),
                to: p("/b/y")
            }
        );
    }

    #[test]
    fn plans_refuse_what_is_missing() {
        let engine = Engine::new(Arc::new(Registry::new()));
        let copy = JobRequest::new(JobId::new(1), JobKind::Copy, vec![p("/a/x")]);
        assert_eq!(engine.plan(&copy), Err(OpsError::NoDestination));
        let rename = JobRequest::new(JobId::new(2), JobKind::Rename, vec![p("/a/x")]);
        assert_eq!(engine.plan(&rename), Err(OpsError::NoName));
        let none = JobRequest::new(JobId::new(3), JobKind::Trash, Vec::new());
        assert_eq!(engine.plan(&none), Err(OpsError::NothingSelected));
        let folder = JobRequest::new(JobId::new(4), JobKind::MakeFolder, Vec::new())
            .to(p("/a"))
            .named("new");
        assert_eq!(
            engine.plan(&folder).unwrap().steps,
            vec![Step::MakeFolder { place: p("/a/new") }]
        );
    }

    #[test]
    fn failures_are_reported_per_entry() {
        let engine = Engine::new(Arc::new(Registry::new()));
        let request = JobRequest::new(JobId::new(9), JobKind::Trash, vec![p("/a/x")]);
        let plan = engine.plan(&request).unwrap();
        let sink = Recorder::default();
        let report = engine.run(&plan, &JobControl::new(), &sink);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            report.failed[0].1,
            "nothing is set up to open folder places"
        );
        assert_eq!(
            report.summary(),
            "Move to the trash finished: 0 done, 1 failed."
        );
        let said = sink.0.lock().unwrap();
        assert_eq!(said.first().unwrap(), "Moving to the trash 0 of 1");
        assert_eq!(said.last().unwrap(), "Moving to the trash 1 of 1");
    }

    #[test]
    fn a_cancelled_job_stops_before_its_steps() {
        let engine = Engine::new(Arc::new(Registry::new()));
        let request = JobRequest::new(JobId::new(9), JobKind::Trash, vec![p("/a/x")]);
        let plan = engine.plan(&request).unwrap();
        let control = JobControl::new();
        control.cancel.cancel();
        let report = engine.run(&plan, &control, &Recorder::default());
        assert!(report.cancelled);
        assert_eq!(report.summary(), "Move to the trash cancelled: 0 done.");
    }

    #[test]
    fn numbered_names_keep_the_extension() {
        assert_eq!(numbered_name("report.pdf", 2), "report (2).pdf");
        assert_eq!(numbered_name("Makefile", 3), "Makefile (3)");
        assert_eq!(numbered_name(".profile", 2), ".profile (2)");
    }

    #[test]
    fn the_conflict_question_ends_with_y_or_n() {
        let conflict = Conflict {
            source: p("/a/report.pdf"),
            target: p("/b/report.pdf"),
        };
        assert_eq!(
            conflict.question().text,
            "report.pdf exists in /b. Replace it? y or n"
        );
    }
}
