//! The window in Masonry's in-process harness (spike S6 of the GUI
//! alpha): a real AccessKit tree, read through AccessKit's own consumer,
//! with no window, no display and no graphics card. Started from
//! textweaver's `crates/textweaver-xilem/tests/it/window_tree.rs`.

// Helpers outside the test functions unwrap as the tests do.
#![allow(clippy::unwrap_used)]

use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::Place;
use diroxide_gui::window::{STATUS_LINE, TITLE_LINE, lines, root_widget};
use masonry::accesskit::Role;
use masonry::widgets::Label;
use masonry_testing::TestHarness;

/// The app on this crate's folder, its listing read.
fn app_here() -> App {
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut app = App::new(AppOptions::new(here.clone(), here));
    let mut runtime = Runtime::with_local();
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    app
}

/// Every node's role and name, depth first from the root, as a screen
/// reader's tree walk finds them.
fn walk(harness: &TestHarness<masonry::widgets::Flex>) -> Vec<(Role, String)> {
    let mut found = Vec::new();
    let mut stack = vec![harness.access_tree().state().root()];
    while let Some(node) = stack.pop() {
        // A label's name is its text, as the platform adapters read it.
        let name = if node.label_comes_from_value() {
            node.value()
        } else {
            node.label()
        };
        found.push((node.role(), name.unwrap_or_default()));
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        stack.extend(children);
    }
    found
}

#[test]
fn the_window_tree_holds_the_title_and_status_lines_in_words() {
    let app = app_here();
    let harness = TestHarness::create(masonry::theme::default_property_set(), root_widget(&app));
    let tree = walk(&harness);
    assert_eq!(tree[0].0, Role::Window, "{tree:?}");
    let labels: Vec<&str> = tree
        .iter()
        .filter(|(role, _)| *role == Role::Label)
        .map(|(_, name)| name.as_str())
        .collect();
    assert_eq!(labels.len(), 2, "{tree:?}");
    assert!(
        labels[0].starts_with("diroxide-gui, left panel, in "),
        "{labels:?}"
    );
    // The status line as the app says it: the folder and its counts, or a
    // message that stands (a folder the system cannot watch, in a
    // container).
    assert_eq!(labels[1], app.status_line(), "{labels:?}");
    assert!(!labels[1].is_empty());
    assert_eq!(
        diroxide_gui::window_title(&app, "diroxide"),
        "diroxide-gui - diroxide"
    );
}

#[test]
fn a_changed_line_reaches_the_tree() {
    let app = app_here();
    let mut harness =
        TestHarness::create(masonry::theme::default_property_set(), root_widget(&app));
    harness.edit_widget(STATUS_LINE, |mut label| {
        Label::set_text(&mut label, "Copied 3 items to Backup");
    });
    let _ = harness.redraw();
    let tree = walk(&harness);
    assert!(
        tree.iter()
            .any(|(role, name)| *role == Role::Label && name == "Copied 3 items to Backup"),
        "{tree:?}"
    );
    // The title line is the first text, the status line the second.
    let [(_, title), (_, status)] = lines(&app);
    assert_eq!(title, app.title_line());
    assert_eq!(status, app.status_line());
    harness.edit_widget(TITLE_LINE, |mut label| Label::set_text(&mut label, title));
}
