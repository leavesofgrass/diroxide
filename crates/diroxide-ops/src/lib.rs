//! The job engine of diroxide.
//!
//! Every operation that touches more than one entry, or can take more than
//! a moment, is a job: a [`Plan`] computed first and shown on request, a
//! run that reports [`JobProgress`] in words, conflicts answered one at a
//! time with apply-to-rest ([`Conflict`]), errors answered with Retry,
//! Skip, Skip all like this or Cancel ([`ItemError`]), cancel and pause
//! through [`JobControl`], a [`JobReport`] of what was done, skipped,
//! failed and warned about, with a [`Reconciliation`] against a fresh read,
//! and an [`UndoLog`] of moves, trashings and created entries. Nothing here
//! runs a shell.
//!
//! The order is create, verify, then destroy (plan section 5.4). The
//! [`fake`] provider has the latency, stall and "lies" switches the tests
//! of that order need.
//!
//! Phase 0 and the Wave 1 contract: the types are the contract; planning
//! does not walk folders and running copies single files only. Wave 1
//! (W1b) replaces the bodies and adds the property test that a plan equals
//! what ran.

pub mod engine;
pub mod error;
pub mod fake;
pub mod plan;
pub mod progress;
pub mod undo;

pub use engine::{
    Conflict, ConflictChoice, Engine, ItemError, JobControl, JobReport, JobSink, Reconciliation,
    numbered_name,
};
pub use error::OpsError;
pub use fake::{FakeProvider, FakeSwitches, Fault, Latency, Lies};
pub use plan::{Plan, Step};
pub use progress::JobProgress;
pub use undo::{UndoLog, UndoRecord};
