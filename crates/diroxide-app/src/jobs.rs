//! The jobs line and the Jobs list: what runs and waits, in words, and the
//! jobs that ran.
//!
//! The rules (the owner's decision of Saturday, October 10, 2026, and R25,
//! part 1, 2.2): a change of state (started, paused, stalled, waiting for
//! an answer or a connection, verifying, finished) changes the line at
//! once; other progress changes it at most every
//! `progress_interval_seconds` in screen-reader mode and twice a second
//! otherwise; the final message is said once. The line is the engine's own
//! sentence, percent first, its estimate inside 40 cells
//! ([`JobProgress::describe`]); Say Status adds the detail line on demand.
//!
//! The queue is the jobs list (R25, 2.2): a job the runtime's queue holds
//! back waits its turn here as a row of its own. With several jobs the
//! line says the count first, then the front job's sentence: "3 jobs, 1
//! waiting: Copying 40 percent". Only the front job's progress is said as
//! it goes; the others speak on a change of state.
//!
//! Wave 1 (W1d) made this module; lane A2 (alpha 2, commit 79) added the
//! waiting jobs, the front job and the rows of the Jobs list.

use diroxide_core::format::count;
use diroxide_core::{JobId, JobKind, JobState, Place, Timestamp};
use diroxide_ops::JobProgress;

/// How long the line keeps a progress sentence before the next, in
/// milliseconds, outside screen-reader mode.
pub const SIGHTED_INTERVAL_MS: u64 = 500;

/// A job that ran, for the Finished group of the Jobs list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedJob {
    /// The job.
    pub job: JobId,
    /// What kind of job it was.
    pub kind: JobKind,
    /// The report's line, problems first: "Copy had 2 failures: 3 done."
    pub summary: String,
    /// What failed, each "name: reason".
    pub failed: Vec<String>,
    /// What was skipped, each "name: reason".
    pub skipped: Vec<String>,
    /// Warnings, each "name: what".
    pub warnings: Vec<String>,
    /// What the job was on, for its row: "12 items to build"; empty when
    /// not known.
    pub what: String,
    /// The places that failed, as the report named them.
    pub failed_places: Vec<Place>,
    /// The job's own sources that hold what failed: Retry the failed
    /// items runs them again and skips what is already there.
    pub retry_sources: Vec<Place>,
    /// Where the job wrote to, when it had a destination: a retry goes
    /// there again.
    pub destination: Option<Place>,
    /// How many items were done.
    pub done: u64,
    /// Whether it was cancelled before its end.
    pub cancelled: bool,
}

impl Default for FinishedJob {
    fn default() -> Self {
        Self {
            job: JobId::new(0),
            kind: JobKind::Unknown,
            summary: String::new(),
            failed: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            what: String::new(),
            failed_places: Vec::new(),
            retry_sources: Vec::new(),
            destination: None,
            done: 0,
            cancelled: false,
        }
    }
}

impl FinishedJob {
    /// The row of the Finished group, the outcome first: "Done: copy 12
    /// items to build", "Failed: copy to build, 2 items not done",
    /// "Cancelled: copy 3 items to build, 1 done".
    pub fn row(&self) -> String {
        let verb = short_verb(self.kind);
        let object = if self.what.is_empty() {
            String::new()
        } else {
            format!(" {}", self.what)
        };
        if self.cancelled {
            format!("Cancelled: {verb}{object}, {} done", self.done)
        } else if !self.failed.is_empty() {
            let failed = self.failed.len() as u64;
            format!(
                "Failed: {verb}{object}, {} not done",
                count(failed, "item", "items")
            )
        } else {
            format!("Done: {verb}{object}")
        }
    }
}

/// What a job is on, kept from its request for its rows and a retry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobFacts {
    /// "12 items to build" ([`job_object`]).
    pub what: String,
    /// Where it writes to.
    pub destination: Option<Place>,
    /// Its sources.
    pub sources: Vec<Place>,
}

impl JobFacts {
    /// The facts of a request's sources and destination.
    pub fn of(sources: &[Place], destination: Option<&Place>) -> Self {
        Self {
            what: job_object(sources, destination),
            destination: destination.cloned(),
            sources: sources.to_vec(),
        }
    }
}

