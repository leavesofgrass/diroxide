//! The runner (R16 3.3 and 3.8; R28 6.3): one task's queue around the
//! arrival tracker, stepped by its caller with the time, so the whole
//! machine runs in tests against a fixed clock; [`super::spawn`] runs it
//! on a thread over a real watch, and [`super::once`] runs it to the end
//! of what is waiting for `diro watch --task NAME --once`.
//!
//! **One file, start to end.** A file that settled and matches waits in a
//! first-in, first-out queue, ties by natural name order. Its turn:
//!
//! 1. Its facts are read and the ledger asked: a file already handled is
//!    passed over, and one whose output was committed before a crash only
//!    has its original moved (nothing is converted twice).
//! 2. Free space is checked where the result goes (below 1 GB, or a tenth
//!    of the drive, the task pauses).
//! 3. The action runs through the job engine: a conversion through the
//!    Convert engine (its tool through the engine's `ToolRunner`, carta in
//!    the child the frontend gives), a move or copy as an ordinary job,
//!    a script or plugin command through the frontend's
//!    [`TaskActions`]. Jobs carry `JobOrigin::HotFolder` and continue and
//!    list, so nothing ever asks and nothing is ever replaced.
//! 4. A made line goes to the ledger, flushed, before the original moves.
//! 5. **Look again:** the file's facts are read again and compared with
//!    the ones taken at the start, and the writer probe asked
//!    ([`super::probe`]); a change means "Changed during the run" and the
//!    file settles and runs again.
//! 6. The original moves to `processed` (or the task's folder), never over
//!    a file; then a done line, the log line, and the polite summary.
//!
//! **When it fails,** by its cause ([`super::retry`]): tried again after
//! 10 seconds, 1 minute and 10 minutes; filed in `failed`; or the task
//! paused, every file left in `in`. Ten files in a row failing with one
//! cause pause it too.
//!
//! **At start:** the task's lock, its approval, the made lines of a crash
//! finished, the partial outputs a crash left listed (never removed), and
//! the catch-up: the files already in `in` are seen just now and settle,
//! or, when the owner said none, they are left.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use diroxide_core::job::ErrorPolicy;
use diroxide_core::names::spell_out;
use diroxide_core::{
    Clock, ConflictPolicy, Entry, EntryKind, JobId, JobKind, JobOrigin, JobRequest, Place,
    SystemClock, Timestamp,
};
use diroxide_vfs::names::is_unfinished;
use diroxide_vfs::{OpContext, Provider, VfsError, WatchEvent};

use super::actions::{ActionResult, ActionRun, TaskActions};
use super::approval::{Approval, Resolved, approval_key};
use super::ledger::{Identity, Ledger, Line, Outcome};
use super::lines::{
    CATCH_UP_YES_UP_TO, FOLDER_MISSING, across_line, breaker_reason, catch_up_question,
    caught_up_line, changed_line, done_line, failed_line, full_line, left_line, locked_line,
    partial_line, paused_line, polled_line, retry_line, skipped_line, space_reason, status_line,
    summary_line, told_line, took_words,
};
use super::log::{TaskLog, log_line};
use super::probe::still_written;
use super::retry::{
    BREAKER_AT, Breaker, CauseClass, RETRY_WAITS, cause_key, classify, wait_before,
};
use super::store::{StoreError, TaskLock, TaskStore};
use super::task::{After, Announce, CatchUpChoice, OnlineOnly, Task, TaskAction};
use super::words::{duration_words, sentence};
use crate::convert::carta::CartaRunner;
use crate::convert::plan::{ConvertError, ConvertRequest};
use crate::convert::recipe::Recipe;
use crate::convert::staging::is_staging_name;
use crate::engine::{Engine, JobControl, Quiet};
use crate::tracker::{
    Facts, Look, SettleRules, SkipReason, Tracker, TrackerEvent, between, is_ignored, later_by,
    writer_open,
};

/// The numbers a runner works by, R16's starting values by default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerOptions {
    /// The summary's interval: at most one line this often (the app's
    /// progress interval; ten seconds by default).
    pub pace: Duration,
    /// The shortest gap between two lines of `announce = "each"`.
    pub each_gap: Duration,
    /// The waits before each retry.
    pub retry_waits: Vec<Duration>,
    /// Files in a row failing with one cause that pause the task.
    pub breaker: u32,
    /// The most files that wait in the queue; past it they wait in `in`
    /// and the task says so once.
    pub queue_cap: usize,
    /// Below this many free bytes where results go, the task pauses.
    pub space_floor: u64,
    /// Below this percentage of the drive free, the task pauses.
    pub space_percent: u64,
    /// How often a place that cannot be watched, or a folder that went
    /// away, is listed.
    pub poll_every: Duration,
    /// How often settling files are looked at.
    pub tick: Duration,
    /// How long a file may keep changing before the task stops waiting
    /// for it.
    pub max_wait: Duration,
}

impl Default for RunnerOptions {
    fn default() -> Self {
        Self {
            pace: Duration::from_secs(10),
            each_gap: Duration::from_secs(2),
            retry_waits: RETRY_WAITS.to_vec(),
            breaker: BREAKER_AT,
            queue_cap: 10_000,
            space_floor: 1_000_000_000,
            space_percent: 10,
            poll_every: Duration::from_secs(30),
            tick: Duration::from_millis(500),
            max_wait: Duration::from_secs(3600),
        }
    }
}

/// What a file came to, for `diro watch`'s lines and the lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileOutcome {
    /// Handled: made and moved, or moved, copied or run.
    Done,
    /// Filed in `failed`, or left in place in `leave` mode.
    Failed,
    /// Nothing was needed (the output exists, or is up to date).
    Skipped,
    /// It will be tried again.
    Retrying,
    /// It changed during the run; it settles and runs again.
    Changed,
    /// A tell task said it arrived.
    Told,
    /// It kept changing for longer than the longest wait.
    GaveUp,
    /// `--once`: its retry waits for the next run.
    Left,
}

/// One file's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileNote {
    /// The file's name in `in`.
    pub name: String,
    /// What it came to.
    pub outcome: FileOutcome,
    /// The line: "Done: scan-004.pdf, 3 seconds".
    pub line: String,
    /// The output made, when there is one.
    pub output: Option<Place>,
}

/// What a runner tells its frontend.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HotNote {
    /// The task began: how many files were waiting to be caught up on,
    /// and how often the place is listed when it cannot be watched.
    Started {
        /// The files waiting that it will process.
        waiting: usize,
        /// The listing interval of a polled place; `None` when watched.
        polled: Option<Duration>,
    },
    /// A polite line to say now, paced by the task's `announce`.
    Say(String),
    /// The status line, never spoken; `None` when nothing waits.
    Status(Option<String>),
    /// One file's outcome.
    File(FileNote),
    /// The task paused: "Scans paused: pdftotext is not installed".
    Paused(String),
    /// The task goes on after a pause.
    Resumed,
}

/// Why a task did not start.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartError {
    /// Another runner has it: "Scans is run by another diro".
    #[error("{0}")]
    Locked(String),
    /// The owner has not approved it as it is now.
    #[error("{question}")]
    NotApproved {
        /// Never, or changed since.
        approval: Approval,
        /// The question to ask after the task's sentence.
        question: String,
        /// The key to approve with ([`TaskStore::approve`]).
        key: String,
    },
    /// The first turn-on asks about the files already waiting
    /// ([`TaskStore::remember_catch_up`], then start again).
    #[error("{question}")]
    AskCatchUp {
        /// How many files wait.
        waiting: usize,
        /// "Process the 14 files in Scans? y or n".
        question: String,
        /// Whether the question starts on Yes (20 files or fewer).
        yes_first: bool,
    },
    /// It cannot run: "Could not run Scans: no recipe is named
    /// pdf-to-text."
    #[error("{0}")]
    Cannot(String),
}

/// A point in a file's run where a test or a spike may stop the runner as
/// a crash would ([`TaskRunner::with_checkpoint`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Checkpoint {
    /// The output is committed and its made line is on the disk; the
    /// original has not moved.
    Committed,
    /// The original moved; its done line is not written yet.
    Moved,
}

/// The checkpoint's observer: `Break` stops the runner there.
pub type CheckpointFn = Arc<dyn Fn(Checkpoint, &str) -> ControlFlow<()> + Send + Sync>;

/// Stops a runner's running jobs from another thread
/// ([`TaskRunner::interrupter`]).
#[derive(Clone, Debug)]
pub struct Interrupter(Arc<Mutex<Vec<(JobId, JobControl)>>>);

