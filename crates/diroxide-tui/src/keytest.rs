//! `diro keys test`: what the terminal sends for each key, in words, and
//! what diroxide would run on it, one line a key, until Escape or ten
//! seconds without a key (research/alpha3-plan.md, the key test; R19
//! spike one). The answer to "my key does nothing": the terminal never
//! sent it, or sent another.
//!
//! The lines put the key first: "Got Control plus F5. Runs Sort by
//! modified. Sent F5 with Control, a press." A release is said too, since
//! it shows what the terminal reports, and runs nothing. Raw mode only;
//! the screen stays as it is, so the lines can be read back afterwards.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode as TermCode, KeyEvent, KeyEventKind};
use crossterm::terminal;
use diroxide_app::keymap::{self, Keymap};

use crate::{TuiError, keys};

/// How long the key test waits for a key.
pub const IDLE: Duration = Duration::from_secs(10);

/// The first line, said before any key.
pub const INTRO: &str = "Key test: press keys to hear what each sends and runs. Escape, or 10 seconds without a key, ends it.";

/// The line for one key event: what it runs first, then what the terminal
/// sent.
pub fn line(keymap: &Keymap, event: &KeyEvent) -> String {
    let sent = keys::sent_words(event);
    if event.kind == KeyEventKind::Release {
        return format!("Released. {sent}");
    }
    let said = match keys::to_key(event) {
        Some(key) => {
            let mut said = keymap::key_test_everywhere(keymap, &key, false);
            if key.mods.ctrl
                && key.mods.alt
                && let diroxide_core::KeyCode::Char(c) = key.code
                && cfg!(windows)
            {
                said.push_str(&format!(" On Windows this may be AltGr: it types {c}."));
            }
            said
        }
        None => "A key diroxide does not use.".to_owned(),
    };
    format!("{said} {sent}")
}

/// What ended the test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Escape was pressed.
    Escape,
    /// No key came for the idle time.
    Idle,
}

/// Runs the key test over an event source: `next(limit)` gives the next
/// terminal event, or `None` when `limit` passed without one. Each line
/// goes to `out` ending with a carriage return and a line feed, as raw
/// mode needs.
pub fn run_with(
    keymap: &Keymap,
    mut next: impl FnMut(Duration) -> io::Result<Option<Event>>,
    out: &mut dyn Write,
) -> Result<Ended, TuiError> {
    writeln!(out, "{INTRO}\r")?;
    out.flush()?;
    loop {
        let Some(found) = next(IDLE)? else {
            writeln!(out, "No key for 10 seconds. Key test ended.\r")?;
            out.flush()?;
            return Ok(Ended::Idle);
        };
        let Event::Key(key) = found else {
            continue;
        };
        writeln!(out, "{}\r", line(keymap, &key))?;
        out.flush()?;
        if key.code == TermCode::Esc && keys::acts(&key) && key.modifiers.is_empty() {
            writeln!(out, "Escape: key test ended.\r")?;
            out.flush()?;
            return Ok(Ended::Escape);
        }
    }
}

/// Turns raw mode off when dropped, on a panic too.
struct Raw;

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Runs the key test on the real terminal: raw mode on, the lines written
/// to standard output, raw mode off at the end whatever happens.
pub fn run(keymap: &Keymap) -> Result<Ended, TuiError> {
    terminal::enable_raw_mode()?;
    let _raw = Raw;
    let mut out = io::stdout();
    run_with(
        keymap,
        |limit| {
            if event::poll(limit)? {
                event::read().map(Some)
            } else {
                Ok(None)
            }
        },
        &mut out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventState, KeyModifiers};

    fn key(code: TermCode, mods: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: mods,
            kind,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn each_key_says_what_it_runs_then_what_was_sent() {
        let keymap = Keymap::defaults();
        let mut script = vec![
            key(TermCode::F(5), KeyModifiers::CONTROL, KeyEventKind::Press),
            key(TermCode::F(5), KeyModifiers::CONTROL, KeyEventKind::Release),
            Event::FocusGained,
            key(
                TermCode::Char('j'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ),
            key(TermCode::Esc, KeyModifiers::NONE, KeyEventKind::Press),
        ]
        .into_iter();
        let mut out = Vec::new();
        let ended = run_with(&keymap, |_| Ok(script.next()), &mut out).unwrap();
        assert_eq!(ended, Ended::Escape);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
        assert_eq!(lines[0], INTRO);
        assert_eq!(
            lines[1],
            "Got Control plus F5. Runs Sort by modified. Sent F5 with Control, a press."
        );
        assert_eq!(lines[2], "Released. Sent F5 with Control, a release.");
        assert!(lines[3].starts_with("Got Control plus J. Nothing runs on it."));
        assert!(lines[4].starts_with("Got Escape."), "{}", lines[4]);
        assert_eq!(lines[5], "Escape: key test ended.");
        for line in &lines[1..] {
            assert!(
                diroxide_core::format::within_cells(line, line.split('.').next().unwrap()),
                "{line}"
            );
        }
    }

    #[test]
    fn ten_seconds_without_a_key_ends_it() {
        let keymap = Keymap::defaults();
        let mut waited = Vec::new();
        let mut out = Vec::new();
        let ended = run_with(
            &keymap,
            |limit| {
                waited.push(limit);
                Ok(None)
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(ended, Ended::Idle);
        assert_eq!(waited, [Duration::from_secs(10)]);
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with("No key for 10 seconds. Key test ended.\r\n"));
    }
}
