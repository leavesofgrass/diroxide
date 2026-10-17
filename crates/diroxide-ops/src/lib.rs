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
//! The order is create, verify, then destroy (plan section 5.4). The
//! [`fake`] provider has the latency, stall and "lies" switches the tests
//! of that order need.
//!
//! The planner walks every source through its provider with the boundary
//! rule (links never followed, other drives and loops left out and said),
//! refuses the anchors, and asks its preflight questions ([`Preflight`])
//! before anything is touched.

pub mod engine;
pub mod error;
pub mod fake;
pub mod join;
pub mod journal;
pub mod links;
pub mod options;
pub mod pace;
pub mod part_names;
pub mod parts;
pub mod plan;
mod planner;
pub mod progress;
mod run;
#[cfg(test)]
mod run_tests;
pub mod sidecar;
pub mod split;
#[cfg(test)]
mod testing;
pub mod undo;
mod walk;

pub use engine::{
    Conflict, ConflictChoice, Engine, ItemError, JobControl, JobReport, JobSink, Reconciliation,
    numbered_name,
};
pub use error::OpsError;
pub use fake::{CallFault, FakeCall, FakeProvider, FakeSwitches, Fault, Latency, Lies, SlowPlace};
pub use join::{JoinFacts, JoinOutcome, Verdict};
pub use journal::{Interrupted, Journal};
pub use links::LinkForm;
pub use options::EngineOptions;
pub use pace::{FakePacer, Pacer, SystemPacer};
pub use part_names::{Discovery, PartSet, SetProblem};
pub use parts::{PartInfo, PartsError, PartsReader};
pub use plan::{ListingGroup, Plan, PlanFacts, Preflight, Step};
pub use progress::JobProgress;
pub use sidecar::{Sidecar, SidecarRead};
pub use split::{SplitFacts, SplitFile};
pub use undo::{LeftAs, UndoLog, UndoRecord};
