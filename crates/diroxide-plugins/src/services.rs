//! Host services, part one: what a plugin says (`ui/announce`), its log,
//! its settings, and its progress (R3, section 3.6; R22, sections 7.2 and
//! 9).
//!
//! - **Cleaned.** Every string a plugin sends loses control, format and
//!   direction characters and terminal escape sequences, becomes one line,
//!   and is cut to 300 characters.
//! - **Attributed, last.** A plugin's announcement ends with its source:
//!   "Lines: 42, files: 1. By Line counter." The meaning stays in the first
//!   40 cells, and a plugin cannot pass for the app.
//! - **Rate-limited.** Ten announcements a second, a burst of twenty; the
//!   rest fold into one line, "12 more messages from Line counter".
//! - **Progress in words.** `$/progress` becomes the job's sentence, with
//!   the plugin's name last in brackets, at most twice a second per token;
//!   a change to waiting for the owner is said once, at once.

use std::collections::HashMap;
use std::time::Duration;

use diroxide_core::Message;
use diroxide_core::names::{CharClass, classify};
use serde_json::{Map, Value};

use crate::lines;
use crate::manifest::Manifest;
use crate::shapes::WireImportance;

/// The longest text a plugin's announcement keeps, in characters.
pub const MAX_ANNOUNCE_CHARS: usize = 300;

/// Announcements a second, over time.
pub const ANNOUNCE_RATE: f64 = 10.0;

/// Announcements at once, in a burst.
pub const ANNOUNCE_BURST: f64 = 20.0;

/// How often a progress token's sentence may change.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(500);

/// Cleans text from a plugin for the screen and the screen reader: control,
/// format and direction characters and terminal escape sequences removed,
/// line breaks and tabs made spaces, runs of spaces made one, trimmed, and
/// cut to `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max * 4));
    let mut chars = text.chars().peekable();
    let mut count = 0;
    let mut space = false;
    while let Some(c) = chars.next() {
        if c == '\u{1B}' || c == '\u{9B}' {
            skip_escape(c, &mut chars);
            continue;
        }
        let c = if matches!(c, '\n' | '\r' | '\t' | '\u{2028}' | '\u{2029}') {
            ' '
        } else {
            c
        };
        if classify(c) != CharClass::Normal {
            continue;
        }
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            if count + 1 >= max {
                break;
            }
            out.push(' ');
            count += 1;
            space = false;
        }
        if count >= max {
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Skips one terminal escape sequence after its introducer: CSI (`ESC [`
/// or U+009B) up to its final byte, OSC (`ESC ]`) up to BEL or `ESC \`,
/// and any other `ESC x` pair.
fn skip_escape(first: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let kind = if first == '\u{9B}' {
        '['
    } else {
        match chars.next() {
            Some(k) => k,
            None => return,
        }
    };
    match kind {
        '[' => {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7E}').contains(&c) {
                    break;
                }
            }
        }
        ']' | 'P' | '_' | '^' => {
            while let Some(c) = chars.next() {
                if c == '\u{07}' {
                    break;
                }
                if c == '\u{1B}' && chars.peek() == Some(&'\\') {
                    chars.next();
                    break;
                }
            }
        }
        _ => {}
    }
}

/// A plugin's sentence with its source last: "Lines: 42, files: 1. By
/// Line counter." The sentence gets a final period when it has no final
/// mark.
pub fn attribute(text: &str, plugin: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return format!("By {plugin}.");
    }
    let end = if text.ends_with(['.', '?', '!']) {
        ""
    } else {
        "."
    };
    format!("{text}{end} By {plugin}.")
}

/// The message for a plugin's `ui/announce`: cleaned, attributed, at the
/// plugin's importance. `None` when nothing is left to say.
pub fn announcement(params: &Map<String, Value>, plugin: &str) -> Option<Message> {
    let text = params.get("text").and_then(Value::as_str)?;
    let text = clean_text(text, MAX_ANNOUNCE_CHARS);
    if text.is_empty() {
        return None;
    }
    let importance = params
        .get("importance")
        .cloned()
        .and_then(|v| serde_json::from_value::<WireImportance>(v).ok())
        .unwrap_or(WireImportance::Info);
    Some(importance.message(attribute(&text, plugin)))
}

