//! The rules every file system in an image is read under, kept once
//! (R29 1.8, R30 commit 95):
//!
//! - **Names.** Every stored name goes through the container rules of
//!   `diroxide_vfs::archive::names::member_path`: a control character is
//!   shown as U+FFFD and flagged, a name the rules refuse (`..`, one that
//!   is empty once made safe, one over the limits) is left out and
//!   counted, and so is one the rules would split into folders (a stored
//!   backslash) and a second entry that would show under a name already
//!   shown. A place names an entry by its shown name; the tree finds the
//!   stored one.
//! - **Limits,** per folder listing, since a file system lists folder by
//!   folder and no whole index is built: the entries, the names, the link
//!   targets.
//! - **Loops.** A folder whose identity (an extent, an inode, a cluster)
//!   is the identity of a folder it is inside is left out and counted
//!   ("Skipped a loop in the image"), and no path goes more than
//!   [`MAX_DEPTH`] folders deep.
//! - **No link is followed:** a link is a row with its target as text.
//! - **A small cache** of the folders listed last, so the reads of a file
//!   copied out in 1 MiB pieces do not list its folders each time.

use std::collections::HashSet;

use diroxide_vfs::archive::limits::Limits;
use diroxide_vfs::archive::names::{NameFlags, member_path};

use crate::error::DiskError;
use crate::image::fs::{Facts, FileSystem, Node, NodeKind};
use crate::lines::cause;

/// The deepest a path in an image may go, in folders.
pub const MAX_DEPTH: usize = 64;

/// The folders the cache keeps.
const CACHED_FOLDERS: usize = 32;

/// The entries the cache keeps across its folders.
const CACHED_ENTRIES: usize = 200_000;

/// One row of a listing: the name shown, and the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The name as the place and the row say it.
    pub shown: String,
    /// Whether the name needed care (control characters, not valid text,
    /// hidden characters).
    pub needs_care: bool,
    /// The node as stored.
    pub node: Node,
}

/// One folder's listing under the rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// The rows, in the file system's order.
    pub rows: Vec<Row>,
    /// Names the rules left out.
    pub names_skipped: usize,
    /// Folders left out because they lead back to one they are inside.
    pub loops_skipped: usize,
}

/// A resolved entry: its stored path and its node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The stored names from the root.
    pub path: Vec<Vec<u8>>,
    /// The node; the root's is a folder named empty.
    pub node: Node,
    /// The identities of the folders on the way, the root's first.
    pub ids: Vec<u64>,
}

/// A file system under the rules.
pub struct Tree {
    fs: Box<dyn FileSystem>,
    limits: Limits,
    cache: Vec<(Vec<Vec<u8>>, Listing)>,
}

impl Tree {
    /// The rules over a file system.
    pub fn new(fs: Box<dyn FileSystem>, limits: Limits) -> Self {
        Self {
            fs,
            limits,
            cache: Vec::new(),
        }
    }

    /// The file system's facts.
    pub fn facts(&self) -> Facts {
        self.fs.facts()
    }

    /// The listing of a folder by its stored path, from the cache when it
    /// is there. `ids` are the folders' identities on the way to it, its
    /// own last, for the loop check.
    fn listing(
        &mut self,
        path: &[Vec<u8>],
        folder: &Node,
        ids: &[u64],
    ) -> Result<Listing, DiskError> {
        if let Some(at) = self.cache.iter().position(|(p, _)| p == path) {
            let hit = self.cache.remove(at);
            let listing = hit.1.clone();
            self.cache.push(hit);
            return Ok(listing);
        }
        let nodes = self.fs.list(path, folder)?;
        if nodes.len() > self.limits.max_entries {
            return Err(DiskError::Refused(cause::too_many_entries(
                self.limits.max_entries,
            )));
        }
        let listing = self.apply(nodes, ids);
        let size = listing.rows.len();
        if size <= CACHED_ENTRIES {
            let held = |cache: &[(Vec<Vec<u8>>, Listing)]| -> usize {
                cache.iter().map(|(_, l)| l.rows.len()).sum()
            };
            while !self.cache.is_empty()
                && (self.cache.len() >= CACHED_FOLDERS || held(&self.cache) + size > CACHED_ENTRIES)
            {
                self.cache.remove(0);
            }
            self.cache.push((path.to_vec(), listing.clone()));
        }
        Ok(listing)
    }

