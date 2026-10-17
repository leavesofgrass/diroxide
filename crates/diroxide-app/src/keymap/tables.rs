//! The three key tables, kept as data files in `crates/diroxide-app/keys/`
//! and built into the program: the keys a terminal cannot be trusted with
//! (`terminal-unsafe.toml`), the keys the screen readers keep
//! (`screen-readers.toml`), and the keys reserved for later alphas
//! (`reserved.toml`). The keymap's checks, `diro keys check`, the Keys list
//! of alpha.3 and the keyboard page all read them from here.

use std::sync::OnceLock;

use diroxide_core::{CommandId, Key, KeyCode, KeyColumn};
use serde::Deserialize;

use super::names::Platform;

/// The terminal table's text.
pub const TERMINAL_UNSAFE_TOML: &str = include_str!("../../keys/terminal-unsafe.toml");
/// The screen reader table's text.
pub const SCREEN_READERS_TOML: &str = include_str!("../../keys/screen-readers.toml");
/// The reserved keys' text.
pub const RESERVED_TOML: &str = include_str!("../../keys/reserved.toml");

/// A family of keys named by a rule rather than listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyPattern {
    /// Insert with any modifiers.
    Insert,
    /// The numeric keypad's plus, minus, star and slash.
    Keypad,
    /// Control and Shift with a letter.
    CtrlShiftLetter,
    /// Alt with a digit.
    AltDigit,
    /// Control and Alt with a digit.
    CtrlAltDigit,
    /// Shift with F1 to F12.
    ShiftFKey,
    /// Control and Alt with any key.
    CtrlAltAny,
}

impl KeyPattern {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "insert" => KeyPattern::Insert,
            "keypad" => KeyPattern::Keypad,
            "ctrl+shift+letter" => KeyPattern::CtrlShiftLetter,
            "alt+digit" => KeyPattern::AltDigit,
            "ctrl+alt+digit" => KeyPattern::CtrlAltDigit,
            "shift+fkey" => KeyPattern::ShiftFKey,
            "ctrl+alt+any" => KeyPattern::CtrlAltAny,
            _ => return None,
        })
    }

    /// Whether a key is in the family.
    pub fn matches(self, key: &Key) -> bool {
        let m = key.mods;
        match self {
            KeyPattern::Insert => key.code == KeyCode::Insert,
            KeyPattern::Keypad => matches!(
                key.code,
                KeyCode::KeypadPlus
                    | KeyCode::KeypadMinus
                    | KeyCode::KeypadMultiply
                    | KeyCode::KeypadDivide
            ),
            KeyPattern::CtrlShiftLetter => {
                matches!(key.code, KeyCode::Char(c) if c.is_ascii_lowercase())
                    && m.ctrl
                    && m.shift
                    && !m.alt
                    && !m.meta
            }
            KeyPattern::AltDigit => {
                matches!(key.code, KeyCode::Char(c) if c.is_ascii_digit())
                    && m.alt
                    && !m.ctrl
                    && !m.meta
            }
            KeyPattern::CtrlAltDigit => {
                matches!(key.code, KeyCode::Char(c) if c.is_ascii_digit()) && m.ctrl && m.alt
            }
            KeyPattern::ShiftFKey => {
                matches!(key.code, KeyCode::F(1..=12)) && m.shift && !m.ctrl && !m.alt && !m.meta
            }
            KeyPattern::CtrlAltAny => m.ctrl && m.alt,
        }
    }

    /// The family in words, for the keyboard page.
    pub const fn words(self) -> &'static str {
        match self {
            KeyPattern::Insert => "Insert, with any key held",
            KeyPattern::Keypad => "The keypad's Plus, Minus, Star and Slash",
            KeyPattern::CtrlShiftLetter => "Ctrl+Shift with a letter",
            KeyPattern::AltDigit => "Alt with a digit",
            KeyPattern::CtrlAltDigit => "Ctrl+Alt with a digit",
            KeyPattern::ShiftFKey => "Shift with F1 to F12",
            KeyPattern::CtrlAltAny => "Ctrl+Option with any key",
        }
    }
}

