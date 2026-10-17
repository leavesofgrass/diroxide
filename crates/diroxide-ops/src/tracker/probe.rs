//! The writer probe (R16 section 2.1, rank 4): whether a program still
//! holds a file open for writing, asked just before a quiet file would
//! count as arrived.
//!
//! On Windows the file is opened for reading while refusing to share
//! writing: the open fails with a sharing violation while any other handle
//! can write to it, which is how a copy, a download or a scanner that
//! pauses mid-file is told apart from a finished one. Deleting and
//! renaming stay shared, so the probe never stops a producer from renaming
//! its finished file into place, and the handle is closed at once. A file
//! that is online only is never probed, since opening it would fetch it.
//!
//! Elsewhere there is no such rule: Unix locks are advisory and a writer
//! rarely takes one, so the probe answers `None` and the quiet period
//! alone decides.

use std::path::Path;

/// Whether a writer still holds the file at `path` open: `Some(true)` when
/// one does, `Some(false)` when none does, `None` when this platform
/// cannot tell or the file could not be opened for another reason.
#[cfg(windows)]
pub fn writer_open(path: &Path) -> Option<bool> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_SHARE_READ | FILE_SHARE_DELETE: writing is not shared.
    const SHARE_READ_AND_DELETE: u32 = 0x1 | 0x4;
    // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION.
    const IN_USE: [i32; 2] = [32, 33];
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_AND_DELETE)
        .open(path)
    {
        Ok(_) => Some(false),
        Err(error)
            if error
                .raw_os_error()
                .is_some_and(|code| IN_USE.contains(&code)) =>
        {
            Some(true)
        }
        Err(_) => None,
    }
}

/// Whether a writer still holds the file at `path` open: `None` on this
/// platform, which cannot tell (see the module documentation).
#[cfg(not(windows))]
pub fn writer_open(path: &Path) -> Option<bool> {
    let _ = path;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn a_file_held_for_writing_is_in_use_and_a_closed_one_is_not() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("scan.pdf");
        let writer = std::fs::File::create(&path).unwrap();
        assert_eq!(writer_open(&path), Some(true));
        drop(writer);
        assert_eq!(writer_open(&path), Some(false));
        // A reader alone does not count as a writer.
        let reader = std::fs::File::open(&path).unwrap();
        assert_eq!(writer_open(&path), Some(false));
        drop(reader);
        assert_eq!(writer_open(&folder.path().join("missing.pdf")), None);
    }

    #[test]
    #[cfg(not(windows))]
    fn elsewhere_the_probe_cannot_tell() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("scan.pdf");
        let _writer = std::fs::File::create(&path).unwrap();
        assert_eq!(writer_open(&path), None);
    }
}
