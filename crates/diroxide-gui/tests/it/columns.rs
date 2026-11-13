//! The columns as a header in words (alpha 4's request MA 6, R34's
//! commit 35): the rows say their cells after the name, and the panel's
//! list box says which columns those are as its description, read after
//! its name: "Columns: Name, Size, Modified".

#![allow(clippy::unwrap_used)]

use diroxide_core::Side;
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, fixture, name_of};

/// Each panel list box's name and description, in order.
fn lists(rig: &Rig) -> Vec<(String, Option<String>)> {
    let state = rig.h.access_tree().state();
    let mut stack = vec![state.root()];
    let mut found = Vec::new();
    while let Some(node) = stack.pop() {
        if node.role() == Role::ListBox {
            found.push((name_of(&node), node.data().description().map(str::to_owned)));
        }
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        stack.extend(children);
    }
    found
}

#[test]
fn each_panel_says_its_columns_after_its_name() {
    let dir = fixture(&["a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let rig = Rig::new(app, runtime);
    let expected = diroxide_gui::model::columns_words(rig.f.app(), Side::Left);
    // The default columns, 35 cells.
    assert_eq!(expected, "Columns: Name, Type, Size, Modified");
    assert_eq!(expected.chars().count(), 35);
    let found = lists(&rig);
    assert_eq!(found.len(), 2, "{found:?}");
    for (name, description) in &found {
        assert_eq!(description.as_deref(), Some(expected.as_str()), "{name}");
    }
    // The words name each column the panel's rows say, in their order.
    let panel = rig.f.app().panel(Side::Left);
    let labels: Vec<String> = panel
        .columns()
        .iter()
        .map(|spec| spec.kind().unwrap().label().to_owned())
        .collect();
    assert_eq!(expected, format!("Columns: Name, {}", labels.join(", ")));
}
