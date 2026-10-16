//! A fake provider: an in-memory tree with switches for the places and the
//! failures the engine must survive (research/file-operations.md, the
//! fake provider; R23). It never touches a disk, a trash or a network, so
//! the engine's tests, the property tests and the bench's slow-place
//! scenario run in CI.
//!
//! The switches:
//!
//! - Place rules: case-insensitive names, Windows reserved names, no
//!   trash, a second volume (a rename across it fails with
//!   `CrossesDevices`), read-only and locked entries, a largest file, a
//!   free-space budget.
//! - Timing ([`Latency`]): a delay on every call, and a stall that holds
//!   one call for a while after a number of calls.
//! - Lies ([`Lies`]): a commit that reports success without writing, a
//!   clone that fails after it has started writing, a trash that reports
//!   the item in the trash and does nothing.
//! - Faults ([`Fault`]): a chosen error at a place, optionally after N
//!   bytes of a read or a write.
//!
//! Phase 0 bodies are naive but whole enough for the engine's tests; W1b
//! completes the switches it needs and tests each one.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use diroxide_core::{Attributes, Entry, EntryKind, Place, Timestamp};
use diroxide_vfs::{
    AttributeChange, Capabilities, CopyHint, FileWriter, LinkKind, LinkRequest, OpContext,
    Provider, ReadSeek, SpaceInfo, TrashCheck, TrashGate, VfsError, VolumeInfo, VolumeKind,
    WriteMode,
};

/// Delays that make a place slow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Latency {
    /// A delay at the start of every call.
    pub per_call: Duration,
    /// After this many calls, one call stalls for `stall_for`.
    pub stall_after_calls: Option<u32>,
    /// How long the stalled call waits.
    pub stall_for: Duration,
}

/// The ways a provider can say yes and not do it (R23).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lies {
    /// A write's commit reports success and stores nothing.
    pub write_without_writing: bool,
    /// A clone (`copy_within`) writes part of the target, then fails.
    pub clone_fails_late: bool,
    /// The trash reports the item in the trash and leaves it where it was.
    pub trash_does_nothing: bool,
}

/// A failure at a place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    /// Where it happens.
    pub place: Place,
    /// After how many bytes of a read or a write; `None` at once, on any
    /// call at the place.
    pub at_byte: Option<u64>,
    /// The error it gives.
    pub error: VfsError,
}

/// The switches of a fake provider.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FakeSwitches {
    /// Names that differ only by case are the same entry.
    pub case_insensitive: bool,
    /// Windows's name rules apply.
    pub reserved_names: bool,
    /// The place has no trash.
    pub no_trash: bool,
    /// Places within this one are on a second volume.
    pub second_volume: Option<Place>,
    /// The volume's kind, for the trash gate (fixed by default).
    pub volume_kind: Option<VolumeKind>,
    /// Entries that cannot be written, renamed or removed.
    pub read_only: Vec<Place>,
    /// Entries another program holds open.
    pub locked: Vec<Place>,
    /// The largest file the place takes.
    pub max_file_size: Option<u64>,
    /// The bytes still free.
    pub free_space: Option<u64>,
    /// Clones are possible (`copy_within`).
    pub clone_copy: bool,
    /// The time new entries get.
    pub now: Option<Timestamp>,
    /// Slowness.
    pub latency: Latency,
    /// Lies.
    pub lies: Lies,
    /// Failures.
    pub faults: Vec<Fault>,
}

#[derive(Clone, Debug)]
struct Node {
    entry: Entry,
    data: Vec<u8>,
}

#[derive(Debug, Default)]
struct State {
    nodes: BTreeMap<String, Node>,
    trash: Vec<(String, Node)>,
    calls: u32,
}

/// An in-memory provider for tests. Its places are local places in the
/// platform's own style; the roots exist from the start.
#[derive(Debug, Default, Clone)]
pub struct FakeProvider {
    state: Arc<Mutex<State>>,
    switches: Arc<Mutex<FakeSwitches>>,
}

fn key_of(place: &Place, insensitive: bool) -> String {
    let text = place.to_string();
    if insensitive {
        text.to_lowercase()
    } else {
        text
    }
}

impl FakeProvider {
    /// An empty fake with no switches set.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake with these switches.
    pub fn with_switches(switches: FakeSwitches) -> Self {
        let fake = Self::new();
        *fake.switches_mut() = switches;
        fake
    }

