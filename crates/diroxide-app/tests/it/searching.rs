//! Search in the app (W3a, plan commit 37): the dialog as a list of
//! settings, the results streaming into a list that never moves the
//! focus, the end and the stop said in words, View at the match, Show in
//! its folder, refine, the check of a result that is gone, and jobs on
//! the real files.

use std::fs;
use std::io::{Cursor, Read};
use std::sync::Arc;
use std::time::Duration;

use diroxide_app::commands::ids;
use diroxide_app::{AppEvent, Dialog};
use diroxide_core::{Entry, EntryKind, Key, Place, Side};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, VfsError, WriteMode,
};

use crate::support::{Session, fixture, place, registry_with_trash, settings, temp};

struct Fixture {
    dir: tempfile::TempDir,
    work: std::path::PathBuf,
    s: Session,
}

fn open() -> Fixture {
    let dir = temp();
    let work = fixture(dir.path());
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let (registry, _trash) = registry_with_trash(&dir.path().join("bin"));
    let s = Session::start(place(&work), place(&out), registry, None, settings());
    Fixture { dir, work, s }
}

fn list_rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

fn last(said: &[String]) -> &str {
    said.last().map(String::as_str).unwrap_or_default()
}

#[test]
fn the_dialog_is_a_list_of_settings_and_typing_on_a_text_row_fills_it() {
    let mut f = open();
    let said = f.s.key("ctrl+f");
    assert_eq!(said, ["Search, Name or filter: every entry, 2 of 11"]);
    let rows = list_rows(&f.s);
    assert_eq!(rows[0], "Search work for every entry");
    assert_eq!(rows[2], "Contains text: none");
    assert_eq!(rows[3], "Look in: work");
    assert_eq!(rows[4], "Hidden files: left out");
    for row in &rows {
        assert!(
            row.chars().count() <= 40 || row.starts_with("Search "),
            "{row}"
        );
    }
    // Enter on a switch changes it and the list says the row again.
    f.s.key("down");
    f.s.key("down");
    f.s.key("down");
    let said = f.s.key("enter");
    assert_eq!(last(&said), "Search, Hidden files: searched, 5 of 11");
    f.s.key("home");
    f.s.key("down");
    // A letter on the name row opens its field with the letter.
    let said = f.s.key("*");
    assert_eq!(said, ["Name or filter: *"]);
    f.s.key("escape");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Search"));
    assert_eq!(f.s.key("escape"), ["Nothing was searched."]);
}

#[test]
fn a_name_search_streams_its_results_and_says_the_end() {
    let mut f = open();
    f.s.key("ctrl+f");
    f.s.type_text("*.txt");
    let said = f.s.key("enter");
    assert_eq!(said[0], "Searching work for *.txt", "{said:?}");
    assert!(last(&said).starts_with("Done: 4 found in "), "{said:?}");
    assert_eq!(
        f.s.dialog_title().as_deref(),
        Some("Results: name matches *.txt")
    );
    let rows = list_rows(&f.s);
    assert_eq!(rows.len(), 4);
    for row in &rows {
        assert!(row.contains(".txt, in work"), "{row}");
    }
    // Say Status says how the search stands, then the row.
    let said = f.s.command(ids::SAY_STATUS);
    assert!(said[0].starts_with("Done: 4 found in "), "{said:?}");
    // Escape closes the ended results; Search results opens them again.
    let said = f.s.key("escape");
    assert_eq!(said, ["Results closed. Search results opens them again."]);
    let said = f.s.command(ids::SEARCH_RESULTS);
    assert!(
        said[0].starts_with("Results: name matches *.txt, "),
        "{said:?}"
    );
    // The key bar shows the results' keys.
    let bar = f.s.app.key_bar();
    assert!(bar.starts_with("Escape Stop  F2 Pause  F3 View"), "{bar}");
}

