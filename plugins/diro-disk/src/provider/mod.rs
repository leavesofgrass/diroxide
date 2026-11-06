//! The `disk` provider: the `provider/*` methods the host calls for a
//! place inside a claimed image, `fedora.iso!EFI/BOOT` (R29 1.7).
//!
//! - **Each image opens once,** by its container's place, and stays open
//!   while its places are used; `provider/disconnect` (the last lease
//!   ended) closes it.
//! - **A change is noticed:** at most every two seconds a call compares
//!   the image file's size and time with what was opened; a change closes
//!   the image, says "Image changed since it was opened", and answers
//!   `conflict`, and the next call opens it again.
//! - **An image inside an image** (`disk.img!partition-1/boot.iso!`) is
//!   read through the outer image's own reader, never back through the
//!   host, so the host never loops into the plugin; the host counts the
//!   levels before it asks (four by default), and the plugin keeps its
//!   own count too.
//! - **Served:** `connect`, `list`, `stat`, `read`, `volumes`; the
//!   manifest leaves every writing method out, so the host refuses those
//!   without asking.

pub mod connect;
pub mod list;
pub mod read;
pub mod stat;
pub mod volumes;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use diroxide_core::Place;
use diroxide_core::place::opens_as_archive;
use diroxide_plugins::base64;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError};
use diroxide_plugins::shapes::{WireEntry, format_time};
use diroxide_vfs::archive::limits::Limits;
use serde_json::Value;

use crate::error::DiskError;
use crate::host::{self, HostFile, Stamp};
use crate::image::fs::NodeKind;
use crate::image::sniff::{self, Container};
use crate::image::source::{Backing, Cached};
use crate::image::source::{SpoolFile, read_vec};
use crate::image::tree::Row;
use crate::image::{Image, Inner, Volume};
use crate::lines;
use crate::rpc::{Cancel, Peer};
use crate::spool::Spool;
use crate::tool::{SevenZip, SevenZipFs};

/// The image extensions the manifest claims, lowercase with the dot: the
/// plugin reads an image inside one of these itself.
pub const CLAIMED: &[&str] = &[
    ".iso",
    ".vhd",
    ".vhdx",
    ".vdi",
    ".vmdk",
    ".qcow2",
    ".squashfs",
    ".sfs",
    ".img",
    ".raw",
    ".bin",
    ".dd",
];

/// How often an open image's file is checked for a change.
pub const CHECK_EVERY: Duration = Duration::from_secs(2);

/// One request from the host, with what answering it needs.
#[derive(Clone, Copy)]
pub struct Call<'a> {
    /// The host.
    pub peer: &'a Arc<Peer>,
    /// The request's parameters.
    pub params: &'a Params,
    /// The request's cancel flag.
    pub cancel: &'a Cancel,
    /// The open images.
    pub images: &'a Images,
}

impl Call<'_> {
    /// A text parameter.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(Value::as_str)
    }

    /// A number parameter.
    pub fn number(&self, key: &str) -> Option<u64> {
        self.params.get(key).and_then(Value::as_u64)
    }

    /// The place in the parameter `key`.
    pub fn at(&self, key: &str) -> Result<At, DiskError> {
        let text = self.text(key).ok_or_else(|| {
            DiskError::Host(RpcError::new(
                ErrorKind::InvalidParams,
                "a place is missing",
            ))
        })?;
        At::parse(text)
    }
}

/// A place inside an image: the image's own place and the path in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
    /// The place as the host gave it.
    pub place: Place,
    /// The image file's place: `D:\dl\fedora.iso`.
    pub container: Place,
    /// The path inside, `/`-separated, empty at the root.
    pub inner: String,
}

impl At {
    /// A place from its text; a place that is not inside an image is not
    /// found (the provider's scheme alone names nothing).
    pub fn parse(text: &str) -> Result<Self, DiskError> {
        let place = Place::parse(text).map_err(|_| DiskError::NotFound)?;
        let (container, inner) = place.archive().ok_or(DiskError::NotFound)?;
        Ok(Self {
            container: container.clone(),
            inner: inner.to_owned(),
            place,
        })
    }

