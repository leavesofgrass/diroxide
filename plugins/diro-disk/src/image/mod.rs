//! An image opened: its container decoded into a disk, the disk's
//! partitions or its one file system, each read under the tree's rules.
//!
//! - **The container** ([`sniff::container`]) is told from the file's
//!   first and last bytes; a raw image is its own disk, a fixed VHD is
//!   its disk and a 512-byte footer.
//! - **The layout:** a disk with a partition table lists one folder per
//!   partition, `partition-1`, `partition-2`, in table order, even when
//!   there is one (the path never changes meaning); a disk with no table
//!   is its file system, so `fedora.iso!EFI/BOOT` names a folder in it.
//! - **A volume opens when first used**, so an image of eight partitions
//!   reads one table and eight boot sectors to list its root.

pub mod container;
pub mod fs;
pub mod sniff;
pub mod source;
pub mod table;
pub mod tree;

use std::sync::{Arc, Mutex, PoisonError};

use diroxide_vfs::archive::limits::Limits;

use crate::error::DiskError;
use crate::host::Stamp;
use crate::image::container::Siblings;
use crate::image::fs::FileSystem;
use crate::image::sniff::{Container, Content};
use crate::image::source::{Backing, Slice, read_vec};
use crate::image::tree::Tree;
use crate::lines::{self, PartitionWords, cause};

/// The prefix of a partition's folder name: `partition-2`.
pub const PARTITION_PREFIX: &str = "partition-";

/// A volume: a partition's bytes, or the whole disk's when it has no
/// table, and its file system once opened.
pub struct Volume {
    /// Its bytes.
    pub backing: Arc<dyn Backing>,
    /// What it holds, by its bytes.
    pub content: Content,
    tree: Mutex<Opened>,
    limits: Limits,
}

enum Opened {
    Not,
    Open(Tree),
    Failed(DiskError),
}

impl Volume {
    /// A volume over `backing`, told by its bytes.
    pub fn new(backing: Arc<dyn Backing>, limits: Limits) -> Result<Self, DiskError> {
        let content = sniff::content(backing.as_ref()).map_err(|e| DiskError::from_io(&e))?;
        Ok(Self::with_content(backing, content, limits))
    }

    /// A volume whose content is already known.
    pub fn with_content(backing: Arc<dyn Backing>, content: Content, limits: Limits) -> Self {
        Self {
            backing,
            content,
            tree: Mutex::new(Opened::Not),
            limits,
        }
    }

    /// Runs `f` on the volume's tree, opening its file system the first
    /// time; a volume that failed to open fails the same way after.
    pub fn with_tree<T>(
        &self,
        f: impl FnOnce(&mut Tree) -> Result<T, DiskError>,
    ) -> Result<T, DiskError> {
        let mut opened = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*opened, Opened::Not) {
            *opened = match fs::open(self.content, Arc::clone(&self.backing), &self.limits) {
                Ok(fs) => Opened::Open(Tree::new(fs, self.limits)),
                Err(DiskError::Cancelled) => return Err(DiskError::Cancelled),
                Err(error) => Opened::Failed(error),
            };
        }
        match &mut *opened {
            Opened::Open(tree) => f(tree),
            Opened::Failed(error) => Err(error.clone()),
            Opened::Not => Err(DiskError::Cancelled),
        }
    }

    /// The file system's words, when it opens: "ext4", "FAT32".
    pub fn file_system_words(&self) -> Option<String> {
        if self.content.is_locked() {
            return Some(self.content.words().to_owned());
        }
        match self.with_tree(|tree| Ok(tree.facts().words)) {
            Ok(words) => Some(words),
            Err(DiskError::Cancelled) => None,
            Err(_) if self.content == Content::Unknown => None,
            Err(_) => Some(self.content.words().to_owned()),
        }
    }

    /// The file system's label, when it opens and has one.
    pub fn label(&self) -> Option<String> {
        self.with_tree(|tree| Ok(tree.facts().label)).ok().flatten()
    }

    /// Replaces the file system with one the caller opened (the 7-Zip
    /// fallback).
    pub fn set_file_system(&self, fs: Box<dyn FileSystem>) {
        let mut opened = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        *opened = Opened::Open(Tree::new(fs, self.limits));
    }

    /// Whether opening failed, and how.
    pub fn failure(&self) -> Option<DiskError> {
        let opened = self.tree.lock().unwrap_or_else(PoisonError::into_inner);
        match &*opened {
            Opened::Failed(error) => Some(error.clone()),
            _ => None,
        }
    }
}