    /// The rules applied to one folder's nodes.
    fn apply(&self, nodes: Vec<Node>, ids: &[u64]) -> Listing {
        let mut listing = Listing::default();
        let mut shown: HashSet<String> = HashSet::with_capacity(nodes.len());
        let ancestors: HashSet<u64> = ids.iter().copied().collect();
        for node in nodes {
            if node.name == b"." || node.name == b".." {
                continue;
            }
            let text = String::from_utf8_lossy(&node.name);
            let not_text = matches!(text, std::borrow::Cow::Owned(_));
            let Ok(made) = member_path(&text, &self.limits) else {
                listing.names_skipped += 1;
                continue;
            };
            if made.path.contains('/') {
                listing.names_skipped += 1;
                continue;
            }
            if let Some(target) = &node.link
                && target.len() > self.limits.max_link_target_bytes
            {
                listing.names_skipped += 1;
                continue;
            }
            if node.kind == NodeKind::Folder
                && let Some(id) = node.id
                && ancestors.contains(&id)
            {
                listing.loops_skipped += 1;
                continue;
            }
            // A `!` after an archive's extension would read back as a
            // nested archive in the place, so the row shows `_` there.
            let name = if made.flags.contains(NameFlags::ARCHIVE_MARKER) {
                made.path.replace('!', "_")
            } else {
                made.path
            };
            if !shown.insert(name.clone()) {
                listing.names_skipped += 1;
                continue;
            }
            listing.rows.push(Row {
                shown: name,
                needs_care: not_text || !made.flags.is_empty(),
                node,
            });
        }
        listing
    }

    /// Finds the entry at a shown path, walking from the root through the
    /// listings, so a name is found exactly as its row showed it.
    pub fn resolve(&mut self, shown: &[&str]) -> Result<Found, DiskError> {
        if shown.len() > MAX_DEPTH {
            return Err(DiskError::Refused(cause::too_deep(MAX_DEPTH)));
        }
        let root_id = self.fs.root_id().unwrap_or(u64::MAX);
        let mut found = Found {
            path: Vec::new(),
            node: Node {
                id: Some(root_id),
                ..Node::new(Vec::new(), NodeKind::Folder)
            },
            ids: vec![root_id],
        };
        for name in shown {
            if found.node.kind != NodeKind::Folder {
                return Err(DiskError::NotAFolder);
            }
            let listing = self.listing(&found.path, &found.node, &found.ids)?;
            let row = listing
                .rows
                .into_iter()
                .find(|row| row.shown == *name)
                .ok_or(DiskError::NotFound)?;
            found.path.push(row.node.name.clone());
            if row.node.kind == NodeKind::Folder {
                found
                    .ids
                    .push(row.node.id.unwrap_or(u64::MAX - found.ids.len() as u64));
            }
            found.node = row.node;
        }
        Ok(found)
    }

    /// The entry at a shown path with its full details.
    pub fn stat(&mut self, shown: &[&str]) -> Result<Found, DiskError> {
        let mut found = self.resolve(shown)?;
        if !shown.is_empty() {
            found.node = self.fs.detail(&found.path, &found.node)?;
        }
        Ok(found)
    }

    /// The listing of the folder at a shown path.
    pub fn list(&mut self, shown: &[&str]) -> Result<Listing, DiskError> {
        let found = self.resolve(shown)?;
        match found.node.kind {
            NodeKind::Folder => {}
            NodeKind::File | NodeKind::Link | NodeKind::Special => {
                return Err(DiskError::NotAFolder);
            }
        }
        if found.path.len() >= MAX_DEPTH {
            return Err(DiskError::Refused(cause::too_deep(MAX_DEPTH)));
        }
        self.listing(&found.path, &found.node, &found.ids)
    }

