//! The window's keys as diroxide's [`Key`]s: what a key press in the window
//! becomes before the app's keymap reads it (the GUI column).
//!
//! The rules (R26 section 7; textweaver's `crates/textweaver-xilem/src/keys.rs`,
//! `chord()`, the owner's own project, is the model):
//!
//! - A printable key arrives as the character the layout typed (`>`
//!   rather than Shift with `.`), which is what the keymap stores; so a
//!   character that is not a letter drops its Shift, and a capital letter
//!   becomes the small letter with Shift, as the terminal's keys do.
//! - With Control, Alt or (on macOS) Command, a letter is its small form
//!   with Shift kept as a modifier.
//! - The keypad's plus, minus, star and slash are the keypad's keys, told
//!   apart by where the key is (its location), not by its character.
//! - The context menu (Applications) key is the keymap's Menu.
//! - AltGr is text: a character typed with AltGr arrives on Windows as
//!   Control plus Alt with the character, and goes to the app as it came,
//!   which types it in a field or the quick search when nothing is bound
//!   to it (as the terminal's keys do); where the system marks AltGr
//!   itself, the character goes with no Control and no Alt.
//! - A key held down repeats only where a repeat means something: the
//!   arrows, Page Up, Page Down, Home and End, and in a text field
//!   Backspace, Delete and typed characters. Any other repeat is dropped,
//!   so a held F8 cannot open a second question.
//! - On macOS, Option types other characters (Option with S types a
//!   sharp s), so a chord with Option is read by the key's place on the
//!   keyboard, its code: Option with S is Alt+S.
//! - A modifier alone, Caps Lock, media keys and a key still being
//!   composed are not keys for the keymap; they give `None`.

use diroxide_app::keymap::Platform;
use diroxide_core::{Key, KeyCode, Modifiers};
use masonry::core::keyboard::{
    Code, Key as WinKey, KeyState, KeyboardEvent, Location, Modifiers as WinMods, NamedKey,
};

/// The diroxide key for a key event in the window on `platform`, or
/// `None` when the event is not a key for the keymap (a release, a
/// modifier alone, a key being composed, a repeat that means nothing).
/// `typing` is true while a text field has the focus, where held
/// Backspace, Delete and characters repeat.
pub fn to_key(event: &KeyboardEvent, platform: Platform, typing: bool) -> Option<Key> {
    if event.state != KeyState::Down || event.is_composing {
        return None;
    }
    let key = chord(event, platform)?;
    if event.repeat && !repeats(&key, typing) {
        return None;
    }
    Some(key)
}

/// Whether a held key repeats: movement keys always, and in a text field
/// the keys that edit it.
pub fn repeats(key: &Key, typing: bool) -> bool {
    match key.code {
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Home
        | KeyCode::End => true,
        KeyCode::Backspace | KeyCode::Delete => typing,
        KeyCode::Char(_) => typing && key.typed_char().is_some(),
        _ => false,
    }
}

/// The modifiers as the keymap reads them on `platform`: the Windows key
/// counts as Meta only on macOS (Command), where the keymap's macOS
/// column uses it; elsewhere it is the system's.
fn modifiers(mods: WinMods, platform: Platform) -> Modifiers {
    Modifiers {
        ctrl: mods.ctrl(),
        alt: mods.alt(),
        shift: mods.shift(),
        meta: mods.meta() && platform == Platform::MacOs,
    }
}

/// The letter or digit a key's place on the keyboard stands for (US
/// layout), for macOS Option chords.
fn code_char(code: Code) -> Option<char> {
    use Code::*;
    let c = match code {
        KeyA => 'a',
        KeyB => 'b',
        KeyC => 'c',
        KeyD => 'd',
        KeyE => 'e',
        KeyF => 'f',
        KeyG => 'g',
        KeyH => 'h',
        KeyI => 'i',
        KeyJ => 'j',
        KeyK => 'k',
        KeyL => 'l',
        KeyM => 'm',
        KeyN => 'n',
        KeyO => 'o',
        KeyP => 'p',
        KeyQ => 'q',
        KeyR => 'r',
        KeyS => 's',
        KeyT => 't',
        KeyU => 'u',
        KeyV => 'v',
        KeyW => 'w',
        KeyX => 'x',
        KeyY => 'y',
        KeyZ => 'z',
        Digit0 => '0',
        Digit1 => '1',
        Digit2 => '2',
        Digit3 => '3',
        Digit4 => '4',
        Digit5 => '5',
        Digit6 => '6',
        Digit7 => '7',
        Digit8 => '8',
        Digit9 => '9',
        Period => '.',
        Comma => ',',
        Slash => '/',
        Minus => '-',
        Equal => '=',
        _ => return None,
    };
    Some(c)
}

