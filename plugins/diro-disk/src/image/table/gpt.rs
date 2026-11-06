//! The GUID partition table: a header at sector 1 (512-byte sectors, or
//! 4,096 when the header is there instead), its entries where the header
//! says, both checked by their CRC-32, and the backup header at the
//! disk's last sector when the first one does not check out. Every count
//! and size the header claims is bounded before anything is read: at most
//! 1 MiB of entries, entries of 128 to 4,096 bytes, at most
//! [`MAX_PARTITIONS`] in use.

use super::{Entry, MAX_PARTITIONS, Table, crc32, le32, le64, within};
use crate::error::DiskError;
use crate::image::source::{Backing, read_vec};

/// The most bytes of entries a header may claim.
pub const MAX_ENTRY_BYTES: u64 = 1024 * 1024;

/// A type GUID as text, as tools print it: `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`.
pub fn guid_text(bytes: &[u8; 16]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]),
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

/// A type GUID in words, for the common ones.
pub fn type_words(guid: &str) -> Option<&'static str> {
    Some(match guid {
        "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" => "EFI System",
        "21686148-6449-6E6F-744E-656564454649" => "BIOS boot",
        "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7" => "Microsoft basic data",
        "E3C9E316-0B5C-4DB8-817D-F92DF00215AE" => "Microsoft reserved",
        "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC" => "Windows recovery",
        "0FC63DAF-8483-4772-8E79-3D69D8477DE4" => "Linux filesystem",
        "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709" => "Linux root",
        "933AC7E1-2EB4-4F13-B844-0E14E2AEF915" => "Linux home",
        "BC13C2FF-59E6-4262-A352-B275FD6F7172" => "Linux boot",
        "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F" => "Linux swap",
        "E6D6D379-F507-44C2-A23C-238F2A3DF928" => "Linux LVM",
        "A19D880F-05FC-4D3B-A006-743F0F84911E" => "Linux RAID",
        "CA7D7CCB-63ED-4C53-861C-1742536059CC" => "LUKS",
        "7C3457EF-0000-11AA-AA11-00306543ECAC" => "Apple APFS",
        "48465300-0000-11AA-AA11-00306543ECAC" => "Apple HFS+",
        "516E7CB4-6ECF-11D6-8FF8-00022D09712B" => "FreeBSD",
        _ => return None,
    })
}

/// The header's fields this reader uses, checked.
struct Header {
    entries_lba: u64,
    count: u64,
    size: u64,
    entries_crc: u32,
    backup_lba: u64,
}

fn header(sector: &[u8]) -> Result<Header, &'static str> {
    if sector.get(..8) != Some(b"EFI PART") {
        return Err("the header's signature is missing");
    }
    let header_size = le32(sector, 12) as usize;
    if !(92..=sector.len()).contains(&header_size) {
        return Err("the header's size is wrong");
    }
    let mut copy = sector[..header_size].to_vec();
    copy[16..20].fill(0);
    if crc32(&copy) != le32(sector, 16) {
        return Err("the header's checksum is wrong");
    }
    let count = u64::from(le32(sector, 80));
    let size = u64::from(le32(sector, 84));
    if !(128..=4096).contains(&size) || !size.is_multiple_of(8) {
        return Err("the entry size is wrong");
    }
    if count.saturating_mul(size) > MAX_ENTRY_BYTES {
        return Err("it claims too many entries");
    }
    Ok(Header {
        entries_lba: le64(sector, 72),
        count,
        size,
        entries_crc: le32(sector, 88),
        backup_lba: le64(sector, 32),
    })
}

/// A partition's name, UTF-16LE up to its first NUL.
fn name_of(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    let name = String::from_utf16_lossy(&units);
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn entries(disk: &dyn Backing, sector: u64, h: &Header) -> Result<Vec<Entry>, &'static str> {
    let bytes = h.count * h.size;
    let array = read_vec(
        disk,
        h.entries_lba.saturating_mul(sector),
        usize::try_from(bytes).unwrap_or(0),
    )
    .map_err(|_| "the entries could not be read")?;
    if array.len() as u64 != bytes {
        return Err("the entries run past the disk");
    }
    if crc32(&array) != h.entries_crc {
        return Err("the entries' checksum is wrong");
    }
    let mut out = Vec::new();
    for (index, raw) in array.chunks_exact(h.size as usize).enumerate() {
        let guid: [u8; 16] = raw[..16].try_into().unwrap_or([0; 16]);
        if guid == [0; 16] {
            continue;
        }
        if out.len() >= MAX_PARTITIONS {
            break;
        }
        let first = le64(raw, 32);
        let last = le64(raw, 40);
        if last < first {
            continue;
        }
        let Some((start, len)) = within(
            disk.len(),
            first.saturating_mul(sector),
            (last - first + 1).saturating_mul(sector),
        ) else {
            continue;
        };
        let text = guid_text(&guid);
        out.push(Entry {
            number: u32::try_from(index + 1).unwrap_or(u32::MAX),
            start,
            len,
            kind: Some(type_words(&text).map_or(text.clone(), str::to_owned)),
            name: name_of(&raw[56..128]),
        });
    }
    Ok(out)
}

