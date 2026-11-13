//! Tabs in the window (alpha 5, R34's commit 32): the decided keys reach
//! the window and run the tab commands (Ctrl+T, Ctrl+W, Ctrl+Tab,
//! Ctrl+Shift+Tab and Alt+1 to Alt+9); the title line, the window's
//! title, both panels' headers and the list boxes' names carry the tab
//! words after the place, only with more than one tab; and no preset
//! takes those keys for something else. No tab bar is drawn: the state
//! is words.

#![allow(clippy::unwrap_used)]

use diroxide_app::Lookup;
use diroxide_app::keymap::{Platform, presets};
use diroxide_core::{Key, KeyColumn, Layer, Side};
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, fixture};

/// The window over a folder named `work`, both panels.
fn window() -> (tempfile::TempDir, Rig) {
    let dir = fixture(&[], &["work"]);
    std::fs::write(dir.path().join("work").join("a.txt"), b"diroxide").unwrap();
    let work = dir.path().join("work");
    let (app, runtime) = app_on(&work, &work);
    (dir, Rig::new(app, runtime))
}

/// The labels in the tree, in order: the title line first.
fn labels(rig: &Rig) -> Vec<String> {
    rig.walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::Label)
        .map(|(_, name)| name)
        .collect()
}

/// The two panels' list boxes' names.
fn lists(rig: &Rig) -> Vec<String> {
    rig.walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBox)
        .map(|(_, name)| name)
        .collect()
}

/// Every line fits a 40-cell Braille line.
fn fits(line: &str) {
    assert!(line.chars().count() <= 40, "{line}: over 40 cells");
}

#[test]
fn the_tab_keys_reach_the_window_and_the_tab_words_follow_the_place() {
    let (_dir, mut rig) = window();
    // One tab: nothing about tabs anywhere.
    assert_eq!(rig.f.window_title(), "work - diroxide");
    assert!(labels(&rig)[0].starts_with("work, left panel, "));
    assert_eq!(lists(&rig), ["Left panel, work", "Right panel, work"]);

    // Ctrl+T: a new tab, said with its number (33 cells at most here).
    rig.press("ctrl+t");
    assert_eq!(rig.f.app().tab_count(Side::Left), 2);
    let said = rig.spoken();
    let new = said.last().unwrap();
    assert_eq!(new, "New tab: work, tab 2 of 2");
    fits(new);
    // "work, tab 2 of 2 - diroxide", 27 cells: the window's title, said
    // when the window takes the focus.
    assert_eq!(rig.f.window_title(), "work, tab 2 of 2 - diroxide");
    fits(&rig.f.window_title());
    let title = labels(&rig)[0].clone();
    assert!(
        title.starts_with("work, tab 2 of 2, left panel, "),
        "{title}"
    );
    // The active list's name, "Left panel, work, tab 2 of 2" (28); the
    // other panel has one tab and says none.
    assert_eq!(
        lists(&rig),
        ["Left panel, work, tab 2 of 2", "Right panel, work"]
    );
    fits(&lists(&rig)[0]);

    // Ctrl+Shift+Tab, Ctrl+Tab and Alt+1 move between the tabs.
    rig.press("ctrl+shift+tab");
    assert_eq!(rig.f.app().tab_index(Side::Left), 0);
    assert_eq!(rig.spoken().last().unwrap(), "work, tab 1 of 2");
    assert_eq!(rig.f.window_title(), "work, tab 1 of 2 - diroxide");
    rig.press("ctrl+tab");
    assert_eq!(rig.f.app().tab_index(Side::Left), 1);
    assert_eq!(rig.spoken().last().unwrap(), "work, tab 2 of 2");
    rig.press("alt+1");
    assert_eq!(rig.f.app().tab_index(Side::Left), 0);
    assert_eq!(rig.spoken().last().unwrap(), "work, tab 1 of 2");
    // The focus is still on the panel's list, its row named.
    let (role, _) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);

    // Ctrl+W closes the shown tab; one is left, and the words go.
    rig.press("ctrl+w");
    assert_eq!(rig.f.app().tab_count(Side::Left), 1);
    let closed = rig.spoken().last().unwrap().clone();
    assert_eq!(closed, "Closed work. work.");
    assert_eq!(rig.f.window_title(), "work - diroxide");
    assert_eq!(lists(&rig), ["Left panel, work", "Right panel, work"]);
}

#[test]
fn the_other_panels_header_says_its_tab() {
    let (_dir, mut rig) = window();
    rig.press("ctrl+t");
    // Tab moves to the right panel; the left panel's header is now drawn
    // from its own place and says its tab after the folder.
    rig.press("tab");
    assert_eq!(rig.active(), Side::Right);
    let header = diroxide_gui::model::inactive_title(rig.f.app(), Side::Left);
    assert!(
        header.starts_with("work, tab 2 of 2, left panel, 1 item"),
        "{header}"
    );
    // The active right panel has one tab: the title line says none.
    let title = labels(&rig)[0].clone();
    assert!(title.starts_with("work, right panel, "), "{title}");
    assert_eq!(rig.f.window_title(), "work - diroxide");
}

#[test]
fn every_preset_keeps_the_tab_keys_in_the_window() {
    let keys = [
        ("ctrl+t", "tab.new"),
        ("ctrl+w", "tab.close"),
        ("ctrl+tab", "tab.next"),
        ("ctrl+shift+tab", "tab.previous"),
        ("alt+1", "tab.go-1"),
        ("alt+2", "tab.go-2"),
        ("alt+3", "tab.go-3"),
        ("alt+4", "tab.go-4"),
        ("alt+5", "tab.go-5"),
        ("alt+6", "tab.go-6"),
        ("alt+7", "tab.go-7"),
        ("alt+8", "tab.go-8"),
        ("alt+9", "tab.go-9"),
    ];
    let layers = [Layer::Global, Layer::Panel, Layer::List];
    let all = presets::presets();
    assert_eq!(all.len(), 6, "the defaults and the five presets");
    for preset in all {
        assert!(
            preset.problems.is_empty(),
            "{}: {:?}",
            preset.id,
            preset.problems
        );
        for platform in [Platform::Windows, Platform::Other] {
            let keymap = preset.keymap(KeyColumn::Gui, platform);
            for (text, command) in keys {
                let key = Key::parse(text).unwrap();
                match keymap.lookup(&layers, &key) {
                    Lookup::Command(id) => assert_eq!(
                        id.as_str(),
                        command,
                        "{}: {text} runs {id} on {platform:?}",
                        preset.id
                    ),
                    Lookup::NoMatch => {
                        panic!("{}: {text} runs nothing on {platform:?}", preset.id)
                    }
                }
            }
        }
    }
}