impl Interrupter {
    /// Interrupts every job the runner runs now: each file stays in `in`
    /// and its partial output is removed by its job.
    pub fn interrupt(&self) {
        for (_, control) in guard(&self.0).iter() {
            control.interrupt();
        }
    }
}

/// A file in the queue, with the tries it has had.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Queued {
    name: String,
    attempt: u32,
}

/// A file waiting for its retry.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Waiting {
    name: String,
    at: Timestamp,
    attempt: u32,
}

/// What one file's run came to, applied to the runner's state after.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ran {
    Done {
        name: String,
        output: Option<Place>,
        took: Duration,
        across: bool,
        finished: bool,
    },
    Skipped {
        name: String,
        why: String,
        quiet: bool,
    },
    Told {
        name: String,
        size: Option<u64>,
    },
    Again {
        name: String,
        cause: String,
        attempt: u32,
    },
    Failed {
        name: String,
        cause: String,
    },
    Left {
        name: String,
    },
    Changed {
        name: String,
        attempt: u32,
    },
    Pause {
        name: Option<String>,
        attempt: u32,
        reason: String,
    },
    Gone {
        name: String,
    },
    Stopped,
}

impl Ran {
    /// The file's name, when the outcome is about one.
    fn name(&self) -> Option<&str> {
        match self {
            Ran::Done { name, .. }
            | Ran::Skipped { name, .. }
            | Ran::Told { name, .. }
            | Ran::Again { name, .. }
            | Ran::Failed { name, .. }
            | Ran::Left { name }
            | Ran::Changed { name, .. }
            | Ran::Gone { name } => Some(name),
            Ran::Pause { name, .. } => name.as_deref(),
            Ran::Stopped => None,
        }
    }
}

/// What an action did.
enum Act {
    Made(Vec<Place>),
    Moved(Option<Place>),
    Skipped(String),
    Fail(String),
    /// Failed, and the action says trying again may help.
    Again(String),
    Pause(String),
    Cancelled,
}

/// The counts since the last summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    done: usize,
    failed: usize,
    skipped: usize,
}

impl Tally {
    fn is_empty(self) -> bool {
        self.done + self.failed + self.skipped == 0
    }
}

/// What a runner has done since it started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Files handled.
    pub done: usize,
    /// Files filed in `failed`.
    pub failed: usize,
    /// Files passed over (output exists, or up to date).
    pub skipped: usize,
    /// Files left in `in` for the next run (`--once`).
    pub left: usize,
}

/// One task's runner. See the module documentation.
pub struct TaskRunner {
    task: Task,
    title: String,
    engine: Engine,
    actions: Arc<dyn TaskActions>,
    store: TaskStore,
    clock: Arc<dyn Clock>,
    carta: CartaRunner,
    spool: Option<PathBuf>,
    options: RunnerOptions,
    checkpoint: Option<CheckpointFn>,
    once: bool,
    next_job: AtomicU64,
    running: Arc<Mutex<Vec<(JobId, JobControl)>>>,
    ledger: Mutex<Ledger>,
    log: TaskLog,
    lock: Option<TaskLock>,
    recipe: Option<Recipe>,
    tracker: Tracker,
    queue: VecDeque<Queued>,
    overflow: VecDeque<String>,
    said_full: bool,
    retries: Vec<Waiting>,
    attempts: HashMap<String, u32>,
    paused: Option<String>,
    missing: bool,
    stopped: bool,
    breaker: Breaker,
    tally: Tally,
    totals: Totals,
    summary_at: Option<Timestamp>,
    last_each: Option<Timestamp>,
    tick_at: Option<Timestamp>,
    poll_every: Option<Duration>,
    poll_at: Option<Timestamp>,
    status: Option<String>,
}

impl std::fmt::Debug for TaskRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskRunner")
            .field("task", &self.task.name)
            .field("queue", &self.queue)
            .field("paused", &self.paused)
            .finish_non_exhaustive()
    }
}

/// Locks a mutex, taking it back from a thread that panicked.
fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The first job id a runner gives, far above the app's own.
const FIRST_JOB: u64 = 1 << 40;

/// The longest `--once` waits for a file that keeps changing.
pub const ONCE_MAX_WAIT: Duration = Duration::from_secs(120);

impl TaskRunner {
    /// A runner for a checked task, running its jobs on `engine` (with the
    /// tools registry set), asking `actions` for what this crate cannot
    /// reach, keeping its state in `store`. Reads the task's ledger.
    pub fn new(
        task: Task,
        engine: Engine,
        actions: Arc<dyn TaskActions>,
        store: TaskStore,
    ) -> Result<Self, StoreError> {
        let ledger = store.ledger()?;
        let log = store.log();
        let options = RunnerOptions::default();
        let rules = SettleRules {
            quiet: task.settle,
            renamed_quiet: task.settle.min(Duration::from_millis(500)),
            tick: options.tick,
            max_wait: options.max_wait,
        };
        Ok(Self {
            title: task.title(),
            tracker: Tracker::new(rules),
            breaker: Breaker::new(options.breaker),
            task,
            engine,
            actions,
            store,
            clock: Arc::new(SystemClock::utc()),
            carta: CartaRunner::InProcess,
            spool: None,
            options,
            checkpoint: None,
            once: false,
            next_job: AtomicU64::new(FIRST_JOB),
            running: Arc::new(Mutex::new(Vec::new())),
            ledger: Mutex::new(ledger),
            log,
            lock: None,
            recipe: None,
            queue: VecDeque::new(),
            overflow: VecDeque::new(),
            said_full: false,
            retries: Vec::new(),
            attempts: HashMap::new(),
            paused: None,
            missing: false,
            stopped: false,
            tally: Tally::default(),
            totals: Totals::default(),
            summary_at: None,
            last_each: None,
            tick_at: None,
            poll_every: None,
            poll_at: None,
            status: None,
        })
    }

    /// The same runner reading the time of its log lines and its runs
    /// from this clock (the app's, with its local offset).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The same runner giving carta recipes this runner: the app and the
    /// command line give a `diro` child they can kill
    /// (`CartaRunner::child(std::env::current_exe()?)`), since a task runs
    /// with no one there to wait.
    pub fn with_carta(mut self, carta: CartaRunner) -> Self {
        self.carta = carta;
        self
    }

    /// The same runner with the state folder's spool, for sources and
    /// results that are not on this computer.
    pub fn with_spool(mut self, spool: impl Into<PathBuf>) -> Self {
        self.spool = Some(spool.into());
        self
    }

    /// The same runner with these numbers.
    pub fn with_options(mut self, options: RunnerOptions) -> Self {
        let rules = SettleRules {
            tick: options.tick,
            max_wait: options.max_wait,
            ..self.tracker.rules()
        };
        self.tracker = Tracker::new(rules);
        self.breaker = Breaker::new(options.breaker);
        self.options = options;
        self
    }

    /// The same runner listing its `in` folder every `every` because the
    /// place cannot be watched; a polled place settles over at least ten
    /// seconds, since a listing sees less than a watch.
    pub fn with_polling(mut self, every: Duration) -> Self {
        let rules = self.tracker.rules();
        self.tracker = Tracker::new(rules.with_quiet(rules.quiet.max(SettleRules::POLLED.quiet)));
        self.poll_every = Some(every);
        self
    }

    /// The same runner calling `observer` at each [`Checkpoint`]; `Break`
    /// stops the runner there as a crash would. For tests and spikes.
    pub fn with_checkpoint(mut self, observer: CheckpointFn) -> Self {
        self.checkpoint = Some(observer);
        self
    }

    /// The same runner for `--once`: a file whose retry would wait is left
    /// in `in` for the next run, said, and counted, and a file still
    /// changing after [`ONCE_MAX_WAIT`] is given up on, so a scheduler's
    /// run always ends.
    pub fn once(mut self) -> Self {
        self.once = true;
        let rules = self.tracker.rules();
        self.tracker = Tracker::new(SettleRules {
            max_wait: rules.max_wait.min(ONCE_MAX_WAIT),
            ..rules
        });
        self
    }

    /// The task.
    pub fn task(&self) -> &Task {
        &self.task
    }

    /// The engine the task's jobs run on.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// What the frontend gives the task.
    pub fn actions(&self) -> &Arc<dyn TaskActions> {
        &self.actions
    }

    /// The task's title: "Scans".
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The task's state folder.
    pub fn store(&self) -> &TaskStore {
        &self.store
    }

    /// Why the task is paused, if it is.
    pub fn paused(&self) -> Option<&str> {
        self.paused.as_deref()
    }

    /// What the runner has done since it started.
    pub fn totals(&self) -> Totals {
        self.totals
    }