/// One partition of a disk with a table.
pub struct Partition {
    /// Its number, from 1, in table order.
    pub number: u32,
    /// Its name in the table, when it has one.
    pub name: Option<String>,
    /// The table's type in words.
    pub kind: Option<String>,
    /// Where it starts on the disk, in bytes.
    pub start: u64,
    /// Its volume.
    pub volume: Volume,
}

impl Partition {
    /// Its folder's name: `partition-2`.
    pub fn folder(&self) -> String {
        format!("{PARTITION_PREFIX}{}", self.number)
    }

    /// The words for its row and its volume.
    pub fn row(&self) -> String {
        let fs = self.volume.file_system_words();
        let label = self.name.clone().or_else(|| self.volume.label());
        lines::partition_row(&PartitionWords {
            number: self.number,
            label: label.as_deref(),
            kind: self.kind.as_deref(),
            file_system: fs.as_deref(),
            size: self.volume.backing.len(),
            locked: self.volume.content.is_locked(),
        })
    }

    /// Its name in the volumes list.
    pub fn volume_name(&self) -> String {
        let fs = self.volume.file_system_words();
        let label = self.name.clone().or_else(|| self.volume.label());
        lines::partition_volume(&PartitionWords {
            number: self.number,
            label: label.as_deref(),
            kind: self.kind.as_deref(),
            file_system: fs.as_deref(),
            size: self.volume.backing.len(),
            locked: self.volume.content.is_locked(),
        })
    }
}

/// How the disk is laid out.
pub enum Layout {
    /// One file system, no table.
    Single(Box<Volume>),
    /// A partition table and its partitions.
    Table {
        /// The table's kind in words: "GPT", "MBR".
        scheme: &'static str,
        /// The partitions, in table order.
        partitions: Vec<Partition>,
    },
}

/// An image opened.
pub struct Image {
    /// The image file's name, for the words: "fedora.iso".
    pub name: String,
    /// What the container is.
    pub container: Container,
    /// The disk's size: what a sparse container claims.
    pub claimed: u64,
    /// The image file's size: what it holds.
    pub held: u64,
    /// The image file's size and time when it was opened.
    pub stamp: Stamp,
    /// The disk's layout.
    pub layout: Layout,
}

/// The parts of an inner path: the partition it is in, when the disk has
/// a table, and the names inside the file system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inner<'a> {
    /// The partition's number, for a disk with a table.
    pub partition: Option<u32>,
    /// The names inside the file system.
    pub names: Vec<&'a str>,
}

impl Image {
    /// Opens an image from its file's bytes.
    pub fn open(
        file: Arc<dyn Backing>,
        name: &str,
        stamp: Stamp,
        limits: &Limits,
        siblings: Siblings<'_>,
    ) -> Result<Self, DiskError> {
        let head = read_vec(file.as_ref(), 0, 4096).map_err(|e| DiskError::from_io(&e))?;
        let tail_at = file.len().saturating_sub(512);
        let tail = read_vec(file.as_ref(), tail_at, 512).map_err(|e| DiskError::from_io(&e))?;
        let container = sniff::container(&head, &tail);
        let held = file.len();
        let disk = container::disk_of(container, file, limits, siblings)?;
        let claimed = disk.len();
        let content = sniff::content(disk.as_ref()).map_err(|e| DiskError::from_io(&e))?;
        let layout = layout_of(content, disk, limits)?;
        Ok(Self {
            name: name.to_owned(),
            container,
            claimed,
            held,
            stamp,
            layout,
        })
    }

    /// An image read by a file system the caller opened over the whole
    /// file (the 7-Zip fallback): no table, its size what the file holds.
    pub fn with_file_system(
        file: Arc<dyn Backing>,
        name: &str,
        container: Container,
        stamp: Stamp,
        fs: Box<dyn FileSystem>,
        limits: &Limits,
    ) -> Self {
        let held = file.len();
        let volume = Volume::with_content(file, Content::Unknown, *limits);
        volume.set_file_system(fs);
        Self {
            name: name.to_owned(),
            container,
            claimed: held,
            held,
            stamp,
            layout: Layout::Single(Box::new(volume)),
        }
    }

