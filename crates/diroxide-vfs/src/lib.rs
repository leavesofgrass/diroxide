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
//! ([`volume`]), the name rules ([`names`]), the walk with its boundary
//! rule ([`mod@walk`]), and the places a recursive removal never takes
//! whole ([`protected`]). [`facts`] reads online-only, sparse and read-only
//! entries the same way for every caller.
//!
//! The local provider ([`LocalProvider`], Wave 1, W1a) is complete on the
//! Wave 1 contract: see its module for what each method does, per place.

pub mod cancel;
pub mod capabilities;
pub mod error;
pub mod facts;
pub mod limits;
pub mod local;
pub mod names;
pub mod protected;
pub mod provider;
pub mod registry;
mod sys;
pub mod trash;
pub mod volume;
pub mod walk;

pub use cancel::{
    CancelToken, NoProgress, NoSecrets, OpContext, ProgressSink, ProgressUpdate, SecretSource,
};
pub use capabilities::{AccessCapabilities, Can, Capabilities};
pub use error::{ErrorClass, ErrorScope, VfsError};
pub use local::{LocalEntryBuilder, LocalProvider};
pub use names::{NameIssue, NameRules, check_name, check_name_with, first_refused};
pub use protected::{Protection, protection};
pub use provider::{
    AttributeChange, CopyHint, FileWriter, LinkKind, LinkRequest, Principal, Provider, ReadSeek,
    StreamingList, WatchEvent, WatchHandle, WatchSink, WriteMode,
};
pub use registry::Registry;
/// The kill-on-close job object of Windows, for the process helper.
#[cfg(windows)]
pub use sys::KillOnCloseJob;
/// The owner-only named pipe of Windows, for the SFTP plugin's askpass
/// endpoint (plan answer 101).
#[cfg(windows)]
pub use sys::{OwnerPipe, PipeStream, plain_pipe_name};
pub use trash::{TrashCheck, TrashFacts, TrashGate, TrashRules, trash_gate};
pub use volume::{LocalPlace, PlaceRole, SpaceInfo, VolumeInfo, VolumeKind};
pub use walk::{SkipReason, WalkEvent, WalkOptions, WalkSummary, walk};

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

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers for the unit tests that write.

    use std::path::Path;

    /// A folder for a unit test that writes, under the build's own target
    /// folder (the test binary lives in `target/<profile>/deps`), never a
    /// user's folder or the system's temporary folder.
    pub(crate) fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().expect("the test's own path");
        let base = exe
            .parent()
            .and_then(Path::parent)
            .expect("the target folder")
            .join("vfs-unit-tmp");
        std::fs::create_dir_all(&base).expect("a scratch folder");
        tempfile::Builder::new()
            .prefix("vfs-unit-")
            .tempdir_in(base)
            .expect("a scratch folder")
    }
}
