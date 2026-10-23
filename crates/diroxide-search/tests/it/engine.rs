//! The search engine on the fake place and on a local tree: streaming,
//! cancel, pause, the summary's words, the row cap.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_core::{FixedClock, Place, UtcOffset};
use diroxide_search::{
    FnSink, Scope, SearchControl, SearchHit, SearchProgress, SearchQuery, SearchSink,
    SearchSummary, run_search,
};

use crate::fake::{Fake, WHEN};

fn clock() -> FixedClock {
    FixedClock::new(WHEN.plus_seconds(86_400), UtcOffset::from_hours(0))
}

/// A sink that notes the hits, when the first came, and the progress.
#[derive(Default)]
struct Notes {
    hits: Vec<SearchHit>,
    first: Option<Instant>,
    progress: Vec<SearchProgress>,
    stop_after: Option<usize>,
}

impl SearchSink for Notes {
    fn found(&mut self, hit: SearchHit) -> ControlFlow<()> {
        self.first.get_or_insert_with(Instant::now);
        self.hits.push(hit);
        match self.stop_after {
            Some(n) if self.hits.len() >= n => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        }
    }

    fn progress(&mut self, progress: &SearchProgress) {
        self.progress.push(progress.clone());
    }
}

/// `count` folders under the top, each with a file named `hit-N.txt`.
fn spread(count: usize) -> Arc<Fake> {
    let fake = Fake::new();
    for n in 0..count {
        fake.file(&format!("/d{n:03}/hit-{n:03}.txt"), b"hello\n");
        fake.file(&format!("/d{n:03}/other-{n:03}.md"), b"other\n");
    }
    fake
}

fn search(fake: &Arc<Fake>, query: &SearchQuery, sink: &mut Notes) -> SearchSummary {
    run_search(
        query,
        &fake.registry(),
        &clock(),
        &SearchControl::new(),
        sink,
    )
    .unwrap()
}

#[test]
fn the_first_hit_arrives_before_the_walk_ends() {
    let fake = spread(30);
    fake.set_delay(Duration::from_millis(10));
    let query = SearchQuery::parse(Fake::place("/"), "name hit*")
        .unwrap()
        .with_scope(Scope::new(Fake::place("/")).with_threads(Some(1)));
    let mut notes = Notes::default();
    let started = Instant::now();
    let summary = search(&fake, &query, &mut notes);
    let ended = Instant::now();
    assert_eq!(summary.hits, 30);
    assert_eq!(notes.hits.len(), 30);
    let first = notes.first.unwrap() - started;
    let total = ended - started;
    // Streaming means the first hit comes long before the walk ends (an
    // engine that collected everything first would report it at the end).
    // A loaded machine stretches the first hit more than the whole walk,
    // so the bound is half the walk, not a quarter: 133 ms of 505 ms was
    // seen with two builders compiling beside the check.
    assert!(first * 2 < total, "first hit at {first:?} of {total:?}");
    assert_eq!(summary.status_line(), "Done: 30 found in 31 folders");
    // Folders are hits too when the expression lets them through.
    assert!(notes.hits.iter().all(|hit| hit.depth == 2));
}

#[test]
fn a_cancel_mid_walk_leaves_no_thread_behind() {
    let fake = spread(100);
    fake.set_delay(Duration::from_millis(5));
    let query = SearchQuery::parse(Fake::place("/"), "name hit*")
        .unwrap()
        .with_scope(Scope::new(Fake::place("/")).with_threads(Some(2)));
    let mut notes = Notes {
        stop_after: Some(1),
        ..Notes::default()
    };
    let summary = search(&fake, &query, &mut notes);
    assert!(summary.cancelled);
    assert_eq!(summary.still_reading, 0);
    assert!(
        summary.status_line().starts_with("Stopped at "),
        "{summary:?}"
    );
    assert!(summary.hits < 100);
    let calls = fake.calls();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(fake.calls(), calls, "a thread was still walking");
}

#[test]
fn a_blocked_read_is_left_behind_and_said() {
    let fake = spread(5);
    fake.folder("/slow");
    fake.stall_at("/slow", Duration::from_secs(4));
    let query = SearchQuery::everything(Fake::place("/"))
        .with_scope(Scope::new(Fake::place("/")).with_threads(Some(1)));
    let control = SearchControl::new();
    let stopper = {
        let control = control.clone();
        std::thread::spawn(move || {
            // Long enough for the one walker thread to reach the stalled
            // folder under a loaded machine; still inside the 4 s stall.
            std::thread::sleep(Duration::from_millis(1_500));
            control.cancel();
        })
    };
    let started = Instant::now();
    let summary = run_search(
        &query,
        &fake.registry(),
        &clock(),
        &control,
        &mut Notes::default(),
    )
    .unwrap();
    let took = started.elapsed();
    stopper.join().unwrap();
    assert!(took < Duration::from_millis(3_500), "took {took:?}");
    assert!(summary.cancelled);
    assert_eq!(summary.still_reading, 1);
    assert!(
        summary
            .sentence()
            .ends_with("1 folder is still being read."),
        "{}",
        summary.sentence()
    );
}

