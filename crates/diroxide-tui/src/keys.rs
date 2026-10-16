//! Terminal key events to diroxide's keys, and the key test's words for
//! what the terminal sent.
//!
//! The rules (R4 sections 3.2 and 4.3, R19 spike one):
//!
//! - Only presses and repeats act. On Windows crossterm reports releases
//!   too, so a press and its release would otherwise run a command twice.
//! - A character that is not a letter carries its Shift in the character
//!   itself ("?" is Shift plus slash on one layout and a plain key on
//!   another), so its Shift is dropped; a capital letter becomes the small
//!   letter with Shift, as the keymap writes it.
//! - On Windows, AltGr arrives as Control plus Alt (crossterm issue 820).
//!   The key goes to the app as it came; the app types its character in a
//!   text field and the quick search, and the key test says so.
//! - Keys diroxide never binds (media keys, a modifier alone, Caps Lock)
//!   are dropped here.

use crossterm::event::{KeyCode as TermCode, KeyEvent, KeyEventKind, KeyModifiers};
use diroxide_core::{Key, KeyCode, Modifiers};

/// Whether a key event acts: a press or a repeat. A release never does.
pub fn acts(event: &KeyEvent) -> bool {
    matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn modifiers(mods: KeyModifiers) -> Modifiers {
    Modifiers {
        ctrl: mods.contains(KeyModifiers::CONTROL),
        alt: mods.contains(KeyModifiers::ALT),
        shift: mods.contains(KeyModifiers::SHIFT),
        meta: mods.intersects(KeyModifiers::SUPER | KeyModifiers::META | KeyModifiers::HYPER),
    }
}

/// The key for a terminal key event, or `None` for a key diroxide never
/// binds. The event's kind is not checked here; see [`acts`].
pub fn to_key(event: &KeyEvent) -> Option<Key> {
    let mut mods = modifiers(event.modifiers);
    let code = match event.code {
        TermCode::Char(c) if c.is_control() => return None,
        TermCode::Char(c) => {
            if !c.is_alphabetic() && c != ' ' {
                mods.shift = false;
            }
            KeyCode::Char(c)
        }
        TermCode::F(n) if (1..=24).contains(&n) => KeyCode::F(n),
        TermCode::Enter => KeyCode::Enter,
        TermCode::Esc => KeyCode::Escape,
        TermCode::Tab => KeyCode::Tab,
        TermCode::BackTab => {
            mods.shift = true;
            KeyCode::Tab
        }
        TermCode::Backspace => KeyCode::Backspace,
        TermCode::Delete => KeyCode::Delete,
        TermCode::Insert => KeyCode::Insert,
        TermCode::Home => KeyCode::Home,
        TermCode::End => KeyCode::End,
        TermCode::PageUp => KeyCode::PageUp,
        TermCode::PageDown => KeyCode::PageDown,
        TermCode::Up => KeyCode::Up,
        TermCode::Down => KeyCode::Down,
        TermCode::Left => KeyCode::Left,
        TermCode::Right => KeyCode::Right,
        TermCode::Menu => KeyCode::Menu,
        _ => return None,
    };
    Some(Key::new(code, mods))
}

/// The terminal's own name for a key, in words: "F5", "the letter q",
/// "Escape".
fn code_words(code: TermCode) -> String {
    match code {
        TermCode::Char(' ') => "Space".to_owned(),
        TermCode::Char(c) if c.is_alphabetic() => format!("the letter {c}"),
        TermCode::Char(c) if c.is_control() => {
            format!("control character {:04X}", u32::from(c))
        }
        TermCode::Char(c) => format!("the character {c}"),
        TermCode::F(n) => format!("F{n}"),
        TermCode::Enter => "Enter".to_owned(),
        TermCode::Esc => "Escape".to_owned(),
        TermCode::Tab => "Tab".to_owned(),
        TermCode::BackTab => "Back Tab".to_owned(),
        TermCode::Backspace => "Backspace".to_owned(),
        TermCode::Delete => "Delete".to_owned(),
        TermCode::Insert => "Insert".to_owned(),
        TermCode::Home => "Home".to_owned(),
        TermCode::End => "End".to_owned(),
        TermCode::PageUp => "Page Up".to_owned(),
        TermCode::PageDown => "Page Down".to_owned(),
        TermCode::Up => "Up".to_owned(),
        TermCode::Down => "Down".to_owned(),
        TermCode::Left => "Left".to_owned(),
        TermCode::Right => "Right".to_owned(),
        TermCode::Menu => "Menu".to_owned(),
        TermCode::CapsLock => "Caps Lock".to_owned(),
        TermCode::NumLock => "Num Lock".to_owned(),
        TermCode::ScrollLock => "Scroll Lock".to_owned(),
        TermCode::PrintScreen => "Print Screen".to_owned(),
        TermCode::Pause => "Pause".to_owned(),
        TermCode::Null => "a null key".to_owned(),
        TermCode::KeypadBegin => "the keypad's middle key".to_owned(),
        TermCode::Media(_) => "a media key".to_owned(),
        TermCode::Modifier(_) => "a modifier key alone".to_owned(),
    }
}

/// What the terminal sent, in words, for the key test: "Sent F5 with
/// Control, a press." The modifiers are the terminal's, before diroxide
/// reads them.
pub fn sent_words(event: &KeyEvent) -> String {
    let mut held = Vec::new();
    for (flag, word) in [
        (KeyModifiers::CONTROL, "Control"),
        (KeyModifiers::ALT, "Alt"),
        (KeyModifiers::SHIFT, "Shift"),
        (KeyModifiers::SUPER, "Super"),
        (KeyModifiers::META, "Meta"),
        (KeyModifiers::HYPER, "Hyper"),
    ] {
        if event.modifiers.contains(flag) {
            held.push(word);
        }
    }
    let kind = match event.kind {
        KeyEventKind::Press => "a press",
        KeyEventKind::Repeat => "a repeat",
        KeyEventKind::Release => "a release",
    };
    let code = code_words(event.code);
    if held.is_empty() {
        format!("Sent {code}, {kind}.")
    } else {
        format!(
            "Sent {code} with {}, {kind}.",
            diroxide_app::settings::list_words(&held, "and")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn event(code: TermCode, mods: KeyModifiers, kind: KeyEventKind) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind,
            state: KeyEventState::NONE,
        }
    }

    fn pressed(code: TermCode, mods: KeyModifiers) -> KeyEvent {
        event(code, mods, KeyEventKind::Press)
    }

    #[test]
    fn keys_read_as_the_keymap_writes_them() {
        let cases = [
            (pressed(TermCode::F(5), KeyModifiers::CONTROL), "ctrl+f5"),
            (pressed(TermCode::Char('Q'), KeyModifiers::SHIFT), "shift+q"),
            (
                pressed(TermCode::Char('q'), KeyModifiers::CONTROL),
                "ctrl+q",
            ),
            (pressed(TermCode::Char('?'), KeyModifiers::SHIFT), "?"),
            (pressed(TermCode::Char('*'), KeyModifiers::SHIFT), "*"),
            (pressed(TermCode::Char(' '), KeyModifiers::NONE), "space"),
            (pressed(TermCode::BackTab, KeyModifiers::SHIFT), "shift+tab"),
            (pressed(TermCode::Home, KeyModifiers::ALT), "alt+home"),
            (pressed(TermCode::Esc, KeyModifiers::NONE), "escape"),
            (pressed(TermCode::Menu, KeyModifiers::NONE), "menu"),
        ];
        for (event, written) in cases {
            assert_eq!(
                to_key(&event),
                Some(Key::parse(written).unwrap()),
                "{written}"
            );
        }
        assert_eq!(
            to_key(&pressed(TermCode::CapsLock, KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn a_release_never_acts() {
        let release = event(TermCode::Down, KeyModifiers::NONE, KeyEventKind::Release);
        let repeat = event(TermCode::Down, KeyModifiers::NONE, KeyEventKind::Repeat);
        assert!(!acts(&release));
        assert!(acts(&repeat));
        assert!(acts(&pressed(TermCode::Down, KeyModifiers::NONE)));
    }

    #[test]
    fn altgr_keeps_its_character_and_both_modifiers() {
        let altgr = pressed(
            TermCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        let key = to_key(&altgr).unwrap();
        assert!(key.mods.ctrl && key.mods.alt);
        assert_eq!(key.code, KeyCode::Char('@'));
        assert_eq!(
            sent_words(&altgr),
            "Sent the character @ with Control and Alt, a press."
        );
    }

    #[test]
    fn what_was_sent_is_said_in_words() {
        assert_eq!(
            sent_words(&pressed(TermCode::F(5), KeyModifiers::CONTROL)),
            "Sent F5 with Control, a press."
        );
        assert_eq!(
            sent_words(&event(
                TermCode::Char('x'),
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            "Sent the letter x, a release."
        );
    }
}
