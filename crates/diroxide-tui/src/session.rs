//! The terminal's loop, one step at a time, over any ratatui backend: the
//! real terminal's loop calls these steps as events arrive, and the tests
//! call them on ratatui's test backend with times of their own.
//!
//! What a step does beyond handing the key to the app:
//!
//! - Releases are dropped, so a press and its release run one command.
//! - A message equal to the one on the status line blanks the line for
//!   150 milliseconds first, so a reader that speaks only changes says it
//!   again (textweaver's way; "At the top already" pressed twice).
//! - The key test ends by itself after ten seconds without a key, so a key
//!   the terminal never sends cannot trap the user.
//! - A resize is drawn once the size has been still for 150 milliseconds.
//! - With the quiet screen setting on, the first frame waits until the
//!   first listings arrived (at most 400 milliseconds), so the start is
//!   drawn once.
//! - After a program had the terminal (the editor, a shell, a command),
//!   the whole screen is drawn again.
//! - After a frame, a changed row that holds a character two cells wide
//!   is sent again from that character to the row's end, each cell placed
//!   on its own (ratatui issue 2651: no release after 0.30.2 carries the
//!   fix, checked Friday, October 16, 2026).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use diroxide_app::config::SecretEcho;
use diroxide_app::panel::place_title;
use diroxide_app::{App, AppEvent, Dialog, Runtime};
use diroxide_core::{Effect, Key, KeyCode, Message, Modifiers, Side};
use diroxide_theme::ColorSupport;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::buffer::Cell;
use ratatui::layout::Position;

use crate::style::Styles;
use crate::view::{self, Look, Memory};
use crate::{Screen, Theme, TuiError, TuiOptions, keys};

/// How long a repeated message's line stays blank before it is drawn.
pub const BLANK: Duration = Duration::from_millis(150);
/// How long the key test waits for a key before it ends.
pub const KEY_TEST_IDLE: Duration = Duration::from_secs(10);
/// How long the size must stay still before a resized screen is drawn.
pub const RESIZE_SETTLE: Duration = Duration::from_millis(150);
/// The longest the quiet screen's first frame waits for the listings.
pub const QUIET_START: Duration = Duration::from_millis(400);

/// A backend's error in the terminal's words.
pub(crate) fn backend_error(error: impl std::fmt::Display) -> TuiError {
    TuiError::Io(std::io::Error::other(error.to_string()))
}

/// Sets the terminal's title; the real terminal's loop gives one.
pub type TitleSink = Box<dyn FnMut(&str)>;

/// One running screen: the app, its runtime and a terminal.
pub struct Session<'a, B: Backend> {
    app: &'a mut App,
    runtime: &'a mut Runtime,
    terminal: Terminal<B>,
    options: TuiOptions,
    themes_dir: Option<PathBuf>,
    styles: Styles,
    memory: Memory,
    redraw: bool,
    resumed: Arc<AtomicBool>,
    blank_until: Option<Instant>,
    shown_status: String,
    key_test_since: Option<Instant>,
    resize_until: Option<Instant>,
    first_draw_at: Option<Instant>,
    lines: Vec<String>,
    cursor: (u16, u16),
    resent: Vec<u16>,
    title: String,
    title_sink: Option<TitleSink>,
    said: Vec<Message>,
    quit: bool,
}

impl<B: Backend> std::fmt::Debug for Session<'_, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("lines", &self.lines)
            .field("cursor", &self.cursor)
            .field("quit", &self.quit)
            .finish_non_exhaustive()
    }
}

