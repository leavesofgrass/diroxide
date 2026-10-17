//! Tar and gzip reading through the provider (commit 16): the plain tar
//! with random reads, the sequential index of a tar.gz as a cancellable
//! job with a progress line, the forward cursor, the cost question, and
//! the regression list of research/archives.md section 13 for tar and
//! gzip.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_archive::{Allowances, ArchiveError, ArchiveProvider, Limits, member_row};
use diroxide_vfs::{CancelToken, OpContext, ProgressSink, ProgressUpdate, Provider, VfsError};

use crate::build::{gzip, tar_header, tar_of, tar_pad};
use crate::world::World;

fn course() -> Vec<u8> {
    tar_of(&[
        ("week1/", b""),
        ("week1/notes.txt", b"hello"),
        ("week1/more.txt", b"more"),
        ("readme.md", b"# Course"),
    ])
}

#[test]
fn a_plain_tar_lists_and_reads_in_place() {
    let world = World::plain();
    let tar = world.put("course.tar", &course());
    let rows = world.rows(&tar.clone().into_archive()).unwrap();
    assert_eq!(rows, ["week1, folder", "readme.md, file, 8 bytes"]);
    assert_eq!(
        world.read(&world.inside(&tar, "week1/more.txt")).unwrap(),
        b"more"
    );
    assert_eq!(
        world.read(&world.inside(&tar, "week1/notes.txt")).unwrap(),
        b"hello"
    );
    assert!(
        world
            .provider
            .capabilities(&world.inside(&tar, "week1"))
            .random_read
    );
    let entry = world
        .provider
        .stat(&world.inside(&tar, "week1/notes.txt"), &OpContext::new())
        .unwrap();
    assert_eq!(
        entry.modified.map(|t| t.unix_seconds()),
        Some(1_760_000_000)
    );
    assert!(entry.attributes.is_some());
}

#[test]
fn a_tar_gz_lists_and_reads_through_the_forward_cursor() {
    let world = World::plain();
    let tgz = world.put("course.tar.gz", &gzip(&course()));
    let rows = world.rows(&tgz.clone().into_archive()).unwrap();
    assert_eq!(rows, ["week1, folder", "readme.md, file, 8 bytes"]);
    assert!(
        !world
            .provider
            .capabilities(&world.inside(&tgz, "week1"))
            .random_read
    );
    // In order, then back to an earlier member, then a later one.
    for member in [
        "week1/notes.txt",
        "week1/more.txt",
        "readme.md",
        "week1/notes.txt",
    ] {
        let want: &[u8] = match member {
            "week1/notes.txt" => b"hello",
            "week1/more.txt" => b"more",
            _ => b"# Course",
        };
        assert_eq!(
            world.read(&world.inside(&tgz, member)).unwrap(),
            want,
            "{member}"
        );
    }
    // A gzip that holds no tar is one member, its size unknown.
    let gz = world.put("notes.txt.gz", &gzip(b"just one file"));
    let rows = world.rows(&gz.clone().into_archive()).unwrap();
    assert_eq!(rows, ["notes.txt, file"]);
    // `.gz` is no archive extension in a typed place, so the root joins.
    let inside = gz.into_archive().join("notes.txt").unwrap();
    assert_eq!(world.read(&inside).unwrap(), b"just one file");
}

/// A progress sink that keeps every update.
#[derive(Default)]
struct Kept(Mutex<Vec<ProgressUpdate>>);

impl ProgressSink for Kept {
    fn report(&self, update: ProgressUpdate) {
        if let Ok(mut kept) = self.0.lock() {
            kept.push(update);
        }
    }
}

/// A tar of `members` files of `size` bytes of zeros, compressed.
fn zeros_tar_gz(members: usize, size: usize) -> Vec<u8> {
    let zeros = vec![0u8; size];
    let names: Vec<String> = (0..members).map(|i| format!("z{i:03}.bin")).collect();
    let list: Vec<(&str, &[u8])> = names
        .iter()
        .map(|n| (n.as_str(), zeros.as_slice()))
        .collect();
    gzip(&tar_of(&list))
}

#[test]
fn listing_a_tar_gz_reports_its_progress_in_words() {
    let world = World::plain();
    let tgz = world.put("backup.tar.gz", &zeros_tar_gz(12, 1 << 20));
    let kept = Arc::new(Kept::default());
    let ctx = OpContext::new().with_progress(kept.clone());
    let listing = world
        .provider
        .list_folder(&tgz.clone().into_archive(), &ctx, Allowances::all())
        .unwrap();
    assert_eq!(listing.entries.len(), 12);
    let updates = kept.0.lock().unwrap();
    assert!(updates.len() >= 2, "{} updates", updates.len());
    let last = updates.last().unwrap();
    assert_eq!(last.items_done, 12);
    let line =
        diroxide_archive::words::reading_line("backup.tar.gz", last.bytes_done, last.bytes_total);
    assert!(line.starts_with("Reading backup.tar.gz: "), "{line}");
    assert!(line.chars().count() <= 40, "{line}");
}