    /// Whether nothing waits: nothing settling, queued or to be retried.
    pub fn is_idle(&self) -> bool {
        self.queue.is_empty()
            && self.overflow.is_empty()
            && self.retries.is_empty()
            && !self.tracker.is_settling()
    }

    /// Whether the runner stopped as a crash would, at a checkpoint, or
    /// after its job was cancelled.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// What the task's action resolves to on this computer, for the
    /// approval key: the recipe and its tool's version, the script's hash,
    /// the plugin's version. An error says why it cannot be found.
    pub fn resolve(&self) -> Result<Resolved, String> {
        match &self.task.action {
            TaskAction::Convert { recipe } => {
                let found = self
                    .actions
                    .recipe(recipe)
                    .ok_or_else(|| format!("no recipe is named {recipe}"))?;
                Ok(Resolved::Recipe {
                    id: found.id.clone(),
                    tool: found.tool.clone(),
                    args: found.args.clone(),
                    timeout: found.timeout.as_secs(),
                    version: self.actions.tool_version(&found.tool),
                })
            }
            TaskAction::Script { id } => self.actions.resolve_script(id),
            TaskAction::Plugin { id } => self.actions.resolve_plugin(id),
            TaskAction::Move | TaskAction::Copy | TaskAction::Tell => Ok(Resolved::Nothing),
        }
    }

    /// Where the task stands with the owner's approval, and the key a yes
    /// is recorded with ([`TaskStore::approve`]).
    pub fn approval(&self) -> Result<(Approval, String), StartError> {
        let resolved = self.resolve().map_err(|why| self.cannot(&why))?;
        let key = approval_key(&self.task, &resolved);
        Ok((self.store.state().approval(&key), key))
    }

    /// "Could not run Scans: CAUSE."
    fn cannot(&self, why: &str) -> StartError {
        StartError::Cannot(format!("Could not run {}: {why}.", self.title))
    }

    fn provider(&self, place: &Place) -> Result<Arc<dyn Provider>, VfsError> {
        self.engine.registry().resolve(place)
    }

    fn stat(&self, place: &Place) -> Result<Entry, VfsError> {
        self.provider(place)?.stat(place, &OpContext::new())
    }

    fn list_in(&self) -> Result<Vec<Entry>, VfsError> {
        let input = &self.task.folders.input;
        self.provider(input)?.list(input, &OpContext::new())
    }

    fn job_id(&self) -> JobId {
        JobId::new(self.next_job.fetch_add(1, Ordering::Relaxed))
    }

    fn origin(&self) -> JobOrigin {
        JobOrigin::HotFolder(self.task.name.clone())
    }

    /// Writes a log line now; a log that cannot be written never stops a
    /// run.
    fn log(&self, outcome: &str, fields: &[&str]) {
        let line = log_line(outcome, fields, self.clock.now(), self.clock.offset());
        let _ = self.log.write(&line);
    }

    fn ledger_line(&self, line: Line) -> Result<(), String> {
        guard(&self.ledger)
            .append(line)
            .map_err(|e| format!("its ledger could not be written: {e}"))
    }

    /// The files in `in` that a start would catch up on: settled-looking
    /// files that match and that the ledger has not seen.
    pub fn waiting(&self) -> Result<Vec<String>, VfsError> {
        let ledger = guard(&self.ledger);
        let mut names: Vec<String> = self
            .list_in()?
            .into_iter()
            .filter(|e| !is_ignored(&e.name))
            .filter(|e| self.facts(e).skip_reason().is_none())
            .filter(|e| self.task.matches_name(&e.name))
            .filter(|e| ledger.handled(&e.name, &Identity::of(e)).is_none())
            .map(|e| e.name)
            .collect();
        names.sort_by(|a, b| {
            diroxide_core::sort::compare_names(a, b, diroxide_core::sort::NameCase::Ignore, true)
        });
        Ok(names)
    }

    /// An entry's facts, an online-only file counted as a file when the
    /// task fetches.
    fn facts(&self, entry: &Entry) -> Facts {
        let mut facts = Facts::from_entry(entry);
        if self.task.online_only == OnlineOnly::Fetch {
            facts.online_only = false;
        }
        facts
    }

    /// Starts the task: its lock, its approval, a crash's made lines
    /// finished, and the catch-up the owner chose (or the task's file
    /// says). See [`StartError`] for what stops it.
    pub fn start(&mut self, now: Timestamp) -> Result<Vec<HotNote>, StartError> {
        self.start_with(now, None)
    }

    /// [`TaskRunner::start`] with the catch-up given: `--once` catches up
    /// on everything.
    pub fn start_with(
        &mut self,
        now: Timestamp,
        catch_up: Option<CatchUpChoice>,
    ) -> Result<Vec<HotNote>, StartError> {
        let lock = self
            .store
            .lock()
            .map_err(|e| self.cannot(&e.to_string()))?
            .ok_or_else(|| StartError::Locked(locked_line(&self.title)))?;
        let resolved = self.resolve().map_err(|why| self.cannot(&why))?;
        let key = approval_key(&self.task, &resolved);
        let state = self.store.state();
        let approval = state.approval(&key);
        if let Some(question) = approval.question(&self.task) {
            return Err(StartError::NotApproved {
                approval,
                question,
                key,
            });
        }
        if let TaskAction::Convert { recipe } = &self.task.action {
            let mut found = self
                .actions
                .recipe(recipe)
                .ok_or_else(|| self.cannot(&format!("no recipe is named {recipe}")))?;
            if let Some(limit) = self.task.timeout {
                found.timeout = found.timeout.min(limit);
            }
            self.recipe = Some(found);
        }
        let mut notes = Vec::new();
        let listing = match self.list_in() {
            Ok(listing) => Some(listing),
            Err(VfsError::NotFound | VfsError::NotAFolder) => None,
            Err(error) => return Err(self.cannot(&format!("in: {error}"))),
        };
        let choice = catch_up
            .or_else(|| state.catch_up())
            .or(match self.task.catch_up {
                CatchUpChoice::Ask => None,
                other => Some(other),
            });
        let waiting = if listing.is_some() {
            self.waiting().map(|w| w.len()).unwrap_or(0)
        } else {
            0
        };
        let choice = match choice {
            Some(choice) => choice,
            None if waiting == 0 => CatchUpChoice::All,
            None => {
                return Err(StartError::AskCatchUp {
                    waiting,
                    question: catch_up_question(&self.title, waiting),
                    yes_first: waiting <= CATCH_UP_YES_UP_TO,
                });
            }
        };
        self.lock = Some(lock);
        self.paused = None;
        let _ = self.store.set_paused(None);
        self.log("started", &[&sentence(&self.task)]);
        let Some(listing) = listing else {
            self.missing = true;
            self.poll_at = Some(later_by(now, self.options.poll_every));
            notes.extend(self.pause(FOLDER_MISSING, now));
            return Ok(notes);
        };
        // A crash's made lines: the outputs are there; move the originals.
        let unfinished: Vec<Line> = guard(&self.ledger)
            .unfinished()
            .into_iter()
            .cloned()
            .collect();
        for line in unfinished {
            let ran = self.finish(&line);
            notes.extend(self.apply(ran, now));
        }
        // Folders the task writes into must be there.
        for (label, folder) in self.needs_present() {
            if matches!(
                self.stat(&folder),
                Err(VfsError::NotFound | VfsError::NotAFolder)
            ) {
                notes.extend(self.pause(&format!("{label} folder is missing"), now));
                return Ok(notes);
            }
        }
        if let Some(reason) = self.tool_missing() {
            notes.extend(self.pause(&reason, now));
        }
        // Partial outputs a crash left: listed, never removed.
        if self.task.action.makes_output()
            && let Ok(entries) = self
                .provider(&self.task.folders.out)
                .and_then(|p| p.list(&self.task.folders.out, &OpContext::new()))
        {
            let partial = entries
                .iter()
                .filter(|e| is_staging_name(&e.name) || is_unfinished(&e.name))
                .count();
            if partial > 0 {
                let line = partial_line(partial, "out");
                self.log("found", &[&line]);
                notes.extend(self.say(line));
            }
        }
        let entries: Vec<(String, Facts)> = listing
            .iter()
            .map(|e| (e.name.clone(), self.facts(e)))
            .collect();
        match choice {
            CatchUpChoice::None => self.tracker.baseline(entries, now),
            _ => {
                let mut events = Vec::new();
                for (name, facts) in entries {
                    events.extend(self.tracker.observe(&name, Look::Present(facts), now));
                }
                notes.extend(self.tracked(events, now));
            }
        }
        let caught = if choice == CatchUpChoice::None {
            0
        } else {
            waiting
        };
        notes.push(HotNote::Started {
            waiting: caught,
            polled: self.poll_every,
        });
        if let Some(every) = self.poll_every {
            self.poll_at = Some(later_by(now, every));
            notes.extend(self.say(polled_line(&self.title, every)));
        }
        if caught > 0 {
            notes.extend(self.say(caught_up_line(&self.title, caught)));
        }
        self.schedule_tick(now);
        notes.extend(self.status_note());
        Ok(notes)
    }

