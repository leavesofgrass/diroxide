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
    // The typed name reaches even the key that cancels a capture; Escape
    // is Cancel's key, so the checks ask first.
    s.type_text("escape");
    let said = s.key("enter");
    assert_eq!(
        said,
        [
            "Got Escape",
            "Escape already cancels. Take it, swap, or cancel?"
        ]
    );
    assert_eq!(s.key("escape"), ["Nothing changed."]);
    let escape = Key::parse("escape").unwrap();
    assert_eq!(
        s.app.keymap().keys_for(&ids::CANCEL),
        vec![&escape],
        "Cancel keeps Escape"
    );
}

#[test]
fn a_key_already_used_asks_take_swap_or_cancel_starting_on_cancel() {
    let (_dir, mut s) = session_with_settle(0);
    s.run_command(change("view.sort-size"));
    assert_eq!(
        s.key("f5"),
        [
            "Got F5",
            "F5 already copies to the other panel. Take it, swap, or cancel?"
        ]
    );
    let focus = s.dialog().unwrap().focus_line();
    assert_eq!(focus, "Cancel, nothing changes, 3 of 3");
    assert_eq!(s.key("enter"), ["Nothing changed."]);
    assert_eq!(s.app.keymap().key_text(&ids::COPY), "F5");
    assert!(s.dialog().is_none());

    // Swap: Copy to other panel gets Sort by size's old key.
    s.run_command(change("view.sort-size"));
    s.key("f5");
    assert_eq!(
        s.key("up"),
        ["Swap, Copy to other panel gets Ctrl+F6, 2 of 3"]
    );
    assert_eq!(
        s.key("enter"),
        ["F5 now runs Sort by size. Copy to other panel moved to Ctrl+F6."]
    );
    assert_eq!(s.app.keymap().key_text(&ids::COPY), "Ctrl+F6");
    assert_eq!(s.app.keymap().key_text(&ids::SORT_SIZE), "F5");

    // Take: Sort by modified is left without a key.
    s.run_command(change("view.sort-name"));
    s.key("ctrl+f5");
    s.key("home");
    assert_eq!(
        s.key("enter"),
        ["Ctrl+F5 now runs Sort by name. Sort by modified has no key now."]
    );
    assert!(s.app.keymap().keys_for(&ids::SORT_MODIFIED).is_empty());
    assert!(s.app.keymap().conflicts().is_empty());
}

#[test]
fn ctrl_f5_says_it_sorts_by_modified_time() {
    let (_dir, mut s) = session_with_settle(0);
    s.run_command(change("view.sort-size"));
    let said = s.key("ctrl+f5");
    assert_eq!(
        said.last().unwrap(),
        "Ctrl+F5 already sorts by modified time. Take it, swap, or cancel?"
    );
    assert_eq!(s.key("escape"), ["Nothing changed."]);
}

#[test]
fn a_key_the_terminal_cannot_send_offers_the_window_column() {
    let (_dir, mut s) = session_with_settle(0);
    s.run_command(change("view.sort-kind"));
    let effects = s.app.end_key_test(10);
    s.run(effects);
    s.type_text("ctrl+enter");
    assert_eq!(
        s.key("enter"),
        [
            "Got Control plus Enter",
            "The terminal cannot send Ctrl+Enter. Use it for the window column only? No is the default. y or n"
        ]
    );
    // In the window column Ctrl+Enter goes to a path; Sort by kind has no
    // key there to swap.
    assert_eq!(
        s.key("y"),
        ["Ctrl+Enter already goes to a path. Take it, or cancel?"]
    );
    assert_eq!(s.key("escape"), ["Nothing changed."]);
    // No to the first question changes nothing either.
    s.run_command(change("view.sort-kind"));
    let effects = s.app.end_key_test(10);
    s.run(effects);
    s.type_text("ctrl+enter");
    s.key("enter");
    assert_eq!(s.key("n"), ["Nothing changed."]);
    assert!(s.app.keymap().keys_for(&ids::SORT_KIND).is_empty());
}

#[test]
fn insert_is_refused_and_a_reader_key_and_a_letter_ask_first() {
    let (_dir, mut s) = session_with_settle(0);
    s.run_command(change("view.sort-kind"));
    assert_eq!(
        s.key("insert"),
        ["Got Insert", "Insert is the screen reader key."]
    );
    assert!(s.dialog().is_none());
    s.run_command(change("view.sort-kind"));
    let said = s.key("ctrl+alt+up");
    assert_eq!(
        said.last().unwrap(),
        "Screen readers keep Ctrl+Alt+Up: NVDA, JAWS and Narrator. Use it anyway? No is the default. y or n"
    );
    assert_eq!(s.key("y"), ["Ctrl+Alt+Up now runs Sort by kind."]);
    s.run_command(change("view.sort-owner"));
    let said = s.key("w");
    assert_eq!(
        said.last().unwrap(),
        "A letter key works only while letters run commands. Go on? No is the default. y or n"
    );
    assert_eq!(s.key("y"), ["W now runs Sort by owner."]);
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
