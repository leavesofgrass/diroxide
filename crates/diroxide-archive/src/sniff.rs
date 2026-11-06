//! What a file is by its first bytes (research/archives.md 5.1, step 2):
//! the signatures of every archive and compression format diroxide may
//! meet, and the decision between the name and the bytes. The bytes win;
//! when they disagree with the name, the title says so ("a.zip, 7z
//! archive, in D:\docs"). "Open as archive" opens by the bytes alone, so a
//! zip named `.dat`, or a document that is a zip inside, opens as one.

use diroxide_vfs::VfsError;

use crate::format::ArchiveFormat;

/// How many bytes the sniffer wants: tar's magic sits at 257 and its
/// header is 512 bytes. ISO's sits at 32,769, read only when given.
pub const SNIFF_LEN: usize = 512;

/// The bytes an ISO image needs for its signature.
pub const ISO_SNIFF_LEN: usize = 32_774;

/// The bytes the other disk-image signatures need: a GPT header at 512
/// and an ext superblock's magic at 1,080 (alpha.7, R29 1.2). "Open as
/// disk image" reads this many; the archive provider reads
/// [`SNIFF_LEN`] and finds the images whose signature is at the start.
pub const IMAGE_SNIFF_LEN: usize = 2_048;

/// What a disk image needs when no plugin claims it: the cause of "Could
/// not open disk.vhd: it needs the disk image plugin or 7-Zip."
pub const DISK_IMAGE_NEEDS: &str = "the disk image plugin or 7-Zip";

/// A format known by its first bytes.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signature {
    /// A zip: `PK` 03 04.
    Zip,
    /// An empty zip: `PK` 05 06, the end record alone.
    ZipEmpty,
    /// The first part of a spanned zip: `PK` 07 08.
    ZipSpanned,
    /// A 7z: `7z` BC AF 27 1C.
    SevenZ,
    /// An xz stream: FD `7zXZ` 00.
    Xz,
    /// A gzip stream: 1F 8B.
    Gzip,
    /// A bzip2 stream: `BZh` and a block size digit.
    Bzip2,
    /// A Zstandard frame: 28 B5 2F FD.
    Zstd,
    /// An lzip stream: `LZIP`.
    Lzip,
    /// A tar: `ustar` at 257.
    Tar,
    /// An old tar without the magic, known by its header checksum.
    TarOld,
    /// A RAR archive: `Rar!` 1A 07.
    Rar,
    /// An ISO 9660 image: `CD001` at 32,769.
    Iso,
    /// A Cabinet archive: `MSCF` and four zero bytes.
    Cab,
    /// An RPM package: ED AB EE DB.
    Rpm,
    /// A cpio archive: `070707`, `070701` or `070702`, or the old binary
    /// header's C7 71 either way round.
    Cpio,
    /// A Unix ar archive: `!<arch>` and a line feed.
    Ar,
    /// An ARJ archive: 60 EA.
    Arj,
    /// An LZH or LHA archive: a method such as `-lh5-` at 2.
    Lzh,
    /// A squashfs image: `hsqs`.
    Squashfs,
    /// A VHD disk image (dynamic, its footer copied first): `conectix`.
    Vhd,
    /// A VHDX disk image: `vhdxfile`.
    Vhdx,
    /// A VirtualBox disk image: its magic 7F 10 DA BE at 64.
    Vdi,
    /// A VMDK disk image: a sparse extent's `KDMV`, or a descriptor that
    /// starts `# Disk DescriptorFile`.
    Vmdk,
    /// A QCOW2 disk image: `QFI` FB.
    Qcow2,
    /// An NTFS volume image: `NTFS` and four spaces at 3.
    Ntfs,
    /// A FAT volume image: `FAT12`, `FAT16` (at 54) or `FAT32` (at 82),
    /// with the boot sector's 55 AA at 510.
    Fat,
    /// A disk with a GUID partition table: `EFI PART` at 512.
    Gpt,
    /// A disk with a master boot record: 55 AA at 510, after the volume
    /// signatures that also end their first sector so.
    Mbr,
    /// An ext2, ext3 or ext4 image: 53 EF at 1,080.
    Ext,
}

