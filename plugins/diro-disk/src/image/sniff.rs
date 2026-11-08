//! What an image is, by its bytes, never by its name (R29 1.2): first the
//! container a virtual-disk program wrapped it in, then what the disk
//! holds, a partition table or one file system. Each signature is checked
//! at its place in the format's own description:
//!
//! | What | Where | Bytes |
//! |---|---|---|
//! | QCOW2 | 0 | `QFI` 0xFB |
//! | VHDX | 0 | `vhdxfile` |
//! | VDI | 64 | 0x7F 0x10 0xDA 0xBE (0xBEDA107F, little-endian) |
//! | VMDK, sparse | 0 | `KDMV` |
//! | VMDK, descriptor | 0 | `# Disk DescriptorFile` |
//! | VHD | the last 512 bytes (and 0 for a dynamic one) | `conectix` |
//! | ISO 9660 | 32,769 (sector 16, byte 1) | `CD001` |
//! | UDF | 32,769 and the next sectors | `BEA01`, `NSR02`, `NSR03` |
//! | GPT | 512 (or 4,096 with 4 KiB sectors) | `EFI PART` |
//! | MBR | 510 | 0x55 0xAA, with sane entries and no boot record |
//! | NTFS | 3 | `NTFS    ` |
//! | exFAT | 3 | `EXFAT   ` |
//! | BitLocker | 3 | `-FVE-FS-` |
//! | FAT | 54 or 82 | `FAT12`, `FAT16`, `FAT32`, with a sane boot record |
//! | ext2, ext3, ext4 | 1,080 | 0x53 0xEF |
//! | squashfs | 0 | `hsqs` |
//! | LUKS | 0 | `LUKS` 0xBA 0xBE |
//! | HFS+ | 1,024 | `H+` or `HX` |
//! | APFS | 32 | `NXSB` |
//!
//! A hybrid ISO, which also carries a partition table so it boots from a
//! stick, opens as the ISO.

use crate::image::source::{Backing, read_vec};

/// The wrapper a virtual-disk program put around the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    /// No wrapper: the bytes are the disk.
    Raw,
    /// A VHD whose footer says fixed: the disk, then 512 bytes.
    VhdFixed,
    /// A dynamic VHD: a block table over the file.
    VhdDynamic,
    /// A differencing VHD, which needs its parent.
    VhdDifferencing,
    /// VHDX.
    Vhdx,
    /// VirtualBox's VDI.
    Vdi,
    /// A sparse VMDK extent (a monolithic sparse or stream-optimized
    /// file).
    VmdkSparse,
    /// A VMDK descriptor, which names its extents.
    VmdkDescriptor,
    /// QCOW2 (or QCOW1, which says its version).
    Qcow2,
}

impl Container {
    /// The format in words.
    pub fn words(self) -> &'static str {
        match self {
            Container::Raw => "raw disk image",
            Container::VhdFixed => "fixed VHD",
            Container::VhdDynamic => "dynamic VHD",
            Container::VhdDifferencing => "differencing VHD",
            Container::Vhdx => "VHDX",
            Container::Vdi => "VDI",
            Container::VmdkSparse => "sparse VMDK",
            Container::VmdkDescriptor => "VMDK descriptor",
            Container::Qcow2 => "QCOW2",
        }
    }

    /// Whether the container may claim more than its file holds.
    pub fn is_sparse(self) -> bool {
        !matches!(self, Container::Raw | Container::VhdFixed)
    }
}

/// What a disk, or a partition, holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// ISO 9660, with Joliet or Rock Ridge when it has them.
    Iso,
    /// UDF with no ISO 9660 beside it.
    Udf,
    /// A GUID partition table.
    Gpt,
    /// A master boot record's partition table.
    Mbr,
    /// FAT12, FAT16 or FAT32.
    Fat,
    /// exFAT.
    ExFat,
    /// NTFS.
    Ntfs,
    /// ext2, ext3 or ext4.
    Ext,
    /// squashfs.
    Squashfs,
    /// HFS+.
    HfsPlus,
    /// APFS.
    Apfs,
    /// A BitLocker volume.
    BitLocker,
    /// A LUKS volume.
    Luks,
    /// Nothing known.
    Unknown,
}

impl Content {
    /// The content in words, for rows and details.
    pub fn words(self) -> &'static str {
        match self {
            Content::Iso => "ISO 9660",
            Content::Udf => "UDF",
            Content::Gpt => "GPT",
            Content::Mbr => "MBR",
            Content::Fat => "FAT",
            Content::ExFat => "exFAT",
            Content::Ntfs => "NTFS",
            Content::Ext => "ext",
            Content::Squashfs => "squashfs",
            Content::HfsPlus => "HFS+",
            Content::Apfs => "APFS",
            Content::BitLocker => "BitLocker",
            Content::Luks => "LUKS",
            Content::Unknown => "unknown data",
        }
    }

    /// Locked by encryption: shown, never opened (R9).
    pub fn is_locked(self) -> bool {
        matches!(self, Content::BitLocker | Content::Luks)
    }
}

