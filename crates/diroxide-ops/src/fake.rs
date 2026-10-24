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
//!   free-space budget, no created time (as on Linux).
//! - Timing ([`Latency`]): a delay on every call, and a stall that holds
//!   one call for a while after a number of calls.
//! - Lies ([`Lies`]): a commit that reports success without writing, a
//!   clone that fails after it has started writing, a trash that reports
//!   the item in the trash and does nothing; and per place, a write that
//!   stores nothing ([`FakeSwitches::lie_about`]) or one changed byte
//!   ([`FakeSwitches::corrupt`]).
//! - Faults ([`Fault`], [`CallFault`]): a chosen error at a place, on any
//!   call or on one kind of call, always or a number of times, optionally
//!   after N bytes of a read or a write.
//! - A lost connection (R25): places taken offline
//!   ([`FakeProvider::take_offline`]) fail every call with a lost network,
//!   transfers already open included, until brought back; a drop
//!   ([`LinkDrop`]) cuts one transfer after N bytes and leaves its place
//!   offline.
//!
//! A write shows its unfinished name (`.diro-part-` and eight hex digits)
//! in the folder while it runs, as the local provider's committing writer
//! does, so a test can check that a cancelled or failed job leaves none.
//! Links are stored, never followed; a folder alias
//! ([`FakeProvider::add_folder_alias`]) makes a folder that is another one
//! under a second name (a hard-linked folder, a bind mount), for the walk's
//! loop test.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Cursor, Read, SeekFrom, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use diroxide_core::{Attributes, Entry, EntryKind, Place, Timestamp};
use diroxide_vfs::names::unfinished_name;
use diroxide_vfs::provider::ResumableWrite;
use diroxide_vfs::{
    AttributeChange, Can, Capabilities, CopyHint, FileWriter, LinkKind, LinkRequest, OpContext,
    Provider, PurgeCheck, ReadSeek, SpaceInfo, TrashBin, TrashCheck, TrashGate, TrashId,
    TrashedItem, VfsError, VolumeInfo, VolumeKind, WriteMode,
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
    /// A delete from the trash reports the item gone and keeps it.
    pub purge_does_nothing: bool,
    /// A kept part file loses its last N bytes to zeros, its length kept,
    /// as a server that crashed before it flushed (spike Q1's case).
    pub kept_tail_zeroed: u64,
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

/// A kind of provider call, for a [`CallFault`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FakeCall {
    /// `list`.
    List,
    /// `stat`.
    Stat,
    /// `read` and `open_seekable`.
    Read,
    /// `write`, when the writer is opened.
    Write,
    /// A writer's `commit`.
    Commit,
    /// `create_dir`.
    CreateDir,
    /// `create_link`.
    CreateLink,
    /// `rename` and `rename_replacing`, at its source.
    Rename,
    /// `remove`.
    Remove,
    /// `trash`.
    Trash,
    /// `restore`, and the trash list's restore of one item.
    Restore,
    /// The trash list's delete for good of one item.
    Purge,
    /// `set_attributes`.
    SetAttributes,
    /// `copy_within`, at its source.
    CopyWithin,
}

/// A failure of one kind of call at one place: always, or the first
/// `times` calls (an auto-retry test fails twice, then succeeds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFault {
    /// Which call fails.
    pub call: FakeCall,
    /// Where.
    pub place: Place,
    /// The error it gives.
    pub error: VfsError,
    /// How many calls fail; `None` for every call.
    pub times: Option<u32>,
}

impl CallFault {
    /// A call at a place that always fails with this error.
    pub fn new(call: FakeCall, place: Place, error: VfsError) -> Self {
        Self {
            call,
            place,
            error,
            times: None,
        }
    }

    /// The same fault for the first `n` calls only.
    pub fn times(mut self, n: u32) -> Self {
        self.times = Some(n);
        self
    }
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
    /// The place holds no links, as a FAT or exFAT drive: the capabilities
    /// say so and `create_link` is unsupported.
    pub no_links: bool,
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
    /// A file or link can be renamed over another in one step
    /// (`rename_replacing`), as on a local drive; off by default, so the
    /// engine's fallbacks are what most tests see.
    pub replace_rename: bool,
    /// Writers can seek in their unfinished file (`FileWriter::seek_to`),
    /// as the local one can; off by default, so a writer streams in order.
    pub seekable_writes: bool,
    /// The time new entries get.
    pub now: Option<Timestamp>,
    /// Slowness.
    pub latency: Latency,
    /// Lies.
    pub lies: Lies,
    /// Failures.
    pub faults: Vec<Fault>,
    /// Failures of one kind of call.
    pub call_faults: Vec<CallFault>,
    /// Writes to these places report success and store nothing: a place
    /// that silently drops one of two case twins.
    pub lie_about: Vec<Place>,
    /// Writes to these places store their bytes with the first one
    /// changed, for the verification of a move.
    pub corrupt: Vec<Place>,
    /// The place cannot keep a created time, as on Linux.
    pub no_created_time: bool,
    /// The trash takes items by copying them to another drive's trash,
    /// as a Linux drive whose own trash cannot be made: the gate answers
    /// [`TrashGate::Copies`].
    pub trash_copies_across: bool,
    /// What the trash's own post-check answers after it has taken an
    /// item: `GoneNotInTrash` is a trash that lost it, `NotChecked` a
    /// plugin's trash that cannot tell. `None` answers `InTrash`.
    pub trash_check: Option<TrashCheck>,
    /// Places that cannot be reached: every call at or inside one fails
    /// with a lost network, as a host that went away (R25's offline mode).
    /// A test brings a place back by taking it off the list.
    pub offline: Vec<Place>,
    /// Connections that drop in the middle of a transfer (R25's drop
    /// mode), each once.
    pub drops: Vec<LinkDrop>,
    /// Writes can be resumed: a kept write leaves its part file and a
    /// later one reopens it (the capability `resume`); off by default, so
    /// a cut write leaves nothing, as before.
    pub resume: bool,
}

/// A connection that drops after a number of bytes (R25's drop mode): a
/// read or a write of a file at or inside `within` fails with a lost
/// network once its transfer reaches `after_bytes`, counted from the start
/// of the file, and the place goes offline when `goes_offline` says so.
/// Each drop fires once, then is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkDrop {
    /// Where the connection drops.
    pub within: Place,
    /// How many bytes of the file get through first.
    pub after_bytes: u64,
    /// Whether the place stays unreachable afterwards (offline), until a
    /// test brings it back.
    pub goes_offline: bool,
}