impl Signature {
    /// Every signature, in the order the tests list them.
    pub const ALL: &'static [Signature] = &[
        Signature::Zip,
        Signature::ZipEmpty,
        Signature::ZipSpanned,
        Signature::SevenZ,
        Signature::Xz,
        Signature::Gzip,
        Signature::Bzip2,
        Signature::Zstd,
        Signature::Lzip,
        Signature::Tar,
        Signature::TarOld,
        Signature::Rar,
        Signature::Iso,
        Signature::Cab,
        Signature::Rpm,
        Signature::Cpio,
        Signature::Ar,
        Signature::Arj,
        Signature::Lzh,
        Signature::Squashfs,
        Signature::Vhd,
        Signature::Vhdx,
        Signature::Vdi,
        Signature::Vmdk,
        Signature::Qcow2,
        Signature::Ntfs,
        Signature::Fat,
        Signature::Gpt,
        Signature::Mbr,
        Signature::Ext,
    ];

    /// Whether it is a disk or volume image, which the disk-image plugin
    /// reads (and 7-Zip, for some).
    pub const fn is_disk_image(self) -> bool {
        matches!(
            self,
            Signature::Iso
                | Signature::Squashfs
                | Signature::Vhd
                | Signature::Vhdx
                | Signature::Vdi
                | Signature::Vmdk
                | Signature::Qcow2
                | Signature::Ntfs
                | Signature::Fat
                | Signature::Gpt
                | Signature::Mbr
                | Signature::Ext
        )
    }

    /// What it is, in words: "zip archive", "RAR archive".
    pub const fn word(self) -> &'static str {
        match self {
            Signature::Zip | Signature::ZipEmpty => "zip archive",
            Signature::ZipSpanned => "spanned zip set",
            Signature::SevenZ => "7z archive",
            Signature::Xz => "xz stream",
            Signature::Gzip => "gzip stream",
            Signature::Bzip2 => "bzip2 stream",
            Signature::Zstd => "Zstandard stream",
            Signature::Lzip => "lzip stream",
            Signature::Tar | Signature::TarOld => "tar archive",
            Signature::Rar => "RAR archive",
            Signature::Iso => "ISO image",
            Signature::Cab => "Cabinet archive",
            Signature::Rpm => "RPM package",
            Signature::Cpio => "cpio archive",
            Signature::Ar => "ar archive",
            Signature::Arj => "ARJ archive",
            Signature::Lzh => "LZH archive",
            Signature::Squashfs => "squashfs image",
            Signature::Vhd => "VHD disk image",
            Signature::Vhdx => "VHDX disk image",
            Signature::Vdi => "VirtualBox disk image",
            Signature::Vmdk => "VMDK disk image",
            Signature::Qcow2 => "QCOW2 disk image",
            Signature::Ntfs => "NTFS volume image",
            Signature::Fat => "FAT volume image",
            Signature::Gpt => "GPT disk image",
            Signature::Mbr => "MBR disk image",
            Signature::Ext => "ext volume image",
        }
    }

    /// The archive format the provider reads it as, by the bytes alone:
    /// a gzip stream as tar.gz and an xz stream as plain xz, until the
    /// provider looks inside. `None` for the formats not read here.
    pub const fn format(self) -> Option<ArchiveFormat> {
        match self {
            Signature::Zip | Signature::ZipEmpty => Some(ArchiveFormat::Zip),
            Signature::SevenZ => Some(ArchiveFormat::SevenZ),
            Signature::Xz => Some(ArchiveFormat::Xz),
            Signature::Gzip => Some(ArchiveFormat::TarGz),
            Signature::Tar | Signature::TarOld => Some(ArchiveFormat::Tar),
            _ => None,
        }
    }

    /// Why it is not read here, as the cause of "Could not open NAME:
    /// CAUSE.": the error the provider gives. A disk image needs the
    /// disk-image plugin or 7-Zip, said as such.
    pub fn not_read(self) -> VfsError {
        if self.is_disk_image() {
            return VfsError::NeedsHelper {
                what: DISK_IMAGE_NEEDS.to_owned(),
            };
        }
        let what = match self {
            Signature::ZipSpanned => "a spanned zip set, which needs the 7z tool",
            Signature::Bzip2 => "bzip2 compression on its own",
            Signature::Zstd => "Zstandard compression",
            Signature::Lzip => "lzip compression",
            Signature::Rar => "the RAR format",
            Signature::Cab => "the Cabinet format",
            Signature::Rpm => "the RPM format",
            Signature::Cpio => "the cpio format",
            Signature::Ar => "the ar format",
            Signature::Arj => "the ARJ format",
            Signature::Lzh => "the LZH format",
            _ => "a format this version does not read",
        };
        VfsError::UnsupportedFeature {
            what: what.to_owned(),
        }
    }
}

