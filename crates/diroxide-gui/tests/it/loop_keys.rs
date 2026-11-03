//! The GUI's loop with scripted keys (R27's commit 8): key events through
//! the window's root to the app, the workers' results back, as the
//! terminal's session test does.

#![allow(clippy::unwrap_used)]

use diroxide_core::{Place, Side};

use crate::rig::{Rig, app_on, fixture};

#[test]
fn scripted_keys_open_a_folder_go_back_and_quit() {
    let dir = fixture(&["a.txt", "b.txt"], &["Photos"]);
    std::fs::write(dir.path().join("Photos").join("crow.jpg"), b"caw").unwrap();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let start = Place::parse(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(rig.f.app().active_panel().place(), &start);
    // The first row is focused; Down and Up move the app's focus.
    let first = rig.f.app().active_panel().list().focus_index();
    assert_eq!(first, Some(0));
    rig.press("down");
    assert_eq!(rig.f.app().active_panel().list().focus_index(), Some(1));
    rig.press("up");
    assert_eq!(rig.f.app().active_panel().list().focus_index(), Some(0));
    // Into the folder by Enter, and back by Backspace.
    assert!(rig.f.app_mut().focus_name(Side::Left, "Photos"));
    rig.press("enter");
    let photos = Place::parse(dir.path().join("Photos").to_str().unwrap()).unwrap();
    assert_eq!(rig.f.app().active_panel().place(), &photos);
    assert_eq!(
        rig.f
            .app()
            .active_panel()
            .list()
            .focused()
            .map(|r| r.id.as_str()),
        Some("crow.jpg")
    );
    rig.press("backspace");
    assert_eq!(rig.f.app().active_panel().place(), &start);
    // Tab is the app's (Switch panel), not Masonry's focus key.
    rig.press("tab");
    assert_eq!(rig.active(), Side::Right);
    assert_eq!(rig.h.focused_widget_id(), Some(rig.panel_id(Side::Right)));
    rig.press("tab");
    assert_eq!(rig.active(), Side::Left);
    assert_eq!(rig.h.focused_widget_id(), Some(rig.panel_id(Side::Left)));
    // Ctrl+Q quits by the quit rule: nothing runs, so it does not ask.
    assert!(!rig.f.wants_quit());
    rig.press("ctrl+q");
    assert!(rig.f.wants_quit());
}

#[test]
fn a_released_key_and_a_held_function_key_run_nothing() {
    let dir = fixture(&["a.txt", "b.txt", "c.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let key = diroxide_core::Key::parse("down").unwrap();
    let mut held = diroxide_gui::keys::press(&key, diroxide_app::keymap::Platform::current());
    held.repeat = true;
    let _ = rig
        .h
        .process_text_event(masonry::core::TextEvent::Keyboard(held));
    rig.settle();
    // A held arrow moves.
    assert_eq!(rig.f.app().active_panel().list().focus_index(), Some(1));
    let mut released = diroxide_gui::keys::press(&key, diroxide_app::keymap::Platform::current());
    released.state = masonry::core::keyboard::KeyState::Up;
    let _ = rig
        .h
        .process_text_event(masonry::core::TextEvent::Keyboard(released));
    rig.settle();
    assert_eq!(rig.f.app().active_panel().list().focus_index(), Some(1));
    // F11 and F12 are never Masonry's debugging keys.
    rig.press("f12");
    rig.press("f11");
    assert!(!rig.f.wants_quit());
}
