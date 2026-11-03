//! The window in Masonry's in-process harness (spike S6 of the GUI
//! alpha): a real AccessKit tree, read through AccessKit's own consumer,
//! with no window, no display and no graphics card. Started from
//! textweaver's `crates/textweaver-xilem/tests/it/window_tree.rs`.

// Helpers outside the test functions unwrap as the tests do.
#![allow(clippy::unwrap_used)]

use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::Place;
use diroxide_gui::frontend::STATUS_LINE;
use masonry::accesskit::Role;
use masonry::widgets::Label;

use crate::rig::Rig;

/// The app on this crate's folder, both panels.
fn here() -> (App, Runtime) {
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    (
        App::new(AppOptions::new(here.clone(), here)),
        Runtime::with_local(),
    )
}

#[test]
fn the_window_tree_holds_the_title_line_the_panels_and_the_status_bar() {
    let (app, runtime) = here();
    let rig = Rig::new(app, runtime);
    let tree = rig.walk();
    assert_eq!(tree[0].0, Role::Window, "{tree:?}");
    let labels: Vec<&str> = tree
        .iter()
        .filter(|(role, _)| *role == Role::Label)
        .map(|(_, name)| name.as_str())
        .collect();
    assert!(
        labels[0].starts_with("diroxide-gui, left panel, in "),
        "{labels:?}"
    );
    let lists: Vec<&str> = tree
        .iter()
        .filter(|(role, _)| *role == Role::ListBox)
        .map(|(_, name)| name.as_str())
        .collect();
    assert_eq!(
        lists,
        ["Left panel, diroxide-gui", "Right panel, diroxide-gui"]
    );
    // The status bar holds the status line as the app says it, and the
    // jobs line.
    let at = tree
        .iter()
        .position(|(role, _)| *role == Role::Status)
        .unwrap();
    let bar: Vec<&str> = tree[at..]
        .iter()
        .filter(|(role, _)| *role == Role::Label)
        .map(|(_, name)| name.as_str())
        .take(2)
        .collect();
    assert_eq!(bar[0], rig.f.app().status_line(), "{tree:?}");
    assert_eq!(bar[1], "No jobs", "{tree:?}");
    assert_eq!(rig.f.window_title(), "diroxide-gui - diroxide");
}

#[test]
fn a_changed_line_reaches_the_tree() {
    let (app, runtime) = here();
    let mut rig = Rig::new(app, runtime);
    rig.h.edit_widget(STATUS_LINE, |mut label| {
        Label::set_text(&mut label, "Copied 3 items to Backup");
    });
    rig.redraw();
    assert!(
        rig.walk()
            .iter()
            .any(|(role, name)| *role == Role::Label && name == "Copied 3 items to Backup")
    );
}
