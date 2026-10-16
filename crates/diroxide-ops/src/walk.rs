//! The walk: every entry under a source, through its provider, with the
//! boundary rule (plan section 5.12; research/file-operations.md, "Symlink
//! loops and the boundary rule").
//!
//! - A link is an entry of its own and is never descended into: a link
//!   out of the root is copied, moved or removed as a link.
//! - A folder on another volume than the root's (a mount point, a bind
//!   mount) is a boundary: it is left out and reported.
//! - A folder already walked, by its volume and inode, is a loop: left out
//!   and reported.
//! - Below the depth limit the walk stops and reports; a tree too deep is
//!   an error in words, never a crash (the walk keeps its own stack).
//! - An unfinished write (`.diro-part-` names) is another job's, and is
//!   left alone and reported.
//!
//! The walk is pre-order: a folder comes before what is in it, and the
//! names in a folder come in a stable order.

use std::collections::HashSet;

use diroxide_core::{Entry, EntryKind, Place};
use diroxide_vfs::names::is_unfinished;
use diroxide_vfs::{OpContext, Provider, VfsError};

/// One entry the walk found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// Where it is.
    pub place: Place,
    /// What it is, as the walk saw it.
    pub entry: Entry,
    /// The names from the root down to it; empty for the root.
    pub path: Vec<String>,
}

impl Found {
    /// How deep it is below the root.
    pub fn depth(&self) -> usize {
        self.path.len()
    }
}

/// What a walk found, and what it left out with the reason in words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Walked {
    /// Every entry, the root first, in pre-order.
    pub found: Vec<Found>,
    /// The entries left out, each with its reason.
    pub left_out: Vec<(Place, String)>,
}

/// The reason words, said in the result list.
pub(crate) const OTHER_DRIVE: &str = "another drive is mounted here";
/// A folder walked twice.
pub(crate) const LOOP: &str = "a loop: this folder was already listed";
/// An unfinished write.
pub(crate) const UNFINISHED: &str = "an unfinished copy, left alone";

/// Walks one source. `count` is told after each entry and may stop the
/// walk with an error (cancel).
pub(crate) fn walk(
    provider: &dyn Provider,
    root: &Place,
    root_entry: Entry,
    ctx: &OpContext,
    max_depth: usize,
    count: &mut dyn FnMut(u64) -> Result<(), VfsError>,
) -> Result<Walked, VfsError> {
    let mut walked = Walked::default();
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let root_volume = root_entry.volume;
    if let (Some(volume), Some(inode)) = (root_entry.volume, root_entry.inode) {
        seen.insert((volume, inode));
    }
    let descend = root_entry.kind == EntryKind::Folder;
    let mut found = vec![Found {
        place: root.clone(),
        entry: root_entry,
        path: Vec::new(),
    }];
    let mut total: u64 = 1;
    count(total)?;
    let mut queue: Vec<(Place, Vec<String>)> = Vec::new();
    if descend {
        queue.push((root.clone(), Vec::new()));
    }
    while let Some((folder, path)) = queue.pop() {
        ctx.check()?;
        let entries = match provider.list(&folder, ctx) {
            Ok(entries) => entries,
            Err(VfsError::Cancelled) => return Err(VfsError::Cancelled),
            Err(error) => {
                walked
                    .left_out
                    .push((folder.clone(), format!("could not be read: {error}")));
                continue;
            }
        };
        for entry in entries {
            let Ok(place) = folder.join(&entry.name) else {
                walked.left_out.push((
                    folder.clone(),
                    format!("a name inside cannot be used: {}", entry.name),
                ));
                continue;
            };
            if is_unfinished(&entry.name) {
                walked.left_out.push((place, UNFINISHED.to_owned()));
                continue;
            }
            if entry.name_not_unicode {
                walked
                    .left_out
                    .push((place, "the name is not valid Unicode".to_owned()));
                continue;
            }
            if !entry.kind.is_file_system() {
                continue;
            }
            let mut child_path = path.clone();
            child_path.push(entry.name.clone());
            if entry.kind == EntryKind::Folder {
                if let (Some(root), Some(here)) = (root_volume, entry.volume)
                    && root != here
                {
                    walked.left_out.push((place, OTHER_DRIVE.to_owned()));
                    continue;
                }
                if let (Some(volume), Some(inode)) = (entry.volume, entry.inode)
                    && !seen.insert((volume, inode))
                {
                    walked.left_out.push((place, LOOP.to_owned()));
                    continue;
                }
                if child_path.len() > max_depth {
                    walked
                        .left_out
                        .push((place, format!("too deep: more than {max_depth} levels")));
                    continue;
                }
                queue.push((place.clone(), child_path.clone()));
            }
            total += 1;
            count(total)?;
            found.push(Found {
                place,
                entry,
                path: child_path,
            });
        }
    }
    walked.found = order_preorder(found);
    Ok(walked)
}

