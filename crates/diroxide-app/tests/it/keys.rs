//! Keys in the app: Change key's capture (the settle time, Escape, AltGr,
//! the say-back, ten seconds and the typed name), driven with the fake
//! clock.

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_app::keymap::Platform;
use diroxide_config::Paths;
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
        ["Got Control plus F9", "Ctrl+F9 saved for Sort by size."]
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
        ["F5 saved for Sort by size. Copy to other panel moved to Ctrl+F6."]
    );
    assert_eq!(s.app.keymap().key_text(&ids::COPY), "Ctrl+F6");
    assert_eq!(s.app.keymap().key_text(&ids::SORT_SIZE), "F5");

    // Take: Sort by modified is left without a key.
    s.run_command(change("view.sort-name"));
    s.key("ctrl+f5");
    s.key("home");
    assert_eq!(
        s.key("enter"),
        ["Ctrl+F5 saved for Sort by name. Sort by modified has no key now."]
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
    assert_eq!(s.key("y"), ["Ctrl+Alt+Up saved for Sort by kind."]);
    s.run_command(change("view.sort-owner"));
    let said = s.key("w");
    assert_eq!(
        said.last().unwrap(),
        "A letter key works only while letters run commands. Go on? No is the default. y or n"
    );
    assert_eq!(s.key("y"), ["W saved for Sort by owner."]);
}

/// The owner's file, with comments and a line this version does not know.
const GOLDEN: &str = "# My keys, kept in my dotfiles.\n\n[terminal.panel]\n# Sorting first.\n\"view.sort-name\" = \"f12\" # my old habit\n\"plugin.thing\" = \"ctrl+k\"\n";

/// A session over a config folder holding the golden `keymap.toml`, its
/// keys built from the file as the real start builds them.
fn session_with_file() -> (tempfile::TempDir, Session, Paths) {
    let dir = temp();
    let work = fixture(dir.path());
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(paths.config_file(Paths::KEYMAP), GOLDEN).unwrap();
    let config = diroxide_config::load(&paths);
    let keymap = diroxide_app::startup::terminal_keymap(&config, Platform::Other);
    let s = Session::open_with_keymap(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup {
            paths: Some(paths.clone()),
            ..Setup::default()
        },
        keymap,
    );
    (dir, s, paths)
}

fn file_text(paths: &Paths) -> String {
    std::fs::read_to_string(paths.config_file(Paths::KEYMAP)).unwrap()
}

#[test]
fn the_keys_list_from_settings_narrows_and_a_change_keeps_the_files_comments() {
    let (_dir, mut s, paths) = session_with_file();
    // 17. Settings, then the Keys list.
    s.command(ids::SETTINGS);
    let said = s.key("k");
    assert!(
        said[0].starts_with("Keys list, every command and its key, "),
        "{said:?}"
    );
    assert_eq!(
        s.key("enter"),
        ["Keys, preset diroxide defaults, 1 changed, empty"]
    );
    let Some(Dialog::Edit {
        candidates: Some(list),
        ..
    }) = s.dialog()
    else {
        panic!("{:?}", s.dialog())
    };
    let rows: Vec<String> = list.rows().iter().map(|r| r.spoken()).collect();
    assert_eq!(rows[0], "Choose a preset, diroxide defaults");
    assert_eq!(rows[1], "Reset all keys, 1 changed");
    assert!(rows[2].starts_with("Single letters, "), "{}", rows[2]);
    assert!(rows.contains(&"Copy to other panel, F5 in both".to_owned()));
    assert!(
        rows.contains(
            &"Sort by name, F12 in a terminal, Ctrl+F3 in the window, changed, check terminal"
                .to_owned()
        )
    );
    let said = s.type_text("copy");
    let last = said.last().unwrap();
    assert!(last.starts_with("Copy path, no key. "), "{last}");
    assert!(last.ends_with(" matches"), "{last}");
    for _ in 0..4 {
        s.key("backspace");
    }

    // 18. Sort by size, Change key, Ctrl+F9.
    s.type_text("sort by size");
    assert_eq!(s.key("down"), ["Sort by size, Ctrl+F6 in both, 1 of 1"]);
    assert_eq!(
        s.key("enter"),
        ["Sort by size, Ctrl+F6 in both, Change key, press the new key, 1 of 5"]
    );
    assert_eq!(s.key("enter"), ["Press the new key now. Escape cancels."]);
    assert_eq!(
        s.key("ctrl+f9"),
        ["Got Control plus F9", "Ctrl+F9 saved for Sort by size."]
    );
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Keys, preset diroxide defaults, 2 changed"),
        "back in the Keys list"
    );
    assert_eq!(s.app.keymap().key_text(&ids::SORT_SIZE), "Ctrl+F9");
    assert_eq!(
        file_text(&paths),
        "# My keys, kept in my dotfiles.\n\n[terminal.panel]\n# Sorting first.\n\"view.sort-name\" = \"f12\" # my old habit\n\"plugin.thing\" = \"ctrl+k\"\n\"view.sort-size\" = \"ctrl+f9\"\n"
    );

    // A key already used: F5, then Cancel.
    s.key("enter");
    s.key("enter");
    assert_eq!(
        s.key("f5"),
        [
            "Got F5",
            "F5 already copies to the other panel. Take it, swap, or cancel?"
        ]
    );
    assert_eq!(s.key("escape"), ["Nothing changed."]);

    // 19. Change key, and ten seconds without a key.
    s.key("enter");
    s.key("enter");
    let effects = s.app.end_key_test(10);
    let said = s.run(effects);
    assert_eq!(
        said,
        [
            "No key heard. The terminal may keep it.",
            "Type the key's name, such as ctrl+f5, empty"
        ]
    );
    s.key("escape");

    // Reset this key: the line goes, the comments stay.
    s.key("enter");
    s.key("end");
    s.key("up");
    assert_eq!(s.key("enter"), ["Sort by size back to Ctrl+F6."]);
    assert_eq!(file_text(&paths), GOLDEN);
}

