//! A small in-memory place for the search's tests, served at
//! `fake://box/...`: folders, files with their bytes and times, links,
//! a delay before every listing and one folder whose listing stalls, and
//! a count of the calls, so a test can see that no thread is still
//! walking after a search returns. It never touches a disk.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diroxide_core::{Attributes, Entry, EntryKind, Link, Place, Timestamp};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, VfsError, WriteMode,
};

/// What is at a path.
#[derive(Clone, Debug)]
pub enum Node {
    /// A folder.
    Folder,
    /// A file, its bytes and when it was changed.
    File(Vec<u8>, Timestamp),
    /// A link and its target, as stored.
    Link(String),
}

/// The in-memory place.
#[derive(Debug, Default)]
pub struct Fake {
    nodes: Mutex<BTreeMap<String, Node>>,
    /// A pause before every listing.
    pub delay: Mutex<Duration>,
    /// A folder whose listing waits this long.
    pub stall: Mutex<Option<(String, Duration)>>,
    /// Files whose data is not on this computer.
    online: Mutex<BTreeSet<String>>,
    /// Folders whose listing is refused.
    refused: Mutex<BTreeSet<String>>,
    calls: AtomicU64,
}

/// A moment in October 2026, for files given no time.
pub const WHEN: Timestamp = Timestamp::from_unix(1_791_000_000, 0);

impl Fake {
    /// An empty place with its top folder.
    pub fn new() -> Arc<Self> {
        let fake = Self::default();
        fake.nodes.lock().unwrap().insert("/".into(), Node::Folder);
        Arc::new(fake)
    }

    /// The place of a path: `fake://box/a/b`.
    pub fn place(path: &str) -> Place {
        Place::parse(&format!("fake://box{path}")).unwrap()
    }

    /// A registry that serves `fake://` from this place.
    pub fn registry(self: &Arc<Self>) -> Registry {
        let mut registry = Registry::with_local();
        registry.register("fake", Arc::clone(self) as Arc<dyn Provider>);
        registry
    }

    fn add(&self, path: &str, node: Node) {
        let mut nodes = self.nodes.lock().unwrap();
        let mut at = String::new();
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            at.push('/');
            at.push_str(part);
            nodes.entry(at.clone()).or_insert(Node::Folder);
        }
        nodes.insert(path.to_owned(), node);
    }

    /// Makes a folder and its parents.
    pub fn folder(&self, path: &str) {
        self.add(path, Node::Folder);
    }

    /// Makes a file and its folders.
    pub fn file(&self, path: &str, bytes: &[u8]) {
        self.add(path, Node::File(bytes.to_vec(), WHEN));
    }

    /// Marks a file as online only, as a cloud placeholder is.
    pub fn online_only(&self, path: &str) {
        self.online.lock().unwrap().insert(path.to_owned());
    }

    /// Makes a folder's listing refused, as a folder without the right
    /// to read it is.
    pub fn refuse(&self, path: &str) {
        self.folder(path);
        self.refused.lock().unwrap().insert(path.to_owned());
    }

    /// Makes a link.
    pub fn link(&self, path: &str, target: &str) {
        self.add(path, Node::Link(target.to_owned()));
    }

    /// Sets the pause before every listing.
    pub fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap() = delay;
    }

    /// Makes one folder's listing wait.
    pub fn stall_at(&self, path: &str, wait: Duration) {
        *self.stall.lock().unwrap() = Some((path.to_owned(), wait));
    }

    /// Calls so far.
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    fn path_of(place: &Place) -> Result<String, VfsError> {
        place
            .remote()
            .map(|remote| remote.path().to_owned())
            .ok_or(VfsError::NotFound)
    }

    fn entry(&self, path: &str, node: &Node) -> Entry {
        let name = path.rsplit('/').next().unwrap_or("").to_owned();
        let hidden = name.starts_with('.');
        let mut entry = match node {
            Node::Folder => Entry::new(name, EntryKind::Folder),
            Node::File(bytes, modified) => {
                let kind = if path.ends_with(".zip") {
                    EntryKind::Archive
                } else {
                    EntryKind::File
                };
                Entry::new(name, kind)
                    .with_size(bytes.len() as u64)
                    .with_modified(*modified)
            }
            Node::Link(target) => {
                let mut entry = Entry::new(name, EntryKind::Link);
                let points_to = self
                    .nodes
                    .lock()
                    .unwrap()
                    .get(target)
                    .map(|node| match node {
                        Node::Folder => EntryKind::Folder,
                        Node::File(..) => EntryKind::File,
                        Node::Link(_) => EntryKind::Link,
                    });
                entry.link = Some(Link {
                    target: target.clone(),
                    points_to,
                });
                entry
            }
        };
        entry.hidden = hidden;
        if self.online.lock().unwrap().contains(path) {
            entry.attributes = Some(Attributes::Windows(0x40_0000));
        }
        entry
    }
}

impl Provider for Fake {
    fn scheme(&self) -> &str {
        "fake"
    }

    fn capabilities(&self, _: &Place) -> Capabilities {
        Capabilities::read_only()
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ctx.check()?;
        let path = Self::path_of(place)?;
        std::thread::sleep(*self.delay.lock().unwrap());
        let stall = self.stall.lock().unwrap().clone();
        if let Some((at, wait)) = stall
            && at == path
        {
            std::thread::sleep(wait);
        }
        if self.refused.lock().unwrap().contains(&path) {
            return Err(VfsError::PermissionDenied);
        }
        let nodes = self.nodes.lock().unwrap().clone();
        match nodes.get(&path) {
            Some(Node::Folder) => {}
            Some(_) => return Err(VfsError::NotAFolder),
            None => return Err(VfsError::NotFound),
        }
        let prefix = if path == "/" {
            "/".to_owned()
        } else {
            format!("{path}/")
        };
        Ok(nodes
            .iter()
            .filter(|(key, _)| {
                key.strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
            })
            .map(|(key, node)| self.entry(key, node))
            .collect())
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ctx.check()?;
        let path = Self::path_of(place)?;
        let node = self.nodes.lock().unwrap().get(&path).cloned();
        match node {
            Some(node) => Ok(self.entry(&path, &node)),
            None => Err(VfsError::NotFound),
        }
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ctx.check()?;
        let path = Self::path_of(place)?;
        match self.nodes.lock().unwrap().get(&path) {
            Some(Node::File(bytes, _)) => Ok(Box::new(Cursor::new(bytes.clone()))),
            Some(_) => Err(VfsError::unsupported("fake", "read a folder")),
            None => Err(VfsError::NotFound),
        }
    }

    fn write(
        &self,
        _: &Place,
        _: WriteMode,
        _: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(VfsError::unsupported("fake", "write"))
    }

    fn create_dir(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::unsupported("fake", "make folders"))
    }

    fn rename(&self, _: &Place, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::unsupported("fake", "rename"))
    }

    fn remove(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::unsupported("fake", "remove"))
    }

    fn set_attributes(
        &self,
        _: &Place,
        _: &AttributeChange,
        _: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(VfsError::unsupported("fake", "change attributes"))
    }
}
