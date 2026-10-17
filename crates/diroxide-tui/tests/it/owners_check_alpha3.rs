//! The owner's check of alpha.3 (research/alpha3-plan.md, "The owner's
//! check") as a scripted test on ratatui's test backend, plan commit 42:
//! each step that is wired, with the words the owner is told to expect,
//! and after every frame the hardware cursor checked by spike Q2's rule:
//! visible, where the frame says, and on a row's first cell unless a text
//! field holds it.
//!
//! The sample folder is made in the test's own folder: a few text files,
//! two near-identical versions of one report (the diff crate's
//! fixtures), an invoice whose third line is "total due now", two logs,
//! two Markdown notes, a hidden file, a folder of 2,000 small files, 35
//! small data files so the Space list has more rows than it shows, and a
//! folder named `in` for the watch step.
//!
//! Not here: step 5's stop is raced against a fast local walk, so either
//! "Stopped at" or "Done:" may end it (the app's own test stops a slow
//! place for certain); steps 18 to 22 (the capture, the timeout, the key
//! test, the presets, `diro keys check`) and 23 (`diro watch`) run in
//! W3k's and W3t's own tests; 24 checks the watch's start and stop lines,
//! not an arrival, which takes two seconds of real time to settle; 25
//! needs a network share.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use diroxide_app::commands::ids;
use diroxide_app::config::{Layout, Paths, Settings};
use diroxide_app::{App, AppOptions, Dialog, Runtime};
use diroxide_core::format::within_cells;
use diroxide_core::{
    Command, CommandId, FixedClock, Key, KeyCode, Modifiers, Timestamp, UtcOffset,
};
use ratatui::backend::TestBackend;

use crate::support::{Clock, place, session, temp};

type S<'a> = diroxide_tui::Session<'a, TestBackend>;

/// The diff crate's two near-identical reports.
fn reports() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diff/report")
}

/// The sample folder, as the check names it.
fn sample(root: &Path) -> PathBuf {
    let sample = root.join("sample");
    fs::create_dir_all(sample.join("many")).unwrap();
    fs::create_dir_all(sample.join("in")).unwrap();
    fs::copy(reports().join("report.txt"), sample.join("report.txt")).unwrap();
    fs::copy(reports().join("old.txt"), sample.join("old.txt")).unwrap();
    fs::write(
        sample.join("invoice.txt"),
        "Invoice for Ada Example\nItems: 3\ntotal due now\nThank you\n",
    )
    .unwrap();
    fs::write(sample.join("notes.txt"), "Notes.\n").unwrap();
    fs::write(sample.join(".secret.txt"), "hidden\n").unwrap();
    fs::write(sample.join("big.log"), "a log line\n".repeat(200)).unwrap();
    fs::write(sample.join("small.log"), "short\n").unwrap();
    fs::write(sample.join("notes.md"), "# Notes\n").unwrap();
    fs::write(sample.join("readme.md"), "# Read me\n").unwrap();
    fs::write(
        sample.join("long.txt"),
        format!("{}\n", "a long line of words ".repeat(400)),
    )
    .unwrap();
    for n in 0..35 {
        fs::write(sample.join(format!("data{n:02}.dat")), vec![b'd'; 10 + n]).unwrap();
    }
    for n in 0..2000 {
        fs::write(sample.join("many").join(format!("item{n:04}.txt")), b"x").unwrap();
    }
    sample
}

/// The owner's script: the session, the test's clocks (the session's
/// timers and the app's own), and how much has been said so far.
struct Check<'a> {
    s: S<'a>,
    clock: Clock,
    app_clock: Arc<FixedClock>,
    mark: usize,
}