    /// Up to `length` bytes of the file at a shown path from `offset`.
    /// A link reads as its target's text, never the target.
    pub fn read(
        &mut self,
        shown: &[&str],
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError> {
        let found = self.resolve(shown)?;
        match found.node.kind {
            NodeKind::Folder => Err(DiskError::IsAFolder),
            NodeKind::Link => {
                let target = found.node.link.clone().unwrap_or_default();
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(target.len());
                let end = start.saturating_add(length).min(target.len());
                Ok(target[start..end].to_vec())
            }
            NodeKind::Special => Ok(Vec::new()),
            NodeKind::File => self.fs.read(&found.path, &found.node, offset, length),
        }
    }
}

#[cfg(test)]
pub mod tests {
    //! A file system in memory, for the tree's rules and the provider's
    //! tests.

    use std::collections::BTreeMap;

    use super::*;

    /// A file system in memory: folders by stored path, files' bytes.
    #[derive(Clone, Debug, Default)]
    pub struct Toy {
        /// Each folder's nodes, by its stored path joined with `/`.
        pub folders: BTreeMap<Vec<u8>, Vec<Node>>,
        /// Each file's bytes, by its stored path joined with `/`.
        pub files: BTreeMap<Vec<u8>, Vec<u8>>,
        /// How many listings reached the file system.
        pub lists: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    fn key(path: &[Vec<u8>]) -> Vec<u8> {
        path.join(&b'/')
    }

    impl Toy {
        /// Adds a folder under `parent` with an identity.
        pub fn folder(&mut self, parent: &[&[u8]], name: &[u8], id: u64) {
            let parent: Vec<Vec<u8>> = parent.iter().map(|p| p.to_vec()).collect();
            let mut node = Node::new(name, NodeKind::Folder);
            node.id = Some(id);
            self.folders.entry(key(&parent)).or_default().push(node);
            let mut own = parent;
            own.push(name.to_vec());
            self.folders.entry(key(&own)).or_default();
        }

        /// Adds a file under `parent`.
        pub fn file(&mut self, parent: &[&[u8]], name: &[u8], bytes: &[u8]) {
            let parent: Vec<Vec<u8>> = parent.iter().map(|p| p.to_vec()).collect();
            let mut node = Node::new(name, NodeKind::File);
            node.size = Some(bytes.len() as u64);
            self.folders.entry(key(&parent)).or_default().push(node);
            let mut own = parent;
            own.push(name.to_vec());
            self.files.insert(key(&own), bytes.to_vec());
        }
    }

    impl FileSystem for Toy {
        fn facts(&self) -> Facts {
            Facts {
                words: "toy".to_owned(),
                ..Facts::default()
            }
        }

        fn root_id(&self) -> Option<u64> {
            Some(1)
        }

        fn list(&mut self, dir: &[Vec<u8>], _folder: &Node) -> Result<Vec<Node>, DiskError> {
            self.lists.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.folders
                .get(&key(dir))
                .cloned()
                .ok_or(DiskError::NotFound)
        }

        fn read(
            &mut self,
            path: &[Vec<u8>],
            _node: &Node,
            offset: u64,
            length: usize,
        ) -> Result<Vec<u8>, DiskError> {
            let bytes = self.files.get(&key(path)).ok_or(DiskError::NotFound)?;
            let start = (offset as usize).min(bytes.len());
            let end = start.saturating_add(length).min(bytes.len());
            Ok(bytes[start..end].to_vec())
        }
    }

    fn names(listing: &Listing) -> Vec<&str> {
        listing.rows.iter().map(|r| r.shown.as_str()).collect()
    }

