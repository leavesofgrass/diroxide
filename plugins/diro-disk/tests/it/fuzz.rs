//! The fuzz targets' bodies on every fixture and builder image, whole,
//! cut short and bent, so the seeds run in normal tests (the fuzz crate
//! under `fuzz/` runs the same functions with libFuzzer at night): no
//! panic, no hang, no allocation by a claimed size.

use diro_disk::fuzzing;

use crate::fixtures::fixture;
use crate::make::{IsoNames, ext2, fat_volume, fedora_iso, qcow2_bomb, vdi_bomb, vhd_bomb};

/// Every image the tests know, by name.
fn seeds() -> Vec<(String, Vec<u8>)> {
    let mut seeds: Vec<(String, Vec<u8>)> = [
        "ntfs.img",
        "gzip.sqfs",
        "xz.sqfs",
        "lz4.sqfs",
        "big-xz.sqfs",
        "xorriso.iso",
        "fat.img",
        "fat.qcow2",
        "deflate.qcow2",
        "fat.vmdk",
        "fat.vhdx",
        "fat.vdi",
        "dynamic.vhd",
        "fixed.vhd",
        "overlay.qcow2",
    ]
    .iter()
    .map(|name| ((*name).to_owned(), fixture(name)))
    .collect();
    let floppy = fat_volume(
        fatfs::FatType::Fat12,
        2880,
        b"SEED       ",
        &[],
        &[("A.TXT", b"a")],
    );
    seeds.push(("iso".to_owned(), fedora_iso(IsoNames::Both)));
    seeds.push(("ext2".to_owned(), ext2()));
    seeds.push(("ext4".to_owned(), crate::make::ext4()));
    seeds.push(("qcow2 bomb".to_owned(), qcow2_bomb(&floppy)));
    seeds.push(("vdi bomb".to_owned(), vdi_bomb(&floppy)));
    seeds.push(("vhd bomb".to_owned(), vhd_bomb()));
    seeds.push(("floppy".to_owned(), floppy));
    seeds
}

/// Bends a copy: every `step`th byte of the first 64 KiB turned over.
fn bent(bytes: &[u8], step: usize, offset: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let end = out.len().min(64 * 1024);
    for at in (offset..end).step_by(step) {
        out[at] ^= 0xff;
    }
    out
}

#[test]
fn every_seed_whole_cut_and_bent_passes_every_target() {
    for (name, bytes) in seeds() {
        eprintln!("seed {name}: {} bytes", bytes.len());
        fuzzing::all(&bytes);
        for cut in [
            0,
            1,
            512,
            4096,
            bytes.len() / 2,
            bytes.len().saturating_sub(1),
        ] {
            fuzzing::all(&bytes[..cut.min(bytes.len())]);
        }
        for (step, offset) in [(509, 3), (127, 64), (61, 1000)] {
            fuzzing::all(&bent(&bytes, step, offset));
        }
    }
}

#[test]
fn listings_of_any_text_parse() {
    for text in [
        "",
        "----------\n",
        "Path = \n\n",
        "Path = a\nPath = b\nFolder = +\n\n\n",
        "Path = ../../../etc/passwd\nSize = 99999999999999999999\n",
        "7-Zip (a) 99.99999999999 (x64)",
    ] {
        fuzzing::seven_listing(text.as_bytes());
    }
    fuzzing::seven_listing(&[0xff, 0xfe, b'\n', b'=', b' ']);
}

/// The nightly fuzz run's first two crash inputs (Sunday, November 8,
/// 2026): a file that is only the VHD footer's magic, and one with four
/// bytes after it. The sniffer took the eight bytes for a 512-byte footer
/// and sliced below zero. Every target body runs them whole.
#[test]
fn the_nightly_crash_inputs_pass_every_target() {
    fuzzing::all(b"conectix");
    fuzzing::all(b"conectix\xff\xff\x83\x0a");
    fuzzing::all(&[b"conectix".as_slice(), &[0u8; 511]].concat());
    fuzzing::all(&[&[0u8; 500], b"conectix".as_slice()].concat());
}
