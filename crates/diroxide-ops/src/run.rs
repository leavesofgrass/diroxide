//! The run: carries out a plan, step by step, in the fixed order create,
//! verify, then destroy (plan section 5.4; research/file-operations.md;
//! research/file-management-lessons.md, part 4).
//!
//! - Before each step the entry is read again and compared with what the
//!   plan saw; a step whose entry changed is skipped with its reason.
//! - A copy is written through the provider's committing writer in 1 MiB
//!   chunks, with pause, cancel and progress between chunks; a clone is
//!   tried first where the place has one, before the target is touched,
//!   and any failure of it falls back to the stream.
//! - A replace writes the new content first, sends the old file to the
//!   trash (where the place has one), and only then commits; a failure
//!   anywhere leaves the old file, or both, never neither.
//! - A move renames where it can; otherwise it copies, verifies (by hash,
//!   while copying), reads the source again, and only then removes it. A
//!   source is removed only when its own copy exists, was verified, and
//!   was made by this job; a folder only when it is empty at that moment.
//! - A trash call is checked after it returns; a trash that fails never
//!   falls through to a permanent delete.
//! - Every conflict is asked one at a time, the job paused while asking;
//!   every error is answered with Retry, Skip, Skip all like this or
//!   Cancel, a lost network retried on its own first.
//! - Every job that makes something ends with a reconciliation against a
//!   fresh read.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::catalog;
use diroxide_core::names::spell_out;
use diroxide_core::{
    ConflictAction, ConflictKind, ConflictPolicy, Entry, EntryKind, ErrorAction, JobKind, JobState,
    Place,
};
use diroxide_vfs::names::{NameRules, check_name_with, is_unfinished, unused_name};
use diroxide_vfs::{
    AttributeChange, CopyHint, ErrorClass, LinkKind, LinkRequest, OpContext, Provider, TrashCheck,
    VfsError, WriteMode,
};
use sha2::{Digest, Sha256};

use crate::engine::{Conflict, Engine, ItemError, JobControl, JobReport, JobSink, Reconciliation};
use crate::journal::JournalWriter;
use crate::plan::{Plan, Step, is_read_only};
use crate::progress::{Estimator, JobProgress};
use crate::undo::{LeftAs, UndoRecord};

mod join;
#[cfg(test)]
mod join_tests;
mod split;
#[cfg(test)]
mod split_tests;

/// What one step came to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Done,
    Skipped(String),
    Failed(String),
    Cancelled,
}

/// Why an attempt stopped: the item failed (with its cause, after the
/// user's Skip), or the job was cancelled.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stop {
    Failed(String),
    Cancelled,
}

impl From<Stop> for Outcome {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Failed(cause) => Outcome::Failed(cause),
            Stop::Cancelled => Outcome::Cancelled,
        }
    }
}

/// What the run shares with its stall detector.
pub(crate) struct Shared {
    progress: Mutex<JobProgress>,
    estimator: Mutex<Estimator>,
    origin: Instant,
    activity: AtomicU64,
    stalled: AtomicBool,
    quiet: AtomicBool,
    done: AtomicBool,
}

impl Shared {
    pub(crate) fn new(progress: JobProgress) -> Self {
        Self {
            progress: Mutex::new(progress),
            estimator: Mutex::new(Estimator::default()),
            origin: Instant::now(),
            activity: AtomicU64::new(0),
            stalled: AtomicBool::new(false),
            quiet: AtomicBool::new(false),
            done: AtomicBool::new(false),
        }
    }

    fn millis(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn touch(&self) {
        self.activity.store(self.millis(), Ordering::Relaxed);
    }

    fn progress(&self) -> std::sync::MutexGuard<'_, JobProgress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn estimator(&self) -> std::sync::MutexGuard<'_, Estimator> {
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Watches for a job that makes no progress: after `after` with no byte
/// and no step, while not paused or asking, the job says it stalled. It
/// ends when the run drops the sender of `stop`.
pub(crate) fn watch_for_stalls(
    shared: &Shared,
    sink: &dyn JobSink,
    after: Duration,
    stop: &mpsc::Receiver<()>,
) {
    let tick = (after / 4).clamp(Duration::from_millis(5), Duration::from_millis(200));
    let limit = u64::try_from(after.as_millis()).unwrap_or(u64::MAX);
    loop {
        match stop.recv_timeout(tick) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            _ => return,
        }
        if shared.quiet.load(Ordering::Relaxed) || shared.done.load(Ordering::Relaxed) {
            continue;
        }
        let idle = shared
            .millis()
            .saturating_sub(shared.activity.load(Ordering::Relaxed));
        if idle >= limit && !shared.stalled.swap(true, Ordering::Relaxed) {
            shared.estimator().reset();
            let mut progress = shared.progress().clone();
            progress.state = JobState::Stalled;
            progress.seconds_left = None;
            sink.progress(&progress);
        }
    }
}

/// How one file is streamed: its write mode, whether it replaces a file,
/// whether the bytes are hashed for the read-back, and whether the commit
/// flushes to storage first (a move, whose source is removed after, and a
/// replace).
#[derive(Clone, Copy, Debug)]
struct Stream {
    mode: WriteMode,
    replace: bool,
    hash: bool,
    durable: bool,
}

/// What a step made, to check against a fresh read at the end.
struct Made {
    place: Place,
    kind: EntryKind,
    size: Option<u64>,
}

/// One run of one plan.
pub(crate) struct Run<'a> {
    engine: &'a Engine,
    plan: &'a Plan,
    control: &'a JobControl,
    sink: &'a dyn JobSink,
    shared: &'a Shared,
    ctx: OpContext,
    report: JobReport,
    answers: HashMap<ConflictKind, ConflictAction>,
    skip_classes: HashSet<ErrorClass>,
    made: Vec<Made>,
    gone: Vec<Place>,
    folders: Vec<(Place, Entry)>,
    lost: Vec<(Place, String)>,
    journal: Option<JournalWriter>,
    paused_said: bool,
    split: Option<split::SplitState>,
    join: Option<join::JoinState>,
}

/// The reason a step is skipped when its entry changed since the plan.
const CHANGED: &str = "it changed since the plan";

/// How closely an entry must match what the plan saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// A copy: only the kind must match; anything else is said.
    Copy,
    /// A step that takes the entry away: the kind and the identity.
    Removal,
    /// A permanent delete or an undo: the size and time too.
    Exact,
}

fn vfs_of(error: io::Error) -> VfsError {
    if let Some(inner) = error.get_ref().and_then(|e| e.downcast_ref::<VfsError>()) {
        return inner.clone();
    }
    VfsError::from(error)
}

fn name_of(place: &Place) -> String {
    place
        .name()
        .map_or_else(|| place.to_string(), |name| spell_out(name).into_owned())
}

fn same_entry(a: &Entry, b: &Entry, a_place: &Place, b_place: &Place) -> bool {
    match (a.volume.zip(a.inode), b.volume.zip(b.inode)) {
        (Some(x), Some(y)) => x == y,
        _ => a_place.is_within(b_place) && b_place.is_within(a_place),
    }
}

impl<'a> Run<'a> {
    pub(crate) fn new(
        engine: &'a Engine,
        plan: &'a Plan,
        control: &'a JobControl,
        sink: &'a dyn JobSink,
        shared: &'a Shared,
    ) -> Self {
        let mut report = JobReport::new(plan.job, plan.kind);
        report.undo.origin = plan.facts.origin.clone();
        Self {
            engine,
            plan,
            control,
            sink,
            shared,
            ctx: OpContext::new().with_cancel(control.cancel.clone()),
            report,
            answers: HashMap::new(),
            skip_classes: HashSet::new(),
            made: Vec::new(),
            gone: Vec::new(),
            folders: Vec::new(),
            lost: Vec::new(),
            journal: None,
            paused_said: false,
            split: None,
            join: None,
        }
    }

