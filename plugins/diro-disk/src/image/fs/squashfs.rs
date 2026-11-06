//! squashfs through `backhand` with its pure-Rust codecs only, gzip and
//! LZ4 (R30 commit 92, G5a's finding): an image that uses xz, zstd, LZO
//! or LZMA, which most distribution images do, goes to 7-Zip, said in
//! words, so no C codec enters the plugin.
//!
//! - **The superblock is checked first:** version 4, a codec read here,
//!   the inode count and the tables inside the image and the limits, so
//!   the crate is never handed an image that claims more than it holds.
//! - **The crate reads the whole tree at open** (its design: one pass
//!   over the inode and folder tables, with its own visited set against
//!   cycles), on a thread with a large stack so a deep chain of folders
//!   cannot overflow a worker's; the plugin then lists a folder from an
//!   index of children, and a file is read by seeking, which skips whole
//!   blocks without decompressing them.

use std::collections::HashMap;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use backhand::{FilesystemReader, InnerNode};
use diroxide_vfs::archive::limits::Limits;

use crate::error::DiskError;
use crate::image::fs::{Facts, FileSystem, Node, NodeKind};
use crate::image::source::{Backing, Source, read_vec};
use crate::image::table::{le16, le32, le64};
use crate::lines::cause;

/// The stack the crate's open runs on: its walk of the folders recurses.
const OPEN_STACK: usize = 64 * 1024 * 1024;

/// A squashfs volume.
pub struct Squashfs {
    fs: FilesystemReader<'static>,
    children: HashMap<PathBuf, Vec<usize>>,
    codec: &'static str,
}

fn from_backhand(error: &backhand::BackhandError) -> DiskError {
    DiskError::damaged("squashfs", error.to_string().to_lowercase())
}

/// The codec's words, and whether the plugin reads it.
fn codec(id: u16) -> (&'static str, bool) {
    match id {
        1 => ("gzip", true),
        2 => ("LZMA", false),
        3 => ("LZO", false),
        4 => ("xz", false),
        5 => ("LZ4", true),
        6 => ("zstd", false),
        _ => ("an unknown codec", false),
    }
}

/// The superblock's checks, before the crate is given the image.
fn guard(backing: &dyn Backing, limits: &Limits) -> Result<&'static str, DiskError> {
    let sb = read_vec(backing, 0, 96).map_err(|e| DiskError::from_io(&e))?;
    if sb.len() < 96 || &sb[..4] != b"hsqs" {
        return Err(DiskError::damaged("squashfs", "its superblock is missing"));
    }
    let major = le16(&sb, 28);
    if major != 4 {
        return Err(DiskError::needs_7zip(&format!("squashfs version {major}")));
    }
    let (words, ours) = codec(le16(&sb, 20));
    if !ours {
        return Err(DiskError::needs_7zip(&format!("squashfs with {words}")));
    }
    let len = backing.len();
    let inodes = u64::from(le32(&sb, 4));
    if inodes > limits.max_entries as u64 * 10 {
        return Err(DiskError::Refused(cause::claims_too_much(
            "inode table",
            inodes,
            limits.max_entries as u64 * 10,
        )));
    }
    let used = le64(&sb, 40);
    let inode_table = le64(&sb, 64);
    let dir_table = le64(&sb, 72);
    if used > len || inode_table > dir_table || dir_table > used {
        return Err(DiskError::damaged(
            "squashfs",
            "its tables run past the image",
        ));
    }
    let tables = used - inode_table;
    if tables > limits.max_index_bytes {
        return Err(DiskError::Refused(cause::claims_too_much(
            "folder tables",
            tables,
            limits.max_index_bytes,
        )));
    }
    Ok(words)
}