    /// Splits an inner path (`partition-2/home/ada`) by the layout.
    pub fn inner<'a>(&self, path: &'a str) -> Result<Inner<'a>, DiskError> {
        let mut names: Vec<&str> = path.split('/').filter(|n| !n.is_empty()).collect();
        match &self.layout {
            Layout::Single(_) => Ok(Inner {
                partition: None,
                names,
            }),
            Layout::Table { .. } => {
                if names.is_empty() {
                    return Ok(Inner {
                        partition: None,
                        names,
                    });
                }
                let first = names.remove(0);
                let number = first
                    .strip_prefix(PARTITION_PREFIX)
                    .and_then(|n| n.parse::<u32>().ok())
                    .filter(|n| n.to_string() == first[PARTITION_PREFIX.len()..])
                    .ok_or(DiskError::NotFound)?;
                Ok(Inner {
                    partition: Some(number),
                    names,
                })
            }
        }
    }

    /// The volume a partition number names, or the one volume.
    pub fn volume(&self, partition: Option<u32>) -> Result<&Volume, DiskError> {
        match (&self.layout, partition) {
            (Layout::Single(volume), None) => Ok(volume),
            (Layout::Table { partitions, .. }, Some(number)) => {
                let partition = partitions
                    .iter()
                    .find(|p| p.number == number)
                    .ok_or(DiskError::NotFound)?;
                if partition.volume.content == Content::Unknown {
                    // Swap, a BIOS boot area, an empty slot: no files, in
                    // the table's words.
                    let what = partition.kind.as_deref().unwrap_or("no file system");
                    return Err(DiskError::Needs {
                        cause: cause::no_files(what),
                        hint: String::new(),
                    });
                }
                Ok(&partition.volume)
            }
            _ => Err(DiskError::NotFound),
        }
    }

    /// The partition a number names.
    pub fn partition(&self, number: u32) -> Option<&Partition> {
        match &self.layout {
            Layout::Table { partitions, .. } => partitions.iter().find(|p| p.number == number),
            Layout::Single(_) => None,
        }
    }

    /// The layout in words: "GPT", "ISO 9660", "ext4".
    pub fn layout_words(&self) -> String {
        match &self.layout {
            Layout::Table { scheme, .. } => (*scheme).to_owned(),
            Layout::Single(volume) => volume
                .file_system_words()
                .unwrap_or_else(|| volume.content.words().to_owned()),
        }
    }

    /// How many partitions, for a disk with a table.
    pub fn partitions(&self) -> Option<usize> {
        match &self.layout {
            Layout::Table { partitions, .. } => Some(partitions.len()),
            Layout::Single(_) => None,
        }
    }

    /// The opened line: "Opened disk.vhdx: 3 partitions, 120.0 GB".
    pub fn opened_line(&self) -> String {
        lines::opened(
            &self.name,
            &self.layout_words(),
            self.partitions(),
            self.claimed,
        )
    }

    /// The details' first row: "Disk image: GPT, 2 partitions, 8.0 GB".
    pub fn details_line(&self) -> String {
        lines::details_first(&self.layout_words(), self.partitions(), self.claimed)
    }

    /// The sparse guard's line, when the container claims more than the
    /// file holds.
    pub fn sparse_line(&self) -> Option<String> {
        (self.container.is_sparse() && self.claimed > self.held)
            .then(|| lines::sparse(self.claimed, self.held))
    }
}

/// The layout of a disk.
fn layout_of(
    content: Content,
    disk: Arc<dyn Backing>,
    limits: &Limits,
) -> Result<Layout, DiskError> {
    match content {
        Content::Gpt | Content::Mbr => {
            let table = table::read(disk.as_ref(), content == Content::Gpt)?;
            let mut partitions = Vec::with_capacity(table.entries.len());
            for entry in table.entries {
                let backing: Arc<dyn Backing> =
                    Arc::new(Slice::new(Arc::clone(&disk), entry.start, entry.len));
                partitions.push(Partition {
                    number: entry.number,
                    name: entry.name,
                    kind: entry.kind,
                    start: entry.start,
                    volume: Volume::new(backing, *limits)?,
                });
            }
            Ok(Layout::Table {
                scheme: table.scheme,
                partitions,
            })
        }
        Content::Unknown => Err(DiskError::Needs {
            cause: cause::NOT_AN_IMAGE.to_owned(),
            hint: String::new(),
        }),
        content => Ok(Layout::Single(Box::new(Volume::with_content(
            disk, content, *limits,
        )))),
    }
}
