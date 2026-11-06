//! The partition tables, read by the plugin itself (R29 1.3, R30 commit
//! 87): a master boot record with its logical partitions, and a GUID
//! partition table with its backup. Both are a few hundred bytes of
//! fixed fields, so a reader of about 250 lines, every field
//! bounds-checked, gives the exact words the rows need and avoids two
//! serde-heavy crates; `mbrman` and `gptman` write the tables the tests
//! cross-check against.
//!
//! A partition that starts past the disk's end is left out; one that runs
//! past it is cut at the end, so no read goes outside the image.

pub mod gpt;
pub mod mbr;

use crate::error::DiskError;
use crate::image::source::Backing;

/// One partition as a table describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its number: GPT's entry index from 1; MBR's slot from 1, logical
    /// partitions from 5.
    pub number: u32,
    /// Where it starts, in bytes.
    pub start: u64,
    /// Its length, in bytes.
    pub len: u64,
    /// The table's type in words: "EFI System", "Linux filesystem".
    pub kind: Option<String>,
    /// Its name in the table (GPT only).
    pub name: Option<String>,
}

/// A table read from a disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The scheme in words: "GPT" or "MBR".
    pub scheme: &'static str,
    /// The partitions in use, in table order.
    pub entries: Vec<Entry>,
}

/// The most partitions a table may name: GPT's usual 128, and the logical
/// partitions an MBR chain may hold.
pub const MAX_PARTITIONS: usize = 128;

/// Keeps a partition inside the disk: `None` when it starts past the end,
/// cut when it runs past it.
pub fn within(disk: u64, start: u64, len: u64) -> Option<(u64, u64)> {
    if len == 0 || start >= disk {
        return None;
    }
    Some((start, len.min(disk - start)))
}

/// Reads the table of a disk the sniffer called GPT or MBR.
pub fn read(disk: &dyn Backing, gpt: bool) -> Result<Table, DiskError> {
    if gpt {
        gpt::read(disk)
    } else {
        mbr::read(disk)
    }
}

/// CRC-32 (IEEE 802.3, the one GPT uses), bit by bit through a table made
/// once.
pub fn crc32(bytes: &[u8]) -> u32 {
    const fn table() -> [u32; 256] {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    }
    const TABLE: [u32; 256] = table();
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc = TABLE[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

/// A little-endian `u16` at `at`, or 0 past the end.
pub fn le16(bytes: &[u8], at: usize) -> u16 {
    bytes
        .get(at..at + 2)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

/// A little-endian `u32` at `at`, or 0 past the end.
pub fn le32(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// A little-endian `u64` at `at`, or 0 past the end.
pub fn le64(bytes: &[u8], at: usize) -> u64 {
    bytes.get(at..at + 8).map_or(0, |b| {
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_the_ieee_one() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn a_partition_stays_inside_the_disk() {
        assert_eq!(within(100, 10, 20), Some((10, 20)));
        assert_eq!(within(100, 90, 20), Some((90, 10)));
        assert_eq!(within(100, 100, 1), None);
        assert_eq!(within(100, 10, 0), None);
        assert_eq!(le32(&[1, 0], 0), 0);
    }
}
