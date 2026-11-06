//! FAT12, FAT16 and FAT32 with long names, through `fatfs` behind a
//! read-only wrapper (R30 commit 89): the crate asks for a storage it may
//! write, and every write the wrapper is given is refused, so the image
//! is never changed, not even its dirty flag. exFAT is not FAT to this
//! crate and goes to 7-Zip.
//!
//! A folder is found by walking its path name by name from the root, the
//! long name as stored; a folder's listing stops one entry past the
//! limit, so a cluster chain that runs in a circle ends at the limit
//! instead of never. The crate's volume is mounted for each call (two
//! sectors, from the block cache after the first), because its options
//! hold references that may not cross threads, and the plugin serves
//! sixteen requests at once.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use diroxide_vfs::archive::limits::Limits;
use fatfs::{FatType, FileAttributes, FsOptions};

use crate::error::DiskError;
use crate::image::fs::{Facts, FileSystem, Node, NodeKind, civil};
use crate::image::source::{Backing, ReadOnly, Source};

type Fs = fatfs::FileSystem<ReadOnly>;
type Dir<'a> = fatfs::Dir<'a, ReadOnly>;

/// A FAT volume.
pub struct Fat {
    backing: Arc<dyn Backing>,
    fat_type: FatType,
    label: Option<String>,
    limits: Limits,
}

fn mount(backing: &Arc<dyn Backing>) -> Result<Fs, DiskError> {
    let storage = ReadOnly(Source::new(Arc::clone(backing)));
    fatfs::FileSystem::new(storage, FsOptions::new()).map_err(|e| damaged(&e))
}

fn damaged(error: &std::io::Error) -> DiskError {
    match DiskError::from_io(error) {
        DiskError::Io(why) => DiskError::damaged("FAT", why),
        other => other,
    }
}

/// Opens a FAT volume.
pub fn open(backing: Arc<dyn Backing>, limits: &Limits) -> Result<Box<dyn FileSystem>, DiskError> {
    let fs = mount(&backing)?;
    let label = fs.volume_label();
    let label = label.trim();
    let label = (!label.is_empty() && label != "NO NAME").then(|| label.to_owned());
    let fat_type = fs.fat_type();
    drop(fs);
    Ok(Box::new(Fat {
        backing,
        fat_type,
        label,
        limits: *limits,
    }))
}

fn time(dt: fatfs::DateTime) -> Option<diroxide_core::Timestamp> {
    civil(
        i32::from(dt.date.year),
        u8::try_from(dt.date.month).ok()?,
        u8::try_from(dt.date.day).ok()?,
        u8::try_from(dt.time.hour).ok()?,
        u8::try_from(dt.time.min).ok()?,
        u8::try_from(dt.time.sec).ok()?,
    )
}

/// The folder at a path of stored names, walked from the root.
fn folder<'a>(fs: &'a Fs, path: &[Vec<u8>]) -> Result<Dir<'a>, DiskError> {
    let mut dir = fs.root_dir();
    for name in path {
        let mut next = None;
        for entry in dir.iter() {
            let entry = entry.map_err(|e| damaged(&e))?;
            if entry.is_dir() && entry.file_name().as_bytes() == name.as_slice() {
                next = Some(entry.to_dir());
                break;
            }
        }
        dir = next.ok_or(DiskError::NotFound)?;
    }
    Ok(dir)
}

impl FileSystem for Fat {
    fn facts(&self) -> Facts {
        let words = match self.fat_type {
            FatType::Fat12 => "FAT12",
            FatType::Fat16 => "FAT16",
            FatType::Fat32 => "FAT32",
        };
        Facts {
            words: words.to_owned(),
            label: self.label.clone(),
            case_sensitive: false,
            free: None,
        }
    }

    fn list(&mut self, path: &[Vec<u8>], _folder: &Node) -> Result<Vec<Node>, DiskError> {
        let fs = mount(&self.backing)?;
        let dir = folder(&fs, path)?;
        let mut nodes = Vec::new();
        for entry in dir.iter() {
            let entry = entry.map_err(|e| damaged(&e))?;
            let attributes = entry.attributes();
            if attributes.contains(FileAttributes::VOLUME_ID) {
                continue;
            }
            let kind = if entry.is_dir() {
                NodeKind::Folder
            } else {
                NodeKind::File
            };
            let mut node = Node::new(entry.file_name().into_bytes(), kind);
            if kind == NodeKind::File {
                node.size = Some(entry.len());
            }
            node.modified = time(entry.modified());
            for (flag, word) in [
                (FileAttributes::READ_ONLY, "read_only"),
                (FileAttributes::HIDDEN, "hidden"),
                (FileAttributes::SYSTEM, "system"),
                (FileAttributes::ARCHIVE, "archive"),
            ] {
                if attributes.contains(flag) {
                    node.attributes.push(word);
                }
            }
            nodes.push(node);
            if nodes.len() > self.limits.max_entries {
                break;
            }
        }
        Ok(nodes)
    }

    fn read(
        &mut self,
        path: &[Vec<u8>],
        node: &Node,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError> {
        let (name, parent) = path.split_last().ok_or(DiskError::IsAFolder)?;
        let fs = mount(&self.backing)?;
        let dir = folder(&fs, parent)?;
        let mut file = None;
        for entry in dir.iter() {
            let entry = entry.map_err(|e| damaged(&e))?;
            if entry.is_file() && entry.file_name().as_bytes() == name.as_slice() {
                file = Some(entry.to_file());
                break;
            }
        }
        let mut file = file.ok_or(DiskError::NotFound)?;
        let size = node.size.unwrap_or(0);
        if offset >= size {
            return Ok(Vec::new());
        }
        let want = usize::try_from((size - offset).min(length as u64)).unwrap_or(0);
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| damaged(&e))?;
        let mut out = vec![0u8; want];
        let mut done = 0;
        while done < want {
            match file.read(&mut out[done..]).map_err(|e| damaged(&e))? {
                0 => break,
                n => done += n,
            }
        }
        out.truncate(done);
        Ok(out)
    }
}
