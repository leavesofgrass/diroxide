//! The engine: plans jobs and runs plans.
//!
//! The order is fixed (plan section 5.4): create, verify, then destroy.
//! A replacement is written and committed before anything it replaces is
//! removed; a provider's clone is tried before the destination is
//! touched; a trash failure never falls through to a permanent delete;
//! every job ends with a reconciliation against a fresh read of what it
//! made. The planner is in `planner.rs`, the run in `run.rs`; this module
//! holds the types the app speaks: the sink, the control, the conflict
//! and error questions, and the report.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use diroxide_core::clock::describe_time;
use diroxide_core::format::{count, human_size};
use diroxide_core::{
    Clock, ConflictAction, ConflictAnswer, ConflictKind, Entry, ErrorAction, JobId, JobKind,
    JobState, Message, Place,
};
use diroxide_vfs::{CancelToken, Registry, VfsError};

use crate::error::OpsError;
use crate::journal::Journal;
use crate::options::EngineOptions;
use crate::pace::{Pacer, SystemPacer};
use crate::plan::Plan;
use crate::progress::JobProgress;
use crate::run::{Run, Shared, watch_for_stalls};
use crate::undo::{UndoLog, UndoRecord};

pub use diroxide_vfs::names::numbered_name;

/// A name that is taken, with both entries, so the question can say how
/// they differ (research/file-operations.md C4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// What kind of clash it is.
    pub kind: ConflictKind,
    /// What was being copied or moved.
    pub source: Place,
    /// The place that already exists.
    pub target: Place,
    /// The source's entry.
    pub source_entry: Entry,
    /// The existing entry.
    pub target_entry: Entry,
    /// The name "Keep both" would give the new one: "report (2).pdf".
    pub keep_both_name: String,
}

/// One choice of a conflict question: its line, verb first and inside 40
/// cells, and the answer it gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictChoice {
    /// The line: "Skip this one", "Keep both: report (2).pdf".
    pub label: String,
    /// The answer. A rename's name is empty: the app asks for it.
    pub answer: ConflictAnswer,
}

fn newer_older(source: &Entry, target: &Entry) -> Option<&'static str> {
    match (source.modified, target.modified) {
        (Some(a), Some(b)) if a > b => Some("newer"),
        (Some(a), Some(b)) if a < b => Some("older"),
        _ => None,
    }
}

fn larger_smaller(source: &Entry, target: &Entry) -> Option<&'static str> {
    match (source.size, target.size) {
        (Some(a), Some(b)) if a > b => Some("larger"),
        (Some(a), Some(b)) if a < b => Some("smaller"),
        _ => None,
    }
}

fn fact_line(who: &str, entry: &Entry, clock: &dyn Clock) -> String {
    let mut parts = Vec::new();
    if let Some(size) = entry.size {
        parts.push(human_size(size));
    }
    if let Some(modified) = entry.modified {
        parts.push(describe_time(modified, clock));
    }
    if parts.is_empty() {
        format!("{who}: {}", entry.kind.word())
    } else {
        format!("{who}: {}", parts.join(", "))
    }
}

impl Conflict {
    /// The question's lines, the situation first (R8): "Name taken:
    /// report.pdf"; then how the new one differs ("New is newer and
    /// larger"); then a fact line for each ("Existing: 2.1 MB, October
    /// 9", "New: 2.3 MB, today, 9:02 PM"). The same file has one line,
    /// and asks nothing.
    pub fn lines(&self, clock: &dyn Clock) -> Vec<String> {
        let name = self.target.name().unwrap_or("the entry");
        let name = diroxide_core::names::spell_out(name);
        match self.kind {
            ConflictKind::SameFile => vec![format!("Same file: {name}. Nothing to do.")],
            ConflictKind::FileOverFolder => vec![format!("Cannot merge: {name} is a folder here")],
            ConflictKind::FolderOverFile => vec![format!("Cannot merge: {name} is a file here")],
            ConflictKind::FolderOverFolder => {
                vec![format!("Folder exists: {name}. Its contents are merged.")]
            }
            ConflictKind::FileOverFile | ConflictKind::ReadOnlyTarget => {
                let time = newer_older(&self.source_entry, &self.target_entry);
                let size = larger_smaller(&self.source_entry, &self.target_entry);
                let difference = match (time, size) {
                    (Some(t), Some(s)) => format!("New is {t} and {s}"),
                    (Some(t), None) => format!("New is {t}, the same size"),
                    (None, Some(s)) => format!("New is {s}, from the same time"),
                    (None, None) => "New has the same size and time".to_owned(),
                };
                vec![
                    format!("{}: {name}", self.kind.situation()),
                    difference,
                    fact_line("Existing", &self.target_entry, clock),
                    fact_line("New", &self.source_entry, clock),
                ]
            }
        }
    }