impl LinkDrop {
    /// A drop after `after_bytes` bytes at or inside `within`, the place
    /// staying offline afterwards.
    pub fn after(within: Place, after_bytes: u64) -> Self {
        Self {
            within,
            after_bytes,
            goes_offline: true,
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    place: Place,
    parent: Option<String>,
    entry: Entry,
    data: Vec<u8>,
}

/// One item in the fake trash: its key, its entries, and its number.
type Trashed = (String, Vec<(String, Node)>, u64);

#[derive(Debug, Default)]
struct State {
    nodes: BTreeMap<String, Node>,
    /// What the fake trash holds: each item's key, its entries, and its
    /// number, which is also when it was trashed (higher is newer).
    trash: Vec<Trashed>,
    next_trashed: u64,
    aliases: Vec<(String, String)>,
    fault_uses: HashMap<usize, u32>,
    calls: u32,
    seeks: u32,
    next_inode: u64,
    next_token: u32,
}

/// An in-memory provider for tests. Its places are local places in the
/// platform's own style, or remote places of any scheme; the roots exist
/// from the start.
#[derive(Debug, Default, Clone)]
pub struct FakeProvider {
    state: Arc<Mutex<State>>,
    switches: Arc<Mutex<FakeSwitches>>,
}

/// The separator under a place, in its text: `\` in a Windows path, `/`
/// everywhere else.
fn separator(place: &Place) -> char {
    match place.local_path() {
        Some(path) if !path.starts_with('/') => '\\',
        _ => '/',
    }
}

/// Wraps a provider error in an I/O error that carries it, so the engine
/// gets the same error back from a reader or a writer.
fn io_error(error: &VfsError) -> io::Error {
    let kind = match error {
        VfsError::NotFound => io::ErrorKind::NotFound,
        VfsError::PermissionDenied => io::ErrorKind::PermissionDenied,
        VfsError::StorageFull => io::ErrorKind::StorageFull,
        VfsError::NetworkLost => io::ErrorKind::NetworkDown,
        VfsError::InUse => io::ErrorKind::ResourceBusy,
        VfsError::Cancelled => io::ErrorKind::Interrupted,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error.clone())
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

    /// The key of a place: its text, folded when names ignore case.
    fn key(&self, place: &Place) -> String {
        let text = place.to_string();
        if self.switches().case_insensitive {
            text.to_lowercase()
        } else {
            text
        }
    }

    /// The prefix every key inside `key` starts with.
    fn inside(key: &str, sep: char) -> String {
        if key.ends_with(sep) {
            key.to_owned()
        } else {
            format!("{key}{sep}")
        }
    }

    /// The key a place resolves to, through any folder alias on its way:
    /// the alias itself stays (so its parent lists it), and what is inside
    /// it is the other folder's.
    fn resolve(state: &State, key: &str, sep: char) -> String {
        for (alias, target) in &state.aliases {
            let prefix = Self::inside(alias, sep);
            if let Some(rest) = key.strip_prefix(&prefix) {
                return Self::resolve(state, &format!("{}{rest}", Self::inside(target, sep)), sep);
            }
        }
        key.to_owned()
    }

    /// The key whose children a listing of this place shows.
    fn listing_key(state: &State, key: &str, sep: char) -> String {
        let key = Self::resolve(state, key, sep);
        match state.aliases.iter().find(|(alias, _)| *alias == key) {
            Some((_, target)) => Self::resolve(state, target, sep),
            None => key,
        }
    }

    fn node_key(&self, place: &Place) -> String {
        let key = self.key(place);
        Self::resolve(&self.state(), &key, separator(place))
    }

    fn new_node(&self, place: &Place, mut entry: Entry, data: Vec<u8>) -> (String, Node) {
        let key = self.node_key(place);
        let parent = place.parent().map(|p| self.node_key(&p));
        let mut state = self.state();
        state.next_inode += 1;
        entry.inode = Some(state.next_inode);
        (
            key,
            Node {
                place: place.clone(),
                parent,
                entry,
                data,
            },
        )
    }

    /// Adds a folder, and its parents.
    pub fn add_folder(&self, place: &Place) {
        if let Some(parent) = place.parent()
            && !parent.is_root()
        {
            self.add_folder(&parent);
        }
        if place.is_root() || self.exists(place) {
            return;
        }
        let name = place.name().unwrap_or_default().to_owned();
        let mut entry = Entry::new(name, EntryKind::Folder);
        entry.modified = self.switches().now;
        let (key, node) = self.new_node(place, entry, Vec::new());
        self.state().nodes.insert(key, node);
    }

    /// Adds a file with these bytes, and its parent folders, replacing
    /// any file there.
    pub fn add_file(&self, place: &Place, data: &[u8]) {
        if let Some(parent) = place.parent() {
            self.add_folder(&parent);
        }
        let mut entry = Entry::new(place.name().unwrap_or_default(), EntryKind::File)
            .with_size(data.len() as u64);
        entry.modified = self.switches().now;
        let (key, node) = self.new_node(place, entry, data.to_vec());
        self.state().nodes.insert(key, node);
    }

    /// Adds a symbolic link that stores `target`, pointing to a folder or
    /// a file. Nothing follows it.
    pub fn add_link(&self, place: &Place, target: &str, to_folder: bool) {
        if let Some(parent) = place.parent() {
            self.add_folder(&parent);
        }
        let mut entry = Entry::new(place.name().unwrap_or_default(), EntryKind::Link);
        entry.link = Some(diroxide_core::Link {
            target: target.to_owned(),
            points_to: Some(if to_folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            }),
        });
        let (key, node) = self.new_node(place, entry, Vec::new());
        self.state().nodes.insert(key, node);
    }

    /// Adds a second name for an existing file: a hard link, the same
    /// entry by identity, with its own copy of the bytes.
    pub fn add_hard_link(&self, place: &Place, existing: &Place) {
        let Some(original) = self.lookup(existing) else {
            return;
        };
        let mut entry = original.entry.clone();
        entry.name = place.name().unwrap_or_default().to_owned();
        let key = self.node_key(place);
        let parent = place.parent().map(|p| self.node_key(&p));
        self.state().nodes.insert(
            key,
            Node {
                place: place.clone(),
                parent,
                entry,
                data: original.data,
            },
        );
    }

    /// Adds a folder `alias` that is the folder `of` under a second name:
    /// the same identity, the same contents. Put it inside `of` and a walk
    /// that followed it would never end.
    pub fn add_folder_alias(&self, alias: &Place, of: &Place) {
        let Some(original) = self.lookup(of) else {
            return;
        };
        let mut entry = original.entry.clone();
        entry.name = alias.name().unwrap_or_default().to_owned();
        let key = self.node_key(alias);
        let target = self.node_key(of);
        let parent = alias.parent().map(|p| self.node_key(&p));
        let mut state = self.state();
        state.nodes.insert(
            key.clone(),
            Node {
                place: alias.clone(),
                parent,
                entry,
                data: Vec::new(),
            },
        );
        state.aliases.push((key, target));
    }

    fn lookup(&self, place: &Place) -> Option<Node> {
        let key = self.node_key(place);
        self.state().nodes.get(&key).cloned()
    }

    /// The bytes of a file, if it exists.
    pub fn contents(&self, place: &Place) -> Option<Vec<u8>> {
        self.lookup(place).map(|node| node.data)
    }

    /// The entry at a place, as `stat` would give it, without counting a
    /// call or applying a switch.
    pub fn entry(&self, place: &Place) -> Option<Entry> {
        let switches = self.switches();
        self.lookup(place).map(|node| {
            let mut entry = node.entry;
            entry.volume = Some(Self::volume_of(&switches, place));
            entry
        })
    }

    /// Whether an entry exists.
    pub fn exists(&self, place: &Place) -> bool {
        place.is_root() || self.lookup(place).is_some()
    }

    /// How many entries are in the fake trash (a trashed folder counts
    /// once).
    pub fn trashed(&self) -> usize {
        self.state().trash.len()
    }

    /// Whether the fake trash holds an entry from this place.
    pub fn in_trash(&self, place: &Place) -> bool {
        let key = self.node_key(place);
        self.state().trash.iter().any(|(k, _, _)| *k == key)
    }

    /// The bytes of every file in the fake trash, a trashed folder's files
    /// included: with [`FakeProvider::snapshot`], what a job may never
    /// lose.
    pub fn trash_contents(&self) -> Vec<Vec<u8>> {
        self.state()
            .trash
            .iter()
            .flat_map(|(_, nodes, _)| nodes.iter())
            .filter(|(_, node)| node.entry.kind == EntryKind::File)
            .map(|(_, node)| node.data.clone())
            .collect()
    }

    /// How many calls the fake has served.
    pub fn calls(&self) -> u32 {
        self.state().calls
    }

    /// How many times a writer moved in its unfinished file
    /// (`FileWriter::seek_to`, with `seekable_writes` on).
    pub fn seeks(&self) -> u32 {
        self.state().seeks
    }

    /// Every entry, by its place's text, with its kind and bytes: the
    /// tree's state, for comparing before and after a job.
    pub fn snapshot(&self) -> BTreeMap<String, (EntryKind, Vec<u8>)> {
        self.state()
            .nodes
            .values()
            .map(|node| (node.place.to_string(), (node.entry.kind, node.data.clone())))
            .collect()
    }

    /// The names of unfinished writes still in the tree.
    pub fn unfinished(&self) -> Vec<String> {
        self.state()
            .nodes
            .values()
            .filter(|node| diroxide_vfs::names::is_unfinished(&node.entry.name))
            .map(|node| node.place.to_string())
            .collect()
    }

    /// Sets the size an entry lists, whatever its bytes: a large file in a
    /// test without its bytes in memory.
    pub fn set_listed_size(&self, place: &Place, size: u64) {
        let key = self.node_key(place);
        if let Some(node) = self.state().nodes.get_mut(&key) {
            node.entry.size = Some(size);
        }
    }

    /// Sets an entry's modified time, as if it had changed.
    pub fn set_modified(&self, place: &Place, modified: Timestamp) {
        let key = self.node_key(place);
        if let Some(node) = self.state().nodes.get_mut(&key) {
            node.entry.modified = Some(modified);
        }
    }

    /// Marks an entry's name as one the place could not read as Unicode,
    /// as a provider does for such a name (the name kept is a lossy copy).
    pub fn mark_not_unicode(&self, place: &Place) {
        let key = self.node_key(place);
        if let Some(node) = self.state().nodes.get_mut(&key) {
            node.entry.name_not_unicode = true;
        }
    }

    /// Sets an entry's attributes directly.
    pub fn set_attributes_now(&self, place: &Place, attributes: Attributes) {
        let key = self.node_key(place);
        if let Some(node) = self.state().nodes.get_mut(&key) {
            node.entry.attributes = Some(attributes);
        }
    }

    /// Removes an entry and everything inside it at once, as another
    /// program would: no call is counted and no switch applies.
    pub fn vanish(&self, place: &Place) {
        let sep = separator(place);
        let key = self.node_key(place);
        let prefix = Self::inside(&key, sep);
        self.state()
            .nodes
            .retain(|k, _| *k != key && !k.starts_with(&prefix));
    }

    /// Delays, stalls and faults at the start of a call.
    fn enter(
        &self,
        call: FakeCall,
        place: &Place,
        ctx: &OpContext,
    ) -> Result<FakeSwitches, VfsError> {
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
        if switches.offline.iter().any(|root| place.is_within(root)) {
            return Err(VfsError::NetworkLost);
        }
        if let Some(fault) = switches
            .faults
            .iter()
            .find(|f| &f.place == place && f.at_byte.is_none())
        {
            return Err(fault.error.clone());
        }
        let key = self.key(place);
        for (index, fault) in switches.call_faults.iter().enumerate() {
            if fault.call != call || self.key(&fault.place) != key {
                continue;
            }
            let mut state = self.state();
            let used = state.fault_uses.entry(index).or_insert(0);
            if fault.times.is_none_or(|times| *used < times) {
                *used += 1;
                return Err(fault.error.clone());
            }
        }
        ctx.check()?;
        Ok(switches)
    }

    /// Takes a place offline: every call at or inside it fails with a lost
    /// network until [`FakeProvider::bring_back`].
    pub fn take_offline(&self, place: &Place) {
        let mut switches = self.switches_mut();
        if !switches.offline.contains(place) {
            switches.offline.push(place.clone());
        }
    }

    /// Brings an offline place back.
    pub fn bring_back(&self, place: &Place) {
        self.switches_mut().offline.retain(|root| root != place);
    }

    /// Whether a place is offline now.
    pub fn is_offline(&self, place: &Place) -> bool {
        self.switches()
            .offline
            .iter()
            .any(|root| place.is_within(root))
    }

    /// The armed drop for a transfer at a place: where it fires.
    fn drop_for(switches: &FakeSwitches, place: &Place) -> Option<LinkDrop> {
        switches
            .drops
            .iter()
            .find(|drop| place.is_within(&drop.within))
            .cloned()
    }

    /// Fires a drop: `true` when it was still armed (it is gone now, and
    /// its place offline when it says so), `false` when another transfer
    /// fired it first.
    fn fire(&self, drop: &LinkDrop) -> bool {
        let mut switches = self.switches_mut();
        let Some(at) = switches.drops.iter().position(|armed| armed == drop) else {
            return false;
        };
        switches.drops.remove(at);
        if drop.goes_offline && !switches.offline.contains(&drop.within) {
            switches.offline.push(drop.within.clone());
        }
        true
    }

    /// A committing writer at a place, its unfinished name showing in the
    /// folder: the body of `write` and of the kept write.
    fn writer(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<FakeWriter, VfsError> {
        let switches = self.enter(FakeCall::Write, place, ctx)?;
        if switches.read_only.contains(place) {
            return Err(VfsError::PermissionDenied);
        }
        if switches.locked.contains(place) {
            return Err(VfsError::InUse);
        }
        let Some(folder) = place.parent() else {
            return Err(VfsError::NotFound);
        };
        if !self.exists(&folder) {
            return Err(VfsError::NotFound);
        }
        if mode == WriteMode::CreateNew && self.exists(place) {
            return Err(VfsError::AlreadyExists);
        }
        let token = {
            let mut state = self.state();
            state.next_token += 1;
            state.next_token
        };
        let unfinished = folder
            .join(&unfinished_name(token))
            .map_err(VfsError::from)?;
        let entry = Entry::new(unfinished.name().unwrap_or_default(), EntryKind::File);
        let (key, node) = self.new_node(&unfinished, entry, Vec::new());
        self.state().nodes.insert(key, node);
        Ok(FakeWriter::new(
            self,
            place,
            Some(unfinished),
            mode,
            &switches,
        ))
    }

    /// Refuses anything but an unfinished name.
    fn unfinished_only(place: &Place) -> Result<(), VfsError> {
        if diroxide_vfs::names::is_unfinished(place.name().unwrap_or_default()) {
            Ok(())
        } else {
            Err(VfsError::Unsafe {
                why: "only an unfinished file is reopened or removed here".to_owned(),
            })
        }
    }

    fn volume_of(switches: &FakeSwitches, place: &Place) -> u64 {
        match &switches.second_volume {
            Some(root) if place.is_within(root) => 2,
            _ => 1,
        }
    }

    fn children(&self, place: &Place) -> Vec<Node> {
        let sep = separator(place);
        let key = self.key(place);
        let state = self.state();
        let parent = Self::listing_key(&state, &key, sep);
        state
            .nodes
            .values()
            .filter(|node| node.parent.as_deref() == Some(parent.as_str()))
            .cloned()
            .collect()
    }

    /// The place of a node moved from `from` to `to`.
    fn rebase(&self, place: &Place, from: &Place, to: &Place) -> Option<Place> {
        let from_key = self.key(from);
        let mut names = Vec::new();
        let mut current = place.clone();
        while self.key(&current) != from_key {
            names.push(current.name()?.to_owned());
            current = current.parent()?;
        }
        let mut rebased = to.clone();
        for name in names.iter().rev() {
            rebased = rebased.join(name).ok()?;
        }
        Some(rebased)
    }

    /// Takes an entry and everything inside it out of the tree.
    fn take_subtree(&self, place: &Place) -> Vec<(String, Node)> {
        let sep = separator(place);
        let key = self.node_key(place);
        let prefix = Self::inside(&key, sep);
        let mut state = self.state();
        let keys: Vec<String> = state
            .nodes
            .keys()
            .filter(|k| **k == key || k.starts_with(&prefix))
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|k| state.nodes.remove(&k).map(|node| (k, node)))
            .collect()
    }

    /// Moves an entry and everything inside it to `to`, whose name must be
    /// free: the body of `rename` and `rename_replacing`, after their
    /// checks.
    fn move_node(&self, from: &Place, to: &Place) {
        let moved = self.take_subtree(from);
        let mut nodes = Vec::with_capacity(moved.len());
        for (_, mut node) in moved {
            let place = self
                .rebase(&node.place, from, to)
                .unwrap_or(node.place.clone());
            if self.key(&node.place) == self.key(from) {
                node.entry.name = to.name().unwrap_or_default().to_owned();
            }
            node.parent = place.parent().map(|p| self.node_key(&p));
            node.place = place;
            nodes.push(node);
        }
        let mut keyed = Vec::with_capacity(nodes.len());
        for node in nodes {
            keyed.push((self.node_key(&node.place), node));
        }
        self.state().nodes.extend(keyed);
    }
}

/// The slow-place scenario for the bench (R23): a source folder of files
/// and an empty target folder on a fake whose every call waits and one
/// call stalls, so the bench (W1e) can time a copy from a slow or stalling
/// place and check that the stall is said.
#[derive(Debug, Clone)]
pub struct SlowPlace {
    /// The fake, slow from the moment it is returned.
    pub fake: FakeProvider,
    /// The files to copy, in order.
    pub sources: Vec<Place>,
    /// The empty folder to copy them into.
    pub target: Place,
}

impl SlowPlace {
    /// Builds `files` files of `size` bytes under `root/source`, and an
    /// empty `root/target`, then makes the fake slow with `latency`. The
    /// stall counts every call from here on, the plan's included.
    pub fn new(
        root: &Place,
        files: usize,
        size: usize,
        latency: Latency,
    ) -> Result<Self, VfsError> {
        let fake = FakeProvider::new();
        let source = root.join("source")?;
        let target = root.join("target")?;
        fake.add_folder(&source);
        fake.add_folder(&target);
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut sources = Vec::with_capacity(files);
        for n in 0..files {
            let place = source.join(&format!("f{n:05}.bin"))?;
            fake.add_file(&place, &data);
            sources.push(place);
        }
        fake.switches_mut().latency = latency;
        Ok(Self {
            fake,
            sources,
            target,
        })
    }
}

/// The connection a transfer runs over: a place that can go offline
/// while it runs, and a drop that may fire in it.
struct Link {
    fake: FakeProvider,
    place: Place,
    drop: Option<LinkDrop>,
}

impl Link {
    fn new(fake: &FakeProvider, place: &Place, switches: &FakeSwitches) -> Self {
        Self {
            fake: fake.clone(),
            place: place.clone(),
            drop: FakeProvider::drop_for(switches, place),
        }
    }

