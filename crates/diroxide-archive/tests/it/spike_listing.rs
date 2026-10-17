//! Spike M1 (research/alpha2-plan.md 4.2): what listing a huge archive
//! costs. A zip, a tar and a 7z of 100,000 empty entries, and of 1,000,000
//! when `DIRO_SPIKE_LARGE=1` is set; half sit at the root of the archive
//! and half in one folder eight levels down, so the root and a deep folder
//! are both listings of half a million.
//!
//! Run with `cargo nextest run -p diroxide-archive spike_listing
//! --no-capture` to read the figures. They go in the hand-back.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use diroxide_core::{Entry, EntryKind};

use crate::build::{RawMember, RawZip, crc, tar_header, write_number};
use crate::memory::{Peak, mib, resident};

/// The folder eight levels down that holds half the entries.
pub const DEEP: &str = "a/b/c/d/e/f/g/h";

/// How many entries: 100,000, or a million when asked for.
pub fn entry_count() -> usize {
    if std::env::var_os("DIRO_SPIKE_LARGE").is_some_and(|v| v == "1") {
        1_000_000
    } else {
        100_000
    }
}

/// The member names: even ones at the root, odd ones deep.
pub fn names(count: usize) -> impl Iterator<Item = String> {
    (0..count).map(|i| {
        if i % 2 == 0 {
            format!("f{i:07}.txt")
        } else {
            format!("{DEEP}/f{i:07}.txt")
        }
    })
}

/// A scratch folder under the build's own temporary folder.
pub fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("diroxide-spike-")
        .tempdir()
        .unwrap()
}

/// Writes a zip of empty stored members, streaming, to `path`.
pub fn write_zip(path: &Path, count: usize) {
    let mut out = BufWriter::new(File::create(path).unwrap());
    let mut central = Vec::new();
    let mut offset = 0u64;
    let empty_crc = crc(b"");
    for name in names(count) {
        let mut member = RawMember::new(&name, b"", crate::build::Method::Stored);
        member.crc = empty_crc;
        let header = RawZip::local_header(&member);
        out.write_all(&header).unwrap();
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&((3u16 << 8) | 20).to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&(1u16 << 11).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x5021u16.to_le_bytes());
        central.extend_from_slice(&empty_crc.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&(offset as u32).to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        offset += header.len() as u64;
    }
    let cd_offset = offset;
    out.write_all(&central).unwrap();
    let cd_size = central.len() as u64;
    drop(central);
    let record_at = cd_offset + cd_size;
    let count = count as u64;
    let mut tail = Vec::new();
    tail.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
    tail.extend_from_slice(&44u64.to_le_bytes());
    tail.extend_from_slice(&45u16.to_le_bytes());
    tail.extend_from_slice(&45u16.to_le_bytes());
    tail.extend_from_slice(&[0; 8]);
    tail.extend_from_slice(&count.to_le_bytes());
    tail.extend_from_slice(&count.to_le_bytes());
    tail.extend_from_slice(&cd_size.to_le_bytes());
    tail.extend_from_slice(&cd_offset.to_le_bytes());
    tail.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    tail.extend_from_slice(&0u32.to_le_bytes());
    tail.extend_from_slice(&record_at.to_le_bytes());
    tail.extend_from_slice(&1u32.to_le_bytes());
    tail.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    tail.extend_from_slice(&[0; 4]);
    tail.extend_from_slice(&0xFFFFu16.to_le_bytes());
    tail.extend_from_slice(&0xFFFFu16.to_le_bytes());
    tail.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    tail.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    tail.extend_from_slice(&0u16.to_le_bytes());
    out.write_all(&tail).unwrap();
    out.flush().unwrap();
}

/// Writes an uncompressed tar of empty members, streaming, to `path`.
pub fn write_tar(path: &Path, count: usize) {
    let mut out = BufWriter::new(File::create(path).unwrap());
    for name in names(count) {
        out.write_all(&tar_header(name.as_bytes(), 0, b'0'))
            .unwrap();
    }
    out.write_all(&[0u8; 1024]).unwrap();
    out.flush().unwrap();
}