    // ----- progress and the state said -----

    fn emit(&self, state: JobState) {
        let snapshot = {
            let mut progress = self.shared.progress();
            if progress.state == JobState::Stalled && state == JobState::Running {
                self.shared.estimator().reset();
            }
            progress.state = state;
            if state == JobState::Running {
                let now = self.engine.pacer().now();
                let (rate, left) =
                    self.shared
                        .estimator()
                        .update(now, progress.bytes_done, progress.bytes_total);
                progress.bytes_per_second = rate;
                progress.seconds_left = left;
            } else {
                progress.seconds_left = None;
            }
            progress.clone()
        };
        if state == JobState::Running {
            self.shared.touch();
            self.shared.stalled.store(false, Ordering::Relaxed);
        }
        self.sink.progress(&snapshot);
    }

    fn advance(&self, bytes: u64) {
        self.shared.progress().bytes_done += bytes;
        self.emit(JobState::Running);
    }

    fn rewind(&self, bytes: u64) {
        let mut progress = self.shared.progress();
        progress.bytes_done = progress.bytes_done.saturating_sub(bytes);
    }

    fn set_current(&self, place: &Place) {
        self.shared.progress().current = Some(name_of(place));
        self.emit(JobState::Running);
    }

    /// Waits while paused, saying so once; `false` when cancelled.
    fn hold(&mut self) -> bool {
        while self.control.is_paused() && !self.control.cancel.is_cancelled() {
            if !self.paused_said {
                self.paused_said = true;
                self.shared.quiet.store(true, Ordering::Relaxed);
                self.emit(JobState::Paused);
            }
            thread::sleep(Duration::from_millis(20));
        }
        if self.paused_said {
            self.paused_said = false;
            self.shared.quiet.store(false, Ordering::Relaxed);
            self.shared.estimator().reset();
            self.emit(JobState::Running);
        }
        !self.control.cancel.is_cancelled()
    }

    fn warn(&mut self, place: &Place, text: impl Into<String>) {
        self.report.warnings.push((place.clone(), text.into()));
    }

    // ----- answers -----

    fn ask_error(&self, error: ItemError) -> ErrorAction {
        self.shared.quiet.store(true, Ordering::Relaxed);
        self.emit(JobState::WaitingForAnswer);
        let answer = self.sink.error(&error);
        self.shared.quiet.store(false, Ordering::Relaxed);
        self.emit(JobState::Running);
        answer
    }

    fn ask_conflict(&self, conflict: &Conflict) -> diroxide_core::ConflictAnswer {
        self.shared.quiet.store(true, Ordering::Relaxed);
        self.emit(JobState::WaitingForAnswer);
        let answer = self.sink.conflict(conflict);
        self.shared.quiet.store(false, Ordering::Relaxed);
        self.emit(JobState::Running);
        answer
    }

    /// Runs an operation on one item, with the error recovery of research
    /// file-operations.md: a lost network retried on its own, a class
    /// already answered "Skip all like this" skipped, everything else asked
    /// with Retry first only where it helps.
    fn attempt<T>(
        &mut self,
        place: &Place,
        mut op: impl FnMut(&mut Self) -> Result<T, VfsError>,
    ) -> Result<T, Stop> {
        let mut tries = 1;
        let mut automatic = 0;
        loop {
            if self.control.cancel.is_cancelled() {
                return Err(Stop::Cancelled);
            }
            let error = match op(self) {
                Ok(value) => {
                    if automatic > 0 {
                        self.warn(place, format!("done after {automatic} automatic retries"));
                    }
                    return Ok(value);
                }
                Err(VfsError::Cancelled) => return Err(Stop::Cancelled),
                Err(error) => error,
            };
            if self.control.cancel.is_cancelled() {
                return Err(Stop::Cancelled);
            }
            let class = error.class();
            if self.skip_classes.contains(&class) {
                return Err(Stop::Failed(error.to_string()));
            }
            let options = self.engine.options();
            if class.retries_on_its_own() && automatic < options.network_retries {
                automatic += 1;
                let wait = options.network_retry_wait;
                if !crate::pace::wait(self.engine.pacer(), wait, &self.control.cancel) {
                    return Err(Stop::Cancelled);
                }
                continue;
            }
            let item = ItemError {
                kind: self.plan.kind,
                place: place.clone(),
                error: error.clone(),
                attempt: tries,
            };
            let choices = item.choices();
            match self.ask_error(item) {
                ErrorAction::Retry if choices.contains(&ErrorAction::Retry) => {
                    tries += 1;
                    automatic = 0;
                }
                ErrorAction::Retry | ErrorAction::Skip => {
                    return Err(Stop::Failed(error.to_string()));
                }
                ErrorAction::SkipAllLikeThis => {
                    self.skip_classes.insert(class);
                    return Err(Stop::Failed(error.to_string()));
                }
                ErrorAction::Cancel => return Err(Stop::Cancelled),
            }
        }
    }

    fn provider(&mut self, place: &Place) -> Result<Arc<dyn Provider>, Stop> {
        let registry = self.engine.registry();
        self.attempt(place, |_| registry.resolve(place))
    }

    /// Reads an entry again before acting on it, and compares it with
    /// what the plan saw. A different kind is always a change, and the
    /// step is skipped; another entry under the name (a new identity) is
    /// skipped by a step that takes the entry away, and only said by a
    /// copy; a permanent delete, and every undo, also checks the size and
    /// the time.
    fn recheck(&mut self, place: &Place, check: Check) -> Result<Entry, Outcome> {
        let provider = self.provider(place)?;
        let ctx = self.ctx.clone();
        let entry = self.attempt(place, |_| provider.stat(place, &ctx))?;
        let Some(seen) = self.plan.expected.get(place) else {
            return Ok(entry);
        };
        if seen.kind != entry.kind {
            return Err(Outcome::Skipped(format!(
                "{CHANGED}: it is now a {}",
                entry.kind.word()
            )));
        }
        let check = if self.plan.kind == JobKind::Undo {
            Check::Exact
        } else {
            check
        };
        let other = matches!(
            (seen.volume.zip(seen.inode), entry.volume.zip(entry.inode)),
            (Some(a), Some(b)) if a != b
        );
        let changed = entry.kind != EntryKind::Folder
            && ((seen.size.is_some() && seen.size != entry.size)
                || (seen.modified.is_some() && seen.modified != entry.modified));
        match check {
            Check::Copy if other || changed => self.warn(place, CHANGED),
            Check::Copy => {}
            Check::Removal | Check::Exact if other => {
                return Err(Outcome::Skipped(format!(
                    "{CHANGED}: another entry has its name"
                )));
            }
            Check::Exact if changed => return Err(Outcome::Skipped(CHANGED.to_owned())),
            Check::Removal | Check::Exact => {}
        }
        Ok(entry)
    }

    /// The folder a step sits under that did not come to be: its steps
    /// are skipped, the folder itself reported once.
    fn lost_under(&self, place: &Place) -> Option<String> {
        self.lost
            .iter()
            .find(|(root, _)| place != root && place.is_within(root))
            .map(|(root, _)| format!("its folder was not made: {}", name_of(root)))
    }

    // ----- the main loop -----

