//! The walk helper, shared by plans, copy, delete, size and the trash gate
//! (research/file-operations.md, "Symlink loops and the boundary rule";
//! plan section 5.12, "Boundaries are respected").
//!
//! It walks through any [`Provider`], one folder at a time, without
//! recursion, and keeps the boundary rule:
//!
//! - a link or junction is never entered, so nothing is followed out of
//!   the root; it is handed over as an item, to be acted on as a link;
//! - a folder on another volume (a mount, a bind mount) is not entered,
//!   where both volumes are known;
//! - a folder seen before (a hard-linked folder, an odd bind mount) is not
//!   entered again, by its volume and inode;
//! - a folder deeper than the limit (256 levels) is not entered, so a
//!   pathological tree is a reported skip and never a stack overflow;
//! - snapshot folders (`.zfs`, `.snapshots`) are not entered.
//!
//! Each skip says why, in words. A folder is announced when entered and
//! again when left, so a delete can act deepest first and a copy can set a
//! folder's times after its contents.

use std::collections::HashSet;
use std::ops::ControlFlow;

use diroxide_core::{Entry, EntryKind, Place};

use crate::cancel::OpContext;
use crate::error::VfsError;
use crate::limits::utf16_units;
use crate::provider::Provider;

/// The deepest a walk goes before it skips a folder as too deep.
pub const DEFAULT_MAX_DEPTH: usize = 256;

/// Folders a walk does not enter by default: file system snapshots, which
/// hold a copy of everything around them.
pub const SNAPSHOT_FOLDERS: [&str; 2] = [".zfs", ".snapshots"];

/// How a walk goes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WalkOptions {
    /// The deepest level entered; the root is level 0.
    pub max_depth: usize,
    /// Folder names never entered.
    pub skip_names: Vec<String>,
    /// Whether a folder on another volume is skipped.
    pub stay_on_volume: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            skip_names: SNAPSHOT_FOLDERS.iter().map(|s| (*s).to_owned()).collect(),
            stay_on_volume: true,
        }
    }
}

impl WalkOptions {
    /// The same options with another depth limit.
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// The same options with other folder names never entered.
    pub fn with_skip_names(mut self, names: Vec<String>) -> Self {
        self.skip_names = names;
        self
    }

    /// The same options, crossing into other volumes or not.
    pub fn with_stay_on_volume(mut self, stay: bool) -> Self {
        self.stay_on_volume = stay;
        self
    }
}

/// Why a walk did not enter a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// Another drive or file system is mounted there.
    OtherVolume,
    /// The folder was entered already: a loop.
    Loop,
    /// The folder is deeper than the limit.
    TooDeep,
    /// A snapshot folder.
    Snapshot,
    /// The folder could not be listed.
    Unreadable(VfsError),
}

impl SkipReason {
    /// Why, in words, for "Skipped photos: another drive is mounted here".
    pub fn words(&self) -> String {
        match self {
            SkipReason::OtherVolume => "another drive is mounted here".to_owned(),
            SkipReason::Loop => "it leads back to a folder already seen".to_owned(),
            SkipReason::TooDeep => format!("it is more than {DEFAULT_MAX_DEPTH} levels deep"),
            SkipReason::Snapshot => "it is a snapshot folder".to_owned(),
            SkipReason::Unreadable(error) => format!("it could not be read: {error}"),
        }
    }

    /// The line for a result list: "Skipped photos: another drive is
    /// mounted here".
    pub fn line(&self, name: &str) -> String {
        format!("Skipped {name}: {}", self.words())
    }
}

/// One step of a walk.
#[derive(Debug)]
pub enum WalkEvent<'a> {
    /// A folder is entered; its contents follow, then its [`WalkEvent::Leave`].
    Enter {
        /// The folder.
        place: &'a Place,
        /// Its entry.
        entry: &'a Entry,
        /// Its level; the root is 0.
        depth: usize,
    },
    /// Something that is not entered: a file, a link (acted on as a link),
    /// a special file, an item.
    Item {
        /// The item.
        place: &'a Place,
        /// Its entry.
        entry: &'a Entry,
        /// Its level.
        depth: usize,
    },
    /// A folder's contents are done: the place for deepest-first work.
    Leave {
        /// The folder.
        place: &'a Place,
        /// Its entry.
        entry: &'a Entry,
        /// Its level.
        depth: usize,
    },
    /// A folder that was not entered, with why.
    Skipped {
        /// The folder.
        place: &'a Place,
        /// Its entry.
        entry: &'a Entry,
        /// Its level.
        depth: usize,
        /// Why it was not entered.
        reason: &'a SkipReason,
    },
}

/// What a walk saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct WalkSummary {
    /// Folders entered, the root included.
    pub folders: u64,
    /// Files and archives.
    pub files: u64,
    /// Links and junctions (never entered).
    pub links: u64,
    /// Special files and items.
    pub others: u64,
    /// Folders skipped, for any reason.
    pub skipped: u64,
    /// The sizes of the files, in bytes.
    pub bytes: u64,
    /// The longest path seen, in UTF-16 units: what the trash gate asks of
    /// a folder.
    pub longest_path_units: usize,
    /// The deepest level seen.
    pub deepest: usize,
}

/// A folder being walked: its place, its entry, its level, and the
/// entries still to visit.
struct Frame {
    place: Place,
    entry: Entry,
    depth: usize,
    children: std::vec::IntoIter<Entry>,
}

