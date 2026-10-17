//! The alpha.3 screens on ratatui's test backend (W3a): the filter prompt
//! on the filter language, and the cursor's cell after every frame.

use std::sync::Arc;
use std::time::Duration;

use diroxide_app::config::{Layout, Settings};
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::{FixedClock, Timestamp, UtcOffset};

use crate::support::{
    Clock, cursor_at, fixture, line_of, place, press, session, start, temp, type_text,
};

/// An app on the fixture with a clock the test moves: Friday, October
/// 16, 2026, 9:00 UTC.
fn app_with_clock(work: &std::path::Path, settings: Settings) -> (App, Runtime, Arc<FixedClock>) {
    let clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    let mut options = AppOptions::new(place(work), place(&work.join("Backup")));
    options.settings = settings;
    options.clock = clock.clone();
    (App::new(options), Runtime::with_local(), clock)
}

fn settings() -> Settings {
    Settings {
        dialog_settle_ms: 0,
        layout: Layout::SideBySide,
        ..Settings::default()
    }
}

/// The words said since `from`.
fn said_since(
    s: &diroxide_tui::Session<'_, ratatui::backend::TestBackend>,
    from: usize,
) -> Vec<String> {
    s.said()[from..].iter().map(|m| m.text.clone()).collect()
}

#[test]
fn the_filter_is_silent_while_typing_then_says_the_count_once() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime, app_clock) = app_with_clock(&work, settings());
    let mut clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    press(&mut s, &clock, "ctrl+s");
    let prompt = line_of(&s.screen(), "Filter:");
    cursor_at(&s, "Filter: ".len(), prompt);
    let before = s.said().len();
    type_text(&mut s, &clock, "*.txt");
    assert!(said_since(&s, before).is_empty(), "silent while typing");
    let prompt = line_of(&s.screen(), "Filter: *.txt");
    cursor_at(&s, "Filter: *.txt".len(), prompt);
    // The panel narrowed under the prompt: four text files.
    let screen = s.screen();
    assert!(screen.lines[2].contains(".txt"), "{:#?}", screen.lines);
    // The pause passes on the app's clock; the session's timer says the
    // count once.
    app_clock.advance(1);
    clock.advance(Duration::from_millis(500));
    s.tick(clock.now());
    s.draw(clock.now()).unwrap();
    let said = said_since(&s, before);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("4 matches, first "), "{said:?}");
    let status = line_of(&s.screen(), "4 matches, first ");
    assert!(status > prompt);
    cursor_at(&s, "Filter: *.txt".len(), prompt);
    clock.advance(Duration::from_millis(500));
    s.tick(clock.now());
    assert_eq!(said_since(&s, before).len(), 1, "said once");
    // Enter sets it; the title reads it back and the cursor is on the
    // first row's first cell.
    press(&mut s, &clock, "enter");
    let screen = s.screen();
    assert!(
        screen.lines[0].starts_with("work, left panel, filter: name matches *.txt"),
        "{:#?}",
        screen.lines
    );
    cursor_at(&s, 0, 2);
    assert!(
        said_since(&s, before).contains(&"Filter on, 4 of 9 shown".to_owned()),
        "{:?}",
        said_since(&s, before)
    );
    // Escape with no job takes the filter off.
    press(&mut s, &clock, "escape");
    assert!(said_since(&s, before).contains(&"Filter off, 9 shown".to_owned()));
    assert!(s.screen().lines[0].starts_with("work, left panel, in "));
    cursor_at(&s, 0, 2);
}

