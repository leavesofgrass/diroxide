//! Secret (R27's commit 19): the masked field and its Show toggle. The
//! typed text never reaches the tree, the drawing's view or any dump
//! while it is hidden; Show, turned on by its key, a click or Space on
//! the toggle, shows it and says so; turned off, it is gone again.

#![allow(clippy::unwrap_used)]

use diroxide_app::{AppEvent, AskId, Question};
use diroxide_core::{SecretKind, SecretRequest};
use masonry::accesskit::{Role, Toggled};

use crate::goldens::dump;
use crate::rig::{HostOf, Rig, app_on, fixture};

const CANARY: &str = "canary-7f3a";

/// The window with a password asked for course.zip.
fn asked() -> (tempfile::TempDir, Rig) {
    let dir = fixture(&["course.zip"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let question = Question::Secret {
        job: None,
        request: SecretRequest::new(SecretKind::ArchivePassword, "course.zip"),
    };
    let effects = rig.f.app_mut().handle(AppEvent::Asks {
        ask: AskId::new(1),
        question,
    });
    assert!(!effects.is_empty());
    rig.f.refresh(&mut HostOf(&mut rig.h));
    rig.redraw();
    (dir, rig)
}

/// Every name, value and description in the tree.
fn every_text(rig: &Rig) -> String {
    let mut all = String::new();
    let mut stack = vec![rig.h.access_tree().state().root()];
    while let Some(node) = stack.pop() {
        for text in [
            node.label(),
            node.value(),
            node.data().description().map(str::to_owned),
        ]
        .into_iter()
        .flatten()
        {
            all.push_str(&text);
            all.push('\n');
        }
        stack.extend(node.children());
    }
    all
}

/// The Show toggle's state, from the tree.
fn show_state(rig: &Rig) -> Option<Toggled> {
    let state = rig.h.access_tree().state();
    let mut stack = vec![state.root()];
    while let Some(node) = stack.pop() {
        if node.role() == Role::CheckBox && !node.is_hidden() {
            return node.toggled();
        }
        stack.extend(node.children());
    }
    None
}

#[test]
fn a_planted_canary_is_in_no_dump_while_hidden() {
    let (dir, mut rig) = asked();
    let (role, name, _) = rig.focus_key();
    assert_eq!(role, Role::PasswordInput);
    assert_eq!(name, "Password for course.zip");
    assert_eq!(rig.dialog_name().unwrap(), "Password for course.zip");
    rig.type_text(CANARY);
    // Not in the tree, the dumps, the dialog's view, the app's dialog
    // as Debug shows it, or anything the live region said.
    assert!(!every_text(&rig).contains("canary"), "the tree");
    let dumped = dump(&rig, dir.path());
    assert!(!dumped.contains("canary"), "{dumped}");
    assert!(
        dumped.contains(&format!("value={:?}", "*".repeat(CANARY.len()))),
        "{dumped}"
    );
    let view = diroxide_gui::model::dialog_view(rig.f.app()).unwrap();
    assert!(!format!("{view:?}").contains("canary"), "the view");
    assert!(
        !format!("{:?}", rig.f.app().dialog()).contains("canary"),
        "the app"
    );
    assert!(
        !rig.spoken().join("\n").contains("canary"),
        "the live region"
    );
    // The dialog said its name once, by itself: the prompt line the app
    // said as it opened is not in the live region.
    assert!(
        !rig.spoken().iter().any(|s| s.starts_with("Password for")),
        "{:?}",
        rig.spoken()
    );
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
}

#[test]
fn show_turns_on_by_space_and_off_by_its_key_and_says_so() {
    let (_dir, mut rig) = asked();
    rig.type_text(CANARY);
    // The toggle: a check box after the field, off, with its key.
    assert_eq!(
        rig.dialog_buttons(),
        [(Role::CheckBox, "Show the password".to_owned())]
    );
    assert_eq!(show_state(&rig), Some(Toggled::False));
    rig.press("tab");
    assert_eq!(
        rig.focus_key(),
        (
            Role::CheckBox,
            "Show the password".to_owned(),
            Some("Alt+S".to_owned())
        )
    );
    rig.press("space");
    assert_eq!(show_state(&rig), Some(Toggled::True));
    assert!(
        rig.spoken()
            .iter()
            .any(|s| s.starts_with("Showing the password.")),
        "{:?}",
        rig.spoken()
    );
    // Shown: the field is a plain field with the text, read as typed.
    rig.press("shift+tab");
    let state = rig.h.access_tree().state();
    let field = state.focus_in_tree();
    assert_eq!(field.role(), Role::TextInput);
    assert_eq!(field.value().as_deref(), Some(CANARY));
    // Its key turns it off: the mask again, the text gone from the tree.
    rig.press("alt+s");
    assert_eq!(show_state(&rig), Some(Toggled::False));
    assert!(rig.spoken().iter().any(|s| s == "Hidden."));
    assert_eq!(rig.focus_key().0, Role::PasswordInput);
    assert!(!every_text(&rig).contains("canary"));
}

#[test]
fn typing_on_the_toggle_types_in_the_field_and_a_click_turns_it() {
    let (_dir, mut rig) = asked();
    rig.press("tab");
    assert_eq!(rig.focus_key().0, Role::CheckBox);
    rig.type_text("ab");
    assert_eq!(rig.focus_key().0, Role::PasswordInput, "the focus is back");
    let state = rig.h.access_tree().state();
    assert_eq!(state.focus_in_tree().value().as_deref(), Some("**"));
    rig.click(Role::CheckBox, "Show the password", 1);
    assert_eq!(show_state(&rig), Some(Toggled::True));
    rig.click(Role::CheckBox, "Show the password", 1);
    assert_eq!(show_state(&rig), Some(Toggled::False));
}

#[test]
fn a_secret_dumps_to_its_golden_with_no_secret_in_it() {
    let (dir, mut rig) = asked();
    rig.type_text(CANARY);
    let text = dump(&rig, dir.path());
    assert!(!text.contains("canary"));
    crate::goldens::check("secret-dialog", &text);
}
