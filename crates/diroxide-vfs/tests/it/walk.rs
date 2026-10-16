//! The walk helper and the boundary rule: links never entered, other
//! volumes, loops, depth, snapshots and unreadable folders skipped with
//! words; folders entered and left in order. Hostile names live only in
//! the in-memory tree here, never on disk (the Wave 1 rule).

use std::collections::HashMap;
use std::io::Read;
use std::ops::ControlFlow;

use diroxide_core::{Entry, EntryKind, Link, Place};
use diroxide_vfs::walk::{SkipReason, WalkEvent, WalkOptions, longest_path_units, walk};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, LocalProvider, OpContext, Provider, VfsError,
    WriteMode,
};

use crate::scratch;

/// An in-memory tree: each folder's text form maps to its entries.
#[derive(Debug, Default)]
struct Tree {
    folders: HashMap<String, Vec<Entry>>,
    unreadable: Vec<String>,
}

fn base() -> Place {
    Place::parse(if cfg!(windows) { "D:\\tree" } else { "/tree" }).unwrap()
}

fn folder(name: &str, volume: u64, inode: u64) -> Entry {
    let mut entry = Entry::new(name, EntryKind::Folder);
    entry.volume = Some(volume);
    entry.inode = Some(inode);
    entry
}

fn file(name: &str, size: u64) -> Entry {
    Entry::new(name, EntryKind::File).with_size(size)
}

impl Tree {
    fn add(&mut self, place: &Place, entries: Vec<Entry>) {
        self.folders.insert(place.to_string(), entries);
    }
}