#[test]
fn listing_a_tar_gz_cancels_in_under_a_second() {
    let world = World::plain();
    // 256 MiB of zeros in 64 members: a long pass, a small file.
    let tgz = world.put("big.tar.gz", &zeros_tar_gz(64, 4 << 20));
    let cancel = CancelToken::new();
    let ctx = OpContext::new().with_cancel(cancel.clone());
    let provider = world.provider.clone();
    let place = tgz.clone().into_archive();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(provider.list_folder(&place, &ctx, Allowances::all()));
    });
    std::thread::sleep(Duration::from_millis(150));
    let asked = Instant::now();
    cancel.cancel();
    let result = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let took = asked.elapsed();
    match result {
        Err(ArchiveError::Vfs(VfsError::Cancelled)) => {
            assert!(took < Duration::from_secs(1), "cancel took {took:?}");
        }
        Ok(listing) => assert_eq!(listing.entries.len(), 64, "it finished first"),
        Err(other) => panic!("{other}"),
    }
}

#[test]
fn reading_far_into_a_tar_gz_asks_first() {
    let world = World::plain();
    let provider = Arc::new(ArchiveProvider::new(Limits::default()).with_cost_question(1 << 20));
    let mut registry = diroxide_vfs::Registry::new();
    registry.set_local(Arc::new(world.fake.clone()));
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    let tgz = world.put("backup.tar.gz", &zeros_tar_gz(4, 1 << 20));
    let last = world.inside(&tgz, "z003.bin");
    let ctx = OpContext::new();
    let asked = provider
        .read_member(
            &last,
            &ctx,
            Allowances {
                cost: false,
                ..Allowances::all()
            },
            None,
        )
        .err()
        .unwrap();
    let ArchiveError::Ask(question) = asked else {
        panic!("{asked:?}");
    };
    assert_eq!(
        question.message().text,
        "Read 3.1 MB first to open z003.bin? y or n"
    );
    let mut reader = provider
        .read_member(&last, &ctx, Allowances::all(), None)
        .unwrap();
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut out).unwrap();
    assert_eq!(out.len(), 1 << 20);
}

#[test]
fn the_regression_list_for_tar_and_gzip() {
    let world = World::plain();
    // A tar with no end marker: listed, with a note.
    let mut open_ended = Vec::new();
    open_ended.extend_from_slice(&tar_header(b"a.txt", 1, b'0'));
    open_ended.push(b'a');
    tar_pad(&mut open_ended);
    let tar = world.put("open.tar", &open_ended);
    let opened = world
        .provider
        .open_archive(&tar, &OpContext::new(), Allowances::default())
        .unwrap();
    assert!(
        opened
            .index()
            .summary()
            .notes
            .iter()
            .any(|n| n.contains("has no end marker"))
    );
    // A header whose checksum is wrong stops the listing there.
    let mut broken = tar_of(&[("a.txt", b"a"), ("b.txt", b"b")]);
    broken[512 * 2 + 148] = b'9';
    let tar = world.put("broken.tar", &broken);
    let opened = world
        .provider
        .open_archive(&tar, &OpContext::new(), Allowances::default())
        .unwrap();
    assert_eq!(opened.index().summary().members, 1);
    assert!(
        opened
            .index()
            .summary()
            .notes
            .iter()
            .any(|n| n.contains("damaged after 1 entries")),
        "{:?}",
        opened.index().summary().notes
    );
    // A gzip of two members reads as one tar; trailing garbage is noted.
    let whole = tar_of(&[("a.txt", b"first"), ("b.txt", b"second")]);
    let (front, back) = whole.split_at(1024);
    let mut two = gzip(front);
    two.extend_from_slice(&gzip(back));
    two.extend_from_slice(b"GARBAGE AFTER THE END");
    let tgz = world.put("two.tar.gz", &two);
    assert_eq!(world.read(&world.inside(&tgz, "b.txt")).unwrap(), b"second");
    // A long GNU name and a symlink then a folder of the same name.
    let long = format!("{}/file.txt", "d".repeat(150));
    let tar = world.put("long.tar", &tar_of(&[(long.as_str(), b"deep")]));
    assert_eq!(world.read(&world.inside(&tar, &long)).unwrap(), b"deep");
    let tar = world.put("trap.tar", &crate::hostile::tar_links_out_of_root());
    let row = &world.rows(&tar.into_archive()).unwrap()[0];
    assert!(row.contains("link and folder share a name"), "{row}");
    // A first header that is not one: not a tar.
    let tar = world.put("not.tar", &vec![7u8; 1024]);
    let error = world.rows(&tar.into_archive()).unwrap_err();
    assert!(error.starts_with("it is damaged"), "{error}");
}

#[test]
fn a_pax_size_wins_and_is_said() {
    let world = World::plain();
    let tar = world.put("pax.tar", &crate::hostile::pax_size_disagrees());
    let entries = world.list(&tar.clone().into_archive());
    assert_eq!(
        member_row(&entries[0]),
        "a.txt, sizes disagree, the PAX size used, file, 5 bytes"
    );
    assert_eq!(world.read(&world.inside(&tar, "a.txt")).unwrap(), b"aaaaa");
    assert_eq!(world.read(&world.inside(&tar, "b.txt")).unwrap(), b"bb");
}
