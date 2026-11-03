//! A plugin's prompts in the window (R27's commit 29): a question of each
//! of the five kinds and a text document, each in one of the dialog
//! shapes, named with the plugin's name last, its focus on a named
//! control, closed by Escape with the focus back on the panel's row; a
//! secret never in the tree; a plugin's dialog waiting behind a
//! destructive confirmation; the consent question in Manage plugins
//! starting on No; and the Plugins menu with Manage plugins.
//!
//! The plugin's calls arrive as the runtime sends them
//! ([`PluginEvent::Call`]), the fake plugin's questions as the terminal's
//! test has them (`crates/diroxide-tui/tests/it/plugins.rs`); no plugin
//! runs.

#![allow(clippy::unwrap_used)]

use diroxide_app::AppEvent;
use diroxide_app::plugins::{
    Call, CallId, Catalog, Choice, Item, ItemKind, ItemState, PathMode, PluginDocument,
    PluginEvent, PluginQuestion, PluginQuestionKind, TextKind,
};
use diroxide_core::Side;
use masonry::accesskit::Role;

use crate::rig::{HostOf, Rig, app_on, fixture};

/// The fake plugin's question, titled with its name last as the host
/// cleans it.
pub fn question(title: &str, kind: PluginQuestionKind) -> PluginQuestion {
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

/// The call that asks it.
pub fn ask(n: u64, question: PluginQuestion) -> AppEvent {
    AppEvent::Plugin(Box::new(PluginEvent::Call {
        call: CallId::new(n),
        plugin: "sftp".into(),
        request: Call::Ask(Box::new(question)),
    }))
}

/// Hands an event to the window's driver as the runtime would, and
/// settles.
pub fn send(rig: &mut Rig, event: AppEvent) {
    let now = rig.now;
    rig.f.event(&mut HostOf(&mut rig.h), event, now);
    rig.settle();
}

/// The window over a folder with the focus on notes.txt.
fn window() -> (tempfile::TempDir, Rig) {
    let dir = fixture(&["notes.txt", "report.pdf"], &["Backup"]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    (dir, rig)
}

/// Escape closes the dialog and the focus is back on the panel's row.
fn escape_back_to_the_row(rig: &mut Rig) {
    rig.press("escape");
    assert!(rig.dialog_name().is_none(), "{:?}", rig.dialog_name());
    let (role, name) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(name.starts_with("notes.txt"), "{name}");
}

#[test]
fn every_kind_of_plugin_question_is_a_dialog_with_the_plugin_last() {
    let (_dir, mut rig) = window();

    // Confirm: the two buttons, No first and focused; the question names
    // the dialog with the plugin last; its message is a list to read.
    send(
        &mut rig,
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
    );
    let name = rig.dialog_name().unwrap();
    assert_eq!(
        name,
        "Trust this host? No is the default. y or n (SFTP places)"
    );
    assert_eq!(rig.focus_node(), (Role::Button, "No".to_owned()));
    let buttons: Vec<String> = rig.dialog_buttons().into_iter().map(|b| b.1).collect();
    assert_eq!(buttons, ["No", "Yes"]);
    assert!(
        rig.dialog_walk()
            .iter()
            .any(|(_, n)| n.starts_with("Key: 1a2b 3c4d")),
        "{:?}",
        rig.dialog_walk()
    );
    escape_back_to_the_row(&mut rig);

    // A confirm with words of its own: a list of the two, starting on
    // the no word when it is destructive.
    let mut own = question(
        "New server key",
        PluginQuestionKind::Confirm {
            destructive: true,
            yes_label: Some("Trust and connect".into()),
            no_label: Some("Do not connect".into()),
        },
    );
    own.lines = vec!["New server key for build.example.".into()];
    send(&mut rig, ask(2, own));
    assert_eq!(
        rig.dialog_name().as_deref(),
        Some("New server key (SFTP places)")
    );
    assert_eq!(
        rig.focus_node(),
        (Role::ListBoxOption, "Do not connect".to_owned())
    );
    escape_back_to_the_row(&mut rig);

    // Choose: the message, then the choices; the focus on the default;
    // a choice's description after its label.
    send(
        &mut rig,
        ask(
            3,
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
    );
    assert_eq!(
        rig.dialog_name().as_deref(),
        Some("Pick a branch (SFTP places)")
    );
    let (role, focused) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(
        focused.starts_with("feature, two commits ahead"),
        "{focused}"
    );
    rig.press("up");
    assert!(rig.focus_node().1.starts_with("main"));
    escape_back_to_the_row(&mut rig);

    // Choose with multiple: Space marks, and the mark is the option's
    // state (a word in its name on macOS).
    send(
        &mut rig,
        ask(
            4,
            question(
                "Pick files",
                PluginQuestionKind::Choose {
                    choices: vec![
                        Choice {
                            id: "x".into(),
                            label: "x.txt".into(),
                            description: None,
                        },
                        Choice {
                            id: "y".into(),
                            label: "y.txt".into(),
                            description: None,
                        },
                    ],
                    default_id: None,
                    multiple: true,
                },
            ),
        ),
    );
    rig.press("space");
    let marked = rig.f.app().dialog().map(|(_, d)| match d {
        diroxide_app::Dialog::Question { choices, .. } => choices.selected_count(),
        _ => 0,
    });
    assert_eq!(marked, Some(1));
    escape_back_to_the_row(&mut rig);

    // Text: the field named by the prompt, the plugin last, its text in
    // the field; what is typed goes in at the caret.
    send(
        &mut rig,
        ask(
            5,
            question(
                "Branch name",
                PluginQuestionKind::Text {
                    initial: "ma".into(),
                    pattern: None,
                },
            ),
        ),
    );
    assert_eq!(
        rig.dialog_name().as_deref(),
        Some("Branch name (SFTP places)")
    );
    rig.type_text("in");
    let field = rig.f.app().dialog().and_then(|(_, d)| match d {
        diroxide_app::Dialog::Edit { field, .. } => Some(field.text().to_owned()),
        _ => None,
    });
    assert_eq!(field.as_deref(), Some("main"));
    escape_back_to_the_row(&mut rig);

    // Path: the same field, places to complete.
    send(
        &mut rig,
        ask(
            6,
            question(
                "Where to",
                PluginQuestionKind::Path {
                    mode: PathMode::Folder,
                    must_exist: false,
                    initial: String::new(),
                },
            ),
        ),
    );
    assert_eq!(rig.dialog_name().as_deref(), Some("Where to (SFTP places)"));
    assert!(matches!(
        rig.focus_node().0,
        Role::EditableComboBox | Role::TextInput
    ));
    escape_back_to_the_row(&mut rig);

    // Secret: the masked field and the Show toggle; what is typed is in
    // no node of the tree.
    send(
        &mut rig,
        ask(7, question("Token", PluginQuestionKind::Secret)),
    );
    let name = rig.dialog_name().unwrap();
    assert!(name.starts_with("Token (SFTP places)"), "{name}");
    rig.type_text("canary");
    let tree = format!("{:?}", rig.walk());
    assert!(!tree.contains("canary"), "{tree}");
    assert!(
        rig.dialog_buttons()
            .iter()
            .any(|(role, n)| *role == Role::CheckBox && n.starts_with("Show the")),
        "{:?}",
        rig.dialog_buttons()
    );
    escape_back_to_the_row(&mut rig);

    // A text document: a list of its lines to read, the focus on the
    // first, Down to the next.
    send(
        &mut rig,
        AppEvent::Plugin(Box::new(PluginEvent::Call {
            call: CallId::new(8),
            plugin: "sftp".into(),
            request: Call::ShowText(Box::new(PluginDocument {
                plugin: "SFTP places".into(),
                title: "Host key (SFTP places)".into(),
                lines: vec!["1a2b 3c4d".into(), "5e6f 7a8b".into()],
                kind: TextKind::Plain,
                wait: false,
            })),
        })),
    );
    assert!(
        rig.dialog_name()
            .is_some_and(|n| n.starts_with("Host key (SFTP places)")),
        "{:?}",
        rig.dialog_name()
    );
    assert_eq!(
        rig.focus_node(),
        (Role::ListBoxOption, "1a2b 3c4d".to_owned())
    );
    rig.press("down");
    assert_eq!(rig.focus_node().1, "5e6f 7a8b");
    escape_back_to_the_row(&mut rig);
    assert_eq!(rig.f.app().active_side(), Side::Left);
}

#[test]
fn a_plugin_dialog_waits_behind_a_destructive_confirmation() {
    let (dir, mut rig) = window();
    rig.press("shift+delete");
    let delete = rig.dialog_name().unwrap();
    assert!(delete.starts_with("Delete"), "{delete}");
    send(
        &mut rig,
        ask(
            9,
            question(
                "Trust this host?",
                PluginQuestionKind::Confirm {
                    destructive: true,
                    yes_label: None,
                    no_label: None,
                },
            ),
        ),
    );
    // Never replaced while it is open: the delete question stays.
    assert_eq!(rig.dialog_name().unwrap(), delete);
    assert_eq!(rig.dialogs_in_tree(), 1);
    rig.press("n");
    let next = rig.dialog_name().unwrap();
    assert!(next.starts_with("Trust this host?"), "{next}");
    assert!(next.ends_with("(SFTP places)"), "{next}");
    assert!(dir.path().join("notes.txt").exists());
    escape_back_to_the_row(&mut rig);
}

#[test]
fn manage_plugins_lists_them_and_the_consent_starts_on_no() {
    let (dir, mut rig) = window();
    let item = Item {
        id: "total-size".into(),
        name: "Total size of selection".into(),
        kind: ItemKind::Script,
        path: dir.path().join("total-size.sh"),
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
    rig.run("plugins.list");
    assert!(
        rig.dialog_name()
            .is_some_and(|n| n.starts_with("Manage plugins")),
        "{:?}",
        rig.dialog_name()
    );
    // The plugins found arrive while the list is open: it shows them.
    send(
        &mut rig,
        AppEvent::Plugin(Box::new(PluginEvent::Found(Box::new(Catalog {
            off: false,
            items: vec![item],
            problems: Vec::new(),
        })))),
    );
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(
        row.starts_with("Total size of selection: not enabled, script"),
        "{row}"
    );
    rig.press("enter");
    assert!(rig.focus_node().1.starts_with("Enable"));
    rig.press("enter");
    let consent = rig.dialog_name().unwrap();
    assert!(
        consent.starts_with("Enable Total size of selection? No is the default"),
        "{consent}"
    );
    assert_eq!(rig.focus_node(), (Role::Button, "No".to_owned()));
    assert!(
        rig.dialog_walk()
            .iter()
            .any(|(_, n)| n.starts_with("It runs: bash total-size.sh")),
        "{:?}",
        rig.dialog_walk()
    );
    rig.press("n");
    assert!(
        rig.said().iter().any(|m| m.text == "Not enabled."),
        "{:?}",
        rig.said()
    );
}

#[test]
fn the_plugins_menu_holds_manage_plugins() {
    let (_dir, rig) = window();
    let tree = diroxide_gui::menus::tree(rig.f.app());
    let plugins = tree
        .iter()
        .find(|menu| menu.label.replace('&', "") == "Plugins")
        .unwrap_or_else(|| panic!("no Plugins menu in {tree:?}"));
    let labels: Vec<String> = plugins
        .entries
        .iter()
        .map(diroxide_gui::menus::Entry::windows_text)
        .collect();
    assert!(
        labels
            .iter()
            .any(|l| l.replace('&', "").starts_with("Manage plugins")),
        "{labels:?}"
    );
}
