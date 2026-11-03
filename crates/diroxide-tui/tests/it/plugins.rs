//! A plugin's dialogs in the terminal (W4d, R22 commit 37): consent, a
//! question of each of the five kinds, and a text document, each named
//! with the plugin's name last, closed by Escape with the focus back on
//! the panel, the hardware cursor on the dialog's focus in every one; a
//! plugin's dialog waiting behind a destructive confirmation; and a plugin
//! list drawn inside its panel, over the folder, with the cursor on its
//! focused row (lane L).
//!
//! The plugin's calls arrive as the runtime sends them
//! ([`PluginEvent::Call`]); no plugin runs.

use diroxide_app::AppEvent;
use diroxide_app::config::Layout;
use diroxide_app::plugins::{
    Call, CallId, Catalog, Choice, Item, ItemKind, ItemState, ListerColumn, ListerSpec, PathMode,
    PluginDocument, PluginEvent, PluginLister, PluginPanel, PluginQuestion, PluginQuestionKind,
    TextKind,
};
use diroxide_core::{Entry, EntryKind, Side};

use crate::support::{
    Clock, app, cursor_at, fixture, focus, line_of, place, press, session, settings, settle, start,
    temp, type_text,
};

fn question(title: &str, kind: PluginQuestionKind) -> PluginQuestion {
    PluginQuestion {
        plugin_id: "sftp".into(),
        plugin: "SFTP places".into(),
        title: format!("{title} (SFTP places)"),
        lines: vec!["Key: 1a2b 3c4d".into()],
        kind,
        prompt_id: None,
        history: Vec::new(),
        completion: None,
    }
}

fn ask(n: u64, question: PluginQuestion) -> AppEvent {
    AppEvent::Plugin(Box::new(PluginEvent::Call {
        call: CallId::new(n),
        plugin: "sftp".into(),
        request: Call::Ask(Box::new(question)),
    }))
}

fn panel_line(s: &diroxide_tui::Session<'_, ratatui::backend::TestBackend>) -> usize {
    s.screen().cursor_line.unwrap()
}

#[test]
fn every_plugin_dialog_is_named_and_keeps_the_cursor_on_its_focus() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut a, mut r) = app(place(&work), place(&work), settings(Layout::SideBySide));
    let mut s = session(&mut a, &mut r, 100, 24, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    focus(&mut s, &clock, "notes.txt");
    let row = panel_line(&s);

    // Confirm: the question first, starting on No, the plugin last; the
    // cursor on the question.
    s.event(
        ask(
            1,
            question(
                "Trust this host?",
                PluginQuestionKind::Confirm {
                    destructive: true,
                    yes_label: None,
                    no_label: None,
                },
            ),
        ),
        clock.now(),
    );
    settle(&mut s, &clock);
    let line = line_of(
        &s.screen(),
        "Trust this host? No is the default. y or n (SFTP places)",
    );
    cursor_at(&s, 0, line);
    assert!(s.screen().lines[line + 1].starts_with("Key: 1a2b 3c4d"));
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
    cursor_at(&s, 0, row);

    // Choose: the lines, then the choices; the cursor on the default.
    s.event(
        ask(
            2,
            question(
                "Pick a branch",
                PluginQuestionKind::Choose {
                    choices: vec![
                        Choice {
                            id: "a".into(),
                            label: "main".into(),
                            description: None,
                        },
                        Choice {
                            id: "b".into(),
                            label: "feature".into(),
                            description: Some("two commits ahead".into()),
                        },
                    ],
                    default_id: Some("b".into()),
                    multiple: false,
                },
            ),
        ),
        clock.now(),
    );
    settle(&mut s, &clock);
    line_of(&s.screen(), "Pick a branch (SFTP places)");
    let focus_line = line_of(&s.screen(), "feature, two commits ahead");
    cursor_at(&s, 0, focus_line);
    press(&mut s, &clock, "up");
    cursor_at(&s, 0, line_of(&s.screen(), "main"));
    press(&mut s, &clock, "escape");
    cursor_at(&s, 0, row);

    // Text: the field in the minibuffer, the cursor at its insertion
    // point.
    s.event(
        ask(
            3,
            question(
                "Branch name",
                PluginQuestionKind::Text {
                    initial: "ma".into(),
                    pattern: None,
                },
            ),
        ),
        clock.now(),
    );
    settle(&mut s, &clock);
    let prompt = line_of(&s.screen(), "Branch name (SFTP places): ma");
    cursor_at(&s, "Branch name (SFTP places): ma".len(), prompt);
    type_text(&mut s, &clock, "in");
    cursor_at(&s, "Branch name (SFTP places): main".len(), prompt);
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
    cursor_at(&s, 0, row);

    // Path: the same field shape, a place to type.
    s.event(
        ask(
            4,
            question(
                "Where to",
                PluginQuestionKind::Path {
                    mode: PathMode::Folder,
                    must_exist: false,
                    initial: String::new(),
                },
            ),
        ),
        clock.now(),
    );
    settle(&mut s, &clock);
    let prompt = line_of(&s.screen(), "Where to (SFTP places):");
    cursor_at(&s, "Where to (SFTP places): ".len(), prompt);
    press(&mut s, &clock, "escape");
    cursor_at(&s, 0, row);

    // Secret: the masked prompt; what is typed never reaches the screen.
    s.event(
        ask(5, question("Token", PluginQuestionKind::Secret)),
        clock.now(),
    );
    settle(&mut s, &clock);
    type_text(&mut s, &clock, "canary");
    let screen = s.screen();
    assert!(
        !screen.lines.iter().any(|l| l.contains("canary")),
        "{:#?}",
        screen.lines
    );
    let prompt = line_of(&screen, "Token (SFTP places) (hidden): ******");
    cursor_at(&s, "Token (SFTP places) (hidden): ******".len(), prompt);
    press(&mut s, &clock, "escape");
    cursor_at(&s, 0, row);

    // A text document: a list of its lines to read, the cursor on the
    // first.
    s.event(
        AppEvent::Plugin(Box::new(PluginEvent::Call {
            call: CallId::new(6),
            plugin: "sftp".into(),
            request: Call::ShowText(Box::new(PluginDocument {
                plugin: "SFTP places".into(),
                title: "Host key (SFTP places)".into(),
                lines: vec!["1a2b 3c4d".into(), "5e6f 7a8b".into()],
                kind: TextKind::Plain,
                wait: false,
            })),
        })),
        clock.now(),
    );
    settle(&mut s, &clock);
    line_of(&s.screen(), "Host key (SFTP places)");
    cursor_at(&s, 0, line_of(&s.screen(), "1a2b 3c4d"));
    press(&mut s, &clock, "down");
    cursor_at(&s, 0, line_of(&s.screen(), "5e6f 7a8b"));
    press(&mut s, &clock, "escape");
    cursor_at(&s, 0, row);
    assert_eq!(s.app().active_side(), Side::Left);
}