    pub(crate) fn execute(&mut self) {
        let plan = self.plan;
        let steps = &plan.steps;
        self.report
            .skipped
            .extend(self.plan.facts.left_out.iter().cloned());
        if let Some(journal) = self.engine.journal() {
            match journal.start(
                self.plan.job,
                self.plan.kind,
                &self.plan.facts.origin,
                steps.len() as u64,
            ) {
                Ok(writer) => self.journal = Some(writer),
                Err(error) => {
                    let first = steps.first().map(|s| s.subject().clone());
                    if let Some(place) = first {
                        self.warn(
                            &place,
                            format!("the undo journal could not be written: {error}"),
                        );
                    }
                }
            }
        }
        self.emit(JobState::Running);
        let mut index = 0;
        while index < steps.len() {
            if !self.hold() {
                self.report.cancelled = true;
                break;
            }
            if let Some(&(_, end)) = plan.facts.whole.iter().find(|(s, _)| *s == index) {
                let end = end.min(steps.len());
                self.run_whole(index, end);
                index = end;
                continue;
            }
            let step = &steps[index];
            self.set_current(step.subject());
            let before = self.mark();
            let outcome = match self.lost_under(step.subject()) {
                Some(reason) => Outcome::Skipped(reason),
                None => self.step(step),
            };
            self.record(index, step.subject(), &outcome, before);
            // A split's sidecar is not one of its parts.
            if !matches!(step, Step::WriteChecksum { .. }) {
                self.shared.progress().items_done += 1;
            }
            if outcome == Outcome::Cancelled {
                self.report.cancelled = true;
                break;
            }
            index += 1;
        }
        self.finish();
    }

    /// The report, once the run is over.
    pub(crate) fn into_report(self) -> JobReport {
        self.report
    }

    /// The lengths of the undo record, before a step.
    fn mark(&self) -> (usize, usize, usize) {
        let undo = &self.report.undo;
        (undo.moves.len(), undo.trashed.len(), undo.created.len())
    }

    fn record(
        &mut self,
        index: usize,
        subject: &Place,
        outcome: &Outcome,
        before: (usize, usize, usize),
    ) {
        let word = match outcome {
            Outcome::Done => {
                self.report.done.push(subject.clone());
                "done"
            }
            Outcome::Skipped(reason) => {
                self.report.skipped.push((subject.clone(), reason.clone()));
                "skipped"
            }
            Outcome::Failed(cause) => {
                self.report.failed.push((subject.clone(), cause.clone()));
                "failed"
            }
            Outcome::Cancelled => "cancelled",
        };
        if let Some(journal) = &mut self.journal {
            let undo = &self.report.undo;
            let tail = |list: &[Place], from: usize| list.get(from..).unwrap_or_default().to_vec();
            let delta = UndoRecord {
                moves: undo.moves.get(before.0..).unwrap_or_default().to_vec(),
                trashed: tail(&undo.trashed, before.1),
                created: tail(&undo.created, before.2),
                ..UndoRecord::default()
            };
            let _ = journal.step(index as u64, word, &delta);
        }
    }

    /// Takes a place back off the made list: the newest mention, the one
    /// this step added.
    fn unmake(&mut self, place: &Place) {
        let created = &mut self.report.undo.created;
        if let Some(at) = created.iter().rposition(|made| made == place) {
            created.remove(at);
        }
    }

    /// An item inside a step that the run reached on its own (a folder's
    /// contents moved one by one), reported in the result list.
    fn record_inner(&mut self, place: &Place, outcome: &Outcome) {
        match outcome {
            Outcome::Done => self.report.done.push(place.clone()),
            Outcome::Skipped(reason) => self.report.skipped.push((place.clone(), reason.clone())),
            Outcome::Failed(cause) => self.report.failed.push((place.clone(), cause.clone())),
            Outcome::Cancelled => {}
        }
    }

    fn finish(&mut self) {
        self.finish_split();
        self.finish_join();
        let folders = std::mem::take(&mut self.folders);
        for (place, source) in folders.iter().rev() {
            if let Ok(provider) = self.engine.registry().resolve(place) {
                self.keep_times(&*provider, place, source);
            }
        }
        self.reconcile();
        {
            let mut progress = self.shared.progress();
            progress.current = None;
        }
        if let Some(journal) = &mut self.journal {
            let how = if self.report.cancelled {
                "cancelled"
            } else {
                "finished"
            };
            let _ = journal.end(how, &self.report.undo.left);
        }
        self.emit(JobState::Finished);
    }

    /// Checks what the job made against a fresh read: present, or
    /// missing (a provider that said yes and did nothing).
    fn reconcile(&mut self) {
        let mut planned = self.plan.planned_results();
        planned.retain(|place| !self.gone.iter().any(|gone| place.is_within(gone)));
        // A join's parts all make one whole: count it once.
        let mut seen = HashSet::new();
        planned.retain(|place| seen.insert(place.clone()));
        for made in &self.made {
            if !planned.contains(&made.place) {
                planned.push(made.place.clone());
            }
        }
        if self.made.is_empty() && planned.is_empty() {
            return;
        }
        let mut check = Reconciliation::of(planned);
        let made = std::mem::take(&mut self.made);
        let ctx = OpContext::new();
        for item in made {
            let found = self
                .engine
                .registry()
                .resolve(&item.place)
                .and_then(|provider| provider.stat(&item.place, &ctx));
            let present = match &found {
                Ok(entry) => {
                    entry.kind == item.kind
                        && (item.kind != EntryKind::File
                            || item.size.is_none()
                            || entry.size == item.size)
                        && (item.place.name() == Some(entry.name.as_str())
                            || item.place.is_root()
                            || (item.kind == EntryKind::File && entry.size == item.size))
                }
                Err(_) => false,
            };
            if present {
                if let Ok(entry) = found {
                    self.report.undo.left.push(LeftAs::of(&item.place, &entry));
                }
                check.present.push(item.place);
            } else {
                check.missing.push(item.place);
            }
        }
        if self.plan.kind != JobKind::Trash && self.plan.kind != JobKind::DeletePermanently {
            self.report.reconciliation = Some(check);
        }
    }

    fn made(&mut self, place: &Place, kind: EntryKind, size: Option<u64>) {
        self.made.push(Made {
            place: place.clone(),
            kind,
            size,
        });
    }

    /// An entry the job itself took away again (a temporary name renamed,
    /// a set-aside copy trashed): it is not looked for at the end.
    fn gone(&mut self, place: &Place) {
        self.made.retain(|made| !made.place.is_within(place));
        self.gone.push(place.clone());
    }

    fn step(&mut self, step: &Step) -> Outcome {
        match step {
            Step::Copy { from, to } => self.copy(from, to),
            Step::Move { from, to } => self.move_entry(from, to, 0),
            Step::Rename { from, to } => self.rename(from, to),
            Step::RenameHop { from, via, to } => self.hop(from, via, to),
            Step::MakeFolder { place } => self.make_folder(place),
            Step::Link { place, request } => self.link(place, request),
            Step::Trash { place } => self.trash(place),
            Step::Restore { place } => self.restore(place),
            Step::Remove { place } => self.remove(place, false),
            Step::RemoveEmptyFolder { place } => self.remove(place, true),
            Step::SplitPart {
                from,
                to,
                offset,
                length,
            } => self.split_part(from, to, *offset, *length),
            Step::WriteChecksum { of, place } => self.write_checksum(of, place),
            Step::JoinPart { from, to } => self.join_part(from, to),
            Step::SetAttributes { place, change } => {
                let provider = match self.provider(place) {
                    Ok(provider) => provider,
                    Err(stop) => return stop.into(),
                };
                let ctx = self.ctx.clone();
                match self.attempt(place, |_| provider.set_attributes(place, change, &ctx)) {
                    Ok(_) => Outcome::Done,
                    Err(stop) => stop.into(),
                }
            }
        }
    }

    // ----- copy -----

