//! The containers through the real host, from the disks qemu-img wrote
//! around one FAT volume: each opens to the same files.

use crate::fixture::{Fixture, at};
use crate::fixtures::fixture;

fn the_fat_inside_reads(f: &Fixture, name: &str, fixture_name: &str) {
    let root = f.image(name, &fixture(fixture_name));
    let mut names = f.names(&root);
    names.sort();
    assert_eq!(names, ["docs", "hello.txt"], "{name}");
    assert_eq!(
        f.read(&at(&root, &["hello.txt"])).unwrap(),
        b"Hello, world.",
        "{name}"
    );
    assert_eq!(
        f.read(&at(&root, &["docs", "inner.txt"])).unwrap(),
        b"inside",
        "{name}"
    );
}

#[test]
fn every_container_qemu_img_writes_opens_to_the_same_files() {
    let f = Fixture::new();
    for (name, fixture_name) in [
        ("plain.img", "fat.img"),
        ("disk.qcow2", "fat.qcow2"),
        ("disk.vmdk", "fat.vmdk"),
        ("disk.vhdx", "fat.vhdx"),
        ("disk.vdi", "fat.vdi"),
        ("dynamic.vhd", "dynamic.vhd"),
        ("fixed.vhd", "fixed.vhd"),
    ] {
        the_fat_inside_reads(&f, name, fixture_name);
    }
}

#[test]
fn a_qcow2_with_deflated_clusters_opens_too() {
    let f = Fixture::new();
    the_fat_inside_reads(&f, "deflate.qcow2", "deflate.qcow2");
}

mod sparse {
    //! The sparse guard: an image that claims 2 TiB in a few megabytes is
    //! listed reading its tables and the blocks the listing touches, in
    //! well under a second, and says both figures.

    use std::time::Instant;

    use crate::make::{TWO_TIB, fat_volume, opened, qcow2_bomb, vdi_bomb, vhd_bomb};

    fn floppy() -> Vec<u8> {
        fat_volume(
            fatfs::FatType::Fat12,
            2880,
            b"FLOPPY     ",
            &[],
            &[("README.TXT", b"Read me first.")],
        )
    }

    #[test]
    fn a_qcow2_and_a_vdi_claiming_2_tib_list_in_kilobytes() {
        for (name, bytes) in [
            ("bomb.qcow2", qcow2_bomb(&floppy())),
            ("bomb.vdi", vdi_bomb(&floppy())),
        ] {
            let held = bytes.len() as u64;
            let started = Instant::now();
            let (image, counted) = opened(bytes, name);
            let rows = image
                .volume(None)
                .unwrap()
                .with_tree(|t| t.list(&[]))
                .unwrap()
                .rows;
            let elapsed = started.elapsed();
            eprintln!(
                "{name}: claims {TWO_TIB}, holds {held}, listed reading {} bytes in {} reads, {elapsed:?}",
                counted.bytes(),
                counted.reads()
            );
            assert_eq!(rows.len(), 1, "{name}");
            assert_eq!(image.claimed, TWO_TIB);
            assert!(
                counted.bytes() < 10 * 1024 * 1024,
                "{name}: {}",
                counted.bytes()
            );
            assert!(elapsed.as_secs() < 5, "{name}: {elapsed:?}");
            let line = image.sparse_line().unwrap();
            assert!(line.starts_with("Sparse: claims 2.2 TB, holds "), "{line}");
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn a_vhd_claiming_2_tib_with_nothing_in_it_says_so_quickly() {
        let bytes = vhd_bomb();
        let started = Instant::now();
        let counted = std::sync::Arc::new(diro_disk::image::source::Counted::new(
            std::sync::Arc::new(diro_disk::image::source::Memory::new(bytes)),
        ));
        let result = diro_disk::image::Image::open(
            std::sync::Arc::clone(&counted) as _,
            "bomb.vhd",
            diro_disk::host::Stamp {
                size: 0,
                modified: None,
            },
            &diroxide_vfs::archive::limits::Limits::default(),
            &|_| Err(diro_disk::error::DiskError::NotFound),
        );
        let elapsed = started.elapsed();
        eprintln!(
            "bomb.vhd: read {} bytes in {} reads, {elapsed:?}",
            counted.bytes(),
            counted.reads()
        );
        assert!(result.is_err(), "an empty disk has nothing to read");
        assert!(counted.bytes() < 10 * 1024 * 1024);
        assert!(elapsed.as_secs() < 5);
    }
}

#[test]
fn a_vmdk_descriptor_reads_its_flat_extent_beside_it() {
    let f = Fixture::new();
    let disk = crate::make::fat_volume(
        fatfs::FatType::Fat12,
        2880,
        b"FLOPPY     ",
        &[],
        &[("README.TXT", b"Read me first.")],
    );
    std::fs::write(f.image_path("disk-flat.vmdk"), &disk).unwrap();
    let descriptor = "# Disk DescriptorFile\nversion=1\nCID=fffffffe\nparentCID=ffffffff\n\
        createType=\"monolithicFlat\"\n\n# Extent description\nRW 2880 FLAT \"disk-flat.vmdk\" 0\n";
    let root = f.image("disk.vmdk", descriptor.as_bytes());
    assert_eq!(f.names(&root), ["README.TXT"]);
    assert_eq!(
        f.read(&at(&root, &["README.TXT"])).unwrap(),
        b"Read me first."
    );
    // An extent outside the folder is refused, never opened.
    let escape = "# Disk DescriptorFile\nRW 2880 FLAT \"../outside.vmdk\" 0\n";
    let root = f.image("escape.vmdk", escape.as_bytes());
    let line = crate::fixture::words(&f.list(&root).unwrap_err());
    assert!(line.contains("an extent outside its folder"), "{line}");
}
