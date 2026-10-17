//! Dirsize and "size every folder": the walk helper's sum with hidden and
//! ignored files counted, hard links, the incomplete words, cloud files,
//! and a stop.

use std::ops::ControlFlow;

use diroxide_core::Place;
use diroxide_search::{DirSize, SearchControl, SearchError, dirsize_with, size_every_folder};
use diroxide_vfs::walk::{WalkOptions, walk};
use diroxide_vfs::{LocalProvider, OpContext, Registry};

use crate::fake::Fake;

fn local(files: &[(&str, &str)]) -> (tempfile::TempDir, Place) {
    let dir = tempfile::Builder::new()
        .prefix("search-dirsize-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = dir.path().join("sized");
    std::fs::create_dir(&root).unwrap();
    for (path, text) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    (dir, Place::parse(&root.to_string_lossy()).unwrap())
}

#[test]
fn dirsize_equals_the_walk_helpers_sum_with_hidden_and_ignored_files() {
    let (_dir, root) = local(&[
        ("a.txt", "alpha"),
        (".hidden.txt", "hidden file"),
        (".gitignore", "*.log\nbuild/\n"),
        ("notes.log", "ignored by the ignore file"),
        ("build/out.o", "ignored folder"),
        (".dotdir/inside.txt", "in a hidden folder"),
        ("sub/deeper/b.md", "beta"),
    ]);
    let size = dirsize_with(&root, &Registry::with_local(), &SearchControl::new()).unwrap();
    let summary = walk(
        &LocalProvider::new(),
        &root,
        &WalkOptions::default(),
        &OpContext::new(),
        &mut |_| ControlFlow::Continue(()),
    )
    .unwrap();
    assert_eq!(size.bytes, summary.bytes);
    assert_eq!(size.files, summary.files);
    assert_eq!(size.folders, summary.folders - 1);
    assert_eq!(size.files, 7);
    assert!(size.is_complete());
    assert_eq!(size.sentence(), size.describe());
    assert!(
        size.sentence().ends_with("in 7 files and 4 folders"),
        "{}",
        size.sentence()
    );
}

#[test]
fn hard_links_are_counted_once_on_unix() {
    let (dir, root) = local(&[("a.bin", &"x".repeat(1000))]);
    let path = dir.path().join("sized");
    if std::fs::hard_link(path.join("a.bin"), path.join("b.bin")).is_err() {
        eprintln!("Hard links could not be made here: test skipped.");
        return;
    }
    let size = dirsize_with(&root, &Registry::with_local(), &SearchControl::new()).unwrap();
    if cfg!(unix) {
        assert_eq!((size.bytes, size.files, size.repeated_links), (1000, 1, 1));
        assert_eq!(size.hard_link_words(), "Counted hard links once.");
    } else {
        assert_eq!((size.bytes, size.files), (2000, 2));
        assert_eq!(
            size.hard_link_words(),
            "Hard links may be counted more than once."
        );
    }
}

#[test]
fn an_unreadable_folder_makes_the_size_at_least() {
    let fake = Fake::new();
    fake.file("/top/a.txt", b"123456");
    fake.refuse("/top/locked");
    fake.link("/top/to-a", "/top/a.txt");
    let size = dirsize_with(
        &Fake::place("/top"),
        &fake.registry(),
        &SearchControl::new(),
    )
    .unwrap();
    assert_eq!((size.bytes, size.files, size.links), (6, 1, 1));
    assert!(!size.is_complete());
    assert_eq!(size.sentence(), "At least 6 bytes, 1 folder unreadable");
}

#[test]
fn cloud_files_give_the_total_and_what_is_here() {
    let fake = Fake::new();
    fake.file("/cloud/here.txt", &[b'h'; 10]);
    fake.file("/cloud/away.txt", &[b'a'; 30]);
    fake.online_only("/cloud/away.txt");
    let size = dirsize_with(
        &Fake::place("/cloud"),
        &fake.registry(),
        &SearchControl::new(),
    )
    .unwrap();
    assert_eq!(
        size.cloud_words().unwrap(),
        "Total 40 bytes, 10 bytes on this computer"
    );
}

#[test]
fn a_stop_says_stopped_and_a_missing_start_is_an_error() {
    let fake = Fake::new();
    fake.file("/f/a.txt", b"a");
    let control = SearchControl::new();
    control.cancel();
    let size = dirsize_with(&Fake::place("/f"), &fake.registry(), &control).unwrap();
    assert!(size.cancelled);
    assert_eq!(size.sentence(), "At least 0 bytes, stopped");
    let error = dirsize_with(
        &Fake::place("/gone"),
        &fake.registry(),
        &SearchControl::new(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "it does not exist");
}

#[test]
fn size_every_folder_tells_each_and_keeps_the_order() {
    let fake = Fake::new();
    for (n, bytes) in [(1usize, 10usize), (2, 20), (3, 30), (4, 40), (5, 50)] {
        fake.file(&format!("/panel/f{n}/data.bin"), &vec![0u8; bytes]);
    }
    let folders: Vec<Place> = (1..=5)
        .map(|n| Fake::place(&format!("/panel/f{n}")))
        .collect();
    let mut told = Vec::new();
    let results = size_every_folder(
        &folders,
        &fake.registry(),
        &SearchControl::new(),
        &mut |place: &Place, size: &Result<DirSize, SearchError>| {
            told.push((place.clone(), size.as_ref().map(|s| s.bytes).ok()));
        },
    );
    assert_eq!(told.len(), 5);
    let sizes: Vec<u64> = results
        .iter()
        .map(|(_, size)| size.as_ref().unwrap().bytes)
        .collect();
    assert_eq!(sizes, [10, 20, 30, 40, 50]);
    assert_eq!(results[2].0, folders[2]);
}
