//! Nested archives (commit 19): the stored-member window, the spool in
//! memory and then in the spool folder, depth four, a fifth level refused
//! in words, and spike S1's check that nothing reaches the platform's
//! temporary folder.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use diroxide_archive::spool::{SPOOL_PREFIX, sweep};
use diroxide_archive::{Allowances, ArchiveProvider, Limits};
use diroxide_vfs::{OpContext, Registry};

use crate::build::{gzip, sevenz_of, tar_of, zip_deflated, zip_of, zip_stored};
use crate::world::World;

/// A world whose provider has these limits and, optionally, a spool
/// folder.
fn world_with(limits: Limits, spool: Option<std::path::PathBuf>) -> World {
    let mut world = World::new(limits);
    let mut provider = ArchiveProvider::new(limits);
    if let Some(dir) = spool {
        provider = provider.with_spool_dir(dir);
    }
    let provider = Arc::new(provider);
    let mut registry = Registry::new();
    registry.set_local(Arc::new(world.fake.clone()));
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    world.provider = provider;
    world.registry = registry;
    world
}

/// Names in a folder that start with the spool prefix.
fn spool_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|items| {
            items
                .filter_map(Result::ok)
                .filter_map(|i| i.file_name().into_string().ok())
                .filter(|n| n.starts_with(SPOOL_PREFIX))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_zip_in_a_zip_opens_stored_as_a_window_and_deflated_through_the_spool() {
    // No memory for a spool and no spool folder: only the window works.
    let limits = Limits {
        max_spool_memory_bytes: 0,
        ..Limits::default()
    };
    let world = world_with(limits, None);
    let inner = zip_of(&[("deep.txt", b"inside the inner zip")]);
    let stored = world.put("stored.zip", &zip_stored("inner.zip", &inner));
    let deep = world.inside(&stored, "inner.zip!deep.txt");
    assert_eq!(world.read(&deep).unwrap(), b"inside the inner zip");
    assert_eq!(world.provider.spool_in_use(), 0);
    let deflated = world.put("deflated.zip", &zip_deflated("inner.zip", &inner));
    let error = world
        .read(&world.inside(&deflated, "inner.zip!deep.txt"))
        .unwrap_err();
    assert_eq!(
        error,
        "it passes the limit for holding an inner archive in memory: 0 bytes, and no spool folder is set"
    );
    // With the default memory spool, the deflated one opens too.
    let world = World::plain();
    let deflated = world.put("deflated.zip", &zip_deflated("inner.zip", &inner));
    let rows = world.rows(&world.inside(&deflated, "inner.zip!")).unwrap();
    assert_eq!(rows, ["deep.txt, file, 20 bytes"]);
    let title = world.provider.title(&world.inside(&deflated, "inner.zip!"));
    assert_eq!(title, "inner.zip, archive, in deflated.zip");
}

#[test]
fn every_format_nests_in_every_other() {
    let world = World::plain();
    let tgz = gzip(&tar_of(&[("logs/a.log", b"log line")]));
    let sevenz = sevenz_of(&[("in7z.txt", b"from the 7z")], false, None);
    let outer = sevenz_of(&[("logs.tar.gz", &tgz), ("inner.7z", &sevenz)], true, None);
    let place = world.put("outer.7z", &outer);
    assert_eq!(
        world
            .read(&world.inside(&place, "logs.tar.gz!logs/a.log"))
            .unwrap(),
        b"log line"
    );
    assert_eq!(
        world
            .read(&world.inside(&place, "inner.7z!in7z.txt"))
            .unwrap(),
        b"from the 7z"
    );
    let zip = world.put("outer.zip", &zip_of(&[("inner.7z", &sevenz)]));
    assert_eq!(
        world
            .read(&world.inside(&zip, "inner.7z!in7z.txt"))
            .unwrap(),
        b"from the 7z"
    );
}

#[test]
fn four_deep_opens_and_a_fifth_level_is_refused_in_words() {
    let world = World::plain();
    let four = world.put("z1.zip", &crate::hostile::nested(4));
    assert_eq!(
        world
            .read(&world.inside(&four, "z2.zip!z3.zip!z4.zip!a.txt"))
            .unwrap(),
        b"deep inside"
    );
    let five = world.put("five.zip", &crate::hostile::nested(5));
    let error = world
        .rows(&world.inside(&five, "z2.zip!z3.zip!z4.zip!z5.zip!"))
        .unwrap_err();
    assert_eq!(error, "it passes the limit for nesting: 4 archives deep");
    let message = diroxide_core::Message::from_error(
        "open",
        "z5.zip",
        &diroxide_vfs::VfsError::LimitReached {
            what: "nesting".to_owned(),
            limit: "4 archives deep".to_owned(),
        },
    );
    assert!(diroxide_core::format::within_cells(
        &message.text,
        "Could not open z5.zip"
    ));
}

#[test]
fn spike_s1_the_spool_folder_holds_the_spool_and_the_temp_folder_nothing() {
    let spool = crate::spike_listing::scratch();
    let temp_before = spool_files(&std::env::temp_dir());
    let limits = Limits {
        max_spool_memory_bytes: 4 << 10,
        ..Limits::default()
    };
    let world = world_with(limits, Some(spool.path().to_path_buf()));
    let inner = gzip(&tar_of(&[("big.bin", &crate::build::noise(64 << 10, 4))]));
    let outer = world.put("outer.zip", &zip_deflated("inner.tar.gz", &inner));
    let member = world.inside(&outer, "inner.tar.gz!big.bin");
    assert_eq!(world.read(&member).unwrap().len(), 64 << 10);
    let held = spool_files(spool.path());
    assert_eq!(held.len(), 1, "{held:?}");
    assert!(
        held[0]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    );
    assert!(world.provider.spool_in_use() > 4 << 10);
    assert_eq!(
        spool_files(&std::env::temp_dir()),
        temp_before,
        "nothing in the temp folder"
    );
    // Leaving the archive lets the spool go.
    world.provider.forget(&outer);
    assert!(
        spool_files(spool.path()).is_empty(),
        "the spool file is removed"
    );
    assert_eq!(world.provider.spool_in_use(), 0);
    // A budget too small for the inner archive refuses in words.
    let small = Limits {
        max_spool_memory_bytes: 4 << 10,
        max_spool_disk_bytes: 16 << 10,
        ..Limits::default()
    };
    let world = world_with(small, Some(spool.path().to_path_buf()));
    let outer = world.put("outer.zip", &zip_deflated("inner.tar.gz", &inner));
    let error = world
        .read(&world.inside(&outer, "inner.tar.gz!big.bin"))
        .unwrap_err();
    assert_eq!(error, "it passes the limit for spool space: 16 KiB");
    assert!(
        spool_files(spool.path()).is_empty(),
        "a refused spool leaves nothing"
    );
    let _ = OpContext::new();
    let _ = Allowances::default();
}

#[test]
fn the_sweep_removes_only_old_spool_files() {
    let dir = crate::spike_listing::scratch();
    let old = dir.path().join(format!("{SPOOL_PREFIX}1-0-0.tmp"));
    let young = dir.path().join(format!("{SPOOL_PREFIX}2-0-0.tmp"));
    let other = dir.path().join("notes.tmp");
    for path in [&old, &young, &other] {
        std::fs::write(path, b"x").unwrap();
    }
    let two_days_ago = SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
    for path in [&old, &other] {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(two_days_ago).unwrap();
    }
    assert_eq!(sweep(dir.path()).unwrap(), 1);
    assert!(!old.exists());
    assert!(young.exists() && other.exists());
}

#[test]
fn stored_zip_members_and_plain_tar_members_seek() {
    use std::io::{Read, Seek, SeekFrom};

    use diroxide_vfs::Provider;
    let world = World::plain();
    let ctx = OpContext::new();
    let zip = world.put("stored.zip", &zip_stored("a.txt", b"0123456789"));
    let mut reader = world
        .provider
        .open_seekable(&world.inside(&zip, "a.txt"), &ctx)
        .unwrap();
    reader.seek(SeekFrom::Start(6)).unwrap();
    let mut tail = String::new();
    reader.read_to_string(&mut tail).unwrap();
    assert_eq!(tail, "6789");
    let tar = world.put("plain.tar", &tar_of(&[("b.txt", b"abcdefghij")]));
    let mut reader = world
        .provider
        .open_seekable(&world.inside(&tar, "b.txt"), &ctx)
        .unwrap();
    reader.seek(SeekFrom::End(-3)).unwrap();
    let mut end = String::new();
    reader.read_to_string(&mut end).unwrap();
    assert_eq!(end, "hij");
    let deflated = world.put("d.zip", &zip_deflated("c.txt", b"compressed"));
    let refused = world
        .provider
        .open_seekable(&world.inside(&deflated, "c.txt"), &ctx)
        .err()
        .unwrap();
    assert_eq!(
        refused.to_string(),
        "the archive provider cannot read a compressed member with random access"
    );
}
