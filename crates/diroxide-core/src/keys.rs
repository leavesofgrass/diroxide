//! Keys, keymap layers and keymap columns, the same for every frontend. A
//! frontend turns its own key events into [`Key`]s; the keymap turns a key
//! into a command.
//!
//! Keys are written as modifiers and a key joined by plus signs:
//! `ctrl+alt+t`, `shift+f10`, `space`, `ctrl+plus`. A binding is one key
//! with its modifiers. There are no sequences and no leader keys (the
//! owner's decision; plan question 56): two keys written one after the
//! other, such as `ctrl+x ctrl+f`, are refused with
//! [`CoreError::KeySequence`], so a keymap file that holds one says so in
//! words.
//!
//! `keymap.toml` is command to key, one line per command, in a table per
//! column and layer (ADR-0005):
//!
//! ```toml
//! [terminal.panel]
//! "file.copy" = "f5"
//! "file.trash" = ["delete", "f8"]
//! "view.sort-list" = "none"
//!
//! [gui.panel]
//! "view.sort-list" = "ctrl+shift+s"
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Modifier keys held with a key.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Alt, or Option on a Mac.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// The Windows, Super or Command key.
    pub meta: bool,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };
    /// Control alone.
    pub const CTRL: Modifiers = Modifiers {
        ctrl: true,
        ..Self::NONE
    };
    /// Alt alone.
    pub const ALT: Modifiers = Modifiers {
        alt: true,
        ..Self::NONE
    };
    /// Shift alone.
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..Self::NONE
    };

    /// Whether no modifier is held.
    pub const fn is_empty(self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.meta)
    }
}

/// A key on the keyboard, without modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyCode {
    /// A character key; letters are stored lowercase, with Shift in the
    /// modifiers. The space bar is `Char(' ')`.
    Char(char),
    /// A function key, F1 to F24.
    F(u8),
    /// Enter.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Insert, which screen readers use as their modifier; diroxide binds
    /// nothing to it by default.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// The context menu (Applications) key.
    Menu,
    /// Plus on the numeric keypad.
    KeypadPlus,
    /// Minus on the numeric keypad.
    KeypadMinus,
    /// Star on the numeric keypad.
    KeypadMultiply,
    /// Slash on the numeric keypad.
    KeypadDivide,
}

const NAMED_KEYS: [(&str, KeyCode, &str); 21] = [
    ("enter", KeyCode::Enter, "Enter"),
    ("escape", KeyCode::Escape, "Escape"),
    ("tab", KeyCode::Tab, "Tab"),
    ("backspace", KeyCode::Backspace, "Backspace"),
    ("delete", KeyCode::Delete, "Delete"),
    ("insert", KeyCode::Insert, "Insert"),
    ("home", KeyCode::Home, "Home"),
    ("end", KeyCode::End, "End"),
    ("pageup", KeyCode::PageUp, "Page Up"),
    ("pagedown", KeyCode::PageDown, "Page Down"),
    ("up", KeyCode::Up, "Up"),
    ("down", KeyCode::Down, "Down"),
    ("left", KeyCode::Left, "Left"),
    ("right", KeyCode::Right, "Right"),
    ("menu", KeyCode::Menu, "Menu"),
    ("kp-plus", KeyCode::KeypadPlus, "Keypad Plus"),
    ("kp-minus", KeyCode::KeypadMinus, "Keypad Minus"),
    ("kp-multiply", KeyCode::KeypadMultiply, "Keypad Star"),
    ("kp-divide", KeyCode::KeypadDivide, "Keypad Slash"),
    ("space", KeyCode::Char(' '), "Space"),
    ("plus", KeyCode::Char('+'), "Plus"),
];

const ALIASES: [(&str, &str); 9] = [
    ("return", "enter"),
    ("esc", "escape"),
    ("del", "delete"),
    ("ins", "insert"),
    ("page-up", "pageup"),
    ("pgup", "pageup"),
    ("page-down", "pagedown"),
    ("pgdn", "pagedown"),
    ("apps", "menu"),
];

/// A key with its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Key {
    /// The key.
    pub code: KeyCode,
    /// The modifiers held.
    pub mods: Modifiers,
}

impl Key {
    /// A key with modifiers, normalized: an uppercase letter becomes the
    /// lowercase letter with Shift. Frontends build keys with this.
    pub fn new(code: KeyCode, mods: Modifiers) -> Self {
        match code {
            KeyCode::Char(c) if c.is_uppercase() => {
                let lower = c.to_lowercase().next().unwrap_or(c);
                Self {
                    code: KeyCode::Char(lower),
                    mods: Modifiers {
                        shift: true,
                        ..mods
                    },
                }
            }
            _ => Self { code, mods },
        }
    }

