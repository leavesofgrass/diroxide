//! The fast paths of a copy within the local disks (research
//! file-operations.md, "Reflinks, copy-on-write and fast paths"; plan
//! section 5.4, "Create, verify, then destroy"): a clone where the volume
//! can clone (Btrfs, XFS, bcachefs, APFS, ReFS), the copy call on a
//! Windows share (the server copies). Each is tried before the destination
//! is touched: the copy goes to an unfinished name in the destination's
//! folder, gets the source's modified and created times and attributes,
//! and is renamed into place without ever replacing. Anything the
//! platform cannot clone answers "unsupported", and the engine streams
//! the bytes itself.

use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

use diroxide_core::Attributes;

use crate::cancel::{OpContext, ProgressUpdate};
use crate::error::VfsError;
use crate::local::attributes::current_attributes;
use crate::local::volumes;
use crate::local::writer::Unfinished;
use crate::provider::CopyHint;
use crate::sys;
use crate::volume::VolumeKind;

/// The Windows attributes a copy keeps: read-only, hidden, system and
/// archive.
#[cfg(windows)]
const WINDOWS_KEPT: u32 = 0x1 | 0x2 | 0x4 | 0x20;

/// The Unix mode bits a copy keeps: permissions and sticky. A copy is the
/// copier's own file, so the set-id bits are dropped, as `cp` drops them
/// when it cannot keep the owner.
#[cfg(unix)]
const UNIX_KEPT: u32 = 0o1777;

/// Whether a copy from `from` to `to` has a fast path.
pub(crate) fn hint(from: &Path, to: &Path) -> CopyHint {
    let source = volumes::facts_at(from);
    let target = volumes::facts_at(to);
    if source.root.is_none() || source.root != target.root {
        return CopyHint::Stream;
    }
    let network = source.network_path || source.kind == VolumeKind::Network;
    if volumes::can_clone(&source) && !network {
        CopyHint::Reflink
    } else if cfg!(windows) && network && !source.read_only {
        CopyHint::ServerSide
    } else {
        CopyHint::Stream
    }
}

/// An error of a clone: one that says the volume cannot clone becomes
/// "unsupported", so the engine streams; the rest keep their meaning.
fn clone_error(error: io::Error) -> VfsError {
    match error.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::PermissionDenied
        | io::ErrorKind::StorageFull
        | io::ErrorKind::QuotaExceeded
        | io::ErrorKind::ReadOnlyFilesystem => error.into(),
        _ => VfsError::unsupported("file", "clone this file here"),
    }
}

/// Gives the copy the source's modified time, its created time where the
/// platform can set one, and its attributes (read-only last, so it never
/// blocks the times). Never the accessed time. Best effort: the copy is
/// whole without them, and the engine's re-check sees what stuck.
fn keep_times_and_attributes(source: &Metadata, copy: &Path) {
    let created = if cfg!(any(windows, target_os = "macos")) {
        source.created().ok()
    } else {
        None
    };
    let _ = sys::set_times(copy, created, source.modified().ok(), None);
    match current_attributes(source) {
        #[cfg(windows)]
        Attributes::Windows(flags) => {
            let _ = sys::set_os_attributes(copy, flags & WINDOWS_KEPT);
        }
        #[cfg(unix)]
        Attributes::Unix(mode) => {
            let _ = sys::set_mode(copy, mode & UNIX_KEPT);
        }
        _ => {}
    }
}

/// Copies one file by its fast path. Fails with `Unsupported` when there
/// is none, before anything is written.
pub(crate) fn copy_within(
    source: &Path,
    target: &Path,
    hint: CopyHint,
    ctx: &OpContext,
) -> Result<(), VfsError> {
    ctx.check()?;
    if hint == CopyHint::Stream {
        return Err(VfsError::unsupported("file", "copy without streaming"));
    }
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() {
        return Err(VfsError::unsupported("file", "clone anything but a file"));
    }
    if fs::symlink_metadata(target).is_ok() {
        return Err(VfsError::AlreadyExists);
    }
    let folder = target.parent().ok_or(VfsError::NotFound)?;
    let mut unfinished = Unfinished::fresh(folder);
    match hint {
        CopyHint::Reflink => {
            reflink_copy::reflink(source, unfinished.path()).map_err(clone_error)?
        }
        CopyHint::ServerSide => {
            sys::copy_file(source, unfinished.path(), ctx).map_err(|error| {
                if ctx.cancel.is_cancelled() {
                    VfsError::Cancelled
                } else {
                    error.into()
                }
            })?
        }
        CopyHint::Stream => return Err(VfsError::unsupported("file", "copy without streaming")),
    }
    keep_times_and_attributes(&metadata, unfinished.path());
    // A cancel now drops the unfinished copy, and the target was never
    // touched.
    ctx.check()?;
    sys::rename_no_replace(unfinished.path(), target)?;
    unfinished.keep();
    ctx.progress.report(ProgressUpdate {
        items_done: 1,
        bytes_done: metadata.len(),
        bytes_total: Some(metadata.len()),
        ..ProgressUpdate::default()
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_that_mean_no_clone_send_the_engine_to_stream() {
        let unsupported = clone_error(io::Error::from(io::ErrorKind::Unsupported));
        assert!(matches!(unsupported, VfsError::Unsupported { .. }));
        let cross = clone_error(io::Error::from(io::ErrorKind::CrossesDevices));
        assert!(matches!(cross, VfsError::Unsupported { .. }));
        assert_eq!(
            clone_error(io::Error::from(io::ErrorKind::StorageFull)),
            VfsError::StorageFull
        );
    }
}
