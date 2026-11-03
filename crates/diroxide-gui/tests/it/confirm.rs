//! Confirm, Question and Read (R27's commit 17): a confirmation is two
//! real buttons, No focused, with its listing as a list to read and its
//! count; y, n, Enter and Escape answer as in the terminal, the buttons
//! press as their keys, the settle time holds, and the focus returns to
//! the panel's row. A question reads its situation and starts on the safe
//! choice; a text to read moves line by line. The dialog's name is said
//! once: by the dialog as the focus moves into it, not again by the live
//! region (the settled double).

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use diroxide_core::clock::{FixedClock, UtcOffset};
use diroxide_core::{Command, CommandId, Side, Timestamp};
use masonry::accesskit::{Action, Role};

use crate::rig::{Rig, app_on, app_with, fixture};

/// A folder with two files and a Backup folder, the left panel on it and
/// the right on Backup, the focus on notes.txt.
fn copy_rig() -> (tempfile::TempDir, Rig) {
    let dir = fixture(&["notes.txt", "report.pdf"], &["Backup"]);
    let backup = dir.path().join("Backup");
    let (app, runtime) = app_on(dir.path(), &backup);
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    (dir, rig)
}

#[test]
fn a_confirmation_is_no_and_yes_on_no_with_its_listing_and_its_count() {
    let (dir, mut rig) = copy_rig();
    rig.press("f5");
    let name = rig.dialog_name().unwrap();
    assert!(name.starts_with("Copy 1 item to Backup"), "{name}");
    assert!(name.ends_with("No is the default. y or n"), "{name}");
    // Two real buttons, No first, the focus on No with its key.
    assert_eq!(
        rig.dialog_buttons(),
        [
            (Role::Button, "No".to_owned()),
            (Role::Button, "Yes".to_owned())
        ]
    );
    assert_eq!(
        rig.focus_key(),
        (Role::Button, "No".to_owned(), Some("N".to_owned()))
    );
    // The dialog said its name as the focus moved into it; the live
    // region did not say the question again.
    assert!(
        rig.said().iter().any(|m| m.text == name),
        "the app said the question"
    );
    assert!(
        !rig.spoken()
            .iter()
            .any(|s| s.contains("Copy 1 item to Backup")),
        "{:?}",
        rig.spoken()
    );
    // Tab: Yes, then the listing, then No again.
    rig.press("tab");
    assert_eq!(
        rig.focus_key(),
        (Role::Button, "Yes".to_owned(), Some("Y".to_owned()))
    );
    rig.press("tab");
    let (role, first) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    let (list, size) = rig.focus_list().unwrap();
    assert!(list.starts_with("Listing, "), "{list}");
    assert!(
        list.ends_with(&format!("{size} lines")) || size == 1,
        "{list}"
    );
    assert!(size >= 1);
    if size > 1 {
        rig.press("down");
        assert_ne!(rig.focus_node().1, first, "Down reads the next line");
        rig.press("home");
    }
    assert_eq!(rig.focus_node().1, first);
    rig.press("shift+tab");
    assert_eq!(rig.focus_key().1, "Yes");
    rig.press("tab");
    rig.press("tab");
    assert_eq!(rig.focus_key().1, "No");
    // Enter on No answers no: nothing copied, the focus back on the row.
    rig.press("enter");
    assert!(rig.dialog_name().is_none());
    assert!(!dir.path().join("Backup").join("notes.txt").exists());
    assert!(rig.spoken().iter().any(|s| s.contains("Nothing changed.")));
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("notes.txt"), "{row}");
}

#[test]
fn yes_answers_by_its_button_by_space_and_by_y() {
    // Enter on Yes.
    let (dir, mut rig) = copy_rig();
    rig.press("f5");
    rig.press("right");
    assert_eq!(rig.focus_key().1, "Yes");
    rig.press("enter");
    assert!(rig.dialog_name().is_none());
    assert!(
        rig.settle_until(|_| dir.path().join("Backup").join("notes.txt").exists()),
        "{:?}",
        rig.spoken()
    );
    // Space on Yes, for report.pdf.
    assert!(rig.f.app_mut().focus_name(Side::Left, "report.pdf"));
    rig.refresh();
    rig.press("f5");
    rig.press("tab");
    rig.press("space");
    assert!(rig.dialog_name().is_none());
    assert!(rig.settle_until(|_| dir.path().join("Backup").join("report.pdf").exists()));
    // y with the focus on No, in a fresh folder.
    let (dir, mut rig) = copy_rig();
    rig.press("f5");
    assert_eq!(rig.focus_key().1, "No");
    rig.press("y");
    assert!(rig.settle_until(|_| dir.path().join("Backup").join("notes.txt").exists()));
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption, "the focus is back on the panel");
    assert!(row.starts_with("notes.txt"), "{row}");
}

