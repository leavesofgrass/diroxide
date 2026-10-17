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
//!
//! `diro keys probe` is the guided form, the key-delivery spike's tool: it
//! asks for each key of the delivery matrix in turn ("Press Control plus
//! V now, 3 of 95."), notes what arrived (the key itself, another key, or
//! nothing within ten seconds, which counts as kept), and ends with one
//! line a key in the matrix's words (`ctrl+v = kept`), for the owner to
//! send back so `terminal-delivery.toml` gets that terminal's column.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode as TermCode, KeyEvent, KeyEventKind};
use crossterm::terminal;
use diroxide_app::keymap::{self, Keymap, Platform};
use diroxide_core::{Key, KeyCode};

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
                && let KeyCode::Char(c) = key.code
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

/// The probe's first line, said before the first key.
pub const PROBE_INTRO: &str = "Key probe: press each key asked for. Escape skips one; 10 seconds without a key counts it as kept.";

/// What arrived when the probe asked for a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probed {
    /// The key itself.
    Delivered,
    /// Nothing within ten seconds: the terminal keeps it.
    Kept,
    /// Another key arrived instead.
    Merged(Key),
    /// Escape: not tried this time.
    Skipped,
}

impl Probed {
    /// The value in `terminal-delivery.toml`'s words: "delivered", "kept",
    /// "merged with tab", or "to probe" for a skipped key.
    pub fn value(self) -> String {
        match self {
            Probed::Delivered => "delivered".to_owned(),
            Probed::Kept => "kept".to_owned(),
            Probed::Merged(other) => format!("merged with {}", keymap::for_file(&other)),
            Probed::Skipped => "to probe".to_owned(),
        }
    }

    /// The line said after the key, meaning first: "Delivered.", "Kept:
    /// nothing arrived in 10 seconds.", "Arrived as Tab.", "Skipped."
    pub fn line(self, platform: Platform) -> String {
        match self {
            Probed::Delivered => "Delivered.".to_owned(),
            Probed::Kept => "Kept: nothing arrived in 10 seconds.".to_owned(),
            Probed::Merged(other) => {
                format!("Arrived as {}.", keymap::spoken(&other, platform))
            }
            Probed::Skipped => "Skipped.".to_owned(),
        }
    }
}

/// Runs the probe over an event source, as [`run_with`] does: asks for
/// each key in turn and returns what arrived for each. Escape skips a key,
/// except while asking for Control plus left bracket, which most terminals
/// send as Escape; a release, a modifier alone or a key diroxide never
/// binds is passed over and the wait goes on.
pub fn probe_with(
    keys: &[Key],
    platform: Platform,
    mut next: impl FnMut(Duration) -> io::Result<Option<Event>>,
    out: &mut dyn Write,
) -> Result<Vec<(Key, Probed)>, TuiError> {
    writeln!(out, "{PROBE_INTRO}\r")?;
    out.flush()?;
    let escape = Key::plain(KeyCode::Escape);
    let bracket = Key::parse("ctrl+[").ok();
    let mut results = Vec::with_capacity(keys.len());
    for (index, wanted) in keys.iter().enumerate() {
        writeln!(
            out,
            "Press {} now, {} of {}.\r",
            keymap::spoken(wanted, platform),
            index + 1,
            keys.len()
        )?;
        out.flush()?;
        let found = loop {
            let Some(found) = next(IDLE)? else {
                break Probed::Kept;
            };
            let Event::Key(event) = found else {
                continue;
            };
            if !keys::acts(&event) {
                continue;
            }
            match keys::to_key(&event) {
                Some(got) if got == *wanted => break Probed::Delivered,
                Some(got) if got == escape && Some(*wanted) != bracket => break Probed::Skipped,
                Some(got) => break Probed::Merged(got),
                None => continue,
            }
        };
        writeln!(out, "{}\r", found.line(platform))?;
        out.flush()?;
        results.push((*wanted, found));
    }
    Ok(results)
}

