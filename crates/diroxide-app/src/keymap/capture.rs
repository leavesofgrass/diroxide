//! The capture engine: Change key in the Keys list waits for one key
//! press and says it back in words (research/alpha3-plan.md, "Change key,
//! step by step", and "Terminal details the capture engine must handle").
//!
//! - **Waiting.** The prompt says "Press the new key now. Escape cancels."
//!   The next press is taken before the keymap sees it, and the first one
//!   ends the capture. A key inside the settle time after the prompt opens
//!   is passed over, so the Enter that chose Change key, held down, is not
//!   taken as the new key.
//! - **Press only.** Releases and a modifier alone never reach here: the
//!   frontend drops them (the terminal's `keys` module), as it does for
//!   every key.
//! - **AltGr.** On Windows a terminal sends AltGr as Control plus Alt, so
//!   Control plus Alt with a printable character is read as typed text,
//!   not a key, and the engine says so and goes on waiting; the typed name
//!   binds such a key.
//! - **Paste.** Pasted text is refused in words and the wait goes on.
//! - **Ten seconds.** With no key for ten seconds the capture ends with
//!   "No key heard. The terminal may keep it." and the typed name is
//!   offered: a key's name in `keymap.toml`'s syntax, read with the same
//!   function the file is read with, so it is an equal way in. It is also
//!   the only way to bind Escape itself.
//! - **Say it back.** "Got Control plus F9", from the live spoken names.
//!
//! The frontend turns the keyboard enhancement protocol on only while a
//! capture waits, when the terminal has it, and back off after; the
//! engine's [`KeyReports`] then says which kind of terminal it heard.
//! The engine keeps its own time from the app's clock, so tests drive it
//! with the fake clock.

use diroxide_core::{CoreError, Key, KeyCode, Timestamp};

use super::names::{Platform, spoken, written};

/// The prompt while a capture waits (38 cells).
pub const PROMPT: &str = "Press the new key now. Escape cancels.";

/// How long a capture waits for a key.
pub const TIMEOUT_SECONDS: u64 = 10;

/// What a capture says when no key came (39 cells).
pub const NO_KEY: &str = "No key heard. The terminal may keep it.";

/// The label of the typed name's field, after a capture timed out or for
/// a column that cannot be captured.
pub const TYPE_THE_NAME: &str = "Type the key's name, such as ctrl+f5";

/// What a paste says while a capture waits.
pub const PASTE_REFUSED: &str = "Pasted text is not a key. Press one key.";

/// What a capture says when it is cancelled.
pub const CANCELLED: &str = "Nothing changed.";

/// How the terminal reports keys, as the frontend found it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyReports {
    /// Not known: nothing is said.
    #[default]
    Unknown,
    /// The terminal reports each key as itself (the enhancement protocol
    /// is on while the capture waits, or the system reads key records).
    Exact,
    /// The terminal sends Ctrl+I as Tab, Ctrl+M as Enter and Ctrl+[ as
    /// Escape.
    Merged,
}

impl KeyReports {
    /// The sentence for the capture prompt's lines, or empty.
    pub const fn sentence(self) -> &'static str {
        match self {
            KeyReports::Unknown => "",
            KeyReports::Exact => "This terminal reports keys exactly.",
            KeyReports::Merged => "This terminal merges Ctrl+I with Tab, and Ctrl+M with Enter.",
        }
    }

    /// The words added after the say-back for a key a merging terminal
    /// shares with another: "This terminal sends Ctrl+I as Tab too."
    pub fn shared(self, key: &Key) -> Option<&'static str> {
        if self != KeyReports::Merged || !key.mods.is_empty() {
            return None;
        }
        match key.code {
            KeyCode::Tab => Some("This terminal sends Ctrl+I as Tab too."),
            KeyCode::Enter => Some("This terminal sends Ctrl+M as Enter too."),
            KeyCode::Escape => Some("This terminal sends Ctrl+[ as Escape too."),
            _ => None,
        }
    }
}

/// What a key or a moment means to a waiting capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Captured {
    /// Still waiting; the words, when there are any, say why the key was
    /// not taken.
    Waiting(Option<String>),
    /// A key was taken; `said` is the say-back, "Got Control plus F9".
    Key {
        /// The key.
        key: Key,
        /// What is said: "Got Control plus F9".
        said: String,
    },
    /// Escape: nothing changes.
    Cancelled,
    /// No key for ten seconds.
    TimedOut,
}

/// A capture waiting for one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    started: Timestamp,
    last: Timestamp,
    settle_ms: u64,
    platform: Platform,
    altgr: bool,
    reports: KeyReports,
}

fn millis_since(from: Timestamp, to: Timestamp) -> u64 {
    let seconds = to.unix_seconds() - from.unix_seconds();
    let nanos = i64::from(to.subsec_nanos()) - i64::from(from.subsec_nanos());
    u64::try_from(seconds.saturating_mul(1000) + nanos / 1_000_000).unwrap_or(0)
}

