//! The job engine of diroxide.
//!
//! Every operation that touches more than one entry, or can take more than
//! a moment, is a job: a [`Plan`] computed first and shown on request, a
//! run that reports [`JobProgress`] in words, a conflict policy asked once,
//! cancel and pause through [`JobControl`], a [`JobReport`] of what was
//! done, skipped and failed, and an [`UndoLog`] for renames, moves and the
//! trash. Nothing here runs a shell.
//!
//! Phase 0: the types are the contract; planning does not walk folders and
//! running copies single files only. Wave 1 (W1b) replaces the bodies,
//! adds the fake provider and the property test that a plan equals what
//! ran.

pub mod engine;
pub mod error;
pub mod plan;
pub mod progress;
pub mod undo;

pub use engine::{Conflict, Engine, JobControl, JobReport, JobSink};
pub use error::OpsError;
pub use plan::{Plan, Step};
pub use progress::JobProgress;
pub use undo::{UndoLog, UndoRecord};