/// The announcement rate limit of one plugin: a bucket of twenty that
/// refills at ten a second; what does not fit is counted and folded.
#[derive(Clone, Debug)]
pub struct AnnounceLimiter {
    tokens: f64,
    last: Duration,
    folded: usize,
    last_fold: Duration,
}

impl AnnounceLimiter {
    /// A full bucket at a time.
    pub fn new(now: Duration) -> Self {
        Self {
            tokens: ANNOUNCE_BURST,
            last: now,
            folded: 0,
            last_fold: now,
        }
    }

    fn refill(&mut self, now: Duration) {
        let elapsed = now.saturating_sub(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * ANNOUNCE_RATE).min(ANNOUNCE_BURST);
    }

    /// Whether an announcement may be said now; when not, it is counted
    /// for the fold line.
    pub fn admit(&mut self, now: Duration) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            self.folded += 1;
            false
        }
    }

    /// The fold line, once a second at most, when announcements were held
    /// back: "12 more messages from Line counter".
    pub fn fold(&mut self, now: Duration, plugin: &str) -> Option<Message> {
        if self.folded == 0 || now.saturating_sub(self.last_fold) < Duration::from_secs(1) {
            return None;
        }
        let count = self.folded;
        self.folded = 0;
        self.last_fold = now;
        Some(Message::status(lines::folded(count, plugin)))
    }
}

/// One progress token's state.
#[derive(Clone, Debug, Default)]
struct Track {
    title: String,
    message: Option<String>,
    current: Option<u64>,
    total: Option<u64>,
    unit: Option<String>,
    waiting: bool,
    said: Option<Duration>,
    pending: bool,
}

/// Turns `$/progress` into the job's sentences, at most twice a second per
/// token, with the change to waiting for the owner said at once.
#[derive(Clone, Debug, Default)]
pub struct ProgressBoard {
    tracks: HashMap<String, Track>,
}

/// What a progress value means for a request's timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// No change.
    Same,
    /// It now waits for the owner.
    Began,
    /// It no longer waits.
    Ended,
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|t| clean_text(t, MAX_ANNOUNCE_CHARS))
        .filter(|t| !t.is_empty())
}

impl ProgressBoard {
    /// Reads one `$/progress` value for a token; returns the sentence to
    /// show now, if any, and the waiting change.
    pub fn update(
        &mut self,
        token: &str,
        value: &Value,
        plugin: &str,
        now: Duration,
    ) -> (Option<Message>, Waiting) {
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("report");
        if kind == "partial" {
            return (None, Waiting::Same);
        }
        let track = self.tracks.entry(token.to_owned()).or_default();
        if let Some(title) = text_of(value, "title") {
            track.title = title;
        }
        if let Some(message) = text_of(value, "message") {
            track.message = Some(message);
        }
        if let Some(n) = value.get("current").and_then(Value::as_u64) {
            track.current = Some(n);
        }
        if let Some(n) = value.get("total").and_then(Value::as_u64) {
            track.total = Some(n);
        }
        if let Some(unit) = text_of(value, "unit") {
            track.unit = Some(unit);
        }
        let waiting = match value.get("state").and_then(Value::as_str) {
            Some("waiting_for_user") if !track.waiting => {
                track.waiting = true;
                Waiting::Began
            }
            Some("running") if track.waiting => {
                track.waiting = false;
                Waiting::Ended
            }
            _ => Waiting::Same,
        };
        match kind {
            "begin" => {
                track.said = Some(now);
                track.pending = false;
                let title = if track.title.is_empty() {
                    plugin.to_owned()
                } else {
                    track.title.clone()
                };
                (
                    Some(Message::progress(lines::progress_started(&title, plugin))),
                    waiting,
                )
            }
            "end" => {
                let track = self.tracks.remove(token).unwrap_or_default();
                let title = if track.title.is_empty() {
                    plugin.to_owned()
                } else {
                    track.title.clone()
                };
                // The end says its own words, or that the job is done; the
                // words of the last report are not repeated.
                let text = match text_of(value, "message") {
                    Some(message) => lines::with_source(&message, plugin),
                    None => lines::progress_done(&title, plugin),
                };
                (Some(Message::progress(text)), Waiting::Ended)
            }
            _ => {
                if waiting == Waiting::Began {
                    track.said = Some(now);
                    track.pending = false;
                    let text = track
                        .message
                        .clone()
                        .unwrap_or_else(|| lines::waiting_for_you(&track.title));
                    return (
                        Some(Message::warning(lines::with_source(&text, plugin))),
                        waiting,
                    );
                }
                if track
                    .said
                    .is_some_and(|t| now.saturating_sub(t) < PROGRESS_EVERY)
                {
                    track.pending = true;
                    return (None, waiting);
                }
                track.said = Some(now);
                track.pending = false;
                (Some(Message::progress(sentence(track, plugin))), waiting)
            }
        }
    }

