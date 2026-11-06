//! ISO 9660 with Joliet and Rock Ridge, through `hadris-iso` (R30 commit
//! 88, G5a's finding: it read every path of a hard image, its 4.5 GiB
//! multi-extent file included, and refused the bomb in 0.3 ms).
//!
//! - **One namespace** is read: Rock Ridge when the image has it (long
//!   names, links, modes), else Joliet (Unicode names), else the plain
//!   names with their `;1` versions and trailing dots removed.
//! - **The guards come first** (the record-claim guard): the primary
//!   descriptor's block size must be 2,048, its path table and root
//!   folder must fit the image and the header limit, and a folder whose
//!   record claims more than the index limit or more than the image holds
//!   is refused before a byte of it is read. A file is read a range at a
//!   time from its extents, never allocated by its claimed size.
//! - **A relocated folder** (Rock Ridge `CL`) is listed where it belongs;
//!   its placeholder (`RE`) is left out; a folder whose extent is one it
//!   is inside is a loop, which the tree cuts.

use std::sync::Arc;

use diroxide_vfs::archive::limits::Limits;
use hadris_iso::directory::DirectoryRef;
use hadris_iso::file::EntryType;
use hadris_iso::io::LogicalSector;
use hadris_iso::read::{DirEntry, IsoImage};

use crate::error::DiskError;
use crate::image::fs::{Facts, FileSystem, Node, NodeKind, civil};
use crate::image::source::{Backing, Source, read_exact_at, read_vec};
use crate::image::table::{le16, le32};
use crate::lines::cause;

/// The ISO sector.
const SECTOR: u64 = 2048;

/// Which names the image is read by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Names {
    RockRidge,
    Joliet,
    Plain,
}

/// An ISO 9660 image.
pub struct Iso {
    image: IsoImage<Source>,
    backing: Arc<dyn Backing>,
    root: DirectoryRef,
    names: Names,
    label: Option<String>,
    limits: Limits,
}

fn from_hadris(error: &hadris_io::Error<hadris_io::ErrorKind>) -> DiskError {
    match error.kind() {
        hadris_io::ErrorKind::Interrupted => DiskError::Cancelled,
        hadris_io::ErrorKind::Unsupported => DiskError::uses(&error.to_string().to_lowercase()),
        _ => DiskError::damaged("ISO 9660", error.to_string().to_lowercase()),
    }
}

/// The checks on the primary descriptor before the reader is given the
/// image: the block size, and the path table and root folder within the
/// image and the limits. Gives the volume's label.
fn guard(backing: &dyn Backing, limits: &Limits) -> Result<Option<String>, DiskError> {
    let pvd =
        read_vec(backing, 16 * SECTOR, SECTOR as usize).map_err(|e| DiskError::from_io(&e))?;
    if pvd.len() < SECTOR as usize || pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return Err(DiskError::damaged(
            "ISO 9660",
            "its first volume descriptor is not the primary one",
        ));
    }
    let block = le16(&pvd, 128);
    if u64::from(block) != SECTOR {
        return Err(DiskError::needs_7zip(&format!(
            "an ISO with {block}-byte blocks"
        )));
    }
    let len = backing.len();
    let path_table = u64::from(le32(&pvd, 132));
    if path_table > limits.max_header_bytes || path_table > len {
        return Err(DiskError::Refused(cause::claims_too_much(
            "path table",
            path_table,
            limits.max_header_bytes.min(len),
        )));
    }
    let root_extent = u64::from(le32(&pvd, 158));
    let root_size = u64::from(le32(&pvd, 166));
    check_folder(root_extent, root_size, len, limits)?;
    let label = String::from_utf8_lossy(&pvd[40..72]).trim().to_owned();
    Ok((!label.is_empty()).then_some(label))
}

/// A folder's record must fit the image and the index limit.
fn check_folder(extent: u64, size: u64, len: u64, limits: &Limits) -> Result<(), DiskError> {
    let end = extent.saturating_mul(SECTOR).saturating_add(size);
    if size > limits.max_index_bytes {
        return Err(DiskError::Refused(cause::claims_too_much(
            "folder record",
            size,
            limits.max_index_bytes,
        )));
    }
    if end > len {
        return Err(DiskError::damaged(
            "ISO 9660",
            "a folder's record runs past the image",
        ));
    }
    Ok(())
}

