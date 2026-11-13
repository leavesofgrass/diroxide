//! Batch rename's types in the app (alpha 5, lane RA): the rules list,
//! the preview, apply and the editor door, over the engine half in
//! `diroxide_ops::rename_rules`.
//!
//! Reserved by the pre-wiring step (lane P5): [`RenameAsk`] travels as
//! [`Request::RenameBatch`](crate::Request::RenameBatch) to the runtime,
//! which answers in `runtime/renaming.rs`, and [`RenameNote`] comes back
//! as [`AppEvent::Renaming`](crate::AppEvent::Renaming), handled in
//! `app/renaming.rs`. Both are empty until lane RA adds its variants
//! here; no shared file changes for them.

/// What the app asks the runtime for a batch rename: reading the
/// folders' facts, the editor buffer, the presets. Lane RA adds the
/// variants.
#[derive(Clone, Debug, PartialEq)]
pub enum RenameAsk {}

/// What the runtime answers a [`RenameAsk`]. Lane RA adds the variants.
#[derive(Debug)]
pub enum RenameNote {}