impl Provider for Tree {
    fn scheme(&self) -> &str {
        "tree"
    }
    fn capabilities(&self, _: &Place) -> Capabilities {
        Capabilities::read_only()
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        ctx.check()?;
        if self.unreadable.contains(&place.to_string()) {
            return Err(VfsError::PermissionDenied);
        }
        self.folders
            .get(&place.to_string())
            .cloned()
            .ok_or(VfsError::NotFound)
    }
    fn stat(&self, place: &Place, _: &OpContext) -> Result<Entry, VfsError> {
        if *place == base() {
            return Ok(folder("tree", 1, 1));
        }
        let parent = place.parent().ok_or(VfsError::NotFound)?;
        let name = place.name().ok_or(VfsError::NotFound)?;
        self.folders
            .get(&parent.to_string())
            .and_then(|entries| entries.iter().find(|e| e.name == name).cloned())
            .ok_or(VfsError::NotFound)
    }
    fn read(&self, _: &Place, _: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Err(VfsError::NotFound)
    }
    fn write(
        &self,
        _: &Place,
        _: WriteMode,
        _: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn create_dir(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn rename(&self, _: &Place, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn remove(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn set_attributes(
        &self,
        _: &Place,
        _: &AttributeChange,
        _: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
}

/// The steps of a walk as short strings: "enter tree", "item a.txt",
/// "leave tree", "skip usb: another drive is mounted here".
fn steps(provider: &dyn Provider, root: &Place, options: &WalkOptions) -> Vec<String> {
    let mut seen = Vec::new();
    walk(provider, root, options, &OpContext::new(), &mut |event| {
        seen.push(match event {
            WalkEvent::Enter { entry, .. } => format!("enter {}", entry.name),
            WalkEvent::Item { entry, .. } => format!("item {}", entry.name),
            WalkEvent::Leave { entry, .. } => format!("leave {}", entry.name),
            WalkEvent::Skipped { entry, reason, .. } => reason.line(&entry.name),
        });
        ControlFlow::Continue(())
    })
    .unwrap();
    seen
}

#[test]
fn the_boundary_rule_holds_in_a_tree() {
    let root = base();
    let mut tree = Tree::default();
    let mut link = Entry::new("out", EntryKind::Link);
    link.link = Some(Link {
        target: "/etc".into(),
        points_to: Some(EntryKind::Folder),
    });
    tree.add(
        &root,
        vec![
            file("a.txt", 3),
            // Hostile names, in memory only: a private-use and a control
            // character.
            file("C\u{F03A}", 1),
            file("bell\u{7}", 1),
            link,
            folder("usb", 2, 10),
            folder("photos", 1, 11),
            folder(".snapshots", 1, 12),
            folder("locked", 1, 13),
        ],
    );
    let photos = root.join("photos").unwrap();
    tree.add(&photos, vec![file("b.jpg", 5), folder("again", 1, 1)]);
    tree.unreadable
        .push(root.join("locked").unwrap().to_string());
    let seen = steps(&tree, &root, &WalkOptions::default());
    assert_eq!(
        seen,
        vec![
            "enter tree",
            "item a.txt",
            "item C\u{F03A}",
            "item bell\u{7}",
            "item out",
            "Skipped usb: another drive is mounted here",
            "enter photos",
            "item b.jpg",
            "Skipped again: it leads back to a folder already seen",
            "leave photos",
            "Skipped .snapshots: it is a snapshot folder",
            "Skipped locked: it could not be read: permission was denied",
            "leave tree",
        ]
    );
    let summary = walk(
        &tree,
        &root,
        &WalkOptions::default(),
        &OpContext::new(),
        &mut |_| ControlFlow::Continue(()),
    )
    .unwrap();
    assert_eq!(
        (
            summary.folders,
            summary.files,
            summary.links,
            summary.skipped
        ),
        (2, 4, 1, 4)
    );
    assert_eq!(summary.bytes, 10);
    assert_eq!(summary.deepest, 2);
}

#[test]
fn a_tree_300_levels_deep_stops_with_a_reason() {
    let root = base();
    let mut tree = Tree::default();
    let mut place = root.clone();
    for level in 0..300u64 {
        tree.add(&place, vec![folder("d", 1, 100 + level)]);
        place = place.join("d").unwrap();
    }
    tree.add(&place, Vec::new());
    let seen = steps(&tree, &root, &WalkOptions::default());
    assert_eq!(seen.iter().filter(|s| s.starts_with("enter")).count(), 257);
    assert!(seen.contains(&"Skipped d: it is more than 256 levels deep".to_owned()));
    let shallow = WalkOptions::default().with_max_depth(2);
    let seen = steps(&tree, &root, &shallow);
    assert_eq!(seen.iter().filter(|s| s.starts_with("enter")).count(), 3);
}

#[test]
fn a_walk_stops_when_asked_and_when_cancelled() {
    let root = base();
    let mut tree = Tree::default();
    tree.add(&root, vec![file("a", 1), file("b", 1), file("c", 1)]);
    let mut count = 0;
    walk(
        &tree,
        &root,
        &WalkOptions::default(),
        &OpContext::new(),
        &mut |event| {
            count += 1;
            if matches!(event, WalkEvent::Item { .. }) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        },
    )
    .unwrap();
    assert_eq!(count, 2, "enter, then the first item");
    let ctx = OpContext::new();
    ctx.cancel.cancel();
    assert_eq!(
        walk(&tree, &root, &WalkOptions::default(), &ctx, &mut |_| {
            ControlFlow::Continue(())
        }),
        Err(VfsError::Cancelled)
    );
}

#[test]
fn a_real_folder_is_walked_deepest_last_and_measured() {
    let scratch = scratch();
    let root = crate::local::place_of(&scratch);
    std::fs::create_dir_all(scratch.path().join("a").join("b")).unwrap();
    std::fs::write(
        scratch.path().join("a").join("b").join("deep.txt"),
        b"12345",
    )
    .unwrap();
    std::fs::write(scratch.path().join("top.txt"), b"1").unwrap();
    let provider = LocalProvider::new();
    let mut leaves = Vec::new();
    let summary = walk(
        &provider,
        &root,
        &WalkOptions::default(),
        &OpContext::new(),
        &mut |event| {
            if let WalkEvent::Leave { entry, .. } = event {
                leaves.push(entry.name.clone());
            }
            ControlFlow::Continue(())
        },
    )
    .unwrap();
    // Folders are left deepest first: the order a delete needs.
    assert_eq!(&leaves[..2], &["b".to_owned(), "a".to_owned()]);
    assert_eq!((summary.files, summary.bytes, summary.folders), (2, 6, 3));
    let deep = root
        .join("a")
        .unwrap()
        .join("b")
        .unwrap()
        .join("deep.txt")
        .unwrap();
    assert_eq!(
        longest_path_units(&provider, &root, &OpContext::new()).unwrap(),
        diroxide_vfs::limits::utf16_units(&deep.to_string())
    );
}

#[cfg(unix)]
#[test]
fn links_out_of_the_root_and_back_up_are_never_followed() {
    use std::os::unix::fs::symlink;
    let scratch = scratch();
    let root = crate::local::place_of(&scratch);
    std::fs::create_dir(scratch.path().join("inside")).unwrap();
    symlink("..", scratch.path().join("inside").join("up")).unwrap();
    symlink("/", scratch.path().join("out")).unwrap();
    let seen = steps(&LocalProvider::new(), &root, &WalkOptions::default());
    assert!(seen.contains(&"item up".to_owned()), "{seen:?}");
    assert!(seen.contains(&"item out".to_owned()), "{seen:?}");
    assert_eq!(seen.iter().filter(|s| s.starts_with("enter")).count(), 2);
}

#[cfg(windows)]
#[test]
fn a_junction_back_up_is_never_followed() {
    use diroxide_vfs::{LinkKind, LinkRequest};
    let scratch = scratch();
    let root = crate::local::place_of(&scratch);
    std::fs::create_dir(scratch.path().join("inside")).unwrap();
    let request = LinkRequest {
        kind: LinkKind::Junction,
        target: scratch.path().to_str().unwrap().to_owned(),
        target_is_folder: true,
    };
    let up = root.join("inside").unwrap().join("up").unwrap();
    LocalProvider::new()
        .create_link(&up, &request, &OpContext::new())
        .unwrap();
    let seen = steps(&LocalProvider::new(), &root, &WalkOptions::default());
    assert!(seen.contains(&"item up".to_owned()), "{seen:?}");
    assert_eq!(seen.iter().filter(|s| s.starts_with("enter")).count(), 2);
}

#[test]
fn skip_reasons_say_why_in_words() {
    assert_eq!(
        SkipReason::OtherVolume.line("usb"),
        "Skipped usb: another drive is mounted here"
    );
    assert!(diroxide_core::format::within_cells(
        &SkipReason::TooDeep.line("photos"),
        "Skipped photos"
    ));
}