/// Puts entries in pre-order: each folder right before what is inside it,
/// names in their sorted order. Entries arrive grouped by folder in the
/// order the folders were listed; sorting by the path's names gives the
/// same order a recursive walk would.
fn order_preorder(mut found: Vec<Found>) -> Vec<Found> {
    found.sort_by(|a, b| {
        let fold = |path: &[String]| -> Vec<(String, String)> {
            path.iter()
                .map(|name| (name.to_lowercase(), name.clone()))
                .collect()
        };
        fold(&a.path).cmp(&fold(&b.path))
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeProvider, FakeSwitches};

    fn p(text: &str) -> Place {
        Place::parse(&if cfg!(windows) {
            format!("D:{}", text.replace('/', "\\"))
        } else {
            text.to_owned()
        })
        .unwrap()
    }

    fn walk_all(fake: &FakeProvider, root: &Place, depth: usize) -> Walked {
        let ctx = OpContext::new();
        let entry = fake.stat(root, &ctx).unwrap();
        walk(fake, root, entry, &ctx, depth, &mut |_| Ok(())).unwrap()
    }

    fn names(walked: &Walked) -> Vec<String> {
        walked.found.iter().map(|f| f.path.join("/")).collect()
    }

    #[test]
    fn a_walk_is_preorder_and_sorted() {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/b.txt"), b"1");
        fake.add_file(&p("/a/C/d.txt"), b"2");
        fake.add_file(&p("/a/a/e.txt"), b"3");
        let walked = walk_all(&fake, &p("/a"), 256);
        assert_eq!(
            names(&walked),
            vec!["", "a", "a/e.txt", "b.txt", "C", "C/d.txt"]
        );
        assert!(walked.left_out.is_empty());
    }

    #[test]
    fn links_are_never_followed_out_of_the_root() {
        let fake = FakeProvider::new();
        fake.add_file(&p("/outside/secret.txt"), b"x");
        fake.add_link(&p("/a/out"), &p("/outside").to_string(), true);
        let walked = walk_all(&fake, &p("/a"), 256);
        assert_eq!(names(&walked), vec!["", "out"]);
        assert_eq!(walked.found[1].entry.kind, EntryKind::Link);
    }

    #[test]
    fn another_volume_and_loops_are_left_out_and_said() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            second_volume: Some(p("/a/usb")),
            ..FakeSwitches::default()
        });
        fake.add_file(&p("/a/usb/x"), b"1");
        fake.add_file(&p("/a/y"), b"2");
        fake.add_folder_alias(&p("/a/loop"), &p("/a"));
        let walked = walk_all(&fake, &p("/a"), 256);
        assert_eq!(names(&walked), vec!["", "y"]);
        let reasons: Vec<&str> = walked.left_out.iter().map(|(_, r)| r.as_str()).collect();
        assert!(reasons.contains(&OTHER_DRIVE));
        assert!(reasons.contains(&LOOP));
    }

    #[test]
    fn a_tree_too_deep_ends_with_a_reason() {
        let fake = FakeProvider::new();
        let mut place = p("/deep");
        for level in 0..300 {
            place = place.join(&format!("l{level}")).unwrap();
        }
        fake.add_file(&place.join("bottom.txt").unwrap(), b"1");
        let walked = walk_all(&fake, &p("/deep"), 256);
        assert_eq!(walked.found.len(), 257);
        assert_eq!(walked.left_out.len(), 1);
        assert!(walked.left_out[0].1.starts_with("too deep"));
    }

    #[test]
    fn unfinished_writes_are_left_alone() {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/.diro-part-0000beef"), b"half");
        fake.add_file(&p("/a/whole.txt"), b"1");
        let walked = walk_all(&fake, &p("/a"), 256);
        assert_eq!(names(&walked), vec!["", "whole.txt"]);
        assert_eq!(walked.left_out[0].1, UNFINISHED);
    }

    #[test]
    fn a_cancel_stops_the_walk() {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/b"), b"1");
        let ctx = OpContext::new();
        let entry = fake.stat(&p("/a"), &ctx).unwrap();
        let result = walk(&fake, &p("/a"), entry, &ctx, 256, &mut |n| {
            if n > 1 {
                Err(VfsError::Cancelled)
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err(VfsError::Cancelled));
    }
}
