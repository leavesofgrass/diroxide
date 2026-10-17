//! The provider trait: everything a kind of place can do.

use std::fmt;
use std::io::{Read, Seek, Write};
use std::ops::ControlFlow;

use diroxide_core::{Attributes, Entry, Place, Timestamp};
use serde::{Deserialize, Serialize};

use crate::cancel::OpContext;
use crate::capabilities::Capabilities;
use crate::error::VfsError;
use crate::trash::{TrashCheck, TrashFacts, TrashGate, TrashRules, trash_gate};
use crate::volume::{SpaceInfo, VolumeInfo};

/// How a write opens its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WriteMode {
    /// Make a new entry; the commit fails if the name is taken. The safe
    /// default.
    #[default]
    CreateNew,
    /// Replace an existing entry's contents at the commit, or make it.
    Overwrite,
}

/// Who owns an entry: by id, and by name when one exists.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Principal {
    /// The decimal uid or gid, or a Windows SID string.
    pub id: String,
    /// The name, when one exists.
    pub name: Option<String>,
}

/// A change to an entry's attributes and times. `None` leaves a value as
/// it is.
///
/// `Attributes::Unix(v)` is the low twelve bits of the mode, and the
/// provider writes all twelve; `Attributes::Windows(v)` is the full value,
/// and the provider keeps the bits it cannot set unchanged. The change
/// time cannot be set. [`Provider::set_attributes`] returns the inverse: a
/// change holding the old values of exactly the fields it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeChange {
    /// New permissions or attributes.
    pub attributes: Option<Attributes>,
    /// A new creation time, where the platform can set one (Windows and
    /// macOS; Linux cannot).
    pub created: Option<Timestamp>,
    /// A new modification time.
    pub modified: Option<Timestamp>,
    /// A new access time.
    pub accessed: Option<Timestamp>,
    /// A new owner.
    pub owner: Option<Principal>,
    /// A new group.
    pub group: Option<Principal>,
    /// If set, the provider changes nothing unless the entry still has
    /// these attributes: the engine's check that nothing changed since
    /// the plan.
    pub expect: Option<Attributes>,
}

impl AttributeChange {
    /// Whether the change changes nothing.
    pub fn is_empty(&self) -> bool {
        self.attributes.is_none()
            && self.created.is_none()
            && self.modified.is_none()
            && self.accessed.is_none()
            && self.owner.is_none()
            && self.group.is_none()
    }
}

/// How a provider can copy within itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CopyHint {
    /// No fast path: the job engine reads and writes the bytes.
    #[default]
    Stream,
    /// A copy-on-write clone (reflink) is possible.
    Reflink,
    /// The server can copy without sending the bytes through diroxide.
    ServerSide,
}

/// What kind of link to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkKind {
    /// A symbolic link, absolute or relative as its target is written. On
    /// Windows it may need Developer Mode, which the error says in words.
    Symbolic,
    /// A Windows directory junction: needs no privilege, folders only, an
    /// absolute target.
    Junction,
    /// A hard link: a second name for the same file, on one volume.
    Hard,
}

/// A link to make: its kind, and its target as it will be stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LinkRequest {
    /// The kind of link.
    pub kind: LinkKind,
    /// The target, as it will be stored in the link (relative or
    /// absolute).
    pub target: String,
    /// Whether the target is a folder (Windows makes folder and file
    /// symbolic links differently).
    pub target_is_folder: bool,
}

/// What changed in a watched place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// An entry appeared.
    Created(Place),
    /// An entry changed.
    Changed(Place),
    /// An entry went away.
    Removed(Place),
    /// The watched folder itself was removed or renamed, as its parent's
    /// watch reports it (spike three): "Folder is gone", and the panel
    /// goes up.
    Gone,
    /// Too much changed to say (the 50-event threshold, the only overflow
    /// signal on Windows); list the place again.
    Rescan,
}

/// Receives watch events, on whatever thread the provider uses. It must
/// never block: send to a channel and return, or events are lost.
pub type WatchSink = Box<dyn Fn(WatchEvent) + Send + Sync>;

/// Keeps a watch alive; dropping it stops the watch at once.
pub struct WatchHandle {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl WatchHandle {
    /// A handle that runs `stop` when dropped.
    pub fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        Self {
            stop: Some(Box::new(stop)),
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

impl fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WatchHandle").finish_non_exhaustive()
    }
}

/// A reader that can also seek: what zip, 7z, the image and metadata
/// readers need. Every `Read + Seek + Send` type is one.
pub trait ReadSeek: Read + Seek + Send {}

impl<T: Read + Seek + Send> ReadSeek for T {}

/// A write in progress (research/file-operations.md C1). The bytes go to
/// an unfinished name beside the target (`.diro-part-` and eight hex
/// digits, [`crate::names::unfinished_name`]); [`FileWriter::commit`] makes
/// them the target's content. Dropping the writer without committing
/// leaves the target as it was and removes the unfinished file, so a
/// cancelled or failed write never leaves half a file.
pub trait FileWriter: Write + Send {
    /// Makes everything written the content of the target and ends the
    /// write. `durable` flushes to storage first. With
    /// [`WriteMode::CreateNew`] the commit fails if the name was taken
    /// meanwhile, and never replaces.
    fn commit(self: Box<Self>, durable: bool) -> Result<(), VfsError>;

