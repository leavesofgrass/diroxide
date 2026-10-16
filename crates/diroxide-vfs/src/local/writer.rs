//! The committing writer and the unfinished names it and the clone write
//! through (research/file-operations.md C1, "Cancellation, atomicity and
//! partial files").
//!
//! The bytes go to `.diro-part-` and eight hexadecimal digits in the
//! target's own folder (one volume, so the final rename is atomic), a name
//! never derived from the real one. The commit renames it into place: with
//! [`WriteMode::CreateNew`] through the no-replace rename, so a file that
//! appeared meanwhile is a conflict and never a loss; with
//! [`WriteMode::Overwrite`] replacing. A writer dropped without a commit,
//! on an error, a cancel or a panic, removes its unfinished file, so no
//! half-written file is ever left under the real name.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::VfsError;
use crate::names;
use crate::provider::{FileWriter, WriteMode};
use crate::sys;

/// A token for an unfinished name: the clock's nanoseconds mixed with a
/// counter, so two writes in the same instant differ.
pub(crate) fn unfinished_token() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    nanos ^ COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed)
}

/// An unfinished file of ours, removed when dropped unless kept. Only a
/// name this program made is ever removed through it.
#[derive(Debug)]
pub(crate) struct Unfinished {
    path: PathBuf,
    armed: bool,
}

impl Unfinished {
    /// A fresh unfinished name in `folder`, not yet made; a call that
    /// makes the file (a clone, the copy call) makes it there.
    pub(crate) fn fresh(folder: &Path) -> Self {
        Self {
            path: folder.join(names::unfinished_name(unfinished_token())),
            armed: true,
        }
    }

    /// Makes a new, empty unfinished file in `folder` and opens it for
    /// writing, trying a few names if one is taken.
    pub(crate) fn create(folder: &Path) -> Result<(File, Self), VfsError> {
        let mut last = VfsError::AlreadyExists;
        for _ in 0..8 {
            let unfinished = Self::fresh(folder);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&unfinished.path)
            {
                Ok(file) => return Ok((file, unfinished)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    // Taken by someone else: never ours to remove.
                    let mut taken = unfinished;
                    taken.keep();
                }
                Err(error) => {
                    let mut failed = unfinished;
                    failed.keep();
                    last = error.into();
                }
            }
        }
        Err(last)
    }

    /// The unfinished file's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Keeps the file: it became the target, or it was never ours.
    pub(crate) fn keep(&mut self) {
        self.armed = false;
    }
}

impl Drop for Unfinished {
    fn drop(&mut self) {
        if self.armed {
            // Best effort: the unfinished file is ours alone, by its name.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The local committing writer.
pub(crate) struct LocalWriter {
    file: Option<File>,
    unfinished: Unfinished,
    target: PathBuf,
    mode: WriteMode,
}

impl LocalWriter {
    /// Starts a write of `target` through an unfinished file in its folder.
    pub(crate) fn start(target: PathBuf, mode: WriteMode) -> Result<Self, VfsError> {
        let folder = target
            .parent()
            .ok_or_else(|| VfsError::unsupported("file", "write at the top of a drive"))?;
        let (file, unfinished) = Unfinished::create(folder)?;
        Ok(Self {
            file: Some(file),
            unfinished,
            target,
            mode,
        })
    }
}

impl Write for LocalWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &mut self.file {
            Some(file) => file.write(bytes),
            None => Err(io::Error::other("the write was already committed")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.file {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

impl FileWriter for LocalWriter {
    fn commit(mut self: Box<Self>, durable: bool) -> Result<(), VfsError> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
            if durable {
                file.sync_all()?;
            }
        }
        match self.mode {
            WriteMode::CreateNew => sys::rename_no_replace(self.unfinished.path(), &self.target)?,
            WriteMode::Overwrite => fs::rename(self.unfinished.path(), &self.target)?,
        }
        self.unfinished.keep();
        Ok(())
    }
}