/// Walks `root` through `provider`, telling `visit` each step. `visit`
/// returns `Break` to stop early; the summary then covers what was seen.
/// The root itself is the first step: a folder is entered, anything else
/// (a link among them) is one item. A cancel ends the walk with
/// `Cancelled`; a folder inside that cannot be listed is a skip.
pub fn walk(
    provider: &dyn Provider,
    root: &Place,
    options: &WalkOptions,
    ctx: &OpContext,
    visit: &mut dyn FnMut(WalkEvent<'_>) -> ControlFlow<()>,
) -> Result<WalkSummary, VfsError> {
    ctx.check()?;
    let root_entry = provider.stat(root, ctx)?;
    let mut summary = WalkSummary {
        longest_path_units: utf16_units(&root.to_string()),
        ..WalkSummary::default()
    };
    if root_entry.kind != EntryKind::Folder {
        count_item(&mut summary, &root_entry);
        let _ = visit(WalkEvent::Item {
            place: root,
            entry: &root_entry,
            depth: 0,
        });
        return Ok(summary);
    }
    let root_volume = root_entry.volume;
    let mut visited = HashSet::new();
    if let (Some(volume), Some(inode)) = (root_entry.volume, root_entry.inode) {
        visited.insert((volume, inode));
    }
    let children = provider.list(root, ctx)?;
    if visit(WalkEvent::Enter {
        place: root,
        entry: &root_entry,
        depth: 0,
    })
    .is_break()
    {
        return Ok(summary);
    }
    let mut stack = vec![Frame {
        place: root.clone(),
        entry: root_entry,
        depth: 0,
        children: children.into_iter(),
    }];
    while let Some(frame) = stack.last_mut() {
        ctx.check()?;
        let Some(entry) = frame.children.next() else {
            let Some(done) = stack.pop() else { break };
            summary.folders += 1;
            if visit(WalkEvent::Leave {
                place: &done.place,
                entry: &done.entry,
                depth: done.depth,
            })
            .is_break()
            {
                return Ok(summary);
            }
            continue;
        };
        let depth = frame.depth + 1;
        let place = match frame.place.join(&entry.name) {
            Ok(place) => place,
            Err(error) => {
                // A name the place cannot hold (it came from the provider,
                // so this is the provider's oddity): reported, not entered.
                summary.skipped += 1;
                let reason = SkipReason::Unreadable(error.into());
                let parent = frame.place.clone();
                if visit(WalkEvent::Skipped {
                    place: &parent,
                    entry: &entry,
                    depth,
                    reason: &reason,
                })
                .is_break()
                {
                    return Ok(summary);
                }
                continue;
            }
        };
        summary.longest_path_units = summary
            .longest_path_units
            .max(utf16_units(&place.to_string()));
        summary.deepest = summary.deepest.max(depth);
        if entry.kind != EntryKind::Folder {
            count_item(&mut summary, &entry);
            if visit(WalkEvent::Item {
                place: &place,
                entry: &entry,
                depth,
            })
            .is_break()
            {
                return Ok(summary);
            }
            continue;
        }
        let skip = if options.skip_names.contains(&entry.name) {
            Some(SkipReason::Snapshot)
        } else if options.stay_on_volume
            && root_volume.is_some()
            && entry.volume.is_some()
            && entry.volume != root_volume
        {
            Some(SkipReason::OtherVolume)
        } else if depth > options.max_depth {
            Some(SkipReason::TooDeep)
        } else if let (Some(volume), Some(inode)) = (entry.volume, entry.inode)
            && !visited.insert((volume, inode))
        {
            Some(SkipReason::Loop)
        } else {
            None
        };
        let listed = match skip {
            Some(reason) => Err(reason),
            None => match provider.list(&place, ctx) {
                Ok(children) => Ok(children),
                Err(VfsError::Cancelled) => return Err(VfsError::Cancelled),
                Err(error) => Err(SkipReason::Unreadable(error)),
            },
        };
        match listed {
            Ok(children) => {
                if visit(WalkEvent::Enter {
                    place: &place,
                    entry: &entry,
                    depth,
                })
                .is_break()
                {
                    return Ok(summary);
                }
                stack.push(Frame {
                    place,
                    entry,
                    depth,
                    children: children.into_iter(),
                });
            }
            Err(reason) => {
                summary.skipped += 1;
                if visit(WalkEvent::Skipped {
                    place: &place,
                    entry: &entry,
                    depth,
                    reason: &reason,
                })
                .is_break()
                {
                    return Ok(summary);
                }
            }
        }
    }
    Ok(summary)
}

fn count_item(summary: &mut WalkSummary, entry: &Entry) {
    match entry.kind {
        EntryKind::File | EntryKind::Archive => {
            summary.files += 1;
            summary.bytes = summary.bytes.saturating_add(entry.size.unwrap_or(0));
        }
        EntryKind::Link => summary.links += 1,
        EntryKind::Folder | EntryKind::Item | EntryKind::Special => summary.others += 1,
    }
}

/// The longest path inside `root` (the root's own included), in UTF-16
/// units: what the trash gate needs of a folder (spike four).
pub fn longest_path_units(
    provider: &dyn Provider,
    root: &Place,
    ctx: &OpContext,
) -> Result<usize, VfsError> {
    walk(provider, root, &WalkOptions::default(), ctx, &mut |_| {
        ControlFlow::Continue(())
    })
    .map(|summary| summary.longest_path_units)
}