    /// Sentences held back by the twice-a-second limit, now that their
    /// time has come.
    pub fn flush(&mut self, plugin: &str, now: Duration) -> Vec<Message> {
        let mut out = Vec::new();
        let mut tokens: Vec<&String> = self.tracks.keys().collect();
        tokens.sort();
        let tokens: Vec<String> = tokens.into_iter().cloned().collect();
        for token in tokens {
            if let Some(track) = self.tracks.get_mut(&token)
                && track.pending
                && track
                    .said
                    .is_none_or(|t| now.saturating_sub(t) >= PROGRESS_EVERY)
            {
                track.pending = false;
                track.said = Some(now);
                out.push(Message::progress(sentence(track, plugin)));
            }
        }
        out
    }

    /// Forgets a token (its request was answered).
    pub fn forget(&mut self, token: &str) {
        self.tracks.remove(token);
    }

    /// The tokens it follows.
    pub fn tokens(&self) -> Vec<String> {
        self.tracks.keys().cloned().collect()
    }
}

/// The sentence of a report: the plugin's words, with the count when they
/// hold no number of their own, and the plugin's name last.
fn sentence(track: &Track, plugin: &str) -> String {
    let words = track.message.clone().unwrap_or_else(|| {
        if track.title.is_empty() {
            "Working".to_owned()
        } else {
            track.title.clone()
        }
    });
    let has_number = words.chars().any(|c| c.is_ascii_digit());
    let text = match (track.current, track.total, has_number) {
        (Some(current), Some(total), false) => {
            let unit = track.unit.as_deref().unwrap_or("");
            format!("{words} {current} of {total} {unit}")
                .trim_end()
                .to_owned()
        }
        (Some(current), None, false) => {
            let unit = track.unit.as_deref().unwrap_or("");
            format!("{words} {current} {unit}").trim_end().to_owned()
        }
        _ => words,
    };
    lines::progress_line(&text, plugin)
}