    /// The switches, to change them during a test.
    pub fn switches_mut(&self) -> MutexGuard<'_, FakeSwitches> {
        self.switches.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn switches(&self) -> FakeSwitches {
        self.switches_mut().clone()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a folder, and its parents.
    pub fn add_folder(&self, place: &Place) {
        if let Some(parent) = place.parent()
            && !parent.is_root()
        {
            self.add_folder(&parent);
        }
        if place.is_root() {
            return;
        }
        let insensitive = self.switches().case_insensitive;
        let name = place.name().unwrap_or_default().to_owned();
        self.state()
            .nodes
            .entry(key_of(place, insensitive))
            .or_insert_with(|| Node {
                entry: Entry::new(name, EntryKind::Folder),
                data: Vec::new(),
            });
    }

    /// Adds a file with these bytes, and its parent folders.
    pub fn add_file(&self, place: &Place, data: &[u8]) {
        if let Some(parent) = place.parent() {
            self.add_folder(&parent);
        }
        let switches = self.switches();
        let mut entry = Entry::new(place.name().unwrap_or_default(), EntryKind::File)
            .with_size(data.len() as u64);
        entry.modified = switches.now;
        self.state().nodes.insert(
            key_of(place, switches.case_insensitive),
            Node {
                entry,
                data: data.to_vec(),
            },
        );
    }

    /// The bytes of a file, if it exists.
    pub fn contents(&self, place: &Place) -> Option<Vec<u8>> {
        let insensitive = self.switches().case_insensitive;
        self.state()
            .nodes
            .get(&key_of(place, insensitive))
            .map(|node| node.data.clone())
    }

    /// Whether an entry exists.
    pub fn exists(&self, place: &Place) -> bool {
        let insensitive = self.switches().case_insensitive;
        place.is_root() || self.state().nodes.contains_key(&key_of(place, insensitive))
    }

    /// How many entries are in the fake trash.
    pub fn trashed(&self) -> usize {
        self.state().trash.len()
    }

    /// Delays, stalls and faults at the start of a call.
    fn enter(&self, place: &Place, ctx: &OpContext) -> Result<FakeSwitches, VfsError> {
        ctx.check()?;
        let switches = self.switches();
        let calls = {
            let mut state = self.state();
            state.calls += 1;
            state.calls
        };
        if !switches.latency.per_call.is_zero() {
            thread::sleep(switches.latency.per_call);
        }
        if switches.latency.stall_after_calls == Some(calls) {
            thread::sleep(switches.latency.stall_for);
        }
        if let Some(fault) = switches
            .faults
            .iter()
            .find(|f| &f.place == place && f.at_byte.is_none())
        {
            return Err(fault.error.clone());
        }
        ctx.check()?;
        Ok(switches)
    }

    fn volume_of(switches: &FakeSwitches, place: &Place) -> u64 {
        match &switches.second_volume {
            Some(root) if place.is_within(root) => 2,
            _ => 1,
        }
    }
}

/// A reader that fails after a number of bytes.
struct FaultyReader {
    inner: Cursor<Vec<u8>>,
    fail_at: Option<u64>,
    error: Option<VfsError>,
}

impl Read for FaultyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let (Some(at), Some(error)) = (self.fail_at, &self.error)
            && self.inner.position() >= at
        {
            return Err(io::Error::other(error.to_string()));
        }
        let limit = match self.fail_at {
            Some(at) => usize::try_from(at - self.inner.position())
                .unwrap_or(usize::MAX)
                .min(buf.len()),
            None => buf.len(),
        };
        self.inner.read(&mut buf[..limit])
    }
}

/// The fake's committing writer.
struct FakeWriter {
    fake: FakeProvider,
    place: Place,
    mode: WriteMode,
    buffer: Vec<u8>,
    fail_at: Option<(u64, VfsError)>,
}

impl Write for FakeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some((at, error)) = &self.fail_at
            && self.buffer.len() as u64 + bytes.len() as u64 > *at
        {
            return Err(io::Error::other(error.to_string()));
        }
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl FileWriter for FakeWriter {
    fn commit(self: Box<Self>, _durable: bool) -> Result<(), VfsError> {
        let switches = self.fake.switches();
        if switches.lies.write_without_writing {
            return Ok(());
        }
        if let Some(max) = switches.max_file_size
            && self.buffer.len() as u64 > max
        {
            return Err(VfsError::FileTooLarge);
        }
        if let Some(free) = switches.free_space
            && self.buffer.len() as u64 > free
        {
            return Err(VfsError::StorageFull);
        }
        if self.mode == WriteMode::CreateNew && self.fake.exists(&self.place) {
            return Err(VfsError::AlreadyExists);
        }
        self.fake.add_file(&self.place, &self.buffer);
        if let Some(free) = switches.free_space {
            self.fake.switches_mut().free_space = Some(free - self.buffer.len() as u64);
        }
        Ok(())
    }
}

