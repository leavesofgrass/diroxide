//! The Convert command and the conversion reports (alpha 4, lane AW, R30
//! commit 62): the recipes list, the plan's question starting on No, the
//! job through CL's engine, the report said and kept, and Undo. The
//! recipes used are carta's, built in and run in process, so no program
//! starts; a recipe whose tool the fake tools runner does not find is
//! listed last and unavailable. Every file is made in the test's own
//! folder; the trash is a test folder.

use std::fs;
use std::path::Path;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_config::Paths;

use crate::support::{Session, Setup, place, registry_with_trash, temp};

/// A folder with a Markdown file and an empty `out`, a session in it
/// with the state folder under the test's folder and a test trash.
fn session(dir: &Path) -> (Session, std::path::PathBuf) {
    let work = dir.join("work");
    fs::create_dir_all(work.join("out")).unwrap();
    fs::write(
        work.join("notes.md"),
        "# Notes\n\nA line of notes.\n\n- one\n- two\n",
    )
    .unwrap();
    let paths = Paths::new(dir.join("config"), dir.join("state"));
    let (registry, _trash) = registry_with_trash(&dir.join("bin"));
    let setup = Setup {
        paths: Some(paths),
        ..Setup::default()
    };
    let s = Session::open(place(&work), place(&work.join("out")), registry, setup);
    (s, work)
}

/// The open list's rows: id, label and whether each can be chosen.
fn rows(s: &Session) -> Vec<(String, String, bool)> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list
            .rows()
            .iter()
            .map(|r| (r.id.clone(), r.label.clone(), r.enabled))
            .collect(),
        other => panic!("{other:?}"),
    }
}

/// Moves the open list's focus to a row by its id.
fn choose(s: &mut Session, id: &str) {
    for _ in 0..60 {
        let focused = match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.id.clone()),
            _ => None,
        };
        if focused.as_deref() == Some(id) {
            return;
        }
        s.key("down");
    }
    panic!("{id} is not in the list");
}

#[test]
fn convert_lists_the_recipes_plans_on_no_runs_and_undo_trashes_the_result() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    s.focus("notes.md");
    s.command(ids::CONVERT);
    assert_eq!(s.dialog_title().as_deref(), Some("Convert notes.md"));
    let listed = rows(&s);
    // The ready recipes first, by name; pandoc's last, unavailable, the
    // name first.
    let first_missing = listed.iter().position(|(_, _, on)| !on).unwrap();
    assert!(listed[..first_missing].iter().all(|(_, _, on)| *on));
    assert!(listed[first_missing..].iter().all(|(_, _, on)| !on));
    assert!(
        listed.iter().any(|(id, label, on)| id == "markdown-to-word"
            && label == "Markdown to Word, needs pandoc"
            && !on),
        "{listed:?}"
    );
    assert!(
        listed
            .iter()
            .any(|(id, label, _)| id == "markdown-to-plain-text"
                && label == "Markdown to plain text"),
        "{listed:?}"
    );
    for (_, label, _) in &listed {
        assert!(label.chars().count() <= 40, "{label}");
    }
    choose(&mut s, "markdown-to-plain-text");
    let said = s.key("enter");
    let question = said.last().cloned().unwrap_or_default();
    assert!(
        question.starts_with("Convert 1 file to ")
            && question.ends_with("No is the default. y or n"),
        "{said:?}"
    );
    let listing = match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        listing.contains(&"notes.md to notes.txt".to_owned()),
        "{listing:?}"
    );
    assert!(
        listing.contains(&"Tool: carta, built in".to_owned()),
        "{listing:?}"
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Convert started: 1 file to ")),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.starts_with("Report: 1 converted")),
        "{said:?}"
    );
    let made = work.join("out").join("notes.txt");
    let text = fs::read_to_string(&made).unwrap();
    assert!(text.contains("A line of notes."), "{text}");
    // Undo trashes the file the job made.
    let said = s.key("ctrl+z");
    assert!(
        said.iter()
            .any(|l| l == "Undid conversion: 1 item is in the trash"),
        "{said:?}"
    );
    assert!(!made.exists());
}