/// How a terminal treats a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerminalKind {
    /// The key cannot reach diroxide in a terminal; the terminal column
    /// never uses it.
    Unsafe,
    /// A terminal program may keep it; a binding needs a menu as the sure
    /// way.
    Program,
    /// Some terminals send it as another key; a binding needs a menu as the
    /// sure way.
    Differs,
}

/// One row of the terminal table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalEntry {
    /// The keys listed.
    pub keys: Vec<Key>,
    /// The family, when the row names one instead.
    pub pattern: Option<KeyPattern>,
    /// What the terminal does.
    pub kind: TerminalKind,
    /// The program that keeps the key, for [`TerminalKind::Program`].
    pub program: String,
    /// Why, finishing the sentence said in words.
    pub why: String,
    /// Where the fact comes from.
    pub source: String,
}

impl TerminalEntry {
    /// Whether the row covers a key.
    pub fn covers(&self, key: &Key) -> bool {
        self.keys.contains(key) || self.pattern.is_some_and(|p| p.matches(key))
    }
}

/// How a screen reader treats a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReaderKind {
    /// The reader takes the key first.
    Owned,
    /// The reader takes it only with Num Lock off.
    NumLock,
}

/// One row of the screen reader table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderEntry {
    /// The keys listed.
    pub keys: Vec<Key>,
    /// The family, when the row names one instead.
    pub pattern: Option<KeyPattern>,
    /// The readers, in words: "NVDA, JAWS and Narrator".
    pub readers: String,
    /// What for, finishing the keyboard page's line: "to start NVDA".
    pub purpose: String,
    /// Whether the reader always takes it.
    pub kind: ReaderKind,
    /// The one system the row holds on, or `None` for every system.
    pub platform: Option<Platform>,
    /// Where the fact comes from.
    pub source: String,
}

impl ReaderEntry {
    /// Whether the row covers a key on a system.
    pub fn covers(&self, key: &Key, platform: Platform) -> bool {
        self.platform.is_none_or(|p| p == platform)
            && (self.keys.contains(key) || self.pattern.is_some_and(|p| p.matches(key)))
    }
}

