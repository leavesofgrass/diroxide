//! Running a synchronize plan: one job of the engine, one undo record.
//!
//! Before the job starts, every place the plan writes over or deletes is
//! read again: an entry changed since the compare, or a name that is now
//! taken where the compare saw nothing, is skipped with its reason, never
//! overwritten. The engine then runs the steps with its own progress,
//! conflicts, errors, cancel and undo; a replace always sends the old file
//! to the trash first where the place has one.
//!
//! Hooks, as copy has them: the `before_*` hooks are asked after the plan
//! and before the confirmation ([`SyncPlan::before_hooks`]), the copies as
//! a copy and the deletes as a delete, so a plugin that guards deletes is
//! asked about a mirror's deletes; the `after_*` hooks hear the report
//! split the same way. The engine does both for every
//! [`JobKind::Synchronize`](diroxide_core::JobKind::Synchronize) plan
//! ([`crate::hooks::HookCall::all`], [`split_report`]).

use diroxide_core::{Entry, EntryKind, JobId, Place};
use diroxide_vfs::{OpContext, VfsError};

pub use crate::hooks::split_report;

use crate::engine::{Engine, JobControl, JobReport, JobSink};
use crate::error::OpsError;
use crate::hooks::{self, HookCall};
use crate::plan::{Plan, Step};
use crate::sync::plan::{SyncAction, SyncPlan};

/// The reason a step is skipped when its entry changed after the compare.
pub const CHANGED_SINCE: &str = "changed since the compare; compare again";
/// The reason a copy is skipped when its target name was taken after the
/// compare.
pub const TAKEN_SINCE: &str = "the name was taken since the compare; compare again";
/// The reason a delete is skipped when its entry is already gone.
pub const ALREADY_GONE: &str = "already gone";

/// What a synchronize runs with.
pub struct SyncEnv<'a> {
    /// The engine: its registry, trash, journal and options.
    pub engine: &'a Engine,
    /// The job's id.
    pub job: JobId,
    /// Cancel and pause.
    pub control: &'a JobControl,
    /// Progress, conflicts and errors.
    pub sink: &'a dyn JobSink,
    /// Whether the app already asked the `before_*` hooks
    /// ([`SyncPlan::before_hooks`]) before its confirmation. When not,
    /// the run asks them first.
    pub hooks_asked: bool,
}

impl std::fmt::Debug for SyncEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncEnv")
            .field("job", &self.job)
            .field("hooks_asked", &self.hooks_asked)
            .finish_non_exhaustive()
    }
}

impl SyncPlan {
    /// The plans the hooks hear, one per event: the copies as a copy, the
    /// deletes as a delete (to the trash, or for good).
    pub fn hook_plans(&self, job: JobId) -> Result<Vec<Plan>, OpsError> {
        Ok(hooks::hook_parts(&self.job_plan(job)?))
    }

    /// The calls the `before_*` hooks hear: a copy call for the copies, a
    /// delete call for the deletes.
    pub fn hook_calls(&self, job: JobId) -> Result<Vec<HookCall>, OpsError> {
        Ok(HookCall::all(&self.job_plan(job)?))
    }

    /// Asks the `before_*` hooks of the plan, after the plan and before
    /// the confirmation, as a copy's planning does: the copies as a copy,
    /// then the deletes as a delete. Returns the lines to say, or
    /// [`OpsError::HookStopped`] with the line that stopped it.
    pub fn before_hooks(
        &self,
        engine: &Engine,
        job: JobId,
        control: &JobControl,
    ) -> Result<Vec<String>, OpsError> {
        engine.before_hooks(&self.job_plan(job)?, control)
    }
}

