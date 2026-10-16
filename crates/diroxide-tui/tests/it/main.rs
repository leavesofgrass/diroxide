//! Integration tests of the terminal's screen, on ratatui's test backend:
//! the cursor in every state, a key's press and release, the three
//! layouts with hostile, emoji and Japanese names, the inactive panel in
//! screen-reader mode, the repeated message, the key test's timeout, the
//! settled resize, attributes without color, and the editor's handover.

// Test helpers: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode as TermCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use diroxide_app::commands::ids;
use diroxide_app::config::Layout;
use diroxide_app::{App, AppEvent, AppOptions, Handover, Launcher, Program, Runtime};
use diroxide_core::{Command, Effect, Entry, EntryKind, Place};
use diroxide_tui::{TuiOptions, render};
use ratatui::style::Modifier;

use support::{
    Clock, app, cursor_at, fixture, focus, line_of, place, press, session, settings, settle, start,
    temp, type_text,
};

#[test]
fn a_narrow_screen_still_starts_with_the_title() {
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut app = App::new(AppOptions::new(here.clone(), here));
    let mut runtime = Runtime::with_local();
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    let options = TuiOptions {
        width: 40,
        height: 6,
        ..TuiOptions::default()
    };
    let screen = render(&app, &options);
    assert!(screen.lines.iter().all(|l| l.chars().count() <= 40));
    assert!(screen.lines[0].starts_with("diroxide-tui, left panel"));
    assert_eq!(screen.lines.len(), 6);
}

#[test]
fn the_cursor_sits_on_the_focus_in_every_state() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(
        place(&work),
        place(&work.join("Backup")),
        settings(Layout::SideBySide),
    );
    let mut clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    // The panel: the title, the places, then the rows; the first row is
    // the first folder.
    let screen = s.screen();
    assert!(
        screen.lines[0].starts_with("work, left panel, in "),
        "{:#?}",
        screen.lines
    );
    assert!(
        screen.lines[2].starts_with("Backup, folder"),
        "{:#?}",
        screen.lines
    );
    cursor_at(&s, 0, 2);
    press(&mut s, &clock, "down");
    cursor_at(&s, 0, 3);
    // The right panel's rows start at its own first cell.
    press(&mut s, &clock, "tab");
    cursor_at(&s, 40, 2);
    press(&mut s, &clock, "tab");
    // An empty folder: the cursor on the "Empty folder" line.
    focus(&mut s, &clock, "Empty");
    press(&mut s, &clock, "enter");
    assert!(s.screen().lines[2].starts_with("Empty folder "));
    cursor_at(&s, 0, 2);
    press(&mut s, &clock, "backspace");
    // A menu: a list over the body, the cursor on its first item.
    press(&mut s, &clock, "f9");
    let screen = s.screen();
    cursor_at(&s, 0, screen.cursor_line.unwrap());
    assert!(screen.cursor_line.unwrap() >= 2);
    assert_eq!(screen.lines[1], s.app().dialog().unwrap().1.title());
    press(&mut s, &clock, "escape");
    // The palette: the cursor at the field's insertion point.
    press(&mut s, &clock, "ctrl+p");
    let screen = s.screen();
    let prompt = line_of(&screen, "Command palette:");
    cursor_at(&s, "Command palette: ".len(), prompt);
    type_text(&mut s, &clock, "sort");
    let prompt = line_of(&s.screen(), "Command palette: sort");
    cursor_at(&s, "Command palette: sort".len(), prompt);
    // The candidate list: Down moves the cursor into it.
    press(&mut s, &clock, "down");
    let screen = s.screen();
    let first = line_of(&screen, "Sort by");
    cursor_at(&s, 0, first);
    press(&mut s, &clock, "escape");
    // A prompt: rename puts the insertion point before the extension.
    focus(&mut s, &clock, "report.pdf");
    press(&mut s, &clock, "f2");
    let screen = s.screen();
    let prompt = line_of(&screen, "Rename report.pdf to: report.pdf");
    cursor_at(&s, "Rename report.pdf to: report".len(), prompt);
    press(&mut s, &clock, "escape");
    // A confirmation: the cursor on the question, which starts on No.
    press(&mut s, &clock, "f8");
    let screen = s.screen();
    assert!(
        screen.lines[1].starts_with("Move report.pdf to the trash?")
            || screen.lines[1].contains("No is the default"),
        "{:#?}",
        screen.lines
    );
    cursor_at(&s, 0, 1);
    press(&mut s, &clock, "n");
    clock.advance(Duration::from_secs(1));
    // Every frame drawn so far showed the cursor.
    assert!(s.terminal().backend().cursor_visible());
}

