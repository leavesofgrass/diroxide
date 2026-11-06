//! The host pieces R29 found shape-only, served and called (alpha 4, lane
//! HP, R30's commits 21 to 23): a plugin's form as a list of setting rows,
//! driven through the app with the in-process test plugin of `plugins`.

use diroxide_app::plugins::{Call, CallId, PluginEvent};
use diroxide_app::{AppEvent, Dialog};
use diroxide_core::{Command, CommandId};
use diroxide_plugins::ask::read_form;
use serde_json::{Value, json};

use crate::plugins::{Act, MANIFEST, TestLauncher, enable, session, world};
use crate::support::Session;

fn id(text: &str) -> CommandId {
    CommandId::new(text).unwrap()
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        _ => Vec::new(),
    }
}

/// The git plugin's commit form, every field kind, the focus on the
/// message.
fn commit_form() -> Value {
    json!({"title": "Commit", "summary": "2 files staged on main",
        "fields": [
            {"id": "message", "label": "Message", "kind": "multiline"},
            {"id": "author", "label": "Author", "kind": "text", "default": "Ada Example"},
            {"id": "branch", "label": "Branch", "kind": "choice", "choices": ["main", "dev"]},
            {"id": "sign", "label": "Sign-off", "kind": "checkbox", "default": false},
            {"id": "where", "label": "Repository", "kind": "path"},
            {"id": "key", "label": "Password", "kind": "secret"}
        ],
        "focus": "message", "default_button": "Commit"})
}

/// Every file's text under a folder, for a test that greps it.
fn walk_text(folder: &std::path::Path) -> String {
    let mut text = String::new();
    let Ok(read) = std::fs::read_dir(folder) else {
        return text;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            text.push_str(&walk_text(&path));
        } else if let Ok(bytes) = std::fs::read(&path) {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    text
}

/// R30's commit 21: a fake plugin's form answered field by field, each
/// row label first and value after, inside 40 cells; the secret reaches
/// the plugin and no message, effect, log or file.
#[test]
fn a_plugins_form_is_answered_field_by_field() {
    let canary = "canary-4b1e-not-a-key";
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/form", commit_form()))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    assert!(
        said.contains(
            &"Commit (Line counter). 2 files staged on main. Message: empty, 2 of 7".to_owned()
        ),
        "{said:?}"
    );
    assert_eq!(s.dialog_title().as_deref(), Some("Commit (Line counter)"));
    let first = rows(&s);
    assert_eq!(
        first,
        [
            "Commit: 2 files staged on main",
            "Message: empty",
            "Author: Ada Example",
            "Branch: main",
            "Sign-off: off",
            "Repository: empty",
            "Password: empty",
        ]
    );
    for row in &first {
        assert!(row.chars().count() <= 40, "{row}");
    }

    // The message: a list of its lines, each typed in the prompt.
    let said = s.key("enter");
    assert!(
        said.contains(&"Message (Line counter), Add a line, 1 of 1".to_owned()),
        "{said:?}"
    );
    s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Message, line 1 (Line counter)")
    );
    s.type_text("Fix the parser");
    let said = s.key("enter");
    assert!(
        said.contains(&"Message (Line counter), Line 1: Fix the parser, 1 of 2".to_owned()),
        "{said:?}"
    );
    s.key("down");
    s.key("enter");
    s.key("enter");
    s.key("down");
    s.key("enter");
    s.type_text("It read one byte too far.");
    s.key("enter");
    assert_eq!(
        rows(&s),
        [
            "Line 1: Fix the parser",
            "Line 2: empty",
            "Line 3: It read one byte too far.",
            "Add a line"
        ]
    );
    let said = s.key("escape");
    assert!(
        said.contains(
            &"Commit (Line counter), Message: Fix the parser, 3 lines, 2 of 7".to_owned()
        ),
        "{said:?}"
    );

    // The author: Escape in its prompt keeps it.
    s.key("down");
    s.key("enter");
    s.type_text(" Jr");
    let said = s.key("escape");
    assert!(
        said.contains(&"Commit (Line counter), Author: Ada Example, 3 of 7".to_owned()),
        "{said:?}"
    );

    // The branch, through a list of its choices.
    s.key("down");
    s.key("enter");
    assert_eq!(rows(&s), ["main", "dev"]);
    s.key("down");
    let said = s.key("enter");
    assert!(
        said.contains(&"Commit (Line counter), Branch: dev, 4 of 7".to_owned()),
        "{said:?}"
    );

    // Sign-off: Space and Enter turn it over in place.
    s.key("down");
    assert_eq!(s.key("space"), ["Sign-off: on"]);
    assert_eq!(s.key("enter"), ["Sign-off: off"]);
    s.key("space");

    // The repository: a path read against the panel's folder.
    s.key("down");
    s.key("enter");
    s.type_text("notes.txt");
    s.key("enter");
    let repository = rows(&s)[5].clone();
    assert!(
        repository.starts_with("Repository: ") && repository.ends_with("notes.txt"),
        "{repository}"
    );

    // The password, in the masked prompt.
    s.key("down");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Password (Line counter) (hidden)")),
        "{said:?}"
    );
    s.type_text(canary);
    let said = s.key("enter");
    assert!(
        said.contains(&"Commit (Line counter), Password: hidden, 7 of 7".to_owned()),
        "{said:?}"
    );

    // The default button answers every field by its id.
    s.key("home");
    let said = s.key("enter");
    let echo = said
        .iter()
        .find(|l| l.starts_with("Host said "))
        .unwrap_or_else(|| panic!("the plugin answers: {said:?}"));
    let answer: Value = serde_json::from_str(
        echo.trim_start_matches("Host said ")
            .trim_end_matches(". By Line counter."),
    )
    .unwrap();
    assert_eq!(
        answer["values"]["message"],
        "Fix the parser\n\nIt read one byte too far."
    );
    assert_eq!(answer["values"]["author"], "Ada Example");
    assert_eq!(answer["values"]["branch"], "dev");
    assert_eq!(answer["values"]["sign"], true);
    assert!(
        answer["values"]["where"]
            .as_str()
            .unwrap()
            .ends_with("notes.txt")
    );
    assert_eq!(answer["values"]["key"], canary);
    assert!(s.dialog().is_none(), "the focus is back on the panel");

    // The secret is in the plugin's own echo alone: no other message, no
    // effect, no log line and no file holds it.
    let mut others = s
        .said
        .iter()
        .filter(|m| !m.text.starts_with("Host said "))
        .map(|m| m.text.clone());
    assert!(!others.any(|t| t.contains(canary)), "never said");
    let effects = s
        .effects
        .iter()
        .map(|e| format!("{e:?}"))
        .filter(|e| !e.contains("Host said "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!effects.contains(canary), "in no effect");
    assert!(!walk_text(&w.paths.state_dir).contains(canary), "in no log");
    assert!(
        !walk_text(&w.paths.config_dir).contains(canary),
        "in no file"
    );
}