    /// The choices, each line starting with its verb, in the order listed;
    /// focus starts on the first, "Skip this one", the choice that changes
    /// nothing, and Escape answers it. A file over a folder, or a folder
    /// over a file, is skipped and never resolved by deleting; the same
    /// file and a folder over a folder ask nothing.
    pub fn choices(&self) -> Vec<ConflictChoice> {
        let choice = |label: String, answer: ConflictAnswer| ConflictChoice { label, answer };
        let skip = choice("Skip this one".to_owned(), ConflictAnswer::skip());
        let skip_all = choice(
            "Skip all that exist".to_owned(),
            ConflictAnswer::for_all(ConflictAction::Skip),
        );
        let cancel = choice(
            "Cancel the job".to_owned(),
            ConflictAnswer::once(ConflictAction::Cancel),
        );
        match self.kind {
            ConflictKind::SameFile | ConflictKind::FolderOverFolder => Vec::new(),
            ConflictKind::FileOverFolder | ConflictKind::FolderOverFile => {
                vec![skip, skip_all, cancel]
            }
            ConflictKind::FileOverFile | ConflictKind::ReadOnlyTarget => vec![
                skip,
                skip_all,
                choice(
                    "Replace this one".to_owned(),
                    ConflictAnswer::once(ConflictAction::Replace),
                ),
                choice(
                    "Replace all that exist".to_owned(),
                    ConflictAnswer::for_all(ConflictAction::Replace),
                ),
                choice(
                    format!(
                        "Keep both: {}",
                        diroxide_core::names::spell_out(&self.keep_both_name)
                    ),
                    ConflictAnswer::once(ConflictAction::KeepBoth),
                ),
                choice(
                    "Keep both for all".to_owned(),
                    ConflictAnswer::for_all(ConflictAction::KeepBoth),
                ),
                choice(
                    "Replace only if newer, for all".to_owned(),
                    ConflictAnswer::for_all(ConflictAction::ReplaceIfNewer),
                ),
                choice(
                    "Rename this copy...".to_owned(),
                    ConflictAnswer::once(ConflictAction::Rename(String::new())),
                ),
                cancel,
            ],
        }
    }

    /// The Phase 0 question, kept for the app's naive dialog: the first
    /// line as a yes-or-no question about replacing. W1d moves to
    /// [`Conflict::lines`] and [`Conflict::choices`].
    pub fn question(&self) -> Message {
        let name = diroxide_core::names::spell_out(self.target.name().unwrap_or("The entry"));
        let folder = self.target.parent().map_or_else(String::new, |p| {
            format!(" in {}", diroxide_core::names::spell_out(&p.to_string()))
        });
        Message::question(&format!("{name} exists{folder}. Replace it?"))
    }
}

/// An error on one item, as the sink is asked about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemError {
    /// The job's kind, for the message's verb.
    pub kind: JobKind,
    /// The item.
    pub place: Place,
    /// What went wrong.
    pub error: VfsError,
    /// Which try failed, from 1.
    pub attempt: u32,
}

impl ItemError {
    /// The message in the house style: "Could not copy report.pdf:
    /// another program is using it. Close it in the other program, then
    /// retry." Hidden characters in the name are spelled out.
    pub fn message(&self) -> Message {
        let name = self
            .place
            .name()
            .map_or_else(|| self.place.to_string(), str::to_owned);
        let name = diroxide_core::names::spell_out(&name);
        Message::from_error(self.kind.verb(), &name, &self.error)
    }

