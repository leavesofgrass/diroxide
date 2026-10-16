//! The jobs line: what is running, in words, and the jobs that ran.
//!
//! The rules (the owner's decision of Saturday, October 10, 2026): a
//! change of state (started, paused, stalled, waiting for an answer,
//! verifying, finished) changes the line at once; other progress changes
//! it at most every `progress_interval_seconds` in screen-reader mode and
//! twice a second otherwise; the final message is said once. The line is
//! the engine's own sentence, percent first, its estimate inside 40 cells
//! ([`JobProgress::describe`]); Say Status adds the detail line on demand.
//!
//! Wave 1 (W1d) owns this module.

use diroxide_core::{JobId, JobKind, JobState, Timestamp};
use diroxide_ops::JobProgress;

/// How long the line keeps a progress sentence before the next, in
/// milliseconds, outside screen-reader mode.
pub const SIGHTED_INTERVAL_MS: u64 = 500;

/// A job that ran, for the job history list.
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Running {
    job: JobId,
    kind: Option<JobKind>,
    shown: String,
    state: Option<JobState>,
    shown_at: Option<Timestamp>,
    latest: Option<JobProgress>,
}

/// The running jobs, their latest progress sentences, and the jobs that
/// ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobTable {
    running: Vec<Running>,
    history: Vec<FinishedJob>,
}

/// The milliseconds from `from` to `to`, zero when `to` is earlier.
pub fn millis_between(from: Timestamp, to: Timestamp) -> u64 {
    let seconds = to.unix_seconds() - from.unix_seconds();
    let nanos = i64::from(to.subsec_nanos()) - i64::from(from.subsec_nanos());
    let total = seconds.saturating_mul(1000) + nanos / 1_000_000;
    u64::try_from(total).unwrap_or(0)
}

impl JobTable {
    /// No jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes a job that started.
    pub fn start(&mut self, job: JobId, text: impl Into<String>) {
        self.running.push(Running {
            job,
            kind: None,
            shown: text.into(),
            state: None,
            shown_at: None,
            latest: None,
        });
    }

    /// Notes a job of a kind that started, with its first line, at a time.
    pub fn begin(&mut self, job: JobId, kind: JobKind, text: impl Into<String>, now: Timestamp) {
        self.running.retain(|r| r.job != job);
        self.running.push(Running {
            job,
            kind: Some(kind),
            shown: text.into(),
            state: Some(JobState::Running),
            shown_at: Some(now),
            latest: None,
        });
    }

    /// Updates a job's progress sentence.
    pub fn update(&mut self, job: JobId, text: impl Into<String>) {
        if let Some(entry) = self.running.iter_mut().find(|r| r.job == job) {
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
            self.running.push(Running {
                job,
                kind: Some(progress.kind),
                shown: String::new(),
                state: None,
                shown_at: None,
                latest: None,
            });
        }
        let entry = self.running.iter_mut().find(|r| r.job == job)?;
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
    }

    /// Removes a finished job and keeps it in the history.
    pub fn finished(&mut self, done: FinishedJob) {
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

    /// The running jobs, oldest first.
    pub fn ids(&self) -> Vec<JobId> {
        self.running.iter().map(|r| r.job).collect()
    }

    /// The newest running job: the job in front, which Cancel stops.
    pub fn newest(&self) -> Option<JobId> {
        self.running.last().map(|r| r.job)
    }

    /// A running job's kind, when known.
    pub fn kind_of(&self, job: JobId) -> Option<JobKind> {
        self.running
            .iter()
            .find(|r| r.job == job)
            .and_then(|r| r.kind)
    }

    /// How many jobs are running.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    /// Whether nothing is running.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// The jobs line: "No jobs", the one job's sentence, or "2 jobs:
    /// Copying 12 of 300; Moving to the trash 1 of 3".
    pub fn line(&self) -> String {
        match self.running.as_slice() {
            [] => "No jobs".to_owned(),
            [only] => only.shown.clone(),
            many => {
                let texts: Vec<&str> = many.iter().map(|r| r.shown.as_str()).collect();
                format!("{} jobs: {}", many.len(), texts.join("; "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::UtcOffset;

    #[test]
    fn the_jobs_line_says_what_runs() {
        let mut jobs = JobTable::new();
        assert_eq!(jobs.line(), "No jobs");
        jobs.start(JobId::new(1), "Copying 0 of 3");
        jobs.update(JobId::new(1), "Copying 1 of 3");
        assert_eq!(jobs.line(), "Copying 1 of 3");
        jobs.start(JobId::new(2), "Moving 0 of 1");
        assert_eq!(jobs.line(), "2 jobs: Copying 1 of 3; Moving 0 of 1");
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
            failed: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
        });
        assert!(jobs.is_empty());
        assert_eq!(jobs.history().len(), 1);
        assert_eq!(millis_between(at(0), at(2)), 2000);
        assert_eq!(millis_between(at(2), at(0)), 0);
    }
}