/// Escape in a form answers "cancelled" for the plugin; a form waits
/// behind a destructive confirmation, as a question does.
#[test]
fn escape_cancels_a_form_and_a_form_waits_its_turn() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/form", commit_form()))]),
    );
    s.run_command(Command::new(id("line-counter.ask")));
    s.key("down");
    s.key("down");
    s.key("enter");
    let said = s.key("escape");
    assert!(
        said.contains(&"Commit (Line counter), Branch: main, 4 of 7".to_owned()),
        "Escape in a field goes back to the form: {said:?}"
    );
    let said = s.key("escape");
    assert!(
        said.contains(&r#"Host said {"cancelled":true}. By Line counter."#.to_owned()),
        "{said:?}"
    );
    assert!(s.dialog().is_none());

    let mut s = session(&w, TestLauncher::with(&[]));
    s.focus("notes.txt");
    s.key("shift+delete");
    let title = s.dialog_title().unwrap();
    assert!(title.starts_with("Delete"), "{title}");
    let form = read_form(commit_form().as_object().unwrap(), "Line counter").unwrap();
    let effects = s.app.handle(AppEvent::Plugin(Box::new(PluginEvent::Call {
        call: CallId::new(99),
        plugin: "line-counter".into(),
        request: Call::Form(Box::new(form)),
    })));
    s.run(effects);
    assert_eq!(s.dialog_title().unwrap(), title, "never replaced");
    let said = s.key("n");
    assert!(said.contains(&"Nothing changed.".to_owned()), "{said:?}");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Commit (Line counter)"),
        "it opens when the confirmation closes"
    );
    s.key("escape");
}
