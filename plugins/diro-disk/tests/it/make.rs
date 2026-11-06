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
