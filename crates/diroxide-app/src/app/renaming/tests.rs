//! Batch rename in the app, driven through the runtime on folders of its
//! own made in the temporary folder: the rules list, the preview, apply
//! with its recheck and one job, the editor door through a fake editor
//! that never starts a program, and the presets.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diroxide_config::Paths;
use diroxide_core::{FixedClock, Timestamp};

use super::*;
use crate::app::{AppEvent, AppOptions};
use crate::runtime::{Launcher, Program, Runtime};

/// An editor that never starts: it rewrites the file it is given with a
/// function, as the owner would in the editor, and keeps what it saw.
struct FakeEditor {
    edit: Box<dyn Fn(&str) -> String + Send + Sync>,
    seen: Mutex<Vec<String>>,
}

impl Launcher for FakeEditor {
    fn run(&self, program: &Program) -> Result<Option<i32>, String> {
        let file = program
            .args
            .last()
            .and_then(|arg| arg.to_str())
            .ok_or_else(|| "no file".to_owned())?
            .to_owned();
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(text.clone());
        }
        std::fs::write(&file, (self.edit)(&text)).map_err(|e| e.to_string())?;
        Ok(Some(0))
    }

    fn shell(&self) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
        ("sh".into(), vec!["-c".into()])
    }

    fn editor(&self) -> std::ffi::OsString {
        "fake-editor".into()
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    folder: PathBuf,
    state: PathBuf,
    config: PathBuf,
    app: App,
    runtime: Runtime,
    editor: Arc<FakeEditor>,
}

fn place(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

/// A folder of photos, its files made here, the app on it with every
/// entry marked.
fn fixture(names: &[&str], edit: impl Fn(&str) -> String + Send + Sync + 'static) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("photos");
    let state = dir.path().join("state");
    let config = dir.path().join("config");
    for made in [&folder, &state, &config] {
        std::fs::create_dir_all(made).unwrap();
    }
    for name in names {
        std::fs::write(folder.join(name), name.as_bytes()).unwrap();
    }
    let mut options = AppOptions::new(place(&folder), place(dir.path()));
    options.clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 11, 12, 16, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    options.settings.dialog_settle_ms = 0;
    let editor = Arc::new(FakeEditor {
        edit: Box::new(edit),
        seen: Mutex::new(Vec::new()),
    });
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local()
        .with_paths(Paths::new(config.clone(), state.clone()))
        .with_launcher(editor.clone());
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    let mut fixture = Fixture {
        _dir: dir,
        folder,
        state,
        config,
        app,
        runtime,
        editor,
    };
    fixture.run(ids::SELECT_ALL);
    fixture
}

impl Fixture {
    fn settle(&mut self, effects: Vec<Effect>) -> Vec<String> {
        self.runtime
            .run_until_idle(&mut self.app, effects)
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Announce(message) => Some(message.text),
                _ => None,
            })
            .collect()
    }

    fn run(&mut self, id: CommandId) -> Vec<String> {
        let effects = self.app.dispatch(&Command::new(id));
        self.settle(effects)
    }

    /// Enter on a row of the list on top, by its id.
    fn enter(&mut self, row: &str) -> Vec<String> {
        match self.app.dialogs.last_mut() {
            Some((_, Dialog::Choose { list, .. })) => {
                assert!(list.focus_id(row), "no row {row} in {}", list.title());
            }
            other => panic!("not a list: {other:?}"),
        }
        self.run(ids::ACCEPT)
    }

    /// Types a field's whole text and presses Enter.
    fn answer(&mut self, text: &str) -> Vec<String> {
        match self.app.dialogs.last_mut() {
            Some((_, Dialog::Edit { field, .. })) => field.set_text(text),
            other => panic!("not a field: {other:?}"),
        }
        self.run(ids::ACCEPT)
    }

    fn yes(&mut self) -> Vec<String> {
        let effects = self.app.handle(AppEvent::Key(Key::parse("y").unwrap()));
        self.settle(effects)
    }

    fn key(&mut self, text: &str) -> Vec<String> {
        // Through the key path, as the owner presses it (lane AS).
        let key = Key::parse(text).unwrap();
        let effects = self.app.handle(AppEvent::Key(key));
        self.settle(effects)
    }

    /// The rows of the list on top.
    fn rows(&self) -> Vec<String> {
        match self.app.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => {
                list.rows().iter().map(|row| row.name()).collect()
            }
            _ => Vec::new(),
        }
    }

    fn title(&self) -> String {
        self.app
            .dialogs
            .last()
            .map(|(_, dialog)| dialog.title())
            .unwrap_or_default()
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Adds a plain replacement through the prompts.
    fn add_replace(&mut self, find: &str, with: &str) -> Vec<String> {
        self.enter("add");
        self.enter("replace");
        self.answer(find);
        self.answer(with)
    }
}

