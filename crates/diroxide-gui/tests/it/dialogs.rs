//! The modal host, the app-driven text field and the palette (R27's
//! commit 12, spike S7): one dialog over the stack, named by its title,
//! the focus on its control and back on the panel's row when it closes;
//! a field whose tree states its characters and its caret; a secret that
//! never enters the tree.

#![allow(clippy::unwrap_used)]

use diroxide_app::{AppEvent, AskId, Question};
use diroxide_core::{SecretKind, SecretRequest, Side};
use masonry::accesskit::Role;

use crate::rig::{Rig, app_on, fixture, name_of};

/// The focused node's role, name, value, and caret (from the text
/// selection, in characters), as a screen reader's text interface reads
/// them.
fn focused_field(rig: &Rig) -> (Role, String, String, Option<usize>) {
    let state = rig.h.access_tree().state();
    let node = state.focus_in_tree();
    if !node.supports_text_ranges() {
        return (node.role(), name_of(&node), String::new(), None);
    }
    let caret = node
        .text_selection_focus()
        .map(|position| position.to_global_usv_index());
    let text = node.document_range().text();
    (node.role(), name_of(&node), text, caret)
}

/// Every name and value in the tree, for a canary search.
fn every_text(rig: &Rig) -> String {
    let mut all = String::new();
    let mut stack = vec![rig.h.access_tree().state().root()];
    while let Some(node) = stack.pop() {
        all.push_str(&node.label().unwrap_or_default());
        all.push('\n');
        all.push_str(&node.value().unwrap_or_default());
        all.push('\n');
        stack.extend(node.children());
    }
    all
}

#[test]
fn spike_s7_a_rename_field_states_its_characters_and_caret() {
    let dir = fixture(&["report.pdf", "notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "report.pdf"));
    rig.press("f2");
    // One dialog node named by the field's label; the window behind it is
    // hidden from screen readers.
    let tree = rig.walk();
    let dialogs: Vec<&String> = tree
        .iter()
        .filter(|(role, _)| *role == Role::Dialog)
        .map(|(_, name)| name)
        .collect();
    assert_eq!(dialogs, ["Rename report.pdf to"], "{tree:?}");
    let (role, name, text, caret) = focused_field(&rig);
    assert_eq!(role, Role::TextInput);
    assert_eq!(name, "Rename report.pdf to");
    assert_eq!(text, "report.pdf");
    // The app puts the caret before the extension.
    assert_eq!(caret, Some("report".len()));
    rig.press("left");
    assert_eq!(focused_field(&rig).3, Some(5));
    rig.press("right");
    rig.type_text("-2026");
    let (_, _, text, caret) = focused_field(&rig);
    assert_eq!(text, "report-2026.pdf");
    assert_eq!(caret, Some("report-2026".len()));
    rig.press("enter");
    assert!(
        dir.path().join("report-2026.pdf").exists(),
        "{:?} {:?}",
        rig.said(),
        rig.f.app().dialog().map(|(_, d)| d.title())
    );
    // The dialog is gone, and the focus is the panel's row again, once
    // the folder is listed again (a worker's result, which can come after
    // the runtime looks idle).
    assert!(!rig.walk().iter().any(|(role, _)| *role == Role::Dialog));
    rig.settle_until(|rig| rig.focus_node().1.starts_with("report-2026.pdf"));
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("report-2026.pdf"), "{row}");
}

#[test]
fn spike_s7_a_secret_never_enters_the_tree() {
    let dir = fixture(&["course.zip"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let question = Question::Secret {
        job: None,
        request: SecretRequest::new(SecretKind::ArchivePassword, "course.zip"),
    };
    let _ = rig.f.app_mut().handle(AppEvent::Asks {
        ask: AskId::new(1),
        question,
    });
    rig.f.refresh(&mut crate::rig::HostOf(&mut rig.h));
    rig.redraw();
    let (role, name, _, _) = focused_field(&rig);
    assert_eq!(role, Role::PasswordInput);
    assert_eq!(name, "Password for course.zip");
    rig.type_text("canary-7f3a");
    let (_, _, text, caret) = focused_field(&rig);
    assert_eq!(text, "*".repeat("canary-7f3a".len()));
    assert_eq!(caret, Some("canary-7f3a".len()));
    let all = every_text(&rig);
    assert!(!all.contains("canary"), "the secret reached the tree");
    rig.press("escape");
    assert!(!rig.walk().iter().any(|(role, _)| *role == Role::Dialog));
}

#[test]
fn the_palette_lists_its_candidates_and_runs_the_chosen_command() {
    let dir = fixture(&["b.txt", "a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.press("ctrl+shift+p");
    let (role, name, _, _) = focused_field(&rig);
    assert_eq!(role, Role::EditableComboBox, "a field with completion");
    assert!(!name.is_empty());
    rig.type_text("sort by size");
    let tree = rig.walk();
    let candidates: Vec<&String> = tree
        .iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect();
    assert!(
        candidates.iter().any(|c| c.starts_with("Sort by size")),
        "{tree:?}"
    );
    // The combo box names the list it controls while it is open.
    let state = rig.h.access_tree().state();
    let field = state.focus_in_tree();
    assert!(field.controls().any(|list| list.role() == Role::ListBox));
    rig.press("enter");
    assert!(
        rig.f.app().dialog().is_none(),
        "{:?}",
        rig.f.app().dialog().map(|(_, d)| d.title())
    );
    let said: Vec<&str> = rig.said().iter().map(|m| m.text.as_str()).collect();
    assert!(
        said.iter().any(|s| s.starts_with("Sorted by size")),
        "{said:?}"
    );
}
