//! The custom columns through the app (alpha 4, lane MA, R30 commit 52):
//! a field's key in the columns setting reads its cells from the files,
//! the rows say "reading" until they come; a regex column from
//! `config.toml`; the owner's own fields; the ask-first sort and Fill
//! columns; nothing read on a place whose contents are not read.

use std::fs;
use std::path::Path;

use diroxide_app::commands::ids;
use diroxide_config::Settings;
use diroxide_core::{Command, Side};
use diroxide_vfs::Registry;

use crate::support::{Session, place, png_bytes, settings, temp};

fn work(root: &Path) -> std::path::PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(folder.join("Shots")).unwrap();
    fs::write(folder.join("photo.png"), png_bytes(4, 3)).unwrap();
    fs::write(folder.join("wide.png"), png_bytes(16, 2)).unwrap();
    fs::write(folder.join("notes.txt"), b"one\ntwo\nthree\n").unwrap();
    fs::write(folder.join("show.S02E07.txt"), b"x\n").unwrap();
    folder
}

fn with_columns(columns: &[&str]) -> Settings {
    let mut settings = settings();
    settings.columns = columns.iter().map(|c| (*c).to_owned()).collect();
    settings
        .regex_columns
        .insert("episode".to_owned(), r"S\d+E(\d+)".to_owned());
    settings
}

fn session(work: &Path, settings: Settings) -> Session {
    Session::start(
        place(work),
        place(work),
        Registry::with_local(),
        None,
        settings,
    )
}

/// Takes the host's own events, which come off the counted workers,
/// until `done` or about five seconds; returns what was said.
fn until(s: &mut Session, done: impl Fn(&Session) -> bool) -> Vec<String> {
    let mut said = Vec::new();
    for _ in 0..50 {
        if done(s) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        let events = s.runtime.poll();
        let effects: Vec<_> = events
            .into_iter()
            .flat_map(|event| s.app.handle(event))
            .collect();
        said.extend(s.run(effects));
    }
    said
}

/// The spoken row of a name in the active panel.
fn row(s: &Session, name: &str) -> String {
    s.app
        .active_panel()
        .list()
        .rows()
        .iter()
        .find(|r| r.id == name)
        .map(|r| r.spoken())
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn a_background_column_fills_near_the_focus_and_a_regex_column_at_once() {
    let dir = temp();
    let work = work(dir.path());
    let s = session(
        &work,
        with_columns(&[
            "type",
            "image.dimensions",
            "regex.episode",
            "custom.project",
        ]),
    );
    // The readers ran when the folder was listed: the rows near the focus
    // say their pictures' sizes, in the columns' order, after the type.
    let photo = row(&s, "photo.png");
    assert!(
        photo.starts_with("photo.png, PNG, dimensions 4 by 3"),
        "{photo}"
    );
    assert_eq!(row(&s, "wide.png"), "wide.png, PNG, dimensions 16 by 2");
    // A file no reader takes says nothing for the column; the regex
    // column says its first group.
    assert!(
        row(&s, "show.S02E07.txt").ends_with("episode 07"),
        "{}",
        row(&s, "show.S02E07.txt")
    );
    assert!(!row(&s, "notes.txt").contains("dimensions"));
    // A folder is never read.
    assert!(!row(&s, "Shots").contains("dimensions"));
    for name in ["photo.png", "show.S02E07.txt"] {
        let row = row(&s, name);
        assert!(diroxide_core::format::within_cells(&row, name), "{row}");
    }
}

#[test]
fn the_owners_fields_are_columns_and_on_demand_waits_for_fill_columns() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work, with_columns(&["custom.project", "text.lines"]));
    // On demand: nothing is read until asked, and the row says so.
    assert!(
        row(&s, "notes.txt").ends_with("lines not computed"),
        "{}",
        row(&s, "notes.txt")
    );
    s.focus("notes.txt");
    let said = s.run_command(Command::new(ids::LABEL_FIELD).with_arg(
        "field",
        diroxide_core::ArgValue::Text("project: Apollo".into()),
    ));
    assert_eq!(said.last().unwrap(), "Field project set on notes.txt");
    assert!(
        row(&s, "notes.txt").contains("project Apollo"),
        "{}",
        row(&s, "notes.txt")
    );
    let said = s.command(ids::FILL_COLUMNS);
    assert!(
        said.iter()
            .any(|l| l.starts_with("Reading ") && l.ends_with(" for the columns")),
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.starts_with("Columns read: ")),
        "{said:?}"
    );
    assert!(
        row(&s, "notes.txt").ends_with("lines 3"),
        "{}",
        row(&s, "notes.txt")
    );
    // A picture the text reader does not take: its cell is left out.
    assert!(
        !row(&s, "photo.png").contains("lines"),
        "{}",
        row(&s, "photo.png")
    );
    assert_eq!(
        s.command(ids::FILL_COLUMNS).last().unwrap(),
        "Every column here is read."
    );
}

