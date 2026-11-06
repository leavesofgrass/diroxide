//! The master boot record: four slots at byte 446 of the first sector,
//! and, behind an extended partition, a chain of extended boot records,
//! each naming one logical partition and the next record. The chain is
//! followed with a visited set and at most [`MAX_PARTITIONS`] links, so a
//! record that points back to itself ends the chain instead of the
//! plugin.

use std::collections::HashSet;

use super::{Entry, MAX_PARTITIONS, Table, le32, within};
use crate::error::DiskError;
use crate::image::source::{Backing, read_vec};

/// The sector size an MBR counts in.
pub const SECTOR: u64 = 512;

/// The table's type byte in words, for the common ones.
pub fn type_words(kind: u8) -> Option<&'static str> {
    Some(match kind {
        0x01 => "FAT12",
        0x04 | 0x06 | 0x0e => "FAT16",
        0x0b | 0x0c => "FAT32",
        0x07 => "NTFS or exFAT",
        0x27 => "Windows recovery",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x8e => "Linux LVM",
        0xa5 => "FreeBSD",
        0xa6 => "OpenBSD",
        0xa9 => "NetBSD",
        0xaf => "HFS+",
        0xee => "GPT protective",
        0xef => "EFI System",
        0xfd => "Linux RAID",
        _ => return None,
    })
}

fn is_extended(kind: u8) -> bool {
    matches!(kind, 0x05 | 0x0f | 0x85)
}

/// One 16-byte slot: its type, its first sector and its sector count.
fn slot(sector: &[u8], index: usize) -> (u8, u64, u64) {
    let at = 446 + index * 16;
    let kind = sector.get(at + 4).copied().unwrap_or(0);
    (
        kind,
        u64::from(le32(sector, at + 8)),
        u64::from(le32(sector, at + 12)),
    )
}

fn sector_at(disk: &dyn Backing, lba: u64) -> Result<Vec<u8>, DiskError> {
    let bytes = read_vec(disk, lba.saturating_mul(SECTOR), SECTOR as usize)
        .map_err(|e| DiskError::from_io(&e))?;
    if bytes.len() < SECTOR as usize || bytes[510..512] != [0x55, 0xaa] {
        return Err(DiskError::damaged("MBR", "a boot record has no end mark"));
    }
    Ok(bytes)
}

fn entry(disk: u64, number: u32, kind: u8, first: u64, count: u64) -> Option<Entry> {
    let (start, len) = within(
        disk,
        first.saturating_mul(SECTOR),
        count.saturating_mul(SECTOR),
    )?;
    Some(Entry {
        number,
        start,
        len,
        kind: Some(type_words(kind).map_or_else(|| format!("type {kind:02X}"), str::to_owned)),
        name: None,
    })
}