    /// Sets times and attributes on the unfinished file before the commit's
    /// rename, so a copy lands with its dates already right (W1a's contract
    /// request 2, R8 C1). The default says the writer cannot; the engine
    /// then sets them through [`Provider::set_attributes`] after the commit.
    fn set_attributes(&mut self, change: &AttributeChange) -> Result<(), VfsError> {
        let _ = change;
        Err(VfsError::UnsupportedFeature {
            what: "attributes on an unfinished file".to_owned(),
        })
    }

    /// Where the bytes go until the commit: the unfinished `.diro-part-`
    /// place beside the target, so the journal can record which unfinished
    /// name belongs to which target and the next start can say what an
    /// interrupted write left (R8 C1; W1b's contract request 1). The
    /// default is `None`: this writer does not say, and nothing that needs
    /// it may assume a name.
    fn unfinished_place(&self) -> Option<Place> {
        None
    }
}

/// A listing delivered in batches, for providers whose folders are slow or
/// huge (a network share of 100,000 entries). The documented fallback of
/// ADR-0003: an extension beside [`Provider`], found through
/// [`Provider::as_streaming`], so the trait the engine uses does not
/// change. Planned for alpha.5's remote providers (R22); no provider has
/// it yet.
pub trait StreamingList: Send + Sync {
    /// Lists a folder, handing each batch of entries to `batch` as it
    /// arrives; `batch` returns `Break` to stop early.
    fn list_batches(
        &self,
        place: &Place,
        ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<Entry>) -> ControlFlow<()>,
    ) -> Result<(), VfsError>;
}

/// A kind of place: the local disk, archives, a plugin's protocol.
///
/// Every method blocks, so callers run them on worker threads, never on the
/// input thread. Every method takes an [`OpContext`], checks its token
/// between steps, and reports progress where it can. Operations act on one
/// entry; the job engine does the walking.
///
/// The job engine's order is create, verify, then destroy (plan section
/// 5.4): it writes and checks a replacement before it removes anything, it
/// tries [`Provider::copy_within`] (a clone) before touching the
/// destination, and a trash failure never falls through to a permanent
/// delete.
pub trait Provider: Send + Sync + fmt::Debug {
    /// The scheme this provider serves: `file`, `archive`, or a plugin's.
    fn scheme(&self) -> &str;

    /// What can be done at a place, read per place.
    fn capabilities(&self, place: &Place) -> Capabilities;

    /// The entries in a folder, in no particular order.
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError>;

    /// One entry's details, without following a final link.
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError>;

    /// Opens a file for reading from the start.
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError>;

