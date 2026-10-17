//! The terminal frontend: ratatui on crossterm, in raw mode on the
//! alternate screen (or the inline viewport with `--inline`), given back
//! as it was on quit, on an error and on a panic.
//!
//! The screen (ADR-0007 as amended twice; plan section 5.10): a title line
//! that says the active place first and the panel before the parent
//! ("Documents, left panel, in D:\work"), the panels in the layout the
//! settings name (side by side, the default; stacked, the fallback for
//! screen-reader users and narrow sessions; single), the minibuffer when a
//! text field is open (the prompt, then its candidates), the key bar in
//! sighted mode, the status line that says the key fact first, and the
//! jobs line. The hardware cursor sits at the focused row's first cell on
//! every frame in every state; a prompt keeps it at the insertion point;
//! the `cursor_fallback` setting parks it on the status line.
//!
//! What a screen reader gets (R4 section 3.2; research/alpha1-plan.md part
//! three): every line says its meaning in its first 40 cells; a selected
//! row starts with an asterisk and says "selected"; no box drawing unless
//! the `borders` setting or the lines theme asks for an ASCII bar; nothing
//! by color alone, and `NO_COLOR` leaves attributes only; no spinners; a
//! repeated message is blanked for 150 milliseconds so it is heard again;
//! in screen-reader mode the inactive panel is redrawn only on demand and
//! a candidate list only when the focus is in it; display names only.
//!
//! The modules: [`keys`] turns terminal key events into diroxide's keys
//! (presses and repeats only); [`style`] applies the theme through its
//! roles; [`session`] is the loop one step at a time over any backend,
//! which the tests drive on ratatui's test backend; [`terminal`] owns the
//! real terminal; [`keytest`] is `diro keys test`; [`viewer`] draws the
//! viewer's rows and their token classes (W2d).
//!
//! Wave 1 (W1d) owns this crate.

use std::io::Write;
use std::path::PathBuf;

use diroxide_app::{App, Runtime};
use diroxide_core::Explain;
use diroxide_theme::ColorSupport;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use thiserror::Error;

pub mod keys;
pub mod keytest;
pub mod session;
pub mod style;
pub mod terminal;
mod view;
pub mod viewer;

pub use session::Session;
pub use style::Styles;
pub use terminal::restore;

/// How the screen is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    /// No box drawing: words and spaces only.
    #[default]
    Plain,
    /// An ASCII bar between the panels, for sighted users, as the
    /// `borders` setting gives.
    Lines,
}

/// Options of the terminal frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuiOptions {
    /// The theme.
    pub theme: Theme,
    /// Behave for a screen reader: the inactive panel redrawn only on
    /// demand, no key bar, candidates listed only on request, progress at
    /// its interval.
    pub screen_reader: bool,
    /// The terminal's width in columns, for [`render`].
    pub width: u16,
    /// The terminal's height in rows, for [`render`].
    pub height: u16,
    /// Draw below the prompt instead of on the alternate screen
    /// (`--inline`).
    pub inline: bool,
    /// The color level; `None` reads it from the environment (`NO_COLOR`,
    /// `TERM`, `COLORTERM`, `DIRO_COLOR`).
    pub color: Option<ColorSupport>,
}

impl Default for TuiOptions {
    fn default() -> Self {
        Self {
            theme: Theme::Plain,
            screen_reader: true,
            width: 80,
            height: 24,
            inline: false,
            color: None,
        }
    }
}

/// The screen as text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    /// The lines, top to bottom, trailing spaces trimmed.
    pub lines: Vec<String>,
    /// The line the hardware cursor sits on: the focused row.
    pub cursor_line: Option<usize>,
    /// The cell the hardware cursor sits on in that line: the row's first
    /// cell, or a prompt's insertion point.
    pub cursor_column: usize,
}

/// Why the terminal frontend stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TuiError {
    /// Writing to the terminal failed.
    #[error("the terminal could not be written: {0}")]
    Io(#[from] std::io::Error),
}

impl Explain for TuiError {
    fn next_step(&self) -> String {
        "Check the terminal, or run diro in another one; diro --inline and diro ls work where the full screen does not.".to_owned()
    }
}

/// The files the terminal reads and writes besides the app's: the owner's
/// themes folder and the terminal's log in the state folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Files {
    /// The folder of the owner's theme files.
    pub themes: Option<PathBuf>,
    /// Where the terminal writes what it found at start (the keyboard
    /// enhancement answer, the color level); never shown on screen.
    pub log: Option<PathBuf>,
}

