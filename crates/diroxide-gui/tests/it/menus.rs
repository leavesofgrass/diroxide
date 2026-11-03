//! The menus (R27's commit 13): with a native bar the menu keys enter
//! it (Alt alone, F9, F10, Alt with a menu's letter) instead of the app's
//! list menus; without one (Linux, `menus = list`) they open the app's
//! list menus in the modal host.

#![allow(clippy::unwrap_used)]

use diroxide_gui::frontend::MenuEnter;
use masonry::accesskit::Role;
use masonry::core::TextEvent;
use masonry::core::keyboard::{Key, KeyState, KeyboardEvent, Modifiers, NamedKey};

use crate::rig::{Rig, app_on, fixture};

fn alt(state: KeyState) -> TextEvent {
    TextEvent::Keyboard(KeyboardEvent {
        state,
        key: Key::Named(NamedKey::Alt),
        modifiers: if state == KeyState::Down {
            Modifiers::ALT
        } else {
            Modifiers::empty()
        },
        ..Default::default()
    })
}

#[test]
fn with_a_native_bar_the_menu_keys_enter_it() {
    let dir = fixture(&["a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.f.set_menu_bar(true);
    rig.press("f10");
    rig.press("f9");
    rig.press("alt+f");
    assert_eq!(
        rig.f.take_menu_requests(),
        [MenuEnter::Bar, MenuEnter::Bar, MenuEnter::Menu('f')]
    );
    assert!(rig.f.app().dialog().is_none(), "no list menu opened");
    // Alt alone, pressed and let go.
    let _ = rig.h.process_text_event(alt(KeyState::Down));
    let _ = rig.h.process_text_event(alt(KeyState::Up));
    rig.settle();
    assert_eq!(rig.f.take_menu_requests(), [MenuEnter::Bar]);
    // Alt with a key between is not Alt alone.
    let _ = rig.h.process_text_event(alt(KeyState::Down));
    rig.press("alt+left");
    let _ = rig.h.process_text_event(alt(KeyState::Up));
    rig.settle();
    assert!(rig.f.take_menu_requests().is_empty());
    // A dialog open: the keys are the dialog's.
    rig.press("ctrl+shift+p");
    rig.press("f10");
    assert!(rig.f.take_menu_requests().is_empty());
}

#[test]
fn without_a_native_bar_the_menus_are_lists_in_the_window() {
    let dir = fixture(&["a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.press("f9");
    let tree = rig.walk();
    let dialog = tree
        .iter()
        .find(|(role, _)| *role == Role::Dialog)
        .map(|(_, name)| name.as_str());
    assert_eq!(dialog, Some("Menu bar"), "{tree:?}");
    let (role, name) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(name.starts_with("File"), "{name}");
    rig.press("escape");
    assert!(rig.f.app().dialog().is_none());
    assert!(rig.f.take_menu_requests().is_empty());
}