impl Provider for FakeProvider {
    fn scheme(&self) -> &str {
        "file"
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        let switches = self.switches();
        let mut capabilities = Capabilities::read_only();
        capabilities.writable = true;
        capabilities.trash = !switches.no_trash;
        capabilities.restore = !switches.no_trash;
        capabilities.links = true;
        capabilities.case_sensitive = !switches.case_insensitive;
        capabilities.reserved_names = switches.reserved_names;
        capabilities.max_name_length = Some(255);
        capabilities.random_read = true;
        capabilities.clone_copy = switches.clone_copy;
        capabilities
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        let switches = self.enter(place, ctx)?;
        if !self.exists(place) {
            return Err(VfsError::NotFound);
        }
        let insensitive = switches.case_insensitive;
        let parent_key = key_of(place, insensitive);
        let state = self.state();
        let mut entries = Vec::new();
        for (key, node) in &state.nodes {
            let Ok(child) = Place::parse(key) else {
                continue;
            };
            if child.parent().map(|parent| key_of(&parent, insensitive)) == Some(parent_key.clone())
            {
                let mut entry = node.entry.clone();
                entry.volume = Some(Self::volume_of(&switches, &child));
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        let switches = self.enter(place, ctx)?;
        if place.is_root() {
            return Ok(Entry::new(place.to_string(), EntryKind::Folder));
        }
        let state = self.state();
        let node = state
            .nodes
            .get(&key_of(place, switches.case_insensitive))
            .ok_or(VfsError::NotFound)?;
        let mut entry = node.entry.clone();
        entry.volume = Some(Self::volume_of(&switches, place));
        Ok(entry)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        let switches = self.enter(place, ctx)?;
        let data = self.contents(place).ok_or(VfsError::NotFound)?;
        let fault = switches
            .faults
            .iter()
            .find(|f| &f.place == place && f.at_byte.is_some());
        Ok(Box::new(FaultyReader {
            inner: Cursor::new(data),
            fail_at: fault.and_then(|f| f.at_byte),
            error: fault.map(|f| f.error.clone()),
        }))
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.enter(place, ctx)?;
        let data = self.contents(place).ok_or(VfsError::NotFound)?;
        Ok(Box::new(Cursor::new(data)))
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        let switches = self.enter(place, ctx)?;
        if switches.read_only.contains(place) {
            return Err(VfsError::PermissionDenied);
        }
        if switches.locked.contains(place) {
            return Err(VfsError::InUse);
        }
        if let Some(parent) = place.parent()
            && !self.exists(&parent)
        {
            return Err(VfsError::NotFound);
        }
        if mode == WriteMode::CreateNew && self.exists(place) {
            return Err(VfsError::AlreadyExists);
        }
        let fail_at = switches
            .faults
            .iter()
            .find(|f| &f.place == place)
            .and_then(|f| f.at_byte.map(|at| (at, f.error.clone())));
        Ok(Box::new(FakeWriter {
            fake: self.clone(),
            place: place.clone(),
            mode,
            buffer: Vec::new(),
            fail_at,
        }))
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.enter(place, ctx)?;
        if self.exists(place) {
            return Err(VfsError::AlreadyExists);
        }
        match place.parent() {
            Some(parent) if self.exists(&parent) => {
                self.add_folder(place);
                Ok(())
            }
            _ => Err(VfsError::NotFound),
        }
    }

    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        let switches = self.enter(place, ctx)?;
        if self.exists(place) {
            return Err(VfsError::AlreadyExists);
        }
        let kind = match request.kind {
            LinkKind::Hard => EntryKind::File,
            LinkKind::Symbolic | LinkKind::Junction => EntryKind::Link,
        };
        let mut entry = Entry::new(place.name().unwrap_or_default(), kind);
        entry.link = (kind == EntryKind::Link).then(|| diroxide_core::Link {
            target: request.target.clone(),
            points_to: None,
        });
        self.state().nodes.insert(
            key_of(place, switches.case_insensitive),
            Node {
                entry,
                data: Vec::new(),
            },
        );
        Ok(())
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(from, ctx)?;
        if switches.read_only.contains(from) {
            return Err(VfsError::PermissionDenied);
        }
        if switches.locked.contains(from) {
            return Err(VfsError::InUse);
        }
        if Self::volume_of(&switches, from) != Self::volume_of(&switches, to) {
            return Err(VfsError::CrossesDevices);
        }
        let insensitive = switches.case_insensitive;
        let (from_key, to_key) = (key_of(from, insensitive), key_of(to, insensitive));
        let mut state = self.state();
        if from_key != to_key && state.nodes.contains_key(&to_key) {
            return Err(VfsError::AlreadyExists);
        }
        let moved: Vec<String> = state
            .nodes
            .keys()
            .filter(|k| **k == from_key || k.starts_with(&format!("{from_key}{}", separator(from))))
            .cloned()
            .collect();
        if moved.is_empty() {
            return Err(VfsError::NotFound);
        }
        for key in moved {
            if let Some(mut node) = state.nodes.remove(&key) {
                if key == from_key {
                    node.entry.name = to.name().unwrap_or_default().to_owned();
                }
                let new_key = format!("{to_key}{}", &key[from_key.len()..]);
                state.nodes.insert(new_key, node);
            }
        }
        Ok(())
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(place, ctx)?;
        if switches.read_only.contains(place) {
            return Err(VfsError::PermissionDenied);
        }
        if switches.locked.contains(place) {
            return Err(VfsError::InUse);
        }
        if !self.list(place, ctx).map(|e| e.is_empty()).unwrap_or(true) {
            return Err(VfsError::NotEmpty);
        }
        let key = key_of(place, switches.case_insensitive);
        match self.state().nodes.remove(&key) {
            Some(_) => Ok(()),
            None => Err(VfsError::NotFound),
        }
    }

    fn trash_gate(&self, place: &Place, ctx: &OpContext) -> Result<TrashGate, VfsError> {
        let switches = self.enter(place, ctx)?;
        if switches.no_trash {
            return Ok(TrashGate::NoTrashHere(
                switches.volume_kind.unwrap_or(VolumeKind::Unknown),
            ));
        }
        Ok(TrashGate::Allowed)
    }

    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        let switches = self.enter(place, ctx)?;
        if switches.no_trash {
            return Err(VfsError::TrashRefused {
                why: "this drive has no trash".to_owned(),
            });
        }
        if switches.locked.contains(place) {
            return Err(VfsError::InUse);
        }
        if switches.lies.trash_does_nothing {
            return Ok(TrashCheck::InTrash);
        }
        let key = key_of(place, switches.case_insensitive);
        let mut state = self.state();
        let node = state.nodes.remove(&key).ok_or(VfsError::NotFound)?;
        state.trash.push((key, node));
        Ok(TrashCheck::InTrash)
    }