#[test]
fn a_filter_problem_is_said_after_the_pause_never_on_each_key() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime, app_clock) = app_with_clock(&work, settings());
    let mut clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    press(&mut s, &clock, "ctrl+s");
    let before = s.said().len();
    type_text(&mut s, &clock, "size over");
    assert!(said_since(&s, before).is_empty());
    app_clock.advance(1);
    clock.advance(Duration::from_millis(500));
    s.tick(clock.now());
    s.draw(clock.now()).unwrap();
    assert_eq!(
        said_since(&s, before),
        ["Filter problem: size needs a number"]
    );
    let prompt = line_of(&s.screen(), "Filter: size over");
    cursor_at(&s, "Filter: size over".len(), prompt);
    app_clock.advance(1);
    clock.advance(Duration::from_millis(500));
    s.tick(clock.now());
    s.draw(clock.now()).unwrap();
    let said = said_since(&s, before);
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[1].starts_with("Word 3. "), "{said:?}");
}

#[test]
fn a_search_lists_its_results_with_the_cursor_on_the_first_row() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime, _app_clock) = app_with_clock(&work, settings());
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    press(&mut s, &clock, "ctrl+f");
    // The dialog covers the body: its name, then the rows, the focus on
    // the name row's first cell.
    let screen = s.screen();
    let title = line_of(&screen, "Search");
    let row = line_of(&screen, "Name or filter: every entry");
    assert!(row > title);
    cursor_at(&s, 0, row);
    type_text(&mut s, &clock, "*.txt");
    let prompt = line_of(&s.screen(), "Name or filter: *.txt");
    cursor_at(&s, "Name or filter: *.txt".len(), prompt);
    let before = s.said().len();
    press(&mut s, &clock, "enter");
    let said = said_since(&s, before);
    assert_eq!(said[0], "Searching work for *.txt", "{said:?}");
    assert!(
        said.iter().any(|t| t.starts_with("Done: 4 found in ")),
        "{said:?}"
    );
    let screen = s.screen();
    let title = line_of(&screen, "Results: name matches *.txt");
    // The first result is the focused row, its first cell under the cursor.
    assert!(
        screen.lines[title + 1].contains(".txt, in work"),
        "{:#?}",
        screen.lines
    );
    cursor_at(&s, 0, title + 1);
    // The key bar names the results' keys.
    assert!(
        screen.lines[23].starts_with("Escape Stop"),
        "{:#?}",
        screen.lines
    );
    press(&mut s, &clock, "down");
    cursor_at(&s, 0, title + 2);
    // The viewer at the match, and back to the same row.
    press(&mut s, &clock, "f3");
    assert!(s.app().viewer_open());
    press(&mut s, &clock, "escape");
    let screen = s.screen();
    let title = line_of(&screen, "Results: name matches *.txt");
    cursor_at(&s, 0, title + 2);
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
    cursor_at(&s, 0, 2);
}

#[test]
fn a_label_word_is_drawn_in_its_labels_color_and_says_its_name() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime, _app_clock) = app_with_clock(
        &work,
        Settings {
            layout: Layout::Single,
            ..settings()
        },
    );
    let clock = Clock::at_start();
    let terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    let options = diroxide_tui::TuiOptions {
        screen_reader: true,
        color: Some(diroxide_theme::ColorSupport::TrueColor),
        ..diroxide_tui::TuiOptions::default()
    };
    let mut s = diroxide_tui::Session::new(&mut app, &mut runtime, terminal, options);
    start(&mut s, &clock);
    crate::support::focus(&mut s, &clock, "notes.txt");
    // Ctrl+1 is Urgent, orange.
    press(&mut s, &clock, "ctrl+1");
    press(&mut s, &clock, "home");
    let screen = s.screen();
    let line = line_of(&screen, "notes.txt, label: urgent");
    let text = &screen.lines[line];
    let word = text.find("urgent").unwrap() as u16;
    let buffer = s.terminal().backend().buffer();
    let label = buffer[(word, line as u16)].clone();
    let name = buffer[(0, line as u16)].clone();
    assert_eq!(label.symbol(), "u");
    assert_ne!(label.fg, name.fg, "the label's word has its own color");
    // The cursor stays on the focused row's first cell, under the title.
    cursor_at(&s, 0, 1);
}