#[test]
fn the_consent_question_starts_on_no_with_the_cursor_on_it() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut a, mut r) = app(place(&work), place(&work), settings(Layout::SideBySide));
    let mut s = session(&mut a, &mut r, 100, 24, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    let row = panel_line(&s);
    let item = Item {
        id: "total-size".into(),
        name: "Total size of selection".into(),
        kind: ItemKind::Script,
        path: work.join("total-size.sh"),
        example: false,
        state: ItemState::NotEnabled,
        description: "Says how big the selection is.".into(),
        enable_rows: vec![
            "Enable Total size of selection?".into(),
            "It runs: bash total-size.sh".into(),
            "While it runs it can do what you can.".into(),
            "Enable it? y or n".into(),
        ],
        changed_rows: Vec::new(),
        commands: Vec::new(),
        settings: Vec::new(),
        listers: Vec::new(),
        manifest: None,
        fingerprint: None,
        values: Default::default(),
    };
    let catalog = Catalog {
        off: false,
        items: vec![item],
        problems: Vec::new(),
    };
    s.command(
        diroxide_core::Command::new(diroxide_app::commands::ids::PLUGINS),
        clock.now(),
    );
    settle(&mut s, &clock);
    line_of(&s.screen(), "Manage plugins");
    // The plugins found arrive while the list is open: it shows them.
    s.event(
        AppEvent::Plugin(Box::new(PluginEvent::Found(Box::new(catalog)))),
        clock.now(),
    );
    settle(&mut s, &clock);
    cursor_at(
        &s,
        0,
        line_of(&s.screen(), "Total size of selection: not enabled, script"),
    );
    press(&mut s, &clock, "enter");
    cursor_at(&s, 0, line_of(&s.screen(), "Enable"));
    press(&mut s, &clock, "enter");
    let line = line_of(
        &s.screen(),
        "Enable Total size of selection? No is the default. y or n",
    );
    cursor_at(&s, 0, line);
    line_of(&s.screen(), "It runs: bash total-size.sh");
    line_of(&s.screen(), "While it runs it can do what you can.");
    press(&mut s, &clock, "n");
    assert!(
        s.said().iter().any(|m| m.text == "Not enabled."),
        "{:?}",
        s.said()
    );
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
    cursor_at(&s, 0, row);
}

