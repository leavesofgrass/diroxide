//! Platform details of the local provider: Windows attributes, long paths,
//! reparse points and the drive list from `GetLogicalDrives`; Unix modes,
//! owners and groups.
//!
//! This is the one module in the workspace allowed to use `unsafe`, for
//! calls into the platform. Every `unsafe` block here carries a comment
//! that says why it is sound, and is reviewed at integration. Phase 0 has
//! none.
#![allow(unsafe_code)]

use std::io;
use std::path::Path;

use diroxide_core::Place;

/// Renames without ever replacing: fails with `AlreadyExists` when the
/// target is another entry. Phase 0 checks, then renames, which leaves a
/// race; W1a replaces it with the platform's own no-replace rename
/// (`renameat2` with `RENAME_NOREPLACE`, `MoveFileExW` without the replace
/// flag, the macOS exclusive rename) and lets a case-only change of the
/// same entry through on a place that ignores case.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// The tops of the local file system. Phase 0: `/` on Unix; on Windows an
/// empty list until W1a reads the drive bitmask, which never touches a
/// drive.
pub(crate) fn roots() -> Vec<Place> {
    if cfg!(windows) {
        Vec::new()
    } else {
        Place::parse("/").into_iter().collect()
    }
}