#[test]
fn the_cursor_fallback_parks_on_the_status_line_but_a_prompt_keeps_its_place() {
    let dir = temp();
    let work = fixture(dir.path());
    let mut settings = settings(Layout::Stacked);
    settings.cursor_fallback = true;
    let (mut app, mut runtime) = app(place(&work), place(&work), settings);
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    cursor_at(&s, 0, 22);
    press(&mut s, &clock, "f7");
    let prompt = line_of(&s.screen(), "New folder in work:");
    cursor_at(&s, "New folder in work: ".len(), prompt);
}

fn key_event(code: TermCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind,
        state: KeyEventState::NONE,
    }
}

#[test]
fn a_press_and_its_release_run_one_command() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    s.key_event(&key_event(TermCode::Down, KeyEventKind::Press), clock.now());
    s.key_event(
        &key_event(TermCode::Down, KeyEventKind::Release),
        clock.now(),
    );
    settle(&mut s, &clock);
    assert_eq!(s.app().active_panel().list().focus_index(), Some(1));
    s.key_event(
        &key_event(TermCode::Down, KeyEventKind::Repeat),
        clock.now(),
    );
    settle(&mut s, &clock);
    assert_eq!(s.app().active_panel().list().focus_index(), Some(2));
    cursor_at(&s, 0, 3);
}

/// Starts an app whose two listings are these entries, answered without
/// the runtime, so names no test may create on disk can be drawn.
fn listed_with(app: &mut App, entries: &[Entry]) {
    let effects = app.start();
    for effect in effects {
        if let Effect::List { request, place } = effect {
            app.handle(AppEvent::Listed {
                request,
                place,
                result: Ok(entries.to_vec()),
            });
        }
    }
    let _ = app.take_requests();
}

fn hostile_entries() -> Vec<Entry> {
    let mut entries = Vec::new();
    for (name, kind) in [
        ("bad\u{1b}[2Jname.txt", EntryKind::File),
        ("invoice\u{202e}fdp.exe", EntryKind::File),
        ("\u{65e5}\u{672c}\u{8a9e}.txt", EntryKind::File),
        ("raccoon \u{1f99d}.txt", EntryKind::File),
        ("Photos", EntryKind::Folder),
    ] {
        let mut entry = Entry::new(name, kind);
        entry.size = Some(12);
        entries.push(entry);
    }
    entries
}

#[test]
fn each_layout_draws_hostile_emoji_and_japanese_names_cleanly() {
    let dir = temp();
    let here = place(dir.path());
    for layout in [Layout::SideBySide, Layout::Stacked, Layout::Single] {
        for screen_reader in [true, false] {
            let (mut app, mut runtime) = app(here.clone(), here.clone(), settings(layout));
            listed_with(&mut app, &hostile_entries());
            let clock = Clock::at_start();
            let mut s = session(&mut app, &mut runtime, 80, 24, screen_reader);
            s.draw(clock.now()).unwrap();
            let screen = s.screen();
            let all = screen.lines.join("\n");
            assert!(!all.contains('\u{1b}'), "{layout:?}: {all}");
            assert!(!all.contains('\u{202e}'), "{layout:?}: {all}");
            assert!(all.contains("bad?[2Jname.txt"), "{layout:?}: {all}");
            assert!(all.contains("\u{65e5}\u{672c}\u{8a9e}.txt"), "{all}");
            assert!(all.contains("raccoon \u{1f99d}.txt"), "{all}");
            for line in &screen.lines {
                assert!(
                    ratatui::text::Span::raw(line.as_str()).width() <= 80,
                    "{layout:?}: too wide: {line}"
                );
            }
            // The rows with a wide character were sent again, each cell
            // placed on its own (ratatui issue 2651's fallback).
            let wide: Vec<u16> = screen
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains('\u{65e5}') || l.contains('\u{1f99d}'))
                .map(|(i, _)| i as u16)
                .collect();
            assert!(!wide.is_empty());
            for row in &wide {
                assert!(s.resent_rows().contains(row), "{layout:?}: row {row}");
            }
            let cursor = s.terminal().backend().cursor_position();
            assert_eq!(usize::from(cursor.y), screen.cursor_line.unwrap());
            // Nothing changed: nothing is sent again.
            s.app_mut();
            s.draw(clock.now()).unwrap();
            assert!(s.resent_rows().is_empty(), "{layout:?}");
            let photos = if screen_reader {
                "Photos, folder"
            } else {
                "  Photos, folder"
            };
            match layout {
                Layout::SideBySide => assert_eq!(line_of(&screen, photos), 2),
                Layout::Stacked => {
                    assert_eq!(line_of(&screen, photos), 2);
                    let name = dir.path().file_name().unwrap().to_string_lossy();
                    assert!(
                        screen.lines[11].starts_with(name.as_ref()),
                        "{:#?}",
                        screen.lines
                    );
                }
                Layout::Single => assert_eq!(line_of(&screen, photos), 1),
            }
        }
    }
}

