//! The File menu inside a zip (alpha 4, lane AE; AW's request 1): Make
//! folder, Rename, Move to trash and Delete for good show there, since
//! the keys already reach them and diroxide rewrites the zip; the items a
//! zip still refuses (Duplicate, Move, Cut, Paste) stay hidden, and a 7z
//! keeps every change hidden. Every archive is made at test time in the
//! test's own folder.

use std::fs;

use diroxide_vfs::Registry;

use crate::archives::{demo, pack_into};
use crate::support::{Session, Setup, place, temp};

/// The ids of a menu's rows for the active place.
fn ids_of(s: &Session, menu: &str) -> Vec<String> {
    s.app
        .menu_list(menu)
        .unwrap()
        .rows()
        .iter()
        .map(|r| r.id.clone())
        .collect()
}

#[test]
fn inside_a_zip_the_file_menu_offers_what_the_rewrite_does() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = Session::open(
        place(&demo),
        place(&demo.join("out")),
        Registry::with_local(),
        Setup::default(),
    );
    let outside = ids_of(&s, "file");
    for id in [
        "folder.make",
        "file.rename",
        "file.trash",
        "file.delete-permanently",
    ] {
        assert!(outside.contains(&id.to_owned()), "{id} in a folder");
    }
    s.focus("course.zip");
    s.key("enter");
    assert!(s.app.active_panel().place().archive().is_some());
    let file = ids_of(&s, "file");
    for id in [
        "folder.make",
        "file.rename",
        "file.trash",
        "file.delete-permanently",
    ] {
        assert!(file.contains(&id.to_owned()), "{id} inside a zip: {file:?}");
    }
    assert!(!file.contains(&"file.duplicate".to_owned()), "{file:?}");
    let edit = ids_of(&s, "edit");
    for id in ["clipboard.cut", "clipboard.paste", "file.move"] {
        assert!(
            !edit.contains(&id.to_owned()),
            "{id} inside a zip: {edit:?}"
        );
    }
}

#[test]
fn inside_a_7z_every_change_stays_hidden() {
    let dir = temp();
    let demo = demo(dir.path());
    let build = dir.path().join("build");
    pack_into(&build, "course.7z", &["notes.txt", "README.md"], None);
    fs::rename(build.join("course.7z"), demo.join("course.7z")).unwrap();
    let mut s = Session::open(
        place(&demo),
        place(&demo.join("out")),
        Registry::with_local(),
        Setup::default(),
    );
    s.focus("course.7z");
    s.key("enter");
    assert!(s.app.active_panel().place().archive().is_some());
    let file = ids_of(&s, "file");
    for id in [
        "folder.make",
        "file.rename",
        "file.trash",
        "file.delete-permanently",
    ] {
        assert!(!file.contains(&id.to_owned()), "{id} inside a 7z: {file:?}");
    }
}