#[test]
fn view_at_the_match_and_back_to_the_results() {
    let mut f = open();
    f.s.key("ctrl+f");
    f.s.key("down");
    // The contents row: typing fills it, Enter starts the search.
    f.s.type_text("jimothy");
    let said = f.s.key("enter");
    assert_eq!(
        said[0], "Searching work for every entry, containing jimothy",
        "{said:?}"
    );
    let rows = list_rows(&f.s);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].starts_with("raccoon \u{1f99d}.txt, line 1: jimothy"),
        "{rows:?}"
    );
    let said = f.s.key("f3");
    assert!(f.s.app.viewer_open(), "{said:?}");
    assert!(f.s.dialog().is_none(), "the results wait aside");
    assert!(
        said.iter().any(|s| s.starts_with("At line 1 of 1")),
        "{said:?}"
    );
    let said = f.s.key("escape");
    assert_eq!(said[0], "Viewer closed: raccoon \u{1f99d}.txt");
    assert!(
        last(&said).starts_with("Back in the results, raccoon"),
        "{said:?}"
    );
    assert_eq!(
        f.s.dialog_title().as_deref(),
        Some("Results: every entry, containing jimothy")
    );
}

#[test]
fn show_in_its_folder_refine_and_the_actions_list() {
    let mut f = open();
    f.s.key("ctrl+f");
    f.s.type_text("*");
    f.s.key("enter");
    let all = list_rows(&f.s).len();
    assert_eq!(all, 10, "{:?}", list_rows(&f.s));
    // The filter's key refines the results, with no new walk.
    let said = f.s.key("ctrl+s");
    assert_eq!(said, ["Refine results, empty"]);
    f.s.type_text("kind folder");
    assert_eq!(f.s.key("enter"), ["Refined: 2 of 10 shown"]);
    f.s.key("ctrl+s");
    for _ in 0.."kind folder".len() {
        f.s.key("backspace");
    }
    f.s.type_text("ext jpg");
    assert_eq!(f.s.key("enter"), ["Refined: 1 of 10 shown"]);
    // The context menu's key lists the actions.
    let said = f.s.key("shift+f10");
    assert!(
        said[0].starts_with("Actions on the results, View at the match, F3, 1 of 12"),
        "{said:?}"
    );
    f.s.key("escape");
    // The key that goes up shows the result in its folder.
    let said = f.s.key("backspace");
    assert!(f.s.dialog().is_none(), "{said:?}");
    assert_eq!(
        f.s.app.active_panel().place(),
        &place(&f.work.join("Photos"))
    );
    assert_eq!(f.s.focused(), "crow.jpg");
}

#[test]
fn a_result_gone_since_the_list_was_made_says_so_and_is_refused() {
    let mut f = open();
    f.s.key("ctrl+f");
    f.s.type_text("notes*");
    f.s.key("enter");
    assert_eq!(list_rows(&f.s).len(), 1);
    fs::remove_file(f.work.join("notes.txt")).unwrap();
    // The focus rests on the row: after the pause its file is checked.
    f.s.key("end");
    assert!(f.s.app.app_wants_tick().is_some());
    f.s.clock.advance(1);
    let effects = f.s.app.app_tick();
    let said = f.s.run(effects);
    assert_eq!(said, ["notes.txt, gone since this list was made"]);
    let said = f.s.key("f3");
    assert_eq!(
        said,
        ["notes.txt, gone since this list was made. Nothing was done."]
    );
}

#[test]
fn the_copy_key_copies_the_results_to_the_other_panel() {
    let mut f = open();
    f.s.key("ctrl+f");
    f.s.type_text("ext pdf");
    f.s.key("enter");
    assert_eq!(list_rows(&f.s).len(), 2, "two reports, one in Backup");
    f.s.key("space");
    let said = f.s.key("f5");
    assert!(
        said.iter().any(|s| s.starts_with("Copy 1 item to out")),
        "{said:?}"
    );
    let said = f.s.key("y");
    assert!(
        said.iter().any(|s| s.starts_with("Copy finished")),
        "{said:?}"
    );
    assert!(f.dir.path().join("out").join("report.pdf").exists());
    assert_eq!(
        f.s.dialog_title().as_deref(),
        Some("Results: extension is pdf"),
        "the results stay open"
    );
}

/// A slow place for the stop test: `slow://box/` has 200 folders of 25
/// files each, and every listing waits.
#[derive(Debug)]
struct Slow;

impl Slow {
    fn path(place: &Place) -> String {
        place
            .remote()
            .map(|r| r.path().trim_end_matches('/').to_owned())
            .unwrap_or_default()
    }
}

