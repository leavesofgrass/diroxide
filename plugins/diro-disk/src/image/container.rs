//! The virtual-disk containers (R30 commit 93): each decoded into the disk
//! it holds, as a positional backing, with the sparse guard's checks done
//! before any crate is given the file.
//!
//! - **VHD:** fixed is the disk and a 512-byte footer, read here (the `vhd`
//!   crate reads a fixed one whole at open, 66 MiB in G5a's spike);
//!   dynamic through `vhd`, its block table bounded first; differencing
//!   needs its parent and says so.
//! - **VHDX** through `vhdx-core`, its region table read here first so its
//!   block table and metadata are bounded.
//! - **VDI** read here, fixed and dynamic: a header, a block map bounded by
//!   the index limit (8 MiB for 2 TB in G5a's spike), and the blocks. (The
//!   `vdi` crate reads dynamic images only and holds a reader that may not
//!   cross threads.)
//! - **VMDK:** a sparse extent through `vmdk-core` (which caps its grain
//!   directory at 16 MiB); a descriptor whose extents are flat files
//!   beside it, each opened through the host by its plain name.
//! - **QCOW2** through `qcow2-core` (which caps its L1 table at 8 MiB),
//!   after its header is read here: a backing file or encryption is said
//!   in words.
//!
//! The listing never allocates by the claimed size: a sparse disk claiming
//! 2 TB reads its tables (bounded) and the blocks a listing touches, and
//! an unallocated block reads as zeros without being made.

use std::sync::Arc;

use diroxide_vfs::archive::limits::Limits;

use crate::error::DiskError;
use crate::image::sniff::Container;
use crate::image::source::{Backing, ReaderBacking, Slice, Source, read_vec};
use crate::image::table::{le32, le64};
use crate::lines::cause;

/// Opens a file beside the image by its plain name (a VMDK's extents).
pub type Siblings<'a> = &'a dyn Fn(&str) -> Result<Arc<dyn Backing>, DiskError>;

