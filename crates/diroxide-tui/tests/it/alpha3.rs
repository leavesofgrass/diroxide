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