/// One reserved key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedEntry {
    /// The keys.
    pub keys: Vec<Key>,
    /// The command they are kept for.
    pub command: CommandId,
    /// Its name, in words.
    pub name: String,
    /// The alpha that brings it: "alpha.3".
    pub alpha: String,
    /// The column, or `None` for both.
    pub column: Option<KeyColumn>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTerminal {
    #[serde(default)]
    keys: Vec<String>,
    pattern: Option<String>,
    kind: String,
    #[serde(default)]
    program: String,
    why: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReader {
    #[serde(default)]
    keys: Vec<String>,
    pattern: Option<String>,
    readers: String,
    purpose: String,
    kind: String,
    platform: Option<String>,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReserved {
    keys: Vec<String>,
    command: String,
    name: String,
    alpha: String,
    column: String,
}

#[derive(Deserialize)]
struct RawFile<T> {
    key: Vec<T>,
}

/// The tables, read once, with what could not be read in words (the tests
/// require none).
struct Tables {
    terminal: Vec<TerminalEntry>,
    readers: Vec<ReaderEntry>,
    reserved: Vec<ReservedEntry>,
    problems: Vec<String>,
}

fn parse_keys(file: &str, texts: &[String], problems: &mut Vec<String>) -> Vec<Key> {
    texts
        .iter()
        .filter_map(|text| match Key::parse(text) {
            Ok(key) => Some(key),
            Err(e) => {
                problems.push(format!("{file}: {e}"));
                None
            }
        })
        .collect()
}

fn parse_pattern(file: &str, name: Option<&str>, problems: &mut Vec<String>) -> Option<KeyPattern> {
    let name = name?;
    let pattern = KeyPattern::from_name(name);
    if pattern.is_none() {
        problems.push(format!("{file}: unknown pattern {name}"));
    }
    pattern
}

fn read_tables() -> Tables {
    let mut problems = Vec::new();
    let mut terminal = Vec::new();
    let file = "terminal-unsafe.toml";
    match toml::from_str::<RawFile<RawTerminal>>(TERMINAL_UNSAFE_TOML) {
        Ok(raw) => {
            for row in raw.key {
                let kind = match row.kind.as_str() {
                    "unsafe" => TerminalKind::Unsafe,
                    "program" => TerminalKind::Program,
                    "differs" => TerminalKind::Differs,
                    other => {
                        problems.push(format!("{file}: unknown kind {other}"));
                        continue;
                    }
                };
                terminal.push(TerminalEntry {
                    keys: parse_keys(file, &row.keys, &mut problems),
                    pattern: parse_pattern(file, row.pattern.as_deref(), &mut problems),
                    kind,
                    program: row.program,
                    why: row.why,
                    source: row.source,
                });
            }
        }
        Err(e) => problems.push(format!("{file}: {e}")),
    }
    let mut readers = Vec::new();
    let file = "screen-readers.toml";
    match toml::from_str::<RawFile<RawReader>>(SCREEN_READERS_TOML) {
        Ok(raw) => {
            for row in raw.key {
                let kind = match row.kind.as_str() {
                    "owned" => ReaderKind::Owned,
                    "num-lock" => ReaderKind::NumLock,
                    other => {
                        problems.push(format!("{file}: unknown kind {other}"));
                        continue;
                    }
                };
                let platform = match row.platform.as_deref() {
                    None => None,
                    Some("macos") => Some(Platform::MacOs),
                    Some("linux") => Some(Platform::Other),
                    Some("windows") => Some(Platform::Windows),
                    Some(other) => {
                        problems.push(format!("{file}: unknown platform {other}"));
                        continue;
                    }
                };
                readers.push(ReaderEntry {
                    keys: parse_keys(file, &row.keys, &mut problems),
                    pattern: parse_pattern(file, row.pattern.as_deref(), &mut problems),
                    readers: row.readers,
                    purpose: row.purpose,
                    kind,
                    platform,
                    source: row.source,
                });
            }
        }
        Err(e) => problems.push(format!("{file}: {e}")),
    }
    let mut reserved = Vec::new();
    let file = "reserved.toml";
    match toml::from_str::<RawFile<RawReserved>>(RESERVED_TOML) {
        Ok(raw) => {
            for row in raw.key {
                let column = match row.column.as_str() {
                    "both" => None,
                    "terminal" => Some(KeyColumn::Terminal),
                    "window" => Some(KeyColumn::Window),
                    other => {
                        problems.push(format!("{file}: unknown column {other}"));
                        continue;
                    }
                };
                let Ok(command) = CommandId::new(row.command.clone()) else {
                    problems.push(format!("{file}: not a command id {}", row.command));
                    continue;
                };
                reserved.push(ReservedEntry {
                    keys: parse_keys(file, &row.keys, &mut problems),
                    command,
                    name: row.name,
                    alpha: row.alpha,
                    column,
                });
            }
        }
        Err(e) => problems.push(format!("{file}: {e}")),
    }
    Tables {
        terminal,
        readers,
        reserved,
        problems,
    }
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(read_tables)
}

/// The terminal table, in file order.
pub fn terminal_table() -> &'static [TerminalEntry] {
    &tables().terminal
}

/// The screen reader table, in file order.
pub fn screen_reader_table() -> &'static [ReaderEntry] {
    &tables().readers
}

/// The reserved keys, in file order.
pub fn reserved_table() -> &'static [ReservedEntry] {
    &tables().reserved
}

/// What could not be read from the three files, in words; empty in a
/// correct build.
pub fn table_problems() -> &'static [String] {
    &tables().problems
}

/// The terminal row for a key: an unsafe row first, then a program row,
/// then a differs row.
pub fn terminal_entry(key: &Key) -> Option<&'static TerminalEntry> {
    let rows = terminal_table();
    [
        TerminalKind::Unsafe,
        TerminalKind::Program,
        TerminalKind::Differs,
    ]
    .into_iter()
    .find_map(|kind| rows.iter().find(|r| r.kind == kind && r.covers(key)))
}