fn at(bytes: &[u8], offset: usize, want: &[u8]) -> bool {
    bytes.get(offset..offset + want.len()) == Some(want)
}

/// The container, from the image's first 4 KiB and its last 512 bytes.
pub fn container(head: &[u8], tail: &[u8]) -> Container {
    if at(head, 0, b"QFI\xfb") {
        return Container::Qcow2;
    }
    if at(head, 0, b"vhdxfile") {
        return Container::Vhdx;
    }
    if at(head, 64, &[0x7f, 0x10, 0xda, 0xbe]) {
        return Container::Vdi;
    }
    if at(head, 0, b"KDMV") {
        return Container::VmdkSparse;
    }
    if at(head, 0, b"# Disk DescriptorFile") {
        return Container::VmdkDescriptor;
    }
    // A tail shorter than the footer is never the footer (the nightly
    // fuzz run found the slice below zero on an eight-byte file).
    let footer = if tail.len() >= 512 && at(tail, tail.len() - 512, b"conectix") {
        Some(&tail[tail.len() - 512..])
    } else if at(head, 0, b"conectix") {
        Some(head)
    } else {
        None
    };
    if let Some(footer) = footer {
        return match footer.get(60..64) {
            Some([0, 0, 0, 2]) => Container::VhdFixed,
            Some([0, 0, 0, 4]) => Container::VhdDifferencing,
            _ => Container::VhdDynamic,
        };
    }
    Container::Raw
}

/// What the disk at `backing` holds, read from a few sectors at their
/// signatures' places.
pub fn content(backing: &dyn Backing) -> std::io::Result<Content> {
    // 36 KiB covers the boot sector, the GPT header at either sector size,
    // the ext superblock and the ISO's first descriptors.
    let head = read_vec(backing, 0, 36 * 1024)?;
    Ok(content_of(&head))
}

/// What a disk holds, from its first 36 KiB.
pub fn content_of(head: &[u8]) -> Content {
    if at(head, 0x8001, b"CD001") {
        return Content::Iso;
    }
    if [b"BEA01", b"NSR02", b"NSR03"]
        .iter()
        .any(|sig| (0..4).any(|i| at(head, 0x8001 + i * 2048, *sig)))
    {
        return Content::Udf;
    }
    if at(head, 512, b"EFI PART") || at(head, 4096, b"EFI PART") {
        return Content::Gpt;
    }
    if at(head, 3, b"-FVE-FS-") {
        return Content::BitLocker;
    }
    if at(head, 0, b"LUKS\xba\xbe") {
        return Content::Luks;
    }
    if at(head, 3, b"NTFS    ") {
        return Content::Ntfs;
    }
    if at(head, 3, b"EXFAT   ") {
        return Content::ExFat;
    }
    if at(head, 0, b"hsqs") {
        return Content::Squashfs;
    }
    if is_fat(head) {
        return Content::Fat;
    }
    if at(head, 1080, &[0x53, 0xef]) {
        return Content::Ext;
    }
    if at(head, 1024, b"H+") || at(head, 1024, b"HX") {
        return Content::HfsPlus;
    }
    if at(head, 32, b"NXSB") {
        return Content::Apfs;
    }
    if is_mbr(head) {
        return Content::Mbr;
    }
    Content::Unknown
}

/// A FAT boot sector: the jump, a power-of-two sector size from 512 to
/// 4,096, a power-of-two cluster, one or two FATs, and the type text at
/// either place.
fn is_fat(head: &[u8]) -> bool {
    let Some(sector) = head.get(..512) else {
        return false;
    };
    let jump = sector[0] == 0xeb || sector[0] == 0xe9;
    let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
    let per_cluster = sector[13];
    let fats = sector[16];
    let sane = jump
        && matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
        && per_cluster.is_power_of_two()
        && (1..=2).contains(&fats);
    let named = at(sector, 54, b"FAT12")
        || at(sector, 54, b"FAT16")
        || at(sector, 54, b"FAT  ")
        || at(sector, 82, b"FAT32");
    sane && (named || u16::from_le_bytes([sector[22], sector[23]]) != 0)
}