    /// How many bytes from `at` may pass before the drop; `None` when no
    /// drop is armed. An error when the place is offline, or the drop
    /// fires at `at`.
    fn room(&mut self, at: u64) -> io::Result<Option<u64>> {
        if self.fake.is_offline(&self.place) {
            return Err(io_error(&VfsError::NetworkLost));
        }
        let Some(drop) = &self.drop else {
            return Ok(None);
        };
        if at < drop.after_bytes {
            return Ok(Some(drop.after_bytes - at));
        }
        let drop = drop.clone();
        self.drop = None;
        if self.fake.fire(&drop) {
            return Err(io_error(&VfsError::NetworkLost));
        }
        Ok(None)
    }
}

/// A reader that fails after a number of bytes, and over a connection
/// that may drop.
struct FaultyReader {
    inner: Cursor<Vec<u8>>,
    fail_at: Option<u64>,
    error: Option<VfsError>,
    link: Link,
}

impl Read for FaultyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let (Some(at), Some(error)) = (self.fail_at, &self.error)
            && self.inner.position() >= at
        {
            return Err(io_error(error));
        }
        let mut limit = match self.fail_at {
            Some(at) => usize::try_from(at.saturating_sub(self.inner.position()))
                .unwrap_or(usize::MAX)
                .min(buf.len()),
            None => buf.len(),
        };
        if let Some(room) = self.link.room(self.inner.position())? {
            limit = limit.min(usize::try_from(room).unwrap_or(usize::MAX));
        }
        self.inner.read(&mut buf[..limit])
    }
}