fn fits(line: &str) {
    assert!(line.chars().count() <= 40, "{line}: over 40 cells");
}

const PHOTOS: [&str; 3] = ["IMG_0001.jpg", "IMG_0002.jpg", "IMG_0003.jpg"];

fn unchanged(text: &str) -> String {
    text.to_owned()
}

#[test]
fn the_rules_list_opens_on_the_marked_files_and_says_the_count_once() {
    let mut f = fixture(&PHOTOS, unchanged);
    let said = f.run(ids::BATCH_RENAME);
    assert_eq!(
        said[0],
        "Batch rename, 3 files, Reading the folder first, 1 of 10"
    );
    assert_eq!(said[1], "No file would change.");
    assert_eq!(said.len(), 2);
    fits(READING);
    assert_eq!(f.title(), "Batch rename, 3 files");
    // A rule made by its prompts: the count line said once, no row read.
    let said = f.add_replace("IMG_", "trip-");
    assert_eq!(said, ["3 of 3 files will change. No problems."]);
    fits(&said[0]);
    let rows = f.rows();
    assert_eq!(rows[0], "3 of 3 files will change. No problems.");
    assert_eq!(rows[1], "Rename now");
    assert_eq!(rows[2], "Replace IMG_ with trip-, in the name");
    assert_eq!(rows[3], "Add a rule");
    assert_eq!(rows[4], "Counter: from 1, step 1, 1 digit");
    for row in &rows[1..] {
        fits(row);
    }
}

#[test]
fn the_preview_is_named_by_the_count_line_and_space_skips_a_row() {
    let mut f = fixture(&PHOTOS, unchanged);
    f.run(ids::BATCH_RENAME);
    f.add_replace("IMG_", "trip-");
    let said = f.enter("count");
    assert!(
        said[0].starts_with("3 of 3 files will change. No problems., "),
        "{said:?}"
    );
    assert_eq!(
        f.rows(),
        [
            "IMG_0001.jpg becomes trip-0001.jpg",
            "IMG_0002.jpg becomes trip-0002.jpg",
            "IMG_0003.jpg becomes trip-0003.jpg",
        ]
    );
    fits(&f.rows()[0]);
    // Space skips the focused row and says it once; again takes it back.
    let said = f.key("space");
    assert_eq!(said, ["Skipped by you: IMG_0001.jpg"]);
    assert_eq!(f.title(), "2 of 3 files will change. No problems.");
    let said = f.key("space");
    assert_eq!(said, ["IMG_0001.jpg becomes trip-0001.jpg"]);
    // F2 edits the row's new name.
    f.key("f2");
    assert_eq!(f.title(), "New name for IMG_0001.jpg");
    let said = f.answer("first.jpg");
    // The engine's row words: the state and the old name in the first
    // 40 cells, the new name after them (45 cells in all).
    assert_eq!(said, ["Edited by you: IMG_0001.jpg becomes first.jpg"]);
    fits("Edited by you: IMG_0001.jpg becomes");
    // Say both names, from the row's actions.
    f.enter("0");
    let said = f.enter("both");
    assert_eq!(said, ["Old: IMG_0001.jpg. New: first.jpg"]);
}

#[test]
fn problems_come_first_and_rename_now_refuses_with_the_focus_on_one() {
    let mut f = fixture(&["a.txt", "b.txt", "c.txt"], unchanged);
    f.run(ids::BATCH_RENAME);
    // Every name becomes the same, so every row clashes.
    f.enter("add");
    f.enter("template");
    let said = f.enter("type");
    assert_eq!(said[0], "Template, empty");
    f.answer("same");
    let rows = f.rows();
    assert_eq!(rows[0], "3 problems. 0 of 3 files will change.");
    fits(&rows[0]);
    let said = f.enter("apply");
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(&said[0][..26], "Cannot rename: 3 problems.");
    // The preview is on top, problems first, the focus on the first.
    let rows = f.rows();
    assert!(
        rows.iter().all(|row| row.starts_with("Problem, ")),
        "{rows:?}"
    );
    let focused = match f.app.dialogs.last() {
        Some((_, Dialog::Choose { list, .. })) => list.focus_index(),
        _ => None,
    };
    assert_eq!(focused, Some(0));
    // Next problem walks them; Make names unique numbers the clashes.
    let said = f.run(ids::RENAME_NEXT_PROBLEM);
    assert!(said[0].starts_with("Problem, "), "{said:?}");
    let said = f.run(ids::RENAME_MAKE_UNIQUE);
    assert_eq!(
        said,
        ["Numbered the clashes. 3 of 3 files will change. No problems."]
    );
    assert!(
        f.rows().iter().any(|row| row.contains("same (2).txt")),
        "{:?}",
        f.rows()
    );
}

