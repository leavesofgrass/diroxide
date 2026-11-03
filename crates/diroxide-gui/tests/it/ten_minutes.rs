//! The owner's ten minutes in the GUI, through the harness (R27's commit
//! 15, section 4): the walk of the terminal's scripted session
//! (`crates/diroxide-cli/tests/it/screen.rs`, lines 298 to 435) in the
//! window's tree. Each step checks what a screen reader hears: the live
//! region's announcement, and the focus's name, each with its meaning in
//! the first 40 cells of a Braille display.
//!
//! The terminal's walk moves a file to a test trash and undoes it; this
//! crate may not depend on the provider crate that test trash needs (the
//! dependency table of `cargo xtask deps`), so here the trash question is
//! answered No and the undo takes back the rename (a contract request
//! asks for the test trash).

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use diroxide_core::Side;
use diroxide_core::format::{BRAILLE_CELLS, first_cells, within_cells};
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on};

/// The terminal walk's folder: eight entries, a Backup folder with one.
fn fixture(root: &Path) -> PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(folder.join("Backup")).unwrap();
    fs::create_dir_all(folder.join("Photos")).unwrap();
    fs::write(folder.join("report.pdf"), vec![b'r'; 2100]).unwrap();
    fs::write(folder.join("notes.txt"), b"notes").unwrap();
    fs::write(folder.join("budget.xlsx"), vec![b'b'; 300]).unwrap();
    fs::write(folder.join("R\u{e9}sum\u{e9}.txt"), b"cv").unwrap();
    fs::write(folder.join("\u{65e5}\u{672c}\u{8a9e}.txt"), b"ja").unwrap();
    fs::write(folder.join("raccoon \u{1f99d}.txt"), b"jimothy").unwrap();
    fs::write(folder.join("Photos").join("crow.jpg"), b"caw").unwrap();
    fs::write(folder.join("Backup").join("report.pdf"), b"old").unwrap();
    folder
}

/// What the walk heard, in order, for the report and the failures.
struct Heard {
    lines: Vec<String>,
}

impl Heard {
    /// One of the announcements since `from`, the last that says `fact`
    /// in its first 40 cells; the walk fails if none does.
    fn said(&mut self, rig: &Rig, from: usize, fact: &str) -> usize {
        let said: Vec<&str> = rig.said()[from..].iter().map(|m| m.text.as_str()).collect();
        let found = said.iter().rev().find(|text| within_cells(text, fact));
        let Some(text) = found else {
            panic!("\"{fact}\" is not in the first {BRAILLE_CELLS} cells of any of {said:?}");
        };
        self.lines.push(first_cells(text, BRAILLE_CELLS).to_owned());
        rig.said().len()
    }

    /// The focus's name, asserted to start with `start`.
    fn focus(&mut self, rig: &Rig, start: &str) {
        let (role, name) = rig.focus_node();
        assert!(
            name.starts_with(start),
            "the focus is {role:?} \"{name}\", not \"{start}\""
        );
        self.lines
            .push(first_cells(&name, BRAILLE_CELLS).to_owned());
    }
}

/// The dialog's name in the tree.
fn dialog(rig: &Rig) -> Option<String> {
    rig.walk()
        .into_iter()
        .find(|(role, _)| *role == Role::Dialog)
        .map(|(_, name)| name)
}

