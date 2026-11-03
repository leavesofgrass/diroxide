//! The handover in the window (R27's commit 25, decision 8): Shell here
//! opens the system's terminal in the folder, F4 starts the editor
//! directly, Run command runs its line in a terminal left open; the
//! window waits for none of them, and the app says what started in its
//! own words (lane A5's `ProgramStarted`: the window's launcher does not
//! wait). The launcher's starter records each plan here, so no test
//! starts a program. And the text clipboard both ways: a copied name
//! pasted into a field.

#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex, PoisonError};

use diroxide_core::Side;
use diroxide_gui::spawn::{Planned, Started, WindowLauncher};
use masonry::core::TextEvent;

use crate::rig::{Rig, app_on, fixture};

/// A window whose launcher records what it would start.
fn recording(dir: &std::path::Path) -> (Rig, Arc<Mutex<Vec<Planned>>>) {
    let (app, runtime) = app_on(dir, dir);
    let plans = Arc::new(Mutex::new(Vec::new()));
    let theirs = Arc::clone(&plans);
    // The terminal is the test's choice on Unix, whatever the machine has.
    let launcher = WindowLauncher::with_starter(Box::new(move |planned: &Planned| {
        theirs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(planned.clone());
        Ok(())
    }))
    .with_terminal("xterm");
    let runtime = runtime.with_launcher(Arc::new(launcher));
    let rig = Rig::new(app, runtime);
    (rig, plans)
}

#[test]
fn shell_here_opens_a_terminal_and_f4_starts_the_editor_directly() {
    let dir = fixture(&["notes.txt"], &[]);
    let (mut rig, plans) = recording(dir.path());
    let leaf = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    rig.run("file.shell-here");
    assert!(
        rig.spoken()
            .iter()
            .any(|s| s == &format!("Started a shell in {leaf}")),
        "{:?}",
        rig.spoken()
    );
    assert!(
        !rig.spoken()
            .iter()
            .any(|s| s.starts_with("Back in diroxide"))
    );
    {
        let plans = plans.lock().unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].started, Started::Terminal);
        assert_eq!(plans[0].folder, dir.path());
        if cfg!(windows) {
            assert!(plans[0].new_console, "a console of its own");
        }
        assert!(plans[0].env.iter().any(|(n, _)| n == "DIRO_ACTIVE"));
    }
    // F4: the editor on the file, directly.
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    rig.press("f4");
    assert!(
        rig.spoken()
            .iter()
            .any(|s| s == "Started the editor on notes.txt"),
        "{:?}",
        rig.spoken()
    );
    let plans = plans.lock().unwrap();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[1].started, Started::Program);
    assert!(
        plans[1]
            .args
            .iter()
            .any(|a| a.to_string_lossy().ends_with("notes.txt")),
        "{:?}",
        plans[1]
    );
    // The window was never held: the focus is still on the row.
    assert!(rig.focus_node().1.starts_with("notes.txt"));
}

#[test]
fn a_program_that_cannot_start_says_why_in_plain_words() {
    let dir = fixture(&["notes.txt"], &[]);
    let leaf = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let launcher = WindowLauncher::with_starter(Box::new(|_: &Planned| {
        Err("no terminal program was found".to_owned())
    }));
    let runtime = runtime.with_launcher(Arc::new(launcher));
    let mut rig = Rig::new(app, runtime);
    rig.run("file.shell-here");
    let said = format!(
        "Could not start a shell in {leaf}: no terminal program was found. Set the TERMINAL variable to a terminal program, such as xterm."
    );
    assert!(rig.spoken().contains(&said), "{:?}", rig.spoken());
    assert!(
        !rig.spoken()
            .iter()
            .any(|s| s.starts_with("Could not start Shell"))
    );
}

#[test]
fn the_text_clipboard_both_ways() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    // Out: the name onto the clipboard.
    rig.run("file.copy-name");
    let copied = rig.h.clipboard_contents();
    assert_eq!(copied, "notes.txt");
    // In: the platform's paste key gives the window the clipboard's text,
    // typed into the open field.
    rig.run("go.path");
    rig.press("ctrl+a");
    let _ = rig.h.process_text_event(TextEvent::ClipboardPaste(copied));
    rig.settle();
    let state = rig.h.access_tree().state();
    let field = state.focus_in_tree();
    let text = field.document_range().text();
    assert!(text.ends_with("notes.txt"), "{text}");
}