/// The screen reader rows for a key on a system.
pub fn reader_entries(key: &Key, platform: Platform) -> Vec<&'static ReaderEntry> {
    screen_reader_table()
        .iter()
        .filter(|r| r.covers(key, platform))
        .collect()
}

/// The reserved row for a key in a column.
pub fn reserved_entry(key: &Key, column: KeyColumn) -> Option<&'static ReservedEntry> {
    reserved_table()
        .iter()
        .find(|r| r.column.is_none_or(|c| c == column) && r.keys.contains(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    #[test]
    fn the_data_files_read_cleanly() {
        assert_eq!(table_problems(), &[] as &[String]);
        assert!(terminal_table().len() >= 30);
        assert!(screen_reader_table().len() >= 10);
        assert!(reserved_table().len() >= 7);
    }

    #[test]
    fn the_windows_terminal_keys_are_marked() {
        for text in [
            "ctrl+v",
            "ctrl+c",
            "alt+left",
            "alt+right",
            "alt+up",
            "alt+down",
            "alt+enter",
            "ctrl+plus",
            "ctrl+-",
            "ctrl+0",
        ] {
            let row = terminal_entry(&key(text)).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(row.kind, TerminalKind::Program, "{text}");
            assert_eq!(row.program, "Windows Terminal", "{text}");
        }
        for text in [
            "insert",
            "shift+insert",
            "kp-plus",
            "ctrl+i",
            "ctrl+m",
            "ctrl+h",
            "ctrl+j",
            "ctrl+[",
            "ctrl+space",
            "ctrl+enter",
            "shift+enter",
            "ctrl+tab",
            "ctrl+shift+v",
            "ctrl+shift+p",
            "f1",
            "f10",
            "f11",
            "f12",
            "alt+1",
        ] {
            let row = terminal_entry(&key(text)).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(row.kind, TerminalKind::Unsafe, "{text}");
        }
        assert_eq!(
            terminal_entry(&key("shift+f5")).map(|r| r.kind),
            Some(TerminalKind::Differs)
        );
        assert!(terminal_entry(&key("f5")).is_none());
        // The desktop app's built-in terminal keeps Alt+F and Ctrl+Q (the
        // owner's first try of alpha.1 there).
        for text in ["ctrl+q", "alt+f"] {
            let row = terminal_entry(&key(text)).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(row.kind, TerminalKind::Program, "{text}");
            assert_eq!(row.program, "The Claude app's terminal", "{text}");
        }
    }

    #[test]
    fn the_screen_reader_keys_are_found_by_system() {
        assert_eq!(
            reader_entries(&key("ctrl+alt+n"), Platform::Windows)[0].readers,
            "NVDA"
        );
        assert_eq!(
            reader_entries(&key("kp-plus"), Platform::Windows)[0].kind,
            ReaderKind::NumLock
        );
        assert!(reader_entries(&key("meta+f5"), Platform::Windows).is_empty());
        assert_eq!(reader_entries(&key("meta+f5"), Platform::MacOs).len(), 1);
        assert_eq!(reader_entries(&key("ctrl+alt+x"), Platform::MacOs).len(), 1);
        assert!(reader_entries(&key("ctrl+alt+x"), Platform::Windows).is_empty());
        assert!(reader_entries(&key("f5"), Platform::Other).is_empty());
    }

    #[test]
    fn the_later_alphas_keys_are_reserved() {
        for text in ["ctrl+f", "ctrl+t", "ctrl+w"] {
            assert!(
                reserved_entry(&key(text), KeyColumn::Terminal).is_some(),
                "{text}"
            );
        }
        assert!(reserved_entry(&key("ctrl+tab"), KeyColumn::Window).is_some());
        assert!(reserved_entry(&key("ctrl+tab"), KeyColumn::Terminal).is_none());
    }
}
