//! The window's tree as text, held against golden files (R27's commit 15
//! and spike S2): a 40-row panel and a dialog, each dumped as a screen
//! reader's tree walk finds them, with no display and no GPU. A change to
//! what a screen reader gets shows as a change to a file in review.
//!
//! The goldens are in `tests/it/goldens`. A missing golden is written and
//! the test passes, saying so; `DIROXIDE_GOLDEN_UPDATE=1` writes them all
//! again. The temporary folder's path and name are written as `<dir>`.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use diroxide_core::Side;
use masonry::accesskit::{Live, Role, Toggled};

use crate::panel_list::list_synthetic;
use crate::rig::{Rig, app_on, fixture, name_of};

/// The tree as lines: each node's role and name, indented by depth, with
/// the states a screen reader says (selected, unavailable, the focus,
/// the place in its list, a live region's politeness, a field's value
/// and caret). Text runs are left out: they are their label's text.
pub fn dump(rig: &Rig, dir: &Path) -> String {
    let state = rig.h.access_tree().state();
    let focused = {
        let f = state.focus_in_tree();
        f.active_descendant().unwrap_or(f).id()
    };
    // The temporary folder, its name and the folder it is in differ on
    // every run and every system.
    let path = dir.display().to_string();
    let leaf = dir.file_name().unwrap().to_string_lossy().into_owned();
    let parent = dir.parent().unwrap().display().to_string();
    // macOS's temporary folder is reached through a link, so a place the
    // app read back may start with /private.
    let (private_path, private_parent) = (format!("/private{path}"), format!("/private{parent}"));
    // The folder's separator differs by system too: written as a slash.
    let clean = |text: String| {
        text.replace(&private_path, "<dir>")
            .replace(&path, "<dir>")
            .replace(&private_parent, "<tmp>")
            .replace(&parent, "<tmp>")
            .replace(&leaf, "<dir>")
            .replace(std::path::MAIN_SEPARATOR, "/")
    };
    let mut lines = Vec::new();
    let mut stack = vec![(state.root(), 0usize)];
    while let Some((node, depth)) = stack.pop() {
        // A hidden part (the window behind a dialog) is not there for a
        // screen reader, nor anything in it.
        if node.is_hidden() {
            continue;
        }
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        for child in children {
            stack.push((child, depth + 1));
        }
        if node.role() == Role::TextRun {
            continue;
        }
        let mut name = clean(name_of(&node));
        let mut selected = node.is_selected() == Some(true);
        let mut unavailable = node.is_disabled();
        if node.role() == Role::ListBoxOption && diroxide_gui::model::states_in_name() {
            // On macOS an option keeps "selected" and "unavailable" in its
            // name, since VoiceOver reads options as static text with no
            // states (model::states_in_name). The dump writes them as the
            // other systems' states, so one golden holds everywhere; the
            // forty-row test asserts the macOS names apart.
            for (word, flag) in [
                (", selected", &mut selected),
                (", unavailable", &mut unavailable),
            ] {
                if let Some(at) = name.find(word) {
                    name.replace_range(at..at + word.len(), "");
                    *flag = true;
                }
            }
        }
        let mut line = format!("{}{:?} {:?}", "  ".repeat(depth), node.role(), name);
        if node.is_text_input() {
            line.push_str(&format!(
                " value={:?}",
                clean(node.value().unwrap_or_default())
            ));
            if let Some(caret) = node.text_selection_focus() {
                line.push_str(&format!(" caret={}", caret.to_global_usv_index()));
            }
        }
        match node.toggled() {
            Some(Toggled::True) => line.push_str(" on"),
            Some(Toggled::False) => line.push_str(" off"),
            Some(Toggled::Mixed) => line.push_str(" mixed"),
            None => {}
        }
        if selected {
            line.push_str(" selected");
        }
        if unavailable {
            line.push_str(" unavailable");
        }
        if let (Some(at), Some(size)) = (
            node.position_in_set(),
            node.parent().and_then(|p| p.size_of_set()),
        ) {
            line.push_str(&format!(" {} of {size}", at + 1));
        }
        match node.live() {
            Live::Polite => line.push_str(" live=polite"),
            Live::Assertive => line.push_str(" live=assertive"),
            Live::Off => {}
        }
        if node.id() == focused {
            line.push_str(" FOCUS");
        }
        lines.push(line);
    }
    lines.join("\n") + "\n"
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("it")
        .join("goldens")
        .join(format!("{name}.txt"))
}

/// Holds `text` against the golden `name`.
pub fn check(name: &str, text: &str) {
    let path = golden_path(name);
    let update = std::env::var_os("DIROXIDE_GOLDEN_UPDATE").is_some_and(|v| v == "1");
    match std::fs::read_to_string(&path) {
        Ok(golden) if !update => {
            let golden = golden.replace("\r\n", "\n");
            assert!(
                golden == text,
                "the tree differs from {}:\n--- golden\n{golden}\n--- now\n{text}",
                path.display()
            );
        }
        _ => {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            eprintln!("wrote the golden {}", path.display());
        }
    }
}

#[test]
fn spike_s2_a_forty_row_panel_dumps_to_its_golden() {
    let dir = fixture(&[], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    list_synthetic(&mut rig, dir.path(), 40);
    for _ in 0..3 {
        rig.press("down");
    }
    rig.press("space");
    let text = dump(&rig, dir.path());
    assert!(text.contains("ListBoxOption \"file-000004.txt"), "{text}");
    // The marked row as each system's screen reader gets it: a state on
    // Windows and Linux, words in the name on macOS (which the dump
    // writes as a state, so the golden is the same everywhere).
    let marked = {
        let state = rig.h.access_tree().state();
        let mut stack = vec![state.root()];
        let mut found = None;
        while let Some(node) = stack.pop() {
            if node.role() == Role::ListBoxOption && name_of(&node).starts_with("file-000004.txt") {
                found = Some((name_of(&node), node.is_selected()));
            }
            stack.extend(node.children());
        }
        found.unwrap()
    };
    if diroxide_gui::model::states_in_name() {
        assert!(
            marked.0.starts_with("file-000004.txt, selected, "),
            "{marked:?}"
        );
        assert_ne!(marked.1, Some(true), "{marked:?}");
    } else {
        assert!(!marked.0.contains("selected"), "{marked:?}");
        assert_eq!(marked.1, Some(true), "{marked:?}");
    }
    check("forty-rows", &text);
}

#[test]
fn a_confirmation_dumps_to_its_golden() {
    let dir = fixture(&["notes.txt"], &["Backup"]);
    let (app, runtime) = app_on(dir.path(), &dir.path().join("Backup"));
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    rig.press("f5");
    let text = dump(&rig, dir.path());
    assert!(text.contains("Button \"No\" FOCUS"), "{text}");
    check("copy-confirmation", &text);
}

#[test]
fn spike_s2_a_dialog_dumps_to_its_golden() {
    let dir = fixture(&[], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    list_synthetic(&mut rig, dir.path(), 5);
    assert_eq!(rig.active(), Side::Left);
    rig.press("f2");
    let text = dump(&rig, dir.path());
    assert!(
        text.contains("Dialog \"Rename file-000001.txt to\""),
        "{text}"
    );
    check("rename-dialog", &text);
}