fn be32(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be64(bytes: &[u8], at: usize) -> u64 {
    bytes.get(at..at + 8).map_or(0, |b| {
        u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    })
}

fn io(error: &std::io::Error) -> DiskError {
    DiskError::from_io(error)
}

fn bounded(what: &str, claimed: u64, limits: &Limits) -> Result<(), DiskError> {
    if claimed > limits.max_index_bytes {
        return Err(DiskError::Refused(cause::claims_too_much(
            what,
            claimed,
            limits.max_index_bytes,
        )));
    }
    Ok(())
}

/// The disk inside a container.
pub fn disk_of(
    container: Container,
    file: Arc<dyn Backing>,
    limits: &Limits,
    siblings: Siblings<'_>,
) -> Result<Arc<dyn Backing>, DiskError> {
    crate::error::guarded(container.words(), || {
        open_container(container, file, limits, siblings)
    })
}

fn open_container(
    container: Container,
    file: Arc<dyn Backing>,
    limits: &Limits,
    siblings: Siblings<'_>,
) -> Result<Arc<dyn Backing>, DiskError> {
    match container {
        Container::Raw => Ok(file),
        Container::VhdFixed => vhd_fixed(file),
        Container::VhdDynamic => vhd_dynamic(file, limits),
        Container::VhdDifferencing => Err(DiskError::uses(
            "a differencing VHD, which needs its parent image",
        )),
        Container::Vhdx => vhdx(file, limits),
        Container::Vdi => vdi(file, limits),
        Container::VmdkSparse => vmdk_sparse(file),
        Container::VmdkDescriptor => vmdk_descriptor(&file, siblings),
        Container::Qcow2 => qcow2(file, limits),
    }
}

/// A fixed VHD: the disk, then the footer, whose current size is kept
/// when it is no larger than the file holds.
fn vhd_fixed(file: Arc<dyn Backing>) -> Result<Arc<dyn Backing>, DiskError> {
    let held = file.len().saturating_sub(512);
    let footer = read_vec(file.as_ref(), held, 512).map_err(|e| io(&e))?;
    let size = be64(&footer, 48);
    let len = if size > 0 && size <= held { size } else { held };
    Ok(Arc::new(Slice::new(file, 0, len)))
}

fn vhd_dynamic(file: Arc<dyn Backing>, limits: &Limits) -> Result<Arc<dyn Backing>, DiskError> {
    let footer = read_vec(file.as_ref(), 0, 512).map_err(|e| io(&e))?;
    let header_at = be64(&footer, 16);
    let header = read_vec(file.as_ref(), header_at, 1024).map_err(|e| io(&e))?;
    if header.get(..8) != Some(b"cxsparse") {
        return Err(DiskError::damaged("VHD", "its dynamic header is missing"));
    }
    let entries = u64::from(be32(&header, 28));
    bounded("block table", entries.saturating_mul(4), limits)?;
    let reader = vhd::VhdReader::open_reader(Box::new(Source::new(file)))
        .map_err(|e| DiskError::damaged("VHD", e.to_string().to_lowercase()))?;
    let len = reader.virtual_disk_size();
    Ok(Arc::new(ReaderBacking::new(reader, len)))
}

/// The VHDX region table's entries, from either copy: each a GUID's
/// first bytes, an offset and a length.
fn vhdx_regions(file: &dyn Backing) -> Result<Vec<(u64, u64)>, DiskError> {
    for at in [192 * 1024u64, 256 * 1024] {
        let table = read_vec(file, at, 64 * 1024).map_err(|e| io(&e))?;
        if table.get(..4) != Some(b"regi") {
            continue;
        }
        let count = le32(&table, 8) as usize;
        if count > 2047 {
            continue;
        }
        let regions = (0..count)
            .map(|i| {
                let entry = 16 + i * 32;
                (
                    le64(&table, entry + 16),
                    u64::from(le32(&table, entry + 24)),
                )
            })
            .collect();
        return Ok(regions);
    }
    Err(DiskError::damaged("VHDX", "its region table is missing"))
}

fn vhdx(file: Arc<dyn Backing>, limits: &Limits) -> Result<Arc<dyn Backing>, DiskError> {
    for (_, length) in vhdx_regions(file.as_ref())? {
        bounded("region", length, limits)?;
    }
    let reader =
        vhdx::VhdxReader::open_reader(Box::new(Source::new(file))).map_err(|e| match e {
            vhdx::VhdxError::DifferencingNotSupported => {
                DiskError::uses("a differencing VHDX, which needs its parent image")
            }
            other => DiskError::damaged("VHDX", other.to_string().to_lowercase()),
        })?;
    let len = reader.virtual_disk_size();
    Ok(Arc::new(ReaderBacking::new(reader, len)))
}

/// A VDI disk: fixed (the data at its offset) or dynamic (a block map).
struct Vdi {
    file: Arc<dyn Backing>,
    data: u64,
    block: u64,
    extra: u64,
    map: Vec<u32>,
    len: u64,
}

impl Backing for Vdi {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let wanted = buffer.len().min(room);
        let mut done = 0;
        while done < wanted {
            let at = offset + done as u64;
            let index = at / self.block;
            let within = at % self.block;
            let n = usize::try_from((self.block - within).min((wanted - done) as u64)).unwrap_or(0);
            let out = &mut buffer[done..done + n];
            match self.map.get(usize::try_from(index).unwrap_or(usize::MAX)) {
                Some(&slot) if slot < 0xffff_fffe => {
                    let start = self.data
                        + u64::from(slot) * (self.block + self.extra)
                        + self.extra
                        + within;
                    crate::image::source::read_exact_at(self.file.as_ref(), start, out)?;
                }
                // Not allocated, or allocated as zeros.
                _ => out.fill(0),
            }
            done += n;
        }
        Ok(done)
    }
}