#[test]
fn a_plugin_dialog_waits_behind_a_destructive_confirmation() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut a, mut r) = app(place(&work), place(&work), settings(Layout::SideBySide));
    let mut s = session(&mut a, &mut r, 100, 24, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    focus(&mut s, &clock, "notes.txt");
    press(&mut s, &clock, "shift+delete");
    let delete = line_of(&s.screen(), "Delete");
    cursor_at(&s, 0, delete);
    s.event(
        ask(
            7,
            question(
                "Trust this host?",
                PluginQuestionKind::Confirm {
                    destructive: true,
                    yes_label: None,
                    no_label: None,
                },
            ),
        ),
        clock.now(),
    );
    settle(&mut s, &clock);
    cursor_at(&s, 0, delete);
    assert!(
        !s.screen()
            .lines
            .iter()
            .any(|l| l.starts_with("Trust this host?")),
        "never replaced"
    );
    press(&mut s, &clock, "n");
    let line = line_of(
        &s.screen(),
        "Trust this host? No is the default. y or n (SFTP places)",
    );
    cursor_at(&s, 0, line);
    assert!(work.join("notes.txt").exists());
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
}

/// A row of the changed-files list: its name, its id and its state.
fn item(name: &str, state: &str) -> Entry {
    let mut entry = Entry::new(name, EntryKind::Item);
    entry.extra.insert("id".into(), name.into());
    entry.extra.insert("state".into(), state.into());
    entry
}

/// The changed-files list as its plugin opens it, in the active panel.
fn changed_files() -> PluginLister {
    PluginLister {
        plugin_id: "changed-files".into(),
        plugin: "Changed files".into(),
        spec: ListerSpec {
            id: "changes".into(),
            name: "Changed files".into(),
            columns: vec![ListerColumn {
                id: "state".into(),
                header: "State".into(),
                kind: "text".into(),
                sorts: true,
            }],
            activate: Some("diff".into()),
            actions: Vec::new(),
        },
        title: "Changed files (Changed files)".into(),
        rows: vec![item("b.txt", "Modified"), item("a.txt", "Added")],
        panel: PluginPanel::Active,
        total: Some(2),
        truncated: false,
    }
}

#[test]
fn a_plugin_list_is_drawn_in_its_panel_with_the_cursor_on_its_row() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut a, mut r) = app(place(&work), place(&work), settings(Layout::SideBySide));
    let mut s = session(&mut a, &mut r, 100, 24, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    focus(&mut s, &clock, "notes.txt");
    let row = panel_line(&s);
    s.event(
        AppEvent::Plugin(Box::new(PluginEvent::Call {
            call: CallId::new(9),
            plugin: "changed-files".into(),
            request: Call::OpenLister(Box::new(changed_files())),
        })),
        clock.now(),
    );
    settle(&mut s, &clock);
    assert!(
        s.said()
            .iter()
            .any(|m| m.text == "Changed files, 2 rows, left panel (Changed files)"),
        "{:?}",
        s.said()
    );
    assert!(s.app().dialog().is_none(), "in the panel, not a dialog");
    let screen = s.screen();
    assert!(
        screen.lines[0].starts_with("Changed files, left panel (Changed files)"),
        "the title line: {:#?}",
        screen.lines
    );
    // The rows meaning first, the cursor at the focused row's first cell.
    let first = line_of(&screen, "b.txt, state Modified");
    cursor_at(&s, 0, first);
    assert!(screen.lines[first + 1].starts_with("a.txt, state Added"));
    assert!(
        !screen.lines.iter().any(|l| l.starts_with("notes.txt")),
        "the folder's rows are beneath the list: {:#?}",
        screen.lines
    );
    press(&mut s, &clock, "down");
    cursor_at(&s, 0, line_of(&s.screen(), "a.txt, state Added"));
    press(&mut s, &clock, "space");
    line_of(&s.screen(), "* a.txt, selected, state Added");
    assert!(s.said().iter().any(|m| m.text == "Marked"));
    press(&mut s, &clock, "f5");
    assert!(
        s.said().iter().any(|m| m.text == "This row is not a file."),
        "{:?}",
        s.said()
    );
    // Backspace leaves the list: the folder as it was, the cursor on its
    // row again.
    press(&mut s, &clock, "backspace");
    assert!(s.app().active_panel().plugin_list().is_none());
    assert!(
        s.said()
            .iter()
            .any(|m| m.text == "Plugin list closed (Changed files)"),
        "{:?}",
        s.said()
    );
    // The folder's landing line, as when the panel first landed there.
    let landed = &s.said()[0].text;
    assert!(landed.starts_with("work, 9 items"), "{landed}");
    assert_eq!(&s.said().last().unwrap().text, landed);
    cursor_at(&s, 0, row);
    assert!(s.screen().lines[row].starts_with("notes.txt"));
    assert_eq!(s.app().active_side(), Side::Left);
}