#[test]
fn tab_in_the_stacked_layout_rewrites_no_row() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(
        place(&work),
        place(&work.join("Photos")),
        settings(Layout::Stacked),
    );
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    let before = s.screen();
    press(&mut s, &clock, "tab");
    let after = s.screen();
    assert!(
        after.lines[0].starts_with("Photos, right panel"),
        "{:#?}",
        after.lines
    );
    assert!(after.lines[22].starts_with("Right panel: Photos, 1 item"));
    for line in 1..22 {
        assert_eq!(before.lines[line], after.lines[line], "line {line} changed");
    }
    let row = line_of(&after, "crow.jpg");
    cursor_at(&s, 0, row);
}

#[test]
fn a_background_change_never_redraws_the_inactive_panel_for_a_screen_reader() {
    for screen_reader in [true, false] {
        let dir = temp();
        let work = fixture(dir.path());
        let (mut app, mut runtime) = app(
            place(&work),
            place(&work.join("Backup")),
            settings(Layout::SideBySide),
        );
        let clock = Clock::at_start();
        let mut s = session(&mut app, &mut runtime, 80, 24, screen_reader);
        start(&mut s, &clock);
        let before = s.screen();
        focus(&mut s, &clock, "notes.txt");
        press(&mut s, &clock, "f5");
        press(&mut s, &clock, "y");
        assert!(work.join("Backup").join("notes.txt").exists());
        let after = s.screen();
        let right = |lines: &[String]| -> Vec<String> {
            lines[1..20]
                .iter()
                .map(|l| l.chars().skip(40).collect::<String>())
                .collect()
        };
        if screen_reader {
            assert_eq!(right(&before.lines), right(&after.lines));
            // Becoming active is the demand that draws it.
            press(&mut s, &clock, "tab");
            assert!(
                right(&s.screen().lines)
                    .iter()
                    .any(|l| l.starts_with("notes.txt"))
            );
        } else {
            assert!(
                right(&after.lines)
                    .iter()
                    .any(|l| l.starts_with("  notes.txt"))
            );
        }
    }
}

#[test]
fn a_repeated_message_blanks_the_status_line_first() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let mut clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    press(&mut s, &clock, "up");
    assert_eq!(s.screen().lines[22], "Top of the list");
    press(&mut s, &clock, "up");
    assert_eq!(s.screen().lines[22], "");
    clock.advance(Duration::from_millis(149));
    s.tick(clock.now());
    s.draw(clock.now()).unwrap();
    assert_eq!(s.screen().lines[22], "");
    clock.advance(Duration::from_millis(1));
    s.tick(clock.now());
    assert!(s.draw(clock.now()).unwrap());
    assert_eq!(s.screen().lines[22], "Top of the list");
    // A move keeps the line as it is, so a reader says the row alone.
    press(&mut s, &clock, "down");
    assert_eq!(s.screen().lines[22], "Top of the list");
}

#[test]
fn the_key_test_ends_after_ten_seconds_without_a_key() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let mut clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    s.command(Command::new(ids::KEY_TEST), clock.now());
    s.draw(clock.now()).unwrap();
    assert!(s.screen().lines[22].starts_with("Key test: press keys"));
    assert!(s.app().key_test_active());
    assert_eq!(
        s.next_deadline(),
        Some(clock.now() + Duration::from_secs(10))
    );
    clock.advance(Duration::from_secs(6));
    press(&mut s, &clock, "ctrl+f5");
    assert!(s.screen().lines[22].starts_with("Got Control plus F5. Runs Sort by modified."));
    clock.advance(Duration::from_secs(9));
    s.tick(clock.now());
    assert!(s.app().key_test_active());
    clock.advance(Duration::from_secs(1));
    s.tick(clock.now());
    s.draw(clock.now()).unwrap();
    assert!(!s.app().key_test_active());
    assert!(s.screen().lines[22].starts_with("No key for 10 seconds. Key test ended."));
}