    fn copy(&mut self, from: &Place, to: &Place) -> Outcome {
        let entry = match self.recheck(from, Check::Copy) {
            Ok(entry) => entry,
            Err(outcome) => {
                self.lose(from, &outcome);
                return outcome;
            }
        };
        let outcome = match entry.kind {
            EntryKind::Folder => self.copy_folder(from, to, &entry),
            EntryKind::Link => self.copy_link(from, to, &entry),
            EntryKind::File | EntryKind::Archive => match self.settle(from, &entry, to) {
                Ok((target, mode, replace)) => {
                    match self.place_copy(from, &target, mode, replace, &entry, false) {
                        Ok(_) => Outcome::Done,
                        Err(outcome) => outcome,
                    }
                }
                Err(outcome) => outcome,
            },
            EntryKind::Item | EntryKind::Special => {
                Outcome::Skipped("a special file is not copied".to_owned())
            }
        };
        if entry.kind == EntryKind::Folder {
            self.lose(from, &outcome);
        }
        outcome
    }

    /// A folder whose step did not make its target: what is inside it is
    /// skipped, and the folder reported once.
    fn lose(&mut self, place: &Place, outcome: &Outcome) {
        if !matches!(outcome, Outcome::Done) {
            self.lost.push((place.clone(), String::new()));
        }
    }

    fn copy_folder(&mut self, from: &Place, to: &Place, entry: &Entry) -> Outcome {
        let provider = match self.provider(to) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        for _ in 0..3 {
            let ctx = self.ctx.clone();
            match provider.stat(to, &ctx) {
                Ok(existing) if existing.kind == EntryKind::Folder => {
                    // A folder over a folder merges: nothing to ask.
                    self.made(to, EntryKind::Folder, None);
                    return Outcome::Done;
                }
                Ok(existing) => {
                    let conflict = self.type_clash(from, entry, to, &existing);
                    return conflict;
                }
                Err(VfsError::NotFound) => {}
                Err(VfsError::Cancelled) => return Outcome::Cancelled,
                Err(_) => {
                    // Asked like any error; on Retry, look again.
                    let ctx = self.ctx.clone();
                    let provider = Arc::clone(&provider);
                    let retried = self.attempt(to, |_| match provider.stat(to, &ctx) {
                        Ok(_) | Err(VfsError::NotFound) => Ok(()),
                        Err(error) => Err(error),
                    });
                    if let Err(stop) = retried {
                        return stop.into();
                    }
                    continue;
                }
            }
            if let Err(outcome) = self.check_target_name(&*provider, to) {
                return outcome;
            }
            let ctx = self.ctx.clone();
            let made = self.attempt(to, |_| match provider.create_dir(to, &ctx) {
                Err(VfsError::AlreadyExists) => Ok(false),
                other => other.map(|()| true),
            });
            match made {
                Ok(true) => {
                    self.report.undo.created.push(to.clone());
                    self.folders.push((to.clone(), entry.clone()));
                    self.made(to, EntryKind::Folder, None);
                    return Outcome::Done;
                }
                Ok(false) => continue,
                Err(stop) => return stop.into(),
            }
        }
        Outcome::Failed("the folder could not be made".to_owned())
    }

    /// A file over a folder, or a folder over a file: asked under the Ask
    /// policy so the user can cancel, and always skipped, never resolved by
    /// deleting.
    fn type_clash(&mut self, from: &Place, entry: &Entry, to: &Place, existing: &Entry) -> Outcome {
        let kind = if entry.kind == EntryKind::Folder {
            ConflictKind::FolderOverFile
        } else {
            ConflictKind::FileOverFolder
        };
        let reason = if kind == ConflictKind::FolderOverFile {
            "a file has that name here"
        } else {
            "a folder has that name here"
        };
        let asks = matches!(
            self.plan.conflicts,
            ConflictPolicy::Ask | ConflictPolicy::Rename
        ) && !self.answers.contains_key(&kind);
        if asks {
            let answer = self.ask_conflict(&Conflict {
                kind,
                source: from.clone(),
                target: to.clone(),
                source_entry: entry.clone(),
                target_entry: existing.clone(),
                keep_both_name: String::new(),
            });
            if answer.action == ConflictAction::Cancel {
                return Outcome::Cancelled;
            }
            if answer.apply_to_rest {
                self.answers.insert(kind, ConflictAction::Skip);
            }
        }
        Outcome::Skipped(reason.to_owned())
    }

    /// Refuses a target name the destination would refuse or change.
    fn check_target_name(&self, provider: &dyn Provider, target: &Place) -> Result<(), Outcome> {
        let Some(name) = target.name() else {
            return Ok(());
        };
        let rules = NameRules::from_capabilities(&provider.capabilities(target));
        match check_name_with(name, &[], &rules)
            .into_iter()
            .find(|issue| issue.refused(&rules))
        {
            Some(issue) => Err(Outcome::Failed(format!(
                "the name is not allowed on this drive: {}",
                issue.describe()
            ))),
            None => Ok(()),
        }
    }

