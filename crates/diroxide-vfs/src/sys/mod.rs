//! Platform details of the local provider: Windows attributes, long paths,
//! reparse points and the drive list from `GetLogicalDrives`; Unix modes,
//! owners and groups.
//!
//! It also holds the kill-on-close job object of Windows, which the
//! process helper of `diroxide-tools` puts every process tree in, and the
//! owner-only named pipe of Windows, the SFTP plugin's askpass endpoint
//! (plan answer 101).
//!
//! This is the one module in the workspace allowed to use `unsafe`, for
//! calls into the platform. Every `unsafe` block here carries a comment
//! that says why it is sound, and is reviewed at integration. The Unix
//! side has none: it uses the standard library and `rustix`'s safe
//! wrappers.
//!
//! Each platform file offers the same `pub(crate)` functions with the
//! same signatures, so the provider above calls them without `cfg`.
#![allow(unsafe_code)]

#[cfg(windows)]
mod owner_pipe;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use owner_pipe::{OwnerPipe, PipeStream, plain_pipe_name};
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(windows)]
pub use windows::KillOnCloseJob;
#[cfg(windows)]
pub(crate) use windows::*;

/// Who an entry is, apart from its name: its volume and its file id
/// (the Unix device and inode, or the Windows volume serial and file id).
/// Two names with one identity are one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "compared whole on Unix; read by field on Windows")
)]
pub(crate) struct Identity {
    /// The volume: the Unix device number or the Windows serial number.
    pub(crate) volume: u64,
    /// The file id: the inode, or the 128-bit Windows file id.
    pub(crate) id: u128,
}

impl Identity {
    /// The id as an [`diroxide_core::Entry::inode`]: the low 64 bits,
    /// which hold the whole id on every file system but ReFS.
    #[cfg_attr(not(windows), allow(dead_code, reason = "Unix reads the inode itself"))]
    pub(crate) fn inode(self) -> u64 {
        (self.id & u128::from(u64::MAX)) as u64
    }
}