    /// The choices, at most four and the same everywhere: Retry first for
    /// the errors retrying can fix (in use, network lost), Skip first for
    /// the rest, and no Retry where it cannot help.
    pub fn choices(&self) -> Vec<ErrorAction> {
        let class = self.error.class();
        let mut choices = Vec::with_capacity(4);
        if class.retry_first() {
            choices.push(ErrorAction::Retry);
        }
        choices.push(ErrorAction::Skip);
        choices.push(ErrorAction::SkipAllLikeThis);
        if class.can_retry() && !class.retry_first() {
            choices.push(ErrorAction::Retry);
        }
        choices.push(ErrorAction::Cancel);
        choices
    }
}

/// Receives a running job's progress and answers its questions. Called on
/// the job's thread; the answers block until the user gives them.
pub trait JobSink: Send + Sync {
    /// Takes a progress update. The app says a change of state at once
    /// and the rest at its progress interval.
    fn progress(&self, progress: &JobProgress);

    /// Answers one conflict. `apply_to_rest` settles the job's later
    /// conflicts of the same kind.
    fn conflict(&self, conflict: &Conflict) -> ConflictAnswer;

    /// Answers an error on one item that the job's policy does not already
    /// settle (the engine retries a lost network on its own first, and
    /// skips a class already answered "Skip all like this").
    fn error(&self, error: &ItemError) -> ErrorAction;
}

/// Cancels or pauses a running job. Cloning shares the control.
#[derive(Clone, Debug, Default)]
pub struct JobControl {
    /// Cancels the job between steps, and inside providers through the
    /// token.
    pub cancel: CancelToken,
    paused: Arc<AtomicBool>,
}

impl JobControl {
    /// A control that is neither cancelled nor paused.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pauses the job after its current chunk.
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

/// What a job made, checked against a fresh read after the run (plan
/// section 5.4): every planned result, whether it is present, and which
/// are missing. The count comes first in the result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reconciliation {
    /// What the plan was to make or change.
    pub planned: Vec<Place>,
    /// What a fresh read found.
    pub present: Vec<Place>,
    /// What a fresh read did not find, though the job did not skip or fail
    /// it: a provider that said yes and did nothing.
    pub missing: Vec<Place>,
}

impl Reconciliation {
    /// A reconciliation of a plan's results, nothing read yet.
    pub fn of(planned: Vec<Place>) -> Self {
        Self {
            planned,
            present: Vec::new(),
            missing: Vec::new(),
        }
    }

    /// Whether every planned result that was not skipped or failed was
    /// found.
    pub fn is_whole(&self) -> bool {
        self.missing.is_empty()
    }

    /// The result's line, the count first: "Checked 120 of 120: all
    /// present", "Checked 118 of 120: 2 missing".
    pub fn summary(&self) -> String {
        let checked = self.present.len() + self.missing.len();
        if self.missing.is_empty() {
            format!("Checked {checked} of {}: all present", self.planned.len())
        } else {
            format!(
                "Checked {checked} of {}: {} missing",
                self.planned.len(),
                self.missing.len()
            )
        }
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
    /// Done, with something to say: an attribute or time that could not be
    /// kept, a trash that could not be checked.
    pub warnings: Vec<(Place, String)>,
    /// Whether the job was cancelled before its end.
    pub cancelled: bool,
    /// Whether the job stopped at its first failure, as its error policy
    /// said ([`diroxide_core::job::ErrorPolicy::StopAtFirst`]); the failure
    /// is in `failed`.
    pub stopped: bool,
    /// How many times the engine tried an item again on its own, silently,
    /// after a lost network: counted here and never said one by one.
    pub retries: u32,
    /// The check of what the job made against a fresh read, when it ran.
    pub reconciliation: Option<Reconciliation>,
    /// What the Undo command can reverse.
    pub undo: UndoRecord,
    /// A join's whole and its verdict, said after the summary; a verified
    /// join's [`JoinOutcome::trash_offer`] asks about the parts.
    ///
    /// [`JoinOutcome::trash_offer`]: crate::join::JoinOutcome::trash_offer
    pub joined: Option<crate::join::JoinOutcome>,
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
            warnings: Vec::new(),
            cancelled: false,
            stopped: false,
            retries: 0,
            reconciliation: None,
            undo: UndoRecord::for_job(job, kind),
            joined: None,
        }
    }

    /// The report in one line, the problems first (R8): "Copy had 8
    /// failures: 290 done, 2 skipped."; "Copy cancelled: 2 done."; "Copy
    /// stopped at the first error: 2 done."; "Copy finished with 1
    /// warning: 3 done."; "Copy finished: 3 done, 1 skipped." A missing
    /// result counts as a failure.
    pub fn summary(&self) -> String {
        let verb = summary_word(self.kind);
        let mut counts = vec![format!("{} done", self.done.len())];
        if !self.skipped.is_empty() {
            counts.push(format!("{} skipped", self.skipped.len()));
        }
        let missing = self.reconciliation.as_ref().map_or(0, |r| r.missing.len());
        let failures = self.failed.len() + missing;
        let head = if self.cancelled {
            format!("{verb} cancelled")
        } else if self.stopped {
            format!("{verb} stopped at the first error")
        } else if failures > 0 {
            format!(
                "{verb} had {}",
                count(failures as u64, "failure", "failures")
            )
        } else if !self.warnings.is_empty() {
            format!(
                "{verb} finished with {}",
                count(self.warnings.len() as u64, "warning", "warnings")
            )
        } else {
            format!("{verb} finished")
        };
        format!("{head}: {}.", counts.join(", "))
    }
}

