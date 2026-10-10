//! The virtual file system of diroxide.
//!
//! A [`Provider`] serves places of one kind: the local disk, archives, SFTP,
//! or a plugin's protocol. Every operation takes an [`OpContext`] that
//! carries a [`CancelToken`] and a [`ProgressSink`]. The [`Registry`]
//! resolves a place to its provider. The local provider lives here, with
//! its platform details behind the `sys` module, the one module in the
//! workspace allowed to use `unsafe` (reviewed at integration).
//!
//! Phase 0: the trait and types are the contract; the local provider's
//! bodies are deliberately naive (`std::fs` only, no attributes, no trash,
//! no watching, no long-path handling). Wave 1 (W1a) replaces them.

pub mod cancel;
pub mod capabilities;
pub mod error;
pub mod local;
pub mod names;
pub mod provider;
pub mod registry;
mod sys;

pub use cancel::{CancelToken, NoProgress, OpContext, ProgressSink, ProgressUpdate};
pub use capabilities::Capabilities;
pub use error::VfsError;
pub use local::LocalProvider;
pub use names::{NameIssue, check_name};
pub use provider::{
    AttributeChange, CopyHint, Provider, WatchEvent, WatchHandle, WatchSink, WriteMode,
};
pub use registry::Registry;

/// The tops of the local file system: the drives on Windows, `/` elsewhere.
/// Listing them never touches a drive.
pub fn local_roots() -> Vec<diroxide_core::Place> {
    sys::roots()
}
