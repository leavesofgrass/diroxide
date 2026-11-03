//! Completion, paste and the input method (R27's commit 20): a field with
//! completion is an editable combo box whose list it names while open;
//! Tab completes a path from the folder; a paste and an input method's
//! text type into the field; every field but a password's opens an input
//! method session; copied text goes to the system clipboard, and text
//! pasted where no field is open is refused in words.

#![allow(clippy::unwrap_used)]

use diroxide_app::{AppEvent, AskId, Question};
use diroxide_core::{SecretKind, SecretRequest, Side};
use masonry::accesskit::Role;
use masonry::core::{Ime, TextEvent};

use crate::rig::{HostOf, Rig, app_on, fixture};

/// The focused field's role and text, as a screen reader's text
/// interface reads it.
fn field(rig: &Rig) -> (Role, String) {
    let state = rig.h.access_tree().state();
    let node = state.focus_in_tree();
    let text = if node.supports_text_ranges() {
        node.document_range().text()
    } else {
        String::new()
    };
    (node.role(), text)
}

#[test]
fn tab_completes_a_path_in_a_fixture_folder() {
    let dir = fixture(&["notes.txt"], &["Photos", "Backup"]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.run("go.path");
    // The field starts with this folder's path; a name after it, Tab.
    let (role, start) = field(&rig);
    assert_eq!(role, Role::EditableComboBox, "a path field completes");
    rig.type_text("/Pho");
    rig.press("tab");
    assert!(
        rig.settle_until(|rig| {
            let text = field(rig).1;
            text.starts_with(&start) && text.trim_end_matches(['/', '\\']).ends_with("Photos")
        }),
        "{:?}",
        field(&rig)
    );
    rig.press("enter");
    assert!(rig.settle_until(|rig| rig.f.app().panel(Side::Left).place().name() == Some("Photos")));
}

#[test]
fn a_field_opens_an_input_method_session_and_a_password_does_not() {
    let dir = fixture(&["report.pdf"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(!rig.h.has_ime_session(), "no field, no session");
    rig.press("f2");
    assert!(rig.h.has_ime_session(), "the rename field");
    // What the input method commits is typed at the caret.
    rig.press("end");
    let _ = rig
        .h
        .process_text_event(TextEvent::Ime(Ime::Commit("\u{65e5}\u{672c}".to_owned())));
    rig.settle();
    assert_eq!(field(&rig).1, "report.pdf\u{65e5}\u{672c}");
    rig.press("escape");
    assert!(!rig.h.has_ime_session(), "the field is gone");
    // A password field opens none.
    let question = Question::Secret {
        job: None,
        request: SecretRequest::new(SecretKind::ArchivePassword, "report.pdf"),
    };
    let _ = rig.f.app_mut().handle(AppEvent::Asks {
        ask: AskId::new(1),
        question,
    });
    rig.f.refresh(&mut HostOf(&mut rig.h));
    rig.redraw();
    assert_eq!(field(&rig).0, Role::PasswordInput);
    assert!(!rig.h.has_ime_session(), "a password opens no session");
    rig.press("escape");
}

#[test]
fn a_paste_types_its_first_line_and_elsewhere_is_refused() {
    let dir = fixture(&["report.pdf"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.press("f2");
    rig.press("home");
    let _ = rig
        .h
        .process_text_event(TextEvent::ClipboardPaste("2026-\nsecond line".to_owned()));
    rig.settle();
    assert_eq!(field(&rig).1, "2026-report.pdf");
    rig.press("escape");
    // On the panel the paste is refused in words: its letters would run
    // as keys.
    let _ = rig
        .h
        .process_text_event(TextEvent::ClipboardPaste("dd".to_owned()));
    rig.settle();
    assert!(
        rig.spoken()
            .iter()
            .any(|s| s.starts_with("Text paste ignored here.")),
        "{:?}",
        rig.spoken()
    );
    assert!(dir.path().join("report.pdf").exists());
}

#[test]
fn copied_text_goes_to_the_system_clipboard() {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    rig.run("file.copy-name");
    // The harness keeps what the window put on the clipboard (the real
    // clipboard is never touched by a test).
    assert_eq!(rig.h.clipboard_contents(), "notes.txt");
    assert!(rig.spoken().iter().any(|s| s == "Name copied: notes.txt"));
    rig.run("file.copy-path");
    let path = rig.h.clipboard_contents();
    assert!(
        path.ends_with("notes.txt") && path.len() > "notes.txt".len(),
        "{path}"
    );
    assert_eq!(rig.f.clipboard_texts().len(), 2);
}
