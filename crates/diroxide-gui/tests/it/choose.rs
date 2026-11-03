//! Choose and the stack (R27's commit 18): a list dialog is one modal
//! list box named by its title, its help drawn as a line, letters jumping
//! to rows; only the top dialog of the app's stack is in the tree; Escape
//! goes back down the stack to the panel; a dialog's list takes clicks.

#![allow(clippy::unwrap_used)]

use masonry::accesskit::{Action, Role};

use crate::rig::{Rig, app_on, fixture};

#[test]
fn escape_twice_from_a_value_list_lands_on_the_panel() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.run("app.settings");
    let settings = rig.dialog_name().unwrap();
    assert!(settings.starts_with("Settings"), "{settings}");
    assert_eq!(rig.focus_node().0, Role::ListBoxOption);
    // A choice setting opens its values over the settings.
    rig.focus_dialog_row("layout");
    let setting = rig.focus_node().1;
    assert!(setting.starts_with("Layout"), "{setting}");
    rig.press("enter");
    let values = rig.dialog_name().unwrap();
    assert!(values.starts_with("Layout"), "{values}");
    assert_ne!(values, settings);
    // Only the top dialog is in the tree, the one beneath out of it.
    assert_eq!(rig.dialogs_in_tree(), 1);
    assert_eq!(rig.f.app().dialogs().len(), 2);
    let (role, value) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(
        value.contains("current"),
        "the current value first: {value}"
    );
    // Escape: the settings again, the focus on the setting it left.
    rig.press("escape");
    assert_eq!(rig.dialog_name().unwrap(), settings);
    assert_eq!(rig.dialogs_in_tree(), 1);
    assert!(rig.focus_node().1.starts_with("Layout"));
    // Escape: the panel, its row focused.
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("notes.txt"), "{row}");
}

#[test]
fn a_letter_jumps_and_the_list_says_its_help() {
    let dir = fixture(&["b.txt", "a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.run("view.sort-list");
    let name = rig.dialog_name().unwrap();
    assert!(name.starts_with("Sort"), "{name}");
    let (rows, _) = rig.dialog_rows();
    assert!(rows.len() > 2, "{rows:?}");
    // The list's help is drawn as a line in the dialog.
    let help = rig
        .dialog_walk()
        .into_iter()
        .any(|(role, text)| role == Role::Label && text.contains("Enter"));
    assert!(help, "{:?}", rig.dialog_walk());
    // A letter moves to the next row that starts with it.
    let before = rig.focus_node().1;
    rig.press("s");
    let after = rig.focus_node().1;
    assert!(after.starts_with('S') || after.starts_with('s'), "{after}");
    assert_ne!(before, after);
}

#[test]
fn a_click_focuses_a_row_and_a_double_click_chooses_it() {
    let dir = fixture(&["b.txt", "a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.run("view.sort-list");
    let (rows, focus) = rig.dialog_rows();
    // A click on another row moves the app's focus there.
    let target = (focus.unwrap_or(0) + 2) % rows.len();
    let name = {
        let walk = rig.dialog_walk();
        let options: Vec<String> = walk
            .into_iter()
            .filter(|(role, _)| *role == Role::ListBoxOption)
            .map(|(_, n)| n)
            .collect();
        options[target].clone()
    };
    // The list is inside the window, tall enough for its rows (its help
    // line above it takes only its own height).
    {
        let list = rig.h.get_widget(diroxide_gui::widgets::modal::DIALOG_LIST);
        let at = list.ctx().window_transform().translation();
        let size = list.ctx().border_box();
        assert!(
            at.y + size.height() <= f64::from(crate::rig::SIZE.1),
            "{at:?}"
        );
        assert!(size.height() >= 28.0 * 5.0, "{size:?}");
    }
    rig.click(Role::ListBoxOption, &name, 1);
    assert_eq!(rig.dialog_rows().1, Some(target));
    assert_eq!(rig.focus_node().1, name);
    // A screen reader's Focus on the first option moves there too.
    let first = {
        let walk = rig.dialog_walk();
        walk.into_iter()
            .find(|(role, _)| *role == Role::ListBoxOption)
            .unwrap()
            .1
    };
    rig.act(Action::Focus, Role::ListBoxOption, &first);
    assert_eq!(rig.dialog_rows().1, Some(0));
    // A double click chooses: the list closes and the sort is said.
    rig.click(Role::ListBoxOption, &name, 1);
    rig.click(Role::ListBoxOption, &name, 2);
    assert!(rig.dialog_name().is_none(), "{:?}", rig.spoken());
    assert!(
        rig.spoken().iter().any(|s| s.starts_with("Sorted")),
        "{:?}",
        rig.spoken()
    );
}
