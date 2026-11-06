//! NTFS through `ntfs` 0.4 (R30 commit 91): read only, no unsafe, over
//! `Read + Seek`.
//!
//! - **One name a file.** A folder's index holds a file under its long
//!   name and, when Windows made one, its DOS 8.3 twin; the twins are left
//!   out (G5a's finding), so each file is one row.
//! - **The system's own files** (`$MFT`, `$Bitmap` and the rest, records
//!   below 16) are not rows.
//! - **Compressed and encrypted files** are rows with a note: "compressed
//!   by NTFS, read it with 7-Zip", and reading one says so, because the
//!   crate reads neither.
//! - **Alternate data streams are not rows** (a `:` in a row's name would
//!   be a reserved character on Windows); a file's details say "Has
//!   alternate streams: 2".
//! - A folder or a file is found again by its record number, kept with
//!   its row, so a read never walks the path.

use std::sync::Arc;

use diroxide_vfs::archive::limits::Limits;
use ntfs::structured_values::{NtfsFileAttributeFlags, NtfsFileNamespace};
use ntfs::{Ntfs, NtfsAttributeType, NtfsReadSeek};

use crate::error::DiskError;
use crate::image::fs::{Facts, FileSystem, Node, NodeKind};
use crate::image::source::{Backing, Source};
use crate::lines;

/// The first record number a user's file may have.
const FIRST_USER_RECORD: u64 = 16;

/// The root folder's record number.
const ROOT_RECORD: u64 = 5;

/// Seconds between 1601 and 1970.
const EPOCH_DIFFERENCE: i64 = 11_644_473_600;

/// An NTFS volume.
pub struct Ntfs4 {
    ntfs: Ntfs,
    source: Source,
    label: Option<String>,
    limits: Limits,
}

fn from_ntfs(error: &ntfs::NtfsError) -> DiskError {
    DiskError::damaged("NTFS", error.to_string().to_lowercase())
}

/// Opens an NTFS volume.
pub fn open(backing: Arc<dyn Backing>, limits: &Limits) -> Result<Box<dyn FileSystem>, DiskError> {
    let mut source = Source::new(backing);
    let ntfs = Ntfs::new(&mut source).map_err(|e| from_ntfs(&e))?;
    let label = ntfs
        .volume_name(&mut source)
        .and_then(Result::ok)
        .map(|name| name.name().to_string_lossy())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    Ok(Box::new(Ntfs4 {
        ntfs,
        source,
        label,
        limits: *limits,
    }))
}

fn time(nt: ntfs::NtfsTime) -> Option<diroxide_core::Timestamp> {
    let intervals = nt.nt_timestamp();
    let secs = i64::try_from(intervals / 10_000_000).ok()? - EPOCH_DIFFERENCE;
    let nanos = u32::try_from((intervals % 10_000_000) * 100).ok()?;
    Some(diroxide_core::Timestamp::from_unix(secs, nanos))
}

impl Ntfs4 {
    fn record_of(node: &Node) -> u64 {
        node.token.map_or(ROOT_RECORD, |(record, _)| record)
    }
}

impl FileSystem for Ntfs4 {
    fn facts(&self) -> Facts {
        Facts {
            words: "NTFS".to_owned(),
            label: self.label.clone(),
            case_sensitive: false,
            free: None,
        }
    }

    fn root_id(&self) -> Option<u64> {
        Some(ROOT_RECORD)
    }

