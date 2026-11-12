//! The flat view on a small local tree: what is listed, the key and the
//! folder after the name, the depth, the mode, hidden entries and ignore
//! files, the stop and the row cap, and every quoted line with its cell
//! count.

use std::ops::ControlFlow;
use std::path::Path;

use diroxide_core::format::{BRAILLE_CELLS, within_cells};
use diroxide_core::{Attributes, Place};
use diroxide_vfs::{AttributeChange, LocalProvider, OpContext, Provider, Registry};

use super::*;
use crate::FnSink;
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
///   2026/q3/deep/notes.txt (a name that repeats)
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
    write(
        &root.join("2026").join("q3").join("deep").join("notes.txt"),
        "more\n",
    );
    let root = Place::parse(&root.to_string_lossy()).unwrap();
    Tree { _dir: dir, root }
}

/// Runs a flat view and keeps every hit.
fn run(root: &Place, options: &FlatOptions) -> (Vec<SearchHit>, FlatSummary) {
    let mut hits = Vec::new();
    let summary = flat_view(
        root,
        options,
        &Registry::with_local(),
        &SearchControl::new(),
        &mut FnSink(|hit| hits.push(hit)),
    )
    .unwrap();
    (hits, summary)
}

fn keys(root: &Place, hits: &[SearchHit]) -> Vec<String> {
    let mut keys: Vec<String> = hits.iter().map(|hit| flat_key(hit, root)).collect();
    keys.sort();
    keys
}

#[test]
fn files_are_listed_as_hits_keyed_by_their_path_below_the_base() {
    let tree = tree();
    let (hits, summary) = run(&tree.root, &FlatOptions::new());
    assert_eq!(
        keys(&tree.root, &hits),
        [
            "2026/notes.txt",
            "2026/q3/deep/notes.txt",
            "2026/q3/deep/x.txt",
            "2026/q3/report.pdf",
            "a.txt",
            "skip.log"
        ],
        "a panel reads no ignore files, so the flat view reads none"
    );
    assert_eq!(summary.files, 6);
    assert_eq!(summary.folders, 3);
    assert_eq!(summary.folders_with_files, 4);
    assert!(!summary.cancelled && !summary.capped());
    let list = flat_list(&tree.root, &hits);
    let ids: HashSet<&str> = list.rows().iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids.len(), 6, "two notes.txt rows, two keys");
    let report = hits
        .iter()
        .find(|hit| hit.entry.name == "report.pdf")
        .unwrap();
    // 33 cells: the results lister's words under the base.
    let spoken = flat_row(report, &tree.root).spoken();
    assert_eq!(spoken, "report.pdf, in work/2026/q3, 2.1 KB");
    assert!(spoken.chars().count() <= BRAILLE_CELLS);
    // 39 cells.
    let line = summary.count_line();
    assert_eq!(line, "Flat view of work: 6 files, every level");
    assert!(line.chars().count() <= BRAILLE_CELLS);
    assert_eq!(summary.on_words(), "Flat view on, 6 files in 4 folders");
    assert_eq!(
        summary.sentence(),
        "Flat view of work: 6 files, every level. Hidden files skipped."
    );
}

#[test]
fn the_depth_limits_the_levels_listed() {
    let tree = tree();
    let options = FlatOptions::new().with_depth(Some(2));
    let (hits, summary) = run(&tree.root, &options);
    assert_eq!(
        keys(&tree.root, &hits),
        ["2026/notes.txt", "a.txt", "skip.log"],
        "depth 2 lists the base folder's files and its folders' files"
    );
    assert_eq!(summary.count_line(), "Flat view of work: 3 files, depth 2");
    assert_eq!(FlatOptions::new().with_depth(Some(0)).depth, None);
    assert_eq!(flat_query(&tree.root, &options).scope.max_depth, Some(2));
    assert!(flat_query(&tree.root, &options).filter.is_none());
}

#[test]
fn folders_are_rows_when_the_mode_says_so() {
    let tree = tree();
    let options = FlatOptions::new().with_folders(true);
    let (hits, summary) = run(&tree.root, &options);
    let listed = keys(&tree.root, &hits);
    assert!(listed.contains(&"2026".to_owned()), "{listed:?}");
    assert!(listed.contains(&"2026/q3/deep".to_owned()), "{listed:?}");
    assert_eq!(listed.len(), 9);
    let q3 = hits.iter().find(|hit| hit.entry.name == "q3").unwrap();
    assert_eq!(
        flat_row(q3, &tree.root).spoken(),
        "q3, folder, in work/2026"
    );
    assert_eq!(
        summary.count_line(),
        "Flat view of work: 6 files, 3 folders, every level"
    );
    assert_eq!(options.mode_words(), "files and folders");
    assert_eq!(FlatOptions::new().mode_words(), "files only");
}

#[test]
fn hidden_entries_follow_the_panel_and_ignore_files_are_asked_for() {
    let tree = tree();
    let (hits, _) = run(&tree.root, &FlatOptions::for_panel(true));
    let listed = keys(&tree.root, &hits);
    assert!(listed.contains(&".secret.txt".to_owned()), "{listed:?}");
    let options = FlatOptions::for_panel(false).with_ignore_files(true);
    let (hits, summary) = run(&tree.root, &options);
    let listed = keys(&tree.root, &hits);
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
fn a_stop_from_the_sink_ends_it_and_says_how_far_it_got() {
    let dir = scratch();
    let root = dir.path().join("Photos");
    for n in 0..200 {
        write(&root.join(format!("p{n:03}.jpg")), "x");
    }
    let root = Place::parse(&root.to_string_lossy()).unwrap();
    struct StopAt(u64);
    impl SearchSink for StopAt {
        fn found(&mut self, _hit: SearchHit) -> ControlFlow<()> {
            self.0 += 1;
            if self.0 >= 10 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }
    let mut sink = StopAt(0);
    let summary = flat_view(
        &root,
        &FlatOptions::new().with_threads(Some(1)),
        &Registry::with_local(),
        &SearchControl::new(),
        &mut sink,
    )
    .unwrap();
    assert!(summary.cancelled);
    assert_eq!(sink.0, 10);
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
        &mut FnSink(|_| rows += 1),
    )
    .unwrap();
    assert!(summary.cancelled);
    assert_eq!(rows, summary.shown);
    assert!(summary.on_words().starts_with("Stopped at "));
}

#[test]
fn the_row_cap_stops_the_rows_and_not_the_count() {
    let tree = tree();
    let (hits, summary) = run(&tree.root, &FlatOptions::new().with_row_cap(Some(2)));
    assert_eq!(hits.len(), 2);
    assert_eq!(summary.files, 6);
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
    // 35 cells: the catalog's form.
    let small = FlatSummary {
        files: 14,
        folders_with_files: 4,
        ..summary.clone()
    };
    assert_eq!(small.on_words(), "Flat view on, 14 files in 4 folders");
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
    let photos_place = Place::parse(if cfg!(windows) {
        r"D:\Photos"
    } else {
        "/Photos"
    })
    .unwrap();
    let progress = SearchProgress {
        found: 4200,
        ..SearchProgress::default()
    };
    let line = flat_progress_line(&photos_place, &progress);
    assert_eq!(line, "Flat view of Photos: 4,200 entries so far");
    assert!(within_cells(&line, "Flat view of Photos: 4,200 entries"));
    let held = SearchProgress {
        paused: true,
        ..progress
    };
    assert!(within_cells(
        &flat_progress_line(&photos_place, &held),
        "Flat view of Photos held: 4,200"
    ));
    // 23 cells.
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