    /// Settles where a file goes: the target if free; else the conflict
    /// policy or the user's answer. Returns the target, how to write it,
    /// and whether it replaces a file that is there.
    fn settle(
        &mut self,
        from: &Place,
        entry: &Entry,
        to: &Place,
    ) -> Result<(Place, WriteMode, bool), Outcome> {
        let provider = self.provider(to)?;
        let mut target = to.clone();
        for _ in 0..8 {
            let ctx = self.ctx.clone();
            let existing = {
                let provider = Arc::clone(&provider);
                let target = target.clone();
                self.attempt(&target.clone(), move |_| {
                    match provider.stat(&target, &ctx) {
                        Ok(existing) => Ok(Some(existing)),
                        Err(VfsError::NotFound) => Ok(None),
                        Err(error) => Err(error),
                    }
                })?
            };
            let Some(existing) = existing else {
                return Ok((target, WriteMode::CreateNew, false));
            };
            if same_entry(entry, &existing, from, &target) {
                return Err(Outcome::Skipped(format!(
                    "Same file: {}. Nothing to do.",
                    name_of(&target)
                )));
            }
            if existing.kind == EntryKind::Folder {
                return Err(self.type_clash(from, entry, &target, &existing));
            }
            let kind = if is_read_only(&existing) || existing.hidden {
                ConflictKind::ReadOnlyTarget
            } else {
                ConflictKind::FileOverFile
            };
            let folder = target
                .parent()
                .ok_or(Outcome::Failed("the target has no folder".to_owned()))?;
            let keep_both = {
                let ctx = self.ctx.clone();
                let siblings: Vec<String> = provider
                    .list(&folder, &ctx)
                    .map(|entries| entries.into_iter().map(|e| e.name).collect())
                    .unwrap_or_default();
                let siblings: Vec<&str> = siblings.iter().map(String::as_str).collect();
                unused_name(target.name().unwrap_or_default(), &siblings)
            };
            let action = match self.answers.get(&kind) {
                Some(action) => action.clone(),
                None => match (self.plan.conflicts, kind) {
                    (ConflictPolicy::Skip, _) => ConflictAction::Skip,
                    (ConflictPolicy::KeepBoth, _) => ConflictAction::KeepBoth,
                    (ConflictPolicy::Overwrite, ConflictKind::FileOverFile) => {
                        ConflictAction::Replace
                    }
                    (ConflictPolicy::Newer, ConflictKind::FileOverFile) => {
                        ConflictAction::ReplaceIfNewer
                    }
                    _ => {
                        let answer = self.ask_conflict(&Conflict {
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
            let action = match action {
                ConflictAction::ReplaceIfNewer => {
                    let tolerance = self.engine.options().time_tolerance_seconds;
                    match (entry.modified, existing.modified) {
                        (Some(new), Some(old)) if new > old.plus_seconds(tolerance) => {
                            ConflictAction::Replace
                        }
                        _ => {
                            return Err(Outcome::Skipped(
                                "not newer than the one there".to_owned(),
                            ));
                        }
                    }
                }
                other => other,
            };
            match action {
                ConflictAction::Skip | ConflictAction::ReplaceIfNewer => {
                    return Err(Outcome::Skipped("the name is taken".to_owned()));
                }
                ConflictAction::Cancel => return Err(Outcome::Cancelled),
                ConflictAction::Replace => return Ok((target, WriteMode::Overwrite, true)),
                ConflictAction::KeepBoth => {
                    let Some(name) = keep_both else {
                        return Err(Outcome::Skipped(
                            "no free numbered name after 999 tries".to_owned(),
                        ));
                    };
                    let place = folder
                        .join(&name)
                        .map_err(|e| Outcome::Failed(VfsError::from(e).to_string()))?;
                    return Ok((place, WriteMode::CreateNew, false));
                }
                ConflictAction::Rename(name) => {
                    if name.is_empty() {
                        return Err(Outcome::Skipped("no new name was given".to_owned()));
                    }
                    target = match folder.join(&name) {
                        Ok(place) => place,
                        Err(error) => {
                            return Err(Outcome::Failed(VfsError::from(error).to_string()));
                        }
                    };
                }
            }
        }
        Err(Outcome::Skipped("the name is taken".to_owned()))
    }

    /// Whether a replace at this place sends the old file to the trash
    /// first.
    fn keeps_old(&self, provider: &dyn Provider, target: &Place) -> bool {
        self.engine.options().replace_keeps_old_in_trash && provider.capabilities(target).trash
    }

    /// Writes a copy of one file to a settled target: a clone first where
    /// the place has one and the target is free; else the stream. Then the
    /// size is checked, the copy read back when asked, the dates kept, and
    /// the copy recorded. Returns the bytes written.
    fn place_copy(
        &mut self,
        from: &Place,
        target: &Place,
        mode: WriteMode,
        replace: bool,
        entry: &Entry,
        moving: bool,
    ) -> Result<u64, Outcome> {
        let source = self.provider(from)?;
        let destination = self.provider(target)?;
        self.check_target_name(&*destination, target)?;
        let verify = if moving {
            self.engine.options().verify_moves
        } else {
            self.engine.options().verify_copies
        };
        let mut placed: Option<(u64, Option<Vec<u8>>)> = None;
        if mode == WriteMode::CreateNew
            && Arc::ptr_eq(&source, &destination)
            && destination.capabilities(target).clone_copy
            && destination.copy_hint(from, target) != CopyHint::Stream
        {
            let ctx = self.ctx.clone();
            match source.copy_within(from, target, &ctx) {
                Ok(()) => {
                    let size = entry.size.unwrap_or(0);
                    self.advance(size);
                    placed = Some((size, None));
                }
                Err(VfsError::Cancelled) => return Err(Outcome::Cancelled),
                Err(VfsError::Unsupported { .. }) => {}
                Err(_) => {
                    // A clone that failed late may have left part of the
                    // target: it is ours, since the target was free.
                    let ctx = OpContext::new();
                    if destination.stat(target, &ctx).is_ok() {
                        let _ = destination.remove(target, &ctx);
                    }
                }
            }
        }
        let (bytes, hash) = match placed {
            Some(done) => done,
            None => {
                let source = Arc::clone(&source);
                let destination = Arc::clone(&destination);
                let how = Stream {
                    mode,
                    replace,
                    hash: verify,
                    durable: moving || replace,
                };
                self.attempt(from, |run| {
                    run.stream_once(&*source, from, &*destination, target, how)
                })?
            }
        };
        // The commit's word is not proof: read the target again. When it
        // is not there, the old file it replaced comes back from the trash.
        let ctx = self.ctx.clone();
        let problem = match destination.stat(target, &ctx) {
            Ok(found) if found.size.is_none() || found.size == Some(bytes) => None,
            Ok(_) => Some("the copy is not the size of its source"),
            Err(_) => Some("nothing was written: the drive said yes, and the file is not there"),
        };
        if let Some(problem) = problem {
            if self.report.undo.trashed.last() == Some(target) {
                let missing = destination.stat(target, &ctx).is_err();
                if missing && destination.restore(target, &OpContext::new()).is_ok() {
                    self.report.undo.trashed.pop();
                } else {
                    self.warn(target, "the old copy is in the trash");
                }
            }
            return Err(Outcome::Failed(problem.to_owned()));
        }
        if verify
            && let Some(expected) = hash.as_deref()
            && let Err(outcome) = self.verify(&*destination, target, expected)
        {
            if mode == WriteMode::CreateNew {
                let _ = destination.remove(target, &OpContext::new());
            }
            return Err(outcome);
        }
        if entry.size.is_some_and(|size| size != bytes) {
            self.warn(from, "changed while copying");
        }
        self.keep_times(&*destination, target, entry);
        if replace && !self.keeps_old(&*destination, target) {
            self.warn(target, "replaced; this cannot be undone");
        }
        self.report.undo.created.push(target.clone());
        self.made(target, EntryKind::File, Some(bytes));
        Ok(bytes)
    }

    /// One try at streaming a file into a committing writer, in chunks.
    /// A replace sends the old file to the trash only after every byte is
    /// written, and brings it back if the commit fails.
    fn stream_once(
        &mut self,
        source: &dyn Provider,
        from: &Place,
        destination: &dyn Provider,
        target: &Place,
        how: Stream,
    ) -> Result<(u64, Option<Vec<u8>>), VfsError> {
        let Stream {
            mode,
            replace,
            hash,
            durable,
        } = how;
        let ctx = self.ctx.clone();
        let mut reader = source.read(from, &ctx)?;
        let mut writer = destination.write(target, mode, &ctx)?;
        let mut hasher = hash.then(Sha256::new);
        let mut buffer = vec![0u8; self.engine.options().chunk_bytes.max(1)];
        let mut bytes: u64 = 0;
        let result = (|| -> Result<(), VfsError> {
            loop {
                if !self.hold() {
                    return Err(VfsError::Cancelled);
                }
                let read = match reader.read(&mut buffer) {
                    Ok(read) => read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(vfs_of(error)),
                };
                if read == 0 {
                    return Ok(());
                }
                writer.write_all(&buffer[..read]).map_err(vfs_of)?;
                if let Some(hasher) = &mut hasher {
                    hasher.update(&buffer[..read]);
                }
                bytes += read as u64;
                self.advance(read as u64);
            }
        })();
        if let Err(error) = result {
            drop(writer);
            self.rewind(bytes);
            return Err(error);
        }
        if let Err(error) = writer.flush() {
            drop(writer);
            self.rewind(bytes);
            return Err(vfs_of(error));
        }
        let mut old_in_trash = false;
        if replace && self.keeps_old(destination, target) {
            let check = match destination.trash(target, &ctx) {
                Ok(check) => check,
                Err(error) => {
                    drop(writer);
                    self.rewind(bytes);
                    return Err(error);
                }
            };
            match self.post_check(destination, target, check) {
                TrashCheck::InTrash => old_in_trash = true,
                TrashCheck::NotChecked => {
                    old_in_trash = true;
                    self.warn(target, "the old copy went to the trash; not checked");
                }
                TrashCheck::GoneNotInTrash => {
                    self.warn(target, catalog::TRASH_NOT_CONFIRMED);
                }
                TrashCheck::StillThere => {
                    drop(writer);
                    self.rewind(bytes);
                    return Err(VfsError::Unsafe {
                        why: "the old copy is still there after the trash".to_owned(),
                    });
                }
            }
        }
        if let Err(error) = writer.commit(durable) {
            if old_in_trash && destination.restore(target, &OpContext::new()).is_err() {
                self.warn(target, "the old copy is in the trash");
            }
            self.rewind(bytes);
            return Err(error);
        }
        if old_in_trash {
            self.report.undo.trashed.push(target.clone());
        }
        Ok((bytes, hasher.map(|h| h.finalize().to_vec())))
    }

    /// Reads a target back and compares its hash, saying "Verifying".
    fn verify(
        &mut self,
        provider: &dyn Provider,
        target: &Place,
        expected: &[u8],
    ) -> Result<(), Outcome> {
        self.emit(JobState::Verifying);
        let ctx = self.ctx.clone();
        let mut hasher = Sha256::new();
        let read = (|| -> Result<(), VfsError> {
            let mut reader = provider.read(target, &ctx)?;
            let mut buffer = vec![0u8; self.engine.options().chunk_bytes.max(1)];
            loop {
                ctx.check()?;
                let read = reader.read(&mut buffer).map_err(vfs_of)?;
                if read == 0 {
                    return Ok(());
                }
                hasher.update(&buffer[..read]);
                self.shared.touch();
            }
        })();
        self.emit(JobState::Running);
        match read {
            Err(VfsError::Cancelled) => Err(Outcome::Cancelled),
            Err(error) => Err(Outcome::Failed(format!(
                "the copy could not be read back: {error}"
            ))),
            Ok(()) if hasher.finalize().as_slice() == expected => Ok(()),
            Ok(()) => Err(Outcome::Failed(
                "the copy did not match its source; the source is kept".to_owned(),
            )),
        }
    }

    /// Keeps the dates (and the attributes) of the source on a copy: the
    /// modified time; the created time where the place can set one; never
    /// the accessed time. A failure is a warning, never a failure.
    fn keep_times(&mut self, provider: &dyn Provider, target: &Place, source: &Entry) {
        let options = self.engine.options();
        let capabilities = provider.capabilities(target);
        let mut change = AttributeChange::default();
        if options.keep_dates {
            change.modified = source.modified;
            if source.created.is_some() {
                if capabilities.access.created_time.is_possible() {
                    change.created = source.created;
                } else if options.warn_created_time {
                    self.warn(
                        target,
                        "copied, but its created time could not be kept: this drive cannot set one",
                    );
                }
            }
        }
        change.attributes = match source.attributes {
            Some(attributes @ diroxide_core::Attributes::Unix(_))
                if capabilities.access.mode.is_possible() =>
            {
                Some(attributes)
            }
            Some(attributes @ diroxide_core::Attributes::Windows(_))
                if capabilities.access.os_attributes.is_possible() =>
            {
                Some(attributes)
            }
            _ => None,
        };
        if change.is_empty() {
            return;
        }
        let ctx = OpContext::new();
        if let Err(error) = provider.set_attributes(target, &change, &ctx) {
            let what = if change.modified.is_some() {
                "its modified time"
            } else {
                "its attributes"
            };
            self.warn(
                target,
                format!("copied, but could not keep {what}: {error}"),
            );
        }
    }

    fn copy_link(&mut self, from: &Place, to: &Place, entry: &Entry) -> Outcome {
        let Some(link) = entry.link.clone() else {
            return Outcome::Skipped("the link's target could not be read".to_owned());
        };
        let (target, _, replace) = match self.settle(from, entry, to) {
            Ok(settled) => settled,
            Err(outcome) => return outcome,
        };
        let provider = match self.provider(&target) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        if replace {
            return Outcome::Skipped("a link does not replace a file; the file is kept".to_owned());
        }
        let request = LinkRequest {
            kind: LinkKind::Symbolic,
            target: link.target,
            target_is_folder: link.points_to == Some(EntryKind::Folder),
        };
        let ctx = self.ctx.clone();
        match self.attempt(&target, |_| provider.create_link(&target, &request, &ctx)) {
            Ok(()) => {
                self.report.undo.created.push(target.clone());
                self.made(&target, EntryKind::Link, None);
                Outcome::Done
            }
            Err(stop) => stop.into(),
        }
    }

    // ----- move -----

    fn move_entry(&mut self, from: &Place, to: &Place, depth: usize) -> Outcome {
        let entry = match self.recheck(from, Check::Removal) {
            Ok(entry) => entry,
            Err(outcome) => return outcome,
        };
        if entry.kind == EntryKind::Folder {
            self.move_folder(from, to, &entry, depth)
        } else {
            self.move_file(from, to, &entry)
        }
    }

    /// Renames a file where it can; otherwise copies, verifies, reads the
    /// source again, and only then removes it.
    fn move_file(&mut self, from: &Place, to: &Place, entry: &Entry) -> Outcome {
        let source = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let destination = match self.provider(to) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let (target, mode, replace) = match self.settle(from, entry, to) {
            Ok(settled) => settled,
            Err(outcome) => return outcome,
        };
        if let Err(outcome) = self.check_target_name(&*destination, &target) {
            return outcome;
        }
        if let Err(outcome) = self.ensure_parent(&*destination, &target) {
            return outcome;
        }
        if Arc::ptr_eq(&source, &destination) && !replace {
            let ctx = self.ctx.clone();
            let renamed = self.attempt(from, |_| match source.rename(from, &target, &ctx) {
                Ok(()) => Ok(true),
                Err(VfsError::CrossesDevices) => Ok(false),
                Err(error) => Err(error),
            });
            match renamed {
                Ok(true) => {
                    self.report.undo.moves.push((from.clone(), target.clone()));
                    self.made(&target, entry.kind, entry.size);
                    return Outcome::Done;
                }
                Ok(false) => {}
                Err(stop) => return stop.into(),
            }
        }
        if entry.kind == EntryKind::Link {
            return self.move_link(from, &target, entry);
        }
        self.copy_verify_remove(from, &target, mode, replace, entry)
    }

    /// A link moved across drives: a new link with the same target, then
    /// the old link removed. The target is never touched.
    fn move_link(&mut self, from: &Place, target: &Place, entry: &Entry) -> Outcome {
        match self.copy_link(from, target, entry) {
            Outcome::Done => {}
            other => return other,
        }
        self.unmake(target);
        self.remove_source(from, target)
    }

    fn copy_verify_remove(
        &mut self,
        from: &Place,
        target: &Place,
        mode: WriteMode,
        replace: bool,
        entry: &Entry,
    ) -> Outcome {
        if let Err(outcome) = self.place_copy(from, target, mode, replace, entry, true) {
            return outcome;
        }
        // The source is read again: a source that changed while it was
        // copied is kept, and so is the copy.
        let source = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        match source.stat(from, &ctx) {
            Ok(now)
                if now.kind == entry.kind
                    && now.size == entry.size
                    && now.modified == entry.modified => {}
            Ok(_) => {
                return Outcome::Failed("it changed while moving; both copies are kept".to_owned());
            }
            Err(error) => {
                return Outcome::Failed(format!(
                    "the original could not be read again, so it is kept: {error}"
                ));
            }
        }
        self.unmake(target);
        self.remove_source(from, target)
    }

    fn remove_source(&mut self, from: &Place, target: &Place) -> Outcome {
        let source = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        match self.attempt(from, |_| source.remove(from, &ctx)) {
            Ok(()) => {
                self.report.undo.moves.push((from.clone(), target.clone()));
                Outcome::Done
            }
            Err(Stop::Cancelled) => {
                self.report.undo.created.push(target.clone());
                Outcome::Cancelled
            }
            Err(Stop::Failed(cause)) => {
                self.report.undo.created.push(target.clone());
                Outcome::Failed(format!(
                    "copied, but the original could not be removed: {cause}"
                ))
            }
        }
    }

    /// Moves a folder: one rename where it can; onto a folder that is
    /// there, or across drives, its contents one by one, then the folder
    /// itself only when it is empty.
    fn move_folder(&mut self, from: &Place, to: &Place, entry: &Entry, depth: usize) -> Outcome {
        let source = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let destination = match self.provider(to) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        let existing = match destination.stat(to, &ctx) {
            Ok(existing) => Some(existing),
            Err(VfsError::NotFound) => None,
            Err(VfsError::Cancelled) => return Outcome::Cancelled,
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let mark = self.report.undo.moves.len();
        let mut made_here = false;
        match existing {
            Some(existing) if same_entry(entry, &existing, from, to) => {
                return Outcome::Skipped(format!("Same file: {}. Nothing to do.", name_of(to)));
            }
            Some(existing) if existing.kind != EntryKind::Folder => {
                return self.type_clash(from, entry, to, &existing);
            }
            Some(_) => {}
            None => {
                if let Err(outcome) = self.check_target_name(&*destination, to) {
                    return outcome;
                }
                if let Err(outcome) = self.ensure_parent(&*destination, to) {
                    return outcome;
                }
                if Arc::ptr_eq(&source, &destination) {
                    let ctx = self.ctx.clone();
                    let renamed = self.attempt(from, |_| match source.rename(from, to, &ctx) {
                        Ok(()) => Ok(true),
                        Err(VfsError::CrossesDevices) => Ok(false),
                        Err(error) => Err(error),
                    });
                    match renamed {
                        Ok(true) => {
                            self.report.undo.moves.push((from.clone(), to.clone()));
                            self.made(to, EntryKind::Folder, None);
                            return Outcome::Done;
                        }
                        Ok(false) => {}
                        Err(stop) => return stop.into(),
                    }
                }
                let ctx = self.ctx.clone();
                match self.attempt(to, |_| destination.create_dir(to, &ctx)) {
                    Ok(()) => {
                        made_here = true;
                        self.report.undo.created.push(to.clone());
                        self.folders.push((to.clone(), entry.clone()));
                        self.made(to, EntryKind::Folder, None);
                    }
                    Err(stop) => return stop.into(),
                }
            }
        }
        let outcome = self.move_contents(from, to, entry, depth);
        if outcome == Outcome::Done && made_here {
            // The whole folder went: undo moves it back as one.
            self.report.undo.moves.truncate(mark);
            self.unmake(to);
            self.report.undo.moves.push((from.clone(), to.clone()));
        }
        outcome
    }

    /// Makes the missing folders above a place, for an undo that puts an
    /// entry back where a folder of its was removed.
    fn ensure_parent(&mut self, provider: &dyn Provider, place: &Place) -> Result<(), Outcome> {
        if self.plan.kind != JobKind::Undo {
            return Ok(());
        }
        let mut missing = Vec::new();
        let mut current = place.parent();
        let ctx = OpContext::new();
        while let Some(folder) = current {
            if folder.is_root() || provider.stat(&folder, &ctx).is_ok() {
                break;
            }
            current = folder.parent();
            missing.push(folder);
        }
        for folder in missing.into_iter().rev() {
            if let Err(error) = provider.create_dir(&folder, &ctx)
                && error != VfsError::AlreadyExists
            {
                return Err(Outcome::Failed(format!(
                    "its folder could not be made again: {error}"
                )));
            }
        }
        Ok(())
    }

    fn move_contents(&mut self, from: &Place, to: &Place, entry: &Entry, depth: usize) -> Outcome {
        if depth >= self.engine.options().max_depth {
            return Outcome::Skipped(format!(
                "too deep: more than {} levels",
                self.engine.options().max_depth
            ));
        }
        let source = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        let mut children = match self.attempt(from, |_| source.list(from, &ctx)) {
            Ok(children) => children,
            Err(stop) => return stop.into(),
        };
        children.sort_by(|a, b| a.name.cmp(&b.name));
        for child in children {
            if !self.hold() {
                return Outcome::Cancelled;
            }
            let (Ok(child_from), Ok(child_to)) = (from.join(&child.name), to.join(&child.name))
            else {
                continue;
            };
            let outcome = if is_unfinished(&child.name) {
                Outcome::Skipped(crate::walk::UNFINISHED.to_owned())
            } else if child.kind == EntryKind::Folder
                && entry.volume.is_some()
                && child.volume.is_some()
                && entry.volume != child.volume
            {
                Outcome::Skipped(crate::walk::OTHER_DRIVE.to_owned())
            } else {
                self.set_current(&child_from);
                self.move_entry(&child_from, &child_to, depth + 1)
            };
            if outcome == Outcome::Cancelled {
                return Outcome::Cancelled;
            }
            self.record_inner(&child_from, &outcome);
        }
        let ctx = self.ctx.clone();
        match source.remove(from, &ctx) {
            Ok(()) => Outcome::Done,
            Err(VfsError::NotEmpty) => {
                let left = source.list(from, &ctx).map_or(0, |entries| entries.len());
                Outcome::Skipped(format!(
                    "Folder kept: {} inside were not moved",
                    diroxide_core::format::count(left as u64, "item", "items")
                ))
            }
            Err(VfsError::Cancelled) => Outcome::Cancelled,
            Err(error) => Outcome::Failed(format!(
                "its contents moved, but the folder could not be removed: {error}"
            )),
        }
    }

    // ----- rename -----

    fn rename(&mut self, from: &Place, to: &Place) -> Outcome {
        let entry = match self.recheck(from, Check::Removal) {
            Ok(entry) => entry,
            Err(outcome) => return outcome,
        };
        let provider = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        match self.attempt(from, |_| provider.rename(from, to, &ctx)) {
            Ok(()) => {
                self.report.undo.moves.push((from.clone(), to.clone()));
                self.made(to, entry.kind, entry.size);
                Outcome::Done
            }
            Err(stop) => stop.into(),
        }
    }

    /// A rename through a temporary name, both halves at once, the first
    /// put back when the second fails.
    fn hop(&mut self, from: &Place, via: &Place, to: &Place) -> Outcome {
        let entry = match self.recheck(from, Check::Removal) {
            Ok(entry) => entry,
            Err(outcome) => return outcome,
        };
        let provider = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        if let Err(error) = provider.rename(from, via, &ctx) {
            return Outcome::Failed(error.to_string());
        }
        match provider.rename(via, to, &ctx) {
            Ok(()) => {
                self.report.undo.moves.push((from.clone(), via.clone()));
                self.report.undo.moves.push((via.clone(), to.clone()));
                self.made(to, entry.kind, entry.size);
                Outcome::Done
            }
            Err(error) => {
                if provider.rename(via, from, &OpContext::new()).is_err() {
                    self.warn(
                        from,
                        format!("left under the temporary name {}", name_of(via)),
                    );
                }
                Outcome::Failed(error.to_string())
            }
        }
    }

    /// Runs a group of renames whole (a cycle): each hop's first half in
    /// its place, the second halves at the end; a failure puts every name
    /// back and fails the group as one item each.
    fn run_whole(&mut self, start: usize, end: usize) {
        let plan = self.plan;
        let steps = &plan.steps[start..end];
        let mut before = self.mark();
        let mut done: Vec<(Place, Place, Arc<dyn Provider>)> = Vec::new();
        let mut finish: Vec<(Place, Place)> = Vec::new();
        let mut failure: Option<String> = None;
        let mut renames: Vec<(Place, Place)> = Vec::new();
        for step in steps {
            match step {
                Step::RenameHop { from, via, to } => {
                    renames.push((from.clone(), via.clone()));
                    finish.push((via.clone(), to.clone()));
                }
                Step::Rename { from, to } => renames.push((from.clone(), to.clone())),
                _ => {}
            }
        }
        renames.extend(finish);
        for (from, to) in renames {
            if plan.expected.contains_key(&from) {
                match self.recheck(&from, Check::Removal) {
                    Ok(_) => {}
                    Err(Outcome::Skipped(reason) | Outcome::Failed(reason)) => {
                        failure = Some(reason);
                        break;
                    }
                    Err(_) => {
                        failure = Some("it was cancelled".to_owned());
                        break;
                    }
                }
            }
            let provider = match self.engine.registry().resolve(&from) {
                Ok(provider) => provider,
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            };
            match provider.rename(&from, &to, &OpContext::new()) {
                Ok(()) => done.push((from, to, provider)),
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
        match failure {
            None => {
                for (from, to, _) in &done {
                    self.report.undo.moves.push((from.clone(), to.clone()));
                }
                for step in steps {
                    if let Some(to) = step.result() {
                        let seen = plan.expected.get(step.subject());
                        self.made(
                            to,
                            seen.map_or(EntryKind::File, |e| e.kind),
                            seen.and_then(|e| e.size),
                        );
                    }
                }
                for (offset, step) in steps.iter().enumerate() {
                    self.record(start + offset, step.subject(), &Outcome::Done, before);
                    before = self.mark();
                    self.shared.progress().items_done += 1;
                }
            }
            Some(cause) => {
                for (from, to, provider) in done.iter().rev() {
                    if provider.rename(to, from, &OpContext::new()).is_err() {
                        self.warn(from, format!("could not be put back from {}", name_of(to)));
                    }
                }
                let reason =
                    format!("the swap could not finish, the names are as they were: {cause}");
                for (offset, step) in steps.iter().enumerate() {
                    self.record(
                        start + offset,
                        step.subject(),
                        &Outcome::Failed(reason.clone()),
                        before,
                    );
                    before = self.mark();
                    self.shared.progress().items_done += 1;
                }
            }
        }
    }

    // ----- make folder, link, restore -----

    fn make_folder(&mut self, place: &Place) -> Outcome {
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        for _ in 0..3 {
            let ctx = self.ctx.clone();
            match provider.stat(place, &ctx) {
                Ok(existing) if existing.kind == EntryKind::Folder => return Outcome::Done,
                Ok(_) => return Outcome::Failed("a file has that name".to_owned()),
                Err(VfsError::NotFound) => {}
                Err(VfsError::Cancelled) => return Outcome::Cancelled,
                Err(error) => return Outcome::Failed(error.to_string()),
            }
            let ctx = self.ctx.clone();
            let made = self.attempt(place, |_| match provider.create_dir(place, &ctx) {
                Err(VfsError::AlreadyExists) => Ok(false),
                other => other.map(|()| true),
            });
            match made {
                Ok(true) => {
                    self.report.undo.created.push(place.clone());
                    self.made(place, EntryKind::Folder, None);
                    return Outcome::Done;
                }
                Ok(false) => continue,
                Err(stop) => return stop.into(),
            }
        }
        Outcome::Failed("the folder could not be made".to_owned())
    }

    fn link(&mut self, place: &Place, request: &LinkRequest) -> Outcome {
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        match self.attempt(place, |_| provider.create_link(place, request, &ctx)) {
            Ok(()) => {
                self.report.undo.created.push(place.clone());
                let kind = if request.kind == LinkKind::Hard {
                    EntryKind::File
                } else {
                    EntryKind::Link
                };
                self.made(place, kind, None);
                Outcome::Done
            }
            Err(stop) => stop.into(),
        }
    }

    fn restore(&mut self, place: &Place) -> Outcome {
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        match self.attempt(place, |_| provider.restore(place, &ctx)) {
            Ok(()) => {
                let kind = provider
                    .stat(place, &OpContext::new())
                    .map_or(EntryKind::File, |entry| entry.kind);
                self.made(place, kind, None);
                Outcome::Done
            }
            Err(stop) => stop.into(),
        }
    }

    // ----- the trash and removal -----

    /// The engine's own post-check, on top of the provider's: an item
    /// still at its place is not in the trash, whatever the call said.
    fn post_check(&self, provider: &dyn Provider, place: &Place, check: TrashCheck) -> TrashCheck {
        let still = provider.stat(place, &OpContext::new()).is_ok();
        match check {
            TrashCheck::InTrash | TrashCheck::NotChecked if still => TrashCheck::StillThere,
            other => other,
        }
    }

    fn trash(&mut self, place: &Place) -> Outcome {
        if let Err(outcome) = self.recheck(place, Check::Removal) {
            return outcome;
        }
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        let check = match self.attempt(place, |_| provider.trash(place, &ctx)) {
            Ok(check) => check,
            Err(stop) => return stop.into(),
        };
        match self.post_check(&*provider, place, check) {
            TrashCheck::InTrash => {
                self.report.undo.trashed.push(place.clone());
                self.gone(place);
                Outcome::Done
            }
            TrashCheck::NotChecked => {
                self.report.undo.trashed.push(place.clone());
                self.gone(place);
                self.warn(place, "Moved to the trash; not checked.");
                Outcome::Done
            }
            problem => Outcome::Failed(
                crate::error::OpsError::TrashNotConfirmed(
                    problem.problem().unwrap_or_default().to_owned(),
                )
                .to_string(),
            ),
        }
    }

    /// Removes one entry for good, or an emptied folder. A read-only entry
    /// is cleared first, as part of the listed delete; a folder that says
    /// "not empty" right after its contents went is tried three times; a
    /// folder with something left inside is kept and said once.
    fn remove(&mut self, place: &Place, empty_only: bool) -> Outcome {
        if empty_only
            && let Ok(provider) = self.engine.registry().resolve(place)
            && provider.stat(place, &OpContext::new()) == Err(VfsError::NotFound)
        {
            return Outcome::Skipped("it is not there".to_owned());
        }
        let check = if empty_only {
            Check::Removal
        } else {
            Check::Exact
        };
        let entry = match self.recheck(place, check) {
            Ok(entry) => entry,
            Err(outcome) => return outcome,
        };
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let mut cleared = false;
        let mut busy = 0;
        let result = self.attempt(place, |run| {
            loop {
                let ctx = run.ctx.clone();
                match provider.remove(place, &ctx) {
                    Err(VfsError::PermissionDenied) if !cleared && is_read_only(&entry) => {
                        cleared = true;
                        let change = AttributeChange {
                            attributes: match entry.attributes {
                                Some(diroxide_core::Attributes::Windows(flags)) => {
                                    Some(diroxide_core::Attributes::Windows(flags & !0x1))
                                }
                                Some(diroxide_core::Attributes::Unix(mode)) => {
                                    Some(diroxide_core::Attributes::Unix(mode | 0o200))
                                }
                                None => None,
                            },
                            ..AttributeChange::default()
                        };
                        provider.set_attributes(place, &change, &ctx)?;
                    }
                    Err(VfsError::NotEmpty) if busy < 3 => {
                        busy += 1;
                        crate::pace::wait(
                            run.engine.pacer(),
                            Duration::from_millis(100),
                            &run.control.cancel,
                        );
                    }
                    Err(VfsError::NotEmpty) => return Ok(false),
                    other => return other.map(|()| true),
                }
            }
        });
        match result {
            Ok(true) => {
                self.gone(place);
                Outcome::Done
            }
            Ok(false) => Outcome::Skipped("Folder kept: it is not empty".to_owned()),
            Err(stop) => stop.into(),
        }
    }
}
