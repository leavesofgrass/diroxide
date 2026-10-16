//! Helpers for driving the terminal's session on ratatui's test backend:
//! fixture folders made at test time under the build's temporary folder
//! (plain Unicode names only, emoji and Japanese among them), an app on
//! them with a fixed clock, and steps that settle the runtime and draw.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_app::config::{Layout, Settings};
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::{FixedClock, Key, Place, Timestamp, UtcOffset};
use diroxide_theme::ColorSupport;
use diroxide_tui::{Screen, Session, TuiOptions};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A place for a path.
pub fn place(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

/// A temporary folder for one test.
pub fn temp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("tui-it-")
        .tempdir()
        .unwrap()
}

/// The fixture: plain names, accents, Japanese and an emoji, a `Backup`
/// folder with an older report, a `Photos` folder and an empty folder.
pub fn fixture(root: &Path) -> PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(folder.join("Backup")).unwrap();
    fs::create_dir_all(folder.join("Photos")).unwrap();
    fs::create_dir_all(folder.join("Empty")).unwrap();
    fs::write(folder.join("report.pdf"), vec![b'r'; 2100]).unwrap();
    fs::write(folder.join("notes.txt"), b"notes").unwrap();
    fs::write(folder.join("budget.xlsx"), vec![b'b'; 300]).unwrap();
    fs::write(folder.join("R\u{e9}sum\u{e9}.txt"), b"cv").unwrap();
    fs::write(folder.join("\u{65e5}\u{672c}\u{8a9e}.txt"), b"ja").unwrap();
    fs::write(folder.join("raccoon \u{1f99d}.txt"), b"jimothy").unwrap();
    fs::write(folder.join("Photos").join("crow.jpg"), b"caw").unwrap();
    fs::write(folder.join("Backup").join("report.pdf"), b"old").unwrap();
    folder
}

/// Settings for tests: no settle time, so a key answers a question at
/// once, and the layout asked for.
pub fn settings(layout: Layout) -> Settings {
    Settings {
        dialog_settle_ms: 0,
        layout,
        ..Settings::default()
    }
}

/// An app on two places with these settings and a fixed clock (Friday,
/// October 16, 2026, 9:00 UTC), and a runtime with the local provider.
pub fn app(left: Place, right: Place, settings: Settings) -> (App, Runtime) {
    let mut options = AppOptions::new(left, right);
    options.settings = settings;
    options.clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    (App::new(options), Runtime::with_local())
}

/// A session on a test backend of a size.
pub fn session<'a>(
    app: &'a mut App,
    runtime: &'a mut Runtime,
    width: u16,
    height: u16,
    screen_reader: bool,
) -> Session<'a, TestBackend> {
    let terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let options = TuiOptions {
        screen_reader,
        color: Some(ColorSupport::NoColor),
        ..TuiOptions::default()
    };
    Session::new(app, runtime, terminal, options)
}

/// The test's clock for the session's timers: a start and an offset.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    start: Instant,
    offset: Duration,
}

impl Clock {
    /// A clock at its start.
    pub fn at_start() -> Self {
        Self {
            start: Instant::now(),
            offset: Duration::ZERO,
        }
    }

    /// Now, as the clock says.
    pub fn now(&self) -> Instant {
        self.start + self.offset
    }

    /// Moves the clock on.
    pub fn advance(&mut self, by: Duration) {
        self.offset += by;
    }
}

/// Starts the session, waits for the listings and draws.
pub fn start(s: &mut Session<'_, TestBackend>, clock: &Clock) {
    s.start(clock.now());
    settle(s, clock);
}

/// Waits for the workers, takes their results, and draws.
pub fn settle(s: &mut Session<'_, TestBackend>, clock: &Clock) {
    s.settle(clock.now(), Duration::from_secs(10));
    s.tick(clock.now());
    s.draw(clock.now()).unwrap();
}

/// Presses a key by its name, waits for the workers, and draws.
pub fn press(s: &mut Session<'_, TestBackend>, clock: &Clock, key: &str) {
    s.key(Key::parse(key).unwrap(), clock.now());
    settle(s, clock);
}

/// Types text, one key a character.
pub fn type_text(s: &mut Session<'_, TestBackend>, clock: &Clock, text: &str) {
    for c in text.chars() {
        s.key(
            Key::new(
                diroxide_core::KeyCode::Char(c),
                diroxide_core::Modifiers::NONE,
            ),
            clock.now(),
        );
    }
    settle(s, clock);
}

/// The index of the first line that starts with `start`.
pub fn line_of(screen: &Screen, start: &str) -> usize {
    screen
        .lines
        .iter()
        .position(|l| l.starts_with(start))
        .unwrap_or_else(|| panic!("no line starts with \"{start}\": {:#?}", screen.lines))
}

/// Asserts the cursor in the session's screen and on the test backend.
pub fn cursor_at(s: &Session<'_, TestBackend>, column: usize, line: usize) {
    let screen = s.screen();
    assert_eq!(
        (screen.cursor_column, screen.cursor_line),
        (column, Some(line)),
        "{:#?}",
        screen.lines
    );
    let backend = s.terminal().backend();
    assert!(backend.cursor_visible(), "the cursor is hidden");
    let at = backend.cursor_position();
    assert_eq!((usize::from(at.x), usize::from(at.y)), (column, line));
}

/// Moves the active panel's focus to the row with this name.
pub fn focus(s: &mut Session<'_, TestBackend>, clock: &Clock, name: &str) {
    let side = s.app().active_side();
    assert!(s.app_mut().focus_name(side, name), "{name} is not listed");
    s.draw(clock.now()).unwrap();
}
