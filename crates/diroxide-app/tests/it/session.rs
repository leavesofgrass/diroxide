//! The scripted end-to-end session of the W1d brief, driven through the
//! app and its runtime without a terminal: open a fixture folder, move by
//! every key, mark three rows, copy them to the other panel, rename, move
//! one to the test trash, undo, quit, then start again with the same state
//! folder and find the cursor where it was.

use diroxide_config::Paths;
use diroxide_core::Side;

use crate::support::{Session, fixture, place, registry_with_trash, settings, temp};

#[test]
fn a_whole_session_and_the_cursor_restored_after_a_restart() {
    let dir = temp();
    let work = fixture(dir.path());
    let backup = work.join("Backup");
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let (registry, trash) = registry_with_trash(&dir.path().join("bin"));
    let mut s = Session::start(
        place(&work),
        place(&backup),
        registry,
        Some(&paths),
        settings(),
    );
    assert!(
        s.said[0].text.starts_with("diroxide alpha 1."),
        "{:?}",
        s.said
    );
    assert_eq!(
        s.names(Side::Left),
        [
            "Backup",
            "Photos",
            "budget.xlsx",
            "notes.txt",
            "raccoon \u{1f99d}.txt",
            "report.pdf",
            "R\u{e9}sum\u{e9}.txt",
            "\u{65e5}\u{672c}\u{8a9e}.txt",
        ]
    );

    // Move by every key.
    s.key("down");
    assert_eq!(s.focused(), "Photos");
    s.key("end");
    assert_eq!(s.focused(), "\u{65e5}\u{672c}\u{8a9e}.txt");
    s.key("up");
    assert_eq!(s.focused(), "R\u{e9}sum\u{e9}.txt");
    s.key("pageup");
    assert_eq!(s.focused(), "Backup");
    s.key("pagedown");
    assert_eq!(s.focused(), "\u{65e5}\u{672c}\u{8a9e}.txt");
    s.key("home");
    assert_eq!(s.focused(), "Backup");
    s.key("down");
    s.key("right");
    assert_eq!(s.names(Side::Left), ["crow.jpg"]);
    s.key("left");
    assert_eq!(s.focused(), "Photos");
    s.key("enter");
    s.key("backspace");
    assert_eq!(s.focused(), "Photos");
    s.key("alt+left");
    s.key("alt+right");
    assert_eq!(s.app.active_panel().place(), &place(&work));
    s.key("tab");
    assert_eq!(s.app.active_side(), Side::Right);
    s.key("tab");

    // The emoji and the Japanese name read cleanly, one row each, the
    // name first.
    for name in [
        "raccoon \u{1f99d}.txt",
        "\u{65e5}\u{672c}\u{8a9e}.txt",
        "R\u{e9}sum\u{e9}.txt",
    ] {
        s.focus(name);
        let row = s.app.active_panel().list().focused().unwrap().spoken();
        assert!(row.starts_with(&format!("{name}, text, ")), "{row}");
    }

    // Mark three rows.
    s.focus("budget.xlsx");
    assert_eq!(s.key("space"), ["Selected budget.xlsx. 1 selected."]);
    s.key("space");
    assert_eq!(
        s.key("space"),
        ["Selected raccoon \u{1f99d}.txt. 3 selected."]
    );
    assert_eq!(s.app.active_panel().list().selected_count(), 3);

    // Copy them to the other panel, after the question.
    s.key("f5");
    let question = s.dialog_title().unwrap();
    assert!(
        question.starts_with("Copy 3 items to ")
            && question.ends_with("? No is the default. y or n"),
        "{question}"
    );
    let said = s.key("y");
    assert!(
        said.contains(&"Copy finished: 3 done.".to_owned()),
        "{said:?}"
    );
    assert!(backup.join("budget.xlsx").exists());
    assert!(backup.join("raccoon \u{1f99d}.txt").exists());
    assert_eq!(s.app.active_panel().list().selected_count(), 0);

    // Rename: the cursor starts before the extension.
    s.focus("report.pdf");
    s.key("f2");
    assert_eq!(s.dialog_title().as_deref(), Some("Rename report.pdf to"));
    s.type_text("-2026");
    let said = s.key("enter");
    assert!(
        said.contains(&"Renamed report.pdf to report-2026.pdf".to_owned()),
        "{said:?}"
    );
    assert!(work.join("report-2026.pdf").exists());
    assert_eq!(s.focused(), "report-2026.pdf");

    // Move one to the test trash, then undo.
    s.focus("notes.txt");
    s.key("f8");
    let question = s.dialog_title().unwrap();
    assert!(
        question.starts_with("Move 1 item to the trash")
            && question.ends_with("? No is the default. y or n"),
        "{question}"
    );
    let said = s.key("y");
    assert!(
        said.contains(&"Moved 1 item to the trash. It can be restored.".to_owned()),
        "{said:?}"
    );
    assert!(!work.join("notes.txt").exists());
    assert_eq!(trash.count(), 1);
    let said = s.key("ctrl+z");
    assert!(
        said.contains(&"Undid trash: 1 item is back".to_owned()),
        "{said:?}"
    );
    assert!(work.join("notes.txt").exists());

    // Quit with the cursor on a row, and start again.
    s.focus("budget.xlsx");
    s.key("ctrl+q");
    assert!(s.app.wants_quit());
    assert!(paths.state_file(Paths::SESSION).exists());
    drop(s);

    let (registry, _trash) = registry_with_trash(&dir.path().join("bin"));
    let again = Session::start(
        place(&work),
        place(&backup),
        registry,
        Some(&paths),
        settings(),
    );
    assert_eq!(again.focused(), "budget.xlsx");
    assert!(
        !again
            .said
            .iter()
            .any(|m| m.text.starts_with("diroxide alpha 1."))
    );
    assert!(
        !again.app.undo_log().is_empty(),
        "the journal gives undo after a restart"
    );
}