    /// Opens a file for reading with random access, where
    /// [`Capabilities::random_read`] says it can without a spool.
    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        let _ = (place, ctx);
        Err(VfsError::unsupported(
            self.scheme(),
            "read with random access",
        ))
    }

    /// Starts a write of a file. Nothing changes at the target until the
    /// writer's [`FileWriter::commit`].
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError>;

    /// Makes one folder; its parent must exist.
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError>;

    /// Makes a link at `place`. Unsupported unless the capabilities say
    /// `links`.
    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        let _ = (place, request, ctx);
        Err(VfsError::unsupported(self.scheme(), "make links"))
    }

    /// Renames or moves one entry within this provider. Never replaces:
    /// it fails with `AlreadyExists` when the target is a different entry,
    /// atomically where the platform allows. A change of case or of
    /// Unicode form of the same entry succeeds, on a place that ignores
    /// case or form. Across volumes it fails with `CrossesDevices`, and
    /// the engine copies, verifies, then removes.
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError>;

    /// Removes one file, link or empty folder for good. A link is removed,
    /// never what it points to. The job engine lists everything first and
    /// uses the trash by default.
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError>;

    /// The trash gate's decision for one item, before any call. The
    /// default: the platform's rules over the volume's kind and the path's
    /// length. A folder's longest inner path is the engine's to add, from
    /// its walk.
    fn trash_gate(&self, place: &Place, ctx: &OpContext) -> Result<TrashGate, VfsError> {
        if !self.capabilities(place).trash {
            return Ok(TrashGate::NoTrashHere(self.volume(place, ctx)?.kind));
        }
        let volume = self.volume(place, ctx)?;
        let facts = TrashFacts {
            volume: volume.kind,
            longest_path_units: crate::limits::utf16_units(&place.to_string()),
            network_path: place.remote().is_some(),
            // A provider that copies across drives says so in its own gate.
            copies_across_drives: false,
        };
        Ok(trash_gate(&facts, TrashRules::native()))
    }

    /// Moves one entry to the trash, behind the gate, and checks the
    /// result. A provider refuses with `TrashRefused` what the gate
    /// refuses; after the call it answers what the post-check found. The
    /// engine reports anything but [`TrashCheck::InTrash`] as a problem,
    /// and never falls through to a permanent delete. Unsupported unless
    /// the capabilities say `trash`.
    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        let _ = (place, ctx);
        Err(VfsError::unsupported(self.scheme(), "move to the trash"))
    }

    /// Restores the newest trashed entry whose former place is `original`.
    /// Fails with `AlreadyExists` when the name is taken; never replaces.
    /// Unsupported unless the capabilities say `restore`.
    fn restore(&self, original: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let _ = (original, ctx);
        Err(VfsError::unsupported(
            self.scheme(),
            "restore from the trash",
        ))
    }

    /// Changes attributes and times, and returns the inverse: a change
    /// holding, for exactly the fields that were changed, the values the
    /// entry had just before, read through the same handle. Applying it
    /// restores them; undo needs nothing more.
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError>;

    /// Whether a copy from `from` to `to`, both served by this provider,
    /// has a fast path.
    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        let _ = (from, to);
        CopyHint::Stream
    }

    /// Copies one file by the fast path that [`Provider::copy_hint`]
    /// named: a clone or a server-side copy, tried before the destination
    /// is touched. Unsupported when there is none; the engine then streams.
    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        let _ = (from, to, ctx);
        Err(VfsError::unsupported(
            self.scheme(),
            "copy without streaming",
        ))
    }

    /// The volume a place is on, asked of the path's own volume root. The
    /// default is a volume of unknown kind at the place's top.
    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        let _ = ctx;
        Ok(VolumeInfo::unknown(VolumeInfo::top_of(place)))
    }

    /// Free space and size limits at a place. The default knows nothing.
    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        let _ = (place, ctx);
        Ok(SpaceInfo::default())
    }

    /// Starts watching a folder. `Ok(None)` when this provider cannot, or
    /// will not (a network drive): the app then says "Folder not watched"
    /// with its reload key.
    fn watch(&self, place: &Place, sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        let _ = (place, sink);
        Ok(None)
    }

    /// The streaming listing, when this provider has one.
    fn as_streaming(&self) -> Option<&dyn StreamingList> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn dropping_a_watch_handle_stops_it() {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        let handle = WatchHandle::new(move || flag.store(true, Ordering::Relaxed));
        assert!(!stopped.load(Ordering::Relaxed));
        drop(handle);
        assert!(stopped.load(Ordering::Relaxed));
    }

    #[test]
    fn readers_that_seek_are_read_seek() {
        let boxed: Box<dyn ReadSeek> = Box::new(Cursor::new(vec![1u8, 2, 3]));
        let mut boxed = boxed;
        let mut byte = [0u8; 1];
        boxed.seek(std::io::SeekFrom::End(-1)).unwrap();
        boxed.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [3]);
    }

    #[test]
    fn an_empty_change_is_empty() {
        assert!(AttributeChange::default().is_empty());
        let change = AttributeChange {
            expect: Some(Attributes::Unix(0o644)),
            ..AttributeChange::default()
        };
        assert!(change.is_empty(), "expect alone changes nothing");
    }

    /// A stub provider, to check the trait's defaults through a trait
    /// object.
    #[derive(Debug)]
    struct Stub;

    impl Provider for Stub {
        fn scheme(&self) -> &str {
            "stub"
        }
        fn capabilities(&self, _: &Place) -> Capabilities {
            Capabilities::read_only()
        }
        fn list(&self, _: &Place, _: &OpContext) -> Result<Vec<Entry>, VfsError> {
            Ok(Vec::new())
        }
        fn stat(&self, _: &Place, _: &OpContext) -> Result<Entry, VfsError> {
            Err(VfsError::NotFound)
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

    #[test]
    fn the_defaults_say_what_a_provider_cannot_do() {
        let provider: Arc<dyn Provider> = Arc::new(Stub);
        let ctx = OpContext::new();
        let place = Place::parse(if cfg!(windows) { "D:\\a\\b" } else { "/a/b" }).unwrap();
        assert!(matches!(
            provider.open_seekable(&place, &ctx),
            Err(VfsError::Unsupported { .. })
        ));
        assert!(matches!(
            provider.restore(&place, &ctx),
            Err(VfsError::Unsupported { .. })
        ));
        assert!(matches!(
            provider.trash(&place, &ctx),
            Err(VfsError::Unsupported { .. })
        ));
        assert_eq!(
            provider.trash_gate(&place, &ctx).unwrap(),
            TrashGate::NoTrashHere(crate::VolumeKind::Unknown)
        );
        let link = LinkRequest {
            kind: LinkKind::Symbolic,
            target: "b".into(),
            target_is_folder: false,
        };
        assert!(provider.create_link(&place, &link, &ctx).is_err());
        assert_eq!(provider.space(&place, &ctx).unwrap(), SpaceInfo::default());
        assert!(provider.volume(&place, &ctx).unwrap().root.is_root());
        assert!(provider.as_streaming().is_none());
    }
}
