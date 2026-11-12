//! The flat view on a small local tree: what is listed, the folder after
//! the name, the depth, the mode, hidden entries and ignore files, the
//! batches, the stop and the row cap, and every quoted line with its cell
//! count.

use std::ops::ControlFlow;
use std::path::Path;
use std::time::Duration;

use diroxide_core::format::{BRAILLE_CELLS, within_cells};
use diroxide_core::{Attributes, Place};
use diroxide_vfs::{AttributeChange, LocalProvider, OpContext, Provider, Registry};

use super::*;
use crate::test_support::scratch;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// Marks an entry hidden the Windows way (on Unix the leading dot does).
fn hide(path: &Path) {
    if cfg!(windows) {
        let local = LocalProvider::new();
        let place = Place::parse(&path.to_string_lossy()).unwrap();
        let ctx = OpContext::new();
        let bits = match local.stat(&place, &ctx).unwrap().attributes {
            Some(Attributes::Windows(bits)) => bits,
            _ => 0,
        };
        let change = AttributeChange {
            attributes: Some(Attributes::Windows(bits | 0x2)),
            ..AttributeChange::default()
        };
        local.set_attributes(&place, &change, &ctx).unwrap();
    }
}

/// work/
///   a.txt, .secret.txt (hidden), skip.log (ignored), .gitignore (*.log)
///   2026/notes.txt, 2026/q3/report.pdf (2,100 bytes), 2026/q3/deep/x.txt
struct Tree {
    _dir: tempfile::TempDir,
    root: Place,
}

fn tree() -> Tree {
    let dir = scratch();
    let root = dir.path().join("work");
    write(&root.join("a.txt"), "alpha\n");
    write(&root.join(".secret.txt"), "hidden\n");
    hide(&root.join(".secret.txt"));
    write(&root.join("skip.log"), "log\n");
    write(&root.join(".gitignore"), "*.log\n");
    hide(&root.join(".gitignore"));
    write(&root.join("2026").join("notes.txt"), "notes\n");
    write(
        &root.join("2026").join("q3").join("report.pdf"),
        &"x".repeat(2100),
    );
    write(
        &root.join("2026").join("q3").join("deep").join("x.txt"),
        "x\n",
    );
    let root = Place::parse(&root.to_string_lossy()).unwrap();
    Tree { _dir: dir, root }
}

/// Runs a flat view and keeps every batch.
fn run(root: &Place, options: &FlatOptions) -> (Vec<Vec<FlatEntry>>, FlatSummary) {
    let mut batches = Vec::new();
    let summary = flat_view(
        root,
        options,
        &Registry::with_local(),
        &SearchControl::new(),
        &mut FnFlatSink(|rows| batches.push(rows)),
    )
    .unwrap();
    (batches, summary)
}

fn paths(batches: &[Vec<FlatEntry>]) -> Vec<String> {
    let mut paths: Vec<String> = batches
        .iter()
        .flatten()
        .map(FlatEntry::relative_path)
        .collect();
    paths.sort();
    paths
}

#[test]
fn files_are_listed_with_their_folder_after_the_name() {
    let tree = tree();
    let (batches, summary) = run(&tree.root, &FlatOptions::new());
    assert_eq!(
        paths(&batches),
        [
            "2026/notes.txt",
            "2026/q3/deep/x.txt",
            "2026/q3/report.pdf",
            "a.txt",
            "skip.log"
        ]
    );
    assert_eq!(summary.files, 5);
    assert_eq!(summary.folders, 3);
    assert_eq!(summary.folders_with_files, 4);
    assert!(!summary.cancelled && !summary.capped());
    let report = batches
        .iter()
        .flatten()
        .find(|row| row.entry.name == "report.pdf")
        .unwrap();
    assert_eq!(report.folder, "2026/q3");
    assert_eq!(report.depth, 3);
    let spoken = report.row().spoken();
    assert!(
        spoken.starts_with("report.pdf, in 2026/q3, 2.1 KB"),
        "{spoken}"
    );
    assert!(within_cells(&spoken, "report.pdf, in 2026/q3, 2.1 KB"));
    let top = batches
        .iter()
        .flatten()
        .find(|row| row.entry.name == "a.txt")
        .unwrap();
    assert_eq!(top.folder_words(), "at the top");
    assert!(top.row().spoken().starts_with("a.txt, at the top, 6 bytes"));
    // "Flat view of work: 5 files, every level", 39 cells.
    let line = summary.count_line();
    assert_eq!(line, "Flat view of work: 5 files, every level");
    assert!(line.chars().count() <= BRAILLE_CELLS);
    assert_eq!(summary.on_words(), "Flat view on, 5 files in 4 folders");
    assert_eq!(
        summary.sentence(),
        "Flat view of work: 5 files, every level. Hidden files skipped."
    );
}

