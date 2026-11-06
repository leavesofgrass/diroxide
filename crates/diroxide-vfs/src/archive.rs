//! What every reader of a container shares, whoever serves it: the
//! limits that keep a hostile archive or disk image from filling the
//! memory or the disk ([`limits`]), and the rules a member's name must
//! pass before it becomes a row ([`names`]).
//!
//! Lifted from `diroxide-archive` by alpha 4's contract step (P4, R29's
//! question 2), which re-exports both modules under their old paths, so a
//! plugin that reads containers (the disk-image plugin) takes them from
//! this crate without linking the job engine.

pub mod limits;
pub mod names;
