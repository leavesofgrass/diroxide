//! ISO 9660 with Joliet and Rock Ridge, through the real host and through
//! the image alone with its reads counted.

use diroxide_core::EntryKind;

use crate::fixture::{Fixture, at};
use crate::make::{IsoNames, fedora_iso, opened};

#[test]
fn an_iso_lists_reads_and_copies_out_through_the_host() {
    let f = Fixture::new();
    let root = f.image("fedora.iso", &fedora_iso(IsoNames::Both));
    let mut names = f.names(&root);
    names.sort();
    assert_eq!(
        names,
        [
            "BIG.BIN",
            "EFI",
            "README.TXT",
            "a rather long name for a plain ISO.txt",
            "latest"
        ]
    );
    assert_eq!(f.names(&at(&root, &["EFI"])), ["BOOT"]);
    assert_eq!(
        f.read(&at(&root, &["EFI", "BOOT", "BOOTX64.EFI"])).unwrap(),
        b"boot loader"
    );
    let big = f.read(&at(&root, &["BIG.BIN"])).unwrap();
    assert_eq!(big.len(), 300_000);
    assert!(big.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
    // A link is a row with its target as text, and reads as that text.
    let link = f.stat(&at(&root, &["latest"])).unwrap();
    assert_eq!(link.kind, EntryKind::Link);
    assert_eq!(
        link.link.as_ref().map(|l| l.target.as_str()),
        Some("README.TXT")
    );
    assert_eq!(f.read(&at(&root, &["latest"])).unwrap(), b"README.TXT");
    let readme = f.stat(&at(&root, &["README.TXT"])).unwrap();
    assert_eq!(readme.size, Some(14));
    assert!(readme.modified.is_some());
    assert!(f.stat(&at(&root, &["NOPE.TXT"])).is_err());
}

#[test]
fn joliet_and_plain_names_read_without_their_versions() {
    let (image, _) = opened(fedora_iso(IsoNames::Joliet), "joliet.iso");
    let volume = image.volume(None).unwrap();
    let mut names: Vec<String> = volume
        .with_tree(|t| t.list(&[]))
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.shown)
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "BIG.BIN",
            "EFI",
            "README.TXT",
            "a rather long name for a plain ISO.txt"
        ]
    );
    assert_eq!(image.layout_words(), "ISO 9660, Joliet");
    let (plain, _) = opened(fedora_iso(IsoNames::Plain), "plain.iso");
    let mut names: Vec<String> = plain
        .volume(None)
        .unwrap()
        .with_tree(|t| t.list(&[]))
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.shown)
        .collect();
    names.sort();
    assert_eq!(names, ["BIG.BIN", "EFI", "README.TXT"]);
    assert_eq!(plain.layout_words(), "ISO 9660");
}

#[test]
fn a_listing_reads_kilobytes_not_the_image() {
    let bytes = fedora_iso(IsoNames::Both);
    let size = bytes.len() as u64;
    let (image, counted) = opened(bytes, "fedora.iso");
    let volume = image.volume(None).unwrap();
    volume.with_tree(|t| t.list(&["EFI", "BOOT"])).unwrap();
    eprintln!(
        "ISO listing: {} bytes in {} reads",
        counted.bytes(),
        counted.reads()
    );
    // Open and list three folders: the descriptors and three folder
    // records, never the 300 KB file.
    assert!(
        counted.bytes() < 200 * 1024,
        "{} of {size}",
        counted.bytes()
    );
    assert!(counted.reads() < 400, "{} reads", counted.reads());
}