    /// The folders a run writes into, beside `in`, that must exist.
    fn needs_present(&self) -> Vec<(String, Place)> {
        let mut out: Vec<(String, Place)> = self
            .task
            .needs_folders()
            .into_iter()
            .filter(|(label, _)| *label != "in")
            .map(|(label, place)| (label.to_owned(), place.clone()))
            .collect();
        if let Some(to) = &self.task.folders.to {
            out.push(("to".to_owned(), to.clone()));
        }
        if let After::Folder(folder) = &self.task.after
            && self.task.action != TaskAction::Tell
        {
            out.push(("after".to_owned(), folder.clone()));
        }
        out
    }

    /// Why the task's tool cannot run, if it cannot: "pdftotext is not
    /// installed".
    fn tool_missing(&self) -> Option<String> {
        let recipe = self.recipe.as_ref()?;
        if recipe.tool == "carta" || self.engine.tools().has(&recipe.tool) {
            None
        } else {
            Some(format!(
                "{} is not installed",
                self.engine.tools().name_of(&recipe.tool)
            ))
        }
    }

    /// Makes the folders the task needs that are missing (its home, then
    /// `in`, `out`, `processed` and `failed` as it uses them), each with
    /// the ordinary make-folder job, and returns their names as said.
    /// The owner is told first: "Will make in, out, processed and failed
    /// in D:\scans".
    pub fn make_folders(&self) -> Result<Vec<String>, String> {
        let mut made = Vec::new();
        let mut wanted: Vec<(String, Place)> =
            vec![("home".to_owned(), self.task.folders.home.clone())];
        wanted.extend(
            self.task
                .needs_folders()
                .into_iter()
                .map(|(label, place)| (label.to_owned(), place.clone())),
        );
        for (label, folder) in wanted {
            match self.stat(&folder) {
                Ok(entry) if entry.kind == EntryKind::Folder => continue,
                Ok(_) => return Err(format!("{label} is not a folder")),
                Err(VfsError::NotFound) => {}
                Err(error) => return Err(format!("{label}: {error}")),
            }
            let (Some(parent), Some(name)) = (folder.parent(), folder.name()) else {
                return Err(format!("{label} has no folder above it"));
            };
            let mut request = JobRequest::new(self.job_id(), JobKind::MakeFolder, Vec::new());
            request.destination = Some(parent);
            request.new_name = Some(name.to_owned());
            request.origin = self.origin();
            request.errors = ErrorPolicy::ContinueAndList;
            let control = JobControl::new();
            let plan = self
                .engine
                .plan_with(&request, &control, &Quiet)
                .map_err(|e| format!("{label}: {e}"))?;
            let report = self.engine.run(&plan, &control, &Quiet);
            if let Some((_, cause)) = report.failed.first() {
                return Err(format!("{label}: {cause}"));
            }
            made.push(label);
        }
        Ok(made)
    }

    /// Takes a watch event for the `in` folder: the name it names is
    /// looked at; a rescan, or an event of a kind this build does not
    /// know, lists the folder; the folder gone pauses the task until it
    /// comes back.
    pub fn event(&mut self, event: &WatchEvent, now: Timestamp) -> Vec<HotNote> {
        if self.stopped {
            return Vec::new();
        }
        let renamed = matches!(event, WatchEvent::RenamedIn(_));
        let events = match event {
            WatchEvent::Created(place)
            | WatchEvent::Changed(place)
            | WatchEvent::Removed(place)
            | WatchEvent::RenamedIn(place) => {
                let Some(name) = place.name().map(str::to_owned) else {
                    return Vec::new();
                };
                if is_ignored(&name) {
                    return Vec::new();
                }
                let look = self.look(&name, false);
                if renamed {
                    self.tracker.renamed_in(&name, look, now)
                } else {
                    self.tracker.observe(&name, look, now)
                }
            }
            WatchEvent::Gone => return self.folder_gone(now),
            _ => return self.rescan(now),
        };
        let mut notes = self.tracked(events, now);
        self.schedule_tick(now);
        notes.extend(self.status_note());
        notes
    }

    /// Lists `in` and compares, as after an overflow or on a poll.
    pub fn rescan(&mut self, now: Timestamp) -> Vec<HotNote> {
        match self.list_in() {
            Ok(entries) => {
                let listing: Vec<(String, Facts)> = entries
                    .iter()
                    .map(|e| (e.name.clone(), self.facts(e)))
                    .collect();
                let events = self.tracker.reconcile(listing, now);
                let mut notes = self.tracked(events, now);
                self.schedule_tick(now);
                notes.extend(self.status_note());
                notes
            }
            Err(VfsError::NotFound | VfsError::NotAFolder) => self.folder_gone(now),
            Err(_) => Vec::new(),
        }
    }

    fn folder_gone(&mut self, now: Timestamp) -> Vec<HotNote> {
        if self.missing {
            return Vec::new();
        }
        self.missing = true;
        self.poll_at = Some(later_by(now, self.options.poll_every));
        self.pause(FOLDER_MISSING, now)
    }

    /// The time-driven work, when due: a poll, the tick over the settling
    /// files, retries whose time has come, one batch of the queue (as many
    /// files as the task's `parallel`), and the summary.
    pub fn wake(&mut self, now: Timestamp) -> Vec<HotNote> {
        let mut notes = Vec::new();
        if self.stopped {
            return notes;
        }
        if self.poll_at.is_some_and(|at| at <= now) {
            self.poll_at = None;
            if self.missing {
                if self.list_in().is_ok() {
                    self.missing = false;
                    if self.paused.as_deref() == Some(FOLDER_MISSING) {
                        notes.extend(self.resume(now));
                    } else {
                        notes.extend(self.rescan(now));
                    }
                } else {
                    self.poll_at = Some(later_by(now, self.options.poll_every));
                }
            } else {
                notes.extend(self.rescan(now));
            }
            if let Some(every) = self.poll_every
                && self.poll_at.is_none()
            {
                self.poll_at = Some(later_by(now, every));
            }
        }
        if self.tick_at.is_some_and(|at| at <= now) {
            self.tick_at = None;
            let provider = self.provider(&self.task.folders.input).ok();
            let folder = self.task.folders.input.clone();
            let fetch = self.task.online_only == OnlineOnly::Fetch;
            let mut look = |name: &str, due: bool| match &provider {
                Some(provider) => look_at(&**provider, &folder, name, due, fetch),
                None => Look::Unreadable,
            };
            let events = self.tracker.tick(now, &mut look);
            notes.extend(self.tracked(events, now));
            self.schedule_tick(now);
        }
        let (due, later): (Vec<Waiting>, Vec<Waiting>) =
            self.retries.drain(..).partition(|r| r.at <= now);
        self.retries = later;
        for retry in due {
            self.queue.push_back(Queued {
                name: retry.name,
                attempt: retry.attempt,
            });
        }
        notes.extend(self.process(now));
        if let Some(at) = self.summary_at
            && at <= now
        {
            self.summary_at = None;
            notes.extend(self.summary());
        }
        notes.extend(self.status_note());
        notes
    }

    /// How long until [`TaskRunner::wake`] has work; zero while files wait
    /// to run; `None` when only an event can bring any.
    pub fn next_wake(&self, now: Timestamp) -> Option<Duration> {
        if self.stopped {
            return None;
        }
        if self.paused.is_none() && !self.queue.is_empty() {
            return Some(Duration::ZERO);
        }
        let retry = if self.paused.is_none() {
            self.retries.iter().map(|r| r.at).min()
        } else {
            None
        };
        [self.tick_at, self.poll_at, retry, self.summary_at]
            .into_iter()
            .flatten()
            .map(|at| between(now, at))
            .min()
    }

    /// Pauses the task with this reason: every file stays where it is,
    /// and it says so once.
    pub fn pause(&mut self, reason: &str, now: Timestamp) -> Vec<HotNote> {
        let _ = now;
        if self.paused.as_deref() == Some(reason) {
            return Vec::new();
        }
        self.paused = Some(reason.to_owned());
        let _ = self.store.set_paused(Some(reason));
        let line = paused_line(&self.title, reason);
        self.log("paused", &[reason]);
        let mut notes = vec![HotNote::Paused(line.clone())];
        notes.extend(self.say(line));
        notes
    }

