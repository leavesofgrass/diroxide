//! ext2, ext3 and ext4 through `ext4-view` (R30 commit 90), read only by
//! the crate's own design, over positional reads.
//!
//! - **Its own thread.** The crate's volume counts its references without
//!   atomics, so it lives on a thread of its own ([`Pinned`]) and is
//!   loaded once (the superblock, the group descriptors, the journal).
//! - **Refusals in words, never a panic:** a feature the crate does not
//!   read ("incompatible filesystem: ...") or an encrypted folder says so;
//!   a damaged volume says what is damaged.
//! - **The words** come from the superblock, read here: ext4 when it has
//!   extents, 64-bit numbers or flexible groups, ext3 when it has a
//!   journal, else ext2; its label; and its free space.

use std::sync::Arc;

use diroxide_vfs::archive::limits::Limits;
use ext4_view::{Ext4, Ext4Error, Ext4Read};

use crate::error::{DiskError, HostIo};
use crate::image::fs::pinned::Pinned;
use crate::image::fs::{Facts, FileSystem, Node, NodeKind};
use crate::image::source::{Backing, read_exact_at, read_vec};
use crate::image::table::le32;

/// An ext volume.
pub struct Ext {
    fs: Pinned<Ext4>,
    facts: Facts,
    limits: Limits,
}

/// The volume's bytes for the crate: exactly the bytes asked, or an
/// error.
struct Reader(Arc<dyn Backing>);

impl Ext4Read for Reader {
    fn read(
        &mut self,
        start_byte: u64,
        dst: &mut [u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        read_exact_at(self.0.as_ref(), start_byte, dst).map_err(|e| Box::new(e) as _)
    }
}

fn from_ext4(error: &Ext4Error) -> DiskError {
    match error {
        Ext4Error::NotFound => DiskError::NotFound,
        Ext4Error::IsADirectory => DiskError::IsAFolder,
        Ext4Error::NotADirectory => DiskError::NotAFolder,
        Ext4Error::Encrypted => DiskError::uses("an encrypted folder"),
        Ext4Error::Incompatible(why) => DiskError::uses(&format!("ext features {why}")),
        Ext4Error::Io(inner) => match inner.downcast_ref::<std::io::Error>() {
            Some(io) => match io.get_ref().and_then(|e| e.downcast_ref::<HostIo>()) {
                Some(HostIo(disk)) => disk.clone(),
                None => DiskError::from_io(io),
            },
            None => DiskError::Io(inner.to_string()),
        },
        other => DiskError::damaged("ext", other.to_string()),
    }
}

/// The superblock's words, label and free space.
fn facts_of(backing: &dyn Backing) -> Facts {
    let sb = read_vec(backing, 1024, 1024).unwrap_or_default();
    let compat = le32(&sb, 0x5c);
    let incompat = le32(&sb, 0x60);
    let words = if incompat & (0x40 | 0x80 | 0x200) != 0 {
        "ext4"
    } else if compat & 0x4 != 0 {
        "ext3"
    } else {
        "ext2"
    };
    let label = sb
        .get(0x78..0x88)
        .map(|l| {
            String::from_utf8_lossy(l)
                .trim_end_matches('\0')
                .trim()
                .to_owned()
        })
        .filter(|l| !l.is_empty());
    let block = 1024u64.checked_shl(le32(&sb, 0x18)).unwrap_or(0);
    let mut free_blocks = u64::from(le32(&sb, 0x0c));
    if incompat & 0x80 != 0 {
        free_blocks |= u64::from(le32(&sb, 0x158)) << 32;
    }
    Facts {
        words: words.to_owned(),
        label,
        case_sensitive: true,
        free: Some(free_blocks.saturating_mul(block)),
    }
}

/// Opens an ext volume.
pub fn open(backing: Arc<dyn Backing>, limits: &Limits) -> Result<Box<dyn FileSystem>, DiskError> {
    let facts = facts_of(backing.as_ref());
    let reader = Reader(Arc::clone(&backing));
    let fs = Pinned::spawn("disk-ext4", move || {
        Ext4::load(Box::new(reader)).map_err(|e| from_ext4(&e))
    })?;
    Ok(Box::new(Ext {
        fs,
        facts,
        limits: *limits,
    }))
}

/// A stored path as the crate's absolute path bytes: `/a/b`.
fn absolute(path: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for name in path {
        out.push(b'/');
        out.extend_from_slice(name);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

impl FileSystem for Ext {
    fn facts(&self) -> Facts {
        self.facts.clone()
    }

    fn list(&mut self, path: &[Vec<u8>], _folder: &Node) -> Result<Vec<Node>, DiskError> {
        let dir = absolute(path);
        let most = self.limits.max_entries;
        self.fs.run(move |fs| -> Result<Vec<Node>, DiskError> {
            let mut nodes = Vec::new();
            let entries = fs.read_dir(dir.as_slice()).map_err(|e| from_ext4(&e))?;
            for entry in entries {
                let entry = entry.map_err(|e| from_ext4(&e))?;
                let name = entry.file_name().as_ref().to_vec();
                if name == b"." || name == b".." {
                    continue;
                }
                let file_type = entry.file_type().map_err(|e| from_ext4(&e))?;
                let kind = if file_type.is_dir() {
                    NodeKind::Folder
                } else if file_type.is_symlink() {
                    NodeKind::Link
                } else if file_type.is_regular_file() {
                    NodeKind::File
                } else {
                    NodeKind::Special
                };
                let mut node = Node::new(name, kind);
                let metadata = entry.metadata().map_err(|e| from_ext4(&e))?;
                if kind == NodeKind::File {
                    node.size = Some(metadata.len());
                }
                node.mode = Some(u32::from(metadata.mode()) & 0o7777);
                let modified = metadata.modified();
                node.modified = Some(diroxide_core::Timestamp::from_unix(
                    modified.seconds(),
                    modified.nanoseconds(),
                ));
                if kind == NodeKind::Link {
                    let target = fs
                        .read_link(entry.path().as_ref())
                        .map_err(|e| from_ext4(&e))?;
                    node.link = Some(target.as_ref().to_vec());
                }
                nodes.push(node);
                if nodes.len() > most {
                    break;
                }
            }
            Ok(nodes)
        })?
    }

    fn read(
        &mut self,
        path: &[Vec<u8>],
        _node: &Node,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError> {
        let file = absolute(path);
        self.fs.run(move |fs| -> Result<Vec<u8>, DiskError> {
            let mut handle = fs.open(file.as_slice()).map_err(|e| from_ext4(&e))?;
            handle.seek_to(offset).map_err(|e| from_ext4(&e))?;
            let size = handle.metadata().len();
            let want = usize::try_from(size.saturating_sub(offset).min(length as u64)).unwrap_or(0);
            let mut out = vec![0u8; want];
            let mut done = 0;
            while done < want {
                match handle
                    .read_bytes(&mut out[done..])
                    .map_err(|e| from_ext4(&e))?
                {
                    0 => break,
                    n => done += n,
                }
            }
            out.truncate(done);
            Ok(out)
        })?
    }
}
