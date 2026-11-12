//! Compare folders and synchronize, their types in the app (alpha 5,
//! lane SA): the compare, the plan view and the run, over the engine
//! half in `diroxide_ops::sync`.
//!
//! Reserved by the pre-wiring step (lane P5): [`SyncAsk`] travels as
//! [`Request::Sync`](crate::Request::Sync) to the runtime, which answers
//! in `runtime/syncing.rs`, and [`SyncNote`] comes back as
//! [`AppEvent::Syncing`](crate::AppEvent::Syncing), handled in
//! `app/syncing.rs`. Both are empty until lane SA adds its variants here;
//! no shared file changes for them. The job kinds are
//! `JobKind::Compare` and `JobKind::Synchronize`.

/// What the app asks the runtime for a compare or a synchronize. Lane SA
/// adds the variants.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncAsk {}

/// What the runtime answers a [`SyncAsk`]. Lane SA adds the variants.
#[derive(Debug)]
pub enum SyncNote {}