#[test]
fn yes_reads_the_folder_again_and_renames_in_one_job_with_one_undo() {
    let mut f = fixture(&PHOTOS, unchanged);
    f.run(ids::BATCH_RENAME);
    f.add_replace("IMG_", "trip-");
    let said = f.enter("apply");
    // The folder's own name, "No is the default" inside 40 cells (lane
    // AS).
    let question = "Rename 3 files in photos? No is the default. y or n";
    assert_eq!(said, [question]);
    assert_eq!(
        "Rename 3 files in photos? No is the default."
            .chars()
            .count(),
        44
    );
    assert_eq!("Rename 3 files in photos? No is".chars().count(), 31);
    let said = f.yes();
    assert!(
        said.contains(&"Checking the folder again.".to_owned()),
        "{said:?}"
    );
    assert_eq!(
        f.names(),
        ["trip-0001.jpg", "trip-0002.jpg", "trip-0003.jpg"]
    );
    assert!(f.app.dialogs.is_empty(), "the batch's dialogs close");
    // The batch's finish line (lane AS): "Renamed 50 files." is 17 cells.
    assert!(said.contains(&"Renamed 3 files.".to_owned()), "{said:?}");
    assert_eq!("Renamed 50 files.".chars().count(), 17);
    // One undo brings every name back.
    f.run(ids::UNDO);
    assert_eq!(f.names(), PHOTOS);
    // The last rules are kept for the next batch.
    assert!(f.state.join(crate::rename_batch::LAST_FILE).is_file());
    f.run(ids::SELECT_ALL);
    let said = f.run(ids::BATCH_RENAME);
    assert_eq!(
        said[1],
        "The last rules used: 3 of 3 files will change. No problems."
    );
}

#[test]
fn a_change_in_the_folder_before_yes_is_refused_in_words() {
    let mut f = fixture(&PHOTOS, unchanged);
    f.run(ids::BATCH_RENAME);
    f.add_replace("IMG_0001", "taken");
    f.enter("apply");
    std::fs::write(f.folder.join("taken.jpg"), b"new").unwrap();
    let said = f.yes();
    assert!(
        said.iter()
            .any(|line| line.starts_with("The folder changed since the preview: name taken")),
        "{said:?}"
    );
    assert!(f.folder.join("IMG_0001.jpg").is_file());
}

#[test]
fn the_editor_door_reads_keyed_lines_back_and_keeps_a_refused_buffer() {
    let mut f = fixture(&PHOTOS, |text| text.replacen("IMG_0002", "middle", 1));
    f.run(ids::BATCH_RENAME);
    let said = f.enter("editor");
    // The question is there before the editor takes the screen, and
    // stays after it hands the screen back.
    assert_eq!(said[0], "Done editing? No is the default. y or n");
    assert_eq!(said[1], "Back in diroxide, photos");
    let buffer = f.state.join(crate::rename_batch::buffer_name(1));
    assert!(buffer.is_file(), "the buffer is in the state folder");
    let seen = f.editor.seen.lock().unwrap().clone();
    assert_eq!(
        seen[0].lines().next(),
        Some("IMG_0001.jpg (was IMG_0001.jpg)")
    );
    let said = f.yes();
    assert_eq!(said, ["Names read. 1 of 3 files will change. No problems."]);
    assert!(!buffer.exists(), "the buffer goes once its names are read");
    f.enter("count");
    assert!(
        f.rows()
            .contains(&"Edited by you: IMG_0002.jpg becomes middle.jpg".to_owned()),
        "{:?}",
        f.rows()
    );
}

#[test]
fn a_lost_line_in_the_buffer_renames_nothing_and_keeps_the_edits() {
    let mut f = fixture(&PHOTOS, |text| {
        text.lines().take(2).map(|l| format!("{l}\n")).collect()
    });
    f.run(ids::BATCH_RENAME);
    f.enter("editor");
    let said = f.yes();
    assert_eq!(
        said,
        [
            "Names not read: 1 file has no line. Nothing renamed.",
            "Put back a line for each file; to keep a name, leave its line as it is. Your edits are kept.",
        ]
    );
    let buffer = f.state.join(crate::rename_batch::buffer_name(1));
    assert!(buffer.is_file(), "kept for another try");
    assert_eq!(f.names(), PHOTOS);
}