impl Check<'_> {
    /// Spike Q2's rule after every frame.
    fn cursor(&self) {
        let screen = self.s.screen();
        let backend = self.s.terminal().backend();
        assert!(backend.cursor_visible(), "the cursor is hidden");
        let at = backend.cursor_position();
        let line = screen.cursor_line.expect("the cursor has a line");
        assert_eq!(
            (usize::from(at.x), usize::from(at.y)),
            (screen.cursor_column, line),
            "{:#?}",
            screen.lines
        );
        let typing = self.s.app().dialog().is_some_and(|(_, d)| {
            matches!(
                d,
                Dialog::Edit { in_list: false, .. } | Dialog::Secret { .. } | Dialog::Prompt { .. }
            )
        });
        if !typing {
            assert_eq!(
                screen.cursor_column, 0,
                "the cursor is not on a row's first cell: {:#?}",
                screen.lines
            );
            let blank_allowed = self.s.app().viewer_open() && self.s.app().dialog().is_none();
            assert!(
                blank_allowed || !screen.lines[line].trim().is_empty(),
                "{:#?}",
                screen.lines
            );
        }
    }

    /// The workers settled, the timers given their time, a frame drawn
    /// and the cursor checked; returns what was said since the last step.
    fn settle(&mut self) -> Vec<String> {
        self.clock.advance(Duration::from_millis(50));
        for _ in 0..200 {
            self.s.settle(self.clock.now(), Duration::from_secs(30));
            self.s.tick(self.clock.now());
            if self.s.app().viewer_wants_tick().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            self.clock.advance(Duration::from_millis(50));
        }
        self.s.settle(self.clock.now(), Duration::from_secs(30));
        self.s.draw(self.clock.now()).unwrap();
        self.cursor();
        self.said()
    }

    fn said(&mut self) -> Vec<String> {
        let said = self.s.said()[self.mark..]
            .iter()
            .map(|m| m.text.clone())
            .collect();
        self.mark = self.s.said().len();
        said
    }

    fn key(&mut self, key: &str) -> Vec<String> {
        self.s.key(Key::parse(key).unwrap(), self.clock.now());
        self.settle()
    }

    /// A key carried out without waiting for the workers.
    fn key_now(&mut self, key: &str) {
        self.s.key(Key::parse(key).unwrap(), self.clock.now());
    }

    fn command(&mut self, id: CommandId) -> Vec<String> {
        self.s.command(Command::new(id), self.clock.now());
        self.settle()
    }

    /// Types text one key at a time, a frame and the cursor checked after
    /// each key, and returns what was said meanwhile.
    fn type_slowly(&mut self, text: &str) -> Vec<String> {
        let mut said = Vec::new();
        for c in text.chars() {
            let key = Key::new(KeyCode::Char(c), Modifiers::NONE);
            self.s.key(key, self.clock.now());
            said.extend(self.settle());
        }
        said
    }

    /// Lets the filter's pause pass on the app's clock.
    fn pause(&mut self) -> Vec<String> {
        self.app_clock.advance(1);
        self.clock.advance(Duration::from_millis(500));
        self.s.tick(self.clock.now());
        self.settle()
    }

    fn clear_field(&mut self) {
        for _ in 0..60 {
            self.s
                .key(Key::parse("backspace").unwrap(), self.clock.now());
        }
        self.settle();
    }

    fn focus(&mut self, name: &str) {
        let side = self.s.app().active_side();
        assert!(
            self.s.app_mut().focus_name(side, name),
            "{name} is not listed"
        );
        self.settle();
    }

    fn line(&self, at: usize) -> String {
        self.s.screen().lines[at].clone()
    }

    /// The open list's rows as said.
    fn rows(&self) -> Vec<String> {
        match self.s.app().dialog() {
            Some((_, Dialog::Choose { list, .. })) => {
                list.rows().iter().map(|r| r.spoken()).collect()
            }
            Some((
                _,
                Dialog::Edit {
                    candidates: Some(list),
                    ..
                },
            )) => list.rows().iter().map(|r| r.spoken()).collect(),
            _ => Vec::new(),
        }
    }

    fn title(&self) -> String {
        self.s
            .app()
            .dialog()
            .map(|(_, d)| d.title())
            .unwrap_or_default()
    }
}

fn any_starts(said: &[String], start: &str) -> bool {
    said.iter().any(|l| l.starts_with(start))
}

fn any_contains(said: &[String], part: &str) -> bool {
    said.iter().any(|l| l.contains(part))
}

