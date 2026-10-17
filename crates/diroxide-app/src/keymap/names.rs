//! Keys in words: the written form for menus, rows and the keyboard page
//! ("Ctrl+F5", "Alt+Period"), and the spoken form for messages and the key
//! test ("Control plus F5", "Alt plus Period"). Both come from the live
//! keymap's keys, never from fixed text, so a rebound key is said as it is
//! now (textweaver's lesson).

use diroxide_core::{Key, KeyCode};

/// The system the keys are pressed on, which names the Meta key and
/// decides the macOS window column (Command for Control).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// Linux and the rest.
    Other,
}

impl Platform {
    /// The system this program was built for.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Other
        }
    }

    /// The platform's name in `screen-readers.toml`.
    pub const fn word(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::MacOs => "macos",
            Platform::Other => "linux",
        }
    }
}

fn written_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char('+') => "Plus".to_owned(),
        KeyCode::Char('-') => "Minus".to_owned(),
        KeyCode::Char('.') => "Period".to_owned(),
        KeyCode::Char(',') => "Comma".to_owned(),
        KeyCode::Char(c) => c.to_uppercase().collect(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Escape => "Escape".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Delete => "Delete".to_owned(),
        KeyCode::Insert => "Insert".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "Page Up".to_owned(),
        KeyCode::PageDown => "Page Down".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Menu => "Menu".to_owned(),
        KeyCode::KeypadPlus => "Keypad Plus".to_owned(),
        KeyCode::KeypadMinus => "Keypad Minus".to_owned(),
        KeyCode::KeypadMultiply => "Keypad Star".to_owned(),
        KeyCode::KeypadDivide => "Keypad Slash".to_owned(),
    }
}

fn spoken_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char('*') => "Star".to_owned(),
        KeyCode::Char('/') => "Slash".to_owned(),
        KeyCode::Char('?') => "Question mark".to_owned(),
        KeyCode::Char('\\') => "Backslash".to_owned(),
        KeyCode::Char('`') => "Backquote".to_owned(),
        KeyCode::Char('[') => "Left bracket".to_owned(),
        KeyCode::Char(']') => "Right bracket".to_owned(),
        KeyCode::Char(';') => "Semicolon".to_owned(),
        KeyCode::Char('\'') => "Quote".to_owned(),
        KeyCode::Char('=') => "Equals".to_owned(),
        KeyCode::Up => "Up arrow".to_owned(),
        KeyCode::Down => "Down arrow".to_owned(),
        KeyCode::Left => "Left arrow".to_owned(),
        KeyCode::Right => "Right arrow".to_owned(),
        KeyCode::Menu => "Menu key".to_owned(),
        other => written_code(other),
    }
}

fn meta_word(platform: Platform, spoken: bool) -> &'static str {
    match (platform, spoken) {
        (Platform::MacOs, true) => "Command",
        (Platform::MacOs, false) => "Cmd",
        (Platform::Windows, true) => "Windows key",
        (Platform::Windows, false) => "Win",
        (Platform::Other, _) => "Super",
    }
}

/// A key as menus, rows and the keyboard page write it: "Ctrl+F5",
/// "Shift+F3", "Alt+Period", "Keypad Plus". On macOS Meta is "Cmd".
pub fn written(key: &Key, platform: Platform) -> String {
    let mut parts: Vec<String> = Vec::new();
    if key.mods.ctrl {
        parts.push("Ctrl".to_owned());
    }
    if key.mods.meta {
        parts.push(meta_word(platform, false).to_owned());
    }
    if key.mods.alt {
        parts.push(
            if platform == Platform::MacOs {
                "Option"
            } else {
                "Alt"
            }
            .to_owned(),
        );
    }
    if key.mods.shift {
        parts.push("Shift".to_owned());
    }
    parts.push(written_code(key.code));
    parts.join("+")
}

/// A key as `keymap.toml` writes it: "ctrl+f5", "shift+tab", "kp-plus".
/// The typed name the Keys list takes is read back with
/// [`Key::parse`], so the two always agree.
pub fn for_file(key: &Key) -> String {
    diroxide_config::key_text_for_file(key)
}

/// A key as it is said: "Control plus F5", "Shift plus F3", "Alt plus
/// Period", "Question mark", "Up arrow". On macOS Meta is "Command".
pub fn spoken(key: &Key, platform: Platform) -> String {
    let mut parts: Vec<String> = Vec::new();
    if key.mods.ctrl {
        parts.push("Control".to_owned());
    }
    if key.mods.meta {
        parts.push(meta_word(platform, true).to_owned());
    }
    if key.mods.alt {
        parts.push(
            if platform == Platform::MacOs {
                "Option"
            } else {
                "Alt"
            }
            .to_owned(),
        );
    }
    if key.mods.shift {
        parts.push("Shift".to_owned());
    }
    parts.push(spoken_code(key.code));
    parts.join(" plus ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    #[test]
    fn keys_are_written_and_said() {
        let p = Platform::Windows;
        assert_eq!(written(&key("ctrl+f5"), p), "Ctrl+F5");
        assert_eq!(spoken(&key("ctrl+f5"), p), "Control plus F5");
        assert_eq!(written(&key("alt+."), p), "Alt+Period");
        assert_eq!(spoken(&key("alt+."), p), "Alt plus Period");
        assert_eq!(written(&key("plus"), p), "Plus");
        assert_eq!(written(&key("*"), p), "*");
        assert_eq!(spoken(&key("*"), p), "Star");
        assert_eq!(spoken(&key("?"), p), "Question mark");
        assert_eq!(written(&key("ctrl+shift+s"), p), "Ctrl+Shift+S");
        assert_eq!(spoken(&key("ctrl+shift+s"), p), "Control plus Shift plus S");
        assert_eq!(spoken(&key("alt+left"), p), "Alt plus Left arrow");
        assert_eq!(spoken(&key("menu"), p), "Menu key");
        assert_eq!(written(&key("kp-multiply"), p), "Keypad Star");
        assert_eq!(spoken(&key("space"), p), "Space");
        assert_eq!(written(&key("meta+q"), Platform::MacOs), "Cmd+Q");
        assert_eq!(spoken(&key("meta+q"), Platform::MacOs), "Command plus Q");
        assert_eq!(
            spoken(&key("alt+home"), Platform::MacOs),
            "Option plus Home"
        );
        assert_eq!(
            spoken(&key("meta+e"), Platform::Windows),
            "Windows key plus E"
        );
    }
}