#[test]
fn presets_save_load_and_delete_through_their_file() {
    let mut f = fixture(&PHOTOS, unchanged);
    f.run(ids::BATCH_RENAME);
    f.add_replace("IMG_", "trip-");
    f.enter("presets");
    assert_eq!(f.title(), "Rename presets, 0 saved");
    f.enter("save");
    let said = f.answer("Trip");
    assert_eq!(said, ["Saved preset Trip."]);
    f.runtime.run_until_idle(&mut f.app, Vec::new());
    let file = f.config.join(crate::rename_batch::PRESETS_FILE);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("Trip"), "{text}");
    // Remove the rule, then load the preset back.
    f.run(ids::RENAME_PRESETS);
    assert_eq!(f.rows(), ["Save these rules as a preset", "Trip: 1 rule"]);
    f.enter("preset-0");
    let said = f.enter("load");
    assert_eq!(
        said,
        ["Loaded Trip: 3 of 3 files will change. No problems."]
    );
    // Delete asks first, starting on No.
    f.enter("presets");
    f.enter("preset-0");
    let said = f.enter("delete");
    assert_eq!(said, ["Delete preset Trip? No is the default. y or n"]);
    let said = f.yes();
    assert_eq!(said.last().unwrap(), "Deleted preset Trip.");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("Trip"), "{text}");
}

#[test]
fn rules_change_move_and_go_and_the_counter_is_typed() {
    let mut f = fixture(&PHOTOS, unchanged);
    f.run(ids::BATCH_RENAME);
    f.enter("add");
    f.enter("case");
    let said = f.enter("lower");
    assert_eq!(said, ["3 of 3 files will change. No problems."]);
    f.add_replace("IMG_", "trip-");
    assert_eq!(f.rows()[2], "Change case to lowercase, in the name");
    f.enter("rule-1");
    let said = f.enter("up");
    assert_eq!(said, ["Now rule 1. 3 of 3 files will change. No problems."]);
    assert_eq!(f.rows()[2], "Replace IMG_ with trip-, in the name");
    f.enter("rule-0");
    let said = f.enter("part");
    assert!(
        said[0].starts_with("Rule 1 edits the extension."),
        "{said:?}"
    );
    f.enter("rule-0");
    f.enter("remove");
    f.enter("counter");
    let said = f.answer("1, 1, 99");
    assert_eq!(said, ["Counter not read: digits from 1 to 20"]);
    f.answer("5, 5, 3");
    assert!(
        f.rows()
            .contains(&"Counter: from 5, step 5, 3 digits".to_owned())
    );
    // A template with a placeholder chosen from the list.
    f.enter("add");
    f.enter("template");
    let said = f.enter("{counter}");
    assert_eq!(said[0], "Template: {counter}");
    f.answer("pic-{counter}");
    f.enter("count");
    assert!(
        f.rows()
            .contains(&"IMG_0001.jpg becomes pic-005.jpg".to_owned()),
        "{:?}",
        f.rows()
    );
}

#[test]
fn escape_on_the_rules_list_renames_nothing() {
    let mut f = fixture(&PHOTOS, unchanged);
    f.run(ids::BATCH_RENAME);
    f.add_replace("IMG_", "trip-");
    let effects = f.app.dialog_escape();
    let said = f.settle(effects);
    assert_eq!(said, ["Nothing renamed."]);
    assert_eq!(f.names(), PHOTOS);
}

#[test]
fn rename_one_by_one_keeps_the_stepping_prompt() {
    let mut f = fixture(&PHOTOS, unchanged);
    let said = f.run(ids::RENAME_ONE_BY_ONE);
    assert!(said[0].starts_with("Rename IMG_0001.jpg to"), "{said:?}");
}

/// Question 132: F2 with several marked opens Batch rename; F2 on one
/// entry keeps the name prompt (lane AS).
#[test]
fn f2_on_several_marked_opens_batch_rename_and_on_one_the_prompt() {
    let mut f = fixture(&PHOTOS, unchanged);
    let effects = f.app.handle(AppEvent::Key(Key::parse("f2").unwrap()));
    let said = f.settle(effects);
    assert_eq!(
        said[0],
        "Batch rename, 3 files, Reading the folder first, 1 of 10"
    );
    assert_eq!(f.title(), "Batch rename, 3 files");
    let effects = f.app.dialog_escape();
    f.settle(effects);
    f.run(ids::SELECT_NONE);
    let effects = f.app.handle(AppEvent::Key(Key::parse("f2").unwrap()));
    let said = f.settle(effects);
    assert!(said[0].starts_with("Rename "), "{said:?}");
    assert!(said[0].contains(" to"), "{said:?}");
}

#[test]
fn the_words_fit_forty_cells() {
    for line in [READING, NO_BATCH, DONE_EDITING, NO_PROBLEMS, STILL_READING] {
        fits(line);
    }
    fits(&counter_words(&Counter::default()));
    for (_, words) in KINDS {
        fits(words);
    }
}