/// The fake's committing writer. Its unfinished name shows in the folder
/// until the commit or the drop.
struct FakeWriter {
    fake: FakeProvider,
    place: Place,
    unfinished: Option<Place>,
    mode: WriteMode,
    buffer: Vec<u8>,
    /// Where the next bytes go: the end, unless a seek moved it.
    pos: usize,
    seekable: bool,
    fail_at: Option<(u64, VfsError)>,
    link: Link,
    /// A kept write: dropped without its commit, its bytes stay in the
    /// part file.
    kept: bool,
}

impl Write for FakeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut bytes = bytes;
        if let Some(room) = self.link.room(self.pos as u64)? {
            bytes = &bytes[..bytes.len().min(usize::try_from(room).unwrap_or(usize::MAX))];
        }
        let end = self.pos + bytes.len();
        if let Some((at, error)) = &self.fail_at
            && end as u64 > *at
        {
            return Err(io_error(error));
        }
        if self.buffer.len() < end {
            self.buffer.resize(end, 0);
        }
        self.buffer[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl FakeWriter {
    fn new(
        fake: &FakeProvider,
        place: &Place,
        unfinished: Option<Place>,
        mode: WriteMode,
        switches: &FakeSwitches,
    ) -> Self {
        let fail_at = switches
            .faults
            .iter()
            .find(|f| &f.place == place)
            .and_then(|f| f.at_byte.map(|at| (at, f.error.clone())));
        Self {
            fake: fake.clone(),
            place: place.clone(),
            unfinished,
            mode,
            buffer: Vec::new(),
            pos: 0,
            seekable: switches.seekable_writes,
            fail_at,
            link: Link::new(fake, place, switches),
            kept: false,
        }
    }
}

impl Drop for FakeWriter {
    fn drop(&mut self) {
        let Some(unfinished) = self.unfinished.take() else {
            return;
        };
        if !self.kept {
            self.fake.vanish(&unfinished);
            return;
        }
        let mut data = std::mem::take(&mut self.buffer);
        let zeroed = usize::try_from(self.fake.switches().lies.kept_tail_zeroed)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let from = data.len() - zeroed;
        data[from..].fill(0);
        self.fake.add_file(&unfinished, &data);
    }
}

impl FileWriter for FakeWriter {
    fn unfinished_place(&self) -> Option<Place> {
        self.unfinished.clone()
    }

    fn seek_to(&mut self, to: SeekFrom) -> Result<u64, VfsError> {
        if !self.seekable {
            return Err(VfsError::UnsupportedFeature {
                what: "seeking in an unfinished file".to_owned(),
            });
        }
        let at = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(delta) => self.buffer.len() as i128 + i128::from(delta),
            SeekFrom::Current(delta) => self.pos as i128 + i128::from(delta),
        };
        let at = usize::try_from(at).map_err(|_| VfsError::Io {
            kind: io::ErrorKind::InvalidInput,
            message: "a seek before the start of the file".to_owned(),
        })?;
        self.fake.state().seeks += 1;
        self.pos = at;
        Ok(at as u64)
    }

    fn commit(mut self: Box<Self>, _durable: bool) -> Result<(), VfsError> {
        if let Some(unfinished) = self.unfinished.take() {
            self.fake.vanish(&unfinished);
        }
        let ctx = OpContext::new();
        let switches = self.fake.enter(FakeCall::Commit, &self.place, &ctx)?;
        if switches.lies.write_without_writing || switches.lie_about.contains(&self.place) {
            return Ok(());
        }
        if let Some(max) = switches.max_file_size
            && self.buffer.len() as u64 > max
        {
            return Err(VfsError::FileTooLarge);
        }
        let old_size = self
            .fake
            .contents(&self.place)
            .map_or(0, |data| data.len() as u64);
        let needed = (self.buffer.len() as u64).saturating_sub(old_size);
        if let Some(free) = switches.free_space
            && needed > free
        {
            return Err(VfsError::StorageFull);
        }
        if let Some(parent) = self.place.parent()
            && !self.fake.exists(&parent)
        {
            return Err(VfsError::NotFound);
        }
        if let Some(existing) = self.fake.lookup(&self.place) {
            if self.mode == WriteMode::CreateNew {
                return Err(VfsError::AlreadyExists);
            }
            if existing.entry.kind == EntryKind::Folder {
                return Err(VfsError::AlreadyExists);
            }
        }
        let mut data = std::mem::take(&mut self.buffer);
        if switches.corrupt.contains(&self.place)
            && let Some(first) = data.first_mut()
        {
            *first ^= 0xFF;
        }
        self.fake.add_file(&self.place, &data);
        if let Some(free) = switches.free_space {
            self.fake.switches_mut().free_space = Some(free - needed);
        }
        Ok(())
    }
}

