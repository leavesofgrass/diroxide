//! The keys the window cannot be trusted with
//! (`crates/diroxide-app/keys/gui-unsafe.toml`) and their reachability
//! test: no default in the keymap's GUI column uses one, so every
//! command's GUI key reaches the window.

#![allow(clippy::unwrap_used)]

use diroxide_app::Keymap;
use diroxide_app::keymap::Platform;
use diroxide_core::{Key, KeyColumn, Layer};
use serde::Deserialize;

/// The sheet's text.
const GUI_UNSAFE: &str = include_str!("../../../diroxide-app/keys/gui-unsafe.toml");

#[derive(Debug, Deserialize)]
struct Sheet {
    key: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    keys: Vec<String>,
    kind: String,
    why: String,
}

#[test]
fn no_default_in_the_gui_column_uses_a_key_the_window_cannot_get() {
    let sheet: Sheet = toml::from_str(GUI_UNSAFE).unwrap();
    assert!(!sheet.key.is_empty());
    let layers = [
        Layer::Global,
        Layer::Panel,
        Layer::List,
        Layer::Dialog,
        Layer::Viewer,
        Layer::Preview,
    ];
    for platform in [Platform::Windows, Platform::Other] {
        let keymap = Keymap::defaults_for(KeyColumn::Gui).for_platform(platform);
        for entry in &sheet.key {
            assert!(
                ["system", "reader", "dropped"].contains(&entry.kind.as_str()),
                "{entry:?}"
            );
            assert!(
                !entry.why.is_empty() && !entry.why.ends_with('.'),
                "{entry:?}"
            );
            for text in &entry.keys {
                let key = Key::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
                let found = keymap.lookup(&layers, &key);
                assert!(
                    matches!(found, diroxide_app::Lookup::NoMatch),
                    "{text} cannot reach diroxide's window ({}), but the GUI column binds it to {found:?} on {platform:?}",
                    entry.why
                );
            }
        }
    }
}

#[test]
fn the_windows_key_chords_never_reach_the_keymap() {
    // The window drops them (keys::chord), so the sheet's "dropped" keys
    // are never pressed, whatever the keymap says.
    let key = Key::parse("meta+e").unwrap();
    let event = diroxide_gui::keys::press(&key, Platform::MacOs);
    assert_eq!(
        diroxide_gui::keys::to_key(&event, Platform::Windows, false),
        None
    );
}