/// The key for a key event, before the press, composition and repeat
/// checks of [`to_key`]: `None` for a modifier alone or a key the keymap
/// never binds.
pub fn chord(event: &KeyboardEvent, platform: Platform) -> Option<Key> {
    if event.modifiers.meta() && platform != Platform::MacOs {
        // The Windows (Super) key's chords are the system's.
        return None;
    }
    let mut mods = modifiers(event.modifiers, platform);
    let command = mods.ctrl || mods.alt || mods.meta;
    let code = match &event.key {
        WinKey::Named(named) => named_code(named)?,
        WinKey::Character(text) => {
            let mut chars = text.chars();
            let c = chars.next()?;
            if chars.next().is_some() || c.is_control() {
                return None;
            }
            if event.location == Location::Numpad
                && let Some(code) = keypad_code(c)
            {
                // The keypad's own key: its Shift (and Num Lock's) is in
                // the character.
                mods.shift = false;
                return Some(Key::new(code, mods));
            }
            if event.modifiers.contains(WinMods::ALT_GRAPH) && !c.is_alphabetic() {
                // AltGr marked by the system: the character is typed text.
                mods.ctrl = false;
                mods.alt = false;
            }
            if platform == Platform::MacOs
                && mods.alt
                && let Some(base) = code_char(event.code)
            {
                // Option typed another character; the chord is the key's.
                return Some(Key::new(KeyCode::Char(base), mods));
            }
            if c == ' ' {
                KeyCode::Char(' ')
            } else if c.is_alphabetic() {
                // Key::new folds an uppercase letter into the small one
                // with Shift; with a modifier the small letter is meant.
                if command && c.is_uppercase() && !mods.shift {
                    KeyCode::Char(c.to_lowercase().next().unwrap_or(c))
                } else {
                    KeyCode::Char(c)
                }
            } else {
                // The character carries its own Shift ("?" is Shift with
                // slash on one layout and a key of its own on another).
                mods.shift = false;
                KeyCode::Char(c)
            }
        }
    };
    Some(Key::new(code, mods))
}

/// The keymap's code for a named key, or `None` for one it never binds
/// (a modifier, Caps Lock, a media key).
fn named_code(named: &NamedKey) -> Option<KeyCode> {
    let code = match named {
        NamedKey::Enter => KeyCode::Enter,
        NamedKey::Escape => KeyCode::Escape,
        NamedKey::Tab => KeyCode::Tab,
        NamedKey::Backspace => KeyCode::Backspace,
        NamedKey::Delete => KeyCode::Delete,
        NamedKey::Insert => KeyCode::Insert,
        NamedKey::Home => KeyCode::Home,
        NamedKey::End => KeyCode::End,
        NamedKey::PageUp => KeyCode::PageUp,
        NamedKey::PageDown => KeyCode::PageDown,
        NamedKey::ArrowUp => KeyCode::Up,
        NamedKey::ArrowDown => KeyCode::Down,
        NamedKey::ArrowLeft => KeyCode::Left,
        NamedKey::ArrowRight => KeyCode::Right,
        NamedKey::ContextMenu => KeyCode::Menu,
        other => KeyCode::F(function_number(other)?),
    };
    Some(code)
}

/// The keypad key for a character typed on the keypad, when the keymap
/// has one.
fn keypad_code(c: char) -> Option<KeyCode> {
    match c {
        '+' => Some(KeyCode::KeypadPlus),
        '-' => Some(KeyCode::KeypadMinus),
        '*' => Some(KeyCode::KeypadMultiply),
        '/' => Some(KeyCode::KeypadDivide),
        _ => None,
    }
}

/// The function keys F1 to F24, in order.
const FUNCTION_KEYS: [NamedKey; 24] = [
    NamedKey::F1,
    NamedKey::F2,
    NamedKey::F3,
    NamedKey::F4,
    NamedKey::F5,
    NamedKey::F6,
    NamedKey::F7,
    NamedKey::F8,
    NamedKey::F9,
    NamedKey::F10,
    NamedKey::F11,
    NamedKey::F12,
    NamedKey::F13,
    NamedKey::F14,
    NamedKey::F15,
    NamedKey::F16,
    NamedKey::F17,
    NamedKey::F18,
    NamedKey::F19,
    NamedKey::F20,
    NamedKey::F21,
    NamedKey::F22,
    NamedKey::F23,
    NamedKey::F24,
];