fn vdi(file: Arc<dyn Backing>, limits: &Limits) -> Result<Arc<dyn Backing>, DiskError> {
    let header = read_vec(file.as_ref(), 0, 512).map_err(|e| io(&e))?;
    let kind = le32(&header, 76);
    let blocks_at = u64::from(le32(&header, 340));
    let data = u64::from(le32(&header, 344));
    let len = le64(&header, 368);
    let block = u64::from(le32(&header, 376));
    let extra = u64::from(le32(&header, 380));
    let blocks = u64::from(le32(&header, 384));
    if kind == 2 {
        return Ok(Arc::new(Slice::new(file, data, len)));
    }
    if kind != 1 {
        return Err(DiskError::uses(&format!("a VDI of type {kind}")));
    }
    if block == 0 || !block.is_power_of_two() || len.div_ceil(block) > blocks {
        return Err(DiskError::damaged(
            "VDI",
            "its block size and count disagree",
        ));
    }
    bounded("block map", blocks.saturating_mul(4), limits)?;
    let raw = read_vec(
        file.as_ref(),
        blocks_at,
        usize::try_from(blocks * 4).unwrap_or(0),
    )
    .map_err(|e| io(&e))?;
    if raw.len() as u64 != blocks * 4 {
        return Err(DiskError::damaged(
            "VDI",
            "its block map runs past the file",
        ));
    }
    let map = raw
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Ok(Arc::new(Vdi {
        file,
        data,
        block,
        extra,
        map,
        len,
    }))
}

fn vmdk_sparse(file: Arc<dyn Backing>) -> Result<Arc<dyn Backing>, DiskError> {
    let reader = vmdk::VmdkReader::open(Source::new(file))
        .map_err(|e| DiskError::damaged("VMDK", e.to_string().to_lowercase()))?;
    let len = reader.virtual_disk_size();
    Ok(Arc::new(ReaderBacking::new(reader, len)))
}

/// One extent line of a VMDK descriptor: `RW 2048 FLAT "disk-flat.vmdk" 0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtentLine {
    /// Its length in 512-byte sectors.
    pub sectors: u64,
    /// Its kind: `FLAT`, `ZERO`, `SPARSE` and the rest.
    pub kind: String,
    /// The file it names, when it names one.
    pub file: Option<String>,
    /// Where in that file it starts, in sectors.
    pub offset: u64,
}

/// The extent lines of a VMDK descriptor's text.
pub fn extent_lines(text: &str) -> Vec<ExtentLine> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = ["RW ", "RDONLY ", "NOACCESS "]
            .iter()
            .find_map(|access| line.strip_prefix(access))
        else {
            continue;
        };
        let mut words = rest.splitn(3, ' ');
        let (Some(sectors), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(sectors) = sectors.parse::<u64>() else {
            continue;
        };
        let tail = words.next().unwrap_or("").trim();
        let (file, offset) = match tail.strip_prefix('"').and_then(|t| t.split_once('"')) {
            Some((name, after)) => (
                Some(name.to_owned()),
                after.trim().parse::<u64>().unwrap_or(0),
            ),
            None => (None, 0),
        };
        out.push(ExtentLine {
            sectors,
            kind: kind.to_owned(),
            file,
            offset,
        });
    }
    out
}

/// Disks made of extents in order, each a backing of its length.
struct Joined {
    parts: Vec<(u64, Arc<dyn Backing>)>,
    len: u64,
}

impl Backing for Joined {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        // Each part's length is the descriptor's claim, so the sum of the
        // parts can pass the end of a u64 (the nightly fuzz run found the
        // addition overflowing on a descriptor of two huge extents). The
        // offset is measured into each part instead of the parts summed.
        let mut within = offset;
        for (len, part) in &self.parts {
            if within < *len {
                let room = usize::try_from(len - within).unwrap_or(usize::MAX);
                let n = buffer.len().min(room);
                return part.read_at(within, &mut buffer[..n]);
            }
            within -= len;
        }
        Ok(0)
    }
}

/// A plain sibling file name: letters, digits, `-`, `_`, `.` and spaces,
/// no separator, no dot name.
fn plain_sibling(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
}