/// Runs a synchronize plan as one job: the places it writes over or
/// deletes read again first, then the engine's run. The report has one
/// undo record for the whole job; the steps skipped by the check are in
/// its skipped list with their reasons.
///
/// Refused with [`OpsError::NoChange`] when the plan has nothing to do,
/// and with [`OpsError::HookStopped`] when a hook stops it.
pub fn run_sync(plan: &SyncPlan, env: &SyncEnv<'_>) -> Result<JobReport, OpsError> {
    let steps = plan.steps()?;
    if steps.is_empty() {
        return Err(OpsError::NoChange);
    }
    let mut said = Vec::new();
    if !env.hooks_asked {
        said = plan.before_hooks(env.engine, env.job, env.control)?;
    }
    let ctx = OpContext::new().with_cancel(env.control.cancel.clone());
    let mut kept = Vec::with_capacity(steps.len());
    let mut skipped = Vec::new();
    for (index, step) in steps {
        if env.control.cancel.is_cancelled() {
            return Err(VfsError::Cancelled.into());
        }
        match recheck(plan, env.engine, index, &step, &ctx) {
            Ok(()) => kept.push((index, step)),
            Err(why) => skipped.push((step.subject().clone(), why)),
        }
    }
    let job_plan = plan.plan_of(&kept, env.job);
    // A synchronize never replaces without the trash where there is one.
    let engine = if env.engine.options().replace_keeps_old_in_trash {
        env.engine.clone()
    } else {
        let mut options = env.engine.options().clone();
        options.replace_keeps_old_in_trash = true;
        env.engine.clone().with_options(options)
    };
    let mut report = engine.run(&job_plan, env.control, env.sink);
    report.skipped.splice(0..0, skipped);
    report
        .warnings
        .extend(said.into_iter().map(|line| (plan.left.clone(), line)));
    Ok(report)
}

/// Reads a step's places again and compares them with the compare's
/// entries. `Err` with the reason to skip it.
fn recheck(
    plan: &SyncPlan,
    engine: &Engine,
    index: usize,
    step: &Step,
    ctx: &OpContext,
) -> Result<(), String> {
    let row = &plan.rows[index];
    let stat = |place: &Place| -> Result<Option<Entry>, String> {
        let provider = engine
            .registry()
            .resolve(place)
            .map_err(|error| error.to_string())?;
        match provider.stat(place, ctx) {
            Ok(entry) => Ok(Some(entry)),
            Err(VfsError::NotFound) => Ok(None),
            Err(VfsError::Cancelled) => Err(VfsError::Cancelled.to_string()),
            // Anything else is the engine's to retry or ask about.
            Err(_) => Err(String::new()),
        }
    };
    let quiet = |result: Result<Option<Entry>, String>| match result {
        Err(why) if why.is_empty() => Ok(None),
        other => other,
    };
    match (step, row.action) {
        (Step::Copy { from, to }, SyncAction::CopyTo(side)) => {
            let source = row.row.entry(side.other());
            if let (Some(seen), Some(now)) = (source, quiet(stat(from))?)
                && changed(seen, &now)
            {
                return Err(CHANGED_SINCE.to_owned());
            }
            let now = quiet(stat(to))?;
            match (row.row.entry(side), now) {
                (None, Some(now)) if !(now.kind == EntryKind::Folder && row.row.is_folder()) => {
                    Err(TAKEN_SINCE.to_owned())
                }
                (Some(seen), Some(now)) if changed(seen, &now) => Err(CHANGED_SINCE.to_owned()),
                _ => Ok(()),
            }
        }
        (Step::Trash { place } | Step::Remove { place }, SyncAction::RemoveFrom(side)) => {
            let Some(now) = quiet(stat(place))? else {
                return Err(ALREADY_GONE.to_owned());
            };
            match row.row.entry(side) {
                Some(seen) if changed(seen, &now) => Err(CHANGED_SINCE.to_owned()),
                _ => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// Whether an entry is not what the compare saw: another kind, or for a
/// file another size or modified time.
fn changed(seen: &Entry, now: &Entry) -> bool {
    seen.kind != now.kind
        || (seen.kind != EntryKind::Folder
            && ((seen.size.is_some() && seen.size != now.size)
                || (seen.modified.is_some() && seen.modified != now.modified)))
}