/// The fake trash as a list, for the trash browser's jobs
/// ([`FakeProvider::trash_list`]): each item's id is its number
/// ("fake-3"), and its number is when it was trashed. It shares the
/// fake's entries and trash; never the real trash.
#[derive(Clone, Debug)]
pub struct FakeTrash {
    fake: FakeProvider,
}

impl FakeProvider {
    /// The fake's trash as a list, for [`crate::Engine::with_trash`].
    pub fn trash_list(&self) -> FakeTrash {
        FakeTrash { fake: self.clone() }
    }
}

impl TrashBin for FakeTrash {
    fn list(
        &self,
        ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<TrashedItem>) -> std::ops::ControlFlow<()>,
    ) -> Result<(), VfsError> {
        ctx.check()?;
        let mut items: Vec<TrashedItem> = self
            .fake
            .state()
            .trash
            .iter()
            .filter_map(|(key, nodes, number)| {
                let root = nodes.iter().find(|(k, _)| k == key)?;
                let deleted = Timestamp::from_unix(i64::try_from(*number).ok()?, 0);
                let size = root.1.entry.size;
                let folder = root.1.entry.kind == EntryKind::Folder;
                Some(
                    TrashedItem::new(
                        TrashId::new(format!("fake-{number}")),
                        root.1.place.clone(),
                        Some(deleted),
                    )
                    .measured(
                        if folder { None } else { size },
                        folder.then(|| nodes.len().saturating_sub(1) as u64),
                    ),
                )
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.deleted));
        for chunk in items.chunks(diroxide_vfs::local::TRASH_BATCH) {
            if batch(chunk.to_vec()).is_break() {
                break;
            }
        }
        Ok(())
    }

    fn restore(&self, item: &TrashedItem, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.enter(FakeCall::Restore, &item.original, ctx)?;
        let key = self.fake.node_key(&item.original);
        if self.fake.exists(&item.original) {
            return Err(VfsError::AlreadyExists);
        }
        if !item
            .original
            .parent()
            .is_some_and(|parent| self.fake.exists(&parent))
        {
            return Err(VfsError::NotFound);
        }
        let id = item.id.as_os_str().to_string_lossy().into_owned();
        let mut state = self.fake.state();
        let at = state
            .trash
            .iter()
            .position(|(k, _, number)| *k == key && format!("fake-{number}") == id)
            .ok_or(VfsError::NotFound)?;
        let (_, subtree, _) = state.trash.remove(at);
        state.nodes.extend(subtree);
        Ok(())
    }

    fn purge(&self, item: &TrashedItem, ctx: &OpContext) -> Result<PurgeCheck, VfsError> {
        let switches = self.fake.enter(FakeCall::Purge, &item.original, ctx)?;
        if switches.lies.purge_does_nothing {
            return Ok(PurgeCheck::Gone);
        }
        let id = item.id.as_os_str().to_string_lossy().into_owned();
        let mut state = self.fake.state();
        let at = state
            .trash
            .iter()
            .position(|(_, _, number)| format!("fake-{number}") == id)
            .ok_or(VfsError::NotFound)?;
        state.trash.remove(at);
        Ok(PurgeCheck::Gone)
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
        capabilities.links = !switches.no_links;
        capabilities.attributes = true;
        capabilities.access.mode = Can::Yes;
        capabilities.access.os_attributes = Can::Yes;
        capabilities.access.times = Can::Yes;
        capabilities.access.created_time = if switches.no_created_time {
            Can::No
        } else {
            Can::Yes
        };
        capabilities.case_sensitive = !switches.case_insensitive;
        capabilities.reserved_names = switches.reserved_names;
        capabilities.max_name_length = Some(255);
        capabilities.random_read = true;
        capabilities.clone_copy = switches.clone_copy;
        capabilities.replace_rename = switches.replace_rename;
        capabilities.resume = switches.resume;
        capabilities
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        let switches = self.enter(FakeCall::List, place, ctx)?;
        match self.lookup(place) {
            None if !place.is_root() => return Err(VfsError::NotFound),
            Some(node) if node.entry.kind != EntryKind::Folder => {
                return Err(VfsError::NotAFolder);
            }
            _ => {}
        }
        Ok(self
            .children(place)
            .into_iter()
            .map(|node| {
                let mut entry = node.entry;
                entry.volume = Some(Self::volume_of(&switches, &node.place));
                entry
            })
            .collect())
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        let switches = self.enter(FakeCall::Stat, place, ctx)?;
        if place.is_root() {
            let mut entry = Entry::new(place.to_string(), EntryKind::Folder);
            entry.volume = Some(Self::volume_of(&switches, place));
            return Ok(entry);
        }
        let node = self.lookup(place).ok_or(VfsError::NotFound)?;
        let mut entry = node.entry;
        entry.volume = Some(Self::volume_of(&switches, place));
        Ok(entry)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        let switches = self.enter(FakeCall::Read, place, ctx)?;
        let node = self.lookup(place).ok_or(VfsError::NotFound)?;
        if node.entry.kind == EntryKind::Folder {
            return Err(VfsError::NotAFolder);
        }
        if switches.locked.contains(place) {
            return Err(VfsError::InUse);
        }
        let fault = switches
            .faults
            .iter()
            .find(|f| &f.place == place && f.at_byte.is_some());
        Ok(Box::new(FaultyReader {
            inner: Cursor::new(node.data),
            fail_at: fault.and_then(|f| f.at_byte),
            error: fault.map(|f| f.error.clone()),
            link: Link::new(self, place, &switches),
        }))
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.enter(FakeCall::Read, place, ctx)?;
        let data = self.contents(place).ok_or(VfsError::NotFound)?;
        Ok(Box::new(Cursor::new(data)))
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Ok(Box::new(self.writer(place, mode, ctx)?))
    }

    fn as_resumable(&self) -> Option<&dyn ResumableWrite> {
        Some(self)
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.enter(FakeCall::CreateDir, place, ctx)?;
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
        let switches = self.enter(FakeCall::CreateLink, place, ctx)?;
        if switches.no_links {
            return Err(VfsError::unsupported(self.scheme(), "make links"));
        }
        if self.exists(place) {
            return Err(VfsError::AlreadyExists);
        }
        if !place.parent().is_some_and(|parent| self.exists(&parent)) {
            return Err(VfsError::NotFound);
        }
        match request.kind {
            LinkKind::Hard => match self.lookup(&Place::parse(&request.target)?) {
                Some(node) if node.entry.kind == EntryKind::File => {
                    self.add_hard_link(place, &node.place);
                }
                Some(_) => return Err(VfsError::NotAFolder),
                None => return Err(VfsError::NotFound),
            },
            LinkKind::Symbolic | LinkKind::Junction => {
                self.add_link(place, &request.target, request.target_is_folder);
            }
        }
        Ok(())
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(FakeCall::Rename, from, ctx)?;
        if switches.read_only.contains(from) {
            return Err(VfsError::PermissionDenied);
        }
        if switches.locked.contains(from) {
            return Err(VfsError::InUse);
        }
        if Self::volume_of(&switches, from) != Self::volume_of(&switches, to) {
            return Err(VfsError::CrossesDevices);
        }
        let from_key = self.node_key(from);
        let to_key = self.node_key(to);
        if !self.state().nodes.contains_key(&from_key) {
            return Err(VfsError::NotFound);
        }
        if from_key != to_key && self.state().nodes.contains_key(&to_key) {
            return Err(VfsError::AlreadyExists);
        }
        if !to.parent().is_some_and(|parent| self.exists(&parent)) {
            return Err(VfsError::NotFound);
        }
        if to.is_within(from) && from_key != to_key {
            return Err(VfsError::Unsafe {
                why: "a folder cannot go inside itself".to_owned(),
            });
        }
        self.move_node(from, to);
        Ok(())
    }

    fn rename_replacing(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(FakeCall::Rename, from, ctx)?;
        if !switches.replace_rename {
            return Err(VfsError::unsupported(
                self.scheme(),
                "rename over another entry",
            ));
        }
        for place in [from, to] {
            if switches.read_only.contains(place) {
                return Err(VfsError::PermissionDenied);
            }
            if switches.locked.contains(place) {
                return Err(VfsError::InUse);
            }
        }
        if Self::volume_of(&switches, from) != Self::volume_of(&switches, to) {
            return Err(VfsError::CrossesDevices);
        }
        let source = self.lookup(from).ok_or(VfsError::NotFound)?;
        if source.entry.kind == EntryKind::Folder {
            return Err(VfsError::unsupported(
                self.scheme(),
                "rename a folder over another entry",
            ));
        }
        if !to.parent().is_some_and(|parent| self.exists(&parent)) {
            return Err(VfsError::NotFound);
        }
        let (from_key, to_key) = (self.node_key(from), self.node_key(to));
        if from_key == to_key {
            return Ok(());
        }
        if let Some(there) = self.lookup(to) {
            if there.entry.kind == EntryKind::Folder {
                return Err(VfsError::AlreadyExists);
            }
            let mut state = self.state();
            state.nodes.remove(&to_key);
            state.aliases.retain(|(alias, _)| *alias != to_key);
        }
        self.move_node(from, to);
        Ok(())
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let switches = self.enter(FakeCall::Remove, place, ctx)?;
        if switches.read_only.contains(place) {
            return Err(VfsError::PermissionDenied);
        }
        if switches.locked.contains(place) {
            return Err(VfsError::InUse);
        }
        let node = self.lookup(place).ok_or(VfsError::NotFound)?;
        if matches!(node.entry.attributes, Some(Attributes::Windows(flags)) if flags & 0x1 != 0) {
            // As Windows's own delete call: a read-only file is refused
            // until its attribute is cleared.
            return Err(VfsError::PermissionDenied);
        }
        if node.entry.kind == EntryKind::Folder && !self.children(place).is_empty() {
            return Err(VfsError::NotEmpty);
        }
        let key = self.node_key(place);
        let mut state = self.state();
        state.nodes.remove(&key);
        state.aliases.retain(|(alias, _)| *alias != key);
        Ok(())
    }

    fn trash_gate(&self, place: &Place, ctx: &OpContext) -> Result<TrashGate, VfsError> {
        let switches = self.enter(FakeCall::Stat, place, ctx)?;
        if switches.no_trash {
            return Ok(TrashGate::NoTrashHere(
                switches.volume_kind.unwrap_or(VolumeKind::Unknown),
            ));
        }
        if switches.trash_copies_across {
            return Ok(TrashGate::Copies);
        }
        Ok(TrashGate::Allowed)
    }

    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        let switches = self.enter(FakeCall::Trash, place, ctx)?;
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
        if !self.exists(place) {
            return Err(VfsError::NotFound);
        }
        let key = self.node_key(place);
        let subtree = self.take_subtree(place);
        let mut state = self.state();
        state.aliases.retain(|(alias, _)| *alias != key);
        let check = switches.trash_check.unwrap_or(TrashCheck::InTrash);
        if check != TrashCheck::GoneNotInTrash {
            state.next_trashed += 1;
            let number = state.next_trashed;
            state.trash.push((key, subtree, number));
        }
        Ok(check)
    }

    fn restore(&self, original: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.enter(FakeCall::Restore, original, ctx)?;
        let key = self.node_key(original);
        if self.state().nodes.contains_key(&key) {
            return Err(VfsError::AlreadyExists);
        }
        if !original.parent().is_some_and(|parent| self.exists(&parent)) {
            return Err(VfsError::NotFound);
        }
        let mut state = self.state();
        let at = state
            .trash
            .iter()
            .rposition(|(k, _, _)| *k == key)
            .ok_or(VfsError::NotFound)?;
        let (_, subtree, _) = state.trash.remove(at);
        state.nodes.extend(subtree);
        Ok(())
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        let switches = self.enter(FakeCall::SetAttributes, place, ctx)?;
        if change.created.is_some() && switches.no_created_time {
            return Err(VfsError::unsupported("file", "set a created time"));
        }
        let key = self.node_key(place);
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
        let switches = self.enter(FakeCall::CopyWithin, from, ctx)?;
        if !switches.clone_copy
            || Self::volume_of(&switches, from) != Self::volume_of(&switches, to)
        {
            return Err(VfsError::unsupported("file", "copy without streaming"));
        }
        let source = self.lookup(from).ok_or(VfsError::NotFound)?;
        if source.entry.kind != EntryKind::File {
            return Err(VfsError::unsupported("file", "clone a folder"));
        }
        if self.exists(to) {
            return Err(VfsError::AlreadyExists);
        }
        if !to.parent().is_some_and(|parent| self.exists(&parent)) {
            return Err(VfsError::NotFound);
        }
        if switches.lies.clone_fails_late {
            self.add_file(to, &source.data[..source.data.len() / 2]);
            return Err(VfsError::Io {
                kind: io::ErrorKind::Other,
                message: "the clone failed part of the way".to_owned(),
            });
        }
        self.add_file(to, &source.data);
        let key = self.node_key(to);
        if let Some(node) = self.state().nodes.get_mut(&key) {
            node.entry.modified = source.entry.modified;
            node.entry.created = source.entry.created;
            node.entry.attributes = source.entry.attributes;
        }
        Ok(())
    }

    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        let switches = self.enter(FakeCall::Stat, place, ctx)?;
        let root = match &switches.second_volume {
            Some(root) if place.is_within(root) => root.clone(),
            _ => VolumeInfo::top_of(place),
        };
        Ok(VolumeInfo::unknown(root).with_kind(switches.volume_kind.unwrap_or(VolumeKind::Fixed)))
    }

    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        let switches = self.enter(FakeCall::Stat, place, ctx)?;
        Ok(SpaceInfo {
            free_to_user: switches.free_space,
            total: None,
            max_file_size: switches.max_file_size,
        })
    }
}