/// The say-back for a key: "Got Control plus F9".
pub fn say_back(key: &Key, platform: Platform) -> String {
    format!("Got {}", spoken(key, platform))
}

/// The character a key types when AltGr arrives as Control plus Alt, as
/// it does on Windows (crossterm issue 820): Control plus Alt with a
/// printable character. `None` for any other key.
pub fn is_altgr(key: &Key) -> Option<char> {
    if !(key.mods.ctrl && key.mods.alt) || key.mods.meta {
        return None;
    }
    match key.code {
        KeyCode::Char(c) if !c.is_control() && c != ' ' => Some(c),
        _ => None,
    }
}

impl Capture {
    /// A capture that starts waiting now, naming keys for `platform`.
    /// Keys inside `settle_ms` are passed over. On Windows, Control plus
    /// Alt with a character is read as AltGr typing; see
    /// [`Capture::with_altgr`].
    pub fn new(now: Timestamp, settle_ms: u64, platform: Platform) -> Self {
        Self {
            started: now,
            last: now,
            settle_ms,
            platform,
            altgr: platform == Platform::Windows,
            reports: KeyReports::Unknown,
        }
    }

    /// The same capture, reading Control plus Alt with a character as
    /// AltGr typing or not: the app says so for the system it runs on,
    /// whatever system its keys are named for.
    pub fn with_altgr(mut self, altgr: bool) -> Self {
        self.altgr = altgr;
        self
    }

    /// The same capture with what the frontend knows of the terminal.
    pub fn with_reports(mut self, reports: KeyReports) -> Self {
        self.reports = reports;
        self
    }

    /// What the frontend knows of the terminal.
    pub fn reports(&self) -> KeyReports {
        self.reports
    }

    /// The prompt's lines: the prompt, how to bind Escape, and the kind of
    /// terminal when it is known.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            PROMPT.to_owned(),
            format!("Or wait {TIMEOUT_SECONDS} seconds to type the key's name instead."),
        ];
        let kind = self.reports.sentence();
        if !kind.is_empty() {
            lines.push(kind.to_owned());
        }
        lines
    }

    /// A key press. The first one past the settle time ends the capture,
    /// unless it is AltGr typing, which is said and passed over.
    pub fn press(&mut self, key: Key, now: Timestamp) -> Captured {
        if millis_since(self.started, now) < self.settle_ms {
            return Captured::Waiting(None);
        }
        self.last = now;
        if key == Key::plain(KeyCode::Escape) {
            return Captured::Cancelled;
        }
        if let Some(c) = is_altgr(&key).filter(|_| self.altgr) {
            return Captured::Waiting(Some(format!(
                "{} reads as AltGr typing {c}. Type its name to bind it.",
                written(&key, self.platform)
            )));
        }
        let mut said = say_back(&key, self.platform);
        if let Some(more) = self.reports.shared(&key) {
            said.push_str(". ");
            said.push_str(more);
        }
        Captured::Key { key, said }
    }

    /// Text the terminal pasted: refused in words; the wait starts again.
    pub fn paste(&mut self, now: Timestamp) -> Captured {
        self.last = now;
        Captured::Waiting(Some(PASTE_REFUSED.to_owned()))
    }

    /// Whether ten seconds passed since the capture started or since the
    /// last key or paste it passed over.
    pub fn timed_out(&self, now: Timestamp) -> bool {
        millis_since(self.last, now) >= TIMEOUT_SECONDS * 1000
    }
}