#[test]
fn the_depth_limits_the_levels_listed() {
    let tree = tree();
    let options = FlatOptions::new().with_depth(Some(2));
    let (batches, summary) = run(&tree.root, &options);
    assert_eq!(
        paths(&batches),
        ["2026/notes.txt", "a.txt", "skip.log"],
        "depth 2 lists the base folder's files and its folders' files"
    );
    assert_eq!(summary.count_line(), "Flat view of work: 3 files, depth 2");
    assert_eq!(FlatOptions::new().with_depth(Some(0)).depth, None);
}

#[test]
fn folders_are_rows_when_the_mode_says_so() {
    let tree = tree();
    let options = FlatOptions::new().with_folders(true);
    let (batches, summary) = run(&tree.root, &options);
    let listed = paths(&batches);
    assert!(listed.contains(&"2026".to_owned()), "{listed:?}");
    assert!(listed.contains(&"2026/q3/deep".to_owned()), "{listed:?}");
    assert_eq!(listed.len(), 8);
    let q3 = batches
        .iter()
        .flatten()
        .find(|row| row.entry.name == "q3")
        .unwrap();
    assert!(q3.is_folder());
    assert_eq!(q3.row().spoken(), "q3, folder, in 2026");
    assert_eq!(
        summary.count_line(),
        "Flat view of work: 5 files, 3 folders, every level"
    );
    assert_eq!(options.mode_words(), "files and folders");
    assert_eq!(FlatOptions::new().mode_words(), "files only");
}

#[test]
fn hidden_entries_and_ignore_files_follow_the_panel() {
    let tree = tree();
    let (batches, _) = run(&tree.root, &FlatOptions::for_panel(true, false));
    let listed = paths(&batches);
    assert!(listed.contains(&".secret.txt".to_owned()), "{listed:?}");
    assert!(listed.contains(&"skip.log".to_owned()), "{listed:?}");
    let (batches, summary) = run(&tree.root, &FlatOptions::for_panel(false, true));
    let listed = paths(&batches);
    assert!(!listed.contains(&".secret.txt".to_owned()), "{listed:?}");
    assert!(!listed.contains(&"skip.log".to_owned()), "{listed:?}");
    assert!(
        summary
            .skip_sentences()
            .contains(&"Files in ignore files skipped.".to_owned()),
        "{:?}",
        summary.skip_sentences()
    );
}

#[test]
fn rows_come_in_batches_and_the_first_is_small() {
    let dir = scratch();
    let root = dir.path().join("many");
    for n in 0..100 {
        write(&root.join(format!("f{n:03}.txt")), "x");
    }
    let root = Place::parse(&root.to_string_lossy()).unwrap();
    let options = FlatOptions::new().with_batches(40, Duration::from_secs(60));
    let (batches, summary) = run(&root, &options);
    assert_eq!(summary.shown, 100);
    let sizes: Vec<usize> = batches.iter().map(Vec::len).collect();
    assert_eq!(sizes, [FIRST_BATCH, 40, 28], "{sizes:?}");
}

#[test]
fn a_stop_from_the_sink_ends_it_and_says_how_far_it_got() {
    let dir = scratch();
    let root = dir.path().join("Photos");
    for n in 0..200 {
        write(&root.join(format!("p{n:03}.jpg")), "x");
    }
    let root = Place::parse(&root.to_string_lossy()).unwrap();
    struct StopAfterOne(usize);
    impl FlatSink for StopAfterOne {
        fn batch(&mut self, rows: Vec<FlatEntry>) -> ControlFlow<()> {
            self.0 += rows.len();
            ControlFlow::Break(())
        }
    }
    let mut sink = StopAfterOne(0);
    let summary = flat_view(
        &root,
        &FlatOptions::new().with_threads(Some(1)),
        &Registry::with_local(),
        &SearchControl::new(),
        &mut sink,
    )
    .unwrap();
    assert!(summary.cancelled);
    assert_eq!(sink.0, FIRST_BATCH);
    let words = summary.on_words();
    assert!(words.starts_with("Stopped at "), "{words}");
    assert!(words.ends_with(" entries"), "{words}");
    assert_eq!(summary.status_line(), words);
}

