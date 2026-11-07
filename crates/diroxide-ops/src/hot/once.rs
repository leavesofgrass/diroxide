//! `diro watch --task NAME --once` and the dry run (R16 3.7 and 3.8; R28
//! 6.3; the owner's answers for the family's exit codes): the catch-up
//! scan, then the queue, then the end, so a scheduler gets catch-up for
//! free; and the same look at `in` with nothing touched.
//!
//! The exit codes are the family's: 0 when everything waiting was done, 1
//! when the task could not run (not approved, invalid, locked, its tool
//! missing, its folder missing), 2 for wrong use (the command line's own),
//! 3 when it was partly done (a file failed, or was left for the next
//! run). R16 had 2 for "some failed"; the family's 3 replaces it.

use std::time::Duration;

use diroxide_core::sort::{NameCase, compare_names};
use diroxide_core::{Clock, Place};
use diroxide_vfs::OpContext;

use super::ledger::{Identity, Outcome};
use super::lines::{dry_run_line, summary_line, would_line};
use super::runner::{FileOutcome, HotNote, StartError, TaskRunner};
use super::task::{OnlineOnly, TaskAction};
use crate::tracker::{Facts, is_ignored};

/// Everything waiting was done.
pub const EXIT_DONE: i32 = 0;
/// The task could not run.
pub const EXIT_COULD_NOT_RUN: i32 = 1;
/// Wrong use of the command (the command line's own).
pub const EXIT_WRONG_USE: i32 = 2;
/// Partly done: a file failed or was left for the next run.
pub const EXIT_PARTLY: i32 = 3;

/// What `--once` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnceReport {
    /// The task's title.
    pub title: String,
    /// Files handled.
    pub done: usize,
    /// Files filed in `failed`.
    pub failed: usize,
    /// Files passed over.
    pub skipped: usize,
    /// Files left in `in` for the next run.
    pub left: usize,
    /// Why it could not run, or paused part way: the line.
    pub stopped: Option<String>,
    /// Every file's line, in order, for standard output; the progress
    /// words go to standard error.
    pub lines: Vec<String>,
}

impl OnceReport {
    /// The exit code. See the module documentation.
    pub fn exit_code(&self) -> i32 {
        match &self.stopped {
            Some(_) if self.done == 0 && self.failed == 0 => EXIT_COULD_NOT_RUN,
            Some(_) => EXIT_PARTLY,
            None if self.failed > 0 || self.left > 0 => EXIT_PARTLY,
            None => EXIT_DONE,
        }
    }

    /// The last line: "Scans: 3 done, 1 failed", or the reason it stopped.
    pub fn summary(&self) -> String {
        match &self.stopped {
            Some(line) if self.done == 0 && self.failed == 0 => line.clone(),
            _ => summary_line(&self.title, self.done, self.failed, self.skipped),
        }
    }
}

/// Runs a task once: starts it catching up on everything in `in` (the
/// owner's catch-up answer does not apply: `--once` is the catch-up),
/// waits on `clock` with `sleep` while files settle, runs the queue, and
/// ends when nothing waits. A retry that would wait is left for the next
/// run. `sleep` is the real wait in `diro watch`, a step of a fixed clock
/// in tests.
pub fn run_once(
    runner: TaskRunner,
    clock: &dyn Clock,
    sleep: &mut dyn FnMut(Duration),
) -> OnceReport {
    let mut runner = runner.once();
    let mut report = OnceReport {
        title: runner.title().to_owned(),
        ..OnceReport::default()
    };
    let started = runner.start_with(clock.now(), Some(super::task::CatchUpChoice::All));
    let notes = match started {
        Ok(notes) => notes,
        Err(error) => {
            report.stopped = Some(start_line(&error));
            return report;
        }
    };
    take(&mut report, notes);
    while report.stopped.is_none() && !runner.is_stopped() {
        if runner.is_idle() {
            break;
        }
        match runner.next_wake(clock.now()) {
            Some(wait) if !wait.is_zero() => sleep(wait),
            Some(_) => {}
            None => break,
        }
        let notes = runner.wake(clock.now());
        take(&mut report, notes);
    }
    report
}

/// The line a refused start says.
pub fn start_line(error: &StartError) -> String {
    error.to_string()
}