#[test]
fn no_changes_nothing_and_the_reports_list_reads_a_kept_report() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    s.focus("notes.md");
    s.command(ids::CONVERT);
    choose(&mut s, "markdown-to-org");
    s.key("enter");
    assert_eq!(s.key("n"), ["Nothing changed."]);
    assert!(!work.join("out").join("notes.org").exists());
    assert_eq!(
        s.command(ids::CONVERT_REPORTS),
        ["No conversion reports yet"]
    );
    s.command(ids::CONVERT);
    choose(&mut s, "markdown-to-org");
    s.key("enter");
    s.key("y");
    assert!(work.join("out").join("notes.org").exists());
    s.command(ids::CONVERT_REPORTS);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Conversion reports: 1 kept")
    );
    let listed = rows(&s);
    assert_eq!(listed[0].1, "Report: 1 converted");
    s.key("enter");
    match s.dialog() {
        Some(Dialog::Read { title, lines }) => {
            assert_eq!(title, "Report: 1 converted");
            assert!(
                lines.iter().any(|l| l == "Made: notes.org from notes.md"),
                "{lines:?}"
            );
            assert!(
                lines.iter().any(|l| l.starts_with("Finished: ")),
                "{lines:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_no_recipe_takes_says_so() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    fs::write(work.join("blob.qqq"), "x").unwrap();
    s.key("ctrl+r");
    s.focus("blob.qqq");
    assert_eq!(s.command(ids::CONVERT), ["No recipe takes these files"]);
}

/// An app over two made-up folders whose `notes` type converts with a
/// recipe (R30 commit 64), its listing given by hand; nothing on disk.
fn typed_app() -> diroxide_app::App {
    use diroxide_app::{App, AppEvent, AppOptions};
    use diroxide_core::{Effect, Entry, EntryKind, Place};
    let filetypes = diroxide_config::FileTypesConfig::from_toml(
        r#"
[[type]]
id = "mynotes"
name = "Notes file"
short = "Notes"
extensions = ["notes"]

[[type.actions]]
name = "To Org"
slot = "convert"
recipe = "markdown-to-org"
"#,
    )
    .unwrap();
    let local = |p: &str| Place::parse_with(p, diroxide_core::place::PathStyle::Unix).unwrap();
    let mut options = AppOptions::new(local("/work"), local("/other")).in_utc();
    options.types = diroxide_filetype::TypeRegistry::builtin().with_config(&filetypes);
    options.startup.type_slots = diroxide_app::typing::TypeSlots::from_config(&filetypes);
    let mut app = App::new(options);
    for effect in app.start() {
        if let Effect::List { request, place } = effect {
            let entries = if place == local("/work") {
                ["day.notes", "plain.txt"]
                    .into_iter()
                    .map(|n| Entry::new(n, EntryKind::File).with_size(10))
                    .collect()
            } else {
                Vec::new()
            };
            app.handle(AppEvent::Listed {
                request,
                place,
                result: Ok(entries),
            });
        }
    }
    app.take_requests();
    app
}

#[test]
fn a_type_s_recipe_action_plans_at_once_and_actions_offer_convert() {
    use diroxide_app::conversions::ConvertAsk;
    use diroxide_app::request::Request;
    use diroxide_core::{Command, Side};
    let mut app = typed_app();
    assert!(app.focus_name(Side::Left, "day.notes"));
    app.dispatch(&Command::new(ids::CONVERT));
    let asked: Vec<Request> = app.take_requests();
    assert!(
        asked.iter().any(|r| matches!(
            r,
            Request::Convert(ask) if matches!(
                ask.as_ref(),
                ConvertAsk::Plan { recipe, .. } if recipe == "markdown-to-org"
            )
        )),
        "{asked:?}"
    );
    // A type with no convert action: Actions for this file lists Convert,
    // which opens the recipes list.
    assert!(app.focus_name(Side::Left, "plain.txt"));
    app.dispatch(&Command::new(ids::FILE_ACTIONS));
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no actions list");
    };
    let at = list
        .rows()
        .iter()
        .position(|r| r.label == "Convert")
        .expect("a Convert row");
    for _ in 0..at {
        app.dispatch(&Command::new(ids::LIST_DOWN));
    }
    app.dispatch(&Command::new(ids::ACCEPT));
    let asked: Vec<Request> = app.take_requests();
    assert!(
        asked.iter().any(|r| matches!(
            r,
            Request::Convert(ask) if matches!(ask.as_ref(), ConvertAsk::Recipes { .. })
        )),
        "{asked:?}"
    );
}