    /// A key with no modifiers.
    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, Modifiers::NONE)
    }

    /// A key with Control.
    pub fn ctrl(code: KeyCode) -> Self {
        Self::new(code, Modifiers::CTRL)
    }

    /// Reads a key such as `ctrl+alt+t`, `F5` or `shift+tab`. Two keys one
    /// after the other (`ctrl+x ctrl+f`) are refused as a sequence.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        let bad = || CoreError::BadKey(text.to_owned());
        let trimmed = text.trim();
        if trimmed.contains(char::is_whitespace) {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() > 1 && parts.iter().all(|part| Self::parse(part).is_ok()) {
                return Err(CoreError::KeySequence(text.to_owned()));
            }
            return Err(bad());
        }
        let (mods_text, key_text) = match trimmed.rsplit_once('+') {
            Some((mods, "")) if mods.ends_with('+') || mods.is_empty() => {
                (mods.strip_suffix('+').unwrap_or(mods), "+")
            }
            Some((mods, key)) => (mods, key),
            None => ("", trimmed),
        };
        let mut mods = Modifiers::NONE;
        if !mods_text.is_empty() {
            for part in mods_text.split('+') {
                match part.trim().to_lowercase().as_str() {
                    "ctrl" | "control" => mods.ctrl = true,
                    "alt" | "option" => mods.alt = true,
                    "shift" => mods.shift = true,
                    "meta" | "super" | "win" | "cmd" | "command" => mods.meta = true,
                    _ => return Err(bad()),
                }
            }
        }
        let mut chars = key_text.chars();
        let code = match (chars.next(), chars.next()) {
            // Letters are read without regard to case, so "Ctrl+X" means
            // Control and x; Shift is written out when it is meant.
            (Some(c), None) if !c.is_whitespace() && !c.is_control() => {
                KeyCode::Char(c.to_lowercase().next().unwrap_or(c))
            }
            (Some(_), Some(_)) => {
                let name = key_text.trim().to_lowercase();
                let name = ALIASES
                    .iter()
                    .find(|(alias, _)| *alias == name)
                    .map_or(name.as_str(), |(_, canonical)| canonical);
                if let Some((_, code, _)) = NAMED_KEYS.iter().find(|(n, _, _)| *n == name) {
                    *code
                } else if let Some(number) = name.strip_prefix('f') {
                    match number.parse::<u8>() {
                        Ok(n @ 1..=24) => KeyCode::F(n),
                        _ => return Err(bad()),
                    }
                } else {
                    return Err(bad());
                }
            }
            _ => return Err(bad()),
        };
        Ok(Self::new(code, mods))
    }

    /// Whether this is a single printable character with no Control, Alt
    /// or Meta: a single-character shortcut, which WCAG 2.1.4 lets the user
    /// turn off.
    pub fn is_single_character(&self) -> bool {
        matches!(self.code, KeyCode::Char(c) if c != ' ')
            && !(self.mods.ctrl || self.mods.alt || self.mods.meta)
    }

    /// The text the key types, if it is a printable character with at most
    /// Shift: what type-ahead and letter jump use.
    pub fn typed_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !(self.mods.ctrl || self.mods.alt || self.mods.meta) => {
                if self.mods.shift {
                    c.to_uppercase().next()
                } else {
                    Some(c)
                }
            }
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    /// Writes the key as it is said: "Ctrl+Alt+T", "Shift+F10", "Space".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mods = [
            (self.mods.ctrl, "Ctrl"),
            (self.mods.alt, "Alt"),
            (self.mods.shift, "Shift"),
            (self.mods.meta, "Meta"),
        ];
        for (held, name) in mods {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match self.code {
            KeyCode::F(n) => write!(f, "F{n}"),
            KeyCode::Char(c) => match NAMED_KEYS.iter().find(|(_, code, _)| *code == self.code) {
                Some((_, _, name)) => f.write_str(name),
                None => write!(f, "{}", c.to_uppercase()),
            },
            other => match NAMED_KEYS.iter().find(|(_, code, _)| *code == other) {
                Some((_, _, name)) => f.write_str(name),
                None => write!(f, "{other:?}"),
            },
        }
    }
}

impl TryFrom<String> for Key {
    type Error = CoreError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Key> for String {
    fn from(key: Key) -> Self {
        key.to_string()
    }
}

/// The two columns of the keymap: the keys a terminal can deliver, and the
/// keys of the GUI. Each command has a binding in each column (plan
/// section 5.7); the GUI column uses Command for Control on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyColumn {
    /// The terminal frontend, which never uses keys a terminal cannot
    /// deliver (the unsafe-key table).
    Terminal,
    /// The GUI frontend.
    Gui,
}

