//! What the plugin keeps between the host's questions about one folder
//! (R29 2.3; the host-pieces lane's finding): the host asks for the title
//! line on every listing of a folder (a reload, a change its watch sees, a
//! job's end) and the column a page at a time, so one status serves them.
//!
//! **The status cache** is keyed by the repository's top and the folder,
//! and an entry is dropped when any of its stamps changes: the stat (time
//! and size) of `.git/index`, of `.git/HEAD`, and of the folder itself. An
//! edit inside a file changes none of these, so an entry is also kept only
//! for a short while: [`TITLE_FRESH`] for the title line, which is asked
//! once per listing and so should see that listing's changes, and
//! [`COLUMN_FRESH`] for the column's pages, which follow a listing; a
//! command that changes the repository drops its entries at once.
//!
//! **The scan cache** keeps the trust scan by the stamps of `config` and
//! `config.worktree`; a configuration that includes another file is never
//! kept, since what it names may change on its own.
//!
//! One lock per repository makes a page of the column wait for the status
//! the title line is already running, rather than run a second.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::git::status::Status;
use crate::git::trust::Scan;
use crate::repo::Stamp;

/// How long a status serves the title line.
pub const TITLE_FRESH: Duration = Duration::from_millis(500);

/// How long a status serves the column's pages.
pub const COLUMN_FRESH: Duration = Duration::from_secs(5);

/// The most folders the status cache holds; the oldest goes first.
pub const MAX_FOLDERS: usize = 256;

/// One folder's status, when it was taken and the stamps it was taken
/// with.
#[derive(Clone, Debug)]
pub struct Kept {
    /// The status, pathspec-limited to the folder.
    pub status: Arc<Status>,
    /// Whether untracked files were counted.
    pub untracked_counted: bool,
    /// When it was taken.
    pub taken: Instant,
    /// The stamps of the index, `HEAD` and the folder.
    pub stamps: [Stamp; 3],
}

type Key = (PathBuf, String);

/// Folder statuses by repository and folder.
#[derive(Debug, Default)]
pub struct StatusCache {
    entries: Mutex<HashMap<Key, Kept>>,
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl StatusCache {
    /// The lock for one repository: hold it while looking and running.
    pub fn lock_for(&self, root: &Path) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(locks.entry(root.to_path_buf()).or_default())
    }

    /// A kept status still good: the same stamps and younger than
    /// `fresh`.
    pub fn get(
        &self,
        root: &Path,
        folder: &str,
        stamps: [Stamp; 3],
        fresh: Duration,
    ) -> Option<Kept> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(&(root.to_path_buf(), folder.to_owned()))
            .filter(|kept| kept.stamps == stamps && kept.taken.elapsed() < fresh)
            .cloned()
    }

    /// Keeps a status.
    pub fn put(&self, root: &Path, folder: &str, kept: Kept) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= MAX_FOLDERS {
            let oldest = entries
                .iter()
                .min_by_key(|(_, k)| k.taken)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert((root.to_path_buf(), folder.to_owned()), kept);
    }

    /// Drops every folder of a repository: a command changed it.
    pub fn drop_repository(&self, root: &Path) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(r, _), _| r != root);
    }

    /// How many folders are kept.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A kept scan and the stamps of the configuration it read.
type KeptScan = ([Stamp; 2], Arc<Scan>);

/// Trust scans by repository, with the stamps of its configuration.
#[derive(Debug, Default)]
pub struct ScanCache {
    entries: Mutex<HashMap<PathBuf, KeptScan>>,
}

impl ScanCache {
    /// A kept scan whose configuration has not changed.
    pub fn get(&self, root: &Path, stamps: [Stamp; 2]) -> Option<Arc<Scan>> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(root)
            .filter(|(s, _)| *s == stamps)
            .map(|(_, scan)| Arc::clone(scan))
    }

    /// Keeps a scan, unless it includes another file.
    pub fn put(&self, root: &Path, stamps: [Stamp; 2], scan: Arc<Scan>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if scan.includes {
            entries.remove(root);
        } else {
            entries.insert(root.to_path_buf(), (stamps, scan));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn kept(stamps: [Stamp; 3]) -> Kept {
        Kept {
            status: Arc::new(Status::default()),
            untracked_counted: true,
            taken: Instant::now(),
            stamps,
        }
    }

    #[test]
    fn a_status_is_kept_until_a_stamp_changes_or_it_ages() {
        let cache = StatusCache::default();
        let root = Path::new("repo");
        let stamps = [Stamp::default(); 3];
        cache.put(root, "src", kept(stamps));
        assert!(cache.get(root, "src", stamps, COLUMN_FRESH).is_some());
        assert!(cache.get(root, "docs", stamps, COLUMN_FRESH).is_none());
        let mut changed = stamps;
        changed[0] = Stamp {
            modified: Some(SystemTime::UNIX_EPOCH),
            len: 1,
        };
        assert!(cache.get(root, "src", changed, COLUMN_FRESH).is_none());
        assert!(cache.get(root, "src", stamps, Duration::ZERO).is_none());
        cache.drop_repository(root);
        assert!(cache.is_empty());
    }

    #[test]
    fn the_cache_holds_a_bounded_number_of_folders() {
        let cache = StatusCache::default();
        for n in 0..MAX_FOLDERS + 10 {
            cache.put(
                Path::new("repo"),
                &n.to_string(),
                kept([Stamp::default(); 3]),
            );
        }
        assert_eq!(cache.len(), MAX_FOLDERS);
    }

    #[test]
    fn a_scan_that_includes_a_file_is_never_kept() {
        let cache = ScanCache::default();
        let root = Path::new("repo");
        let stamps = [Stamp::default(); 2];
        cache.put(root, stamps, Arc::new(Scan::default()));
        assert!(cache.get(root, stamps).is_some());
        cache.put(
            root,
            stamps,
            Arc::new(Scan {
                includes: true,
                ..Scan::default()
            }),
        );
        assert!(cache.get(root, stamps).is_none());
    }
}