    fn list(&mut self, _path: &[Vec<u8>], folder: &Node) -> Result<Vec<Node>, DiskError> {
        let most = self.limits.max_entries;
        let record = Self::record_of(folder);
        let ntfs = &self.ntfs;
        let source = &mut self.source;
        let dir = if record == ROOT_RECORD {
            ntfs.root_directory(source)
        } else {
            ntfs.file(source, record)
        }
        .map_err(|e| from_ntfs(&e))?;
        let index = dir.directory_index(source).map_err(|e| from_ntfs(&e))?;
        let mut entries = index.entries();
        let mut nodes = Vec::new();
        while let Some(entry) = entries.next(source) {
            let entry = entry.map_err(|e| from_ntfs(&e))?;
            let Some(key) = entry.key() else {
                continue;
            };
            let name = key.map_err(|e| from_ntfs(&e))?;
            if name.namespace() == NtfsFileNamespace::Dos {
                continue;
            }
            let number = entry.file_reference().file_record_number();
            if number < FIRST_USER_RECORD {
                continue;
            }
            let flags = name.file_attributes();
            let kind = if name.is_directory() {
                NodeKind::Folder
            } else if flags.contains(NtfsFileAttributeFlags::REPARSE_POINT) {
                NodeKind::Special
            } else {
                NodeKind::File
            };
            let mut node = Node::new(name.name().to_string_lossy().into_bytes(), kind);
            node.token = Some((number, 0));
            node.modified = time(name.modification_time());
            if kind == NodeKind::Folder {
                node.id = Some(number);
            } else {
                node.size = Some(name.data_size());
            }
            for (flag, word) in [
                (NtfsFileAttributeFlags::READ_ONLY, "read_only"),
                (NtfsFileAttributeFlags::HIDDEN, "hidden"),
                (NtfsFileAttributeFlags::SYSTEM, "system"),
                (NtfsFileAttributeFlags::ARCHIVE, "archive"),
                (NtfsFileAttributeFlags::REPARSE_POINT, "reparse_point"),
            ] {
                if flags.contains(flag) {
                    node.attributes.push(word);
                }
            }
            if kind == NodeKind::File {
                if flags.contains(NtfsFileAttributeFlags::COMPRESSED) {
                    node.note = Some(lines::NTFS_COMPRESSED.to_owned());
                } else if flags.contains(NtfsFileAttributeFlags::ENCRYPTED) {
                    node.note = Some(lines::NTFS_ENCRYPTED.to_owned());
                }
            }
            nodes.push(node);
            if nodes.len() > most {
                break;
            }
        }
        Ok(nodes)
    }

    fn read(
        &mut self,
        _path: &[Vec<u8>],
        node: &Node,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError> {
        if node.note.as_deref() == Some(lines::NTFS_COMPRESSED) {
            return Err(DiskError::Needs {
                cause: lines::cause::NTFS_COMPRESSED.to_owned(),
                hint: lines::cause::GET_7ZIP.to_owned(),
            });
        }
        if node.note.as_deref() == Some(lines::NTFS_ENCRYPTED) {
            return Err(DiskError::uses("NTFS encryption"));
        }
        let record = Self::record_of(node);
        let ntfs = &self.ntfs;
        let source = &mut self.source;
        let file = ntfs.file(source, record).map_err(|e| from_ntfs(&e))?;
        let Some(item) = file.data(source, "") else {
            return Ok(Vec::new());
        };
        let item = item.map_err(|e| from_ntfs(&e))?;
        let attribute = item.to_attribute().map_err(|e| from_ntfs(&e))?;
        let mut value = attribute.value(source).map_err(|e| from_ntfs(&e))?;
        let size = value.len();
        if offset >= size {
            return Ok(Vec::new());
        }
        value
            .seek(source, std::io::SeekFrom::Start(offset))
            .map_err(|e| from_ntfs(&e))?;
        let want = usize::try_from((size - offset).min(length as u64)).unwrap_or(0);
        let mut out = vec![0u8; want];
        let mut done = 0;
        while done < want {
            match value
                .read(source, &mut out[done..])
                .map_err(|e| from_ntfs(&e))?
            {
                0 => break,
                n => done += n,
            }
        }
        out.truncate(done);
        Ok(out)
    }

    fn detail(&mut self, _path: &[Vec<u8>], node: &Node) -> Result<Node, DiskError> {
        let mut node = node.clone();
        if node.kind != NodeKind::File {
            return Ok(node);
        }
        let record = Self::record_of(&node);
        let ntfs = &self.ntfs;
        let source = &mut self.source;
        let file = ntfs.file(source, record).map_err(|e| from_ntfs(&e))?;
        let mut streams = 0usize;
        let mut attributes = file.attributes();
        while let Some(item) = attributes.next(source) {
            let Ok(item) = item else {
                break;
            };
            let Ok(attribute) = item.to_attribute() else {
                continue;
            };
            if attribute.ty().ok() == Some(NtfsAttributeType::Data)
                && attribute.name().map(|n| !n.is_empty()).unwrap_or(false)
            {
                streams += 1;
            }
        }
        if streams > 0 {
            let note = lines::alternate_streams(streams);
            node.note = Some(match node.note {
                Some(before) => format!("{before}; {note}"),
                None => note,
            });
        }
        Ok(node)
    }
}