fn function_number(named: &NamedKey) -> Option<u8> {
    FUNCTION_KEYS
        .iter()
        .position(|f| f == named)
        .and_then(|i| u8::try_from(i + 1).ok())
}

/// The key event a person presses for `key` on `platform`: the inverse of
/// [`chord`], for the tests and the scripted walks, so they press only
/// keys the window can deliver. A letter with Shift is typed uppercase
/// with Shift held, as a keyboard sends it.
pub fn press(key: &Key, platform: Platform) -> KeyboardEvent {
    let mut mods = WinMods::empty();
    mods.set(WinMods::CONTROL, key.mods.ctrl);
    mods.set(WinMods::ALT, key.mods.alt);
    mods.set(WinMods::SHIFT, key.mods.shift);
    mods.set(WinMods::META, key.mods.meta && platform == Platform::MacOs);
    let mut location = Location::Standard;
    let named = |n: NamedKey| WinKey::Named(n);
    let winkey = match key.code {
        KeyCode::Char(c) => {
            let shown = if key.mods.shift && c.is_alphabetic() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c
            };
            WinKey::Character(shown.to_string())
        }
        KeyCode::F(n) => named(
            FUNCTION_KEYS
                .get(usize::from(n).saturating_sub(1))
                .cloned()
                .unwrap_or(NamedKey::F1),
        ),
        KeyCode::Enter => named(NamedKey::Enter),
        KeyCode::Escape => named(NamedKey::Escape),
        KeyCode::Tab => named(NamedKey::Tab),
        KeyCode::Backspace => named(NamedKey::Backspace),
        KeyCode::Delete => named(NamedKey::Delete),
        KeyCode::Insert => named(NamedKey::Insert),
        KeyCode::Home => named(NamedKey::Home),
        KeyCode::End => named(NamedKey::End),
        KeyCode::PageUp => named(NamedKey::PageUp),
        KeyCode::PageDown => named(NamedKey::PageDown),
        KeyCode::Up => named(NamedKey::ArrowUp),
        KeyCode::Down => named(NamedKey::ArrowDown),
        KeyCode::Left => named(NamedKey::ArrowLeft),
        KeyCode::Right => named(NamedKey::ArrowRight),
        KeyCode::Menu => named(NamedKey::ContextMenu),
        KeyCode::KeypadPlus
        | KeyCode::KeypadMinus
        | KeyCode::KeypadMultiply
        | KeyCode::KeypadDivide => {
            location = Location::Numpad;
            let c = match key.code {
                KeyCode::KeypadPlus => "+",
                KeyCode::KeypadMinus => "-",
                KeyCode::KeypadMultiply => "*",
                _ => "/",
            };
            WinKey::Character(c.to_owned())
        }
    };
    KeyboardEvent {
        state: KeyState::Down,
        key: winkey,
        location,
        modifiers: mods,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(text: &str) -> Key {
        Key::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn event(key: WinKey, mods: WinMods) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers: mods,
            ..Default::default()
        }
    }

    #[test]
    fn every_key_the_keymap_writes_survives_a_press() {
        for text in [
            "f5",
            "shift+f10",
            "ctrl+q",
            "ctrl+shift+p",
            "alt+left",
            "space",
            "ctrl+space",
            "enter",
            "escape",
            "tab",
            "shift+tab",
            "backspace",
            "delete",
            "insert",
            "home",
            "end",
            "pageup",
            "pagedown",
            "up",
            "down",
            "left",
            "right",
            "menu",
            "kp-plus",
            "kp-minus",
            "kp-multiply",
            "kp-divide",
            "?",
            "+",
            "*",
            "q",
            "shift+q",
            "alt+f",
            "ctrl+f6",
            "f24",
        ] {
            let key = k(text);
            for platform in [Platform::Windows, Platform::Other, Platform::MacOs] {
                let pressed = press(&key, platform);
                assert_eq!(
                    to_key(&pressed, platform, false),
                    Some(key),
                    "{text} on {platform:?}"
                );
            }
        }
    }

    #[test]
    fn a_typed_character_carries_its_own_shift() {
        let question = event(WinKey::Character("?".into()), WinMods::SHIFT);
        assert_eq!(to_key(&question, Platform::Windows, false), Some(k("?")));
        let capital = event(WinKey::Character("Q".into()), WinMods::SHIFT);
        assert_eq!(
            to_key(&capital, Platform::Windows, false),
            Some(k("shift+q"))
        );
        let ctrl_q = event(WinKey::Character("Q".into()), WinMods::CONTROL);
        assert_eq!(to_key(&ctrl_q, Platform::Windows, false), Some(k("ctrl+q")));
    }

    #[test]
    fn the_keypad_is_told_apart_by_where_the_key_is() {
        let mut plus = event(WinKey::Character("+".into()), WinMods::empty());
        assert_eq!(to_key(&plus, Platform::Windows, false), Some(k("+")));
        plus.location = Location::Numpad;
        assert_eq!(to_key(&plus, Platform::Windows, false), Some(k("kp-plus")));
        // A keypad digit is the digit.
        let mut seven = event(WinKey::Character("7".into()), WinMods::empty());
        seven.location = Location::Numpad;
        assert_eq!(to_key(&seven, Platform::Windows, false), Some(k("7")));
    }

    #[test]
    fn altgr_types_its_character() {
        // Windows: Control plus Alt with the character, as it came; the
        // app types it when nothing is bound.
        let at = event(
            WinKey::Character("@".into()),
            WinMods::CONTROL | WinMods::ALT,
        );
        let key = to_key(&at, Platform::Windows, true).unwrap();
        assert_eq!(key.code, KeyCode::Char('@'));
        assert!(key.mods.ctrl && key.mods.alt);
        // Marked by the system: plain text.
        let marked = event(WinKey::Character("@".into()), WinMods::ALT_GRAPH);
        assert_eq!(to_key(&marked, Platform::Other, true), Some(k("@")));
    }

    #[test]
    fn repeats_act_only_where_they_mean_something() {
        let mut down = press(&k("down"), Platform::Windows);
        down.repeat = true;
        assert_eq!(to_key(&down, Platform::Windows, false), Some(k("down")));
        let mut f8 = press(&k("f8"), Platform::Windows);
        f8.repeat = true;
        assert_eq!(to_key(&f8, Platform::Windows, false), None);
        let mut back = press(&k("backspace"), Platform::Windows);
        back.repeat = true;
        assert_eq!(to_key(&back, Platform::Windows, false), None);
        assert_eq!(to_key(&back, Platform::Windows, true), Some(k("backspace")));
        let mut letter = press(&k("a"), Platform::Windows);
        letter.repeat = true;
        assert_eq!(to_key(&letter, Platform::Windows, false), None);
        assert_eq!(to_key(&letter, Platform::Windows, true), Some(k("a")));
    }

    #[test]
    fn releases_lone_modifiers_and_compositions_are_not_keys() {
        let mut up = press(&k("f5"), Platform::Windows);
        up.state = KeyState::Up;
        assert_eq!(to_key(&up, Platform::Windows, false), None);
        let shift = event(WinKey::Named(NamedKey::Shift), WinMods::SHIFT);
        assert_eq!(to_key(&shift, Platform::Windows, false), None);
        let caps = event(WinKey::Named(NamedKey::CapsLock), WinMods::empty());
        assert_eq!(to_key(&caps, Platform::Windows, false), None);
        let mut composing = press(&k("a"), Platform::Windows);
        composing.is_composing = true;
        assert_eq!(to_key(&composing, Platform::Windows, true), None);
    }

    #[test]
    fn the_context_menu_key_is_menu_and_the_windows_key_is_the_systems() {
        let menu = event(WinKey::Named(NamedKey::ContextMenu), WinMods::empty());
        assert_eq!(to_key(&menu, Platform::Windows, false), Some(k("menu")));
        let win_e = event(WinKey::Character("e".into()), WinMods::META);
        assert_eq!(to_key(&win_e, Platform::Windows, false), None);
        assert_eq!(to_key(&win_e, Platform::Other, false), None);
        assert_eq!(to_key(&win_e, Platform::MacOs, false), Some(k("cmd+e")));
    }

    #[test]
    fn macos_option_chords_are_read_by_the_keys_place() {
        let mut sharp_s = event(WinKey::Character("\u{df}".into()), WinMods::ALT);
        sharp_s.code = Code::KeyS;
        assert_eq!(to_key(&sharp_s, Platform::MacOs, false), Some(k("alt+s")));
        // Elsewhere the character is what came.
        let key = to_key(&sharp_s, Platform::Windows, false).unwrap();
        assert_eq!(key.code, KeyCode::Char('\u{df}'));
    }
}