#[test]
fn a_pause_holds_the_walk_and_resume_lets_it_go() {
    let fake = spread(40);
    fake.set_delay(Duration::from_millis(5));
    let control = SearchControl::new();
    let query = SearchQuery::parse(Fake::place("/"), "name hit*")
        .unwrap()
        .with_scope(Scope::new(Fake::place("/")).with_threads(Some(2)));
    let registry = fake.registry();
    let runner = {
        let control = control.clone();
        std::thread::spawn(move || {
            run_search(&query, &registry, &clock(), &control, &mut Notes::default()).unwrap()
        })
    };
    std::thread::sleep(Duration::from_millis(30));
    control.pause();
    // The listings already under way finish; then nothing moves.
    std::thread::sleep(Duration::from_millis(150));
    let held = fake.calls();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(fake.calls(), held, "the walk went on while paused");
    control.resume();
    let summary = runner.join().unwrap();
    assert!(!summary.cancelled);
    assert_eq!(summary.hits, 40);
}

#[test]
fn the_summary_says_what_was_skipped() {
    let fake = spread(3);
    fake.file("/.secret.txt", b"x");
    fake.file("/snap/inside.txt", b"x");
    fake.link("/to-d000", "/d000");
    let scope = Scope::new(Fake::place("/")).with_skip_names(vec!["snap".into()]);
    let query = SearchQuery::parse(Fake::place("/"), "ext txt")
        .unwrap()
        .with_scope(scope);
    let mut notes = Notes::default();
    let summary = search(&fake, &query, &mut notes);
    assert_eq!(summary.hits, 3);
    assert_eq!(
        summary.sentence(),
        "3 found. Hidden files skipped. 1 link to a folder not followed. 1 snapshot folder skipped."
    );
    let with_hidden = SearchQuery::parse(Fake::place("/"), "hidden and ext txt").unwrap();
    let summary = search(&fake, &with_hidden, &mut Notes::default());
    assert_eq!(summary.hits, 1);
    assert_eq!(summary.skipped.hidden, 0);
}

#[test]
fn the_row_cap_keeps_counting() {
    let fake = spread(25);
    let scope = Scope::new(Fake::place("/")).with_row_cap(Some(10));
    let query = SearchQuery::parse(Fake::place("/"), "kind file")
        .unwrap()
        .with_scope(scope);
    let mut notes = Notes::default();
    let summary = search(&fake, &query, &mut notes);
    assert_eq!(notes.hits.len(), 10);
    assert_eq!((summary.hits, summary.shown), (50, 10));
    assert!(
        summary
            .sentence()
            .starts_with("50 found. Showing the first 10. Narrow the search."),
        "{}",
        summary.sentence()
    );
}

#[test]
fn progress_is_heard_on_a_slow_place() {
    let fake = spread(20);
    fake.set_delay(Duration::from_millis(15));
    let query = SearchQuery::parse(Fake::place("/"), "name hit*")
        .unwrap()
        .with_scope(Scope::new(Fake::place("/")).with_threads(Some(1)));
    let mut notes = Notes::default();
    search(&fake, &query, &mut notes);
    let last = notes.progress.last().expect("progress was said");
    assert!(last.network);
    assert!(
        last.line().starts_with("Searching over the network: "),
        "{}",
        last.line()
    );
}

#[test]
fn several_starts_and_a_missing_one() {
    let fake = spread(4);
    let scope = Scope::of(vec![
        Fake::place("/d000"),
        Fake::place("/d001"),
        Fake::place("/gone"),
    ]);
    let query = SearchQuery::parse(Fake::place("/"), "name hit*")
        .unwrap()
        .with_scope(scope);
    let summary = search(&fake, &query, &mut Notes::default());
    assert_eq!(summary.hits, 2);
    assert_eq!(summary.unreadable, 1);
    assert!(summary.is_partial());
    let only = SearchQuery::parse(Fake::place("/gone"), "x").unwrap();
    let error = run_search(
        &only,
        &fake.registry(),
        &clock(),
        &SearchControl::new(),
        &mut Notes::default(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "it does not exist");
}

/// A local tree under the test's own target folder.
fn local_tree() -> (tempfile::TempDir, Place) {
    let dir = tempfile::Builder::new()
        .prefix("search-engine-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = dir.path().join("tree");
    for (path, text) in [
        ("a.txt", "a"),
        ("b.md", "b"),
        ("q3/notes.txt", "notes"),
        ("q3/deep/more.txt", "more"),
        (".dot.txt", "dot"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let place = Place::parse(&root.to_string_lossy()).unwrap();
    (dir, place)
}

#[test]
fn a_local_search_runs_on_the_parallel_walker() {
    let (_dir, root) = local_tree();
    let query = SearchQuery::parse(root.clone(), "ext txt and size under 1 KB").unwrap();
    let found = Arc::new(Mutex::new(Vec::new()));
    let summary = {
        let found = Arc::clone(&found);
        run_search(
            &query,
            &diroxide_vfs::Registry::with_local(),
            &clock(),
            &SearchControl::new(),
            &mut FnSink(move |hit: SearchHit| found.lock().unwrap().push(hit)),
        )
        .unwrap()
    };
    let hits = found.lock().unwrap();
    let mut names: Vec<&str> = hits.iter().map(|h| h.entry.name.as_str()).collect();
    names.sort_unstable();
    // On Windows a leading dot does not make a file hidden.
    if cfg!(windows) {
        assert_eq!(names, [".dot.txt", "a.txt", "more.txt", "notes.txt"]);
    } else {
        assert_eq!(names, ["a.txt", "more.txt", "notes.txt"]);
        assert!(summary.sentence().contains("Hidden files skipped."));
    }
    assert_eq!(summary.folders, 3);
    let deep = hits.iter().find(|h| h.entry.name == "more.txt").unwrap();
    assert_eq!(deep.depth, 3);
    assert_eq!(
        deep.row(Some(&root)).spoken(),
        "more.txt, in tree/q3/deep, 4 bytes"
    );
}
