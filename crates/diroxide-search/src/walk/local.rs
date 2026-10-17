//! The local walker: `ignore`'s parallel walker with its standard filters
//! off. Links are never followed here (a search that follows them takes
//! the provider walk, which keeps the boundary rule). Hidden entries,
//! snapshot folders and marked places are pruned by diroxide's own test
//! and counted; ignore files are read only when the scope asks, and then
//! only `.ignore` and `.gitignore` inside the start, in or out of a git
//! repository.
//!
//! On Windows another drive mounted in a folder is a reparse point, which
//! the walk sees as a link and does not enter; on Unix a folder on
//! another device is handed over and not entered, and counted. A folder
//! that is one of its own ancestors (a bind mount of a folder above it,
//! known by volume and inode where the entry carries them, as on Unix) is
//! a loop: handed over, not entered, counted, as the provider walk does.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_core::{Entry, EntryKind};
use diroxide_vfs::LocalEntryBuilder;
use ignore::{DirEntry, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState};

use super::{Candidate, Location, Visitor, WalkPlan, WalkStats};
use crate::local_entry::{entry_from, is_hidden};

/// What the walker's own test pruned, counted from every thread.
#[derive(Default)]
struct Pruned {
    hidden: AtomicU64,
    snapshots: AtomicU64,
    marked: AtomicU64,
}

/// A place's text in the form paths are compared in: without case on
/// Windows.
fn comparable(text: &str) -> String {
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.to_owned()
    }
}

/// Walks a local start with `ignore`'s parallel walker, handing every
/// entry to a visitor made by `make` on each thread.
pub(crate) fn walk<V, F>(plan: &WalkPlan<'_>, root_path: &Path, make: &F) -> WalkStats
where
    F: Fn() -> V + Sync,
    V: Visitor,
{
    let pruned = Arc::new(Pruned::default());
    let mut builder = WalkBuilder::new(root_path);
    builder
        .standard_filters(false)
        .follow_links(false)
        .threads(plan.threads)
        .max_depth(Some(plan.depth_limit() as usize));
    if plan.ignore_files {
        builder.ignore(true).git_ignore(true).require_git(false);
    }
    let hidden = plan.hidden;
    let skip_names: Vec<String> = plan.skip_names.to_vec();
    let skip_places: HashSet<String> = plan
        .skip_places
        .iter()
        .filter_map(|p| p.local_path().map(comparable))
        .collect();
    let counts = Arc::clone(&pruned);
    builder.filter_entry(move |dent: &DirEntry| {
        if !hidden {
            // The hidden attribute is in the folder read on Windows; the
            // Finder's flag needs a look on macOS, taken for folders only,
            // which prune whole subtrees (a file with the flag is caught
            // when its metadata is read).
            let metadata = if cfg!(windows)
                || (cfg!(target_os = "macos") && dent.file_type().is_some_and(|t| t.is_dir()))
            {
                dent.metadata().ok()
            } else {
                None
            };
            if is_hidden(dent.file_name(), metadata.as_ref()) {
                counts.hidden.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        }
        if dent.file_type().is_some_and(|t| t.is_dir()) {
            if dent
                .file_name()
                .to_str()
                .is_some_and(|name| skip_names.iter().any(|s| s == name))
            {
                counts.snapshots.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            if !skip_places.is_empty()
                && skip_places.contains(&comparable(&dent.path().to_string_lossy()))
            {
                counts.marked.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        }
        true
    });
    let stats = Mutex::new(WalkStats::default());
    #[cfg(unix)]
    let root_device = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(root_path).ok().map(|m| m.dev())
    };
    #[cfg(not(unix))]
    let root_device: Option<u64> = None;
    // The folders walked so far by their volume and inode, where entries
    // carry them, for the loop check on each folder's own ancestors.
    let entries = LocalEntryBuilder::new();
    let mut walked = HashMap::new();
    if let Ok(metadata) = std::fs::metadata(root_path) {
        let root = entry_from(&entries, OsStr::new(""), root_path, None, Some(&metadata));
        if let Some(id) = root.volume.zip(root.inode) {
            walked.insert(root_path.to_path_buf(), id);
        }
    }
    let walked = Mutex::new(walked);
    let mut threads = Threads {
        plan,
        make,
        stats: &stats,
        root_device,
        root_path,
        walked: &walked,
        entries: &entries,
    };
    builder.build_parallel().visit(&mut threads);
    let mut stats = stats.into_inner().unwrap_or_else(PoisonError::into_inner);
    stats.skips.hidden += pruned.hidden.load(Ordering::Relaxed);
    stats.skips.snapshots += pruned.snapshots.load(Ordering::Relaxed);
    stats.skips.marked += pruned.marked.load(Ordering::Relaxed);
    stats.skips.ignore_files = plan.ignore_files;
    stats
}

/// Makes one visitor per walking thread.
struct Threads<'a, F> {
    plan: &'a WalkPlan<'a>,
    make: &'a F,
    stats: &'a Mutex<WalkStats>,
    root_device: Option<u64>,
    root_path: &'a Path,
    walked: &'a Mutex<HashMap<PathBuf, (u64, u64)>>,
    entries: &'a LocalEntryBuilder,
}

impl<'s, F, V> ParallelVisitorBuilder<'s> for Threads<'s, F>
where
    F: Fn() -> V + Sync,
    V: Visitor + 's,
{
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        Box::new(Thread {
            plan: self.plan,
            visitor: (self.make)(),
            stats: WalkStats::default(),
            shared: self.stats,
            root_device: self.root_device,
            root_path: self.root_path,
            walked: self.walked,
            entries: self.entries,
        })
    }
}

/// One walking thread: its visitor and its own counts, added to the
/// shared counts when the thread ends.
struct Thread<'a, V: Visitor> {
    plan: &'a WalkPlan<'a>,
    visitor: V,
    stats: WalkStats,
    shared: &'a Mutex<WalkStats>,
    root_device: Option<u64>,
    root_path: &'a Path,
    walked: &'a Mutex<HashMap<PathBuf, (u64, u64)>>,
    /// Makes entries as the local provider lists them.
    entries: &'a LocalEntryBuilder,
}