#[test]
fn sorting_by_a_column_reads_first_then_says_the_order() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work, with_columns(&["text.lines"]));
    // The Sort list offers the column after the built-in keys.
    s.command(ids::SORT_LIST);
    let rows = match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => list.rows().to_vec(),
        other => panic!("no Sort list: {other:?}"),
    };
    let last = rows.last().unwrap();
    assert_eq!(
        (last.id.as_str(), last.label.as_str()),
        ("column:text.lines", "Lines")
    );
    s.key("end");
    let said = s.key("enter");
    // Few files wait: they are read without a question, then sorted.
    assert!(
        said.iter()
            .any(|l| l.starts_with("Reading ") && l.ends_with(" to sort by Lines")),
        "{said:?}"
    );
    assert_eq!(said.last().unwrap(), "Sorted by lines, A to Z");
    let names = s.names(Side::Left);
    let first_files: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| n.ends_with(".txt"))
        .collect();
    assert_eq!(first_files, ["show.S02E07.txt", "notes.txt"]);
}

#[test]
fn a_big_read_asks_first_and_no_keeps_the_order() {
    let dir = temp();
    let work = work(dir.path());
    for n in 0..230 {
        fs::write(work.join(format!("n{n:03}.txt")), b"a\n").unwrap();
    }
    let mut s = session(&work, with_columns(&["text.lines"]));
    s.command(ids::SORT_LIST);
    s.key("end");
    let said = s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Read 234 files to sort by Lines? Under a second. No is the default. y or n")
    );
    assert_eq!(said.len(), 1, "{said:?}");
    let said = s.key("n");
    assert_eq!(said.last().unwrap(), "Nothing read; the order stays.");
    assert_eq!(
        s.app.active_panel().sort().key,
        diroxide_core::SortKey::Name
    );
}

#[test]
fn a_column_key_in_the_settings_list_takes_effect_at_once() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work, settings());
    assert!(!row(&s, "photo.png").contains("dimensions"));
    s.command(ids::SETTINGS);
    for _ in 0..200 {
        let on_columns = matches!(
            s.dialog(),
            Some(diroxide_app::Dialog::Choose { list, .. })
                if list.focused().is_some_and(|r| r.id == "columns")
        );
        if on_columns {
            break;
        }
        s.key("down");
    }
    s.key("enter");
    for _ in 0..60 {
        s.key("backspace");
    }
    s.type_text("type, image.dimensions");
    let said = s.key("enter");
    assert_eq!(said[0], "Columns: type, image.dimensions", "{said:?}");
    s.key("escape");
    // Both panels take the new columns at once, their cells read.
    assert!(
        row(&s, "photo.png").contains("dimensions 4 by 3"),
        "{}",
        row(&s, "photo.png")
    );
    let other = s
        .app
        .panel(Side::Right)
        .list()
        .rows()
        .iter()
        .find(|r| r.id == "photo.png")
        .unwrap()
        .spoken();
    assert!(other.contains("dimensions"), "{other}");
}

/// R30's commit 53: a plugin's column on a panel, end to end through the
/// real host and the in-process test plugin answering `column/format` as
/// diro-git does: "pending" until a cell comes, a word for each file, a
/// folder's count, and a sort by it once every cell is in.
#[test]
fn a_plugins_column_fills_the_rows_and_sorts_when_every_cell_is_in() {
    use crate::plugins::{MANIFEST, TestLauncher, enable, world};
    let manifest =
        format!("{MANIFEST}\n[[column]]\nid = \"git\"\nheader = \"Git\"\nkind = \"text\"\n");
    let w = world(&manifest);
    enable(&w.paths);
    let here = place(&w.work);
    let mut s = Session::open(
        here.clone(),
        here,
        Registry::with_local(),
        crate::support::Setup {
            settings: with_columns(&["size", "line-counter.git"]),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(TestLauncher::with(&[])),
            ..crate::support::Setup::default()
        },
    );
    until(&mut s, |s| row(s, "report.pdf").ends_with("modified"));
    assert_eq!(
        row(&s, "report.pdf"),
        "report.pdf, 2.1 KB, git Staged, modified"
    );
    assert_eq!(row(&s, "Backup"), "Backup, git 3 changed inside");
    assert_eq!(
        row(&s, "budget.xlsx"),
        "budget.xlsx, 300 bytes, git Ignored"
    );
    // A cell the plugin did not answer yet says so, and is asked again.
    assert!(
        row(&s, "notes.txt").ends_with("git pending"),
        "{}",
        row(&s, "notes.txt")
    );
    s.command(ids::SORT_LIST);
    let rows = match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => list.rows().to_vec(),
        other => panic!("no Sort list: {other:?}"),
    };
    let last = rows.last().unwrap();
    assert_eq!(
        (last.id.as_str(), last.label.as_str()),
        ("column:line-counter.git", "Git")
    );
    s.key("end");
    let mut said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l == "Asking Git for 1 entry to sort by it"),
        "{said:?}"
    );
    said.extend(until(&mut s, |s| {
        s.said
            .last()
            .is_some_and(|m| m.text.starts_with("Sorted by"))
    }));
    assert_eq!(said.last().unwrap(), "Sorted by git, A to Z");
    assert!(
        row(&s, "notes.txt").ends_with("git Clean"),
        "{}",
        row(&s, "notes.txt")
    );
    let names = s.names(Side::Left);
    assert_eq!(
        names.first().map(String::as_str),
        Some("Backup"),
        "{names:?}"
    );
    for name in ["report.pdf", "budget.xlsx"] {
        let row = row(&s, name);
        assert!(diroxide_core::format::within_cells(&row, name), "{row}");
    }
}
