//! Dirsize: the total size of a folder, as a job on the walk helper with
//! no filters at all (hidden files, ignored files and everything else are
//! counted; R14: the search walker's filters would undercount), and
//! "size every folder" over a panel's folders.
//!
//! A Unix hard link is counted once, by its volume and inode, for files
//! with more than one link; Windows listings do not say how many links a
//! file has, so there the words say hard links may be counted more than
//! once. Links are counted as links, never followed. Snapshot folders,
//! other drives, loops and folders past 256 levels are not entered, and
//! say so: the size is then "at least". Cloud folders give the total and
//! what is on this computer.

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::sync::mpsc::channel;

use diroxide_core::format::human_size;
use diroxide_core::{EntryKind, Place};
use diroxide_vfs::walk::{SkipReason, WalkEvent, WalkOptions, walk};
use diroxide_vfs::{Registry, VfsError, facts};

use crate::summary::counted;
use crate::{DirSize, SearchControl, SearchError};

impl DirSize {
    /// Whether every folder below was read and counted.
    pub fn is_complete(&self) -> bool {
        self.unreadable == 0 && self.skipped == 0 && !self.cancelled
    }

    /// The size in words, meaning first: "2.1 MB in 40 files and 3
    /// folders" when complete, "At least 12.3 GB, 2 folders unreadable"
    /// when not.
    pub fn sentence(&self) -> String {
        if self.is_complete() {
            return self.describe();
        }
        let why = if self.cancelled {
            "stopped".to_owned()
        } else if self.unreadable > 0 {
            format!(
                "{} unreadable",
                counted(self.unreadable, "folder", "folders")
            )
        } else {
            format!("{} skipped", counted(self.skipped, "folder", "folders"))
        };
        format!("At least {}, {why}", human_size(self.bytes))
    }

    /// What the count did with hard links: "Counted hard links once." on
    /// Unix, "Hard links may be counted more than once." where listings
    /// do not say.
    pub fn hard_link_words(&self) -> &'static str {
        if self.hard_links_once {
            "Counted hard links once."
        } else {
            "Hard links may be counted more than once."
        }
    }

    /// For a cloud folder, the total and what is here: "Total 4.2 GB, 1.1
    /// GB on this computer"; `None` when nothing is online only.
    pub fn cloud_words(&self) -> Option<String> {
        (self.online_only > 0).then(|| {
            format!(
                "Total {}, {} on this computer",
                human_size(self.bytes),
                human_size(self.on_this_computer)
            )
        })
    }
}

/// Adds up a folder through its provider with no filters, as a job:
/// `control` cancels or holds it. A start that cannot be read is an
/// error; a folder inside it that cannot be read is counted.
pub fn dirsize_with(
    place: &Place,
    registry: &Registry,
    control: &SearchControl,
) -> Result<DirSize, SearchError> {
    let provider = registry.resolve(place)?;
    let ctx = control.op_context();
    let mut size = DirSize {
        hard_links_once: cfg!(unix) && place.local_path().is_some(),
        ..DirSize::default()
    };
    let mut seen = HashSet::new();
    let result = walk(
        &*provider,
        place,
        &WalkOptions::default(),
        &ctx,
        &mut |event| {
            if control.checkpoint() {
                return ControlFlow::Break(());
            }
            match event {
                WalkEvent::Enter { depth, .. } if depth > 0 => size.folders += 1,
                WalkEvent::Item { entry, .. } => match entry.kind {
                    EntryKind::File | EntryKind::Archive => {
                        let repeated = entry.hard_links.is_some_and(|n| n > 1)
                            && entry
                                .volume
                                .zip(entry.inode)
                                .is_some_and(|key| !seen.insert(key));
                        if repeated {
                            size.repeated_links += 1;
                        } else {
                            let bytes = entry.size.unwrap_or(0);
                            size.files += 1;
                            size.bytes = size.bytes.saturating_add(bytes);
                            if facts::is_online_only(entry) {
                                size.online_only += 1;
                            } else {
                                size.on_this_computer = size.on_this_computer.saturating_add(bytes);
                            }
                        }
                    }
                    EntryKind::Link => size.links += 1,
                    _ => {}
                },
                WalkEvent::Skipped { reason, .. } => match reason {
                    SkipReason::Unreadable(_) => size.unreadable += 1,
                    _ => size.skipped += 1,
                },
                _ => {}
            }
            ControlFlow::Continue(())
        },
    );
    match result {
        Ok(_) => {}
        Err(VfsError::Cancelled) => size.cancelled = true,
        Err(error) => return Err(error.into()),
    }
    size.cancelled |= control.is_cancelled();
    Ok(size)
}

/// Size every folder: dirsize for each of `folders`, up to four at a
/// time, telling `each` about every folder as it is done (in the order
/// they finish), and returning every result in the order given.
pub fn size_every_folder(
    folders: &[Place],
    registry: &Registry,
    control: &SearchControl,
    each: &mut dyn FnMut(&Place, &Result<DirSize, SearchError>),
) -> Vec<(Place, Result<DirSize, SearchError>)> {
    let mut results: Vec<Option<Result<DirSize, SearchError>>> = vec![None; folders.len()];
    let (tx, rx) = channel();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = folders.len().clamp(1, 4);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(folder) = folders.get(index) else {
                        break;
                    };
                    if control.is_cancelled() {
                        break;
                    }
                    let size = dirsize_with(folder, registry, control);
                    if tx.send((index, size)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for (index, size) in rx {
            each(&folders[index], &size);
            results[index] = Some(size);
        }
    });
    folders
        .iter()
        .zip(results)
        .map(|(folder, result)| {
            let result = result.unwrap_or_else(|| {
                Ok(DirSize {
                    cancelled: true,
                    ..DirSize::default()
                })
            });
            (folder.clone(), result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_say_complete_or_at_least() {
        let mut size = DirSize {
            bytes: 12_300_000_000,
            files: 40,
            folders: 3,
            ..DirSize::default()
        };
        assert_eq!(size.sentence(), "12.3 GB in 40 files and 3 folders");
        size.unreadable = 2;
        assert_eq!(size.sentence(), "At least 12.3 GB, 2 folders unreadable");
        assert!(size.sentence().chars().count() <= 40);
        size.unreadable = 0;
        size.skipped = 1;
        assert_eq!(size.sentence(), "At least 12.3 GB, 1 folder skipped");
        size.cancelled = true;
        assert_eq!(size.sentence(), "At least 12.3 GB, stopped");
        assert_eq!(
            size.hard_link_words(),
            "Hard links may be counted more than once."
        );
        size.hard_links_once = true;
        assert_eq!(size.hard_link_words(), "Counted hard links once.");
        assert_eq!(size.cloud_words(), None);
        size.online_only = 3;
        size.on_this_computer = 1_100_000_000;
        assert_eq!(
            size.cloud_words().unwrap(),
            "Total 12.3 GB, 1.1 GB on this computer"
        );
    }
}