/// Reads an MBR and its logical partitions.
pub fn read(disk: &dyn Backing) -> Result<Table, DiskError> {
    let size = disk.len();
    let first = sector_at(disk, 0)?;
    let mut entries = Vec::new();
    let mut extended = None;
    for index in 0..4 {
        let (kind, lba, count) = slot(&first, index);
        if kind == 0 || count == 0 {
            continue;
        }
        if is_extended(kind) {
            extended.get_or_insert((lba, count));
            continue;
        }
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        entries.extend(entry(size, number, kind, lba, count));
    }
    if let Some((base, _)) = extended {
        let mut seen = HashSet::new();
        let mut next = base;
        let mut number = 5u32;
        while seen.insert(next) && seen.len() <= MAX_PARTITIONS {
            let Ok(record) = sector_at(disk, next) else {
                break;
            };
            let (kind, lba, count) = slot(&record, 0);
            if kind != 0 && count != 0 && !is_extended(kind) {
                entries.extend(entry(size, number, kind, next.saturating_add(lba), count));
                number += 1;
            }
            let (link, link_lba, _) = slot(&record, 1);
            if !is_extended(link) || link_lba == 0 {
                break;
            }
            next = base.saturating_add(link_lba);
        }
    }
    Ok(Table {
        scheme: "MBR",
        entries,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::image::source::Memory;

    /// A disk of `sectors` with primaries and logicals written by
    /// `mbrman`, the cross-check.
    fn disk_by_mbrman(sectors: u32) -> Vec<u8> {
        let mut cur = Cursor::new(vec![0u8; sectors as usize * 512]);
        let mut mbr = mbrman::MBR::new_from(&mut cur, 512, [0x12, 0x34, 0x56, 0x78]).unwrap();
        mbr[1] = mbrman::MBRPartitionEntry {
            boot: mbrman::BOOT_ACTIVE,
            first_chs: mbrman::CHS::empty(),
            sys: 0x0c,
            last_chs: mbrman::CHS::empty(),
            starting_lba: 2048,
            sectors: 4096,
        };
        mbr[2] = mbrman::MBRPartitionEntry {
            boot: mbrman::BOOT_INACTIVE,
            first_chs: mbrman::CHS::empty(),
            sys: 0x0f,
            last_chs: mbrman::CHS::empty(),
            starting_lba: 8192,
            sectors: sectors - 8192,
        };
        mbr.align = 1;
        mbr.push(0x83, 8192 + 2048, 2048).unwrap();
        mbr.push(0x82, 8192 + 6144, 1024).unwrap();
        mbr.write_into(&mut cur).unwrap();
        cur.into_inner()
    }

    #[test]
    fn primaries_and_logicals_read_as_mbrman_wrote_them() {
        let bytes = disk_by_mbrman(32_768);
        let disk = Memory::new(bytes.clone());
        let table = read(&disk).unwrap();
        let theirs = mbrman::MBR::read_from(&mut Cursor::new(bytes), 512).unwrap();
        // mbrman's own reading numbers the logical partitions from 5 too.
        let mut numbers = Vec::new();
        for (i, p) in theirs.iter() {
            if p.is_used() && !is_extended(p.sys) {
                numbers.push((i as u32, u64::from(p.starting_lba) * 512));
            }
        }
        let ours: Vec<(u32, u64)> = table.entries.iter().map(|e| (e.number, e.start)).collect();
        assert_eq!(ours, numbers);
        assert_eq!(table.entries[0].kind.as_deref(), Some("FAT32"));
        assert_eq!(table.entries[0].len, 4096 * 512);
        assert_eq!(table.entries[1].kind.as_deref(), Some("Linux"));
        assert_eq!(table.entries[2].kind.as_deref(), Some("Linux swap"));
        assert_eq!(table.entries.len(), 3);
    }

    #[test]
    fn a_chain_that_points_back_ends_and_a_partition_past_the_end_is_cut() {
        let mut bytes = vec![0u8; 64 * 512];
        bytes[510] = 0x55;
        bytes[511] = 0xaa;
        // An extended partition at sector 4 whose record links to itself.
        bytes[446 + 4] = 0x05;
        bytes[446 + 8..446 + 12].copy_from_slice(&4u32.to_le_bytes());
        bytes[446 + 12..446 + 16].copy_from_slice(&60u32.to_le_bytes());
        // A primary that runs far past the disk.
        bytes[462 + 4] = 0x83;
        bytes[462 + 8..462 + 12].copy_from_slice(&40u32.to_le_bytes());
        bytes[462 + 12..462 + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        let ebr = 4 * 512;
        bytes[ebr + 510] = 0x55;
        bytes[ebr + 511] = 0xaa;
        bytes[ebr + 446 + 4] = 0x83;
        bytes[ebr + 446 + 8..ebr + 446 + 12].copy_from_slice(&1u32.to_le_bytes());
        bytes[ebr + 446 + 12..ebr + 446 + 16].copy_from_slice(&2u32.to_le_bytes());
        bytes[ebr + 462 + 4] = 0x05;
        // Links back to the extended partition's own first record.
        bytes[ebr + 462 + 8..ebr + 462 + 12].copy_from_slice(&0u32.to_le_bytes());
        let table = read(&Memory::new(bytes)).unwrap();
        assert_eq!(table.entries.len(), 2);
        assert_eq!(table.entries[0].number, 2);
        assert_eq!(table.entries[0].len, 24 * 512);
        assert_eq!(table.entries[1].number, 5);
    }

    #[test]
    fn a_first_sector_with_no_end_mark_is_damaged() {
        let error = read(&Memory::new(vec![0u8; 1024])).unwrap_err();
        assert!(matches!(error, DiskError::Damaged(_)), "{error:?}");
        assert_eq!(type_words(0x99), None);
    }
}
