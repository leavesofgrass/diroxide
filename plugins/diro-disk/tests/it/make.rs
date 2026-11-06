//! Images written in pure Rust by the tests, on every system (R29 1.10):
//! never downloaded, never made by a tool the system may lack.

// Each reader's tests use some of the builders.
#![allow(dead_code)]

use std::io::Cursor;
use std::sync::Arc;

use diro_disk::host::Stamp;
use diro_disk::image::Image;
use diro_disk::image::source::{Backing, Counted, Memory};
use diroxide_vfs::archive::limits::Limits;
use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::PathSeparator;
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{InputEntry, InputTree, IsoImageWriter};

/// Which names an ISO is written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsoNames {
    /// Rock Ridge and Joliet both.
    Both,
    /// Joliet only.
    Joliet,
    /// Plain ISO 9660 names only.
    Plain,
}

/// An ISO of `entries`, with the names asked for.
pub fn iso_with(entries: Vec<InputEntry>, names: IsoNames) -> Vec<u8> {
    let features = match names {
        IsoNames::Both => CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: true,
            },
            joliet: Some(JolietLevel::Level3),
            rock_ridge: Some(RripOptions::default()),
            ..CreationFeatures::default()
        },
        IsoNames::Joliet => CreationFeatures::joliet(JolietLevel::Level3),
        IsoNames::Plain => CreationFeatures::default(),
    };
    let options = IsoFormatOptions {
        volume_name: "FEDORA_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: false,
        features,
    };
    let tree = InputTree::new(PathSeparator::ForwardSlash, entries);
    let data =
        IsoImageWriter::create(Cursor::new(vec![0; 4 * 1024 * 1024]), tree, options).unwrap();
    data.into_inner()
}

/// The ISO most tests use: an EFI tree, a readme, a long name, a link
/// and a file of 300 KB.
pub fn fedora_iso(names: IsoNames) -> Vec<u8> {
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let mut entries = vec![
        InputEntry::directory(
            "EFI",
            vec![InputEntry::directory(
                "BOOT",
                vec![InputEntry::file("BOOTX64.EFI", b"boot loader".to_vec())],
            )],
        ),
        InputEntry::file("README.TXT", b"Read me first.".to_vec()),
        InputEntry::file("BIG.BIN", big),
    ];
    if names != IsoNames::Plain {
        entries.push(InputEntry::file(
            "a rather long name for a plain ISO.txt",
            b"long".to_vec(),
        ));
    }
    if names == IsoNames::Both {
        entries.push(InputEntry::symlink("latest", "README.TXT"));
    }
    iso_with(entries, names)
}

/// An image opened from bytes in memory, the reads counted.
pub fn opened(bytes: Vec<u8>, name: &str) -> (Image, Arc<Counted>) {
    let counted = Arc::new(Counted::new(Arc::new(Memory::new(bytes))));
    let stamp = Stamp {
        size: counted.len(),
        modified: None,
    };
    let image = Image::open(
        Arc::clone(&counted) as Arc<dyn Backing>,
        name,
        stamp,
        &Limits::default(),
    )
    .unwrap();
    (image, counted)
}

/// A FAT volume of `sectors` 512-byte sectors, with folders and files
/// at their paths, written by `fatfs`.
pub fn fat_volume(
    fat_type: fatfs::FatType,
    sectors: u32,
    label: &[u8; 11],
    folders: &[&str],
    files: &[(&str, &[u8])],
) -> Vec<u8> {
    use std::io::Write;
    let mut cur = Cursor::new(vec![0u8; sectors as usize * 512]);
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fat_type)
        .total_sectors(sectors)
        .bytes_per_cluster(512)
        .volume_label(*label);
    fatfs::format_volume(&mut cur, options).unwrap();
    cur.set_position(0);
    {
        let fs = fatfs::FileSystem::new(&mut cur, fatfs::FsOptions::new()).unwrap();
        let root = fs.root_dir();
        for folder in folders {
            root.create_dir(folder).unwrap();
        }
        for (path, bytes) in files {
            let mut file = root.create_file(path).unwrap();
            file.write_all(bytes).unwrap();
        }
    }
    cur.into_inner()
}

/// A GPT disk of `size` bytes holding `volumes` in order, each with its
/// type GUID and name, the first at 1 MiB, written by `gptman`.
pub fn gpt_with(size: usize, volumes: &[([u8; 16], &str, &[u8])]) -> Vec<u8> {
    use std::io::{Seek, SeekFrom, Write};
    let sector = 512u64;
    let mut cur = Cursor::new(vec![0u8; size]);
    let mut gpt = gptman::GPT::new_from(&mut cur, sector, [0x5a; 16]).unwrap();
    let mut lba = 2048u64;
    let mut places = Vec::new();
    for (i, (kind, name, bytes)) in volumes.iter().enumerate() {
        let sectors = (bytes.len() as u64).div_ceil(sector);
        gpt[i as u32 + 1] = gptman::GPTPartitionEntry {
            partition_type_guid: *kind,
            unique_partition_guid: [i as u8 + 1; 16],
            starting_lba: lba,
            ending_lba: lba + sectors - 1,
            attribute_bits: 0,
            partition_name: (*name).into(),
        };
        places.push((lba * sector, *bytes));
        lba += sectors.next_multiple_of(2048);
    }
    gptman::GPT::write_protective_mbr_into(&mut cur, sector).unwrap();
    gpt.write_into(&mut cur).unwrap();
    for (at, bytes) in places {
        cur.seek(SeekFrom::Start(at)).unwrap();
        cur.write_all(bytes).unwrap();
    }
    cur.into_inner()
}

/// GPT type GUIDs as stored, for the tests.
pub mod guid {
    /// EFI System.
    pub const EFI: [u8; 16] = [
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ];
    /// Linux filesystem.
    pub const LINUX: [u8; 16] = [
        0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d,
        0xe4,
    ];
    /// Microsoft basic data.
    pub const BASIC_DATA: [u8; 16] = [
        0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99,
        0xc7,
    ];
}