#[test]
fn a_resize_is_drawn_once_the_size_is_still() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let mut clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    start(&mut s, &clock);
    s.terminal_mut().backend_mut().resize(100, 30);
    s.resize(clock.now());
    s.app_mut();
    assert!(!s.draw(clock.now()).unwrap());
    clock.advance(Duration::from_millis(150));
    s.tick(clock.now());
    assert!(s.draw(clock.now()).unwrap());
    let screen = s.screen();
    assert_eq!(screen.lines.len(), 30);
    assert_eq!(screen.lines[29], "No jobs");
}

#[test]
fn without_color_the_focus_is_reversed_and_a_mark_is_an_asterisk_in_bold() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, false);
    start(&mut s, &clock);
    focus(&mut s, &clock, "budget.xlsx");
    press(&mut s, &clock, "space");
    let screen = s.screen();
    let marked = line_of(&screen, "* budget.xlsx, selected");
    let focused = screen.cursor_line.unwrap();
    assert_ne!(marked, focused);
    let buffer = s.terminal().backend().buffer();
    let cell = |x: u16, y: usize| buffer[(x, y as u16)].clone();
    assert!(cell(0, focused).modifier.contains(Modifier::REVERSED));
    assert!(cell(0, marked).modifier.contains(Modifier::BOLD));
    for y in 0..24u16 {
        for x in 0..80u16 {
            let c = &buffer[(x, y)];
            assert_eq!(c.fg, ratatui::style::Color::Reset, "a color at {x}, {y}");
            assert_eq!(c.bg, ratatui::style::Color::Reset, "a color at {x}, {y}");
        }
    }
}

#[test]
fn next_theme_draws_the_screen_in_the_next_themes_colors() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let clock = Clock::at_start();
    let terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    let options = TuiOptions {
        screen_reader: true,
        color: Some(diroxide_theme::ColorSupport::TrueColor),
        ..TuiOptions::default()
    };
    let mut s = diroxide_tui::Session::new(&mut app, &mut runtime, terminal, options);
    start(&mut s, &clock);
    let before = s.terminal().backend().buffer()[(0, 0)].clone();
    assert_ne!(before.fg, ratatui::style::Color::Reset);
    s.command(Command::new(ids::NEXT_THEME), clock.now());
    settle(&mut s, &clock);
    assert!(
        s.screen().lines[22].starts_with("Theme Galaxy Light, meets AA"),
        "{}",
        s.screen().lines[22]
    );
    let after = s.terminal().backend().buffer()[(0, 0)].clone();
    assert!(
        before.fg != after.fg || before.bg != after.bg,
        "the title line kept its colors"
    );
    // The words and the attribute stay.
    assert_eq!(before.symbol(), after.symbol());
    assert!(after.modifier.contains(Modifier::BOLD));
}

#[test]
fn the_inline_viewport_holds_the_whole_screen_in_its_rows() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let clock = Clock::at_start();
    let terminal = ratatui::Terminal::with_options(
        ratatui::backend::TestBackend::new(80, 30),
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(12),
        },
    )
    .unwrap();
    let options = TuiOptions {
        screen_reader: true,
        inline: true,
        color: Some(diroxide_theme::ColorSupport::NoColor),
        ..TuiOptions::default()
    };
    let mut s = diroxide_tui::Session::new(&mut app, &mut runtime, terminal, options);
    start(&mut s, &clock);
    let screen = s.screen();
    assert_eq!(screen.lines.len(), 12, "{:#?}", screen.lines);
    assert!(screen.lines[0].starts_with("work, left panel"));
    assert_eq!(screen.lines[11], "No jobs");
    let top = s.terminal_mut().get_frame().area().y;
    let at = s.terminal().backend().cursor_position();
    assert_eq!(usize::from(at.y - top), screen.cursor_line.unwrap());
}

#[derive(Default)]
struct Recorder {
    programs: Mutex<Vec<Program>>,
}

