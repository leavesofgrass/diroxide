//! The virtual file system of diroxide.
//!
//! A [`Provider`] serves places of one kind: the local disk, archives, or
//! a plugin's protocol. Every operation takes an [`OpContext`] that
//! carries a [`CancelToken`], a [`ProgressSink`] and a [`SecretSource`].
//! The [`Registry`] resolves a place to its provider. The local provider
//! lives here, with its platform details behind the `sys` module, the one
//! module in the workspace allowed to use `unsafe` (reviewed at
//! integration).
//!
//! The safety rules of plan section 5.12 have their types here: the trash
//! gate and its post-check ([`trash`]), the depth limits of the
//! shell-bound operations ([`limits`]), the volume kind a place is on
//! ([`volume`]), and the name rules ([`names`]).
//!
//! Phase 0 and the Wave 1 contract: the trait and types are the contract;
//! the local provider's bodies are deliberately naive (`std::fs` only, no
//! attributes, no trash, no watching, no long-path handling). Wave 1 (W1a)
//! replaces them.

pub mod cancel;
pub mod capabilities;
pub mod error;
pub mod facts;
pub mod limits;
pub mod local;
pub mod names;
pub mod provider;
pub mod registry;
mod sys;
pub mod trash;
pub mod volume;

pub use cancel::{
    CancelToken, NoProgress, NoSecrets, OpContext, ProgressSink, ProgressUpdate, SecretSource,
};
pub use capabilities::{AccessCapabilities, Can, Capabilities};
pub use error::{ErrorClass, ErrorScope, VfsError};
pub use local::LocalProvider;
pub use names::{NameIssue, NameRules, check_name, check_name_with, first_refused};
pub use provider::{
    AttributeChange, CopyHint, FileWriter, LinkKind, LinkRequest, Principal, Provider, ReadSeek,
    StreamingList, WatchEvent, WatchHandle, WatchSink, WriteMode,
};
pub use registry::Registry;
pub use trash::{TrashCheck, TrashFacts, TrashGate, TrashRules, trash_gate};
pub use volume::{LocalPlace, PlaceRole, SpaceInfo, VolumeInfo, VolumeKind};

/// The tops of the local file system: the drives on Windows, `/` elsewhere.
/// Listing them never touches a drive.
pub fn local_roots() -> Vec<diroxide_core::Place> {
    sys::roots()
}

/// The places for the go-to-place list, each with its kind and its words:
/// see [`local::places`].
pub fn local_places() -> Vec<LocalPlace> {
    local::places()
}