/// What the queue said about its jobs (the runtime's side of the jobs
/// list, R25, 2.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueNote {
    /// The queue let a job start: its thread runs it now.
    Started(JobId),
    /// A job waits its turn: its class's slots are taken, or the queue is
    /// paused.
    Waits(JobId),
    /// The queue's order and state, after any change.
    View {
        /// The jobs that wait their turn, in the order they start.
        waiting: Vec<JobId>,
        /// Whether the queue is paused: nothing new starts.
        paused: bool,
    },
}

/// What the app asks of the queue: the Jobs list's actions and the
/// queue's settings (R25, 2.2 and 2.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueRequest {
    /// Start a waiting job now, past the limits.
    RunNow(JobId),
    /// Move a waiting job one place earlier.
    MoveUp(JobId),
    /// Move a waiting job one place later.
    MoveDown(JobId),
    /// Move a waiting job before every other.
    MoveToFront(JobId),
    /// Move a waiting job after every other.
    MoveToEnd(JobId),
    /// Pause a running job after its current chunk.
    Pause(JobId),
    /// Let a paused job go on.
    Resume(JobId),
    /// Pause the queue: nothing new starts.
    PauseQueue,
    /// Let the queue start jobs again.
    ResumeQueue,
    /// Pause the queue and every running job.
    PauseAll,
    /// Let the queue and every paused job go on.
    ResumeAll,
    /// A job's own speed limit, or the global one with no job; `None`
    /// takes it off.
    SpeedLimit {
        /// The job, or `None` for the global limit.
        job: Option<JobId>,
        /// The limit.
        limit: Option<diroxide_ops::SpeedLimit>,
    },
    /// Which jobs the global limit slows.
    LimitScope(diroxide_ops::LimitScope),
    /// How many jobs run at once.
    Limits(diroxide_ops::QueueLimits),
    /// A waiting job's error policy.
    ErrorPolicy {
        /// The job.
        job: JobId,
        /// Its policy.
        policy: diroxide_core::job::ErrorPolicy,
    },
}

/// The group a row of the Jobs list belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobGroup {
    /// Started and not finished: running, paused, stalled, waiting for an
    /// answer or a connection, verifying.
    Running,
    /// Waiting its turn in the queue.
    Waiting,
    /// Done, failed or cancelled.
    Finished,
}

impl JobGroup {
    /// The group's heading with its count: "Running, 2 jobs".
    pub fn heading(self, jobs: usize) -> String {
        let name = match self {
            JobGroup::Running => "Running",
            JobGroup::Waiting => "Waiting",
            JobGroup::Finished => "Finished",
        };
        format!("{name}, {}", count(jobs as u64, "job", "jobs"))
    }
}

/// One row of the Jobs list: its job, its group, its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRow {
    /// The job.
    pub job: JobId,
    /// Its group.
    pub group: JobGroup,
    /// The row, the state first: "Copying 40 percent: 12 items to build".
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Running {
    job: JobId,
    kind: Option<JobKind>,
    shown: String,
    state: Option<JobState>,
    shown_at: Option<Timestamp>,
    latest: Option<JobProgress>,
    /// What the job is on: "12 items to build"; empty when not known.
    what: String,
    /// Where it writes to.
    destination: Option<Place>,
    /// Its own sources, for a retry.
    sources: Vec<Place>,
    /// The start line, said when the queue lets it start, when it was
    /// held back first.
    start_line: Option<String>,
}

impl Running {
    fn new(job: JobId, kind: Option<JobKind>, shown: String) -> Self {
        Self {
            job,
            kind,
            shown,
            state: None,
            shown_at: None,
            latest: None,
            what: String::new(),
            destination: None,
            sources: Vec::new(),
            start_line: None,
        }
    }

    fn waits(&self) -> bool {
        self.state == Some(JobState::Queued)
    }
}

/// The running and waiting jobs, their latest progress sentences, and the
/// jobs that ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobTable {
    running: Vec<Running>,
    history: Vec<FinishedJob>,
    /// The queue's order of the waiting jobs, from its last view.
    order: Vec<JobId>,
    /// Whether the queue is paused.
    queue_paused: bool,
}