impl Provider for Slow {
    fn scheme(&self) -> &str {
        "slow"
    }
    fn capabilities(&self, _: &Place) -> Capabilities {
        Capabilities::read_only()
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        std::thread::sleep(Duration::from_millis(30));
        if ctx.cancel.is_cancelled() {
            return Err(VfsError::Cancelled);
        }
        let path = Self::path(place);
        if path.is_empty() {
            return Ok((0..200)
                .map(|i| Entry::new(format!("dir{i:03}"), EntryKind::Folder))
                .collect());
        }
        Ok((0..25)
            .map(|i| Entry::new(format!("file{i:02}.txt"), EntryKind::File).with_size(3))
            .collect())
    }
    fn stat(&self, place: &Place, _: &OpContext) -> Result<Entry, VfsError> {
        let path = Self::path(place);
        let name = path.rsplit('/').next().unwrap_or("box").to_owned();
        if path.is_empty() || !name.ends_with(".txt") {
            return Ok(Entry::new(name, EntryKind::Folder));
        }
        Ok(Entry::new(name, EntryKind::File).with_size(3))
    }
    fn read(&self, _: &Place, _: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Ok(Box::new(Cursor::new(b"abc".to_vec())))
    }
    fn write(
        &self,
        _: &Place,
        _: WriteMode,
        _: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn create_dir(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn rename(&self, _: &Place, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn remove(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
    fn set_attributes(
        &self,
        _: &Place,
        _: &AttributeChange,
        _: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(VfsError::ReadOnlyFilesystem)
    }
}

/// Presses a key and carries out its requests without waiting for the
/// workers; returns what was said.
fn press_no_wait(s: &mut Session, key: &str) -> Vec<String> {
    let effects = s.app.handle(AppEvent::Key(Key::parse(key).unwrap()));
    let screen = s.runtime.carry_out(&mut s.app, effects);
    s.run_quiet(screen)
}

#[test]
fn escape_stops_a_slow_search_with_the_count_and_the_focus_never_moves() {
    let dir = temp();
    let work = fixture(dir.path());
    let mut registry = Registry::with_local();
    registry.register("slow", Arc::new(Slow) as Arc<dyn Provider>);
    let mut s = Session::start(place(&work), place(&work), registry, None, settings());
    s.key("ctrl+f");
    s.key("down");
    s.key("down");
    // A letter on the place row starts its field with that letter.
    s.type_text("slow://box/");
    // Enter in the place field goes back to the dialog; the first row
    // starts the search.
    s.key("enter");
    s.key("home");
    let effects = s.app.handle(AppEvent::Key(Key::parse("enter").unwrap()));
    let screen = s.runtime.carry_out(&mut s.app, effects);
    assert!(
        screen.iter().any(|e| matches!(
            e,
            diroxide_core::Effect::Announce(m) if m.text.starts_with("Searching ")
        )),
        "{screen:?}"
    );
    // Take what has come so far without waiting for the end.
    let mut focus_was = None;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(20));
        for event in s.runtime.poll() {
            let effects = s.app.handle(event);
            s.run_quiet(effects);
        }
        if let Some(Dialog::Choose { list, .. }) = s.dialog()
            && list.len() > 1
        {
            focus_was = list.focused().map(|r| r.id.clone());
            break;
        }
    }
    let said: Vec<&str> = s.said.iter().map(|m| m.text.as_str()).collect();
    let focus_was = focus_was.unwrap_or_else(|| panic!("no results: {said:?}"));
    // Pause and resume, without waiting: a held search keeps its worker.
    let said = press_no_wait(&mut s, "f2");
    assert!(said[0].starts_with("Paused at "), "{said:?}");
    let said = press_no_wait(&mut s, "f2");
    assert!(said[0].starts_with("Searching again, "), "{said:?}");
    let said = s.key("escape");
    assert!(
        said.iter()
            .any(|t| t.starts_with("Stopped at ") && t.contains(" found.")),
        "{said:?}"
    );
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => {
            assert_eq!(list.focused().map(|r| r.id.clone()), Some(focus_was));
            assert_eq!(list.focus_index(), Some(0), "the focus never moved");
        }
        other => panic!("{other:?}"),
    }
    let _ = Side::Left;
}