#[test]
fn the_owners_ten_minutes_in_the_window() {
    let dir = tempfile::Builder::new()
        .prefix("gui-it-")
        .tempdir()
        .unwrap();
    let work = fixture(dir.path());
    let backup = work.join("Backup");
    let (app, runtime) = app_on(&work, &backup);
    let mut rig = Rig::new(app, runtime);
    let mut heard = Heard { lines: Vec::new() };
    // 1. Start: the title line says the place first; the focus is the
    // first row, name first, its position the list's own.
    let tree = rig.walk();
    assert!(
        tree.iter()
            .any(|(role, name)| *role == Role::Label && within_cells(name, "work, left panel, in ")),
        "{tree:?}"
    );
    heard.focus(&rig, "Backup");
    // 2. Move by every key: the focus is a row each time, never a
    // position in its name.
    for key in [
        "down", "down", "up", "end", "home", "pagedown", "pageup", "down",
    ] {
        rig.press(key);
        let (role, name) = rig.focus_node();
        assert_eq!(role, Role::ListBoxOption, "{key}");
        assert!(!name.contains(" of 8"), "{name}");
    }
    // Into a folder and back.
    assert!(rig.f.app_mut().focus_name(Side::Left, "Photos"));
    rig.f.refresh(&mut crate::rig::HostOf(&mut rig.h));
    for key in [
        "right",
        "left",
        "enter",
        "backspace",
        "alt+left",
        "alt+right",
    ] {
        rig.press(key);
        assert_eq!(rig.focus_node().0, Role::ListBoxOption, "{key}");
    }
    assert_eq!(rig.f.app().active_panel().place().name(), Some("work"));
    // 3. Tab to the other panel and back: one announcement each.
    let mut at = rig.said().len();
    rig.press("tab");
    at = heard.said(&rig, at, "Right panel: Backup, 1 item");
    heard.focus(&rig, "report.pdf");
    rig.press("tab");
    at = heard.said(&rig, at, "Left panel: work, 8 items");
    // 4. Mark three rows; Ctrl+D says the selection's size.
    assert!(rig.f.app_mut().focus_name(Side::Left, "budget.xlsx"));
    rig.f.refresh(&mut crate::rig::HostOf(&mut rig.h));
    rig.press("space");
    at = heard.said(&rig, at, "Selected budget.xlsx. 1 selected.");
    rig.press("space");
    rig.press("space");
    at = heard.said(&rig, at, "3 selected.");
    let marked = rig
        .walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .count();
    assert!(marked >= 3);
    rig.press("ctrl+d");
    at = heard.said(&rig, at, "3 selected, 312 bytes.");
    // 6. Copy to the other panel: a dialog named by its question, the
    // focus on No; y answers.
    rig.press("f5");
    let question = dialog(&rig).unwrap();
    assert!(
        within_cells(&question, "Copy 3 items to Backup, 312 bytes?"),
        "{question}"
    );
    heard
        .lines
        .push(first_cells(&question, BRAILLE_CELLS).to_owned());
    heard.focus(&rig, "No");
    rig.press("y");
    at = heard.said(&rig, at, "Copy finished: 3 done");
    assert!(dialog(&rig).is_none());
    for name in ["budget.xlsx", "notes.txt", "raccoon \u{1f99d}.txt"] {
        assert!(backup.join(name).exists(), "{name} was not copied");
    }
    // 7. Rename: the field says the name, the caret before the extension.
    assert!(rig.f.app_mut().focus_name(Side::Left, "report.pdf"));
    rig.f.refresh(&mut crate::rig::HostOf(&mut rig.h));
    rig.press("f2");
    let field = dialog(&rig).unwrap();
    assert_eq!(field, "Rename report.pdf to");
    heard.lines.push(field);
    rig.type_text("-2026");
    rig.press("enter");
    heard.said(&rig, at, "Renamed report.pdf to report-2026.pdf");
    assert!(work.join("report-2026.pdf").exists());
    heard.focus(&rig, "report-2026.pdf");
    // 8. The trash question, answered No here (see the module's note).
    rig.press("f8");
    let trash = dialog(&rig).unwrap();
    assert!(
        within_cells(&trash, "Move 1 item to the trash, 2.1 KB?"),
        "{trash}"
    );
    heard
        .lines
        .push(first_cells(&trash, BRAILLE_CELLS).to_owned());
    rig.press("n");
    assert!(dialog(&rig).is_none());
    assert!(work.join("report-2026.pdf").exists());
    at = rig.said().len();
    // Undo takes back the rename.
    rig.press("ctrl+z");
    heard.said(&rig, at, "Undid rename");
    assert!(work.join("report.pdf").exists());
    // 9. The palette: its field is a combo box named by its label.
    rig.press("ctrl+shift+p");
    let palette = dialog(&rig).unwrap();
    heard.lines.push(palette.clone());
    assert_eq!(rig.focus_node().0, Role::EditableComboBox);
    rig.press("escape");
    assert!(dialog(&rig).is_none());
    // 11. Quit: nothing runs, so it does not ask.
    rig.press("ctrl+q");
    assert!(rig.f.wants_quit());
    // What the owner hears, each line inside 40 cells, meaning first.
    for line in &heard.lines {
        eprintln!("heard: {line}");
        assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
    }
}