#[test]
fn a_cancel_before_the_start_lists_nothing() {
    let tree = tree();
    let control = SearchControl::new();
    control.cancel();
    let mut rows = 0;
    let summary = flat_view(
        &tree.root,
        &FlatOptions::new(),
        &Registry::with_local(),
        &control,
        &mut FnFlatSink(|batch: Vec<FlatEntry>| rows += batch.len()),
    )
    .unwrap();
    assert!(summary.cancelled);
    assert_eq!(rows as u64, summary.shown);
    assert!(summary.on_words().starts_with("Stopped at "));
}

#[test]
fn the_row_cap_stops_the_rows_and_not_the_count() {
    let tree = tree();
    let (batches, summary) = run(&tree.root, &FlatOptions::new().with_row_cap(Some(2)));
    assert_eq!(paths(&batches).len(), 2);
    assert_eq!(summary.files, 5);
    assert!(summary.capped());
    assert_eq!(
        summary.skip_sentences()[0],
        "Showing the first 2. Set a depth to list fewer."
    );
}

#[test]
fn the_words_say_their_meaning_first() {
    let root = Place::parse(if cfg!(windows) { r"D:\work" } else { "/work" }).unwrap();
    let summary = FlatSummary {
        root: root.clone(),
        root_name: root_name(&root),
        options: FlatOptions::new().with_depth(Some(3)),
        files: 1204,
        folders: 400,
        folders_with_files: 310,
        shown: 1204,
        cancelled: false,
        search: SearchSummary::default(),
    };
    // 39 cells.
    assert_eq!(
        summary.count_line(),
        "Flat view of work: 1,204 files, depth 3"
    );
    let photos = FlatSummary {
        root_name: "Photos".to_owned(),
        files: 4200,
        ..summary.clone()
    };
    // 40 cells.
    let on = photos.on_words();
    assert_eq!(on, "Flat view on, 4,200 files in 310 folders");
    assert_eq!(on.chars().count(), BRAILLE_CELLS);
    // 24 cells.
    let stopped = FlatSummary {
        cancelled: true,
        ..photos
    };
    assert_eq!(stopped.on_words(), "Stopped at 4,200 entries");
    // 41 cells; the count is within the first 40.
    let progress = FlatProgress {
        root_name: "Photos".to_owned(),
        files: 4200,
        ..FlatProgress::default()
    };
    assert_eq!(progress.line(), "Flat view of Photos: 4,200 entries so far");
    assert!(within_cells(
        &progress.line(),
        "Flat view of Photos: 4,200 entries"
    ));
    let held = FlatProgress {
        paused: true,
        ..progress
    };
    assert!(within_cells(
        &held.line(),
        "Flat view of Photos held: 4,200"
    ));
    // 23 cells.
    let photos_place = Place::parse(if cfg!(windows) {
        r"D:\Photos"
    } else {
        "/Photos"
    })
    .unwrap();
    assert_eq!(flat_off_words(&photos_place), "Flat view off. Photos.");
    let list = flat_list(&photos_place, &[]);
    assert_eq!(list.title(), "Flat view of Photos");
    assert_eq!(list.empty_text(), "No files below this folder");
}

#[test]
fn the_folder_is_said_from_the_base() {
    let (root, place, outside) = if cfg!(windows) {
        (r"D:\work", r"D:\work\2026\q3\report.pdf", r"E:\other\x.txt")
    } else {
        ("/work", "/work/2026/q3/report.pdf", "/other/x.txt")
    };
    let root = Place::parse(root).unwrap();
    assert_eq!(
        relative_folder(&Place::parse(place).unwrap(), &root),
        "2026/q3"
    );
    assert_eq!(relative_folder(&root.join("a.txt").unwrap(), &root), "");
    let away = relative_folder(&Place::parse(outside).unwrap(), &root);
    assert!(away.contains("other"), "{away}");
}
