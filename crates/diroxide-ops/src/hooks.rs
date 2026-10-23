//! Hooks in the engine (R25 commit 71; R3, section 9): a plugin may be
//! told before a copy, a move, a delete, a rename or a new folder, and
//! may stop it; and it is told after, with the report.
//!
//! - **`before_*`** runs after the plan and before the confirmation, so a
//!   veto spares the owner a question ([`crate::Engine::before_hooks`],
//!   called at the end of [`crate::Engine::plan_with`] when the engine has
//!   a runner). Each
//!   hook that wakes is asked on a thread of its own, never the caller's,
//!   and waited for at most its manifest's timeout (2 seconds by default).
//!   Its answer stops the job with its reason, or lets it go on; silence
//!   or an error is what its `on_timeout` says, with a line either way,
//!   never a quiet veto: "Copy goes on: no answer (Folder guard)", "Copy
//!   stopped: no answer (Folder guard)". A hook cannot change the plan.
//! - **`after_*`** is sent once with the report when the run ends, on a
//!   thread of its own, never waited for.
//! - **Who wakes** is the runner's to say (its filters: the event, the
//!   schemes, a glob, a kind): the app implements [`HookRunner`] through
//!   the plugin host (lane A2), the plugin's name last in every line
//!   (answer 90).
//!
//! A queued job runs its `before` hooks when it is planned, at submission,
//! and its `after` hooks when it finishes.

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use diroxide_core::{EntryKind, JobId, JobKind, Place};
use diroxide_vfs::CancelToken;

use crate::engine::JobReport;
use crate::plan::Plan;

/// The most items a hook hears about; the count says how many in all.
pub const MAX_HOOK_ITEMS: usize = 1000;

/// The jobs a plugin can hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// A copy or a duplicate.
    Copy,
    /// A move.
    Move,
    /// A move to the trash, or a delete for good.
    Delete,
    /// A rename.
    Rename,
    /// A new folder.
    MakeFolder,
}

impl HookEvent {
    /// The event of a job's kind, when it has one.
    pub const fn of(kind: JobKind) -> Option<Self> {
        match kind {
            JobKind::Copy | JobKind::Duplicate => Some(HookEvent::Copy),
            JobKind::Move => Some(HookEvent::Move),
            JobKind::Trash | JobKind::DeletePermanently => Some(HookEvent::Delete),
            JobKind::Rename => Some(HookEvent::Rename),
            JobKind::MakeFolder => Some(HookEvent::MakeFolder),
            _ => None,
        }
    }

    /// The `before_*` event's name, as manifests write it.
    pub const fn before(self) -> &'static str {
        match self {
            HookEvent::Copy => "before_copy",
            HookEvent::Move => "before_move",
            HookEvent::Delete => "before_delete",
            HookEvent::Rename => "before_rename",
            HookEvent::MakeFolder => "before_make_folder",
        }
    }

    /// The `after_*` event's name.
    pub const fn after(self) -> &'static str {
        match self {
            HookEvent::Copy => "after_copy",
            HookEvent::Move => "after_move",
            HookEvent::Delete => "after_delete",
            HookEvent::Rename => "after_rename",
            HookEvent::MakeFolder => "after_make_folder",
        }
    }

    /// The verb a hook's line starts with: "Delete stopped: ...".
    pub const fn verb(self) -> &'static str {
        match self {
            HookEvent::Copy => "Copy",
            HookEvent::Move => "Move",
            HookEvent::Delete => "Delete",
            HookEvent::Rename => "Rename",
            HookEvent::MakeFolder => "Make folder",
        }
    }
}

/// What became of an item, for `after_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HookResult {
    /// Done.
    Done,
    /// Skipped, with its reason in the report.
    Skipped,
    /// Failed, with its cause in the report.
    Failed,
}

impl HookResult {
    /// The word on the wire: `done`, `skipped`, `failed`.
    pub const fn word(self) -> &'static str {
        match self {
            HookResult::Done => "done",
            HookResult::Skipped => "skipped",
            HookResult::Failed => "failed",
        }
    }
}

/// One item a job touches, as a hook hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookItem {
    /// Where it is.
    pub from: Place,
    /// Where it goes, for a copy, a move or a rename.
    pub to: Option<Place>,
    /// Whether it is a folder.
    pub folder: bool,
    /// What became of it, in `after_*`.
    pub result: Option<HookResult>,
}

