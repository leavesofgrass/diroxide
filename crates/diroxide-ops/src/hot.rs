//! The hot-folder engine's pure parts (alpha.7, R28 part 6): the task's
//! validation and refused folders, the ledger and the log, the runner
//! loop around the arrival tracker, catch-up, the dry run and the
//! summaries. Its actions (a conversion, a script command, a plugin
//! command) come in through a trait the app and the command line
//! implement, so this crate depends on nothing but core and vfs.
//!
//! Declared empty by the contract step (P4) so the hot-folder lane fills
//! it without editing this crate's `lib.rs`.