/// Opens a squashfs volume.
pub fn open(backing: Arc<dyn Backing>, limits: &Limits) -> Result<Box<dyn FileSystem>, DiskError> {
    let codec = guard(backing.as_ref(), limits)?;
    let source = BufReader::new(Source::new(backing));
    let cancel = crate::host::serving();
    let fs = std::thread::Builder::new()
        .name("disk-squashfs-open".to_owned())
        .stack_size(OPEN_STACK)
        .spawn(move || {
            let _serving = cancel.as_ref().map(crate::host::Serving::start);
            FilesystemReader::from_reader(source).map_err(|e| from_backhand(&e))
        })
        .map_err(|e| DiskError::Io(e.to_string()))?
        .join()
        .map_err(|_| DiskError::damaged("squashfs", "its reader stopped"))??;
    let mut children: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (index, node) in fs.files().enumerate() {
        if let Some(parent) = node.fullpath.parent() {
            children
                .entry(parent.to_path_buf())
                .or_default()
                .push(index);
        }
    }
    Ok(Box::new(Squashfs {
        fs,
        children,
        codec,
    }))
}

/// A stored path as the crate's full path: `/a/b`.
fn full(path: &[Vec<u8>]) -> PathBuf {
    let mut out = PathBuf::from("/");
    for name in path {
        out.push(String::from_utf8_lossy(name).as_ref());
    }
    out
}

fn name_of(path: &Path) -> Vec<u8> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned().into_bytes())
        .unwrap_or_default()
}

impl FileSystem for Squashfs {
    fn facts(&self) -> Facts {
        Facts {
            words: format!("squashfs, {}", self.codec),
            label: None,
            case_sensitive: true,
            free: Some(0),
        }
    }

    fn root_id(&self) -> Option<u64> {
        Some(0)
    }

    fn list(&mut self, path: &[Vec<u8>], _folder: &Node) -> Result<Vec<Node>, DiskError> {
        let dir = full(path);
        let nodes: Vec<_> = self.fs.files().collect();
        let Some(indexes) = self.children.get(&dir) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(indexes.len());
        for &index in indexes {
            let Some(entry) = nodes.get(index) else {
                continue;
            };
            if entry.fullpath == dir {
                continue;
            }
            let (kind, size, link) = match &entry.inner {
                InnerNode::Dir(_) => (NodeKind::Folder, None, None),
                InnerNode::File(file) => (NodeKind::File, Some(file.file_len() as u64), None),
                InnerNode::Symlink(link) => (
                    NodeKind::Link,
                    None,
                    Some(link.link.to_string_lossy().into_owned().into_bytes()),
                ),
                _ => (NodeKind::Special, None, None),
            };
            let mut node = Node::new(name_of(&entry.fullpath), kind);
            node.size = size;
            node.link = link;
            node.mode = Some(u32::from(entry.header.permissions) & 0o7777);
            node.modified = Some(diroxide_core::Timestamp::from_unix(
                i64::from(entry.header.mtime),
                0,
            ));
            node.token = Some((index as u64, 0));
            if kind == NodeKind::Folder {
                node.id = Some(index as u64);
            }
            out.push(node);
        }
        Ok(out)
    }

    fn read(
        &mut self,
        _path: &[Vec<u8>],
        node: &Node,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError> {
        let index = node
            .token
            .map(|(i, _)| i as usize)
            .ok_or(DiskError::NotFound)?;
        let entry = self.fs.files().nth(index).ok_or(DiskError::NotFound)?;
        let InnerNode::File(file) = &entry.inner else {
            return Err(DiskError::IsAFolder);
        };
        let size = file.file_len() as u64;
        if offset >= size {
            return Ok(Vec::new());
        }
        let mut reader = self.fs.file(file).reader();
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(|e| DiskError::from_io(&e))?;
        let want = usize::try_from((size - offset).min(length as u64)).unwrap_or(0);
        let mut out = vec![0u8; want];
        let mut done = 0;
        while done < want {
            match reader
                .read(&mut out[done..])
                .map_err(|e| DiskError::from_io(&e))?
            {
                0 => break,
                n => done += n,
            }
        }
        out.truncate(done);
        Ok(out)
    }
}