    /// Goes on after a pause: the breaker's count starts again, the tool
    /// is checked again, and `in` is listed so nothing that came meanwhile
    /// is missed.
    pub fn resume(&mut self, now: Timestamp) -> Vec<HotNote> {
        if self.paused.is_none() {
            return Vec::new();
        }
        self.paused = None;
        let _ = self.store.set_paused(None);
        self.breaker.succeeded();
        self.log("resumed", &[]);
        let mut notes = vec![HotNote::Resumed];
        if let Some(reason) = self.tool_missing() {
            notes.extend(self.pause(&reason, now));
            return notes;
        }
        notes.extend(self.rescan(now));
        notes
    }

    /// Stops the jobs running now, as Quit does: their files stay in `in`.
    pub fn interrupt(&self) {
        self.interrupter().interrupt();
    }

    /// A handle that stops this runner's running jobs from another thread,
    /// without waiting for the runner.
    pub fn interrupter(&self) -> Interrupter {
        Interrupter(Arc::clone(&self.running))
    }

    fn schedule_tick(&mut self, now: Timestamp) {
        if self.tracker.is_settling() {
            if self.tick_at.is_none() {
                self.tick_at = Some(later_by(now, self.tracker.rules().tick));
            }
        } else {
            self.tick_at = None;
        }
    }

    fn look(&self, name: &str, probe: bool) -> Look {
        match self.provider(&self.task.folders.input) {
            Ok(provider) => look_at(
                &*provider,
                &self.task.folders.input,
                name,
                probe,
                self.task.online_only == OnlineOnly::Fetch,
            ),
            Err(_) => Look::Unreadable,
        }
    }

    /// The tracker's decisions: an arrival that matches joins the queue; a
    /// link or an online-only file is said; a file that never settles is
    /// given up on.
    fn tracked(&mut self, events: Vec<TrackerEvent>, now: Timestamp) -> Vec<HotNote> {
        let mut notes = Vec::new();
        for event in events {
            match event {
                TrackerEvent::Arrived(arrival) => notes.extend(self.arrived(arrival.name, now)),
                TrackerEvent::Skipped { name, reason }
                    if matches!(reason, SkipReason::Link | SkipReason::OnlineOnly) =>
                {
                    let why = reason.words();
                    self.log("skipped", &[&name, why]);
                    let line = skipped_line(&name, why);
                    notes.push(HotNote::File(FileNote {
                        name,
                        outcome: FileOutcome::Skipped,
                        line: line.clone(),
                        output: None,
                    }));
                    notes.extend(self.say(line));
                }
                TrackerEvent::GaveUp { name } => {
                    self.log("gave up", &[&name, "it kept changing"]);
                    let line = crate::tracker::gave_up_line(&name);
                    notes.push(HotNote::File(FileNote {
                        name,
                        outcome: FileOutcome::GaveUp,
                        line: line.clone(),
                        output: None,
                    }));
                    notes.extend(self.say(line));
                }
                TrackerEvent::Skipped { .. } | TrackerEvent::Gone { .. } => {}
            }
        }
        notes
    }

    /// A settled file: it joins the queue when it matches the patterns and
    /// the filter.
    fn arrived(&mut self, name: String, now: Timestamp) -> Vec<HotNote> {
        if !self.task.matches_name(&name) {
            return Vec::new();
        }
        if let Some(filter) = self.task.filter.clone() {
            let Ok(place) = self.task.folders.input.join(&name) else {
                return Vec::new();
            };
            let Ok(entry) = self.stat(&place) else {
                return Vec::new();
            };
            match self.actions.passes(&filter, &place, &entry) {
                Ok(true) => {}
                Ok(false) => return Vec::new(),
                Err(why) => {
                    self.queue.push_back(Queued {
                        attempt: self.attempts.get(&name).copied().unwrap_or(0),
                        name,
                    });
                    return self.pause(&format!("the filter cannot be read: {why}"), now);
                }
            }
        }
        if self.queue.iter().any(|q| q.name == name) || self.retries.iter().any(|r| r.name == name)
        {
            return Vec::new();
        }
        if self.queue.len() >= self.options.queue_cap {
            if !self.overflow.contains(&name) {
                self.overflow.push_back(name);
            }
            if !self.said_full {
                self.said_full = true;
                let line = full_line(&self.title, self.options.queue_cap);
                self.log("waiting", &[&line]);
                return self.say(line);
            }
            return Vec::new();
        }
        self.queue.push_back(Queued {
            attempt: self.attempts.get(&name).copied().unwrap_or(0),
            name,
        });
        Vec::new()
    }

