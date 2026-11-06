//! The plugin's own files in the host's spool folder: a partition's or an
//! image's bytes for 7-Zip or `qemu-img` to read, a member 7-Zip unpacked,
//! a disk `qemu-img` converted. Every one is named plainly
//! (`diro-disk-N-WHAT`), counted against one budget (the archive limits'
//! `max_spool_disk_bytes`, 4 GiB), and removed when its image is closed or
//! the plugin stops: the plugin removes only what it made, by the path it
//! made, never a folder it did not.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::error::DiskError;
use crate::lines::cause;

/// The plugin's files in the spool.
#[derive(Debug)]
pub struct Spool {
    dir: PathBuf,
    budget: u64,
    used: AtomicU64,
    next: AtomicU64,
    made: Mutex<Vec<PathBuf>>,
}

impl Spool {
    /// The spool folder the host named, with a budget in bytes.
    pub fn new(dir: PathBuf, budget: u64) -> Self {
        Self {
            dir,
            budget,
            used: AtomicU64::new(0),
            next: AtomicU64::new(1),
            made: Mutex::new(Vec::new()),
        }
    }

    /// The spool folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Takes `bytes` of the budget, or refuses with both figures.
    pub fn reserve(&self, bytes: u64, held: u64) -> Result<(), DiskError> {
        let taken = self
            .used
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= self.budget)
            });
        match taken {
            Ok(_) => Ok(()),
            Err(_) => Err(DiskError::Refused(cause::too_large_to_unpack(bytes, held))),
        }
    }

    /// A new plain name in the spool for `what`: `diro-disk-7-part.img`.
    pub fn name(&self, what: &str) -> PathBuf {
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let what: String = what
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let path = self
            .dir
            .join(format!("diro-disk-{}-{n}-{what}", std::process::id()));
        self.made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.clone());
        path
    }

    /// A new file in the spool, made new and never opened over one that
    /// exists.
    pub fn create(&self, what: &str) -> Result<(PathBuf, File), DiskError> {
        let path = self.name(what);
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| DiskError::Io(e.to_string()))?;
        Ok((path, file))
    }

    /// A new folder in the spool, for 7-Zip to unpack into.
    pub fn folder(&self, what: &str) -> Result<PathBuf, DiskError> {
        let path = self.name(what);
        fs::create_dir(&path).map_err(|e| DiskError::Io(e.to_string()))?;
        Ok(path)
    }

    /// Writes `bytes` pieces from `read` into a new spool file of `len`
    /// bytes, inside the budget.
    pub fn copy_in(
        &self,
        what: &str,
        len: u64,
        held: u64,
        mut read: impl FnMut(u64, usize) -> Result<Vec<u8>, DiskError>,
    ) -> Result<PathBuf, DiskError> {
        self.reserve(len, held)?;
        let (path, mut file) = self.create(what)?;
        let mut at = 0u64;
        while at < len {
            let want = usize::try_from((len - at).min(4 * 1024 * 1024)).unwrap_or(0);
            let bytes = read(at, want)?;
            if bytes.is_empty() {
                break;
            }
            file.write_all(&bytes)
                .map_err(|e| DiskError::Io(e.to_string()))?;
            at += bytes.len() as u64;
        }
        file.flush().map_err(|e| DiskError::Io(e.to_string()))?;
        Ok(path)
    }

    /// Removes everything the plugin made in the spool, files and the
    /// folders 7-Zip unpacked into, by the paths it made.
    pub fn clean(&self) {
        let made: Vec<PathBuf> =
            std::mem::take(&mut *self.made.lock().unwrap_or_else(PoisonError::into_inner));
        for path in made {
            if path.parent() != Some(self.dir.as_path()) {
                continue;
            }
            let _ = remove_made(&path);
        }
        self.used.store(0, Ordering::SeqCst);
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.clean();
    }
}

/// Removes one thing the plugin made: a file, or a folder 7-Zip unpacked
/// into with what is in it; a link is removed as itself, never followed.
fn remove_made(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe.parent().unwrap().join("spool-tests");
        fs::create_dir_all(&base).unwrap();
        tempfile::tempdir_in(base).unwrap()
    }

    #[test]
    fn the_budget_holds_and_cleaning_removes_only_what_was_made() {
        let dir = here();
        let spool = Spool::new(dir.path().to_path_buf(), 100);
        let keep = dir.path().join("not-ours.txt");
        fs::write(&keep, b"theirs").unwrap();
        let path = spool
            .copy_in("part.img", 60, 60, |at, n| Ok(vec![at as u8; n]))
            .unwrap();
        assert_eq!(fs::read(&path).unwrap().len(), 60);
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("diro-disk-")
        );
        assert!(matches!(spool.reserve(50, 1), Err(DiskError::Refused(_))));
        let folder = spool.folder("unpacked").unwrap();
        fs::write(folder.join("x.txt"), b"x").unwrap();
        spool.clean();
        assert!(!path.exists());
        assert!(!folder.exists());
        assert!(keep.exists());
        spool.reserve(100, 1).unwrap();
    }
}