/// Opens an ISO 9660 image.
pub fn open(backing: Arc<dyn Backing>, limits: &Limits) -> Result<Box<dyn FileSystem>, DiskError> {
    let label = guard(backing.as_ref(), limits)?;
    let image = IsoImage::open(Source::new(Arc::clone(&backing))).map_err(|e| from_hadris(&e))?;
    let mut best: Option<(u8, DirectoryRef, Names)> = None;
    for root in image.root_dirs() {
        let (rank, names) = match root.entry_type() {
            EntryType::Level1 {
                supports_rrip: true,
                ..
            }
            | EntryType::Level2 {
                supports_rrip: true,
                ..
            }
            | EntryType::Level3 {
                supports_rrip: true,
                ..
            } => (3, Names::RockRidge),
            EntryType::Joliet { .. } => (2, Names::Joliet),
            EntryType::Level3 { .. } => (1, Names::Plain),
            _ => (0, Names::Plain),
        };
        if best.as_ref().is_none_or(|(r, _, _)| rank > *r) {
            best = Some((rank, root.dir_ref(), names));
        }
    }
    let (_, root, names) =
        best.ok_or_else(|| DiskError::damaged("ISO 9660", "it has no root folder"))?;
    check_folder(
        root.extent.0 as u64,
        root.size as u64,
        backing.len(),
        limits,
    )?;
    Ok(Box::new(Iso {
        image,
        backing,
        root,
        names,
        label,
        limits: *limits,
    }))
}

/// A plain ISO name without its version (`;1`) or a trailing dot.
fn plain_name(raw: &[u8]) -> Vec<u8> {
    let mut name = raw.to_vec();
    if let Some(at) = name.iter().rposition(|b| *b == b';')
        && name[at + 1..].iter().all(u8::is_ascii_digit)
    {
        name.truncate(at);
    }
    if name.ends_with(b".") && name.len() > 1 {
        name.pop();
    }
    name
}

/// A record's date: years since 1900, month, day, hour, minute, second,
/// and the zone in 15-minute steps.
fn record_time(header: &[u8]) -> Option<diroxide_core::Timestamp> {
    let d = header.get(18..25)?;
    let time = civil(1900 + i32::from(d[0]), d[1], d[2], d[3], d[4], d[5])?;
    let zone = i64::from(d[6] as i8) * 15 * 60;
    Some(time.plus_seconds(-zone))
}

impl Iso {
    fn node(&self, entry: &DirEntry) -> Result<Option<Node>, DiskError> {
        let raw = entry.name();
        if raw == [0] || raw == [1] {
            return Ok(None);
        }
        let rrip = entry.rrip.as_ref();
        let name = match self.names {
            Names::RockRidge => match rrip.and_then(|r| r.alternate_name.as_ref()) {
                Some(name) => name.as_bytes().to_vec(),
                None => plain_name(raw),
            },
            Names::Joliet => entry.record.joliet_name().into_bytes(),
            Names::Plain => plain_name(raw),
        };
        let mode = rrip
            .and_then(|r| r.posix_attributes.as_ref())
            .map(|px| px.file_mode.read());
        let kind = if entry.is_directory() {
            NodeKind::Folder
        } else if rrip.is_some_and(|r| r.symlink_target.is_some()) {
            NodeKind::Link
        } else if mode.is_some_and(|m| !matches!(m & 0o170_000, 0 | 0o100_000 | 0o040_000)) {
            NodeKind::Special
        } else {
            NodeKind::File
        };
        let mut node = Node::new(name, kind);
        node.mode = mode.map(|m| m & 0o7777);
        node.modified = rrip
            .and_then(|r| r.timestamps.as_ref())
            .and_then(|t| t.modify.as_ref())
            .and_then(|t| {
                civil(
                    i32::from(t.year),
                    t.month,
                    t.day,
                    t.hour,
                    t.minute,
                    t.second,
                )
            })
            .or_else(|| record_time(entry.header().to_bytes()));
        if entry.header().flags & 0x01 != 0 {
            node.attributes.push("hidden");
        }
        match kind {
            NodeKind::Folder => {
                let dir = entry.as_dir_ref(&self.image).map_err(|e| from_hadris(&e))?;
                node.id = Some(dir.extent.0 as u64);
                node.token = Some((dir.extent.0 as u64, dir.size as u64));
            }
            NodeKind::Link => {
                node.link = rrip
                    .and_then(|r| r.symlink_target.clone())
                    .map(String::into_bytes);
            }
            NodeKind::File => {
                node.size = Some(entry.total_size());
                node.extents = entry
                    .extents()
                    .map(|e| (e.sector.0 as u64 * SECTOR, u64::from(e.length)))
                    .collect();
            }
            NodeKind::Special => {}
        }
        Ok(Some(node))
    }
}

impl FileSystem for Iso {
    fn facts(&self) -> Facts {
        let words = match self.names {
            Names::RockRidge => "ISO 9660, Rock Ridge",
            Names::Joliet => "ISO 9660, Joliet",
            Names::Plain => "ISO 9660",
        };
        Facts {
            words: words.to_owned(),
            label: self.label.clone(),
            case_sensitive: self.names == Names::RockRidge,
            free: Some(0),
        }
    }

