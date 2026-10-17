//! Keys in the app: Change key's capture (the settle time, Escape, AltGr,
//! the say-back, ten seconds and the typed name), driven with the fake
//! clock.

use diroxide_app::commands::ids;
use diroxide_core::{ArgValue, Command, Key};
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, fixture, place, settings, temp};

fn change(id: &str) -> Command {
    Command::new(ids::KEYS_CHANGE).with_arg("command", ArgValue::Text(id.to_owned()))
}

fn session_with_settle(settle_ms: u64) -> (tempfile::TempDir, Session) {
    let dir = temp();
    let work = fixture(dir.path());
    let mut slow = settings();
    slow.dialog_settle_ms = settle_ms;
    let s = Session::open(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup {
            settings: slow,
            ..Setup::default()
        },
    );
    (dir, s)
}

#[test]
fn change_key_takes_one_key_after_the_settle_time_and_says_it_back() {
    let (_dir, mut s) = session_with_settle(250);
    let said = s.run_command(change("view.sort-size"));
    assert_eq!(said, ["Press the new key now. Escape cancels."]);
    assert!(s.app.capturing_key());
    assert!(
        s.app.key_test_active(),
        "the frontend sends keys as they came"
    );
    // The Enter that chose Change key, held down, is passed over.
    assert!(s.key("enter").is_empty());
    assert!(s.app.capturing_key());
    s.clock.advance(1);
    let said = s.key("ctrl+f9");
    assert_eq!(
        said,
        ["Got Control plus F9", "Ctrl+F9 now runs Sort by size."]
    );
    assert!(!s.app.capturing_key());
    assert!(s.dialog().is_none(), "the prompt closed");
    assert_eq!(s.app.keymap().key_text(&ids::SORT_SIZE), "Ctrl+F9");
}

#[test]
fn escape_cancels_and_altgr_is_typing() {
    let (_dir, mut s) = session_with_settle(0);
    s.run_command(change("view.sort-size"));
    if cfg!(windows) {
        let said = s.key("ctrl+alt+q");
        assert_eq!(
            said,
            ["Ctrl+Alt+Q reads as AltGr typing q. Type its name to bind it."]
        );
        assert!(s.app.capturing_key(), "the wait goes on");
    }
    assert_eq!(s.key("escape"), ["Nothing changed."]);
    assert!(!s.app.capturing_key());
    assert!(s.dialog().is_none());
    assert_eq!(s.app.keymap().key_text(&ids::SORT_SIZE), "Ctrl+F6");
}

#[test]
fn ten_seconds_without_a_key_end_in_words_and_offer_the_typed_name() {
    let (_dir, mut s) = session_with_settle(0);
    s.run_command(change("view.sort-size"));
    // The frontend's timer: ten seconds without a key.
    let effects = s.app.end_key_test(10);
    let said = s.run(effects);
    assert_eq!(
        said,
        [
            "No key heard. The terminal may keep it.",
            "Type the key's name, such as ctrl+f5, empty"
        ]
    );
    assert!(!s.app.capturing_key());
    s.type_text("hyper+q");
    assert_eq!(
        s.key("enter"),
        ["Not a key name: hyper+q. Write it as keymap.toml does, such as ctrl+f5."]
    );
    for _ in 0.."hyper+q".len() {
        s.key("backspace");
    }
    s.type_text("escape");
    let said = s.key("enter");
    assert_eq!(said, ["Got Escape", "Escape now runs Sort by size."]);
    let escape = Key::parse("escape").unwrap();
    assert_eq!(
        s.app.keymap().keys_for(&ids::SORT_SIZE),
        vec![&escape],
        "a typed name binds even the key that cancels a capture"
    );
}

#[test]
fn change_key_needs_a_command_it_knows() {
    let (_dir, mut s) = session_with_settle(0);
    assert_eq!(
        s.run_command(Command::new(ids::KEYS_CHANGE)),
        ["Change a key needs a command: keys.change command=view.sort-size."]
    );
    assert_eq!(
        s.run_command(change("view.sort-sise")),
        ["No command has the id view.sort-sise. Nothing changed."]
    );
    assert!(!s.app.capturing_key());
}
