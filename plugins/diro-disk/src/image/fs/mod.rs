//! The file systems the plugin reads itself, behind one trait, so the
//! tree above them ([`crate::image::tree`]) keeps the rules once: the
//! names, the limits, the loops and the depth cap.
//!
//! A reader lists one folder at a time, by its path of stored names from
//! the file system's root, and reads a file's range; it never walks the
//! whole image, and it never allocates by a size the image claims.

use std::sync::Arc;

use diroxide_core::Timestamp;
use diroxide_vfs::archive::limits::Limits;

use crate::error::DiskError;
use crate::image::sniff::Content;
use crate::image::source::Backing;
use crate::lines::cause;

pub mod ext;
pub mod fat;
pub mod iso;
pub mod ntfs;
pub mod pinned;

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A folder.
    Folder,
    /// A file.
    File,
    /// A symbolic link: its target is text, never followed.
    Link,
    /// A device, a pipe or a socket.
    Special,
}

/// One entry of a folder, as the file system stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The stored name: UTF-8 where the file system stores UTF-16 (FAT's
    /// long names, Joliet, NTFS), the bytes themselves where it stores
    /// bytes (ext, squashfs, Rock Ridge).
    pub name: Vec<u8>,
    /// What it is.
    pub kind: NodeKind,
    /// Its size in bytes; a folder's is not given.
    pub size: Option<u64>,
    /// When its contents last changed.
    pub modified: Option<Timestamp>,
    /// Unix permission bits, where the file system keeps them.
    pub mode: Option<u32>,
    /// Windows attribute words: `read_only`, `hidden`, `system`,
    /// `archive`.
    pub attributes: Vec<&'static str>,
    /// What identifies a folder in the image (an extent, an inode, a
    /// cluster), so a folder that leads back to one it is inside is cut.
    pub id: Option<u64>,
    /// A link's target, as stored.
    pub link: Option<Vec<u8>>,
    /// A note for the row, in words: "compressed by NTFS, read it with
    /// 7-Zip".
    pub note: Option<String>,
    /// Where the reader finds the contents again, its own numbers.
    pub token: Option<(u64, u64)>,
    /// A file's data as byte ranges of the volume, in order, where the
    /// reader keeps them so (ISO 9660's extents).
    pub extents: Vec<(u64, u64)>,
}

impl Node {
    /// A node of a kind with only a name.
    pub fn new(name: impl Into<Vec<u8>>, kind: NodeKind) -> Self {
        Self {
            name: name.into(),
            kind,
            size: None,
            modified: None,
            mode: None,
            attributes: Vec::new(),
            id: None,
            link: None,
            note: None,
            token: None,
            extents: Vec::new(),
        }
    }
}

/// The facts of a file system, for rows and details.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The file system in words: "ext4", "FAT32", "ISO 9660, Joliet".
    pub words: String,
    /// Its label, when it has one.
    pub label: Option<String>,
    /// Whether names differ by case.
    pub case_sensitive: bool,
    /// The bytes free, when the file system says (an image is read only
    /// here, so this is what the image holds free, for the details).
    pub free: Option<u64>,
}

/// A file system the plugin reads.
pub trait FileSystem: Send {
    /// Its facts.
    fn facts(&self) -> Facts;

    /// What identifies the root folder, for the loop check.
    fn root_id(&self) -> Option<u64> {
        None
    }

    /// The entries of the folder at `dir`, a path of stored names from the
    /// root (empty for the root), without `.` and `..`; `folder` is the
    /// entry its own folder listed (the root's is a folder with no
    /// token). A reader stops after one entry past the limit, so the tree
    /// can refuse the folder without the reader collecting all of it.
    fn list(&mut self, dir: &[Vec<u8>], folder: &Node) -> Result<Vec<Node>, DiskError>;

    /// Up to `length` bytes of the file at `path` from `offset`, fewer at
    /// its end; `node` is the entry its folder listed.
    fn read(
        &mut self,
        path: &[Vec<u8>],
        node: &Node,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError>;

    /// One entry's details, which may read more than its row did (NTFS
    /// counts a file's alternate streams here, never in a listing).
    fn detail(&mut self, path: &[Vec<u8>], node: &Node) -> Result<Node, DiskError> {
        let _ = path;
        Ok(node.clone())
    }
}

/// Opens the file system a volume holds, by what its bytes say.
pub fn open(
    content: Content,
    backing: Arc<dyn Backing>,
    limits: &Limits,
) -> Result<Box<dyn FileSystem>, DiskError> {
    match content {
        Content::Iso => iso::open(backing, limits),
        Content::Fat => fat::open(backing, limits),
        Content::Ext => ext::open(backing, limits),
        Content::Ntfs => ntfs::open(backing, limits),
        Content::BitLocker => Err(locked("BitLocker")),
        Content::Luks => Err(locked("LUKS")),
        Content::Gpt | Content::Mbr => Err(DiskError::uses("a partition table inside a partition")),
        Content::Unknown => Err(DiskError::Needs {
            cause: cause::NOT_AN_IMAGE.to_owned(),
            hint: String::new(),
        }),
        other => Err(DiskError::needs_7zip(other.words())),
    }
}

fn locked(by: &str) -> DiskError {
    DiskError::Needs {
        cause: cause::locked(by),
        hint: String::new(),
    }
}

/// FAT's and NTFS's time, a civil time with no zone, read as UTC (the
/// image does not say its zone).
pub fn civil(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Timestamp> {
    Timestamp::from_civil(
        year,
        month,
        day,
        hour,
        minute,
        second,
        diroxide_core::UtcOffset::UTC,
    )
}