    fn restore(&self, original: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(original, ctx)?;
        let key = key_of(original, switches.case_insensitive);
        let mut state = self.state();
        if state.nodes.contains_key(&key) {
            return Err(VfsError::AlreadyExists);
        }
        let at = state
            .trash
            .iter()
            .rposition(|(k, _)| *k == key)
            .ok_or(VfsError::NotFound)?;
        let (key, node) = state.trash.remove(at);
        state.nodes.insert(key, node);
        Ok(())
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        let switches = self.enter(place, ctx)?;
        let key = key_of(place, switches.case_insensitive);
        let mut state = self.state();
        let node = state.nodes.get_mut(&key).ok_or(VfsError::NotFound)?;
        if let Some(expected) = change.expect
            && node.entry.attributes != Some(expected)
        {
            return Err(VfsError::Unsafe {
                why: "it changed since the plan".to_owned(),
            });
        }
        let mut inverse = AttributeChange::default();
        if let Some(attributes) = change.attributes {
            inverse.attributes = Some(node.entry.attributes.unwrap_or(Attributes::Unix(0o644)));
            node.entry.attributes = Some(attributes);
        }
        if let Some(modified) = change.modified {
            inverse.modified = node.entry.modified;
            node.entry.modified = Some(modified);
        }
        if let Some(accessed) = change.accessed {
            inverse.accessed = node.entry.accessed;
            node.entry.accessed = Some(accessed);
        }
        if let Some(created) = change.created {
            inverse.created = node.entry.created;
            node.entry.created = Some(created);
        }
        Ok(inverse)
    }

    fn copy_hint(&self, _from: &Place, _to: &Place) -> CopyHint {
        if self.switches().clone_copy {
            CopyHint::Reflink
        } else {
            CopyHint::Stream
        }
    }

    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(from, ctx)?;
        if !switches.clone_copy {
            return Err(VfsError::unsupported("file", "copy without streaming"));
        }
        let data = self.contents(from).ok_or(VfsError::NotFound)?;
        if self.exists(to) {
            return Err(VfsError::AlreadyExists);
        }
        if switches.lies.clone_fails_late {
            self.add_file(to, &data[..data.len() / 2]);
            return Err(VfsError::Io {
                kind: io::ErrorKind::Other,
                message: "the clone failed part of the way".to_owned(),
            });
        }
        self.add_file(to, &data);
        Ok(())
    }

    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        let switches = self.enter(place, ctx)?;
        Ok(VolumeInfo::unknown(VolumeInfo::top_of(place))
            .with_kind(switches.volume_kind.unwrap_or(VolumeKind::Fixed)))
    }

    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        let switches = self.enter(place, ctx)?;
        Ok(SpaceInfo {
            free_to_user: switches.free_space,
            total: None,
            max_file_size: switches.max_file_size,
        })
    }
}