fn take(report: &mut OnceReport, notes: Vec<HotNote>) {
    for note in notes {
        match note {
            HotNote::File(file) => {
                match file.outcome {
                    FileOutcome::Done | FileOutcome::Told => report.done += 1,
                    FileOutcome::Failed => report.failed += 1,
                    FileOutcome::Skipped => report.skipped += 1,
                    // A file that never settled stays in `in` too.
                    FileOutcome::Left | FileOutcome::GaveUp => report.left += 1,
                    FileOutcome::Retrying | FileOutcome::Changed => {}
                }
                report.lines.push(file.line);
            }
            HotNote::Paused(line) => report.stopped = Some(line),
            _ => {}
        }
    }
}

/// What a dry run found: a line a file, and the summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DryRun {
    /// One line a file in `in`, in natural name order: "Would convert
    /// scan-004.pdf", "Skipped scan-005.pdf: unfinished name".
    pub lines: Vec<String>,
    /// How many would run.
    pub would: usize,
    /// How many would not.
    pub skipped: usize,
    /// "Dry run: 3 would convert, 1 skipped".
    pub summary: String,
}

/// The verb a dry run says for a task's action: "convert", "move".
pub fn verb(action: &TaskAction) -> &'static str {
    match action {
        TaskAction::Convert { .. } => "convert",
        TaskAction::Move => "move",
        TaskAction::Copy => "copy",
        TaskAction::Script { .. } | TaskAction::Plugin { .. } => "run",
        TaskAction::Tell => "say",
    }
}

impl TaskRunner {
    /// "Preview a rule": what is in `in` now and what the task would do
    /// with each file, or why not; nothing is touched, nothing is written
    /// to the ledger or the log, and no lock is taken.
    pub fn preview(&self) -> Result<DryRun, String> {
        let task = self.task();
        let input = &task.folders.input;
        let provider = self
            .engine()
            .registry()
            .resolve(input)
            .map_err(|e| e.to_string())?;
        let mut entries = provider
            .list(input, &OpContext::new())
            .map_err(|e| format!("in: {e}"))?;
        entries.sort_by(|a, b| compare_names(&a.name, &b.name, NameCase::Ignore, true));
        let verb = verb(&task.action);
        let ledger = self.store().ledger().map_err(|e| e.to_string())?;
        let mut dry = DryRun::default();
        for entry in entries {
            let name = entry.name.clone();
            let mut facts = Facts::from_entry(&entry);
            if task.online_only == OnlineOnly::Fetch {
                facts.online_only = false;
            }
            let why: Option<String> = if is_ignored(&name) {
                Some(if name.starts_with('.') {
                    "hidden or unfinished name".to_owned()
                } else {
                    "unfinished name".to_owned()
                })
            } else if let Some(reason) = facts.skip_reason() {
                Some(reason.words().to_owned())
            } else if !task.matches_name(&name) {
                Some("not a match".to_owned())
            } else if let Some(filter) = &task.filter {
                let place = input.join(&name).unwrap_or_else(|_| input.clone());
                match self.filter_passes(filter, &place, &entry) {
                    Ok(true) => None,
                    Ok(false) => Some("filtered out".to_owned()),
                    Err(why) => Some(format!("the filter cannot be read: {why}")),
                }
            } else {
                None
            };
            let why = why.or_else(|| {
                ledger
                    .handled(&name, &Identity::of(&entry))
                    .filter(|line| line.outcome != Outcome::Made)
                    .map(|_| "already handled".to_owned())
            });
            match why {
                Some(why) => {
                    dry.skipped += 1;
                    dry.lines.push(super::lines::skipped_line(&name, &why));
                }
                None => {
                    dry.would += 1;
                    let finish = ledger
                        .handled(&name, &Identity::of(&entry))
                        .is_some_and(|line| line.outcome == Outcome::Made);
                    dry.lines.push(if finish {
                        would_line("finish moving", &name)
                    } else {
                        would_line(verb, &name)
                    });
                }
            }
        }
        dry.summary = dry_run_line(verb, dry.would, dry.skipped);
        Ok(dry)
    }

    fn filter_passes(
        &self,
        filter: &str,
        place: &Place,
        entry: &diroxide_core::Entry,
    ) -> Result<bool, String> {
        self.actions().passes(filter, place, entry)
    }
}