/// The fake's resumable write (R25 commit 69): a kept write leaves its
/// bytes in its part file, with the tail zeroed when
/// [`Lies::kept_tail_zeroed`] says so; a reopened one goes on at its end.
impl ResumableWrite for FakeProvider {
    fn write_kept(
        &self,
        target: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        let mut writer = self.writer(target, mode, ctx)?;
        writer.kept = true;
        Ok(Box::new(writer))
    }

    fn reopen(
        &self,
        unfinished: &Place,
        target: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<(Box<dyn FileWriter>, u64), VfsError> {
        Self::unfinished_only(unfinished)?;
        if unfinished.parent() != target.parent() {
            return Err(VfsError::Unsafe {
                why: "the unfinished file is not beside its target".to_owned(),
            });
        }
        let switches = self.enter(FakeCall::Write, target, ctx)?;
        let data = self.contents(unfinished).ok_or(VfsError::NotFound)?;
        let length = data.len() as u64;
        let mut writer = FakeWriter::new(self, target, Some(unfinished.clone()), mode, &switches);
        writer.pos = data.len();
        writer.buffer = data;
        writer.kept = true;
        Ok((Box::new(writer), length))
    }

    fn unfinished_len(&self, unfinished: &Place, ctx: &OpContext) -> Result<u64, VfsError> {
        Self::unfinished_only(unfinished)?;
        self.enter(FakeCall::Stat, unfinished, ctx)?;
        self.contents(unfinished)
            .map(|data| data.len() as u64)
            .ok_or(VfsError::NotFound)
    }

    fn read_unfinished(
        &self,
        unfinished: &Place,
        offset: u64,
        length: usize,
        ctx: &OpContext,
    ) -> Result<Vec<u8>, VfsError> {
        Self::unfinished_only(unfinished)?;
        self.enter(FakeCall::Read, unfinished, ctx)?;
        let data = self.contents(unfinished).ok_or(VfsError::NotFound)?;
        let from = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let to = from.saturating_add(length).min(data.len());
        Ok(data[from..to].to_vec())
    }

    fn discard(&self, unfinished: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        Self::unfinished_only(unfinished)?;
        self.enter(FakeCall::Remove, unfinished, ctx)?;
        if !self.exists(unfinished) {
            return Err(VfsError::NotFound);
        }
        self.vanish(unfinished);
        Ok(())
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

    fn names(fake: &FakeProvider, place: &Place) -> Vec<String> {
        let mut names: Vec<String> = fake
            .list(place, &OpContext::new())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_tree_lists_writes_and_renames() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x.txt"), b"hello");
        fake.add_folder(&p("/a/sub"));
        fake.add_file(&p("/a/sub/inner.txt"), b"in");
        assert_eq!(names(&fake, &p("/a")), vec!["sub", "x.txt"]);
        let mut writer = fake
            .write(&p("/a/new.txt"), WriteMode::CreateNew, &ctx)
            .unwrap();
        writer.write_all(b"made").unwrap();
        assert_eq!(fake.unfinished().len(), 1, "the unfinished name shows");
        let unfinished = writer.unfinished_place().unwrap();
        assert_eq!(unfinished.parent(), Some(p("/a")));
        assert!(fake.exists(&unfinished), "the writer names it");
        writer.commit(false).unwrap();
        assert!(fake.unfinished().is_empty());
        assert_eq!(fake.contents(&p("/a/new.txt")).unwrap(), b"made");
        fake.rename(&p("/a/sub"), &p("/a/moved"), &ctx).unwrap();
        assert!(fake.exists(&p("/a/moved")));
        assert_eq!(fake.contents(&p("/a/moved/inner.txt")).unwrap(), b"in");
        assert!(!fake.exists(&p("/a/sub/inner.txt")));
        assert!(matches!(
            fake.rename(&p("/a/x.txt"), &p("/a/new.txt"), &ctx),
            Err(VfsError::AlreadyExists)
        ));
        assert!(matches!(
            fake.remove(&p("/a/moved"), &ctx),
            Err(VfsError::NotEmpty)
        ));
    }

    #[test]
    fn a_dropped_writer_leaves_nothing() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_folder(&p("/a"));
        let mut writer = fake.write(&p("/a/x"), WriteMode::CreateNew, &ctx).unwrap();
        writer.write_all(b"half").unwrap();
        drop(writer);
        assert!(fake.unfinished().is_empty());
        assert!(!fake.exists(&p("/a/x")));
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
        fake.switches_mut().lies.write_without_writing = false;
        fake.switches_mut().lie_about.push(p("/a/y"));
        let mut writer = fake.write(&p("/a/y"), WriteMode::CreateNew, &ctx).unwrap();
        writer.write_all(b"data").unwrap();
        writer.commit(true).unwrap();
        assert!(!fake.exists(&p("/a/y")), "the lie for one place");
    }

    #[test]
    fn a_corrupting_place_changes_one_byte() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            corrupt: vec![p("/a/x")],
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_folder(&p("/a"));
        let mut writer = fake.write(&p("/a/x"), WriteMode::CreateNew, &ctx).unwrap();
        writer.write_all(b"abc").unwrap();
        writer.commit(true).unwrap();
        assert_ne!(fake.contents(&p("/a/x")).unwrap(), b"abc");
        assert_eq!(&fake.contents(&p("/a/x")).unwrap()[1..], b"bc");
    }