/// The signature of the first bytes of a file, or `None`. Give it at
/// least [`SNIFF_LEN`] bytes, and [`ISO_SNIFF_LEN`] to find an ISO image.
pub fn sniff_signature(head: &[u8]) -> Option<Signature> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"PK\x03\x04") {
        Some(Signature::Zip)
    } else if starts(b"PK\x05\x06") {
        Some(Signature::ZipEmpty)
    } else if starts(b"PK\x07\x08") {
        Some(Signature::ZipSpanned)
    } else if starts(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Some(Signature::SevenZ)
    } else if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        Some(Signature::Xz)
    } else if starts(&[0x1F, 0x8B]) {
        Some(Signature::Gzip)
    } else if head.len() >= 4 && starts(b"BZh") && (b'1'..=b'9').contains(&head[3]) {
        Some(Signature::Bzip2)
    } else if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Some(Signature::Zstd)
    } else if starts(b"LZIP") {
        Some(Signature::Lzip)
    } else if starts(b"Rar!\x1A\x07") {
        Some(Signature::Rar)
    } else if let Some(signature) = sniff_start(head) {
        Some(signature)
    } else if head.get(257..262) == Some(b"ustar".as_slice()) {
        Some(Signature::Tar)
    } else if head.get(32_769..32_774) == Some(b"CD001".as_slice()) {
        Some(Signature::Iso)
    } else if let Some(signature) = sniff_volume(head) {
        Some(signature)
    } else if head.len() >= 512 && tar_checksum_ok(&head[..512]) {
        Some(Signature::TarOld)
    } else if head.get(510..512) == Some([0x55, 0xAA].as_slice()) {
        Some(Signature::Mbr)
    } else {
        None
    }
}

/// The signatures at the very start of a file: the 7z tool's archive
/// formats and the disk-image containers (alpha.7, R29 1.2). Two-byte
/// magics are held to their header's next field, so a stray file is not
/// taken for an archive.
fn sniff_start(head: &[u8]) -> Option<Signature> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"MSCF\0\0\0\0") {
        Some(Signature::Cab)
    } else if starts(&[0xED, 0xAB, 0xEE, 0xDB]) {
        Some(Signature::Rpm)
    } else if starts(b"070707") || starts(b"070701") || starts(b"070702") {
        Some(Signature::Cpio)
    } else if starts(b"!<arch>\n") {
        Some(Signature::Ar)
    } else if starts(&[0x60, 0xEA])
        && arj_header_size(head).is_some_and(|n| (30..=2600).contains(&n))
    {
        Some(Signature::Arj)
    } else if lzh_method(head) {
        Some(Signature::Lzh)
    } else if starts(b"hsqs") {
        Some(Signature::Squashfs)
    } else if starts(b"conectix") {
        Some(Signature::Vhd)
    } else if starts(b"vhdxfile") {
        Some(Signature::Vhdx)
    } else if starts(b"KDMV") || starts(b"# Disk DescriptorFile") {
        Some(Signature::Vmdk)
    } else if starts(b"QFI\xFB") {
        Some(Signature::Qcow2)
    } else if head.get(64..68) == Some([0x7F, 0x10, 0xDA, 0xBE].as_slice()) {
        Some(Signature::Vdi)
    } else {
        None
    }
}

/// The volume and partition-table signatures inside the first sectors.
fn sniff_volume(head: &[u8]) -> Option<Signature> {
    let at = |offset: usize, magic: &[u8]| head.get(offset..offset + magic.len()) == Some(magic);
    let boot = at(510, &[0x55, 0xAA]);
    if at(3, b"NTFS    ") {
        Some(Signature::Ntfs)
    } else if boot && (at(54, b"FAT12   ") || at(54, b"FAT16   ") || at(82, b"FAT32   ")) {
        Some(Signature::Fat)
    } else if at(512, b"EFI PART") {
        Some(Signature::Gpt)
    } else if at(1080, &[0x53, 0xEF]) {
        Some(Signature::Ext)
    } else {
        None
    }
}

/// Whether a name is a disk image's by its extension: the names that open
/// by name and the raw ones (core's lists).
fn is_disk_image_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    diroxide_core::place::DISK_IMAGE_EXTENSIONS
        .iter()
        .chain(diroxide_core::place::RAW_IMAGE_EXTENSIONS)
        .any(|ext| lower.len() > ext.len() && lower.ends_with(ext))
}