    /// One batch of the queue.
    fn process(&mut self, now: Timestamp) -> Vec<HotNote> {
        let mut notes = Vec::new();
        if self.paused.is_some() || self.stopped {
            return notes;
        }
        while self.queue.len() < self.options.queue_cap
            && let Some(name) = self.overflow.pop_front()
        {
            self.queue.push_back(Queued {
                attempt: self.attempts.get(&name).copied().unwrap_or(0),
                name,
            });
        }
        if self.overflow.is_empty() {
            self.said_full = false;
        }
        let width = usize::try_from(self.task.parallel.max(1)).unwrap_or(1);
        let batch: Vec<Queued> = (0..width).filter_map(|_| self.queue.pop_front()).collect();
        if batch.is_empty() {
            return notes;
        }
        let results: Vec<Ran> = if batch.len() == 1 {
            vec![self.run_file(&batch[0])]
        } else {
            let this = &*self;
            std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|item| scope.spawn(move || this.run_file(item)))
                    .collect();
                handles
                    .into_iter()
                    .zip(&batch)
                    .map(|(handle, item)| {
                        handle.join().unwrap_or_else(|_| Ran::Pause {
                            name: Some(item.name.clone()),
                            attempt: item.attempt,
                            reason: "a run stopped unexpectedly".to_owned(),
                        })
                    })
                    .collect()
            })
        };
        for ran in results {
            let name = ran.name().map(str::to_owned);
            notes.extend(self.apply(ran, now));
            // A file that left `in` is forgotten, so the same file put
            // back later is a new arrival; one that stays keeps its record.
            if let Some(name) = name {
                self.look_again(&name, now);
            }
        }
        notes
    }

    /// Tells the tracker what is under a name now: gone, or the same.
    fn look_again(&mut self, name: &str, now: Timestamp) {
        if let Look::Absent = self.look(name, false) {
            let _ = self.tracker.observe(name, Look::Absent, now);
        }
    }

    /// Applies one file's outcome to the queue, the counts and the
    /// breaker, and says it.
    fn apply(&mut self, ran: Ran, now: Timestamp) -> Vec<HotNote> {
        let mut notes = Vec::new();
        match ran {
            Ran::Done {
                name,
                output,
                took,
                across,
                finished,
            } => {
                self.attempts.remove(&name);
                self.breaker.succeeded();
                self.totals.done += 1;
                let line = if finished {
                    format!("Done: {}, finished after a restart", spell_out(&name))
                } else {
                    done_line(&name, took)
                };
                if across {
                    notes.extend(self.say_quietly(across_line(&name)));
                }
                notes.push(HotNote::File(FileNote {
                    name,
                    outcome: FileOutcome::Done,
                    line: line.clone(),
                    output,
                }));
                notes.extend(self.counted(FileOutcome::Done, line, now));
            }
            Ran::Skipped { name, why, quiet } => {
                self.attempts.remove(&name);
                if !quiet {
                    self.totals.skipped += 1;
                    let line = skipped_line(&name, &why);
                    notes.push(HotNote::File(FileNote {
                        name,
                        outcome: FileOutcome::Skipped,
                        line: line.clone(),
                        output: None,
                    }));
                    notes.extend(self.counted(FileOutcome::Skipped, line, now));
                }
            }
            Ran::Told { name, size } => {
                self.totals.done += 1;
                let line = told_line(&name, size);
                notes.push(HotNote::File(FileNote {
                    name,
                    outcome: FileOutcome::Told,
                    line: line.clone(),
                    output: None,
                }));
                notes.extend(self.counted(FileOutcome::Told, line, now));
            }
            Ran::Again {
                name,
                cause,
                attempt,
            } => {
                let wait = wait_before(attempt, &self.options.retry_waits);
                self.attempts.insert(name.clone(), attempt);
                self.retries.push(Waiting {
                    name: name.clone(),
                    at: later_by(now, wait),
                    attempt,
                });
                self.log(
                    "retrying",
                    &[&name, &cause, &format!("in {}", duration_words(wait))],
                );
                notes.push(HotNote::File(FileNote {
                    line: retry_line(&name, wait),
                    name,
                    outcome: FileOutcome::Retrying,
                    output: None,
                }));
            }
            Ran::Failed { name, cause } => {
                self.attempts.remove(&name);
                self.totals.failed += 1;
                let line = failed_line(&name, &cause);
                let tripped = self.breaker.failed(&cause_key(&cause, &name));
                notes.push(HotNote::File(FileNote {
                    name,
                    outcome: FileOutcome::Failed,
                    line: line.clone(),
                    output: None,
                }));
                notes.extend(self.counted(FileOutcome::Failed, line, now));
                if tripped {
                    notes.extend(self.pause(&breaker_reason(self.options.breaker), now));
                }
            }
            Ran::Left { name } => {
                self.attempts.remove(&name);
                self.totals.left += 1;
                self.log("left", &[&name, "for the next run"]);
                notes.push(HotNote::File(FileNote {
                    line: left_line(&name),
                    name,
                    outcome: FileOutcome::Left,
                    output: None,
                }));
            }
            Ran::Changed { name, attempt } => {
                self.attempts.insert(name.clone(), attempt);
                self.log("changed", &[&name, "during the run; it runs again"]);
                // Seen anew, so it settles again even when only the
                // writer probe saw the change.
                let look = self.look(&name, false);
                let mut events = self.tracker.observe(&name, Look::Absent, now);
                events.extend(self.tracker.observe(&name, look, now));
                notes.extend(self.tracked(events, now));
                self.schedule_tick(now);
                notes.push(HotNote::File(FileNote {
                    line: changed_line(&name),
                    name,
                    outcome: FileOutcome::Changed,
                    output: None,
                }));
            }
            Ran::Pause {
                name,
                attempt,
                reason,
            } => {
                if let Some(name) = name {
                    self.queue.push_front(Queued { name, attempt });
                }
                notes.extend(self.pause(&reason, now));
            }
            Ran::Gone { name } => {
                self.attempts.remove(&name);
            }
            Ran::Stopped => {
                self.stopped = true;
            }
        }
        notes
    }

    /// Counts an outcome for the summary and says what `announce` wants.
    fn counted(&mut self, outcome: FileOutcome, line: String, now: Timestamp) -> Vec<HotNote> {
        let failure = outcome == FileOutcome::Failed;
        let add = |tally: &mut Tally| match outcome {
            FileOutcome::Failed => tally.failed += 1,
            FileOutcome::Skipped => tally.skipped += 1,
            _ => tally.done += 1,
        };
        match self.task.announce {
            Announce::Off => Vec::new(),
            Announce::Failures if !failure => Vec::new(),
            Announce::Each
                if self
                    .last_each
                    .is_none_or(|last| between(last, now) >= self.options.each_gap) =>
            {
                self.last_each = Some(now);
                vec![HotNote::Say(line)]
            }
            _ => {
                add(&mut self.tally);
                if self.summary_at.is_none() {
                    self.summary_at = Some(later_by(now, self.options.pace));
                }
                Vec::new()
            }
        }
    }

    /// The summary of everything since the last: "Scans: 3 done, 1
    /// failed".
    fn summary(&mut self) -> Vec<HotNote> {
        let tally = std::mem::take(&mut self.tally);
        if tally.is_empty() {
            return Vec::new();
        }
        let line = if self.task.action == TaskAction::Tell {
            format!("{}: {} arrived", self.title, tally.done)
        } else {
            summary_line(&self.title, tally.done, tally.failed, tally.skipped)
        };
        vec![HotNote::Say(line)]
    }

    /// A line to say, unless the task says nothing.
    fn say(&self, line: String) -> Vec<HotNote> {
        if self.task.announce == Announce::Off {
            Vec::new()
        } else {
            vec![HotNote::Say(line)]
        }
    }

    /// A line said only when the task says each file.
    fn say_quietly(&self, line: String) -> Vec<HotNote> {
        if self.task.announce == Announce::Each {
            vec![HotNote::Say(line)]
        } else {
            Vec::new()
        }
    }

    /// The status line, when it changed.
    fn status_note(&mut self) -> Option<HotNote> {
        let line = match &self.paused {
            Some(reason) => Some(paused_line(&self.title, reason)),
            None => status_line(
                &self.title,
                self.queue.len() + self.overflow.len() + self.tracker.waiting().len(),
                self.retries.len(),
            ),
        };
        if line == self.status {
            return None;
        }
        self.status = line.clone();
        Some(HotNote::Status(line))
    }

    /// Calls the checkpoint's observer; true when it says stop.
    fn stop_at(&self, at: Checkpoint, name: &str) -> bool {
        self.checkpoint
            .as_ref()
            .is_some_and(|observer| observer(at, name).is_break())
    }

    /// One file's whole run. Takes only shared access, so a batch runs on
    /// several threads; its outcome is applied after.
    fn run_file(&self, item: &Queued) -> Ran {
        let name = item.name.clone();
        let Ok(place) = self.task.folders.input.join(&name) else {
            return self.final_failure(&name, None, "its name cannot be used here");
        };
        let entry = match self.stat(&place) {
            Ok(entry) => entry,
            Err(VfsError::NotFound) => return Ran::Gone { name },
            Err(error) => return self.failure(item, None, &error.to_string()),
        };
        if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
            return Ran::Skipped {
                why: Facts::from_entry(&entry)
                    .skip_reason()
                    .map_or("not a file", SkipReason::words)
                    .to_owned(),
                name,
                quiet: false,
            };
        }
        let identity = Identity::of(&entry);
        let handled = guard(&self.ledger).handled(&name, &identity).cloned();
        if let Some(line) = handled {
            if line.outcome == Outcome::Made {
                return self.finish(&line);
            }
            return Ran::Skipped {
                name,
                why: "already handled".to_owned(),
                quiet: true,
            };
        }
        if let Some(reason) = self.short_of_space() {
            return Ran::Pause {
                name: Some(name),
                attempt: item.attempt,
                reason,
            };
        }
        let started = self.clock.now();
        let control = JobControl::new();
        let ticket = self.job_id();
        guard(&self.running).push((ticket, control.clone()));
        let act = self.act(&place, &control);
        guard(&self.running).retain(|(id, _)| *id != ticket);
        let outputs = match act {
            Act::Cancelled => {
                return Ran::Stopped;
            }
            Act::Pause(reason) => {
                return Ran::Pause {
                    name: Some(name),
                    attempt: item.attempt,
                    reason,
                };
            }
            Act::Fail(cause) => return self.failure(item, Some(&entry), &cause),
            Act::Again(cause) => {
                return self.failure_as(item, Some(&entry), &cause, CauseClass::Again);
            }
            Act::Skipped(why) => {
                // Nothing to make: the original is handled as done.
                return self.after_made(item, &entry, None, started, Some(why));
            }
            Act::Moved(to) => {
                // A move task's action was the move.
                let line = Line {
                    outcome: Outcome::Done,
                    name: name.clone(),
                    identity,
                    at: self.clock.now(),
                    output: to.as_ref().map(ToString::to_string),
                };
                if let Err(why) = self.ledger_line(line) {
                    return self.pause_on(item, why);
                }
                let took = between(started, self.clock.now());
                let target = to.as_ref().map(ToString::to_string).unwrap_or_default();
                self.log("done", &[&name, "moved", &took_words(took), &target]);
                return Ran::Done {
                    name,
                    output: to,
                    took,
                    across: false,
                    finished: false,
                };
            }
            Act::Made(outputs) => outputs,
        };
        if self.task.action == TaskAction::Tell {
            let line = Line {
                outcome: Outcome::Told,
                name: name.clone(),
                identity,
                at: self.clock.now(),
                output: None,
            };
            if let Err(why) = self.ledger_line(line) {
                return self.pause_on(item, why);
            }
            let size = entry.size;
            self.log(
                "told",
                &[
                    &name,
                    &size
                        .map(diroxide_core::format::human_size)
                        .unwrap_or_default(),
                ],
            );
            return Ran::Told { name, size };
        }
        let output = outputs.into_iter().next();
        self.after_made(item, &entry, output, started, None)
    }

    /// The made line, the look again, the original's move and the done
    /// line, for an action that made `output` (or nothing, `skipped`
    /// saying why).
    fn after_made(
        &self,
        item: &Queued,
        entry: &Entry,
        output: Option<Place>,
        started: Timestamp,
        skipped: Option<String>,
    ) -> Ran {
        let name = item.name.clone();
        let Ok(place) = self.task.folders.input.join(&name) else {
            return Ran::Gone { name };
        };
        let place = &place;
        let identity = &Identity::of(entry);
        let made = Line {
            outcome: Outcome::Made,
            name: name.clone(),
            identity: *identity,
            at: self.clock.now(),
            output: output.as_ref().map(ToString::to_string),
        };
        if let Err(why) = self.ledger_line(made) {
            return self.pause_on(item, why);
        }
        if self.stop_at(Checkpoint::Committed, &name) {
            return Ran::Stopped;
        }
        // Look again before the original moves.
        let again = self.stat(place);
        let changed = match &again {
            Ok(now) => !Identity::of(now).same(identity),
            Err(_) => false,
        };
        let written = place
            .local_path()
            .and_then(|p| still_written(std::path::Path::new(p)))
            == Some(true);
        if changed || written {
            if item.attempt < self.task.retries && !self.once {
                return Ran::Changed {
                    name,
                    attempt: item.attempt + 1,
                };
            }
            if self.once && item.attempt < self.task.retries {
                return Ran::Left { name };
            }
            return self.final_failure(&name, Some(entry), "it kept changing during the run");
        }
        if matches!(again, Err(VfsError::NotFound)) {
            // Gone while it ran: nothing to move; the output stays.
            let line = Line {
                outcome: Outcome::Done,
                ..self.done_line_for(&name, identity, output.as_ref())
            };
            if let Err(why) = self.ledger_line(line) {
                return self.pause_on(item, why);
            }
            return Ran::Done {
                name,
                output,
                took: between(started, self.clock.now()),
                across: false,
                finished: false,
            };
        }
        let across = match self.move_original(place, entry, false) {
            Ok((_, across)) => across,
            Err(reason) => return self.pause_on(item, reason),
        };
        if self.stop_at(Checkpoint::Moved, &name) {
            return Ran::Stopped;
        }
        let outcome = if skipped.is_some() {
            Outcome::Skipped
        } else {
            Outcome::Done
        };
        let line = Line {
            outcome,
            ..self.done_line_for(&name, identity, output.as_ref())
        };
        if let Err(why) = self.ledger_line(line) {
            return self.pause_on(item, why);
        }
        let took = between(started, self.clock.now());
        let tool = match &self.task.action {
            TaskAction::Convert { recipe } => recipe.clone(),
            TaskAction::Script { id } | TaskAction::Plugin { id } => id.clone(),
            other => other.word().to_owned(),
        };
        let shown = output.as_ref().map(|o| self.shown(o)).unwrap_or_default();
        let note = if across { "moved across drives" } else { "" };
        if let Some(why) = skipped {
            self.log("skipped", &[&name, &why, note]);
            return Ran::Skipped {
                name,
                why,
                quiet: false,
            };
        }
        self.log("done", &[&name, &tool, &took_words(took), &shown, note]);
        Ran::Done {
            name,
            output,
            took,
            across,
            finished: false,
        }
    }

    fn done_line_for(&self, name: &str, identity: &Identity, output: Option<&Place>) -> Line {
        Line {
            outcome: Outcome::Done,
            name: name.to_owned(),
            identity: *identity,
            at: self.clock.now(),
            output: output.map(ToString::to_string),
        }
    }

    /// A place as the log shows it: inside the task's home, from there
    /// (`out\scan-004.txt`).
    fn shown(&self, place: &Place) -> String {
        let home = self.task.folders.home.to_string();
        let full = place.to_string();
        full.strip_prefix(&home)
            .map(|rest| rest.trim_start_matches(['\\', '/']).to_owned())
            .filter(|rest| !rest.is_empty())
            .unwrap_or(full)
    }

    /// Finishes a file whose output a crash left committed: the original
    /// moves (nothing is converted again), then the done line.
    fn finish(&self, line: &Line) -> Ran {
        let name = line.name.clone();
        let item = Queued {
            name: name.clone(),
            attempt: 0,
        };
        let output = line.output.as_deref().and_then(|o| Place::parse(o).ok());
        let Ok(place) = self.task.folders.input.join(&name) else {
            return Ran::Gone { name };
        };
        match self.stat(&place) {
            Ok(entry) if Identity::of(&entry).same(&line.identity) => {
                let across = match self.move_original(&place, &entry, false) {
                    Ok((_, across)) => across,
                    Err(reason) => return self.pause_on(&item, reason),
                };
                let done = Line {
                    outcome: Outcome::Done,
                    ..self.done_line_for(&name, &line.identity, output.as_ref())
                };
                if let Err(why) = self.ledger_line(done) {
                    return self.pause_on(&item, why);
                }
                self.log("done", &[&name, "finished after a restart"]);
                Ran::Done {
                    name,
                    output,
                    took: Duration::ZERO,
                    across,
                    finished: true,
                }
            }
            Ok(_) => Ran::Gone { name },
            Err(VfsError::NotFound) => {
                // It moved before the crash; the done line was not
                // written. Write it.
                let done = Line {
                    outcome: Outcome::Done,
                    ..self.done_line_for(&name, &line.identity, output.as_ref())
                };
                let _ = self.ledger_line(done);
                Ran::Gone { name }
            }
            Err(error) => self.pause_on(&item, format!("in: {error}")),
        }
    }

    fn pause_on(&self, item: &Queued, reason: String) -> Ran {
        Ran::Pause {
            name: Some(item.name.clone()),
            attempt: item.attempt,
            reason,
        }
    }

    /// A failure by its cause: tried again, filed, or the task paused.
    fn failure(&self, item: &Queued, entry: Option<&Entry>, cause: &str) -> Ran {
        self.failure_as(item, entry, cause, classify(cause))
    }

    /// A failure of a class already known.
    fn failure_as(
        &self,
        item: &Queued,
        entry: Option<&Entry>,
        cause: &str,
        class: CauseClass,
    ) -> Ran {
        match class {
            CauseClass::Task => Ran::Pause {
                name: Some(item.name.clone()),
                attempt: item.attempt,
                reason: cause.to_owned(),
            },
            CauseClass::Again if item.attempt < self.task.retries => {
                if self.once {
                    Ran::Left {
                        name: item.name.clone(),
                    }
                } else {
                    Ran::Again {
                        name: item.name.clone(),
                        cause: cause.to_owned(),
                        attempt: item.attempt + 1,
                    }
                }
            }
            _ => self.final_failure(&item.name, entry, cause),
        }
    }

    /// A file that failed for good: the original goes to `failed` (left
    /// in place in `leave` mode), never over a file, with its reason.
    fn final_failure(&self, name: &str, entry: Option<&Entry>, cause: &str) -> Ran {
        let item = Queued {
            name: name.to_owned(),
            attempt: 0,
        };
        let Some(entry) = entry else {
            self.log("failed", &[name, cause]);
            return Ran::Failed {
                name: name.to_owned(),
                cause: cause.to_owned(),
            };
        };
        if let Ok(place) = self.task.folders.input.join(name)
            && self.task.after != After::Leave
            && self.task.action != TaskAction::Tell
            && let Err(reason) = self.move_original(&place, entry, true)
        {
            return self.pause_on(&item, reason);
        }
        let line = Line {
            outcome: Outcome::Failed,
            name: name.to_owned(),
            identity: Identity::of(entry),
            at: self.clock.now(),
            output: None,
        };
        if let Err(why) = self.ledger_line(line) {
            return self.pause_on(&item, why);
        }
        self.log("failed", &[name, cause]);
        Ran::Failed {
            name: name.to_owned(),
            cause: cause.to_owned(),
        }
    }

    /// Whether free space where results go is under the floor: the
    /// reason, "only 200 MB free", when it is.
    fn short_of_space(&self) -> Option<String> {
        let target = match &self.task.action {
            TaskAction::Move | TaskAction::Copy => self.task.folders.to.as_ref()?,
            TaskAction::Tell => return None,
            _ => &self.task.folders.out,
        };
        let space = self
            .provider(target)
            .and_then(|p| p.space(target, &OpContext::new()))
            .ok()?;
        let free = space.free_to_user?;
        let share = space
            .total
            .map_or(0, |total| total / 100 * self.options.space_percent);
        let floor = self.options.space_floor.max(share);
        (free < floor).then(|| space_reason(free))
    }

    /// Runs the task's action on one file.
    fn act(&self, place: &Place, control: &JobControl) -> Act {
        match &self.task.action {
            TaskAction::Tell => Act::Made(Vec::new()),
            TaskAction::Convert { .. } => self.convert(place, control),
            TaskAction::Move => {
                let Some(to) = &self.task.folders.to else {
                    return Act::Pause("it has no to folder".to_owned());
                };
                match self.transfer(JobKind::Move, place, to, self.task.conflicts, control) {
                    Act::Made(made) => Act::Moved(made.into_iter().next()),
                    other => other,
                }
            }
            TaskAction::Copy => {
                let Some(to) = &self.task.folders.to else {
                    return Act::Pause("it has no to folder".to_owned());
                };
                self.transfer(JobKind::Copy, place, to, self.task.conflicts, control)
            }
            TaskAction::Script { id } | TaskAction::Plugin { id } => {
                let run = ActionRun {
                    task: self.task.name.clone(),
                    command: id.clone(),
                    input: place.clone(),
                    out: self.task.folders.out.clone(),
                    timeout: self.task.timeout,
                };
                let result = if matches!(self.task.action, TaskAction::Script { .. }) {
                    self.actions.run_script(&run, &control.cancel)
                } else {
                    self.actions.run_plugin(&run, &control.cancel)
                };
                match result {
                    ActionResult::Made(made) => Act::Made(made),
                    ActionResult::Failed { cause, retry: true } => Act::Again(cause),
                    ActionResult::Failed {
                        cause,
                        retry: false,
                    } => Act::Fail(cause),
                    ActionResult::Pause(reason) => Act::Pause(reason),
                    ActionResult::Cancelled => Act::Cancelled,
                }
            }
        }
    }

    /// A conversion through the Convert engine, unattended.
    fn convert(&self, place: &Place, control: &JobControl) -> Act {
        let Some(recipe) = &self.recipe else {
            return Act::Pause("its recipe is not loaded".to_owned());
        };
        let mut request = ConvertRequest::new(
            self.job_id(),
            vec![place.clone()],
            self.task.folders.out.clone(),
            recipe.clone(),
        )
        .conflicts(self.task.conflicts)
        .on_errors(ErrorPolicy::ContinueAndList)
        .from(self.origin())
        .carta(self.carta.clone());
        if let Some(spool) = &self.spool {
            request = request.spool(spool.clone());
        }
        let plan = match self
            .engine
            .plan_convert(&request, self.actions.typing(), control)
        {
            Ok(plan) => plan,
            Err(ConvertError::Cancelled) => return Act::Cancelled,
            Err(
                error @ (ConvertError::ToolMissing { .. }
                | ConvertError::NeedsEncoder { .. }
                | ConvertError::Destination(_)),
            ) => return Act::Pause(error.to_string()),
            Err(error) => return Act::Fail(error.to_string()),
        };
        if let Some((_, why)) = plan.refused.first() {
            return Act::Fail(why.clone());
        }
        if plan.work.is_empty() {
            return match plan.skipped.first() {
                Some((_, why)) => Act::Skipped(why.clone()),
                None => Act::Fail(format!("the recipe {} does not take it", recipe.id)),
            };
        }
        let report = self.engine.run_convert(&plan, control, &Quiet);
        if report.job.cancelled {
            return Act::Cancelled;
        }
        if let Some((_, cause)) = report.job.failed.first() {
            return Act::Fail(cause.clone());
        }
        if !report.made.is_empty() {
            return Act::Made(report.made.into_iter().map(|(_, out)| out).collect());
        }
        match report.job.skipped.first() {
            Some((_, why)) => Act::Skipped(why.clone()),
            None => Act::Fail(format!("{} made no file", plan.tool_name)),
        }
    }

    /// A move or a copy as an ordinary job, unattended: `to` is the
    /// folder; nothing is replaced.
    fn transfer(
        &self,
        kind: JobKind,
        place: &Place,
        to: &Place,
        conflicts: ConflictPolicy,
        control: &JobControl,
    ) -> Act {
        let mut request = JobRequest::new(self.job_id(), kind, vec![place.clone()]);
        request.destination = Some(to.clone());
        request.conflicts = match conflicts {
            ConflictPolicy::Skip | ConflictPolicy::Newer => conflicts,
            _ => ConflictPolicy::KeepBoth,
        };
        request.origin = self.origin();
        request.errors = ErrorPolicy::ContinueAndList;
        let plan = match self.engine.plan_with(&request, control, &Quiet) {
            Ok(plan) => plan,
            Err(error) => {
                let cause = error.to_string();
                return if classify(&cause) == CauseClass::Task {
                    Act::Pause(cause)
                } else {
                    Act::Fail(cause)
                };
            }
        };
        let report = self.engine.run(&plan, control, &Quiet);
        if report.cancelled {
            return Act::Cancelled;
        }
        if let Some((_, cause)) = report.failed.first() {
            return Act::Fail(cause.clone());
        }
        if let Some((_, why)) = report.skipped.first() {
            return Act::Skipped(why.clone());
        }
        let target = report
            .undo
            .moves
            .last()
            .map(|(_, to)| to.clone())
            .or_else(|| report.undo.created.last().cloned())
            .or_else(|| report.done.last().cloned());
        Act::Made(target.into_iter().collect())
    }

    /// Moves the original to `processed` (or the task's folder), or to
    /// `failed` when `failed`, keeping both on a taken name; returns where
    /// it went and whether it crossed drives. An error is the pause's
    /// reason: the output is committed and the ledger has it, so a resume
    /// only moves the original.
    fn move_original(
        &self,
        place: &Place,
        entry: &Entry,
        failed: bool,
    ) -> Result<(Option<Place>, bool), String> {
        let target = if failed {
            self.task.folders.failed.clone()
        } else {
            match (&self.task.action, &self.task.after) {
                (TaskAction::Move | TaskAction::Tell, _) | (_, After::Leave) => {
                    return Ok((None, false));
                }
                (_, After::Move) => self.task.folders.processed.clone(),
                (_, After::Folder(folder)) => folder.clone(),
            }
        };
        let across = match (entry.volume, self.stat(&target).ok().and_then(|e| e.volume)) {
            (Some(from), Some(to)) => from != to,
            _ => false,
        };
        let control = JobControl::new();
        let label = if failed { "failed" } else { "processed" };
        match self.transfer(
            JobKind::Move,
            place,
            &target,
            ConflictPolicy::KeepBoth,
            &control,
        ) {
            Act::Made(made) => Ok((made.into_iter().next(), across)),
            Act::Moved(to) => Ok((to, across)),
            Act::Skipped(why) | Act::Fail(why) | Act::Again(why) | Act::Pause(why) => {
                Err(format!("the original could not be moved to {label}: {why}"))
            }
            Act::Cancelled => Err("the move was cancelled".to_owned()),
        }
    }

    /// "Put back": moves files from `failed` into `in`, keeping both on a
    /// taken name, and clears their ledger lines so they are new arrivals.
    /// Returns the lines to say, one a file.
    pub fn put_back(&mut self, names: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for name in names {
            let Ok(place) = self.task.folders.failed.join(name) else {
                continue;
            };
            let control = JobControl::new();
            match self.transfer(
                JobKind::Move,
                &place,
                &self.task.folders.input.clone(),
                ConflictPolicy::KeepBoth,
                &control,
            ) {
                Act::Made(made) => {
                    let back = made
                        .first()
                        .and_then(|p| p.name().map(str::to_owned))
                        .unwrap_or_else(|| name.clone());
                    for cleared in BTreeSet::from([name.clone(), back.clone()]) {
                        let _ = self.ledger_line(Line {
                            outcome: Outcome::Cleared,
                            name: cleared,
                            identity: Identity::default(),
                            at: self.clock.now(),
                            output: None,
                        });
                    }
                    self.log("put back", &[name]);
                    out.push(format!("Put back: {}", spell_out(name)));
                }
                Act::Skipped(why) | Act::Fail(why) | Act::Again(why) | Act::Pause(why) => {
                    out.push(format!("Not put back: {}, {why}", spell_out(name)));
                }
                Act::Moved(_) | Act::Cancelled => {}
            }
        }
        out
    }

    /// Compacts the ledger when it has passed its cap, keeping the lines
    /// of the files still in `in`.
    pub fn compact_ledger(&self) -> Result<(), String> {
        let mut ledger = guard(&self.ledger);
        if !ledger.is_over_cap() {
            return Ok(());
        }
        let present: BTreeSet<String> = self
            .list_in()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|e| e.name)
            .collect();
        ledger
            .compact(&present)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// One look at a name in a folder, with the writer probe when asked (a
/// local file, never an online-only one), and an online-only file counted
/// as a file when the task fetches.
fn look_at(provider: &dyn Provider, folder: &Place, name: &str, probe: bool, fetch: bool) -> Look {
    let Ok(place) = folder.join(name) else {
        return Look::Unreadable;
    };
    match provider.stat(&place, &OpContext::new()) {
        Ok(entry) => {
            let mut facts = Facts::from_entry(&entry);
            if fetch {
                facts.online_only = false;
            }
            if probe
                && facts.skip_reason().is_none()
                && let Some(path) = place.local_path()
            {
                facts.writer_open = writer_open(std::path::Path::new(path));
            }
            Look::Present(facts)
        }
        Err(VfsError::NotFound) => Look::Absent,
        Err(_) => Look::Unreadable,
    }
}
