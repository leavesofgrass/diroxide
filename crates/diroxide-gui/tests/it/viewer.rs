//! The viewer (R27's commit 22): a list box of the rows on screen in the
//! panels' place, text or hex, its options named by their text with the
//! line's own place and the file's line count; token classes drawn by the
//! theme's roles; the prompts as dialogs; Follow reading what is added; a
//! file of 100,000 lines opening with a screen of nodes, never a full
//! tree.

#![allow(clippy::unwrap_used)]

use std::io::Write;

use diroxide_core::Side;
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, app_with, fixture};

/// The viewer's list box: its name, size and options' names.
fn viewer_list(rig: &Rig) -> Option<(String, usize, Vec<String>)> {
    let state = rig.h.access_tree().state();
    let mut stack = vec![state.root()];
    while let Some(node) = stack.pop() {
        if node.is_hidden() {
            continue;
        }
        let name = crate::rig::name_of(&node);
        if node.role() == Role::ListBox && name.starts_with("Viewer, ") {
            let options = node.children().map(|o| crate::rig::name_of(&o)).collect();
            return Some((name, node.size_of_set().unwrap_or(0), options));
        }
        stack.extend(node.children());
    }
    None
}

#[test]
fn a_hundred_thousand_line_file_opens_with_a_screen_of_nodes() {
    let dir = fixture(&[], &[]);
    {
        let mut file = std::fs::File::create(dir.path().join("big.txt")).unwrap();
        for n in 1..=100_000 {
            writeln!(file, "line {n} of the big file").unwrap();
        }
    }
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "big.txt"));
    rig.refresh();
    rig.press("f3");
    assert!(rig.f.app().viewer().is_some(), "{:?}", rig.spoken());
    let (name, _, options) = viewer_list(&rig).unwrap();
    assert_eq!(name, "Viewer, big.txt");
    assert!(
        !options.is_empty() && options.len() < 60,
        "{} nodes",
        options.len()
    );
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.contains("line 1 of the big file"), "{row}");
    // The panels are out of the tree while the viewer shows.
    assert!(!rig.walk().iter().any(|(_, n)| n.starts_with("Left panel")));
    // End: the last line, once the file is counted, and the list's size
    // is the file's lines, still a screen of nodes.
    rig.press("end");
    assert!(
        rig.settle_until(|rig| rig.focus_node().1.contains("line 100000 of")),
        "{:?}",
        rig.focus_node()
    );
    let (_, size, options) = viewer_list(&rig).unwrap();
    assert_eq!(size, 100_000);
    assert!(options.len() < 60, "{} nodes", options.len());
    // Escape: the panels again, the focus on the file's row.
    rig.press("escape");
    assert!(rig.f.app().viewer().is_none());
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("big.txt"), "{row}");
}

#[test]
fn hex_rows_and_the_jump_prompt() {
    let dir = fixture(&[], &[]);
    let lines: String = (1..=600).map(|n| format!("row {n}\n")).collect();
    std::fs::write(dir.path().join("rows.txt"), lines).unwrap();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "rows.txt"));
    rig.refresh();
    rig.press("f3");
    // The jump prompt is a dialog; the cursor lands on the line.
    rig.run("viewer.jump");
    assert!(rig.dialog_name().is_some());
    rig.type_text("500");
    rig.press("enter");
    assert!(rig.dialog_name().is_none());
    assert!(
        rig.settle_until(|rig| rig.focus_node().1.contains("row 500")),
        "{:?}",
        rig.focus_node()
    );
    // Hex: the same list, its rows the bytes.
    rig.run("viewer.hex");
    let (name, _, options) = viewer_list(&rig).unwrap();
    assert_eq!(name, "Viewer, rows.txt, hex");
    assert!(options.iter().all(|o| !o.is_empty()), "{options:?}");
    assert_ne!(rig.focus_node().1, "row 500");
}

/// With the syntax set in the build (`diro`'s default; the tests' build
/// with --all-features).
#[cfg(feature = "syntax-set")]
#[test]
fn token_classes_are_drawn_by_the_themes_roles() {
    let dir = fixture(&[], &[]);
    std::fs::write(
        dir.path().join("main.rs"),
        "// A comment.\nfn main() {\n    let crow = \"caw\";\n}\n",
    )
    .unwrap();
    let (app, runtime) = app_with(dir.path(), dir.path(), |options| {
        options.settings.highlighting = diroxide_app::viewer::HighlightSetting::On;
    });
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "main.rs"));
    rig.refresh();
    rig.press("f3");
    assert!(rig.settle_until(|rig| {
        rig.f
            .app()
            .viewer()
            .is_some_and(diroxide_app::viewer::Viewer::is_highlighted)
    }));
    let view = diroxide_gui::model::viewer_view(rig.f.app_mut(), 80, 20).unwrap();
    let roles: Vec<_> = view
        .rows
        .iter()
        .flat_map(|r| r.runs.iter().map(|x| x.2))
        .collect();
    use diroxide_app::settings::PanelRole;
    assert!(roles.contains(&PanelRole::TokenKeyword), "{roles:?}");
    assert!(roles.contains(&PanelRole::TokenComment), "{roles:?}");
    assert!(roles.contains(&PanelRole::TokenString), "{roles:?}");
    // The text a screen reader gets is the line's, whatever its classes.
    assert!(
        view.rows
            .iter()
            .any(|r| r.name.contains("let crow = \"caw\";"))
    );
}

#[test]
fn follow_shows_what_is_added_to_the_file() {
    let dir = fixture(&[], &[]);
    let log = dir.path().join("build.log");
    std::fs::write(&log, "started\n").unwrap();
    // Follow looks at the progress interval in screen-reader mode.
    let (app, runtime) = app_with(dir.path(), dir.path(), |options| {
        options.settings.progress_interval_seconds = 1;
    });
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "build.log"));
    rig.refresh();
    rig.press("f3");
    rig.run("viewer.follow");
    {
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(file, "linked the crow").unwrap();
    }
    assert!(
        rig.settle_until(|rig| {
            viewer_list(rig)
                .is_some_and(|(_, _, rows)| rows.iter().any(|r| r.contains("linked the crow")))
        }),
        "{:?} {:?}",
        viewer_list(&rig),
        rig.spoken()
    );
}