    #[test]
    fn names_are_made_safe_and_the_refused_ones_counted() {
        let mut toy = Toy::default();
        toy.file(&[], b"ok.txt", b"one");
        toy.file(&[], b"..", b"dot dot");
        toy.file(&[], b"a\\b", b"backslash");
        toy.file(&[], b"bell\x07.txt", b"control");
        toy.file(&[], b"bad\xff.txt", b"not text");
        toy.file(&[], b"ok.txt", b"again");
        toy.file(&[], &[b'x'; 300], b"long part");
        let mut tree = Tree::new(Box::new(toy), Limits::default());
        let listing = tree.list(&[]).unwrap();
        assert_eq!(
            names(&listing),
            ["ok.txt", "bell\u{FFFD}.txt", "bad\u{FFFD}.txt"]
        );
        assert!(listing.rows[1].needs_care && listing.rows[2].needs_care);
        assert!(!listing.rows[0].needs_care);
        assert_eq!(listing.names_skipped, 3);
        // A shown name finds the stored one.
        assert_eq!(
            tree.read(&["bell\u{FFFD}.txt"], 0, 100).unwrap(),
            b"control"
        );
        assert_eq!(tree.read(&["bad\u{FFFD}.txt"], 1, 2).unwrap(), b"ot");
        assert_eq!(tree.read(&["missing"], 0, 1), Err(DiskError::NotFound));
    }

    #[test]
    fn a_folder_that_leads_back_is_cut_and_the_depth_is_capped() {
        let mut toy = Toy::default();
        toy.folder(&[], b"a", 10);
        toy.folder(&[b"a"], b"b", 11);
        // b holds a folder whose identity is a's: a loop.
        toy.folder(&[b"a", b"b"], b"back", 10);
        toy.folder(&[b"a", b"b"], b"fine", 12);
        let mut tree = Tree::new(Box::new(toy), Limits::default());
        let listing = tree.list(&["a", "b"]).unwrap();
        assert_eq!(names(&listing), ["fine"]);
        assert_eq!(listing.loops_skipped, 1);
        let deep: Vec<&str> = vec!["a"; MAX_DEPTH + 1];
        assert!(matches!(tree.list(&deep), Err(DiskError::Refused(_))));
        assert_eq!(tree.list(&["a", "missing"]), Err(DiskError::NotFound));
    }

    #[test]
    fn too_many_entries_in_one_folder_are_refused() {
        let mut toy = Toy::default();
        for i in 0..20 {
            toy.file(&[], format!("f{i}").as_bytes(), b"");
        }
        let limits = Limits {
            max_entries: 10,
            ..Limits::default()
        };
        let mut tree = Tree::new(Box::new(toy), limits);
        assert_eq!(
            tree.list(&[]),
            Err(DiskError::Refused(cause::too_many_entries(10)))
        );
    }

    #[test]
    fn a_link_reads_as_its_target_never_the_target() {
        let mut toy = Toy::default();
        let mut link = Node::new(b"etc-passwd".to_vec(), NodeKind::Link);
        link.link = Some(b"/etc/passwd".to_vec());
        toy.folders.entry(Vec::new()).or_default().push(link);
        let mut tree = Tree::new(Box::new(toy), Limits::default());
        assert_eq!(tree.read(&["etc-passwd"], 0, 100).unwrap(), b"/etc/passwd");
    }

    #[test]
    fn repeated_reads_of_one_file_list_its_folders_once() {
        let mut toy = Toy::default();
        toy.folder(&[], b"docs", 5);
        toy.file(&[b"docs"], b"big.bin", &[7u8; 1000]);
        let lists = std::sync::Arc::clone(&toy.lists);
        let mut tree = Tree::new(Box::new(toy), Limits::default());
        for offset in (0..1000).step_by(100) {
            assert_eq!(
                tree.read(&["docs", "big.bin"], offset, 100).unwrap().len(),
                100
            );
        }
        assert_eq!(lists.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