/// The probe's results in the delivery matrix's words, one line a key,
/// then the counts: "ctrl+v = kept", "ctrl+i = merged with tab", ...,
/// "Delivered 80, kept 5, merged 4, skipped 3."
pub fn probe_report(results: &[(Key, Probed)]) -> String {
    let mut lines: Vec<String> = results
        .iter()
        .map(|(key, found)| format!("{} = {}", keymap::for_file(key), found.value()))
        .collect();
    let count = |want: fn(&Probed) -> bool| results.iter().filter(|(_, f)| want(f)).count();
    lines.push(format!(
        "Delivered {}, kept {}, merged {}, skipped {}.",
        count(|f| *f == Probed::Delivered),
        count(|f| *f == Probed::Kept),
        count(|f| matches!(f, Probed::Merged(_))),
        count(|f| *f == Probed::Skipped),
    ));
    lines.join("\n")
}

/// Runs the probe on the real terminal over every key of the delivery
/// matrix, raw mode on and off again whatever happens, and returns the
/// report ([`probe_report`]).
pub fn probe(platform: Platform) -> Result<String, TuiError> {
    let keys: Vec<Key> = keymap::tables::delivery_table()
        .iter()
        .map(|row| row.key)
        .collect();
    terminal::enable_raw_mode()?;
    let raw = Raw;
    let mut out = io::stdout();
    let results = probe_with(
        &keys,
        platform,
        |limit| {
            if event::poll(limit)? {
                event::read().map(Some)
            } else {
                Ok(None)
            }
        },
        &mut out,
    );
    drop(raw);
    Ok(probe_report(&results?))
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
    use crossterm::event::{KeyEventState, KeyModifiers, ModifierKeyCode};

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
    fn the_probe_notes_what_arrived_for_each_key() {
        let wanted: Vec<Key> = ["ctrl+v", "ctrl+i", "f5", "ctrl+[", "f11"]
            .iter()
            .map(|t| Key::parse(t).unwrap())
            .collect();
        let mut script = vec![
            // Ctrl+V: nothing, as Windows Terminal keeps it.
            None,
            // Ctrl+I: a modifier alone, then Tab.
            Some(key(
                TermCode::Modifier(ModifierKeyCode::LeftControl),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            )),
            Some(key(TermCode::Tab, KeyModifiers::NONE, KeyEventKind::Press)),
            // F5: its release is passed over, then the press.
            Some(key(
                TermCode::F(5),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            Some(key(TermCode::F(5), KeyModifiers::NONE, KeyEventKind::Press)),
            // Ctrl+[: Escape counts as what it sent.
            Some(key(TermCode::Esc, KeyModifiers::NONE, KeyEventKind::Press)),
            // F11: Escape skips it.
            Some(key(TermCode::Esc, KeyModifiers::NONE, KeyEventKind::Press)),
        ]
        .into_iter();
        let mut out = Vec::new();
        let results = probe_with(
            &wanted,
            Platform::Windows,
            |_| Ok(script.next().flatten()),
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
        assert_eq!(lines[0], PROBE_INTRO);
        assert_eq!(lines[1], "Press Control plus V now, 1 of 5.");
        assert_eq!(lines[2], "Kept: nothing arrived in 10 seconds.");
        assert_eq!(lines[4], "Arrived as Tab.");
        assert_eq!(lines[6], "Delivered.");
        assert_eq!(lines[7], "Press Control plus Left bracket now, 4 of 5.");
        assert_eq!(lines[8], "Arrived as Escape.");
        assert_eq!(lines[10], "Skipped.");
        for line in &lines[1..] {
            let meaning = line.split(" now,").next().unwrap_or(line);
            let meaning = meaning.split(':').next().unwrap_or(meaning);
            assert!(diroxide_core::format::within_cells(line, meaning), "{line}");
        }
        assert_eq!(
            probe_report(&results),
            "ctrl+v = kept\nctrl+i = merged with tab\nf5 = delivered\nctrl+[ = merged with escape\nf11 = to probe\nDelivered 1, kept 1, merged 2, skipped 1."
        );
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
