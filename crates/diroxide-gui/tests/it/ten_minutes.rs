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
//!
//! The alpha 4 walk (R30's commit 114, the fold lane) adds the owner's new
//! steps of R30 section 5.1 that need no helper program, and the lines
//! the lanes since VA named for the owner to hear: Say Details on a
//! picture; a JPEG named .png, the groups and the encoding chooser; Open
//! with; converting a document with carta; Checksum and Verify checksum;
//! the Watched folders list, a task's preview and its approval; the
//! picture view's hint; Change time; the review and the month folders;
//! Delete inside a zip. Its window runs helper programs through the tools
//! double, which finds none, and Open with lists two made-up programs.

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

    /// The same for a line the workers bring (a job's result): the walk
    /// settles until an announcement since `from` says `fact`, for at
    /// most ten seconds of the real clock, then hears it. A state, never
    /// a time, so a loaded machine only makes it slower.
    fn said_when_done(&mut self, rig: &mut Rig, from: usize, fact: &str) -> usize {
        rig.settle_until(|rig| {
            rig.said()[from..]
                .iter()
                .any(|m| within_cells(&m.text, fact))
        });
        self.said(rig, from, fact)
    }

    /// The focus's name once it starts with `start`, for a row the
    /// workers bring (a renamed file arrives with the folder's next
    /// listing): the walk settles until it does, then hears it.
    fn focus_when_listed(&mut self, rig: &mut Rig, start: &str) {
        rig.settle_until(|rig| rig.focus_node().1.starts_with(start));
        self.focus(rig, start);
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
    at = heard.said_when_done(&mut rig, at, "Copy finished: 3 done");
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
    heard.said_when_done(&mut rig, at, "Renamed report.pdf to report-2026.pdf");
    assert!(work.join("report-2026.pdf").exists());
    // The renamed row comes with the folder's next listing, from a
    // worker: CI's loaded Windows runner checked before it came.
    heard.focus_when_listed(&mut rig, "report-2026.pdf");
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
    heard.said_when_done(&mut rig, at, "Undid rename");
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

/// The alpha 4 walk's folder (R30 section 5.1, as far as a test can make
/// it with no helper program): a picture, a JPEG named .png, Polish text
/// in Windows-1250, a report and its Markdown, a download with its
/// checksum list, a zip, and a watched folder's task.
fn fixture_four(root: &Path) -> PathBuf {
    use crate::every_purpose::{beach_jpeg, one_pixel_png, stored_zip};
    let folder = root.join("work");
    fs::create_dir_all(folder.join("Backup")).unwrap();
    fs::create_dir_all(folder.join("Photos")).unwrap();
    fs::write(folder.join("Photos").join("crow.png"), one_pixel_png()).unwrap();
    fs::write(folder.join("Photos").join("raccoon.png"), one_pixel_png()).unwrap();
    fs::write(folder.join("photo.png"), one_pixel_png()).unwrap();
    fs::write(folder.join("liar.png"), beach_jpeg()).unwrap();
    fs::write(
        folder.join("cp1250.txt"),
        [
            90u8, 97, 191, 243, 179, 230, 32, 103, 234, 156, 108, 185, 32, 106, 97, 159, 241, 46,
            32, 75, 114, 243, 108, 32, 191, 121, 106, 101, 46, 10,
        ],
    )
    .unwrap();
    fs::write(folder.join("report.pdf"), vec![b'r'; 2100]).unwrap();
    fs::write(
        folder.join("report.md"),
        "# Report\n\nCrows remember faces.\n",
    )
    .unwrap();
    fs::write(folder.join("example-tool.zip"), b"abc").unwrap();
    fs::write(folder.join("example-notes.txt"), b"notes, edited").unwrap();
    // The SHA-256 of "abc", which every implementation agrees on.
    let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    fs::write(
        folder.join("SHA256SUMS"),
        format!(
            "{abc}  example-tool.zip\n{}  example-notes.txt\n{abc}  example-extra.txt\n",
            "0".repeat(64)
        ),
    )
    .unwrap();
    fs::write(
        folder.join("course.zip"),
        stored_zip(&[("lesson.txt", b"lesson one\n"), ("quiz.txt", b"quiz\n")]),
    )
    .unwrap();
    let home = root.join("notes");
    for part in ["in", "out", "processed", "failed"] {
        fs::create_dir_all(home.join(part)).unwrap();
    }
    fs::write(home.join("in").join("a.md"), "# A\n\nOne.\n").unwrap();
    fs::write(home.join("in").join("b.md"), "# B\n\nTwo.\n").unwrap();
    let tasks = root.join("config").join(diroxide_app::config::Paths::TASKS);
    fs::create_dir_all(&tasks).unwrap();
    let task = format!(
        "name = \"notes\"\nkind = \"convert\"\nrecipe = \"markdown-to-org\"\n\n[trigger]\nkind = \"folder\"\nhome = '{}'\nmatch = [\"*.md\"]\nsettle = \"2s\"\n",
        home.display()
    );
    fs::write(tasks.join("notes.toml"), task).unwrap();
    folder
}

#[test]
fn the_owners_alpha_four_steps_in_the_window() {
    use std::sync::Arc;

    use crate::doubles::{TwoPrograms, WindowTools};
    let dir = tempfile::Builder::new()
        .prefix("gui-it-")
        .tempdir()
        .unwrap();
    let work = fixture_four(dir.path());
    let backup = work.join("Backup");
    let (mut app, runtime) = crate::rig::app_with(&work, &backup, |_| {});
    let tools = Arc::new(WindowTools::finding(&[]));
    app.set_tool_runner(tools.clone());
    let paths =
        diroxide_app::config::Paths::new(dir.path().join("config"), dir.path().join("state"));
    let runtime = runtime
        .with_paths(paths)
        .with_components_offline()
        .with_tool_runner(tools)
        .with_open_with(Arc::new(TwoPrograms));
    let mut rig = Rig::new(app, runtime);
    let mut heard = Heard { lines: Vec::new() };
    let go = |rig: &mut Rig, name: &str| {
        assert!(rig.f.app_mut().focus_name(Side::Left, name), "{name}");
        rig.f.refresh(&mut crate::rig::HostOf(&mut rig.h));
    };
    // A list or question a command opens: its name heard, then the focus.
    let opened = |rig: &mut Rig, heard: &mut Heard, start: &str, focus: &str| {
        rig.settle_until(|r| dialog(r).is_some_and(|d| d.starts_with(start)));
        let name = dialog(rig).unwrap_or_default();
        assert!(
            within_cells(&name, start),
            "the dialog is {name:?}, not {start:?}"
        );
        heard
            .lines
            .push(first_cells(&name, BRAILLE_CELLS).to_owned());
        heard.focus(rig, focus);
    };
    // Step 1, a photo: Say Details speaks the summary; the focus stays.
    go(&mut rig, "photo.png");
    let mut at = rig.said().len();
    rig.run("file.say-details");
    heard.said_when_done(&mut rig, at, "PNG, 1 by 1, 8-bit gray, 70 bytes");
    heard.focus(&rig, "photo.png");
    // Step 4, file types: a JPEG named .png says so in its details.
    go(&mut rig, "liar.png");
    rig.press("alt+enter");
    opened(
        &mut rig,
        &mut heard,
        "Details of liar.png",
        "Type: JPEG image",
    );
    rig.focus_dialog_row("file.type-check");
    heard.focus(&rig, "Name says PNG; contents say JPEG");
    rig.press("escape");
    rig.run("select.groups");
    opened(
        &mut rig,
        &mut heard,
        "Groups: 8 built in",
        "Images: 2 files here",
    );
    rig.press("escape");
    // The viewer guesses Windows-1252 for Polish text; the owner picks
    // Central European (Windows-1250) and the text reads right.
    go(&mut rig, "cp1250.txt");
    at = rig.said().len();
    rig.press("f3");
    at = heard.said_when_done(&mut rig, at, "Encoding: Windows-1252 (not UTF-8)");
    rig.run("viewer.encoding");
    opened(&mut rig, &mut heard, "Encoding of cp1250.txt", "Automatic:");
    rig.focus_dialog_row("windows-1250");
    rig.press("enter");
    heard.said_when_done(&mut rig, at, "Encoding Windows-1250, chosen by you");
    heard.focus_when_listed(
        &mut rig,
        "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105}",
    );
    rig.press("escape");
    go(&mut rig, "report.pdf");
    rig.run("file.open-with");
    opened(
        &mut rig,
        &mut heard,
        "Open report.pdf with:",
        "Default program, Example Reader",
    );
    rig.press("escape");
    // Step 6, convert a document: carta's recipe, built in; the plan
    // starts on No; the result lands in the other panel's folder.
    go(&mut rig, "report.md");
    rig.run("file.convert");
    opened(
        &mut rig,
        &mut heard,
        "Convert report.md",
        "Markdown to LaTeX",
    );
    rig.press("enter");
    opened(&mut rig, &mut heard, "Convert 1 file to LaTeX?", "No");
    at = rig.said().len();
    rig.press("y");
    heard.said_when_done(&mut rig, at, "Report: 1 converted");
    assert!(backup.join("report.tex").exists());
    // Step 7, verify a download: the value's start, then the verdict
    // first; on the list itself the failures, then the signature line.
    go(&mut rig, "example-tool.zip");
    rig.run("file.checksum");
    opened(
        &mut rig,
        &mut heard,
        "SHA-256 of example-tool.zip, ba78 16bf",
        "ba78 16bf 8f01 cfea",
    );
    rig.press("escape");
    rig.run("file.verify-checksum");
    opened(
        &mut rig,
        &mut heard,
        "1 file matches SHA256SUMS",
        "List not signed: 1 of 1 match",
    );
    rig.press("escape");
    go(&mut rig, "SHA256SUMS");
    rig.run("file.verify-checksum");
    opened(
        &mut rig,
        &mut heard,
        "1 differs, 1 missing, 1 match",
        "Does not match: example-notes.txt",
    );
    rig.press("down");
    heard.focus(&rig, "Missing: example-extra.txt");
    rig.press("down");
    heard.focus(&rig, "List not signed: 1 of 3 match");
    rig.press("escape");
    // Step 10, watch a folder: the task's row says its name and state
    // first; its preview; turning it on asks, starting on No.
    rig.run("app.hot-folders");
    opened(
        &mut rig,
        &mut heard,
        "Watched folders: 1 task",
        "Notes, off",
    );
    rig.press("enter");
    opened(&mut rig, &mut heard, "Notes actions", "Turn on, asks first");
    rig.focus_dialog_row("preview");
    rig.press("enter");
    opened(
        &mut rig,
        &mut heard,
        "Notes: 2 would convert, 0 skipped",
        "Would convert a.md",
    );
    rig.press("escape");
    rig.run("app.hot-folders");
    rig.settle_until(|r| dialog(r).is_some());
    rig.press("enter");
    rig.settle_until(|r| dialog(r).is_some_and(|d| d.starts_with("Notes actions")));
    rig.press("enter");
    opened(&mut rig, &mut heard, "Turn on Notes?", "No");
    at = rig.said().len();
    rig.press("n");
    at = heard.said(&rig, at, "Nothing changed.");
    // The picture view (lanes G5b and AE): F3 on a photo says how to see
    // it; Enter would open the picture.
    go(&mut rig, "photo.png");
    rig.press("f3");
    heard.said_when_done(&mut rig, at, "Enter shows the picture.");
    rig.press("escape");
    // Change time (lane AP): a list of settings, its first row the plan.
    go(&mut rig, "report.md");
    rig.run("file.change-time");
    opened(
        &mut rig,
        &mut heard,
        "Change time",
        "Change the times of report.md",
    );
    rig.press("escape");
    // The photo helpers (lane AP): the review's first row and its end,
    // then the month folders' question, answered No.
    go(&mut rig, "Photos");
    rig.press("enter");
    rig.settle_until(|r| r.focus_node().1.starts_with("crow.png"));
    rig.run("photo.review");
    opened(
        &mut rig,
        &mut heard,
        "Review: 1 of 2, crow.png, picture",
        "Keep, K",
    );
    at = rig.said().len();
    rig.press("escape");
    at = heard.said(&rig, at, "Review ended at 1 of 2");
    rig.run("photo.month-folders");
    opened(
        &mut rig,
        &mut heard,
        "Move 2 pictures into 1 month folder?",
        "No",
    );
    rig.press("n");
    at = heard.said(&rig, at, "Nothing moved.");
    // Editing inside a zip (lane AW): Delete asks before the rewrite,
    // starting on No.
    rig.press("backspace");
    rig.settle_until(|r| r.f.app().active_panel().place().name() == Some("work"));
    go(&mut rig, "course.zip");
    rig.press("enter");
    heard.focus_when_listed(&mut rig, "lesson.txt");
    rig.press("delete");
    opened(
        &mut rig,
        &mut heard,
        "Remove lesson.txt from course.zip?",
        "No",
    );
    rig.press("n");
    heard.said(&rig, at, "Nothing changed.");
    assert!(work.join("course.zip").exists());
    // What the owner hears, each line inside 40 cells, meaning first.
    for line in &heard.lines {
        eprintln!("heard: {line}");
        assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
    }
}