impl Launcher for Recorder {
    fn run(&self, program: &Program) -> Result<Option<i32>, String> {
        self.programs.lock().unwrap().push(program.clone());
        Ok(Some(0))
    }
    fn shell(&self) -> (OsString, Vec<OsString>) {
        (OsString::from("testshell"), vec![OsString::from("-c")])
    }
    fn editor(&self) -> OsString {
        OsString::from("testeditor")
    }
}

#[test]
fn the_editor_gets_the_terminal_and_the_screen_comes_back_whole() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut app, runtime) = app(place(&work), place(&work), settings(Layout::Single));
    let recorder = Arc::new(Recorder::default());
    let mut runtime = runtime.with_launcher(recorder.clone());
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    let moments = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&moments);
    let resumed = s.resumed_flag();
    s.set_terminal_hook(Box::new(move |moment| {
        seen.lock().unwrap().push(moment);
        if moment == Handover::Resume {
            resumed.store(true, Ordering::SeqCst);
        }
    }));
    start(&mut s, &clock);
    focus(&mut s, &clock, "notes.txt");
    press(&mut s, &clock, "f4");
    assert_eq!(
        *moments.lock().unwrap(),
        [Handover::Suspend, Handover::Resume]
    );
    let programs = recorder.programs.lock().unwrap();
    assert_eq!(programs[0].program, OsString::from("testeditor"));
    assert!(Path::new(&programs[0].args[0]).ends_with("notes.txt"));
    assert!(!s.resumed_flag().load(Ordering::SeqCst));
    let screen = s.screen();
    assert!(screen.lines[0].starts_with("work, left panel"));
    assert_eq!(screen.lines[22], "Back in diroxide, work");
}

#[test]
fn candidates_are_a_count_while_typing_and_a_list_on_demand_for_a_screen_reader() {
    let dir = temp();
    let work = fixture(dir.path());
    for screen_reader in [true, false] {
        let (mut app, mut runtime) = app(place(&work), place(&work), settings(Layout::Single));
        let clock = Clock::at_start();
        let mut s = session(&mut app, &mut runtime, 80, 24, screen_reader);
        start(&mut s, &clock);
        press(&mut s, &clock, "ctrl+p");
        type_text(&mut s, &clock, "sort by");
        let screen = s.screen();
        let prompt = line_of(&screen, "Command palette: sort by");
        if screen_reader {
            assert!(
                screen.lines[prompt + 1].ends_with("matches, Down arrow lists them"),
                "{:#?}",
                screen.lines
            );
            assert!(!screen.lines[prompt + 2].starts_with("Sort by"));
        } else {
            assert!(screen.lines[prompt + 1].starts_with("Sort by"));
        }
        press(&mut s, &clock, "down");
        let screen = s.screen();
        let prompt = line_of(&screen, "Command palette: sort by");
        assert!(
            screen.lines[prompt + 1].starts_with("Sort by"),
            "{:#?}",
            screen.lines
        );
        cursor_at(&s, 0, prompt + 1);
        press(&mut s, &clock, "escape");
        press(&mut s, &clock, "escape");
        assert!(s.app().dialog().is_none());
        // Tab completes and says so; a second Tab moves into the list.
        press(&mut s, &clock, "ctrl+p");
        press(&mut s, &clock, "tab");
        let screen = s.screen();
        let prompt = line_of(&screen, "Command palette:");
        cursor_at(&s, "Command palette: ".len(), prompt);
        press(&mut s, &clock, "tab");
        let screen = s.screen();
        let prompt = line_of(&screen, "Command palette:");
        cursor_at(&s, 0, prompt + 1);
    }
}

#[test]
fn the_quiet_screen_waits_for_the_listings_before_its_first_frame() {
    let dir = temp();
    let work = fixture(dir.path());
    let mut settings = settings(Layout::Single);
    settings.quiet_screen = true;
    let (mut app, mut runtime) = app(place(&work), place(&work), settings);
    let clock = Clock::at_start();
    let mut s = session(&mut app, &mut runtime, 80, 24, true);
    s.start(clock.now());
    assert_eq!(
        s.next_deadline(),
        Some(clock.now() + Duration::from_millis(400))
    );
    settle(&mut s, &clock);
    let screen = s.screen();
    assert!(
        screen.lines[1].starts_with("Backup, folder"),
        "{:#?}",
        screen.lines
    );
}
