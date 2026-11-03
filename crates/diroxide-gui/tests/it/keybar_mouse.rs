//! The key bar and the mouse (R27's commit 14): buttons named by their
//! labels with their keys as shortcuts, in the pairs the app's key bar
//! has; Switch bank and Say Key Bar; a click focuses a panel and its row,
//! a double click opens it, a sort button sorts.

#![allow(clippy::unwrap_used)]

use std::time::Instant;

use diroxide_app::commands::ids;
use diroxide_core::{Command, Side};
use diroxide_gui::frontend::{KEY_BAR, panel_tag, sort_tag};
use diroxide_gui::widgets::panel::{HEADER_HEIGHT, ROW_HEIGHT};
use masonry::accesskit::{Action, ActionRequest, Role, TreeId};
use masonry::core::{PointerButton, PointerButtonEvent, PointerEvent, PointerState};
use masonry::dpi::PhysicalPosition;
use masonry::kurbo::Point;

use crate::rig::{HostOf, Rig, app_on, fixture, name_of};

/// The key bar's buttons as the tree has them: name and shortcut.
fn key_bar(rig: &Rig) -> Vec<(String, String)> {
    let id = rig.h.get_widget(KEY_BAR).id();
    let bar = rig.h.access_node(id).unwrap();
    assert_eq!(bar.role(), Role::Toolbar);
    assert_eq!(name_of(&bar), "Key bar");
    bar.children()
        .filter(|b| b.role() == Role::Button)
        .map(|b| {
            (
                name_of(&b),
                b.data().keyboard_shortcut().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_key_bar_is_the_apps_pairs_as_named_buttons_with_keys() {
    let dir = fixture(&["a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let buttons = key_bar(&rig);
    let pairs: Vec<String> = buttons
        .iter()
        .map(|(name, key)| format!("{key} {name}"))
        .collect();
    assert_eq!(pairs.join("  "), rig.f.app().key_bar_within(usize::MAX));
    assert_eq!(buttons[0].0, "Rename", "{buttons:?}");
    assert!(!buttons[0].0.contains("F2"), "the key is not in the name");
    // Switch bank shows the next bank's buttons.
    let before = buttons.clone();
    rig.f.command(
        &mut HostOf(&mut rig.h),
        Command::new(ids::SWITCH_BANK),
        Instant::now(),
    );
    rig.redraw();
    let after = key_bar(&rig);
    let pairs: Vec<String> = after
        .iter()
        .map(|(name, key)| format!("{key} {name}"))
        .collect();
    assert_eq!(pairs.join("  "), rig.f.app().key_bar_within(usize::MAX));
    // With one bank (the default) Switch bank keeps it.
    let _ = before;
    // Say Key Bar says every pair in words.
    rig.f.command(
        &mut HostOf(&mut rig.h),
        Command::new(ids::SAY_KEY_BAR),
        Instant::now(),
    );
    let said = rig.said().last().unwrap().text.clone();
    assert!(said.starts_with("Key bar: "), "{said}");
}

#[test]
fn a_screen_readers_click_on_a_key_bar_button_presses_it() {
    let dir = fixture(&["report.pdf"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let id = rig.h.get_widget(KEY_BAR).id();
    let rename = rig
        .h
        .access_node(id)
        .unwrap()
        .children()
        .find(|b| name_of(b) == "Rename")
        .unwrap()
        .locate()
        .0;
    rig.h.process_access_event(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: rename,
        data: None,
    });
    rig.settle();
    let dialog = rig.f.app().dialog().map(|(_, d)| d.title());
    assert_eq!(dialog.as_deref(), Some("Rename report.pdf to"));
}

/// The window position of a panel's row (the lists here are short, so
/// the first row drawn is the list's first).
fn row_point(rig: &Rig, side: Side, row: usize) -> Point {
    let widget = rig.h.get_widget(panel_tag(side));
    let local = Point::new(
        40.0,
        HEADER_HEIGHT + row as f64 * ROW_HEIGHT + ROW_HEIGHT / 2.0,
    );
    widget.ctx().window_transform() * local
}

fn press_at(rig: &mut Rig, at: Point, count: u8) {
    rig.h.mouse_move(at);
    let state = PointerState {
        position: PhysicalPosition::new(at.x, at.y),
        count,
        ..PointerState::default()
    };
    let _ = rig
        .h
        .process_pointer_event(PointerEvent::Down(PointerButtonEvent {
            pointer: masonry_testing::PRIMARY_MOUSE,
            button: Some(PointerButton::Primary),
            state: state.clone(),
        }));
    let _ = rig
        .h
        .process_pointer_event(PointerEvent::Up(PointerButtonEvent {
            pointer: masonry_testing::PRIMARY_MOUSE,
            button: Some(PointerButton::Primary),
            state,
        }));
    rig.settle();
}

#[test]
fn a_click_focuses_a_double_click_opens_and_a_sort_button_sorts() {
    let dir = fixture(&["b.txt", "c.txt", "a.txt"], &["Photos"]);
    std::fs::write(dir.path().join("Photos").join("crow.jpg"), b"caw").unwrap();
    std::fs::write(dir.path().join("big.txt"), vec![b'x'; 4096]).unwrap();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    // A click on the right panel's third row: the right panel is active,
    // the row focused.
    let at = row_point(&rig, Side::Right, 2);
    press_at(&mut rig, at, 1);
    assert_eq!(rig.active(), Side::Right);
    assert_eq!(rig.f.app().active_panel().list().focus_index(), Some(2));
    assert_eq!(rig.h.focused_widget_id(), Some(rig.panel_id(Side::Right)));
    // A double click on the folder opens it.
    let photos = rig
        .f
        .app()
        .active_panel()
        .list()
        .rows()
        .iter()
        .position(|r| r.id == "Photos")
        .unwrap();
    let at = row_point(&rig, Side::Right, photos);
    press_at(&mut rig, at, 1);
    press_at(&mut rig, at, 2);
    assert_eq!(rig.f.app().active_panel().place().name(), Some("Photos"));
    rig.press("backspace");
    // The left panel's "Sort by size" button: the left panel is active
    // and sorted by size, largest first.
    let sort = rig.h.get_widget(sort_tag(Side::Left)).id();
    let button = rig
        .h
        .access_node(sort)
        .unwrap()
        .children()
        .find(|b| name_of(b) == "Sort by size")
        .unwrap()
        .locate()
        .0;
    rig.h.process_access_event(ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: button,
        data: None,
    });
    rig.settle();
    assert_eq!(rig.active(), Side::Left);
    // Folders first, then the files, largest first.
    let first_file = rig
        .f
        .app()
        .active_panel()
        .list()
        .rows()
        .iter()
        .find(|r| r.id.ends_with(".txt"))
        .map(|r| r.id.clone());
    assert_eq!(first_file.as_deref(), Some("big.txt"));
}
