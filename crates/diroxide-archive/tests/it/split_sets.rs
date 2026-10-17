//! Split archives opened as one (plan commit 28, research/alpha2-plan.md
//! 4.3): `backup.7z.001` and its siblings are read through the parts
//! reader as one seekable archive, nothing joined. Spike P1's archive
//! cases through the provider: a 25 MB zip and a 25 MB 7z in 64 KiB parts
//! with a last part of three bytes, every member listed and read and held
//! to the original, and the title in words.

use std::io::{Cursor, Read, Write};

use diroxide_archive::{Allowances, guard};
use diroxide_core::Place;
use diroxide_vfs::{OpContext, Provider};

use crate::build::{noise, zip_of};
use crate::world::World;

/// The part size of the spike.
const PART: usize = 64 << 10;

/// Members of noise, `count` of `size` bytes, in two folders.
fn members(count: usize, size: usize) -> Vec<(String, Vec<u8>)> {
    (0..count)
        .map(|i| {
            let folder = if i % 2 == 0 { "even" } else { "odd" };
            (
                format!("{folder}/m{i:02}.bin"),
                noise(size, i as u64 * 2 + 1),
            )
        })
        .collect()
}

/// A zip of stored members, with a comment that makes its last part
/// exactly three bytes.
fn stored_zip(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let build = |comment: usize| {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in members {
            writer.start_file(name.as_str(), options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.set_comment("c".repeat(comment)).unwrap();
        writer.finish().unwrap().into_inner()
    };
    let plain = build(0).len();
    let comment = (3 + PART - plain % PART) % PART;
    let zip = build(comment);
    assert_eq!(zip.len() % PART, 3);
    zip
}

/// A 7z with no compression (one block per member), padded by its last
/// member so its last part is exactly three bytes.
fn copied_7z(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};
    let build = |pad: usize| {
        let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
        writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
        for (name, data) in members {
            writer
                .push_archive_entry(ArchiveEntry::new_file(name), Some(data.as_slice()))
                .unwrap();
        }
        let padding = vec![b'p'; pad];
        writer
            .push_archive_entry(ArchiveEntry::new_file("pad.txt"), Some(padding.as_slice()))
            .unwrap();
        writer.finish().unwrap().into_inner()
    };
    // The header grows by a byte now and then as the pad's size does, so
    // the pad is found by trying.
    let mut pad = 0usize;
    for _ in 0..8 {
        let len = build(pad).len();
        if len % PART == 3 {
            return build(pad);
        }
        pad += (3 + PART - len % PART) % PART;
    }
    panic!("no pad gives a last part of three bytes");
}

/// Puts the bytes on the fake disk as `base.001`, `base.002` and on, in
/// parts of `part` bytes; gives the first part's place and the count.
fn put_parts(world: &World, base: &str, bytes: &[u8], part: usize) -> (Place, usize) {
    let mut first = None;
    let mut count = 0;
    for (i, chunk) in bytes.chunks(part).enumerate() {
        let place = world.put(&format!("{base}.{:03}", i + 1), chunk);
        first.get_or_insert(place);
        count += 1;
    }
    (first.unwrap(), count)
}

/// Every member read through the generic `read`, held to the original.
fn read_all(world: &World, set: &Place, members: &[(String, Vec<u8>)]) {
    for (name, data) in members {
        let place = world.inside(set, name);
        let mut out = Vec::new();
        world
            .provider
            .read(&place, &OpContext::new())
            .unwrap()
            .read_to_end(&mut out)
            .map_err(guard::from_io)
            .unwrap();
        assert!(out == *data, "{name} differs");
    }
}

#[test]
fn a_split_zip_and_7z_open_as_one_and_read_every_member() {
    let world = World::plain();
    let content = members(25, 1 << 20);
    for (base, bytes) in [
        ("backup.zip", stored_zip(&content)),
        ("backup.7z", copied_7z(&content)),
    ] {
        let (first, count) = put_parts(&world, base, &bytes, PART);
        assert_eq!(count, bytes.len().div_ceil(PART));
        let before: Vec<String> = world.fake.snapshot().into_keys().collect();
        let root = first.clone().into_archive();
        let rows = world.rows(&root).unwrap();
        assert_eq!(
            rows.len(),
            if base.ends_with(".7z") { 3 } else { 2 },
            "{rows:?}"
        );
        let opened = world
            .provider
            .open_archive(&first, &OpContext::new(), Allowances::default())
            .unwrap();
        assert_eq!(opened.parts(), Some(count));
        assert_eq!(
            world.provider.title(&root),
            format!("{base}.001, archive of {count} parts")
        );
        assert!(world.provider.title(&root).chars().count() <= 40);
        read_all(&world, &first, &content);
        let after: Vec<String> = world.fake.snapshot().into_keys().collect();
        assert_eq!(before, after, "{base}: nothing joined, nothing written");
    }
}

#[test]
fn compressed_members_cross_part_boundaries() {
    let world = World::plain();
    let text: Vec<u8> = (0..60_000u32)
        .flat_map(|n| format!("line {n}\n").into_bytes())
        .collect();
    let content = vec![
        ("week1/notes.txt".to_owned(), text.clone()),
        ("week2/noise.bin".to_owned(), noise(300_000, 7)),
    ];
    let as_slices: Vec<(&str, &[u8])> = content
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let zip = zip_of(&as_slices);
    let solid = crate::build::sevenz_of(&as_slices, true, None);
    for (base, bytes) in [("course.zip", zip), ("course.7z", solid)] {
        let (first, count) = put_parts(&world, base, &bytes, 4096);
        assert!(count > 10, "{base}: {count} parts");
        read_all(&world, &first, &content);
        let week1 = world.inside(&first, "week1");
        assert_eq!(world.list(&week1)[0].name, "notes.txt");
    }
}

#[test]
fn a_missing_part_says_which() {
    let world = World::plain();
    let content = members(4, 100_000);
    let bytes = stored_zip(&content);
    let mut first = None;
    for (i, chunk) in bytes.chunks(PART).enumerate() {
        if i == 2 {
            continue;
        }
        let place = world.put(&format!("gappy.zip.{:03}", i + 1), chunk);
        first.get_or_insert(place);
    }
    let first = first.unwrap();
    let error = world
        .provider
        .open_archive(&first, &OpContext::new(), Allowances::default())
        .unwrap_err();
    assert_eq!(
        error.message("open", "gappy.zip.001").text,
        "Could not open gappy.zip.001: it is damaged: part 003 is missing. \
         Get a fresh copy if you can."
    );
}

#[test]
fn a_part_that_changes_opens_again() {
    let world = World::plain();
    let content = members(2, 100_000);
    let bytes = stored_zip(&content);
    let (first, _) = put_parts(&world, "live.zip", &bytes, PART);
    read_all(&world, &first, &content);
    // A part rewritten with other contents of another size: the set is
    // not the one the index was built from, so it is read again, and a
    // read never mixes the two.
    let second = world.root.join("live.zip.002").unwrap();
    world.fake.add_file(&second, &[0u8; 100]);
    let place = world.inside(&first, &content[1].0);
    let mut out = Vec::new();
    let result = world
        .provider
        .read(&place, &OpContext::new())
        .and_then(|mut reader| reader.read_to_end(&mut out).map_err(guard::from_io));
    assert!(
        result.is_err() || out == content[1].1,
        "a changed part gives an error or the right bytes"
    );
}