/// A plugin's settings for `initialize` and `settings/changed`: each
/// declared setting's default, overridden by the owner's values for keys
/// the manifest declares; other keys are left out.
pub fn settings_for(manifest: &Manifest, owner: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for setting in &manifest.settings {
        let default = serde_json::to_value(&setting.default).unwrap_or(Value::Null);
        let value = owner.get(&setting.key).cloned().unwrap_or(default);
        out.insert(setting.key.clone(), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn text_is_cleaned_of_hidden_characters_and_escapes() {
        let dirty =
            "Moved\u{202E} to\u{200B} the \u{1B}[31mtrash\u{1B}[0m\n\tnow\u{7}\u{1B}]0;title\u{7}!";
        assert_eq!(clean_text(dirty, 300), "Moved to the trash now!");
        assert_eq!(clean_text("  spaced   out  ", 300), "spaced out");
        assert_eq!(clean_text(&"a".repeat(500), 300).len(), 300);
        assert_eq!(clean_text("\u{E000}\u{F8FF}", 300), "");
    }

    #[test]
    fn the_plugins_name_comes_last() {
        assert_eq!(
            attribute("Lines: 42, files: 1", "Line counter"),
            "Lines: 42, files: 1. By Line counter."
        );
        assert_eq!(attribute("Done!", "Line counter"), "Done! By Line counter.");
    }

    /// The `spoof` case: text that imitates the app is still attributed,
    /// so it cannot pass for a built-in confirmation.
    #[test]
    fn a_plugin_cannot_pass_for_the_app() {
        let params = json!({"text": "Permanent delete confirmed", "importance": "warning"});
        let message = announcement(params.as_object().unwrap(), "Line counter").unwrap();
        assert_eq!(
            message,
            Message::warning("Permanent delete confirmed. By Line counter.")
        );
        let hidden = json!({"text": "\u{1B}[2J\u{202E}Moved to the trash"});
        let message = announcement(hidden.as_object().unwrap(), "Line counter").unwrap();
        assert_eq!(message.text, "Moved to the trash. By Line counter.");
        assert!(announcement(json!({"text": "\u{200B}"}).as_object().unwrap(), "X").is_none());
    }

    /// The `flood` case: a burst of a hundred.
    #[test]
    fn a_flood_of_announcements_is_folded() {
        let mut limiter = AnnounceLimiter::new(at(0));
        let admitted = (0..100).filter(|_| limiter.admit(at(0))).count();
        assert_eq!(admitted, 20);
        assert_eq!(limiter.fold(at(500), "Line counter"), None);
        assert_eq!(
            limiter.fold(at(1000), "Line counter"),
            Some(Message::status("80 more messages from Line counter"))
        );
        assert_eq!(limiter.fold(at(3000), "Line counter"), None);
        // Two seconds on, the bucket is full again.
        let later = (0..25).filter(|_| limiter.admit(at(2000))).count();
        assert_eq!(later, 20);
    }

    #[test]
    fn progress_becomes_the_jobs_sentence() {
        let mut board = ProgressBoard::default();
        let begin = json!({"kind": "begin", "title": "Count lines"});
        let (said, _) = board.update("t1", &begin, "Line counter", at(0));
        assert_eq!(
            said,
            Some(Message::progress("Count lines: started (Line counter)"))
        );
        let report = json!({"kind": "report", "message": "Counting", "current": 12, "total": 300, "unit": "files"});
        let (said, _) = board.update("t1", &report, "Line counter", at(600));
        assert_eq!(
            said,
            Some(Message::progress("Counting 12 of 300 files (Line counter)"))
        );
        // Twice a second at most: this one waits for the flush.
        let next = json!({"kind": "report", "current": 13});
        assert_eq!(board.update("t1", &next, "Line counter", at(700)).0, None);
        assert!(board.flush("Line counter", at(800)).is_empty());
        assert_eq!(
            board.flush("Line counter", at(1100)),
            [Message::progress("Counting 13 of 300 files (Line counter)")]
        );
        let end = json!({"kind": "end"});
        let (said, waiting) = board.update("t1", &end, "Line counter", at(2000));
        assert_eq!(
            said,
            Some(Message::progress("Count lines done (Line counter)"))
        );
        assert_eq!(waiting, Waiting::Ended);
    }

    #[test]
    fn waiting_for_the_owner_is_said_once_at_once() {
        let mut board = ProgressBoard::default();
        board.update(
            "t1",
            &json!({"kind": "begin", "title": "Copy to phone"}),
            "MTP",
            at(0),
        );
        let wait = json!({"kind": "report", "state": "waiting_for_user", "message": "Unlock the phone and allow file transfer"});
        let (said, waiting) = board.update("t1", &wait, "MTP", at(100));
        assert_eq!(waiting, Waiting::Began);
        assert_eq!(
            said,
            Some(Message::warning(
                "Unlock the phone and allow file transfer (MTP)"
            ))
        );
        let (again, waiting) = board.update("t1", &wait, "MTP", at(5000));
        assert_eq!(waiting, Waiting::Same);
        assert_ne!(
            again.map(|m| m.importance),
            Some(diroxide_core::Importance::High)
        );
        let (_, waiting) = board.update(
            "t1",
            &json!({"kind": "report", "state": "running"}),
            "MTP",
            at(6000),
        );
        assert_eq!(waiting, Waiting::Ended);
    }

    #[test]
    fn settings_take_defaults_and_the_owners_values() {
        let manifest = Manifest::check(
            "[plugin]\nid = \"xy\"\nname = \"X\"\nversion = \"0.1.0\"\nprotocol = 1\n[run]\ncommand = [\"x\"]\n[[setting]]\nkey = \"skip_blank\"\ntype = \"bool\"\ndefault = false\n[[setting]]\nkey = \"depth\"\ntype = \"int\"\ndefault = 3\n",
        )
        .manifest
        .unwrap();
        let mut owner = Map::new();
        owner.insert("depth".into(), json!(5));
        owner.insert("unknown".into(), json!("kept out"));
        assert_eq!(
            Value::Object(settings_for(&manifest, &owner)),
            json!({"skip_blank": false, "depth": 5})
        );
    }
}