/// The typed name, the equal way in: a key in `keymap.toml`'s syntax,
/// read with the same function as the file. The error is one sentence,
/// meaning first.
pub fn typed_key(text: &str) -> Result<Key, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("No key name typed. Type one, such as ctrl+f5.".to_owned());
    }
    match Key::parse(trimmed) {
        Ok(key) => Ok(key),
        Err(CoreError::KeySequence(_)) => Err(format!(
            "Two keys in a row: {trimmed}. A key is one key with its modifiers."
        )),
        Err(_) => Err(format!(
            "Not a key name: {trimmed}. Write it as keymap.toml does, such as ctrl+f5."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    fn at(seconds: i64, millis: u32) -> Timestamp {
        Timestamp::from_unix(1_760_000_000 + seconds, millis * 1_000_000)
    }

    #[test]
    fn the_first_key_past_the_settle_time_is_taken_and_said_back() {
        let mut capture = Capture::new(at(0, 0), 250, Platform::Windows);
        assert_eq!(
            capture.press(key("enter"), at(0, 100)),
            Captured::Waiting(None),
            "the Enter that chose Change key, held down"
        );
        assert_eq!(
            capture.press(key("ctrl+f9"), at(0, 400)),
            Captured::Key {
                key: key("ctrl+f9"),
                said: "Got Control plus F9".to_owned()
            }
        );
        let mut capture = Capture::new(at(0, 0), 0, Platform::MacOs);
        assert_eq!(
            capture.press(key("meta+alt+q"), at(1, 0)),
            Captured::Key {
                key: key("meta+alt+q"),
                said: "Got Command plus Option plus Q".to_owned()
            }
        );
    }

    #[test]
    fn escape_cancels_and_its_name_binds_it() {
        let mut capture = Capture::new(at(0, 0), 0, Platform::Other);
        assert_eq!(capture.press(key("escape"), at(1, 0)), Captured::Cancelled);
        assert_eq!(typed_key(" escape "), Ok(key("escape")));
        let mut capture = Capture::new(at(0, 0), 0, Platform::Other);
        assert!(matches!(
            capture.press(key("shift+escape"), at(1, 0)),
            Captured::Key { .. }
        ));
    }

    #[test]
    fn altgr_and_a_paste_are_said_and_the_wait_goes_on() {
        let mut capture = Capture::new(at(0, 0), 0, Platform::Windows);
        assert_eq!(
            capture.press(key("ctrl+alt+q"), at(1, 0)),
            Captured::Waiting(Some(
                "Ctrl+Alt+Q reads as AltGr typing q. Type its name to bind it.".to_owned()
            ))
        );
        assert_eq!(
            capture.paste(at(9, 0)),
            Captured::Waiting(Some(PASTE_REFUSED.to_owned()))
        );
        assert!(
            !capture.timed_out(at(18, 0)),
            "the paste started the wait again"
        );
        assert!(capture.timed_out(at(19, 0)));
        assert!(matches!(
            capture.press(key("ctrl+alt+left"), at(2, 0)),
            Captured::Key { .. }
        ));
        let mut linux = Capture::new(at(0, 0), 0, Platform::Other);
        assert!(matches!(
            linux.press(key("ctrl+alt+q"), at(1, 0)),
            Captured::Key { .. }
        ));
    }

    #[test]
    fn ten_seconds_without_a_key_times_out() {
        let capture = Capture::new(at(0, 0), 250, Platform::Windows);
        assert!(!capture.timed_out(at(9, 999)));
        assert!(capture.timed_out(at(10, 0)));
    }

    #[test]
    fn a_merging_terminal_is_said() {
        let mut capture =
            Capture::new(at(0, 0), 0, Platform::Other).with_reports(KeyReports::Merged);
        assert_eq!(
            capture.press(key("tab"), at(1, 0)),
            Captured::Key {
                key: key("tab"),
                said: "Got Tab. This terminal sends Ctrl+I as Tab too.".to_owned()
            }
        );
        assert_eq!(
            capture.lines(),
            [
                PROMPT,
                "Or wait 10 seconds to type the key's name instead.",
                "This terminal merges Ctrl+I with Tab, and Ctrl+M with Enter."
            ]
        );
        let exact = Capture::new(at(0, 0), 0, Platform::Other).with_reports(KeyReports::Exact);
        assert_eq!(exact.lines()[2], "This terminal reports keys exactly.");
        assert_eq!(Capture::new(at(0, 0), 0, Platform::Other).lines().len(), 2);
    }

    #[test]
    fn the_typed_name_is_read_as_the_file_reads_it() {
        assert_eq!(typed_key("Ctrl+F5"), Ok(key("ctrl+f5")));
        assert_eq!(typed_key("kp-plus"), Ok(key("kp-plus")));
        assert_eq!(
            typed_key("hyper+q"),
            Err("Not a key name: hyper+q. Write it as keymap.toml does, such as ctrl+f5.".into())
        );
        assert_eq!(
            typed_key("ctrl+x ctrl+f"),
            Err("Two keys in a row: ctrl+x ctrl+f. A key is one key with its modifiers.".into())
        );
        assert!(typed_key("  ").is_err());
    }

    #[test]
    fn every_line_says_its_meaning_within_forty_cells() {
        for line in [PROMPT, NO_KEY, PASTE_REFUSED, CANCELLED, TYPE_THE_NAME] {
            assert!(line.chars().count() <= 40, "{line}");
        }
        for reports in [KeyReports::Exact, KeyReports::Merged] {
            let line = reports.sentence();
            let meaning = line.split(',').next().unwrap_or(line);
            assert!(within_cells(line, meaning), "{line}");
        }
        for bad in ["hyper+q", "ctrl+x ctrl+f", ""] {
            let line = typed_key(bad).unwrap_err();
            let meaning = line.split(". ").next().unwrap_or(&line);
            let meaning = meaning.split(':').next().unwrap_or(meaning);
            assert!(within_cells(&line, meaning), "{line}");
        }
    }
}