/// Builds the screen as text: the title line, the panels or the dialog
/// on top, the minibuffer, the status line and the jobs line, with the
/// cursor. The layout, borders, compact mode and cursor fallback come from
/// the app's settings; the size and screen-reader mode from the options.
pub fn render(app: &App, options: &TuiOptions) -> Screen {
    let settings = app.settings();
    let look = view::Look {
        width: options.width,
        height: options.height,
        screen_reader: options.screen_reader,
        borders: options.theme == Theme::Lines || settings.borders,
        compact: settings.compact,
        cursor_fallback: settings.cursor_fallback,
        status_blank: false,
        secret_silent: settings.secret_echo == diroxide_app::config::SecretEcho::Silent,
    };
    let mut memory = view::Memory::default();
    let built = view::build(app, &look, &app.status_line(), &mut memory);
    let styles = Styles::new(
        &settings.theme,
        &settings.colors,
        None,
        ColorSupport::NoColor,
    );
    let mut buffer = Buffer::empty(Rect::new(0, 0, built.width, built.height));
    view::paint(&built, &styles, &mut buffer);
    Screen {
        lines: (0..built.height)
            .map(|y| view::line_text(&buffer, y))
            .collect(),
        cursor_line: Some(usize::from(built.cursor.1)),
        cursor_column: usize::from(built.cursor.0),
    }
}

/// The terminal frontend.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tui {
    options: TuiOptions,
}

impl Tui {
    /// A frontend with options.
    pub fn new(options: TuiOptions) -> Self {
        Self { options }
    }

    /// Prints one screen and returns: the start's effects carried out, the
    /// screen written to `out` as plain lines. For output that is not a
    /// terminal (a pipe, a file, a test).
    pub fn run(
        &mut self,
        app: &mut App,
        runtime: &mut Runtime,
        out: &mut dyn Write,
    ) -> Result<(), TuiError> {
        let start = app.start();
        runtime.run_until_idle(app, start);
        for line in render(app, &self.options).lines {
            writeln!(out, "{line}")?;
        }
        Ok(())
    }

    /// Runs the app on the real terminal until it quits, then gives the
    /// terminal back as it was. The caller reads [`App::wants_quit`],
    /// [`App::quit_folder`] and [`App::keeps_shell_folder`] afterwards for
    /// `--cwd-file`.
    pub fn run_terminal(
        &mut self,
        app: &mut App,
        runtime: &mut Runtime,
        files: &Files,
    ) -> Result<(), TuiError> {
        terminal::run(
            app,
            runtime,
            self.options,
            files.themes.clone(),
            files.log.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_app::AppOptions;
    use diroxide_core::Place;

    fn app() -> (App, Runtime) {
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let src = here.join("src").unwrap();
        (App::new(AppOptions::new(here, src)), Runtime::with_local())
    }

    #[test]
    fn the_screen_says_the_active_place_first_and_sits_on_the_focus() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        let screen = render(&app, &TuiOptions::default());
        assert!(
            screen.lines[0].starts_with("diroxide-tui, left panel, in "),
            "{:?}",
            screen.lines
        );
        // Side by side: the panels' places on the next line, then rows.
        assert_eq!(screen.cursor_line, Some(2));
        assert_eq!(screen.cursor_column, 0);
        assert!(
            screen.lines[2].starts_with("src, folder"),
            "{:?}",
            screen.lines
        );
        assert!(screen.lines.iter().all(|l| l.chars().count() <= 80));
        assert_eq!(screen.lines.last().unwrap(), "No jobs");
        assert_eq!(screen.lines.len(), 24);
    }

    #[test]
    fn the_lines_theme_draws_an_ascii_bar_between_the_panels() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        let options = TuiOptions {
            screen_reader: false,
            theme: Theme::Lines,
            ..TuiOptions::default()
        };
        let screen = render(&app, &options);
        assert!(screen.lines[1].contains('|'));
        assert!(screen.lines.iter().any(|l| l.contains("lib.rs")));
        assert!(
            screen
                .lines
                .iter()
                .all(|l| l.chars().all(|c| c.is_ascii() || c.is_alphabetic()))
        );
    }

    #[test]
    fn running_prints_one_screen() {
        let (mut app, mut runtime) = app();
        let mut out = Vec::new();
        Tui::default()
            .run(&mut app, &mut runtime, &mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.lines().next().unwrap().contains(", left panel, in "));
    }
}
