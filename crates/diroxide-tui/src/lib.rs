//! The terminal frontend: ratatui on crossterm, in raw mode, from Wave 1.
//!
//! The screen is a title line that says the active place first, the
//! panels, a status line that says the key fact first, and a jobs line.
//! The hardware cursor sits on the focused row, so screen readers and
//! Braille displays follow it. The plain theme draws no boxes; highlights
//! carry an attribute and a text marker (a leading asterisk on selected
//! rows), never color alone. No spinners: progress is a sentence that
//! changes at most twice a second.
//!
//! Phase 0: [`render`] builds the screen as plain lines, and
//! [`Tui::run`] prints one screen and returns, with no raw mode and no
//! ratatui yet. Wave 1 (W1d) owns this crate. In screen-reader mode only
//! the active panel is drawn, full width, so a line never mixes the two
//! panels' rows (R4 checks this choice).

use std::io::Write;

use diroxide_app::{App, Runtime};
use diroxide_core::format::first_cells;
use diroxide_core::{Explain, ListModel};
use thiserror::Error;

/// How the screen is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    /// No box drawing: words and spaces only.
    #[default]
    Plain,
    /// Lines between the panels, for sighted users.
    Lines,
}

/// Options of the terminal frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuiOptions {
    /// The theme.
    pub theme: Theme,
    /// Behave for a screen reader: one panel at a time, full width.
    pub screen_reader: bool,
    /// The terminal's width in columns.
    pub width: u16,
    /// The terminal's height in rows.
    pub height: u16,
}

impl Default for TuiOptions {
    fn default() -> Self {
        Self {
            theme: Theme::Plain,
            screen_reader: true,
            width: 80,
            height: 24,
        }
    }
}

/// The screen as text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    /// The lines, top to bottom.
    pub lines: Vec<String>,
    /// The line the hardware cursor sits on: the focused row.
    pub cursor_line: Option<usize>,
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
        "Check the terminal, or run diro in another one.".to_owned()
    }
}

/// One panel's rows, fitted to a height and a width, with the focused row
/// kept in view; selected rows start with an asterisk.
fn panel_lines(list: &ListModel, width: usize, height: usize) -> (Vec<String>, Option<usize>) {
    let focus = list.focus_index();
    let start = match focus {
        Some(f) if f >= height => f + 1 - height,
        _ => 0,
    };
    let mut lines = Vec::new();
    let mut cursor = None;
    if list.is_empty() {
        lines.push(list.empty_text().to_owned());
    }
    for (i, row) in list.rows().iter().enumerate().skip(start).take(height) {
        if Some(i) == focus {
            cursor = Some(lines.len());
        }
        let marker = if row.selected { "* " } else { "  " };
        let text = format!("{marker}{}", row.spoken());
        lines.push(first_cells(&text, width).to_owned());
    }
    (lines, cursor)
}

/// Builds the screen: the title line, the panels, the status line and the
/// jobs line.
pub fn render(app: &App, options: &TuiOptions) -> Screen {
    let width = usize::from(options.width.max(20));
    let rows = usize::from(options.height.max(5)) - 3;
    let mut lines = vec![first_cells(&app.title_line(), width).to_owned()];
    let active = app.active_side();
    let cursor_line;
    if options.screen_reader {
        let (panel, cursor) = panel_lines(app.panel(active).list(), width, rows);
        cursor_line = cursor.map(|c| c + 1);
        lines.extend(panel);
    } else {
        let half = (width - 1) / 2;
        let separator = if options.theme == Theme::Lines {
            "|"
        } else {
            " "
        };
        let (left, left_cursor) =
            panel_lines(app.panel(diroxide_core::Side::Left).list(), half, rows);
        let (right, right_cursor) =
            panel_lines(app.panel(diroxide_core::Side::Right).list(), half, rows);
        let focused = if active == diroxide_core::Side::Left {
            left_cursor
        } else {
            right_cursor
        };
        cursor_line = focused.map(|c| c + 1);
        for i in 0..left.len().max(right.len()) {
            let l = left.get(i).map_or("", String::as_str);
            let r = right.get(i).map_or("", String::as_str);
            lines.push(format!("{l:<half$}{separator}{r}").trim_end().to_owned());
        }
    }
    lines.push(first_cells(&app.status_line(), width).to_owned());
    lines.push(first_cells(&app.jobs().line(), width).to_owned());
    Screen { lines, cursor_line }
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

    /// Runs the app. Phase 0: carries out the start effects, prints one
    /// screen to `out`, and returns.
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
    fn the_screen_reader_screen_shows_the_active_panel() {
        let (mut app, mut runtime) = app();
        let start = app.start();
        runtime.run_until_idle(&mut app, start);
        let screen = render(&app, &TuiOptions::default());
        assert!(screen.lines[0].starts_with("diroxide-tui, in "));
        assert_eq!(screen.cursor_line, Some(1));
        assert!(
            screen.lines[1].starts_with("  src, folder"),
            "{:?}",
            screen.lines
        );
        assert!(screen.lines.iter().all(|l| l.chars().count() <= 80));
        assert_eq!(screen.lines.last().unwrap(), "No jobs");
    }

    #[test]
    fn the_sighted_screen_shows_both_panels() {
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
    }

    #[test]
    fn running_prints_one_screen() {
        let (mut app, mut runtime) = app();
        let mut out = Vec::new();
        Tui::default()
            .run(&mut app, &mut runtime, &mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.lines().next().unwrap().ends_with(", left panel"));
    }
}