/// A master boot record with a table: the end mark, no boot record of a
/// file system, and four entries whose status bytes are 0 or 0x80, at
/// least one in use.
fn is_mbr(head: &[u8]) -> bool {
    let Some(sector) = head.get(..512) else {
        return false;
    };
    if sector[510..512] != [0x55, 0xaa] {
        return false;
    }
    let mut used = 0;
    for i in 0..4 {
        let entry = &sector[446 + i * 16..462 + i * 16];
        if entry[0] != 0 && entry[0] != 0x80 {
            return false;
        }
        if entry[4] != 0 {
            used += 1;
        }
    }
    used > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(len: usize, marks: &[(usize, &[u8])]) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        for (offset, mark) in marks {
            bytes[*offset..*offset + mark.len()].copy_from_slice(mark);
        }
        bytes
    }

    #[test]
    fn containers_are_told_by_their_signatures() {
        let tail = vec![0u8; 512];
        assert_eq!(container(b"QFI\xfb rest", &tail), Container::Qcow2);
        assert_eq!(container(b"vhdxfile....", &tail), Container::Vhdx);
        assert_eq!(
            container(&with(512, &[(64, &[0x7f, 0x10, 0xda, 0xbe])]), &tail),
            Container::Vdi
        );
        assert_eq!(container(b"KDMV", &tail), Container::VmdkSparse);
        assert_eq!(
            container(b"# Disk DescriptorFile\nversion=1", &tail),
            Container::VmdkDescriptor
        );
        let fixed = with(512, &[(0, b"conectix"), (60, &[0, 0, 0, 2])]);
        assert_eq!(container(&[0u8; 512], &fixed), Container::VhdFixed);
        let dynamic = with(512, &[(0, b"conectix"), (60, &[0, 0, 0, 3])]);
        assert_eq!(container(&dynamic, &dynamic), Container::VhdDynamic);
        let diff = with(512, &[(0, b"conectix"), (60, &[0, 0, 0, 4])]);
        assert_eq!(container(&[0u8; 512], &diff), Container::VhdDifferencing);
        assert_eq!(container(&[0u8; 512], &tail), Container::Raw);
        assert!(Container::Qcow2.is_sparse());
        assert!(!Container::VhdFixed.is_sparse());
    }

    #[test]
    fn contents_are_told_by_their_signatures() {
        let len = 36 * 1024;
        assert_eq!(content_of(&with(len, &[(0x8001, b"CD001")])), Content::Iso);
        assert_eq!(content_of(&with(len, &[(0x8801, b"NSR02")])), Content::Udf);
        assert_eq!(content_of(&with(len, &[(512, b"EFI PART")])), Content::Gpt);
        assert_eq!(content_of(&with(len, &[(4096, b"EFI PART")])), Content::Gpt);
        assert_eq!(content_of(&with(len, &[(3, b"NTFS    ")])), Content::Ntfs);
        assert_eq!(content_of(&with(len, &[(3, b"EXFAT   ")])), Content::ExFat);
        assert_eq!(
            content_of(&with(len, &[(3, b"-FVE-FS-")])),
            Content::BitLocker
        );
        assert_eq!(
            content_of(&with(len, &[(0, b"LUKS\xba\xbe")])),
            Content::Luks
        );
        assert_eq!(content_of(&with(len, &[(0, b"hsqs")])), Content::Squashfs);
        assert_eq!(
            content_of(&with(len, &[(1080, &[0x53, 0xef])])),
            Content::Ext
        );
        assert_eq!(content_of(&with(len, &[(1024, b"H+")])), Content::HfsPlus);
        assert_eq!(content_of(&with(len, &[(32, b"NXSB")])), Content::Apfs);
        let fat = with(
            len,
            &[
                (0, &[0xeb, 0x3c, 0x90]),
                (11, &[0, 2, 4]),
                (16, &[2]),
                (54, b"FAT16   "),
                (510, &[0x55, 0xaa]),
            ],
        );
        assert_eq!(content_of(&fat), Content::Fat);
        let mbr = with(len, &[(446, &[0x80, 0, 0, 0, 0x83]), (510, &[0x55, 0xaa])]);
        assert_eq!(content_of(&mbr), Content::Mbr);
        let not_mbr = with(len, &[(446, &[0x12, 0, 0, 0, 0x83]), (510, &[0x55, 0xaa])]);
        assert_eq!(content_of(&not_mbr), Content::Unknown);
        assert_eq!(content_of(&[]), Content::Unknown);
        // A hybrid ISO, with a table too, opens as the ISO.
        let hybrid = with(
            len,
            &[
                (446, &[0x80, 0, 0, 0, 0x17]),
                (510, &[0x55, 0xaa]),
                (0x8001, b"CD001"),
            ],
        );
        assert_eq!(content_of(&hybrid), Content::Iso);
        assert!(Content::BitLocker.is_locked());
    }
}