    /// The image file's name: "fedora.iso".
    pub fn image_name(&self) -> String {
        self.container.name().unwrap_or("image").to_owned()
    }
}

/// Whether a container place names an image the plugin claims.
pub fn claimed(container: &Place) -> bool {
    let name = container.name().unwrap_or_default().to_lowercase();
    CLAIMED.iter().any(|ext| {
        name.strip_suffix(ext)
            .is_some_and(|stem| !stem.is_empty() && !stem.bytes().all(|b| b == b'.'))
    })
}

/// An open image, and when its file was last compared.
pub struct Open {
    /// The image.
    pub image: Image,
    checked: Mutex<Instant>,
    /// The plugin's files in the spool for this image, removed with it.
    pub spool: Option<Arc<Spool>>,
    /// Held while a partition is handed to 7-Zip, so it is handed once.
    falling_back: Mutex<()>,
}

/// The open images, by their container's place.
pub struct Images {
    open: Mutex<HashMap<String, Arc<Open>>>,
    /// The limits every image is read under.
    pub limits: Limits,
    /// The host's spool folder, from `initialize`.
    pub spool: Mutex<Option<PathBuf>>,
    /// 7-Zip as the host finds it, looked for once.
    seven: Mutex<Option<Option<SevenZip>>>,
}