#[test]
fn remove_a_key_reset_all_and_the_single_letter_switch() {
    let (_dir, mut s, paths) = session_with_file();
    s.command(ids::KEYS_LIST);
    // Help has two terminal keys: Remove asks which.
    s.type_text("help");
    s.key("down");
    s.key("enter");
    let Some(Dialog::Choose { list, .. }) = s.dialog() else {
        panic!("{:?}", s.dialog())
    };
    let remove = list.rows().iter().position(|r| r.id == "remove").unwrap();
    s.key("home");
    for _ in 0..remove {
        s.key("down");
    }
    assert_eq!(
        s.key("enter"),
        ["Remove which key of Help, Shift+F1, 1 of 2"]
    );
    s.key("down");
    assert_eq!(s.key("enter"), ["? removed from Help."]);
    assert_eq!(s.app.keymap().keys_text(&ids::HELP), "Shift+F1");
    assert!(file_text(&paths).contains("\"app.help\" = \"shift+f1\""));
    // Help is critical: its last key stays.
    s.key("enter");
    let Some(Dialog::Choose { list, .. }) = s.dialog() else {
        panic!("{:?}", s.dialog())
    };
    let remove = list.rows().iter().position(|r| r.id == "remove").unwrap();
    s.key("home");
    for _ in 0..remove {
        s.key("down");
    }
    assert_eq!(
        s.key("enter"),
        ["Help needs a key, so Shift+F1 stays. Change it instead."]
    );
    s.key("escape");
    s.key("escape");

    // Reset all: asks, starting on No, and keeps the unknown line.
    s.command(ids::KEYS_LIST);
    s.key("down");
    let said = s.key("down");
    assert!(
        said[0].starts_with("Reset all keys, 2 changed, 2 of "),
        "{said:?}"
    );
    let said = s.key("enter");
    assert_eq!(said, ["Reset all keys? No is the default. y or n"]);
    assert_eq!(s.key("n"), ["Nothing changed."]);
    s.key("enter");
    assert_eq!(
        s.key("y"),
        ["All keys reset: 2 commands back to the default."]
    );
    assert_eq!(
        file_text(&paths),
        "# My keys, kept in my dotfiles.\n\n[terminal.panel]\n# Sorting first.\n\"plugin.thing\" = \"ctrl+k\"\n"
    );
    assert_eq!(s.app.keymap().key_text(&ids::SORT_NAME), "Ctrl+F3");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Keys, preset diroxide defaults, 0 changed")
    );
    s.key("escape");
    assert_eq!(
        s.run_command(Command::new(ids::KEYS_RESET_ALL)),
        ["No key was changed. Nothing to reset."]
    );

    // The single-letter switch keeps the owner's lines.
    s.run_command(change("view.sort-owner"));
    s.key("w");
    s.key("y");
    assert_eq!(s.app.keymap().key_text(&ids::SORT_OWNER), "");
    s.command(ids::KEYS_SINGLE_LETTERS);
    s.key("down");
    assert_eq!(s.key("enter"), ["Single letters: letters run commands"]);
    assert_eq!(s.app.keymap().key_text(&ids::SORT_OWNER), "W");
}

#[test]
fn the_preset_chooser_says_the_preset_in_use() {
    let (_dir, mut s) = session_with_settle(0);
    let said = s.command(ids::KEYS_PRESET);
    assert!(
        said[0].starts_with("Key presets, diroxide defaults, current, The keys most people know"),
        "{said:?}"
    );
    assert_eq!(
        s.key("enter"),
        ["The diroxide default keys are in use already."]
    );
    s.key("escape");
    // The Settings list's Key preset row opens the same chooser.
    s.command(ids::SETTINGS);
    let dialog = s.app.dialog().map(|(id, _)| id).unwrap();
    let effects = s.app.handle(diroxide_app::AppEvent::Answer {
        dialog,
        answer: diroxide_app::Answer::Chosen("preset".to_owned()),
    });
    let said = s.run(effects);
    assert!(said[0].starts_with("Key presets, "), "{said:?}");
}

#[test]
fn an_unknown_preset_setting_is_a_startup_problem_and_the_defaults_apply() {
    let dir = temp();
    let work = fixture(dir.path());
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(paths.config_file(Paths::SETTINGS), "preset = \"norton\"\n").unwrap();
    let s = Session::open(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup {
            paths: Some(paths),
            ..Setup::default()
        },
    );
    let problems: Vec<&str> = s.app.problems().iter().map(|p| p.text.as_str()).collect();
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("Unknown key preset: norton. The diroxide keys are in use")),
        "{problems:?}"
    );
    assert_eq!(s.app.keymap().key_text(&ids::COPY), "F5");
}

#[test]
fn change_key_needs_a_command_it_knows() {
    let (_dir, mut s) = session_with_settle(0);
    assert_eq!(
        s.run_command(Command::new(ids::KEYS_CHANGE)),
        ["Keys, preset diroxide defaults, 0 changed, empty"],
        "without a command, the Keys list"
    );
    s.key("escape");
    assert_eq!(
        s.run_command(change("view.sort-sise")),
        ["No command has the id view.sort-sise. Nothing changed."]
    );
    assert!(!s.app.capturing_key());
}
