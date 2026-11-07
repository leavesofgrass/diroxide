//! "Look again" (R16 2.1, rank 5; R28 6.3): before the original moves,
//! the runner reads the file's facts again and asks whether a program
//! still writes it. Spike H1 (Friday, November 6, 2026) decided the shape:
//!
//! - **NTFS:** std's `File::try_lock` and `try_lock_shared` (Rust 1.96)
//!   took the lock on every look at a file another process was writing
//!   without a lock, so they cannot tell a half-written file from a
//!   finished one; worse, holding that lock for 40 ms made 11 of the
//!   writer's 40 writes fail with error 33, losing 704 KiB. The share-mode
//!   open (the tracker's writer probe) answered "a writer holds it" on
//!   every look while the writer ran, and "no writer" after. So on
//!   Windows the look again is the share-mode probe, and no lock is ever
//!   taken on a file in `in`.
//! - **ext4 (and overlayfs):** a try-lock sees only a writer that locks
//!   (36 of 36 looks said "would block" for such a writer, 36 of 36 "got
//!   it" for one that does not), and a writer that does not lock is not
//!   harmed (0 write errors). So on Linux and macOS the look again tries
//!   an exclusive lock on a read handle and lets it go at once: a bonus
//!   for producers that lock, never a promise.
//!
//! Either way the facts compared before the move (size, modified time,
//! identity) are the rule; the probe only adds "still being written".

use std::path::Path;

/// Whether a program still writes the file at `path`: `Some(true)` when
/// one does, `Some(false)` when none is seen, `None` when this cannot be
/// told (the file could not be opened).
#[cfg(windows)]
pub fn still_written(path: &Path) -> Option<bool> {
    crate::tracker::writer_open(path)
}

/// Whether a program still writes the file at `path`: `Some(true)` when
/// one holds a lock on it, `Some(false)` when none does, `None` when this
/// cannot be told. A writer that takes no lock is not seen.
#[cfg(not(windows))]
pub fn still_written(path: &Path) -> Option<bool> {
    let file = std::fs::File::open(path).ok()?;
    match file.try_lock() {
        Ok(()) => Some(false),
        Err(std::fs::TryLockError::WouldBlock) => Some(true),
        Err(std::fs::TryLockError::Error(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::testing::scratch;

    #[test]
    fn a_closed_file_is_not_being_written() {
        let dir = scratch();
        let path = dir.path().join("scan.pdf");
        std::fs::write(&path, b"done").unwrap();
        assert_eq!(still_written(&path), Some(false));
        assert_eq!(still_written(&dir.path().join("missing.pdf")), None);
    }

    #[test]
    fn a_writer_that_holds_the_file_is_seen() {
        let dir = scratch();
        let path = dir.path().join("scan.pdf");
        let writer = std::fs::File::create(&path).unwrap();
        // On Windows the open handle alone is seen; elsewhere only a lock.
        #[cfg(not(windows))]
        writer.lock().unwrap();
        assert_eq!(still_written(&path), Some(true));
        drop(writer);
        assert_eq!(still_written(&path), Some(false));
    }
}
