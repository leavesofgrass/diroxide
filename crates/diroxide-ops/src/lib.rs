//! The job engine of diroxide.
//!
//! Every operation that touches more than one entry, or can take more than
//! a moment, is a job: a [`Plan`] computed first and shown on request, a
//! run that reports [`JobProgress`] in words, conflicts answered one at a
//! time with apply-to-rest ([`Conflict`]), errors answered with Retry,
//! Skip, Skip all like this or Cancel ([`ItemError`]), cancel and pause
//! through [`JobControl`], a [`JobReport`] of what was done, skipped,
//! failed and warned about, with a [`Reconciliation`] against a fresh read,
//! and an [`UndoLog`] of moves, trashings and created entries, kept on
//! disk by the [`Journal`]. Nothing here runs a shell.
//!
//! The transfer queue is the jobs list (R25, part 1): the [`Queue`] decides
//! when a confirmed job starts, by class and host; a lost host is one fact
//! for every job on it, behind its connection gate ([`Gates`]).
//!
//! The order is create, verify, then destroy (plan section 5.4). The
//! [`fake`] provider has the latency, stall and "lies" switches the tests
//! of that order need.
//!
//! The planner walks every source through its provider with the boundary
//! rule (links never followed, other drives and loops left out and said),
//! refuses the anchors, and asks its preflight questions ([`Preflight`])
//! before anything is touched.

pub mod access;
pub mod engine;
pub mod error;
pub mod fake;
pub mod gate;
#[cfg(test)]
mod gate_tests;
pub mod hooks;
#[cfg(test)]
mod hooks_tests;
pub mod join;
pub mod journal;
#[cfg(test)]
mod limit_tests;
pub mod limiter;
pub mod links;
pub mod options;
pub mod pace;
pub mod part_names;
pub mod parts;
pub mod plan;
mod planner;
pub mod progress;
pub mod queue;
pub mod resume;
#[cfg(test)]
mod resume_tests;
mod run;
#[cfg(test)]
mod run_tests;
pub mod sidecar;
pub mod split;
#[cfg(test)]
mod testing;
// The arrival tracker (W3t, plan commit 33).
pub mod tracker;
pub mod trash;
pub mod undo;
mod walk;

pub use access::{AttributeEdit, AttributeFacts, ModeChange, Recursion, WindowsChange};
pub use engine::{
    Conflict, ConflictChoice, Engine, ItemError, JobControl, JobReport, JobSink, Reconciliation,
    numbered_name,
};
pub use error::OpsError;
pub use fake::{
    CallFault, FakeCall, FakeProvider, FakeSwitches, FakeTrash, Fault, Latency, Lies, LinkDrop,
    SlowPlace,
};
pub use gate::{ConnectionAnswer, ConnectionQuestion, GateEvent, Gates, Waiter};
pub use hooks::{
    HookCall, HookEvent, HookItem, HookReport, HookResult, HookRunner, HookStatus, HookVerdict,
    OnSilence, WakingHook,
};
pub use join::{JoinFacts, JoinOutcome, Verdict};
pub use journal::{Interrupted, Journal};
pub use limiter::{LimitScope, Limiter, SpeedLimit};
pub use links::LinkForm;
pub use options::EngineOptions;
pub use pace::{FakePacer, Pacer, SystemPacer};
pub use part_names::{Discovery, PartSet, SetProblem};
pub use parts::{PartInfo, PartsError, PartsReader};
pub use plan::{ListingGroup, Plan, PlanFacts, Preflight, Step};
pub use progress::JobProgress;
pub use queue::{Cancelled, Grant, HostKey, JobClass, Queue, QueueLimits};
pub use resume::{JobNotice, Ledger, NotResumed, Partial};
pub use sidecar::{Sidecar, SidecarRead};
pub use split::{SplitFacts, SplitFile};
pub use undo::{LeftAs, Relinked, UndoLog, UndoRecord};
