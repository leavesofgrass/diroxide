//! FAT through the real host: a floppy, a FAT32 stick, and a GPT disk
//! whose EFI partition is FAT16, with long names, attributes and a file
//! copied out; the image is never written.

use crate::fixture::{Fixture, at};
use crate::make::{fat_volume, gpt_with, guid};

#[test]
fn a_fat12_floppy_lists_and_reads_with_long_names() {
    let f = Fixture::new();
    let bytes = fat_volume(
        fatfs::FatType::Fat12,
        2880,
        b"FLOPPY     ",
        &["docs"],
        &[
            ("docs/A Long File Name.txt", b"long name"),
            ("README.TXT", b"Read me first."),
        ],
    );
    let path = f.image_path("floppy.img");
    std::fs::write(&path, &bytes).unwrap();
    let root = crate::fixture::root_of(&path);
    let mut names = f.names(&root);
    names.sort();
    assert_eq!(names, ["README.TXT", "docs"]);
    assert_eq!(f.names(&at(&root, &["docs"])), ["A Long File Name.txt"]);
    assert_eq!(
        f.read(&at(&root, &["docs", "A Long File Name.txt"]))
            .unwrap(),
        b"long name"
    );
    let readme = f.stat(&at(&root, &["README.TXT"])).unwrap();
    assert_eq!(readme.size, Some(14));
    // Nothing of the image changed.
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn a_fat32_stick_reads_a_file_across_many_clusters() {
    let f = Fixture::new();
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
    let bytes = fat_volume(
        fatfs::FatType::Fat32,
        70_000,
        b"STICK      ",
        &[],
        &[("big.bin", &big)],
    );
    let root = f.image("stick.img", &bytes);
    assert_eq!(f.read(&at(&root, &["big.bin"])).unwrap(), big);
}

#[test]
fn a_gpt_disk_with_a_fat_partition_says_its_file_system() {
    let f = Fixture::new();
    let efi = fat_volume(
        fatfs::FatType::Fat16,
        8192,
        b"EFI        ",
        &["EFI", "EFI/BOOT"],
        &[("EFI/BOOT/BOOTX64.EFI", b"boot loader")],
    );
    let bytes = gpt_with(8 * 1024 * 1024, &[(guid::EFI, "", &efi)]);
    let root = f.image("disk.img", &bytes);
    let entries = f.list(&root).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].extra.get("disk.about").map(String::as_str),
        Some("Partition 1, EFI, FAT16, 4.2 MB")
    );
    let boot = at(&root, &["partition-1", "EFI", "BOOT"]);
    assert_eq!(f.names(&boot), ["BOOTX64.EFI"]);
    assert_eq!(
        f.read(&at(&boot, &["BOOTX64.EFI"])).unwrap(),
        b"boot loader"
    );
}