impl Default for Images {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl Images {
    /// No image open, under these limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            open: Mutex::new(HashMap::new()),
            limits,
            spool: Mutex::new(None),
            seven: Mutex::new(None),
        }
    }

    /// The image at a container's place: the open one, checked for a
    /// change at most every [`CHECK_EVERY`], or opened now.
    pub fn get(&self, peer: &Arc<Peer>, container: &Place) -> Result<Arc<Open>, DiskError> {
        self.get_at_depth(peer, container, 0)
    }

    fn get_at_depth(
        &self,
        peer: &Arc<Peer>,
        container: &Place,
        depth: usize,
    ) -> Result<Arc<Open>, DiskError> {
        if depth > self.limits.max_nesting {
            return Err(DiskError::Refused(lines::cause::nested(
                self.limits.max_nesting,
            )));
        }
        let key = container.to_string();
        let cached = self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(open) = cached {
            if self.still_the_same(peer, container, &open)? {
                return Ok(open);
            }
            self.close(&key);
            host::announce(peer, lines::IMAGE_CHANGED, "warning");
            return Err(DiskError::Changed);
        }
        let open = Arc::new(self.open_now(peer, container, depth)?);
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, Arc::clone(&open));
        Ok(open)
    }

    /// Whether the image file is as it was opened; checked at most every
    /// [`CHECK_EVERY`]. An image inside an image is checked through its
    /// outer one.
    fn still_the_same(
        &self,
        peer: &Arc<Peer>,
        container: &Place,
        open: &Open,
    ) -> Result<bool, DiskError> {
        let mut checked = open.checked.lock().unwrap_or_else(PoisonError::into_inner);
        if checked.elapsed() < CHECK_EVERY {
            return Ok(true);
        }
        *checked = Instant::now();
        drop(checked);
        if let Some((outer, _)) = container.archive()
            && claimed(outer)
        {
            return match self.get(peer, outer) {
                Ok(_) => Ok(true),
                Err(DiskError::Changed) => Ok(false),
                Err(other) => Err(other),
            };
        }
        let now = host::stat(peer, &container.to_string())?;
        Ok(now == open.image.stamp)
    }

    fn open_now(
        &self,
        peer: &Arc<Peer>,
        container: &Place,
        depth: usize,
    ) -> Result<Open, DiskError> {
        let name = container.name().unwrap_or("image").to_owned();
        let (file, stamp): (Arc<dyn Backing>, Stamp) = match container.archive() {
            Some((outer, inner)) if claimed(outer) => {
                let outer_open = self.get_at_depth(peer, outer, depth + 1)?;
                let inside = Inside::new(outer_open, inner)?;
                let stamp = Stamp {
                    size: inside.len,
                    modified: None,
                };
                (Arc::new(inside), stamp)
            }
            _ => {
                let place = container.to_string();
                let stamp = host::stat(peer, &place)?;
                let file = HostFile::new(Arc::clone(peer), place, stamp.size);
                (Arc::new(file), stamp)
            }
        };
        let file: Arc<dyn Backing> = Arc::new(Cached::new(file));
        let siblings = |sibling: &str| self.sibling(peer, container, sibling, depth);
        let spool = self.new_spool();
        let image = match Image::open(Arc::clone(&file), &name, stamp, &self.limits, &siblings) {
            Ok(image) => image,
            Err(error) if wants_fallback(&error) => self
                .fall_back(peer, container, &file, &name, stamp, spool.as_ref(), depth)?
                .ok_or(error)?,
            Err(error) => return Err(error),
        };
        Ok(Open {
            image,
            checked: Mutex::new(Instant::now()),
            spool,
            falling_back: Mutex::new(()),
        })
    }

    /// A file beside an image, by its plain name (a VMDK's flat extent):
    /// through the outer image when the image is inside one, else through
    /// the host.
    fn sibling(
        &self,
        peer: &Arc<Peer>,
        container: &Place,
        name: &str,
        depth: usize,
    ) -> Result<Arc<dyn Backing>, DiskError> {
        match container.archive() {
            Some((outer, inner)) if claimed(outer) => {
                let outer_open = self.get_at_depth(peer, outer, depth + 1)?;
                let folder = inner.rsplit_once('/').map_or("", |(folder, _)| folder);
                let path = if folder.is_empty() {
                    name.to_owned()
                } else {
                    format!("{folder}/{name}")
                };
                Ok(Arc::new(Inside::new(outer_open, &path)?))
            }
            _ => {
                let place = container
                    .parent()
                    .and_then(|folder| folder.join(name).ok())
                    .ok_or(DiskError::NotFound)?
                    .to_string();
                let stamp = host::stat(peer, &place)?;
                let file = HostFile::new(Arc::clone(peer), place, stamp.size);
                Ok(Arc::new(Cached::new(Arc::new(file))))
            }
        }
    }

    /// A spool for one image, when the host named a spool folder.
    fn new_spool(&self) -> Option<Arc<Spool>> {
        let dir = self
            .spool
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        Some(Arc::new(Spool::new(dir, self.limits.max_spool_disk_bytes)))
    }

    /// 7-Zip, looked for once a session.
    pub fn seven(&self, peer: &Arc<Peer>) -> Result<Option<SevenZip>, DiskError> {
        let mut seven = self.seven.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = seven.as_ref() {
            return Ok(found.clone());
        }
        let found = SevenZip::find(peer)?;
        *seven = Some(found.clone());
        Ok(found)
    }

    /// A path the host's 7-Zip reads for an image file: its own path when
    /// it is on a local drive, else a copy in the spool.
    fn tool_path(
        container: &Place,
        file: &Arc<dyn Backing>,
        spool: Option<&Arc<Spool>>,
    ) -> Result<String, DiskError> {
        if let Some(path) = container.local_path() {
            return Ok(path.to_owned());
        }
        let spool = spool.ok_or_else(|| DiskError::needs_7zip("this image"))?;
        let len = file.len();
        let path = spool.copy_in("image", len, len, |at, n| {
            read_vec(file.as_ref(), at, n).map_err(|e| DiskError::from_io(&e))
        })?;
        Ok(path.display().to_string())
    }

    /// The image read by 7-Zip, or else as the raw disk `qemu-img`
    /// converts it to; `None` when neither tool is there or opens it.
    #[allow(clippy::too_many_arguments)]
    fn fall_back(
        &self,
        peer: &Arc<Peer>,
        container: &Place,
        file: &Arc<dyn Backing>,
        name: &str,
        stamp: Stamp,
        spool: Option<&Arc<Spool>>,
        depth: usize,
    ) -> Result<Option<Image>, DiskError> {
        let head = read_vec(file.as_ref(), 0, 4096).unwrap_or_default();
        let tail = read_vec(file.as_ref(), file.len().saturating_sub(512), 512).unwrap_or_default();
        let kind = sniff::container(&head, &tail);
        if let Some(seven) = self.seven(peer)? {
            let path = Self::tool_path(container, file, spool)?;
            if let Some(fs) = SevenZipFs::open(seven, path, spool.cloned(), file.len())? {
                crate::tool::announce_read_with_7zip(peer);
                return Ok(Some(Image::with_file_system(
                    Arc::clone(file),
                    name,
                    kind,
                    stamp,
                    Box::new(fs),
                    &self.limits,
                )));
            }
        }
        let Some(spool) = spool else {
            return Ok(None);
        };
        if kind == Container::Raw {
            return Ok(None);
        }
        let source = Self::tool_path(container, file, Some(spool))?;
        let Some(raw) = crate::tool::qemu::convert(peer, spool, kind, &source, file.len())? else {
            return Ok(None);
        };
        let raw = std::fs::File::open(&raw).map_err(|e| DiskError::Io(e.to_string()))?;
        let raw: Arc<dyn Backing> =
            Arc::new(SpoolFile::open(raw).map_err(|e| DiskError::Io(e.to_string()))?);
        let siblings = |sibling: &str| self.sibling(peer, container, sibling, depth);
        let mut image = Image::open(raw, name, stamp, &self.limits, &siblings)?;
        image.held = file.len();
        image.container = kind;
        Ok(Some(image))
    }

    /// The volume a call names, handed to 7-Zip first when no built-in
    /// reader opens it (an exFAT partition, an xz squashfs): its bytes are
    /// copied into the spool, inside the budget, and 7-Zip lists them.
    pub fn volume<'a>(
        &self,
        peer: &Arc<Peer>,
        open: &'a Open,
        partition: Option<u32>,
    ) -> Result<&'a Volume, DiskError> {
        let volume = open.image.volume(partition)?;
        let needs_7zip = matches!(
            volume.with_tree(|_| Ok(())),
            Err(DiskError::Needs { ref hint, .. }) if hint == lines::cause::GET_7ZIP
        );
        if !needs_7zip {
            return Ok(volume);
        }
        let _once = open
            .falling_back
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if volume.failure().is_none() {
            return Ok(volume);
        }
        let (Some(seven), Some(spool)) = (self.seven(peer)?, open.spool.as_ref()) else {
            return Ok(volume);
        };
        let backing = Arc::clone(&volume.backing);
        let len = backing.len();
        let path = spool.copy_in("part.img", len, open.image.held, |at, n| {
            read_vec(backing.as_ref(), at, n).map_err(|e| DiskError::from_io(&e))
        })?;
        if let Some(fs) = SevenZipFs::open(
            seven,
            path.display().to_string(),
            Some(Arc::clone(spool)),
            open.image.held,
        )? {
            crate::tool::announce_read_with_7zip(peer);
            volume.set_file_system(Box::new(fs));
        }
        Ok(volume)
    }

    /// Opens an image from bytes the caller has, for the tests and the
    /// fuzz runs.
    pub fn insert(&self, container: &Place, image: Image) -> Arc<Open> {
        let open = Arc::new(Open {
            image,
            checked: Mutex::new(Instant::now()),
            spool: self.new_spool(),
            falling_back: Mutex::new(()),
        });
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(container.to_string(), Arc::clone(&open));
        open
    }

    /// Closes the image at a container's place, and every image inside
    /// it.
    pub fn close(&self, key: &str) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let inside = format!("{key}!");
        open.retain(|k, _| k != key && !k.starts_with(&inside));
    }

    /// Closes every image.
    pub fn close_all(&self) {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// How many images are open.
    pub fn len(&self) -> usize {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether no image is open.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether an error at open is one 7-Zip or `qemu-img` may answer: a
/// format or a feature the plugin does not read, or a structure a crate
/// refused.
fn wants_fallback(error: &DiskError) -> bool {
    matches!(error, DiskError::Needs { .. } | DiskError::Damaged(_))
}

/// A file inside an open image, read as the bytes of an image inside it.
struct Inside {
    outer: Arc<Open>,
    partition: Option<u32>,
    names: Vec<String>,
    len: u64,
}

impl Inside {
    fn new(outer: Arc<Open>, inner: &str) -> Result<Self, DiskError> {
        let parts = outer.image.inner(inner)?;
        let partition = parts.partition;
        let names: Vec<String> = parts.names.iter().map(|s| (*s).to_owned()).collect();
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        let found = outer
            .image
            .volume(partition)?
            .with_tree(|tree| tree.resolve(&borrowed))?;
        if found.node.kind != NodeKind::File {
            return Err(DiskError::IsAFolder);
        }
        let len = found.node.size.unwrap_or(0);
        Ok(Self {
            outer,
            partition,
            names,
            len,
        })
    }
}

impl Backing for Inside {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        let bytes = self
            .outer
            .image
            .volume(self.partition)
            .and_then(|v| v.with_tree(|tree| tree.read(&names, offset, buffer.len())))
            .map_err(crate::error::HostIo::into_io)?;
        let n = bytes.len().min(buffer.len());
        buffer[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }
}

/// A row of a file system as the wire's entry.
pub fn wire_entry(row: &Row) -> WireEntry {
    let node = &row.node;
    let kind = match node.kind {
        NodeKind::Folder => diroxide_core::EntryKind::Folder,
        NodeKind::File if opens_as_archive(&row.shown) => diroxide_core::EntryKind::Archive,
        NodeKind::File => diroxide_core::EntryKind::File,
        NodeKind::Link => diroxide_core::EntryKind::Link,
        NodeKind::Special => diroxide_core::EntryKind::Special,
    };
    let mut extra = std::collections::BTreeMap::new();
    let mut notes = Vec::new();
    if let Some(note) = &node.note {
        notes.push(note.clone());
    }
    let link_target = node
        .link
        .as_ref()
        .map(|t| String::from_utf8_lossy(t).into_owned());
    if let Some(target) = &link_target {
        notes.push(lines::link_to(target));
    }
    if !notes.is_empty() {
        extra.insert(lines::ABOUT.to_owned(), notes.join("; "));
    }
    WireEntry {
        name: row.shown.clone(),
        name_raw: row.needs_care.then(|| base64::encode(&node.name)),
        place: None,
        kind,
        size: if node.kind == NodeKind::Folder {
            None
        } else {
            node.size
        },
        modified: node.modified.map(format_time),
        mode: node.mode,
        attributes: node.attributes.iter().map(|a| (*a).to_owned()).collect(),
        owner: None,
        link_target,
        mime: None,
        revision: None,
        extra,
    }
}

/// A folder's entry with words for its row: a partition, the image's
/// root.
pub fn folder_entry(name: &str, size: Option<u64>, about: Option<String>) -> WireEntry {
    let mut extra = std::collections::BTreeMap::new();
    if let Some(about) = about {
        extra.insert(lines::ABOUT.to_owned(), about);
    }
    WireEntry {
        name: name.to_owned(),
        name_raw: None,
        place: None,
        kind: diroxide_core::EntryKind::Folder,
        size,
        modified: None,
        mode: None,
        attributes: Vec::new(),
        owner: None,
        link_target: None,
        mime: None,
        revision: None,
        extra,
    }
}

/// The volume and the names an inner path holds, for a call.
pub fn inner_of<'a>(open: &'a Open, at: &'a At) -> Result<Inner<'a>, DiskError> {
    open.image.inner(&at.inner)
}