fn vmdk_descriptor(
    file: &Arc<dyn Backing>,
    siblings: Siblings<'_>,
) -> Result<Arc<dyn Backing>, DiskError> {
    let text = read_vec(file.as_ref(), 0, 64 * 1024).map_err(|e| io(&e))?;
    let text = String::from_utf8_lossy(&text);
    let lines = extent_lines(&text);
    if lines.is_empty() {
        return Err(DiskError::damaged("VMDK", "its descriptor names no extent"));
    }
    let mut parts: Vec<(u64, Arc<dyn Backing>)> = Vec::new();
    let mut len = 0u64;
    for line in lines {
        let bytes = line.sectors.saturating_mul(512);
        let part: Arc<dyn Backing> = match (line.kind.as_str(), line.file.as_deref()) {
            ("ZERO", _) => Arc::new(Zeros(bytes)),
            ("FLAT" | "VMFS", Some(name)) if plain_sibling(name) => {
                let backing = siblings(name)?;
                Arc::new(Slice::new(backing, line.offset.saturating_mul(512), bytes))
            }
            ("FLAT" | "VMFS", Some(_)) => {
                return Err(DiskError::Refused(
                    "its descriptor names an extent outside its folder".to_owned(),
                ));
            }
            (kind, _) => {
                return Err(DiskError::needs_7zip(&format!(
                    "a VMDK of {} extents",
                    kind.to_lowercase()
                )));
            }
        };
        len = len.saturating_add(bytes);
        parts.push((bytes, part));
    }
    Ok(Arc::new(Joined { parts, len }))
}

/// A run of zeros.
struct Zeros(u64);

impl Backing for Zeros {
    fn len(&self) -> u64 {
        self.0
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        let room = usize::try_from(self.0.saturating_sub(offset)).unwrap_or(usize::MAX);
        let n = buffer.len().min(room);
        buffer[..n].fill(0);
        Ok(n)
    }
}

fn qcow2(file: Arc<dyn Backing>, limits: &Limits) -> Result<Arc<dyn Backing>, DiskError> {
    let header = read_vec(file.as_ref(), 0, 104).map_err(|e| io(&e))?;
    if be64(&header, 8) != 0 {
        return Err(DiskError::uses("a QCOW2 with a backing file"));
    }
    if be32(&header, 32) != 0 {
        return Err(DiskError::uses("an encrypted QCOW2"));
    }
    let cluster_bits = be32(&header, 20);
    if !(9..=21).contains(&cluster_bits) {
        return Err(DiskError::damaged(
            "QCOW2",
            "its cluster size is out of range",
        ));
    }
    bounded(
        "L1 table",
        u64::from(be32(&header, 36)).saturating_mul(8),
        limits,
    )?;
    let reader = qcow2::Qcow2Reader::from_reader(Source::new(file))
        .map_err(|e| DiskError::damaged("QCOW2", e.to_string().to_lowercase()))?;
    let len = reader.virtual_disk_size();
    Ok(Arc::new(ReaderBacking::new(reader, len)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::source::Memory;

    #[test]
    fn descriptor_extent_lines_read_their_parts() {
        let text = "# Disk DescriptorFile\nversion=1\nRW 2048 FLAT \"disk-flat.vmdk\" 0\nRW 10 ZERO\nRDONLY 4 SPARSE \"s.vmdk\"\n";
        let lines = extent_lines(text);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].file.as_deref(), Some("disk-flat.vmdk"));
        assert_eq!(lines[1].kind, "ZERO");
        assert_eq!(lines[2].sectors, 4);
        assert!(plain_sibling("disk-flat.vmdk"));
        assert!(!plain_sibling("../etc/passwd"));
        assert!(!plain_sibling("C:\\x"));
    }

    #[test]
    fn a_vdi_block_map_past_the_limit_is_refused_before_it_is_read() {
        let mut header = vec![0u8; 512];
        header[64..68].copy_from_slice(&[0x7f, 0x10, 0xda, 0xbe]);
        header[76..80].copy_from_slice(&1u32.to_le_bytes());
        header[368..376].copy_from_slice(&(2u64 << 40).to_le_bytes());
        header[376..380].copy_from_slice(&4096u32.to_le_bytes());
        header[384..388].copy_from_slice(&u32::MAX.to_le_bytes());
        let error = vdi(Arc::new(Memory::new(header)), &Limits::default())
            .err()
            .unwrap();
        assert!(matches!(error, DiskError::Refused(_)), "{error:?}");
    }
}
