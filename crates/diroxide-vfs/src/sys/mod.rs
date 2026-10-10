//! Platform details of the local provider: Windows attributes, long paths,
//! reparse points and the drive list from `GetLogicalDrives`; Unix modes,
//! owners and groups.
//!
//! This is the one module in the workspace allowed to use `unsafe`, for
//! calls into the platform. Every `unsafe` block here carries a comment
//! that says why it is sound, and is reviewed at integration. Phase 0 has
//! none.
#![allow(unsafe_code)]

use diroxide_core::Place;

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
