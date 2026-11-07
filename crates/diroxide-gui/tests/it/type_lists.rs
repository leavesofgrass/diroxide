//! The window's side of TA's contract request 2 (R30 commits 29 to 32,
//! 41 to 44): the commands of alpha 4's lists reach the window's menus
//! and palette, and each opens its list in the modal host, named, its
//! focus on a row, Escape back to the panel's row. The lists are the
//! app's (the Actions, Groups, Open with and Encoding lists, and VA's
//! Tools list); the window keeps no list of commands of its own.

#![allow(clippy::unwrap_used)]

use diroxide_core::Side;
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, fixture};

/// Every command the window's menu tree carries, depth first.
fn menu_commands(rig: &Rig) -> Vec<String> {
    fn walk(entries: &[diroxide_gui::menus::Entry], out: &mut Vec<String>) {
        for entry in entries {
            match entry {
                diroxide_gui::menus::Entry::Item { command, .. } => out.push(command.clone()),
                diroxide_gui::menus::Entry::Submenu { entries, .. } => walk(entries, out),
            }
        }
    }
    let mut out = Vec::new();
    for top in diroxide_gui::menus::tree(rig.f.app()) {
        walk(&top.entries, &mut out);
    }
    out
}

fn window() -> (Rig, tempfile::TempDir) {
    let dir = fixture(&["notes.txt", "report.pdf"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    (rig, dir)
}

#[test]
fn the_menus_carry_alpha_fours_list_commands() {
    let (mut rig, _dir) = window();
    let commands = menu_commands(&rig);
    for id in [
        "file.actions",
        "file.open-with",
        "select.groups",
        "app.tools",
        "file.preview",
    ] {
        assert!(commands.iter().any(|c| c == id), "{id} in {commands:?}");
    }
    // The viewer's Encoding command, while the viewer is open.
    rig.press("f3");
    assert!(rig.f.app().viewer().is_some());
    let commands = menu_commands(&rig);
    assert!(
        commands.iter().any(|c| c == "viewer.encoding"),
        "{commands:?}"
    );
}

/// Runs `id` and checks a named list opens with the focus on a row, then
/// Escape goes back to the panel's row.
fn opens_a_list(rig: &mut Rig, id: &str, named: &str) {
    rig.run(id);
    assert!(
        rig.settle_until(|rig| rig.dialog_name().is_some()),
        "{id}: {:?}",
        rig.spoken()
    );
    let name = rig.dialog_name().unwrap();
    assert!(name.starts_with(named), "{id}: {name}");
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption, "{id}: {row}");
    assert!(!row.is_empty());
    rig.press("escape");
    assert!(rig.dialog_name().is_none(), "{id}");
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("notes.txt"), "{id}: {row}");
}

/// The Tools list (`app.tools`) is only checked in the menus above: its
/// report reads each found helper's version by starting it, and the
/// window's tests start no real program (VA's rule; the app's tests open
/// it over a fake runner).
#[test]
fn the_type_lists_open_in_the_window() {
    let (mut rig, _dir) = window();
    opens_a_list(&mut rig, "file.actions", "Actions for notes.txt");
    opens_a_list(&mut rig, "select.groups", "Groups");
}

#[test]
fn the_palette_finds_the_groups_list_by_name() {
    let (mut rig, _dir) = window();
    rig.run("app.palette");
    assert!(rig.dialog_name().is_some());
    rig.type_text("Groups");
    rig.press("enter");
    assert!(
        rig.settle_until(|rig| rig
            .dialog_name()
            .is_some_and(|name| name.starts_with("Groups"))),
        "{:?} {:?}",
        rig.dialog_name(),
        rig.spoken()
    );
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
}

#[test]
fn the_encoding_list_opens_from_the_viewer() {
    let (mut rig, _dir) = window();
    rig.press("f3");
    assert!(rig.f.app().viewer().is_some());
    rig.run("viewer.encoding");
    let name = rig
        .dialog_name()
        .unwrap_or_else(|| panic!("{:?}", rig.spoken()));
    assert!(name.starts_with("Encoding of notes.txt"), "{name}");
    assert_eq!(rig.focus_node().0, Role::ListBoxOption);
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
    let (list, _) = rig.focus_list().unwrap();
    assert!(list.starts_with("Viewer, notes.txt"), "{list}");
}