impl KeyColumn {
    /// Both columns, the terminal first.
    pub const ALL: [KeyColumn; 2] = [KeyColumn::Terminal, KeyColumn::Gui];

    /// The column's name, as written in `keymap.toml`.
    pub const fn word(self) -> &'static str {
        match self {
            KeyColumn::Terminal => "terminal",
            KeyColumn::Gui => "gui",
        }
    }

    /// The column's name in a sentence: "terminal" or "GUI".
    pub const fn in_words(self) -> &'static str {
        match self {
            KeyColumn::Terminal => "terminal",
            KeyColumn::Gui => "GUI",
        }
    }
}

impl fmt::Display for KeyColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// Where a binding applies. A key is looked up in the most specific layer
/// first: a dialog, then the viewer, the preview or a list, then a panel,
/// then global.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    /// Everywhere.
    Global,
    /// In a file panel.
    Panel,
    /// In any list, including menus and choosers.
    List,
    /// In the viewer.
    Viewer,
    /// In the quick view's preview region, which Tab enters (its own keys
    /// for headings, tables, cells and links; the viewer's where they mean
    /// the same).
    Preview,
    /// In a dialog.
    Dialog,
}

impl Layer {
    /// Every layer, from the most general.
    pub const ALL: [Layer; 6] = [
        Layer::Global,
        Layer::Panel,
        Layer::List,
        Layer::Viewer,
        Layer::Preview,
        Layer::Dialog,
    ];

    /// The layer's name, as written in `keymap.toml`.
    pub const fn word(self) -> &'static str {
        match self {
            Layer::Global => "global",
            Layer::Panel => "panel",
            Layer::List => "list",
            Layer::Viewer => "viewer",
            Layer::Preview => "preview",
            Layer::Dialog => "dialog",
        }
    }
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    #[test]
    fn keys_parse_and_display() {
        assert_eq!(key("ctrl+alt+t").to_string(), "Ctrl+Alt+T");
        assert_eq!(key("Shift+F10").to_string(), "Shift+F10");
        assert_eq!(key("space").to_string(), "Space");
        assert_eq!(key("ctrl+plus"), key("ctrl++"));
        assert_eq!(key("ctrl+plus").to_string(), "Ctrl+Plus");
        assert_eq!(key("+").code, KeyCode::Char('+'));
        assert_eq!(key("pgdn").code, KeyCode::PageDown);
        assert_eq!(key("kp-multiply").to_string(), "Keypad Star");
        assert_eq!(key("F24").code, KeyCode::F(24));
        assert_eq!(key("ctrl+-").code, KeyCode::Char('-'));
    }

    #[test]
    fn written_letters_ignore_case_and_events_carry_shift() {
        assert_eq!(key("A"), key("a"));
        assert_eq!(key("Ctrl+X"), key("ctrl+x"));
        assert_eq!(key("Ctrl+Shift+T").to_string(), "Ctrl+Shift+T");
        assert_eq!(
            Key::new(KeyCode::Char('A'), Modifiers::NONE),
            key("shift+a")
        );
        assert_eq!(
            Key::new(KeyCode::Char('T'), Modifiers::CTRL).to_string(),
            "Ctrl+Shift+T"
        );
        assert_eq!(key("shift+a").typed_char(), Some('A'));
        assert_eq!(key("ctrl+a").typed_char(), None);
    }

    #[test]
    fn bad_keys_are_refused() {
        for bad in ["", "ctrl+", "hyper+a", "f0", "f25", "nonsense", "ctrl+ab"] {
            assert!(Key::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn single_character_shortcuts_are_recognized() {
        assert!(key("a").is_single_character());
        assert!(key("shift+a").is_single_character());
        assert!(!key("ctrl+a").is_single_character());
        assert!(!key("space").is_single_character());
        assert!(!key("f5").is_single_character());
    }

    #[test]
    fn sequences_are_refused_in_words() {
        assert_eq!(
            Key::parse("ctrl+x ctrl+f"),
            Err(CoreError::KeySequence("ctrl+x ctrl+f".into()))
        );
        assert_eq!(
            Key::parse("ctrl+x nonsense"),
            Err(CoreError::BadKey("ctrl+x nonsense".into()))
        );
        assert_eq!(Key::parse("  f5  ").unwrap(), key("f5"));
        assert!(Key::parse("  ").is_err());
        let text: String = key("ctrl+shift+s").into();
        assert_eq!(Key::try_from(text).unwrap(), key("ctrl+shift+s"));
    }

    #[test]
    fn columns_are_named() {
        assert_eq!(KeyColumn::Terminal.word(), "terminal");
        assert_eq!(KeyColumn::ALL[1].to_string(), "gui");
    }
}