/// An ARJ main header's basic size, little-endian after its magic.
fn arj_header_size(head: &[u8]) -> Option<u16> {
    let bytes = head.get(2..4)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

/// Whether bytes 2 to 7 are an LZH method: `-lh5-`, `-lh0-`, `-lhd-`,
/// `-lz5-` and the like.
fn lzh_method(head: &[u8]) -> bool {
    let Some(method) = head.get(2..7) else {
        return false;
    };
    method[0] == b'-'
        && method[1] == b'l'
        && matches!(method[2], b'h' | b'z')
        && (method[3].is_ascii_digit() || method[3] == b'd' || method[3] == b's')
        && method[4] == b'-'
}

/// Whether a 512-byte block is a tar header by its checksum: the sum of
/// its bytes, with the checksum field read as spaces, equals the octal
/// number in that field. An all-zero block is the end of a tar, not a
/// header.
pub fn tar_checksum_ok(block: &[u8]) -> bool {
    if block.len() < 512 || block.iter().all(|&b| b == 0) {
        return false;
    }
    let field = &block[148..156];
    let digits: Vec<u8> = field
        .iter()
        .copied()
        .skip_while(|&b| b == b' ' || b == 0)
        .take_while(u8::is_ascii_digit)
        .collect();
    if digits.is_empty() || digits.iter().any(|&b| b > b'7') {
        return false;
    }
    let Some(stored) = digits.iter().try_fold(0u32, |n, &d| {
        n.checked_mul(8)?.checked_add(u32::from(d - b'0'))
    }) else {
        return false;
    };
    let unsigned: u32 = block[..512]
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u32::from(b)
            }
        })
        .sum();
    stored == unsigned
}

/// The provider's decision for one file: the format it opens it as, and
/// what the name and the bytes said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sniffed {
    /// The format to open it as.
    pub format: ArchiveFormat,
    /// The format the name says, if any.
    pub by_name: Option<ArchiveFormat>,
    /// The signature of the bytes, if any.
    pub signature: Option<Signature>,
}

impl Sniffed {
    /// Whether the bytes and the name disagree, so the title names the
    /// format ("a.zip, 7z archive, in D:\docs").
    pub fn disagrees(&self) -> bool {
        self.by_name
            .is_some_and(|name| !same_family(name, self.format))
    }

    /// Whether the provider still has to look inside a compressed stream
    /// to tell a compressed tar from one compressed file.
    pub fn look_inside(&self) -> bool {
        matches!(self.signature, Some(Signature::Gzip | Signature::Xz))
            && !matches!(
                self.by_name,
                Some(ArchiveFormat::TarGz | ArchiveFormat::TarXz)
            )
    }
}

/// Whether two formats are the same wrapper family, so a `.xz` holding a
/// tar or a `.tar.gz` holding one file is no disagreement worth a title.
fn same_family(a: ArchiveFormat, b: ArchiveFormat) -> bool {
    use ArchiveFormat::{TarGz, TarXz, Xz};
    a == b || matches!((a, b), (TarXz, Xz) | (Xz, TarXz) | (TarGz, TarGz))
}