/// The milliseconds from `from` to `to`, zero when `to` is earlier.
pub fn millis_between(from: Timestamp, to: Timestamp) -> u64 {
    let seconds = to.unix_seconds() - from.unix_seconds();
    let nanos = i64::from(to.subsec_nanos()) - i64::from(from.subsec_nanos());
    let total = seconds.saturating_mul(1000) + nanos / 1_000_000;
    u64::try_from(total).unwrap_or(0)
}

/// The short verb of a job's rows: "copy", "move", "trash", "delete".
pub fn short_verb(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Trash => "trash",
        JobKind::MakeFolder => "make folder",
        JobKind::SetAttributes => "change permissions",
        JobKind::SetTimes => "change times",
        JobKind::TestArchive => "test",
        JobKind::PluginCommand => "run",
        JobKind::Unknown => "job",
        other => other.verb(),
    }
}

/// The verb a job's own lines start with: "Copy", "Trash", "Delete".
pub fn line_verb(kind: JobKind) -> String {
    let verb = short_verb(kind);
    let mut chars = verb.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What a job is on, for its rows and lines: the one item's name or the
/// count, then the destination's folder: "report.iso to build", "12 items
/// to build", "3 items".
pub fn job_object(sources: &[Place], destination: Option<&Place>) -> String {
    let what = match sources {
        [] => String::new(),
        [one] => one
            .name()
            .map(|n| diroxide_core::names::display_name(n).into_owned())
            .unwrap_or_else(|| crate::panel::place_leaf(one)),
        many => count(many.len() as u64, "item", "items"),
    };
    match destination {
        Some(to) if what.is_empty() => format!("to {}", crate::panel::place_leaf(to)),
        Some(to) => format!("{what} to {}", crate::panel::place_leaf(to)),
        None => what,
    }
}

impl JobTable {
    /// No jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes a job that started.
    pub fn start(&mut self, job: JobId, text: impl Into<String>) {
        self.running.push(Running::new(job, None, text.into()));
    }

    /// Notes a job of a kind that started, with its first line, at a time.
    pub fn begin(&mut self, job: JobId, kind: JobKind, text: impl Into<String>, now: Timestamp) {
        self.running.retain(|r| r.job != job);
        let mut entry = Running::new(job, Some(kind), text.into());
        entry.state = Some(JobState::Running);
        entry.shown_at = Some(now);
        self.running.push(entry);
    }

    /// Notes a job sent to the queue: what it is on and where it writes,
    /// and its start line, said now when `started`, or later when the
    /// queue lets it start.
    pub fn submit(
        &mut self,
        job: JobId,
        kind: JobKind,
        start_line: String,
        facts: JobFacts,
        started: bool,
        now: Timestamp,
    ) {
        self.begin(job, kind, start_line.clone(), now);
        if let Some(entry) = self.entry_mut(job) {
            entry.what = facts.what;
            entry.destination = facts.destination;
            entry.sources = facts.sources;
            if !started {
                entry.start_line = Some(start_line);
            }
        }
    }

    /// Takes what the queue said. Returns the line to say, when there is
    /// one: the start line of a job that waited ("Copy started: 12 items
    /// to build"), or "Queued: copy 12 items to build".
    pub fn queue_note(&mut self, note: &QueueNote) -> Option<String> {
        match note {
            QueueNote::Started(job) => {
                let entry = self.entry_mut(*job)?;
                if entry.waits() {
                    entry.state = Some(JobState::Running);
                }
                let line = entry.start_line.take()?;
                entry.shown = line.clone();
                Some(line)
            }
            QueueNote::Waits(job) => {
                let entry = self.entry_mut(*job)?;
                entry.state = Some(JobState::Queued);
                let kind = entry.kind.unwrap_or(JobKind::Unknown);
                entry.shown = JobProgress::queued(kind).describe();
                let object = if entry.what.is_empty() {
                    String::new()
                } else {
                    format!(" {}", entry.what)
                };
                Some(format!("Queued: {}{object}", short_verb(kind)))
            }
            QueueNote::View { waiting, paused } => {
                self.order = waiting.clone();
                self.queue_paused = *paused;
                None
            }
        }
    }

    /// Updates a job's progress sentence.
    pub fn update(&mut self, job: JobId, text: impl Into<String>) {
        if let Some(entry) = self.entry_mut(job) {
            entry.shown = text.into();
        }
    }

    /// Takes a progress update; returns the new line when the jobs line
    /// changes: at once on a change of state, else when `interval_ms` has
    /// passed since the line last changed. `true` with the line when the
    /// state changed. A job not yet started (still being planned) is
    /// started by its first update.
    pub fn progress(
        &mut self,
        job: JobId,
        progress: &JobProgress,
        now: Timestamp,
        interval_ms: u64,
    ) -> Option<(String, bool)> {
        if !self.running.iter().any(|r| r.job == job) {
            self.running
                .push(Running::new(job, Some(progress.kind), String::new()));
        }
        let entry = self.entry_mut(job)?;
        entry.latest = Some(progress.clone());
        entry.kind = Some(progress.kind);
        let changed = entry.state != Some(progress.state);
        let due = entry
            .shown_at
            .is_none_or(|at| millis_between(at, now) >= interval_ms);
        if !(changed || due) {
            return None;
        }
        let text = progress.describe();
        entry.state = Some(progress.state);
        entry.shown_at = Some(now);
        if text == entry.shown {
            return None;
        }
        entry.shown = text.clone();
        Some((text, changed))
    }

    /// The latest detail of a job, for Say Status: "12 of 300 items, 1.2
    /// of 4.0 GB, 48 MB a second".
    pub fn detail(&self, job: JobId) -> Option<String> {
        let entry = self.running.iter().find(|r| r.job == job)?;
        entry
            .latest
            .as_ref()
            .map(|p| format!("{}. {}", p.describe(), p.detail()))
    }

    /// Removes a finished job.
    pub fn finish(&mut self, job: JobId) {
        self.running.retain(|r| r.job != job);
        self.order.retain(|id| *id != job);
    }

    /// Removes a finished job and keeps it in the history, with what it
    /// was on when the table knew it.
    pub fn finished(&mut self, mut done: FinishedJob) {
        if let Some(entry) = self.running.iter().find(|r| r.job == done.job) {
            if done.what.is_empty() {
                done.what = entry.what.clone();
            }
            if done.destination.is_none() {
                done.destination = entry.destination.clone();
            }
            if done.retry_sources.is_empty() {
                done.retry_sources = entry
                    .sources
                    .iter()
                    .filter(|source| {
                        done.failed_places
                            .iter()
                            .any(|failed| failed == *source || failed.is_within(source))
                    })
                    .cloned()
                    .collect();
            }
        }
        self.finish(done.job);
        self.history.push(done);
        if self.history.len() > 100 {
            self.history.remove(0);
        }
    }

    /// The jobs that ran, oldest first.
    pub fn history(&self) -> &[FinishedJob] {
        &self.history
    }

    /// A job that ran, by its id.
    pub fn finished_job(&self, job: JobId) -> Option<&FinishedJob> {
        self.history.iter().rev().find(|f| f.job == job)
    }

    /// The running and waiting jobs, oldest first.
    pub fn ids(&self) -> Vec<JobId> {
        self.running.iter().map(|r| r.job).collect()
    }

    /// The job in front: the newest that has started, else the first that
    /// waits. Cancel stops it, and the jobs line says its sentence.
    pub fn front(&self) -> Option<JobId> {
        self.running
            .iter()
            .rev()
            .find(|r| !r.waits())
            .or_else(|| self.waiting_entries().into_iter().next())
            .map(|r| r.job)
    }

    /// The job in front (the name kept from alpha 1).
    pub fn newest(&self) -> Option<JobId> {
        self.front()
    }

    /// A job's kind, when known.
    pub fn kind_of(&self, job: JobId) -> Option<JobKind> {
        self.running
            .iter()
            .find(|r| r.job == job)
            .and_then(|r| r.kind)
    }

    /// A running or waiting job's state, as the table last heard it.
    pub fn state_of(&self, job: JobId) -> Option<JobState> {
        self.running
            .iter()
            .find(|r| r.job == job)
            .and_then(|r| r.state)
    }

    /// What a running or waiting job is on: "12 items to build".
    pub fn what_of(&self, job: JobId) -> Option<&str> {
        self.running
            .iter()
            .find(|r| r.job == job)
            .map(|r| r.what.as_str())
    }

    /// Whether a job waits its turn in the queue.
    pub fn waits(&self, job: JobId) -> bool {
        self.running.iter().any(|r| r.job == job && r.waits())
    }

    /// How many jobs are running or waiting.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    /// Whether nothing runs or waits.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// How many jobs wait their turn.
    pub fn waiting_count(&self) -> usize {
        self.running.iter().filter(|r| r.waits()).count()
    }

    /// Whether the queue is paused, as it last said.
    pub fn queue_paused(&self) -> bool {
        self.queue_paused
    }

    /// The jobs line: "No jobs", the one job's sentence, or the count
    /// first and the front job's sentence: "3 jobs, 1 waiting: Copying 40
    /// percent".
    pub fn line(&self) -> String {
        match self.running.as_slice() {
            [] => "No jobs".to_owned(),
            [only] => only.shown.clone(),
            many => {
                let front = self
                    .front()
                    .and_then(|job| many.iter().find(|r| r.job == job))
                    .map(|r| r.shown.as_str())
                    .unwrap_or_default();
                let waiting = self.waiting_count();
                let head = if waiting > 0 {
                    format!("{} jobs, {waiting} waiting", many.len())
                } else {
                    format!("{} jobs", many.len())
                };
                format!("{head}: {front}")
            }
        }
    }

    /// A running or waiting job's row, the state first: "Copying 40
    /// percent: 12 items to build", "Waiting 2 of 5: copy 3 items to
    /// build", "Paused at 30 percent: copy to build", "Waiting for build:
    /// copy at 30 percent".
    pub fn row_of(&self, job: JobId) -> Option<String> {
        let entry = self.running.iter().find(|r| r.job == job)?;
        Some(self.row(entry))
    }

    fn row(&self, entry: &Running) -> String {
        let kind = entry.kind.unwrap_or(JobKind::Unknown);
        let verb = short_verb(kind);
        let object = if entry.what.is_empty() {
            String::new()
        } else {
            format!(" {}", entry.what)
        };
        let percent = entry.latest.as_ref().map_or(0, JobProgress::percent);
        match entry.state {
            Some(JobState::Queued) => {
                let waiting = self.waiting_entries();
                let at = waiting
                    .iter()
                    .position(|r| r.job == entry.job)
                    .map_or(1, |i| i + 1);
                format!("Waiting {at} of {}: {verb}{object}", waiting.len())
            }
            Some(JobState::Paused) => format!("Paused at {percent} percent: {verb}{object}"),
            Some(JobState::Stalled) => format!("Stalled at {percent} percent: {verb}{object}"),
            Some(JobState::WaitingForConnection) => {
                let host = entry
                    .latest
                    .as_ref()
                    .and_then(|p| p.waits_for.clone())
                    .unwrap_or_else(|| "the connection".to_owned());
                format!("Waiting for {host}: {verb} at {percent} percent")
            }
            Some(JobState::WaitingForAnswer) => {
                format!("Waiting for an answer: {verb}{object}")
            }
            Some(JobState::Verifying) => format!("Verifying {percent} percent: {verb}{object}"),
            Some(JobState::Counting) => {
                format!("{}, counting:{object}", kind.progress_word())
            }
            _ if entry.latest.is_none() => {
                format!("{} started:{object}", line_verb(kind))
            }
            _ => format!("{} {percent} percent:{object}", kind.progress_word()),
        }
    }

    /// The waiting entries in the queue's order: the order its last view
    /// gave, then any it has not named yet, oldest first.
    fn waiting_entries(&self) -> Vec<&Running> {
        let mut waiting: Vec<&Running> = self
            .order
            .iter()
            .filter_map(|job| self.running.iter().find(|r| r.job == *job && r.waits()))
            .collect();
        for entry in self.running.iter().filter(|r| r.waits()) {
            if !waiting.iter().any(|w| w.job == entry.job) {
                waiting.push(entry);
            }
        }
        waiting
    }

    /// The rows of the Jobs list, group by group: running, then waiting in
    /// the queue's order, then finished, newest first.
    pub fn rows(&self) -> Vec<JobRow> {
        let mut rows: Vec<JobRow> = self
            .running
            .iter()
            .filter(|r| !r.waits())
            .map(|r| JobRow {
                job: r.job,
                group: JobGroup::Running,
                text: self.row(r),
            })
            .collect();
        rows.extend(self.waiting_entries().into_iter().map(|r| JobRow {
            job: r.job,
            group: JobGroup::Waiting,
            text: self.row(r),
        }));
        rows.extend(self.history.iter().rev().map(|f| JobRow {
            job: f.job,
            group: JobGroup::Finished,
            text: f.row(),
        }));
        rows
    }

    fn entry_mut(&mut self, job: JobId) -> Option<&mut Running> {
        self.running.iter_mut().find(|r| r.job == job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::UtcOffset;
    use diroxide_core::format::within_cells;

    #[test]
    fn the_jobs_line_says_what_runs() {
        let mut jobs = JobTable::new();
        assert_eq!(jobs.line(), "No jobs");
        jobs.start(JobId::new(1), "Copying 0 of 3");
        jobs.update(JobId::new(1), "Copying 1 of 3");
        assert_eq!(jobs.line(), "Copying 1 of 3");
        jobs.start(JobId::new(2), "Moving 0 of 1");
        // The count first, then the front job's sentence: the newest.
        assert_eq!(jobs.line(), "2 jobs: Moving 0 of 1");
        jobs.finish(JobId::new(1));
        assert_eq!(jobs.len(), 1);
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC)
            .unwrap()
            .plus_seconds(seconds)
    }

    #[test]
    fn progress_changes_the_line_no_more_often_than_the_interval() {
        let mut jobs = JobTable::new();
        let job = JobId::new(5);
        jobs.begin(job, JobKind::Copy, "Copy started: 3 items", at(0));
        let mut progress = JobProgress::start(JobKind::Copy, 100, None);
        progress.state = JobState::Running;
        let mut changes = Vec::new();
        for second in 1..=30 {
            progress.items_done = second * 3;
            if let Some((line, _)) = jobs.progress(job, &progress, at(second as i64), 10_000) {
                changes.push((second, line));
            }
        }
        // The first update is a change of state; then one every ten
        // seconds.
        let seconds: Vec<u64> = changes.iter().map(|(s, _)| *s).collect();
        assert_eq!(seconds, [10, 20, 30], "{changes:?}");
        progress.state = JobState::Paused;
        let (line, changed) = jobs.progress(job, &progress, at(31), 10_000).unwrap();
        assert!(changed);
        assert_eq!(line, "Copying paused at 90 percent");
        assert!(jobs.detail(job).unwrap().contains("90 of 100 items"));
    }

    #[test]
    fn finished_jobs_are_kept_for_the_history() {
        let mut jobs = JobTable::new();
        jobs.begin(JobId::new(1), JobKind::Trash, "Trash started", at(0));
        jobs.finished(FinishedJob {
            job: JobId::new(1),
            kind: JobKind::Trash,
            summary: "Move to the trash finished: 1 done.".into(),
            ..FinishedJob::default()
        });
        assert!(jobs.is_empty());
        assert_eq!(jobs.history().len(), 1);
        assert_eq!(millis_between(at(0), at(2)), 2000);
        assert_eq!(millis_between(at(2), at(0)), 0);
    }

    fn remote(path: &str) -> Place {
        Place::parse(&format!("sftp://build{path}")).unwrap()
    }

    /// Three copies to `build`: one runs at 40 percent, two wait their
    /// turn, the third first in line.
    fn queue() -> JobTable {
        let mut jobs = JobTable::new();
        let build = remote("/srv/build");
        let sources: Vec<Place> = (0..12).map(|n| remote(&format!("/a/{n}.txt"))).collect();
        for (n, started) in [(1, true), (2, false), (3, false)] {
            let facts = JobFacts::of(&sources, Some(&build));
            jobs.submit(
                JobId::new(n),
                JobKind::Copy,
                format!("Copy started: 12 items to build ({n})"),
                facts,
                started,
                at(0),
            );
        }
        let mut progress = JobProgress::start(JobKind::Copy, 12, Some(1000));
        progress.state = JobState::Running;
        progress.bytes_done = 400;
        jobs.progress(JobId::new(1), &progress, at(11), 10_000);
        assert_eq!(
            jobs.queue_note(&QueueNote::Waits(JobId::new(2))).as_deref(),
            Some("Queued: copy 12 items to build")
        );
        jobs.queue_note(&QueueNote::Waits(JobId::new(3)));
        jobs.queue_note(&QueueNote::View {
            waiting: vec![JobId::new(3), JobId::new(2)],
            paused: false,
        });
        jobs
    }

    /// Golden rows and lines at 40 cells (R25, 2.2): the state first.
    #[test]
    fn the_rows_and_the_line_say_the_state_first_in_40_cells() {
        let mut jobs = queue();
        assert_eq!(jobs.line(), "3 jobs, 2 waiting: Copying 40 percent");
        let rows = jobs.rows();
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Copying 40 percent: 12 items to build",
                "Waiting 1 of 2: copy 12 items to build",
                "Waiting 2 of 2: copy 12 items to build",
            ]
        );
        assert_eq!(rows[1].job, JobId::new(3), "the queue's order");
        assert_eq!(JobGroup::Running.heading(2), "Running, 2 jobs");
        assert_eq!(JobGroup::Waiting.heading(1), "Waiting, 1 job");
        assert_eq!(JobGroup::Finished.heading(5), "Finished, 5 jobs");

        let mut progress = JobProgress::start(JobKind::Copy, 12, Some(1000));
        progress.bytes_done = 300;
        progress.state = JobState::Paused;
        jobs.progress(JobId::new(1), &progress, at(12), 10_000);
        assert_eq!(
            jobs.row_of(JobId::new(1)).unwrap(),
            "Paused at 30 percent: copy 12 items to build"
        );
        progress.state = JobState::WaitingForConnection;
        progress.waits_for = Some("build".into());
        jobs.progress(JobId::new(1), &progress, at(13), 10_000);
        assert_eq!(
            jobs.row_of(JobId::new(1)).unwrap(),
            "Waiting for build: copy at 30 percent"
        );

        // The queue lets job 3 start: its start line is said then.
        assert_eq!(
            jobs.queue_note(&QueueNote::Started(JobId::new(3)))
                .as_deref(),
            Some("Copy started: 12 items to build (3)")
        );
        assert_eq!(jobs.waiting_count(), 1);

        jobs.finished(FinishedJob {
            job: JobId::new(1),
            kind: JobKind::Copy,
            summary: "Copy had 2 failures: 10 done.".into(),
            failed: vec!["a: gone".into(), "b: gone".into()],
            done: 10,
            ..FinishedJob::default()
        });
        let done = jobs.finished_job(JobId::new(1)).unwrap().row();
        assert_eq!(done, "Failed: copy 12 items to build, 2 items not done");
        let clean = FinishedJob {
            kind: JobKind::Copy,
            what: "12 items to build".into(),
            ..FinishedJob::default()
        };
        assert_eq!(clean.row(), "Done: copy 12 items to build");
        let cancelled = FinishedJob {
            kind: JobKind::Copy,
            what: "report.iso to build".into(),
            cancelled: true,
            ..FinishedJob::default()
        };
        assert_eq!(
            cancelled.row(),
            "Cancelled: copy report.iso to build, 0 done"
        );
        for row in jobs.rows() {
            let head = row.text.split(':').next().unwrap();
            assert!(within_cells(&row.text, head), "{}", row.text);
        }
    }

    #[test]
    fn the_front_job_is_the_newest_started_else_the_first_waiting() {
        let mut jobs = queue();
        assert_eq!(jobs.front(), Some(JobId::new(1)));
        jobs.finish(JobId::new(1));
        assert_eq!(jobs.front(), Some(JobId::new(3)), "the first in line");
        assert_eq!(jobs.line(), "2 jobs, 2 waiting: Copying waits its turn");
    }

    #[test]
    fn an_object_names_one_item_or_counts_them() {
        let build = remote("/srv/build");
        assert_eq!(
            job_object(&[remote("/a/report.iso")], Some(&build)),
            "report.iso to build"
        );
        assert_eq!(
            job_object(&[remote("/a/x"), remote("/a/y"), remote("/a/z")], None),
            "3 items"
        );
        assert_eq!(job_object(&[], Some(&build)), "to build");
        assert_eq!(line_verb(JobKind::Trash), "Trash");
    }
}
