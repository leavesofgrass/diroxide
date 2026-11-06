//! Conversions as jobs (alpha.7, R28 part 5): the planner's refusals and
//! skips, the staging name `.diro-part-` plus eight hexadecimal digits
//! that the arrival tracker already ignores by its leading dot, keep-both
//! names, and the report with its failures first. The tools run through
//! the `ToolRunner` of `diroxide-vfs` an operation's context
//! carries, so this crate never depends on the tools registry.
//!
//! Declared empty by the contract step (P4) so the conversion lane fills
//! it without editing this crate's `lib.rs`.
