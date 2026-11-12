//! Compare two folders, and synchronize them from the compare (R2,
//! "Compare and synchronize"; R18, "The status vocabulary"; plan section
//! 5.17; the feature catalog's alpha.8).
//!
//! Compare produces a result, synchronize turns the result into a plan,
//! and the job engine runs the plan:
//!
//! 1. [`compare_dirs`] walks two places on any two providers, the place
//!    here (this panel) first, and gives a [`Comparison`]: one
//!    [`CompareRow`] per relative path with its [`RowState`] in R18's
//!    words (Same, Different with a reason, Only here, Only there, Type
//!    differs, Unreadable, Skipped, Not compared) and a count line first
//!    that adds up ("17 differences in 1,202 items: 12 only here, 3 only
//!    there, 2 different. 1,185 same."). By size and time within two
//!    seconds, said in a row when it decided, or by contents block by
//!    block on request. Cancellable, with progress ([`CompareSink`]).
//! 2. [`plan_sync`] turns the result and a [`SyncMode`] into a
//!    [`SyncPlan`]: one [`SyncAction`] per row, which the owner changes
//!    per row ([`SyncPlan::toggle`], [`SyncPlan::set_direction`]) and
//!    reads as plain lines, problems first, then the removals
//!    ([`SyncPlan::lines`]).
//! 3. [`run_sync`] runs the plan as one job of the engine with one undo
//!    record: the places it writes over or removes are read again first,
//!    the engine checks each step's entry against the compare's
//!    ([`Plan::expected`](crate::plan::Plan::expected)), a replace sends
//!    the old file to the trash first where the place has one, and a
//!    removal goes to the trash or, where there is none, says "for good"
//!    in its line.
//!
//! Rows say "here" and "there", sentences "this panel" and "the other
//! panel", never left and right (R2). Colors and arrows never carry
//! meaning: every status and action is a word at the start of its line.
//!
//! Hooks: a compare reads and writes nothing, so it emits nothing. A
//! synchronize asks the `before_copy` hooks about its copies and the
//! `before_delete` hooks about its removals ([`SyncPlan::before_hooks`]),
//! so a plugin that guards deletes also guards a mirror, and the report is
//! split the same way for the `after_*` hooks ([`split_report`]). No new
//! event names: the job is a `JobKind::Synchronize`, and the engine asks
//! and tells its hooks by the split ([`crate::hooks::HookCall::all`]).
//!
//! Out of this alpha: an rsync engine, Save as script, saved tasks that
//! synchronize, and an archive compare by its stored CRC-32.

mod compare;
mod job;
mod plan;
#[cfg(test)]
mod tests;

pub use compare::{
    CompareBy, CompareCounts, CompareCriteria, CompareProgress, CompareRow, CompareSink,
    Comparison, DEFAULT_MAX_DEPTH, EntryFilter, RowFact, RowState, SideFacts, TOLERANCE, at,
    compare_dirs, panel,
};
pub use job::{ALREADY_GONE, CHANGED_SINCE, SyncEnv, TAKEN_SINCE, run_sync, split_report};
pub use plan::{
    RowStatus, SyncAction, SyncCounts, SyncMode, SyncPlan, SyncProblem, SyncRow, plan_sync,
};