impl<'a, B: Backend> Session<'a, B> {
    /// A session drawing the app on a terminal. The app's screen-reader
    /// behavior is set from the options.
    pub fn new(
        app: &'a mut App,
        runtime: &'a mut Runtime,
        terminal: Terminal<B>,
        options: TuiOptions,
    ) -> Self {
        app.set_screen_reader(options.screen_reader);
        let support = options.color.unwrap_or_else(ColorSupport::detect);
        let styles = Styles::new(&app.settings().theme, &app.settings().colors, None, support);
        Self {
            app,
            runtime,
            terminal,
            options,
            themes_dir: None,
            styles,
            memory: Memory::default(),
            redraw: true,
            resumed: Arc::new(AtomicBool::new(false)),
            blank_until: None,
            shown_status: String::new(),
            key_test_since: None,
            resize_until: None,
            first_draw_at: None,
            lines: Vec::new(),
            cursor: (0, 0),
            resent: Vec::new(),
            title: String::new(),
            title_sink: None,
            said: Vec::new(),
            quit: false,
        }
    }

    /// The same session with the owner's themes folder, for theme files.
    pub fn with_themes_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.themes_dir = dir;
        let settings = self.app.settings();
        self.styles = Styles::new(
            &settings.theme,
            &settings.colors,
            self.themes_dir.as_deref(),
            self.styles.support(),
        );
        self
    }

    /// The same session setting the terminal's title to the active place
    /// whenever it changes.
    pub fn with_title_sink(mut self, sink: TitleSink) -> Self {
        self.title_sink = Some(sink);
        self
    }

    /// The flag a terminal hook sets when it takes the terminal back from
    /// a program, so the next frame draws everything again.
    pub fn resumed_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.resumed)
    }

    /// Sets what happens around a program that takes the terminal (the
    /// editor, a shell, a command): the runtime calls it before and after.
    pub fn set_terminal_hook(&mut self, hook: diroxide_app::runtime::TerminalHook) {
        self.runtime.set_terminal_hook(hook);
    }

    /// The app.
    pub fn app(&self) -> &App {
        self.app
    }

    /// The app, for a test's hooks (the server's and the window's ways in,
    /// such as [`App::focus_name`]).
    pub fn app_mut(&mut self) -> &mut App {
        self.redraw = true;
        self.app
    }

    /// Runs a command, as the palette or a menu does, and carries out its
    /// effects.
    pub fn command(&mut self, command: diroxide_core::Command, now: Instant) {
        let effects = self.app.handle(AppEvent::Command(command));
        self.carry(effects, now);
    }

    /// Hands the app an event from outside the terminal (a test's
    /// listing, a server's request) and carries out its effects.
    pub fn event(&mut self, event: AppEvent, now: Instant) {
        let effects = self.app.handle(event);
        self.carry(effects, now);
    }

    /// The terminal.
    pub fn terminal(&self) -> &Terminal<B> {
        &self.terminal
    }

    /// The terminal, to resize a test backend.
    pub fn terminal_mut(&mut self) -> &mut Terminal<B> {
        &mut self.terminal
    }

    /// Everything said so far, in order.
    pub fn said(&self) -> &[Message] {
        &self.said
    }

    /// The rows the last frame sent again for a wide character.
    pub fn resent_rows(&self) -> &[u16] {
        &self.resent
    }

    /// Whether the app quit.
    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    /// The last frame as text, with the cursor.
    pub fn screen(&self) -> Screen {
        Screen {
            lines: self.lines.clone(),
            cursor_line: Some(usize::from(self.cursor.1)),
            cursor_column: usize::from(self.cursor.0),
        }
    }

    /// Starts the app: the startup lines and both listings. With the quiet
    /// screen setting on, the first frame waits for the listings.
    pub fn start(&mut self, now: Instant) {
        if self.app.settings().quiet_screen {
            self.first_draw_at = Some(now + QUIET_START);
        }
        let effects = self.app.start();
        self.carry(effects, now);
    }

    fn carry(&mut self, effects: Vec<Effect>, now: Instant) {
        let screen = self.runtime.carry_out(self.app, effects);
        if self.resumed.swap(false, Ordering::SeqCst) {
            let _ = self.terminal.clear();
            self.memory.thaw_all();
            self.lines.clear();
            self.redraw = true;
        }
        for effect in screen {
            match effect {
                Effect::Announce(message) => {
                    if !message.text.is_empty()
                        && message.text == self.shown_status
                        && self.app.status_line() == message.text
                    {
                        self.blank_until = Some(now + BLANK);
                    }
                    if message.text.starts_with("Other panel: ") {
                        // Say Other Panel is the demand that redraws the
                        // other panel in screen-reader mode.
                        self.memory.thaw(self.app.active_side().other());
                    }
                    self.said.push(message);
                    self.redraw = true;
                }
                Effect::Quit => self.quit = true,
                _ => self.redraw = true,
            }
        }
        if self.app.wants_quit() {
            self.quit = true;
        }
        match (self.app.key_test_active(), self.key_test_since) {
            (true, None) => self.key_test_since = Some(now),
            (false, Some(_)) => self.key_test_since = None,
            _ => {}
        }
    }

    /// A key from the terminal: a press or a repeat goes to the app; a
    /// release is dropped.
    pub fn key_event(&mut self, event: &KeyEvent, now: Instant) {
        if !keys::acts(event) {
            return;
        }
        if let Some(key) = keys::to_key(event) {
            self.key(key, now);
        }
    }

    /// A key, as the app reads it.
    pub fn key(&mut self, key: Key, now: Instant) {
        if self.key_test_since.is_some() {
            self.key_test_since = Some(now);
        }
        let effects = self.app.handle(AppEvent::Key(key));
        self.carry(effects, now);
    }

    /// Text the terminal pasted: typed into an open text field, one
    /// character at a time up to the first line break; anywhere else it is
    /// refused in words, since a paste in a panel would run its letters as
    /// keys.
    pub fn paste(&mut self, text: &str, now: Instant) {
        let typing = self
            .app
            .dialog()
            .is_some_and(|(_, d)| matches!(d, Dialog::Edit { .. } | Dialog::Secret { .. }));
        if !typing {
            let message =
                Message::info("Text paste ignored here. Paste files with Paste in the Edit menu.");
            let effects = self.app.handle(AppEvent::Problem(message));
            self.carry(effects, now);
            return;
        }
        for c in text.chars() {
            if c == '\r' || c == '\n' {
                break;
            }
            if c.is_control() {
                continue;
            }
            self.key(Key::new(KeyCode::Char(c), Modifiers::NONE), now);
        }
    }

    /// The terminal changed size: drawn again once the size is still.
    pub fn resize(&mut self, now: Instant) {
        self.resize_until = Some(now + RESIZE_SETTLE);
    }

    /// Takes the runtime's waiting results to the app.
    pub fn wake(&mut self, now: Instant) {
        loop {
            let events = self.runtime.poll();
            if events.is_empty() {
                break;
            }
            for event in events {
                let effects = self.app.handle(event);
                self.carry(effects, now);
            }
        }
    }

    /// Waits for the workers to finish, at most `limit`, taking their
    /// results to the app; for tests and the end of the loop.
    pub fn settle(&mut self, now: Instant, limit: Duration) {
        let deadline = Instant::now() + limit;
        loop {
            self.wake(now);
            if !self.runtime.is_busy() || Instant::now() >= deadline {
                break;
            }
            let events = self.runtime.wait(Duration::from_millis(10));
            for event in events {
                let effects = self.app.handle(event);
                self.carry(effects, now);
            }
        }
        self.wake(now);
    }

    /// The timers: the blank line's end, the key test's ten seconds, the
    /// settled resize, the quiet start.
    pub fn tick(&mut self, now: Instant) {
        if self.blank_until.is_some_and(|until| now >= until) {
            self.blank_until = None;
            self.redraw = true;
        }
        if let Some(since) = self.key_test_since
            && now.duration_since(since) >= KEY_TEST_IDLE
        {
            self.key_test_since = None;
            let effects = self.app.end_key_test(KEY_TEST_IDLE.as_secs());
            self.carry(effects, now);
        }
        if self.resize_until.is_some_and(|until| now >= until) {
            self.resize_until = None;
            let _ = self.terminal.autoresize();
            let _ = self.terminal.clear();
            self.memory.thaw_all();
            self.lines.clear();
            self.redraw = true;
        }
        if self.first_draw_at.is_some_and(|at| now >= at) {
            self.first_draw_at = None;
            self.redraw = true;
        }
    }

    /// The next moment a timer needs the loop.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.blank_until,
            self.key_test_since.map(|since| since + KEY_TEST_IDLE),
            self.resize_until,
            self.first_draw_at,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn look(&self, width: u16, height: u16, now: Instant) -> Look {
        let settings = self.app.settings();
        Look {
            width,
            height,
            screen_reader: self.app.screen_reader(),
            borders: self.options.theme == Theme::Lines || settings.borders,
            compact: settings.compact,
            cursor_fallback: settings.cursor_fallback,
            status_blank: self.blank_until.is_some_and(|until| now < until),
            secret_silent: settings.secret_echo == SecretEcho::Silent,
        }
    }

    /// Draws a frame when something changed, unless a resize is settling
    /// or the quiet start still waits. Returns whether it drew.
    pub fn draw(&mut self, now: Instant) -> Result<bool, TuiError> {
        if let Some(at) = self.first_draw_at {
            if now < at && self.runtime.is_busy() {
                return Ok(false);
            }
            self.first_draw_at = None;
        }
        if self.resize_until.is_some() || !self.redraw {
            return Ok(false);
        }
        self.redraw = false;
        let settings = self.app.settings();
        if !self.styles.matches(&settings.theme, &settings.colors) {
            self.styles = Styles::new(
                &settings.theme,
                &settings.colors,
                self.themes_dir.as_deref(),
                self.styles.support(),
            );
        }
        let size = self.terminal.size().map_err(backend_error)?;
        let look = self.look(size.width, size.height, now);
        self.memory.freeze = look.screen_reader;
        let view = view::build(self.app, &look, &mut self.memory);
        for side in Side::BOTH {
            self.app.set_page_rows(side, view.page_rows[side.index()]);
        }
        let styles = &self.styles;
        let mut origin = (0, 0);
        let completed = self
            .terminal
            .draw(|frame| {
                let area = frame.area();
                origin = (area.x, area.y);
                view::paint(&view, styles, frame.buffer_mut());
                frame.set_cursor_position(Position::new(
                    area.x + view.cursor.0,
                    area.y + view.cursor.1,
                ));
            })
            .map_err(backend_error)?;
        let buffer = completed.buffer;
        let area = buffer.area;
        let mut lines = Vec::new();
        let mut again: Vec<(u16, u16, Cell)> = Vec::new();
        let mut resent = Vec::new();
        for y in area.top()..area.bottom() {
            let text = view::line_text(buffer, y);
            let row = usize::from(y - area.y);
            let changed = self.lines.get(row) != Some(&text);
            if changed && let Some(first) = view::first_wide(buffer, y) {
                resent.push(y - area.y);
                let mut x = first;
                while x < area.right() {
                    let cell = &buffer[(x, y)];
                    let width = view::cells(cell.symbol()).max(1) as u16;
                    again.push((x, y, cell.clone()));
                    x += width;
                }
            }
            lines.push(text);
        }
        if !again.is_empty() {
            let cursor = Position::new(origin.0 + view.cursor.0, origin.1 + view.cursor.1);
            let backend = self.terminal.backend_mut();
            backend
                .draw(again.iter().map(|(x, y, cell)| (*x, *y, cell)))
                .map_err(backend_error)?;
            backend.set_cursor_position(cursor).map_err(backend_error)?;
            backend.flush().map_err(backend_error)?;
        }
        self.resent = resent;
        self.shown_status = if look.status_blank {
            String::new()
        } else {
            view::clean(&self.app.status_line())
        };
        self.lines = lines;
        self.cursor = view.cursor;
        let active = self.app.active_panel();
        let title = format!(
            "{} - diroxide",
            view::clean(&place_title(
                active.loading_place().unwrap_or(active.place())
            ))
        );
        if title != self.title {
            if let Some(sink) = self.title_sink.as_mut() {
                sink(&title);
            }
            self.title = title;
        }
        Ok(true)
    }
}
