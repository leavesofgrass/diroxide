//! A plugin's keys in `keymap.toml` and in the Keys list (lane A2, W4d's
//! request 2): the owner's line for a plugin command takes its key over
//! from the plugin's proposal, is not reported as an unknown command, and
//! the Keys list lists every plugin command with its keys, how it got
//! them, and the plugin last.

use std::fs;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_core::{CommandId, Key, Layer};

use crate::plugins::{MANIFEST, TestLauncher, enable, session, world};

#[test]
fn keymap_toml_sets_a_plugin_commands_key_and_the_keys_list_shows_it() {
    let w = world(MANIFEST);
    enable(&w.paths);
    fs::create_dir_all(&w.paths.config_dir).unwrap();
    fs::write(
        w.paths.config_file(diroxide_config::Paths::KEYMAP),
        "[terminal.global]\n\"line-counter.count\" = \"ctrl+alt+k\"\n\n[gui.global]\n\"line-counter.count\" = \"ctrl+alt+k\"\n",
    )
    .unwrap();
    let mut s = session(&w, TestLauncher::with(&[]));
    let count = CommandId::new("line-counter.count").unwrap();
    let held = s
        .app
        .keymap()
        .binding(Layer::Global, &Key::parse("ctrl+alt+k").unwrap())
        .map(|b| b.command.clone());
    assert_eq!(held, Some(count.clone()), "the owner's key");
    let proposed = s
        .app
        .keymap()
        .binding(Layer::Global, &Key::parse("ctrl+alt+l").unwrap())
        .map(|b| b.command.clone());
    assert_eq!(proposed, None, "the proposal gave way");
    assert!(
        !s.app
            .problems()
            .iter()
            .any(|p| p.text.contains("line-counter.count")),
        "{:?}",
        s.app.problems()
    );

    s.command(ids::KEYS_LIST);
    let rows: Vec<String> = match s.dialog() {
        Some(Dialog::Edit {
            candidates: Some(list),
            ..
        }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("{other:?}"),
    };
    assert!(
        rows.contains(&"Count lines, Ctrl+Alt+K, set in keymap.toml, by Line counter".to_owned()),
        "{rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|r| r
                .starts_with("Copy with a taken key, no key, F5 not bound: it already runs Copy")),
        "{rows:#?}"
    );
}