fn separator(place: &Place) -> char {
    if place.to_string().starts_with('/') {
        '/'
    } else {
        '\\'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn p(text: &str) -> Place {
        Place::parse(&if cfg!(windows) {
            format!("D:{}", text.replace('/', "\\"))
        } else {
            text.to_owned()
        })
        .unwrap()
    }

    #[test]
    fn a_tree_lists_writes_and_renames() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x.txt"), b"hello");
        fake.add_folder(&p("/a/sub"));
        let names: Vec<String> = fake
            .list(&p("/a"), &ctx)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let mut writer = fake
            .write(&p("/a/new.txt"), WriteMode::CreateNew, &ctx)
            .unwrap();
        writer.write_all(b"made").unwrap();
        writer.commit(false).unwrap();
        assert_eq!(fake.contents(&p("/a/new.txt")).unwrap(), b"made");
        fake.rename(&p("/a/sub"), &p("/a/moved"), &ctx).unwrap();
        assert!(fake.exists(&p("/a/moved")));
        assert!(matches!(
            fake.rename(&p("/a/x.txt"), &p("/a/new.txt"), &ctx),
            Err(VfsError::AlreadyExists)
        ));
    }

    #[test]
    fn a_write_that_lies_stores_nothing() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            lies: Lies {
                write_without_writing: true,
                ..Lies::default()
            },
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_folder(&p("/a"));
        let mut writer = fake.write(&p("/a/x"), WriteMode::CreateNew, &ctx).unwrap();
        writer.write_all(b"data").unwrap();
        writer.commit(true).unwrap();
        assert!(!fake.exists(&p("/a/x")));
    }

    #[test]
    fn a_trash_that_lies_leaves_the_item() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"1");
        assert_eq!(fake.trash(&p("/a/x"), &ctx).unwrap(), TrashCheck::InTrash);
        assert!(!fake.exists(&p("/a/x")));
        fake.restore(&p("/a/x"), &ctx).unwrap();
        assert!(fake.exists(&p("/a/x")));
        fake.switches_mut().lies.trash_does_nothing = true;
        assert_eq!(fake.trash(&p("/a/x"), &ctx).unwrap(), TrashCheck::InTrash);
        assert!(fake.exists(&p("/a/x")), "the lie: still there");
        fake.switches_mut().no_trash = true;
        assert!(matches!(
            fake.trash(&p("/a/x"), &ctx),
            Err(VfsError::TrashRefused { .. })
        ));
    }

    #[test]
    fn latency_and_a_stall_slow_the_calls() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            latency: Latency {
                per_call: Duration::from_millis(5),
                stall_after_calls: Some(2),
                stall_for: Duration::from_millis(40),
            },
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        let start = Instant::now();
        fake.add_folder(&p("/a"));
        let _ = fake.stat(&p("/a"), &ctx);
        let _ = fake.stat(&p("/a"), &ctx);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn a_second_volume_refuses_a_rename_across() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            second_volume: Some(p("/b")),
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"1");
        fake.add_folder(&p("/b"));
        assert_eq!(
            fake.rename(&p("/a/x"), &p("/b/x"), &ctx),
            Err(VfsError::CrossesDevices)
        );
    }

    #[test]
    fn a_clone_that_fails_late_leaves_a_partial_target() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            clone_copy: true,
            lies: Lies {
                clone_fails_late: true,
                ..Lies::default()
            },
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"abcdef");
        assert!(fake.copy_within(&p("/a/x"), &p("/a/y"), &ctx).is_err());
        assert_eq!(fake.contents(&p("/a/y")).unwrap(), b"abc");
    }

    #[test]
    fn faults_fail_reads_after_n_bytes() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"abcdef");
        fake.switches_mut().faults.push(Fault {
            place: p("/a/x"),
            at_byte: Some(3),
            error: VfsError::NetworkLost,
        });
        let mut text = Vec::new();
        let result = fake.read(&p("/a/x"), &ctx).unwrap().read_to_end(&mut text);
        assert!(result.is_err());
        assert_eq!(text, b"abc");
    }
}