    #[test]
    fn a_trash_that_lies_leaves_the_item() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"1");
        assert_eq!(fake.trash(&p("/a/x"), &ctx).unwrap(), TrashCheck::InTrash);
        assert!(!fake.exists(&p("/a/x")));
        assert!(fake.in_trash(&p("/a/x")));
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
    fn a_trashed_folder_takes_its_contents_and_comes_back_whole() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/f/1.txt"), b"1");
        fake.add_file(&p("/a/f/g/2.txt"), b"2");
        fake.trash(&p("/a/f"), &ctx).unwrap();
        assert!(!fake.exists(&p("/a/f/g/2.txt")));
        assert_eq!(fake.trashed(), 1);
        fake.add_file(&p("/a/f"), b"taken");
        assert!(matches!(
            fake.restore(&p("/a/f"), &ctx),
            Err(VfsError::AlreadyExists)
        ));
        fake.vanish(&p("/a/f"));
        fake.restore(&p("/a/f"), &ctx).unwrap();
        assert_eq!(fake.contents(&p("/a/f/g/2.txt")).unwrap(), b"2");
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
        assert_eq!(fake.calls(), 2);
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
        assert_eq!(fake.stat(&p("/b"), &ctx).unwrap().volume, Some(2));
        assert_eq!(fake.stat(&p("/a/x"), &ctx).unwrap().volume, Some(1));
        assert_eq!(fake.volume(&p("/b/y"), &ctx).unwrap().root, p("/b"));
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
        fake.switches_mut().lies.clone_fails_late = false;
        fake.copy_within(&p("/a/x"), &p("/a/z"), &ctx).unwrap();
        assert_eq!(fake.contents(&p("/a/z")).unwrap(), b"abcdef");
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
        let error = fake
            .read(&p("/a/x"), &ctx)
            .unwrap()
            .read_to_end(&mut text)
            .unwrap_err();
        assert_eq!(text, b"abc");
        let inner = error.get_ref().unwrap().downcast_ref::<VfsError>();
        assert_eq!(inner, Some(&VfsError::NetworkLost), "the error travels");
    }

    #[test]
    fn faults_fail_writes_after_n_bytes() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_folder(&p("/a"));
        fake.switches_mut().faults.push(Fault {
            place: p("/a/x"),
            at_byte: Some(4),
            error: VfsError::StorageFull,
        });
        let mut writer = fake.write(&p("/a/x"), WriteMode::CreateNew, &ctx).unwrap();
        writer.write_all(b"ab").unwrap();
        assert!(writer.write_all(b"cdef").is_err());
        drop(writer);
        assert!(!fake.exists(&p("/a/x")));
        assert!(fake.unfinished().is_empty());
    }

    #[test]
    fn an_offline_place_and_a_drop_lose_the_connection() {
        let fake = FakeProvider::new();
        let host = Place::parse("sftp://build/").unwrap();
        let file = Place::parse("sftp://build/srv/big.bin").unwrap();
        let data: Vec<u8> = (0..100u8).collect();
        fake.add_file(&file, &data);
        let ctx = OpContext::new();
        fake.switches_mut()
            .drops
            .push(LinkDrop::after(host.clone(), 40));
        let mut reader = fake.read(&file, &ctx).unwrap();
        let mut got = Vec::new();
        let error = reader.read_to_end(&mut got).unwrap_err();
        assert_eq!(got.len(), 40, "the bytes before the drop got through");
        assert_eq!(error.kind(), io::ErrorKind::NetworkDown);
        assert!(fake.is_offline(&file));
        assert_eq!(fake.stat(&file, &ctx).unwrap_err(), VfsError::NetworkLost);
        fake.bring_back(&host);
        assert!(fake.stat(&host, &ctx).is_ok(), "the root answers again");
        let mut again = Vec::new();
        fake.read(&file, &ctx)
            .unwrap()
            .read_to_end(&mut again)
            .unwrap();
        assert_eq!(again, data, "a drop fires once");
        // A write cut the same way leaves nothing at the target.
        let target = Place::parse("sftp://build/srv/up.bin").unwrap();
        fake.switches_mut()
            .drops
            .push(LinkDrop::after(host.clone(), 30));
        let mut writer = fake.write(&target, WriteMode::CreateNew, &ctx).unwrap();
        assert!(writer.write_all(&data).is_err());
        drop(writer);
        assert!(!fake.exists(&target));
        assert!(fake.unfinished().is_empty());
        fake.bring_back(&host);
        fake.take_offline(&host);
        assert!(fake.list(&host, &ctx).is_err());
    }

    #[test]
    fn a_call_fault_fails_its_call_a_number_of_times() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"1");
        fake.switches_mut()
            .call_faults
            .push(CallFault::new(FakeCall::Remove, p("/a/x"), VfsError::NetworkLost).times(2));
        assert!(fake.stat(&p("/a/x"), &ctx).is_ok(), "other calls work");
        assert_eq!(fake.remove(&p("/a/x"), &ctx), Err(VfsError::NetworkLost));
        assert_eq!(fake.remove(&p("/a/x"), &ctx), Err(VfsError::NetworkLost));
        assert_eq!(fake.remove(&p("/a/x"), &ctx), Ok(()));
    }

    #[test]
    fn names_that_differ_by_case_are_one_entry_when_case_is_ignored() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            case_insensitive: true,
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_file(&p("/a/Report.txt"), b"1");
        assert!(fake.exists(&p("/a/report.TXT")));
        assert_eq!(
            fake.stat(&p("/a/REPORT.txt"), &ctx).unwrap().name,
            "Report.txt"
        );
        fake.rename(&p("/a/Report.txt"), &p("/a/report.txt"), &ctx)
            .unwrap();
        assert_eq!(names(&fake, &p("/a")), vec!["report.txt"]);
        assert!(!fake.capabilities(&p("/a")).case_sensitive);
    }

    #[test]
    fn read_only_and_locked_entries_refuse_changes() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            read_only: vec![p("/a/r")],
            locked: vec![p("/a/l")],
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_file(&p("/a/r"), b"1");
        fake.add_file(&p("/a/l"), b"1");
        assert_eq!(
            fake.remove(&p("/a/r"), &ctx),
            Err(VfsError::PermissionDenied)
        );
        assert_eq!(fake.remove(&p("/a/l"), &ctx), Err(VfsError::InUse));
        assert!(matches!(
            fake.write(&p("/a/r"), WriteMode::Overwrite, &ctx),
            Err(VfsError::PermissionDenied)
        ));
    }

    #[test]
    fn space_and_size_limits_are_kept() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            free_space: Some(5),
            max_file_size: Some(4),
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        fake.add_folder(&p("/a"));
        let mut big = fake
            .write(&p("/a/big"), WriteMode::CreateNew, &ctx)
            .unwrap();
        big.write_all(b"12345").unwrap();
        assert_eq!(big.commit(false), Err(VfsError::FileTooLarge));
        let mut ok = fake.write(&p("/a/ok"), WriteMode::CreateNew, &ctx).unwrap();
        ok.write_all(b"1234").unwrap();
        ok.commit(false).unwrap();
        assert_eq!(
            fake.space(&p("/a"), &ctx).unwrap().free_to_user,
            Some(1),
            "the budget shrinks"
        );
        let mut full = fake
            .write(&p("/a/two"), WriteMode::CreateNew, &ctx)
            .unwrap();
        full.write_all(b"12").unwrap();
        assert_eq!(full.commit(false), Err(VfsError::StorageFull));
    }

    #[test]
    fn reserved_names_and_created_times_follow_the_switches() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            reserved_names: true,
            no_created_time: true,
            ..FakeSwitches::default()
        });
        let ctx = OpContext::new();
        let capabilities = fake.capabilities(&p("/a"));
        assert!(capabilities.reserved_names);
        assert_eq!(capabilities.access.created_time, Can::No);
        fake.add_file(&p("/a/x"), b"1");
        let change = AttributeChange {
            created: Some(Timestamp::from_unix(5, 0)),
            ..AttributeChange::default()
        };
        assert!(fake.set_attributes(&p("/a/x"), &change, &ctx).is_err());
        let change = AttributeChange {
            modified: Some(Timestamp::from_unix(7, 0)),
            ..AttributeChange::default()
        };
        let inverse = fake.set_attributes(&p("/a/x"), &change, &ctx).unwrap();
        assert_eq!(inverse.modified, None);
        assert_eq!(
            fake.entry(&p("/a/x")).unwrap().modified,
            Some(Timestamp::from_unix(7, 0))
        );
    }

    #[test]
    fn links_are_stored_and_never_followed() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/outside/secret.txt"), b"keep");
        fake.add_link(&p("/a/out"), &p("/outside").to_string(), true);
        let entry = fake.stat(&p("/a/out"), &ctx).unwrap();
        assert_eq!(entry.kind, EntryKind::Link);
        assert!(fake.list(&p("/a/out"), &ctx).is_err(), "not a folder");
        fake.remove(&p("/a/out"), &ctx).unwrap();
        assert_eq!(fake.contents(&p("/outside/secret.txt")).unwrap(), b"keep");
    }

    #[test]
    fn hard_links_and_folder_aliases_share_an_identity() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        fake.add_file(&p("/a/x"), b"1");
        fake.add_hard_link(&p("/a/y"), &p("/a/x"));
        let x = fake.stat(&p("/a/x"), &ctx).unwrap();
        let y = fake.stat(&p("/a/y"), &ctx).unwrap();
        assert_eq!((x.volume, x.inode), (y.volume, y.inode));
        assert!(matches!(
            fake.rename(&p("/a/x"), &p("/a/y"), &ctx),
            Err(VfsError::AlreadyExists)
        ));
        fake.add_folder_alias(&p("/a/loop"), &p("/a"));
        let a = fake.stat(&p("/a"), &ctx).unwrap();
        let alias = fake.stat(&p("/a/loop"), &ctx).unwrap();
        assert_eq!(a.inode, alias.inode);
        assert_eq!(names(&fake, &p("/a/loop")), vec!["loop", "x", "y"]);
        assert_eq!(names(&fake, &p("/a/loop/loop")), vec!["loop", "x", "y"]);
        assert_eq!(fake.contents(&p("/a/loop/loop/x")).unwrap(), b"1");
    }

    #[test]
    fn remote_places_have_their_own_tree() {
        let fake = FakeProvider::new();
        let ctx = OpContext::new();
        let remote = Place::parse("fake://host/share/x.txt").unwrap();
        fake.add_file(&remote, b"far");
        assert_eq!(
            names(&fake, &Place::parse("fake://host/share").unwrap()),
            vec!["x.txt"]
        );
        fake.rename(
            &Place::parse("fake://host/share").unwrap(),
            &Place::parse("fake://host/moved").unwrap(),
            &ctx,
        )
        .unwrap();
        assert_eq!(
            fake.contents(&Place::parse("fake://host/moved/x.txt").unwrap())
                .unwrap(),
            b"far"
        );
    }
}
