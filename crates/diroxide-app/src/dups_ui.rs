//! Duplicates, their types in the app (alpha 5, lane DA): the scope, the
//! scan, the groups and the keep rules, over the engine half in
//! `diroxide_ops::dups` (its default hasher `Blake3Hasher`).
//!
//! Reserved by the pre-wiring step (lane P5): [`DupsAsk`] travels as
//! [`Request::Duplicates`](crate::Request::Duplicates) to the runtime,
//! which answers in `runtime/duplicates.rs`, and [`DupsNote`] comes back
//! as [`AppEvent::Duplicates`](crate::AppEvent::Duplicates), handled in
//! `app/duplicates.rs`. Both are empty until lane DA adds its variants
//! here; no shared file changes for them. The job kind is
//! `JobKind::FindDuplicates`.

/// What the app asks the runtime for a duplicates scan. Lane DA adds the
/// variants.
#[derive(Clone, Debug, PartialEq)]
pub enum DupsAsk {}

/// What the runtime answers a [`DupsAsk`]. Lane DA adds the variants.
#[derive(Debug)]
pub enum DupsNote {}
