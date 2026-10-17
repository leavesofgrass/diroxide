//! What search offers beyond Find files (W3a, plan commit 38): saved
//! searches that run on open, the Space list, the largest items below a
//! folder, folder sizes and Size every folder with the size sort, and
//! newest files.

use std::fs;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_config::Paths;
use diroxide_vfs::Registry;

use crate::support::{Session, fixture, place, settings, temp};

struct Fixture {
    _dir: tempfile::TempDir,
    paths: Paths,
    s: Session,
}

fn open() -> Fixture {
    let dir = temp();
    let work = fixture(dir.path());
    fs::write(work.join("Photos").join("big.jpg"), vec![b'p'; 5000]).unwrap();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let s = Session::start(
        place(&work),
        place(&work),
        Registry::with_local(),
        Some(&paths),
        settings(),
    );
    Fixture {
        _dir: dir,
        paths,
        s,
    }
}

fn last(said: &[String]) -> &str {
    said.last().map(String::as_str).unwrap_or_default()
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn a_saved_search_is_kept_listed_and_runs_on_open() {
    let mut f = open();
    assert_eq!(
        f.s.command(ids::SEARCH_SAVE),
        ["No search to save. Find files starts one."]
    );
    f.s.key("ctrl+f");
    f.s.type_text("*.txt");
    f.s.key("enter");
    f.s.key("escape");
    let said = f.s.command(ids::SEARCH_SAVE);
    assert_eq!(said, ["Save the search as: name matches *.txt"]);
    for _ in 0.."name matches *.txt".len() {
        f.s.key("backspace");
    }
    f.s.type_text("Text files");
    let said = f.s.key("enter");
    assert_eq!(
        said,
        ["Search saved as Text files. Saved searches runs it."]
    );
    assert!(
        f.paths
            .state_dir
            .join("saved")
            .join("text-files.toml")
            .exists()
    );
    let said = f.s.command(ids::SEARCH_SAVED);
    assert_eq!(
        said,
        ["Saved searches, 1 search, Text files, saved search, runs on open, 1 of 1"]
    );
    let said = f.s.key("enter");
    assert!(
        said[0].starts_with("Running Text files: Searching work for "),
        "{said:?}"
    );
    assert!(last(&said).starts_with("Done: 4 found in "), "{said:?}");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Results: Text files"));
    f.s.key("escape");
    // The trash key deletes a saved search after a question on No.
    f.s.command(ids::SEARCH_SAVED);
    let said = f.s.key("delete");
    assert_eq!(
        said,
        ["Delete saved search Text files? No is the default. y or n"]
    );
    assert_eq!(f.s.key("y"), ["Deleted saved search Text files"]);
    assert!(
        !f.paths
            .state_dir
            .join("saved")
            .join("text-files.toml")
            .exists()
    );
}

#[test]
fn the_space_list_says_the_total_first_and_goes_into_a_folder() {
    let mut f = open();
    let said = f.s.command(ids::SPACE_LIST);
    assert_eq!(said[0], "Counting the space in work");
    assert!(
        said[1].starts_with("Total 7.4 KB. Largest 3 hold "),
        "{said:?}"
    );
    assert!(
        said[1].chars().count() <= 40 || said[1].contains(". "),
        "{said:?}"
    );
    let list = rows(&f.s);
    assert!(list[0].starts_with("Photos, folder, 5.0 KB, "), "{list:?}");
    let said = f.s.key("enter");
    assert!(last(&said).starts_with("Total 5.0 KB."), "{said:?}");
    assert_eq!(
        f.s.dialog_title().as_deref(),
        Some("Space in Photos, largest first")
    );
    let said = f.s.key("backspace");
    assert!(last(&said).starts_with("Total 7.4 KB."), "{said:?}");
    f.s.key("escape");
    let said = f.s.command(ids::SPACE_BY_KIND);
    assert!(last(&said).starts_with("Total "), "{said:?}");
    assert!(
        rows(&f.s)
            .iter()
            .any(|r| r.starts_with("Images, 2 files, ")),
        "{:?}",
        rows(&f.s)
    );
}

#[test]
fn the_largest_items_and_newest_files_open_as_results() {
    let mut f = open();
    let said = f.s.command(ids::LARGEST_BELOW);
    assert_eq!(said[0], "Looking for the largest items below work");
    assert!(
        last(&said).starts_with("Done: the largest 9 items below work"),
        "{said:?}"
    );
    let list = rows(&f.s);
    assert!(
        list[0].starts_with("big.jpg, in work/Photos, 5.0 KB"),
        "{list:?}"
    );
    f.s.key("escape");
    let said = f.s.command(ids::NEWEST_FILES);
    assert_eq!(said[0], "Looking for the newest files below work");
    assert!(
        last(&said).starts_with("Done. Newest files: 9 changed on "),
        "{said:?}"
    );
    let title = f.s.dialog_title().unwrap();
    assert!(title.starts_with("Newest files: 9 changed on "), "{title}");
    // Earlier day, from the results' actions: nothing before today here.
    f.s.key("shift+f10");
    f.s.key("end");
    let said = f.s.key("enter");
    assert!(
        last(&said).starts_with("Done. Newest files: none before "),
        "{said:?}"
    );
}

#[test]
fn folder_sizes_and_size_every_folder_with_the_size_sort() {
    let mut f = open();
    f.s.focus("Photos");
    let said = f.s.command(ids::DIR_SIZES);
    assert_eq!(
        said,
        [
            "Sizing 1 folder",
            "Photos: 5.0 KB in 2 files and no folders"
        ]
    );
    let row = f.s.app.active_panel().list().focused().unwrap().spoken();
    assert!(row.starts_with("Photos, folder, 5.0 KB"), "{row}");
    let said = f.s.command(ids::SIZE_EVERY_FOLDER);
    assert_eq!(said[0], "Sizing 2 folders");
    assert_eq!(
        last(&said),
        "Sized 2 folders, 5.0 KB in all. Sort by size? No is the default. y or n"
    );
    let said = f.s.key("y");
    assert_eq!(said, ["Sorted by size, largest first"]);
    assert_eq!(f.s.names(diroxide_core::Side::Left)[0], "Photos");
}