impl<V: Visitor> Drop for Thread<'_, V> {
    fn drop(&mut self) {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .add(&self.stats);
    }
}

impl<V: Visitor> ParallelVisitor for Thread<'_, V> {
    fn visit(&mut self, result: Result<DirEntry, ignore::Error>) -> WalkState {
        if self.plan.control.checkpoint() {
            self.stats.stopped = true;
            return WalkState::Quit;
        }
        let dent = match result {
            Ok(dent) => dent,
            Err(_) => {
                self.stats.unreadable += 1;
                return WalkState::Continue;
            }
        };
        let depth = u32::try_from(dent.depth()).unwrap_or(u32::MAX);
        let is_dir = dent.file_type().is_some_and(|t| t.is_dir());
        if depth == 0 && is_dir {
            self.stats.folders += 1;
            self.plan.live.folders.fetch_add(1, Ordering::Relaxed);
            return WalkState::Continue;
        }
        let is_link = dent.path_is_symlink();
        let metadata = if cfg!(windows) || self.plan.metadata || (is_dir && cfg!(unix)) {
            dent.metadata().ok()
        } else {
            None
        };
        let entry = entry_from(
            self.entries,
            dent.file_name(),
            dent.path(),
            dent.file_type(),
            metadata.as_ref(),
        );
        if !self.plan.hidden && entry.hidden {
            // Hidden only by what the metadata says (the Finder's flag on
            // macOS), which the walk's own test could not see.
            self.stats.skips.hidden += 1;
            return WalkState::Skip;
        }
        let mut descend = true;
        if is_dir {
            if self.other_device(metadata.as_ref()) {
                self.stats.skips.other_drives += 1;
                descend = false;
            } else if !self.plan.enters(depth) {
                descend = false;
                if self.plan.too_deep(depth) {
                    self.stats.skips.too_deep += 1;
                }
            } else if self.repeats_an_ancestor(dent.path(), &entry) {
                self.stats.skips.loops += 1;
                descend = false;
            } else {
                self.stats.folders += 1;
                self.plan.live.folders.fetch_add(1, Ordering::Relaxed);
            }
        } else if is_link
            && entry
                .link
                .as_ref()
                .is_some_and(|l| l.points_to == Some(EntryKind::Folder))
        {
            self.stats.skips.links_not_followed += 1;
        }
        self.stats.entries += 1;
        let candidate = Candidate {
            entry,
            depth,
            location: Location::Local(dent.into_path()),
        };
        if self.visitor.visit(candidate) == ControlFlow::Break(()) {
            self.stats.stopped = true;
            return WalkState::Quit;
        }
        if descend {
            WalkState::Continue
        } else {
            WalkState::Skip
        }
    }
}

impl<V: Visitor> Thread<'_, V> {
    /// Whether a folder is one of the folders above it on its own path, by
    /// volume and inode (a bind mount of an ancestor); folders reached
    /// another way are walked. A folder that is not a repeat is noted, so
    /// the folders below it can be checked against it.
    fn repeats_an_ancestor(&self, path: &Path, entry: &Entry) -> bool {
        let Some(id) = entry.volume.zip(entry.inode) else {
            return false;
        };
        let mut walked = self.walked.lock().unwrap_or_else(PoisonError::into_inner);
        let repeat = path
            .ancestors()
            .skip(1)
            .take_while(|above| above.starts_with(self.root_path))
            .any(|above| walked.get(above) == Some(&id));
        if !repeat {
            walked.insert(path.to_path_buf(), id);
        }
        repeat
    }

    /// Whether a folder is on another device than the start (Unix; on
    /// Windows such a folder is a reparse point, which is a link).
    fn other_device(&self, metadata: Option<&std::fs::Metadata>) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            self.plan.one_file_system
                && matches!((self.root_device, metadata), (Some(root), Some(m)) if m.dev() != root)
        }
        #[cfg(not(unix))]
        {
            let _ = (metadata, self.root_device);
            false
        }
    }
}
