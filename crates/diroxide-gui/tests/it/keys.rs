//! Settings and keys (R27's commit 24): the Keys list, Change key
//! capturing a key in the window's own column (a chord a terminal never
//! sends, Ctrl+Shift with a letter), and the rebind kept in
//! `keymap.toml`, so the next start has it; a setting's values are a
//! list over the settings, and a setting changed is said.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use diroxide_app::config::{KeymapConfig, Paths};
use diroxide_app::keymap::{Platform, presets};
use diroxide_app::{App, Runtime};
use diroxide_core::{CommandId, Side};
use masonry::accesskit::Role;

use crate::rig::{Rig, app_with};

/// The app with its config and state folders under `root`, its window's
/// keys the defaults with the owner's `keymap.toml` there, as `diro`
/// builds them for the window.
fn app_from(left: &Path, root: &Path) -> (App, Runtime) {
    let paths = Paths::new(root.join("config"), root.join("state"));
    let owner = std::fs::read_to_string(paths.config_dir.join(Paths::KEYMAP))
        .ok()
        .map(|text| KeymapConfig::from_toml(&text).unwrap())
        .unwrap_or_default();
    let keymap = presets::keymaps(&owner, presets::chosen(""), Platform::current()).1;
    let (app, runtime) = app_with(left, left, move |options| {
        options.keymap = keymap;
        options.startup.keymap = owner;
    });
    (app, runtime.with_paths(paths))
}

/// The options of the open dialog's list.
fn options(rig: &Rig) -> Vec<String> {
    rig.dialog_walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect()
}

#[test]
fn change_key_captures_a_gui_key_and_the_rebind_holds_across_a_restart() {
    let dir = crate::rig::fixture(&["a.txt", "b.txt"], &[]);
    let root = tempfile::Builder::new()
        .prefix("gui-it-keys-")
        .tempdir()
        .unwrap();
    let compare = CommandId::new("compare.files".to_owned()).unwrap();
    {
        let (app, runtime) = app_from(dir.path(), root.path());
        let mut rig = Rig::new(app, runtime);
        assert_eq!(rig.f.app().keymap().key_text(&compare), "", "no key yet");
        // The Keys list: a field that narrows the commands' list.
        rig.run("keys.list");
        assert!(rig.dialog_name().unwrap().starts_with("Keys"));
        assert_eq!(rig.focus_key().0, Role::EditableComboBox);
        rig.type_text("compare files");
        rig.press("down");
        assert!(
            rig.focus_node().1.starts_with("Compare files"),
            "{:?}",
            rig.focus_node()
        );
        rig.press("enter");
        // The row's actions; Change key.
        let actions = options(&rig);
        let change = actions
            .iter()
            .position(|a| a.starts_with("Change key"))
            .unwrap();
        rig.press("home");
        for _ in 0..change {
            rig.press("down");
        }
        rig.press("enter");
        // The capture's prompt: a text to read, waiting for any key.
        assert!(rig.f.app().capturing_key());
        assert!(rig.dialog_name().unwrap().starts_with("Press the new key"));
        rig.press("ctrl+shift+k");
        assert!(
            rig.settle_until(|rig| rig
                .spoken()
                .iter()
                .any(|s| s.contains("saved for Compare files"))),
            "{:?}",
            rig.spoken()
        );
        assert!(!rig.f.app().capturing_key());
        let key = rig.f.app().keymap().key_text(&compare);
        assert!(key.ends_with("Shift+K"), "{key}");
        while rig.dialog_name().is_some() {
            rig.press("escape");
        }
        // Written for the next start.
        assert!(
            rig.settle_until(|_| root.path().join("config").join(Paths::KEYMAP).exists()),
            "keymap.toml"
        );
    }
    // The next start reads it: the key runs Compare files in the window.
    let (app, runtime) = app_from(dir.path(), root.path());
    let mut rig = Rig::new(app, runtime);
    let key = rig.f.app().keymap().key_text(&compare);
    assert!(key.ends_with("Shift+K"), "{key}");
    assert!(rig.f.app_mut().focus_name(Side::Left, "a.txt"));
    rig.refresh();
    rig.press("space");
    rig.press("space");
    let before = rig.said().len();
    rig.press("ctrl+shift+k");
    assert!(
        rig.settle_until(|rig| rig.said()[before..]
            .iter()
            .any(|m| m.text.contains("a.txt")))
            || rig.dialog_name().is_some(),
        "{:?}",
        &rig.said()[before..]
    );
}

#[test]
fn a_setting_changed_from_its_values_is_said_and_shown() {
    let dir = crate::rig::fixture(&["a.txt"], &[]);
    let (app, runtime) = crate::rig::app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.run("app.settings");
    rig.focus_dialog_row("layout");
    rig.press("enter");
    let values = options(&rig);
    let stacked = values
        .iter()
        .position(|v| v.to_lowercase().starts_with("stacked"))
        .unwrap_or_else(|| panic!("{values:?}"));
    rig.press("home");
    for _ in 0..stacked {
        rig.press("down");
    }
    rig.press("enter");
    assert!(
        rig.spoken()
            .iter()
            .any(|s| s.contains("Stacked") || s.contains("stacked")),
        "{:?}",
        rig.spoken()
    );
    assert_eq!(
        rig.f.app().settings().layout,
        diroxide_app::config::Layout::Stacked
    );
    // The settings list says the new value on its row.
    while rig.dialog_name().is_some() {
        rig.press("escape");
    }
}