/// Reads the GPT, from its backup when the first header fails.
pub fn read(disk: &dyn Backing) -> Result<Table, DiskError> {
    let head = read_vec(disk, 0, 8192).map_err(|e| DiskError::from_io(&e))?;
    let sector: u64 = if head.get(512..520) == Some(b"EFI PART") {
        512
    } else {
        4096
    };
    let at = |lba: u64| read_vec(disk, lba.saturating_mul(sector), sector as usize);
    let primary = at(1).map_err(|e| DiskError::from_io(&e))?;
    let first = header(&primary).and_then(|h| entries(disk, sector, &h).map(|e| (h, e)));
    let why = match first {
        Ok((_, entries)) => {
            return Ok(Table {
                scheme: "GPT",
                entries,
            });
        }
        Err(why) => why,
    };
    let last = disk.len() / sector;
    let backup_lba = header(&primary)
        .ok()
        .map(|h| h.backup_lba)
        .filter(|lba| *lba > 1 && *lba < last)
        .unwrap_or(last.saturating_sub(1));
    let backup = at(backup_lba).map_err(|e| DiskError::from_io(&e))?;
    let entries = header(&backup)
        .and_then(|h| entries(disk, sector, &h))
        .map_err(|_| DiskError::damaged("GPT", why))?;
    Ok(Table {
        scheme: "GPT",
        entries,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::image::source::Memory;

    const EFI: [u8; 16] = [
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ];
    const LINUX: [u8; 16] = [
        0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d,
        0xe4,
    ];

    /// A disk with two partitions written by `gptman`, the cross-check.
    fn disk_by_gptman(sector: u64, sectors: u64) -> Vec<u8> {
        let mut cur = Cursor::new(vec![0u8; (sector * sectors) as usize]);
        let mut gpt = gptman::GPT::new_from(&mut cur, sector, [0xab; 16]).unwrap();
        gpt[1] = gptman::GPTPartitionEntry {
            partition_type_guid: EFI,
            unique_partition_guid: [1; 16],
            starting_lba: gpt.header.first_usable_lba,
            ending_lba: gpt.header.first_usable_lba + 99,
            attribute_bits: 0,
            partition_name: "".into(),
        };
        gpt[3] = gptman::GPTPartitionEntry {
            partition_type_guid: LINUX,
            unique_partition_guid: [2; 16],
            starting_lba: gpt.header.first_usable_lba + 100,
            ending_lba: gpt.header.first_usable_lba + 299,
            attribute_bits: 0,
            partition_name: "home".into(),
        };
        gptman::GPT::write_protective_mbr_into(&mut cur, sector).unwrap();
        gpt.write_into(&mut cur).unwrap();
        cur.into_inner()
    }

    #[test]
    fn a_gpt_reads_as_gptman_wrote_it_at_both_sector_sizes() {
        for sector in [512u64, 4096] {
            let bytes = disk_by_gptman(sector, 1024);
            let table = read(&Memory::new(bytes.clone())).unwrap();
            let theirs = gptman::GPT::find_from(&mut Cursor::new(bytes)).unwrap();
            assert_eq!(table.scheme, "GPT");
            assert_eq!(table.entries.len(), 2);
            for entry in &table.entries {
                let p = &theirs[entry.number];
                assert_eq!(entry.start, p.starting_lba * sector);
                assert_eq!(entry.len, (p.ending_lba - p.starting_lba + 1) * sector);
            }
            assert_eq!(table.entries[0].number, 1);
            assert_eq!(table.entries[0].kind.as_deref(), Some("EFI System"));
            assert_eq!(table.entries[0].name, None);
            assert_eq!(table.entries[1].number, 3);
            assert_eq!(table.entries[1].kind.as_deref(), Some("Linux filesystem"));
            assert_eq!(table.entries[1].name.as_deref(), Some("home"));
        }
    }

    #[test]
    fn a_damaged_first_header_reads_from_the_backup() {
        let mut bytes = disk_by_gptman(512, 1024);
        bytes[512 + 30] ^= 0xff;
        let table = read(&Memory::new(bytes.clone())).unwrap();
        assert_eq!(table.entries.len(), 2);
        // Both gone: the words say why the first one failed.
        let last = bytes.len() - 512;
        bytes[last + 30] ^= 0xff;
        let error = read(&Memory::new(bytes)).unwrap_err();
        assert_eq!(
            error,
            DiskError::damaged("GPT", "the header's checksum is wrong")
        );
    }

    #[test]
    fn a_header_claiming_a_huge_entry_array_is_refused_before_reading() {
        let mut sector = vec![0u8; 512];
        sector[..8].copy_from_slice(b"EFI PART");
        sector[12..16].copy_from_slice(&92u32.to_le_bytes());
        sector[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        sector[84..88].copy_from_slice(&128u32.to_le_bytes());
        let crc = crc32(&sector[..92]);
        sector[16..20].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(header(&sector).err(), Some("it claims too many entries"));
        assert_eq!(guid_text(&EFI), "C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
        assert_eq!(guid_text(&LINUX), "0FC63DAF-8483-4772-8E79-3D69D8477DE4");
    }
}