    fn root_id(&self) -> Option<u64> {
        Some(self.root.extent.0 as u64)
    }

    fn list(&mut self, _path: &[Vec<u8>], folder: &Node) -> Result<Vec<Node>, DiskError> {
        let dir = match folder.token {
            Some((extent, size)) => {
                check_folder(extent, size, self.backing.len(), &self.limits)?;
                DirectoryRef {
                    extent: LogicalSector(usize::try_from(extent).unwrap_or(usize::MAX)),
                    size: usize::try_from(size).unwrap_or(usize::MAX),
                }
            }
            None => self.root,
        };
        let mut nodes = Vec::new();
        for entry in self.image.open_dir(dir).entries() {
            let entry = entry.map_err(|e| from_hadris(&e))?;
            if let Some(node) = self.node(&entry)? {
                nodes.push(node);
                if nodes.len() > self.limits.max_entries {
                    break;
                }
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
        read_extents(self.backing.as_ref(), &node.extents, offset, length)
    }
}

/// Up to `length` bytes from `offset` of a file stored in extents, each a
/// byte start and length in the volume, in order.
pub fn read_extents(
    backing: &dyn Backing,
    extents: &[(u64, u64)],
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, DiskError> {
    let total: u64 = extents.iter().map(|(_, len)| *len).sum();
    let end = offset.saturating_add(length as u64).min(total);
    if offset >= end {
        return Ok(Vec::new());
    }
    let mut out = vec![0u8; usize::try_from(end - offset).unwrap_or(0)];
    let mut at = 0u64;
    for (start, len) in extents {
        let (first, last) = (at, at + len);
        at = last;
        if last <= offset || first >= end {
            continue;
        }
        let from = offset.max(first);
        let to = end.min(last);
        let within = from - first;
        let into = usize::try_from(from - offset).unwrap_or(0);
        let n = usize::try_from(to - from).unwrap_or(0);
        if start.saturating_add(within).saturating_add(n as u64) > backing.len() {
            return Err(DiskError::damaged("file", "its data runs past the image"));
        }
        read_exact_at(backing, start + within, &mut out[into..into + n])
            .map_err(|e| DiskError::from_io(&e))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::source::Memory;

    #[test]
    fn plain_names_lose_their_version_and_trailing_dot() {
        assert_eq!(plain_name(b"README.TXT;1"), b"README.TXT");
        assert_eq!(plain_name(b"NOEXT.;1"), b"NOEXT");
        assert_eq!(plain_name(b"A;B"), b"A;B");
        assert_eq!(plain_name(b"."), b".");
    }

    #[test]
    fn extents_read_in_order_and_never_past_the_image() {
        let bytes: Vec<u8> = (0..100u8).collect();
        let memory = Memory::new(bytes);
        let extents = [(10, 5), (50, 5)];
        assert_eq!(
            read_extents(&memory, &extents, 3, 4).unwrap(),
            [13, 14, 50, 51]
        );
        assert_eq!(read_extents(&memory, &extents, 9, 10).unwrap(), [54]);
        assert!(read_extents(&memory, &extents, 10, 1).unwrap().is_empty());
        let past = [(98, 10)];
        assert!(read_extents(&memory, &past, 0, 10).is_err());
    }

    #[test]
    fn a_descriptor_with_other_blocks_or_huge_claims_is_refused() {
        let mut bytes = vec![0u8; 40 * 2048];
        let pvd = 16 * 2048;
        bytes[pvd] = 1;
        bytes[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        bytes[pvd + 128..pvd + 130].copy_from_slice(&512u16.to_le_bytes());
        let memory = Memory::new(bytes.clone());
        assert!(matches!(
            guard(&memory, &Limits::default()),
            Err(DiskError::Needs { .. })
        ));
        bytes[pvd + 128..pvd + 130].copy_from_slice(&2048u16.to_le_bytes());
        bytes[pvd + 132..pvd + 136].copy_from_slice(&u32::MAX.to_le_bytes());
        let memory = Memory::new(bytes.clone());
        assert!(matches!(
            guard(&memory, &Limits::default()),
            Err(DiskError::Refused(_))
        ));
        bytes[pvd + 132..pvd + 136].copy_from_slice(&10u32.to_le_bytes());
        bytes[pvd + 158..pvd + 162].copy_from_slice(&20u32.to_le_bytes());
        bytes[pvd + 166..pvd + 170].copy_from_slice(&u32::MAX.to_le_bytes());
        let memory = Memory::new(bytes);
        assert!(matches!(
            guard(&memory, &Limits::default()),
            Err(DiskError::Refused(_))
        ));
    }
}