#[test]
fn the_owners_check_of_alpha_3_runs_with_its_words() {
    let dir = temp();
    let sample = sample(dir.path());
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let app_clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    let mut options = AppOptions::new(place(&sample), place(&sample));
    options.settings = Settings {
        dialog_settle_ms: 0,
        layout: Layout::Single,
        ..Settings::default()
    };
    options.clock = app_clock.clone();
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local().with_paths(paths);
    let mut c = Check {
        s: session(&mut app, &mut runtime, 80, 24, true),
        clock: Clock::at_start(),
        app_clock,
        mark: 0,
    };
    c.s.start(c.clock.now());
    c.settle();

    // 1. The title starts with the folder's name.
    assert!(c.line(0).starts_with("sample, left panel"), "{}", c.line(0));

    // 2. Ctrl+F: a dialog named Search with labeled fields; *.txt and
    // Enter: the start, the end with its count, what was skipped.
    let said = c.key("ctrl+f");
    assert_eq!(said, ["Search, Name or filter: every entry, 2 of 11"]);
    assert_eq!(c.title(), "Search");
    let rows = c.rows();
    for label in [
        "Name or filter:",
        "Contains text:",
        "Look in:",
        "Hidden files:",
    ] {
        assert!(
            rows.iter().any(|r| r.starts_with(label)),
            "{label}: {rows:?}"
        );
    }
    c.type_slowly("*.txt");
    let said = c.key("enter");
    assert_eq!(said[0], "Searching sample for *.txt", "{said:?}");
    assert!(any_starts(&said, "Done: "), "{said:?}");
    // A leading dot hides a file on Unix; on Windows the attribute does.
    if cfg!(unix) {
        assert!(any_contains(&said, "Hidden files skipped."), "{said:?}");
    }

    // 3. Down through the results: the name, then "in" and the folder,
    // the position last.
    let said = c.key("down");
    assert!(said[0].contains(".txt, in "), "{said:?}");
    assert!(
        said[0].ends_with(&format!(" of {}", c.rows().len())),
        "{said:?}"
    );

    // 4. The text "total due" in the contents; F3 at the match.
    c.key("escape");
    c.key("ctrl+f");
    c.key("down");
    c.type_slowly("total due");
    let said = c.key("enter");
    assert!(
        any_starts(&said, "Searching sample for *.txt, containing total due"),
        "{said:?}"
    );
    let rows = c.rows();
    // The reports hold "total due" too, and hits come in the order they
    // are found, which differs between systems: go to the invoice's row.
    let invoice = rows
        .iter()
        .position(|r| r.starts_with("invoice.txt, line 3: total due"))
        .unwrap_or_else(|| panic!("{rows:?}"));
    c.key("home");
    for _ in 0..invoice {
        c.key("down");
    }
    let said = c.key("f3");
    assert!(c.s.app().viewer_open(), "{said:?}");
    assert!(any_starts(&said, "At line 3 of 4"), "{said:?}");
    let said = c.key("escape");
    assert!(
        any_starts(&said, "Back in the results, invoice.txt"),
        "{said:?}"
    );
    c.key("escape");

    // 5. Something that matches thousands of files, and Escape while it
    // runs: the count is said, and the cursor never moved.
    c.key("ctrl+f");
    c.key("down");
    let said = c.key("enter");
    assert_eq!(said, ["Contains text: total due"]);
    c.clear_field();
    c.key_now("enter");
    c.key_now("escape");
    let said = c.settle();
    assert!(
        any_starts(&said, "Stopped at ") || any_starts(&said, "Done: "),
        "{said:?}"
    );
    match c.s.app().dialog() {
        Some((_, Dialog::Choose { list, .. })) => {
            if any_starts(&said, "Done: ") {
                assert!(list.len() > 1000, "{}", list.len());
            }
            assert!(
                matches!(list.focus_index(), None | Some(0)),
                "the focus never moved"
            );
        }
        other => panic!("{other:?}"),
    }
    let said = c.key("escape");
    assert_eq!(said, ["Results closed. Search results opens them again."]);

    // 6. The filter: silence while typing, then one line, and the title
    // reads it back.
    c.key("ctrl+s");
    let said = c.type_slowly("size over 1 KB and name *.log");
    assert!(said.is_empty(), "silent while typing: {said:?}");
    let said = c.pause();
    assert_eq!(said, ["1 match, big.log"]);
    assert!(
        c.line(0)
            .contains("filter: size over 1 KB and name matches *.log"),
        "{}",
        c.line(0)
    );

    // 7. A problem, once, after the pause.
    c.clear_field();
    let said = c.type_slowly("size over");
    assert!(said.is_empty(), "{said:?}");
    assert_eq!(c.pause(), ["Filter problem: size needs a number"]);

    // 8. Say filter spells last week's days (the test clock is Friday,
    // October 16, 2026; weeks start on Sunday).
    c.clear_field();
    c.type_slowly("modified last week");
    let said = c.command(ids::SAY_FILTER);
    assert!(
        said[0].starts_with("modified last week, October 4 to 10"),
        "{said:?}"
    );
    let said = c.key("enter");
    assert!(any_starts(&said, "Filter on, 0 of "), "{said:?}");

    // 9. Escape clears it: "Filter off" and the count.
    let said = c.key("escape");
    assert!(any_starts(&said, "Filter off, "), "{said:?}");

    // 10. Plus and *.md: the count first, starting on No.
    let said = c.key("plus");
    assert_eq!(said, ["Select by pattern: *"]);
    c.key("backspace");
    c.type_slowly("*.md");
    let said = c.key("enter");
    assert_eq!(
        said,
        ["2 entries will be selected. No is the default. y or n"]
    );
    assert_eq!(c.key("y"), ["2 selected"]);

    // 11. A label, its undo, the Labels list, and its items as results.
    c.key("alt+l");
    c.type_slowly("urg");
    let said = c.key("enter");
    assert!(
        any_starts(&said, "Label urgent added to 2 items"),
        "{said:?}"
    );
    let said = c.key("ctrl+z");
    assert!(
        any_starts(&said, "Undid label urgent on 2 items"),
        "{said:?}"
    );
    c.key("alt+l");
    c.type_slowly("urg");
    c.key("enter");
    let said = c.command(ids::LABELS);
    assert!(
        any_starts(&said, "Labels, 6 labels, Urgent, label, 2 items"),
        "{said:?}"
    );
    let focus = c.s.app().dialog().unwrap().1.focus_line();
    assert!(
        focus.starts_with("Urgent, label, 2 items, 0 missing"),
        "{focus}"
    );
    let said = c.key("enter");
    assert!(
        any_starts(&said, "Looking for 2 items labeled urgent"),
        "{said:?}"
    );
    assert!(
        any_starts(&said, "Done: 2 items labeled urgent"),
        "{said:?}"
    );
    c.key("escape");
    c.key("escape");
    c.command(ids::SELECT_NONE);

    // 12. Bookmarks: Add this folder, then the row and Remove this folder.
    let said = c.key("ctrl+b");
    assert!(any_contains(&said, "Add this folder"), "{said:?}");
    let said = c.key("enter");
    assert!(any_starts(&said, "Bookmarked sample"), "{said:?}");
    c.key("ctrl+b");
    let rows = c.rows();
    assert!(rows[0].starts_with("Remove this folder"), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.starts_with("sample, bookmark, in ")),
        "{rows:?}"
    );
    c.key("escape");

    // 13. Save the last search, and run it from the saved searches.
    let said = c.command(ids::SEARCH_SAVE);
    assert!(any_starts(&said, "Save the search as: "), "{said:?}");
    c.clear_field();
    c.type_slowly("Text files");
    let said = c.key("enter");
    assert_eq!(
        said,
        ["Search saved as Text files. Saved searches runs it."]
    );
    let said = c.command(ids::SEARCH_SAVED);
    assert_eq!(
        said,
        ["Saved searches, 1 search, Text files, saved search, runs on open, 1 of 1"]
    );
    let said = c.key("enter");
    assert!(any_starts(&said, "Running Text files: "), "{said:?}");
    assert!(any_starts(&said, "Done: "), "{said:?}");
    c.key("escape");

    // 14. The Space list: the total first, then the rows, the smaller
    // items last.
    let said = c.command(ids::SPACE_LIST);
    assert!(any_starts(&said, "Total "), "{said:?}");
    let total = said.iter().find(|l| l.starts_with("Total ")).unwrap();
    assert!(
        total.contains(" hold ") || total.contains(" holds "),
        "{total}"
    );
    let rows = c.rows();
    assert!(
        rows.iter()
            .any(|r| r.contains(", folder, ") && r.contains(" percent")),
        "{rows:?}"
    );
    assert!(rows.last().unwrap().starts_with("And "), "{rows:?}");
    c.key("escape");

    // 15. The two near-identical files, marked in one panel: the verdict
    // in 40 cells, the counts, the direction; N and P; Enter; Swap.
    c.focus("old.txt");
    c.key("space");
    c.focus("report.txt");
    c.key("space");
    let said = c.command(ids::COMPARE_FILES);
    assert_eq!(said[0], "Comparing old.txt with report.txt");
    assert!(
        said[1].starts_with(
            "5 changes between old.txt and report.txt. Changed 3, added 1, removed 1."
        ),
        "{said:?}"
    );
    assert!(said[1].contains("From old.txt to report.txt."), "{said:?}");
    assert!(c.title().chars().count() <= 40, "{}", c.title());
    let said = c.key("n");
    assert!(said[0].ends_with("2 of 5"), "{said:?}");
    let said = c.key("p");
    assert!(said[0].ends_with("1 of 5"), "{said:?}");
    let said = c.key("enter");
    assert!(any_starts(&said, "Change 1 of 5, line "), "{said:?}");
    assert!(any_contains(&said, "Before, line "), "{said:?}");
    c.key("escape");
    let said = c.key("s");
    assert_eq!(said, ["Swapped. From report.txt to old.txt."]);
    c.key("escape");
    c.command(ids::SELECT_NONE);

    // 16. A file compared with itself.
    c.focus("report.txt");
    let said = c.command(ids::COMPARE_FILES);
    assert_eq!(said[0], "Comparing report.txt here with report.txt there");
    assert!(said[1].starts_with("Same file: report.txt"), "{said:?}");
    c.key("escape");

    // 17. The Keys list, and typing narrows it.
    let said = c.command(ids::KEYS_LIST);
    assert!(
        any_starts(&said, "Keys, preset diroxide defaults, 0 changed"),
        "{said:?}"
    );
    c.type_slowly("copy");
    let rows = c.rows();
    assert!(
        rows.iter()
            .any(|r| r.starts_with("Copy to other panel, F5 in both")),
        "{rows:?}"
    );
    c.key("escape");
    if c.s.app().dialog().is_some() {
        c.key("escape");
    }

    // 24. Watch this folder on the in folder: its start and its stop.
    c.focus("in");
    c.key("enter");
    let said = c.command(ids::WATCH_FOLDER);
    assert_eq!(said, ["Watching in for new files. Escape stops."]);
    let said = c.key("escape");
    assert_eq!(said, ["Stopped watching in"]);

    // Every line said keeps its meaning in its first 40 cells: its first
    // clause, up to a comma, a colon or a period, fits.
    // "Comparing a here with b there" names both files, so a long pair
    // runs past the line; its verb and the first name are inside it.
    let long: Vec<&str> =
        c.s.said()
            .iter()
            .map(|m| m.text.as_str())
            .filter(|text| {
                let end = [", ", ": ", ". "]
                    .iter()
                    .filter_map(|cut| text.find(cut))
                    .min()
                    .unwrap_or(text.len());
                !within_cells(text, &text[..end]) && !text.starts_with("Comparing ")
            })
            .collect();
    assert!(long.is_empty(), "{long:#?}");
    let _ = c.key("backspace");
}