/// A planned job, as a `before_*` hook hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookCall {
    /// The event.
    pub event: HookEvent,
    /// The job.
    pub job: JobId,
    /// The job's kind.
    pub kind: JobKind,
    /// The items, at most [`MAX_HOOK_ITEMS`].
    pub items: Vec<HookItem>,
    /// How many items in all.
    pub count: u64,
    /// The plan in words: "Move 3 items to the trash".
    pub summary: String,
    /// The destination folder, for a copy or a move.
    pub destination: Option<Place>,
}

impl HookCall {
    /// The call for a plan, when its kind has an event.
    pub fn of(plan: &Plan) -> Option<Self> {
        let event = HookEvent::of(plan.kind)?;
        let items = plan
            .steps
            .iter()
            .take(MAX_HOOK_ITEMS)
            .map(|step| HookItem {
                from: step.subject().clone(),
                to: step.result().cloned(),
                folder: plan
                    .expected
                    .get(step.subject())
                    .is_some_and(|entry| entry.kind == EntryKind::Folder),
                result: None,
            })
            .collect();
        Some(Self {
            event,
            job: plan.job,
            kind: plan.kind,
            items,
            count: plan.entries(),
            summary: plan.summary(),
            destination: plan.facts.destination.clone(),
        })
    }
}

/// How a job ended, for `after_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HookStatus {
    /// Every item done or skipped.
    Done,
    /// Some item failed.
    Failed,
    /// Cancelled, or stopped before its end.
    Cancelled,
}

impl HookStatus {
    /// The word on the wire: `done`, `failed`, `cancelled`.
    pub const fn word(self) -> &'static str {
        match self {
            HookStatus::Done => "done",
            HookStatus::Failed => "failed",
            HookStatus::Cancelled => "cancelled",
        }
    }
}

/// A finished job, as an `after_*` hook hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookReport {
    /// The event.
    pub event: HookEvent,
    /// The job.
    pub job: JobId,
    /// How it ended.
    pub status: HookStatus,
    /// Its items with their results, at most [`MAX_HOOK_ITEMS`].
    pub items: Vec<HookItem>,
}

impl HookReport {
    /// The after-report of a job, when its kind has an event.
    pub fn of(report: &JobReport) -> Option<Self> {
        let event = HookEvent::of(report.kind)?;
        let status = if report.cancelled || report.stopped {
            HookStatus::Cancelled
        } else if report.failed.is_empty() {
            HookStatus::Done
        } else {
            HookStatus::Failed
        };
        let item = |place: &Place, result| HookItem {
            from: place.clone(),
            to: None,
            folder: false,
            result: Some(result),
        };
        let items = report
            .done
            .iter()
            .map(|place| item(place, HookResult::Done))
            .chain(
                report
                    .skipped
                    .iter()
                    .map(|(place, _)| item(place, HookResult::Skipped)),
            )
            .chain(
                report
                    .failed
                    .iter()
                    .map(|(place, _)| item(place, HookResult::Failed)),
            )
            .take(MAX_HOOK_ITEMS)
            .collect();
        Some(Self {
            event,
            job: report.job,
            status,
            items,
        })
    }
}

/// What a hook's silence or error means (its manifest's `on_timeout`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OnSilence {
    /// Go on, with a line: the default.
    #[default]
    Continue,
    /// Stop the job, with a line.
    Cancel,
}

/// A hook that wakes for a job: whose, and how long to wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakingHook {
    /// The plugin's id, for the runner.
    pub plugin: String,
    /// The plugin's name, last in every line.
    pub name: String,
    /// Which of the plugin's hooks, for the runner.
    pub index: usize,
    /// How long to wait for its answer.
    pub timeout: Duration,
    /// What its silence means.
    pub on_timeout: OnSilence,
}

/// What a `before_*` hook decided, each line ending with the plugin's
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookVerdict {
    /// Go on, saying nothing.
    Continue,
    /// Go on, and say this line (the plugin did not answer).
    ContinueWithWarning(String),
    /// Stop the job, saying this line: "Delete stopped: keep.txt is
    /// protected (Folder guard)".
    Cancel(String),
}

/// Runs the hooks: the app implements it through the plugin host. Every
/// method may block; the engine calls `before` on a thread of its own with
/// a time limit, and `after` on a thread it never waits for.
pub trait HookRunner: Send + Sync + fmt::Debug {
    /// The hooks that wake for this call's `before_*` event, by their
    /// filters, in the order to ask them.
    fn waking(&self, call: &HookCall) -> Vec<WakingHook>;