/// Writes a 7z of empty files with a plain header to `path`.
pub fn write_7z(path: &Path, count: usize) {
    let bytes = crate::build::sevenz_empty_files(names(count));
    std::fs::write(path, bytes).unwrap();
}

/// The three spike archives in a scratch folder.
pub fn spike_archives(count: usize) -> (tempfile::TempDir, [PathBuf; 3]) {
    let dir = scratch();
    let zip = dir.path().join("spike.zip");
    let tar = dir.path().join("spike.tar");
    let sevenz = dir.path().join("spike.7z");
    write_zip(&zip, count);
    write_tar(&tar, count);
    write_7z(&sevenz, count);
    (dir, [zip, tar, sevenz])
}

#[test]
fn the_row_type_and_a_flat_listing_cost_this_much() {
    let count = entry_count();
    eprintln!("size_of::<Entry>() = {} bytes", size_of::<Entry>());
    let before = resident();
    let peak = Peak::start();
    let started = Instant::now();
    let rows: Vec<Entry> = names(count)
        .map(|name| Entry::new(name, EntryKind::File).with_size(0))
        .collect();
    let elapsed = started.elapsed();
    let peak = peak.stop();
    eprintln!(
        "{count} entries as rows: {:.1} MiB more resident ({:.1} MiB peak), {:?}",
        mib(resident().saturating_sub(before)),
        mib(peak),
        elapsed
    );
    assert_eq!(rows.len(), count);
}

#[test]
fn the_crates_alone_list_a_huge_zip_tar_and_7z() {
    let count = entry_count();
    let (_dir, [zip_path, tar_path, sevenz_path]) = spike_archives(count);
    // zip: the crate reads the whole central directory into its own map.
    let before = resident();
    let peak = Peak::start();
    let started = Instant::now();
    let archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
    let names_seen = archive.file_names().count();
    let elapsed = started.elapsed();
    let peak = peak.stop();
    eprintln!(
        "zip crate, {count} entries: {:.1} MiB more ({:.1} MiB peak), {elapsed:?}",
        mib(resident().saturating_sub(before)),
        mib(peak)
    );
    assert_eq!(names_seen, count);
    drop(archive);
    // tar: headers one by one, nothing kept.
    let started = Instant::now();
    let mut seen = 0usize;
    let mut tar = tar::Archive::new(File::open(&tar_path).unwrap());
    for entry in tar.entries().unwrap() {
        let entry = entry.unwrap();
        seen += usize::from(!entry.path_bytes().is_empty());
    }
    eprintln!(
        "tar crate, {count} entries walked: {:?}, {} bytes of archive",
        started.elapsed(),
        std::fs::metadata(&tar_path).unwrap().len()
    );
    assert_eq!(seen, count);
    // 7z: the crate reads the header and keeps every entry.
    let before = resident();
    let peak = Peak::start();
    let started = Instant::now();
    let archive = sevenz_rust2::Archive::read(
        &mut File::open(&sevenz_path).unwrap(),
        &sevenz_rust2::Password::empty(),
    )
    .unwrap();
    let elapsed = started.elapsed();
    let peak = peak.stop();
    eprintln!(
        "7z crate, {count} entries: {:.1} MiB more ({:.1} MiB peak), {elapsed:?}",
        mib(resident().saturating_sub(before)),
        mib(peak)
    );
    assert_eq!(archive.files.len(), count);
}

/// The 7z number writer agrees with the format's reading rule.
#[test]
fn seven_z_numbers_round_trip() {
    for value in [
        0u64,
        1,
        127,
        128,
        0x3FFF,
        0x4000,
        1 << 20,
        1 << 40,
        u64::MAX,
    ] {
        let mut out = Vec::new();
        write_number(&mut out, value);
        let first = out[0];
        let mut mask = 0x80u8;
        let mut read = 0u64;
        let mut i = 0usize;
        loop {
            if i == 8 || first & mask == 0 {
                if i < 8 {
                    read |= u64::from(first & mask.wrapping_sub(1)) << (8 * i);
                }
                break;
            }
            read |= u64::from(out[1 + i]) << (8 * i);
            mask >>= 1;
            i += 1;
        }
        assert_eq!(read, value, "{value}");
    }
}