/// Decides the format of a file from its name and its first bytes (at
/// least [`SNIFF_LEN`] of them when the file is that long). The bytes win.
/// A zip by its name is accepted without a signature at the start, since a
/// self-extracting zip carries a program first; every other format needs
/// its signature. A known format that is not read here, or no format at
/// all, is the error to say.
pub fn decide(name: &str, head: &[u8]) -> Result<Sniffed, VfsError> {
    let by_name = ArchiveFormat::from_name(name);
    let signature = sniff_signature(head);
    let format = match (signature, by_name) {
        (Some(signature), _) => match signature.format() {
            Some(ArchiveFormat::Xz) if by_name == Some(ArchiveFormat::TarXz) => {
                ArchiveFormat::TarXz
            }
            Some(format) => format,
            None => return Err(signature.not_read()),
        },
        (None, Some(ArchiveFormat::Zip)) => ArchiveFormat::Zip,
        (None, Some(_)) => {
            return Err(VfsError::Corrupt {
                what: "its first bytes are not the archive its name says".to_owned(),
            });
        }
        (None, None) if is_disk_image_name(name) => {
            return Err(VfsError::NeedsHelper {
                what: DISK_IMAGE_NEEDS.to_owned(),
            });
        }
        (None, None) => {
            return Err(VfsError::UnsupportedFeature {
                what: "no archive format diroxide reads".to_owned(),
            });
        }
    };
    Ok(Sniffed {
        format,
        by_name,
        signature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A first block that carries each signature.
    fn head_of(signature: Signature) -> Vec<u8> {
        let mut head = vec![0u8; 512];
        let put = |head: &mut Vec<u8>, at: usize, magic: &[u8]| {
            head[at..at + magic.len()].copy_from_slice(magic);
        };
        match signature {
            Signature::Zip => put(&mut head, 0, b"PK\x03\x04"),
            Signature::ZipEmpty => put(&mut head, 0, b"PK\x05\x06"),
            Signature::ZipSpanned => put(&mut head, 0, b"PK\x07\x08"),
            Signature::SevenZ => put(&mut head, 0, &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]),
            Signature::Xz => put(&mut head, 0, &[0xFD, b'7', b'z', b'X', b'Z', 0]),
            Signature::Gzip => put(&mut head, 0, &[0x1F, 0x8B, 0x08]),
            Signature::Bzip2 => put(&mut head, 0, b"BZh9"),
            Signature::Zstd => put(&mut head, 0, &[0x28, 0xB5, 0x2F, 0xFD]),
            Signature::Lzip => put(&mut head, 0, b"LZIP\x01"),
            Signature::Tar => put(&mut head, 257, b"ustar\0"),
            Signature::TarOld => {
                put(&mut head, 0, b"old.txt");
                put(&mut head, 124, b"00000000000");
                let sum: u32 = head
                    .iter()
                    .enumerate()
                    .map(|(i, &b)| {
                        if (148..156).contains(&i) {
                            32
                        } else {
                            u32::from(b)
                        }
                    })
                    .sum();
                put(&mut head, 148, format!("{sum:06o}\0 ").as_bytes());
            }
            Signature::Rar => put(&mut head, 0, b"Rar!\x1A\x07\x01\x00"),
            Signature::Iso => {
                head.resize(ISO_SNIFF_LEN, 0);
                put(&mut head, 32_769, b"CD001");
            }
            Signature::Cab => put(&mut head, 0, b"MSCF\0\0\0\0"),
            Signature::Rpm => put(&mut head, 0, &[0xED, 0xAB, 0xEE, 0xDB, 3, 0]),
            Signature::Cpio => put(&mut head, 0, b"070701000000"),
            Signature::Ar => put(&mut head, 0, b"!<arch>\ndebian-binary   "),
            Signature::Arj => put(&mut head, 0, &[0x60, 0xEA, 40, 0]),
            Signature::Lzh => put(&mut head, 0, b"\x24\x00-lh5-"),
            Signature::Squashfs => put(&mut head, 0, b"hsqs"),
            Signature::Vhd => put(&mut head, 0, b"conectix"),
            Signature::Vhdx => put(&mut head, 0, b"vhdxfile"),
            Signature::Vdi => {
                put(&mut head, 0, b"<<< Oracle VM VirtualBox Disk Image >>>\n");
                put(&mut head, 64, &[0x7F, 0x10, 0xDA, 0xBE]);
            }
            Signature::Vmdk => put(&mut head, 0, b"KDMV\x01\x00\x00\x00"),
            Signature::Qcow2 => put(&mut head, 0, b"QFI\xFB\x00\x00\x00\x03"),
            Signature::Ntfs => {
                put(&mut head, 0, &[0xEB, 0x52, 0x90]);
                put(&mut head, 3, b"NTFS    ");
                put(&mut head, 510, &[0x55, 0xAA]);
            }
            Signature::Fat => {
                put(&mut head, 0, &[0xEB, 0x3C, 0x90]);
                put(&mut head, 54, b"FAT16   ");
                put(&mut head, 510, &[0x55, 0xAA]);
            }
            Signature::Gpt => {
                head.resize(IMAGE_SNIFF_LEN, 0);
                put(&mut head, 510, &[0x55, 0xAA]);
                put(&mut head, 512, b"EFI PART");
            }
            Signature::Mbr => put(&mut head, 510, &[0x55, 0xAA]),
            Signature::Ext => {
                head.resize(IMAGE_SNIFF_LEN, 0);
                put(&mut head, 1080, &[0x53, 0xEF]);
            }
        }
        head
    }

    /// The disk images and the 7z tool's formats of alpha.7 (R29 1.2):
    /// none is read in process; a disk image says what it needs, by its
    /// bytes or by its name, and a hybrid ISO is an ISO.
    #[test]
    fn disk_images_say_what_they_need() {
        for &signature in Signature::ALL {
            if signature.is_disk_image() {
                assert_eq!(signature.format(), None, "{signature:?}");
                assert_eq!(
                    signature.not_read().to_string(),
                    "it needs the disk image plugin or 7-Zip",
                    "{signature:?}"
                );
            }
        }
        let vhd = decide("disk.vhd", &head_of(Signature::Vhd)).unwrap_err();
        assert_eq!(vhd.to_string(), "it needs the disk image plugin or 7-Zip");
        // A fixed VHD keeps its footer at the end, so its start is plain
        // data: the name says what it is.
        let fixed = decide("fixed.vhd", &[0u8; 512]).unwrap_err();
        assert!(matches!(fixed, VfsError::NeedsHelper { .. }), "{fixed}");
        let raw = decide("card.img", &[0u8; 512]).unwrap_err();
        assert!(matches!(raw, VfsError::NeedsHelper { .. }), "{raw}");
        let cab = decide("setup.cab", &head_of(Signature::Cab)).unwrap_err();
        assert_eq!(
            cab.to_string(),
            "it uses the Cabinet format, which is not supported"
        );
        // A hybrid ISO carries a partition table too, and is an ISO.
        let mut hybrid = head_of(Signature::Iso);
        hybrid[510] = 0x55;
        hybrid[511] = 0xAA;
        assert_eq!(sniff_signature(&hybrid), Some(Signature::Iso));
        // A GPT disk's protective MBR does not hide its table.
        assert_eq!(
            sniff_signature(&head_of(Signature::Gpt)),
            Some(Signature::Gpt)
        );
        // Two-byte magics need their header's next field.
        assert_eq!(sniff_signature(&[0x60, 0xEA, 0, 0]), None);
        assert_eq!(sniff_signature(b"xx-lhz-"), None);
    }

    #[test]
    fn every_signature_is_known_by_its_bytes() {
        for &signature in Signature::ALL {
            assert_eq!(
                sniff_signature(&head_of(signature)),
                Some(signature),
                "{signature:?}"
            );
            assert!(!signature.word().is_empty());
        }
        assert_eq!(sniff_signature(b"%PDF-1.7"), None);
        assert_eq!(sniff_signature(&[0u8; 512]), None, "the end of a tar");
        assert_eq!(sniff_signature(b"BZh"), None, "no block size");
        assert_eq!(sniff_signature(b""), None);
    }

    #[test]
    fn the_bytes_win_over_the_name() {
        let zip = head_of(Signature::Zip);
        let as_dat = decide("data.dat", &zip).unwrap();
        assert_eq!(as_dat.format, ArchiveFormat::Zip);
        assert!(!as_dat.disagrees(), "no name format, nothing to disagree");
        let seven = head_of(Signature::SevenZ);
        let named_zip = decide("a.zip", &seven).unwrap();
        assert_eq!(named_zip.format, ArchiveFormat::SevenZ);
        assert!(named_zip.disagrees());
        let xz = head_of(Signature::Xz);
        assert_eq!(
            decide("logs.tar.xz", &xz).unwrap().format,
            ArchiveFormat::TarXz
        );
        let plain = decide("notes.xz", &xz).unwrap();
        assert_eq!(plain.format, ArchiveFormat::Xz);
        assert!(plain.look_inside());
        assert!(
            !decide("logs.tgz", &head_of(Signature::Gzip))
                .unwrap()
                .look_inside()
        );
        let document = decide("report.docx", &zip).unwrap();
        assert_eq!(document.format, ArchiveFormat::Zip);
    }

    #[test]
    fn what_is_not_read_says_so() {
        let rar = decide("a.rar", &head_of(Signature::Rar)).unwrap_err();
        assert_eq!(
            rar.to_string(),
            "it uses the RAR format, which is not supported"
        );
        let spanned = decide("a.zip", &head_of(Signature::ZipSpanned)).unwrap_err();
        assert!(spanned.to_string().contains("needs the 7z tool"));
        let nothing = decide("notes.txt", b"hello").unwrap_err();
        assert_eq!(
            nothing.to_string(),
            "it uses no archive format diroxide reads, which is not supported"
        );
        let lying = decide("a.7z", b"hello").unwrap_err();
        assert!(lying.to_string().starts_with("it is damaged"));
        // A self-extracting zip starts with a program, not a signature.
        assert_eq!(
            decide("setup.zip", b"MZ\x90\x00").unwrap().format,
            ArchiveFormat::Zip
        );
    }
}