    /// Asks one hook; its silence or an error is the runner's
    /// [`HookVerdict::ContinueWithWarning`] or [`HookVerdict::Cancel`] by
    /// its `on_timeout`, as the plugin host decides.
    fn before(&self, hook: &WakingHook, call: &HookCall) -> HookVerdict;

    /// Tells the hooks of the `after_*` event what happened.
    fn after(&self, report: &HookReport);
}

/// The line for a hook that did not answer in time: "Copy goes on: no
/// answer (Folder guard)" or "Copy stopped: no answer (Folder guard)".
pub fn silent_line(event: HookEvent, stops: bool, name: &str) -> String {
    let what = if stops { "stopped" } else { "goes on" };
    format!("{} {what}: no answer ({name})", event.verb())
}

/// Asks every waking hook of a call in order, each on its own thread with
/// its time limit; stops at the first veto. Returns the warnings to say,
/// or the line that stopped the job. `None` when the job was cancelled
/// while a hook was asked.
pub(crate) fn ask_before(
    runner: &Arc<dyn HookRunner>,
    call: &HookCall,
    cancel: &CancelToken,
) -> Option<Result<Vec<String>, String>> {
    const SLICE: Duration = Duration::from_millis(20);
    let mut warnings = Vec::new();
    for hook in runner.waking(call) {
        let silence = || {
            let stops = hook.on_timeout == OnSilence::Cancel;
            let line = silent_line(call.event, stops, &hook.name);
            if stops {
                HookVerdict::Cancel(line)
            } else {
                HookVerdict::ContinueWithWarning(line)
            }
        };
        let (send, answer) = mpsc::channel();
        let asked = {
            let runner = Arc::clone(runner);
            let hook = hook.clone();
            let call = call.clone();
            // The hook may never answer: its thread is left to end on its
            // own, and the engine goes by the hook's time limit.
            thread::Builder::new()
                .name("diroxide-hook".to_owned())
                .spawn(move || {
                    let _ = send.send(runner.before(&hook, &call));
                })
        };
        let mut waited = Duration::ZERO;
        let verdict = loop {
            if cancel.is_cancelled() {
                return None;
            }
            if asked.is_err() {
                break silence();
            }
            match answer.recv_timeout(SLICE.min(hook.timeout.saturating_sub(waited))) {
                Ok(verdict) => break verdict,
                Err(mpsc::RecvTimeoutError::Timeout) if waited < hook.timeout => {
                    waited += SLICE;
                }
                Err(_) => break silence(),
            }
        };
        match verdict {
            HookVerdict::Continue => {}
            HookVerdict::ContinueWithWarning(line) => warnings.push(line),
            HookVerdict::Cancel(line) => return Some(Err(line)),
        }
    }
    Some(Ok(warnings))
}

/// Sends a finished job's `after_*` event on a thread of its own, never
/// waited for.
pub(crate) fn send_after(runner: &Arc<dyn HookRunner>, report: &JobReport) {
    let Some(after) = HookReport::of(report) else {
        return;
    };
    let runner = Arc::clone(runner);
    let _ = thread::Builder::new()
        .name("diroxide-hook-after".to_owned())
        .spawn(move || runner.after(&after));
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn events_and_lines_read_as_the_manifest_and_the_owner_expect() {
        assert_eq!(HookEvent::of(JobKind::Trash), Some(HookEvent::Delete));
        assert_eq!(
            HookEvent::of(JobKind::DeletePermanently),
            Some(HookEvent::Delete)
        );
        assert_eq!(HookEvent::of(JobKind::Duplicate), Some(HookEvent::Copy));
        assert_eq!(HookEvent::of(JobKind::Pack), None);
        assert_eq!(HookEvent::Delete.before(), "before_delete");
        assert_eq!(HookEvent::MakeFolder.after(), "after_make_folder");
        let line = silent_line(HookEvent::Copy, false, "Folder guard");
        assert_eq!(line, "Copy goes on: no answer (Folder guard)");
        assert_eq!(
            silent_line(HookEvent::Delete, true, "Folder guard"),
            "Delete stopped: no answer (Folder guard)"
        );
        assert!(within_cells(&line, "Copy goes on: no answer"));
        assert_eq!(HookStatus::Cancelled.word(), "cancelled");
        assert_eq!(HookResult::Skipped.word(), "skipped");
    }
}