#[test]
fn n_escape_and_a_click_on_no_answer_no() {
    let (dir, mut rig) = copy_rig();
    for key in ["n", "escape"] {
        rig.press("f5");
        assert!(rig.dialog_name().is_some(), "{key}");
        rig.press(key);
        assert!(rig.dialog_name().is_none(), "{key}");
    }
    // A click on No, and a screen reader's Click on it.
    rig.press("f5");
    rig.click(Role::Button, "No", 1);
    assert!(rig.dialog_name().is_none(), "a click on No");
    rig.press("f5");
    rig.act(Action::Click, Role::Button, "No");
    assert!(rig.dialog_name().is_none(), "a screen reader's click on No");
    assert!(!dir.path().join("Backup").join("notes.txt").exists());
}

#[test]
fn a_click_on_yes_answers_and_a_focus_on_it_moves_the_focus() {
    let (dir, mut rig) = copy_rig();
    rig.press("f5");
    // A screen reader's Focus on Yes moves the window's focus there and
    // answers nothing.
    rig.act(Action::Focus, Role::Button, "Yes");
    assert!(rig.dialog_name().is_some());
    assert_eq!(rig.focus_key().1, "Yes");
    rig.click(Role::Button, "Yes", 1);
    assert!(rig.dialog_name().is_none());
    assert!(rig.settle_until(|_| dir.path().join("Backup").join("notes.txt").exists()));
}

#[test]
fn a_held_key_cannot_answer_inside_the_settle_time() {
    let dir = fixture(&["notes.txt"], &["Backup"]);
    let backup = dir.path().join("Backup");
    let clock = Arc::new(FixedClock::new(
        Timestamp::from_unix(1_800_000_000, 0),
        UtcOffset::from_seconds(0),
    ));
    let theirs = Arc::clone(&clock);
    let (app, runtime) = app_with(dir.path(), &backup, move |options| {
        options.settings.dialog_settle_ms = 250;
        options.clock = theirs;
    });
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    rig.press("f5");
    assert!(rig.dialog_name().is_some());
    // The clock stands still: every key comes inside the settle time.
    rig.press("y");
    rig.press_repeat("y");
    rig.press("right");
    rig.press("enter");
    rig.press("space");
    assert!(
        rig.dialog_name().is_some(),
        "a key inside the settle time answered"
    );
    assert!(!backup.join("notes.txt").exists());
    // A held F5 does not open a second question either.
    rig.press_repeat("f5");
    assert_eq!(rig.f.app().dialogs().len(), 1);
    // After the settle time, Yes answers.
    clock.advance(1);
    rig.press("enter");
    assert!(rig.dialog_name().is_none());
    assert!(rig.settle_until(|_| backup.join("notes.txt").exists()));
}

#[test]
fn a_question_reads_its_situation_and_starts_on_the_safe_choice() {
    let dir = fixture(&["report.pdf"], &["Backup"]);
    let backup = dir.path().join("Backup");
    std::fs::write(backup.join("report.pdf"), b"older and longer").unwrap();
    let (app, runtime) = app_on(dir.path(), &backup);
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "report.pdf"));
    rig.refresh();
    rig.press("f5");
    rig.press("y");
    // The name taken: a question named by its first line, the situation
    // as its description, the focus on the choice that changes nothing.
    assert!(rig.settle_until(|rig| rig.dialog_name().is_some()));
    let name = rig.dialog_name().unwrap();
    assert!(name.starts_with("Name taken"), "{name}");
    let (role, choice) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(choice.starts_with("Skip"), "{choice}");
    assert!(
        !rig.spoken().iter().any(|s| s.starts_with(&name)),
        "{:?}",
        rig.spoken()
    );
    // Escape answers it with the safe choice: the old file stays.
    rig.press("escape");
    assert!(rig.settle_until(|rig| rig.dialog_name().is_none()));
    assert_eq!(
        std::fs::read(backup.join("report.pdf")).unwrap(),
        b"older and longer"
    );
    let _ = rig.dialog_description();
}

#[test]
fn a_text_to_read_moves_line_by_line_and_escape_goes_back() {
    let dir = fixture(&[], &[]);
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "one\nTWO\nthree\nFOUR\n").unwrap();
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "a.txt"));
    rig.refresh();
    rig.press("space");
    rig.press("space");
    let now = rig.now;
    rig.f.command(
        &mut crate::rig::HostOf(&mut rig.h),
        Command::new(CommandId::from_static("compare.files")),
        now,
    );
    assert!(rig.settle_until(|rig| rig.dialog_name().is_some()));
    // The changes, then Enter: one change as a text to read.
    rig.press("enter");
    let name = rig.dialog_name().unwrap();
    assert!(name.starts_with("Change 1 of"), "{name}");
    let (role, first) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    let (_, size) = rig.focus_list().unwrap();
    assert!(size >= 2, "{size}");
    rig.press("down");
    let (_, second) = rig.focus_node();
    assert_ne!(first, second);
    rig.press("end");
    rig.press("home");
    assert_eq!(rig.focus_node().1, first);
    // Escape: the changes again, then the panel.
    rig.press("escape");
    assert!(rig.dialog_name().is_some());
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
    assert_eq!(rig.focus_node().0, Role::ListBoxOption);
}