/// The word a report's line starts with: the job's verb, capitalized
/// ("Copy", "Move to the trash"), except where the verb needs its object
/// and would read wrong before "finished": "Permissions finished: 1
/// done.", not "Change the attributes of finished".
fn summary_word(kind: JobKind) -> String {
    match kind {
        JobKind::SetAttributes => "Permissions".to_owned(),
        JobKind::SetTimes => "Times".to_owned(),
        JobKind::MakeFolder => "Make folder".to_owned(),
        JobKind::Unknown => "Job".to_owned(),
        _ => {
            let verb = kind.verb();
            let mut chars = verb.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// A sink that says nothing, skips every conflict and every error: for
/// planning without a frontend.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Quiet;

impl JobSink for Quiet {
    fn progress(&self, _progress: &JobProgress) {}

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

/// The job engine. Planning is quick; running blocks, so the app runs it
/// on a worker thread.
#[derive(Clone, Debug)]
pub struct Engine {
    registry: Arc<Registry>,
    options: EngineOptions,
    pacer: Arc<dyn Pacer>,
    journal: Option<Journal>,
}

impl Engine {
    /// An engine that reaches places through this registry, with the
    /// default options, the real clock and no journal.
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            options: EngineOptions::default(),
            pacer: Arc::new(SystemPacer::new()),
            journal: None,
        }
    }

    /// The same engine with these options.
    pub fn with_options(mut self, options: EngineOptions) -> Self {
        self.options = options;
        self
    }

    /// The same engine reading time from this pacer (a [`FakePacer`] in
    /// tests, so waits take no time).
    ///
    /// [`FakePacer`]: crate::pace::FakePacer
    pub fn with_pacer(mut self, pacer: Arc<dyn Pacer>) -> Self {
        self.pacer = pacer;
        self
    }

    /// The registry the engine uses.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The engine's options.
    pub fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// The engine's clock.
    pub fn pacer(&self) -> &dyn Pacer {
        &*self.pacer
    }

    /// The same engine writing the undo journal into this journal's
    /// folder, before each job's first step and after every step.
    pub fn with_journal(mut self, journal: Journal) -> Self {
        self.journal = Some(journal);
        self
    }

    /// The undo journal, when the engine writes one.
    pub fn journal(&self) -> Option<&Journal> {
        self.journal.as_ref()
    }

    /// Runs a plan to its end, or until cancelled, reporting progress and
    /// asking about conflicts and errors through the sink. Blocking: the
    /// app runs it on a worker thread. A stall detector watches beside it
    /// and says "stalled" when nothing moves for a while.
    pub fn run(&self, plan: &Plan, control: &JobControl, sink: &dyn JobSink) -> JobReport {
        let by_bytes = matches!(
            plan.kind,
            JobKind::Copy | JobKind::Duplicate | JobKind::Move | JobKind::Split | JobKind::Join
        ) && plan.facts.counted
            && plan.facts.bytes > 0;
        // A split counts its parts, not its sidecars.
        let items = match &plan.facts.split {
            Some(split) if plan.kind == JobKind::Split => split.parts(),
            _ => plan.steps.len() as u64,
        };
        let mut progress =
            JobProgress::start(plan.kind, items, by_bytes.then_some(plan.facts.bytes));
        progress.state = JobState::Running;
        let shared = Shared::new(progress);
        let stall = self.options.stall_after;
        thread::scope(|scope| {
            let (stop, stopped) = mpsc::channel::<()>();
            if !stall.is_zero() {
                let shared = &shared;
                scope.spawn(move || watch_for_stalls(shared, sink, stall, &stopped));
            }
            let mut run = Run::new(self, plan, control, sink, &shared);
            run.execute();
            drop(stop);
            run.into_report()
        })
    }

    /// Undoes the newest job in the log: plans its undo
    /// ([`Engine::plan_undo`]), takes it off the log and runs it. The
    /// report says what could not be undone, each with its reason (an
    /// entry changed since, a name taken, a place with no trash).
    pub fn undo_last(
        &self,
        log: &mut UndoLog,
        job: JobId,
        control: &JobControl,
        sink: &dyn JobSink,
    ) -> Result<JobReport, OpsError> {
        let record = log.last().cloned().ok_or(OpsError::NothingToUndo)?;
        let plan = self.plan_undo(&record, job)?;
        log.pop();
        Ok(self.run(&plan, control, sink))
    }

    /// Whether replacing at a conflict's target can be undone: the place
    /// has a trash and the old file goes there first. When it cannot, the
    /// question says "Replacing cannot be undone."
    pub fn replace_can_be_undone(&self, conflict: &Conflict) -> bool {
        self.options.replace_keeps_old_in_trash
            && self
                .registry
                .resolve(&conflict.target)
                .is_ok_and(|provider| provider.capabilities(&conflict.target).trash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Step;
    use crate::testing;
    use diroxide_core::format::within_cells;
    use diroxide_core::place::PathStyle;
    use diroxide_core::{ConflictPolicy, JobRequest};
    use diroxide_core::{EntryKind, FixedClock, Timestamp, UtcOffset};
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

        fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
            ConflictAnswer::skip()
        }

        fn error(&self, _error: &ItemError) -> ErrorAction {
            ErrorAction::Skip
        }
    }

    fn fake_engine() -> Engine {
        let fake = crate::fake::FakeProvider::new();
        fake.add_file(&testing::p("/a/x"), b"x");
        fake.add_file(&testing::p("/a/y"), b"y");
        fake.add_file(&testing::p("/a/1.jpg"), b"1");
        fake.add_file(&testing::p("/a/2.jpg"), b"2");
        fake.add_folder(&testing::p("/b"));
        testing::engine(&fake)
    }

    #[test]
    fn copy_plans_one_step_per_source() {
        let engine = fake_engine();
        let request = JobRequest::new(
            JobId::new(7),
            JobKind::Copy,
            vec![testing::p("/a/x"), testing::p("/a/y")],
        )
        .to(testing::p("/b"));
        let plan = engine.plan(&request).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(
            plan.steps[1],
            Step::Copy {
                from: testing::p("/a/y"),
                to: testing::p("/b/y")
            }
        );
    }

    #[test]
    fn plans_refuse_what_is_missing() {
        let engine = fake_engine();
        let copy = JobRequest::new(JobId::new(1), JobKind::Copy, vec![p("/a/x")]);
        assert_eq!(engine.plan(&copy), Err(OpsError::NoDestination));
        let rename = JobRequest::new(JobId::new(2), JobKind::Rename, vec![p("/a/x")]);
        assert_eq!(engine.plan(&rename), Err(OpsError::NoName));
        let none = JobRequest::new(JobId::new(3), JobKind::Trash, Vec::new());
        assert_eq!(engine.plan(&none), Err(OpsError::NothingSelected));
        let folder = JobRequest::new(JobId::new(4), JobKind::MakeFolder, Vec::new())
            .to(testing::p("/a"))
            .named("new");
        assert_eq!(
            engine.plan(&folder).unwrap().steps,
            vec![Step::MakeFolder {
                place: testing::p("/a/new")
            }]
        );
        let unknown = JobRequest::new(JobId::new(5), JobKind::Unknown, vec![p("/a/x")]);
        assert!(matches!(engine.plan(&unknown), Err(OpsError::NotBuilt(_))));
        let undo = JobRequest::new(JobId::new(6), JobKind::Undo, vec![p("/a/x")]);
        assert_eq!(engine.plan(&undo), Err(OpsError::NothingToUndo));
    }

    #[test]
    fn several_renames_plan_from_their_pairs() {
        let engine = fake_engine();
        let request = JobRequest::new(JobId::new(6), JobKind::Rename, Vec::new()).renaming(vec![
            (testing::p("/a/1.jpg"), "one.jpg".into()),
            (testing::p("/a/2.jpg"), "two.jpg".into()),
        ]);
        let plan = engine.plan(&request).unwrap();
        assert_eq!(
            plan.steps[1],
            Step::Rename {
                from: testing::p("/a/2.jpg"),
                to: testing::p("/a/two.jpg")
            }
        );
    }

    #[test]
    fn failures_are_reported_per_entry_problems_first() {
        let engine = Engine::new(Arc::new(Registry::new()));
        let plan = Plan::new(
            JobId::new(9),
            JobKind::Trash,
            vec![Step::Trash { place: p("/a/x") }],
            ConflictPolicy::Ask,
        );
        let sink = Recorder::default();
        let report = engine.run(&plan, &JobControl::new(), &sink);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            report.failed[0].1,
            "nothing is set up to open folder places"
        );
        assert_eq!(report.summary(), "Move to the trash had 1 failure: 0 done.");
        let said = sink.0.lock().unwrap();
        assert_eq!(said.first().unwrap(), "Moving to the trash 0 percent");
        assert_eq!(said.last().unwrap(), "Moving to the trash finished");
    }

    #[test]
    fn a_cancelled_job_stops_before_its_steps() {
        let engine = Engine::new(Arc::new(Registry::new()));
        let plan = Plan::new(
            JobId::new(9),
            JobKind::Trash,
            vec![Step::Trash { place: p("/a/x") }],
            ConflictPolicy::Ask,
        );
        let control = JobControl::new();
        control.cancel.cancel();
        let report = engine.run(&plan, &control, &Recorder::default());
        assert!(report.cancelled);
        assert_eq!(report.summary(), "Move to the trash cancelled: 0 done.");
    }

    #[test]
    fn summaries_put_problems_first_inside_40_cells() {
        let mut report = JobReport::new(JobId::new(1), JobKind::Copy);
        report.done = vec![p("/b/x"); 290];
        report.skipped = vec![(p("/b/y"), "taken".into()); 2];
        report.failed = vec![(p("/b/z"), "in use".into()); 8];
        assert_eq!(
            report.summary(),
            "Copy had 8 failures: 290 done, 2 skipped."
        );
        assert!(report.summary().chars().count() <= 41);
        report.failed.clear();
        report.warnings.push((p("/b/x"), "times not kept".into()));
        assert_eq!(
            report.summary(),
            "Copy finished with 1 warning: 290 done, 2 skipped."
        );
        report.warnings.clear();
        report.reconciliation = Some(Reconciliation {
            planned: vec![p("/b/x"), p("/b/w")],
            present: vec![p("/b/x")],
            missing: vec![p("/b/w")],
        });
        assert!(report.summary().starts_with("Copy had 1 failure"));
        assert_eq!(
            report.reconciliation.as_ref().unwrap().summary(),
            "Checked 2 of 2: 1 missing"
        );
    }

    #[test]
    fn numbered_names_keep_the_extension() {
        assert_eq!(numbered_name("report.pdf", 2), "report (2).pdf");
        assert_eq!(numbered_name("Makefile", 3), "Makefile (3)");
        assert_eq!(numbered_name(".profile", 2), ".profile (2)");
        assert_eq!(numbered_name("archive.tar.gz", 2), "archive (2).tar.gz");
    }

    fn conflict(kind: ConflictKind) -> Conflict {
        let now = Timestamp::from_unix(1_791_000_000, 0);
        Conflict {
            kind,
            source: p("/a/report.pdf"),
            target: p("/b/report.pdf"),
            source_entry: Entry::new("report.pdf", EntryKind::File)
                .with_size(2_300_000)
                .with_modified(now),
            target_entry: Entry::new("report.pdf", EntryKind::File)
                .with_size(2_100_000)
                .with_modified(now.plus_seconds(-86_400 * 3)),
            keep_both_name: "report (2).pdf".into(),
        }
    }

    #[test]
    fn the_conflict_question_says_the_situation_first() {
        let clock = FixedClock::new(Timestamp::from_unix(1_791_000_000, 0), UtcOffset::UTC);
        let lines = conflict(ConflictKind::FileOverFile).lines(&clock);
        assert_eq!(lines[0], "Name taken: report.pdf");
        assert_eq!(lines[1], "New is newer and larger");
        assert!(lines[2].starts_with("Existing: 2.1 MB, "));
        assert!(lines[3].starts_with("New: 2.3 MB, today"));
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line}");
        }
        let choices = conflict(ConflictKind::FileOverFile).choices();
        assert_eq!(choices.len(), 9);
        assert_eq!(choices[0].label, "Skip this one");
        assert_eq!(choices[0].answer, ConflictAnswer::skip());
        assert_eq!(choices[4].label, "Keep both: report (2).pdf");
        assert_eq!(choices[8].label, "Cancel the job");
        for choice in &choices {
            assert!(choice.label.chars().count() <= 40, "{}", choice.label);
        }
        let same = conflict(ConflictKind::SameFile);
        assert_eq!(
            same.lines(&clock),
            vec!["Same file: report.pdf. Nothing to do."]
        );
        assert!(same.choices().is_empty());
        let clash = conflict(ConflictKind::FolderOverFile);
        assert!(within_cells(
            &clash.lines(&clock)[0],
            "Cannot merge: report.pdf"
        ));
        assert_eq!(clash.choices().len(), 3);
        assert_eq!(
            conflict(ConflictKind::FileOverFile).question().text,
            "report.pdf exists in /b. Replace it? y or n"
        );
    }

    #[test]
    fn errors_offer_retry_only_where_it_helps() {
        let error = |error| ItemError {
            kind: JobKind::Copy,
            place: p("/a/report.pdf"),
            error,
            attempt: 1,
        };
        assert_eq!(
            error(VfsError::InUse).choices(),
            vec![
                ErrorAction::Retry,
                ErrorAction::Skip,
                ErrorAction::SkipAllLikeThis,
                ErrorAction::Cancel
            ]
        );
        assert_eq!(
            error(VfsError::ReadOnlyFilesystem).choices(),
            vec![
                ErrorAction::Skip,
                ErrorAction::SkipAllLikeThis,
                ErrorAction::Cancel
            ]
        );
        assert_eq!(
            error(VfsError::PermissionDenied).choices()[0],
            ErrorAction::Skip
        );
        let message = error(VfsError::InUse).message();
        assert!(within_cells(&message.text, "Could not copy report.pdf"));
    }

    #[test]
    fn every_report_line_starts_with_a_word_that_reads_before_finished() {
        let report = |kind| {
            let mut report = JobReport::new(JobId::new(1), kind);
            report.done.push(p("/a"));
            report.summary()
        };
        assert_eq!(
            report(JobKind::SetAttributes),
            "Permissions finished: 1 done."
        );
        assert_eq!(report(JobKind::SetTimes), "Times finished: 1 done.");
        assert_eq!(report(JobKind::MakeFolder), "Make folder finished: 1 done.");
        assert_eq!(report(JobKind::Copy), "Copy finished: 1 done.");
        assert_eq!(
            report(JobKind::Trash),
            "Move to the trash finished: 1 done."
        );
        for &kind in JobKind::ALL {
            let line = report(kind);
            assert!(!line.contains(" of finished"), "{line}");
            assert!(!line.contains(" on finished"), "{line}");
            assert!(!line.contains(" the folder finished"), "{line}");
        }
    }
}
