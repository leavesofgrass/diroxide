//! Links, permissions and the item information list (plan commit 66).

use std::fs;

use diroxide_app::commands::ids;
use diroxide_vfs::Registry;

use crate::archives::demo;
use crate::support::{Session, Setup, place, temp};

fn session(demo: &std::path::Path) -> Session {
    Session::open(
        place(demo),
        place(&demo.join("out")),
        Registry::with_local(),
        Setup::default(),
    )
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => {
            list.rows().iter().map(|r| r.label.clone()).collect()
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn details_list_the_detail_rows_and_an_archives_entries() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = session(&demo);
    s.focus("course.zip");
    let said = s.key("alt+enter");
    assert!(
        said[0].starts_with("Details of course.zip, Type: "),
        "{said:?}"
    );
    let rows = rows(&s);
    assert!(rows[1].starts_with("Size: "), "{rows:?}");
    assert!(rows[1].contains(" bytes)"), "{rows:?}");
    assert_eq!(rows[2], "Entries: 10 items", "{rows:?}");
    s.key("escape");
}

#[test]
fn the_link_dialog_is_a_list_of_settings() {
    let dir = temp();
    let demo = demo(dir.path());
    fs::write(demo.join("main.rs"), "fn main() {}\n").unwrap();
    let mut s = session(&demo);
    s.focus("main.rs");
    let said = s.command(ids::MAKE_LINK);
    assert_eq!(said, ["Make link, Make a link to main.rs in out, 1 of 4"]);
    let said = s.key("down");
    assert_eq!(said, ["Kind: symbolic, a pointer to the place, 2 of 4"]);
    s.key("down");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Make link, Target written: relative")),
        "{said:?}"
    );
    s.key("home");
    let said = s.key("enter");
    let question = said.last().cloned().unwrap_or_default();
    assert!(question.ends_with("No is the default. y or n"), "{said:?}");
    let said = s.key("y");
    let made = demo.join("out").join("main.rs");
    let link_made = fs::symlink_metadata(&made).is_ok_and(|m| m.file_type().is_symlink());
    assert!(
        link_made
            || said
                .iter()
                .any(|l| l.contains("Cannot link: needs developer mode")),
        "{said:?}"
    );
}

#[test]
fn the_permissions_dialog_changes_one_setting_and_plans_it_in_words() {
    let dir = temp();
    let demo = demo(dir.path());
    fs::write(demo.join("main.rs"), "fn main() {}\n").unwrap();
    let mut s = session(&demo);
    s.focus("main.rs");
    let said = s.command(ids::PERMISSIONS);
    assert!(
        said[0].starts_with("Permissions, Change permissions of main.rs, 1 of "),
        "{said:?}"
    );
    let row = if cfg!(windows) {
        "Hidden: off"
    } else {
        "Owner: read and write"
    };
    let rows_now = rows(&s);
    let at = rows_now
        .iter()
        .position(|r| r == row)
        .unwrap_or_else(|| panic!("{rows_now:?}"));
    for _ in 0..at {
        s.key("down");
    }
    let said = s.key("enter");
    let changed = if cfg!(windows) {
        "Hidden: on"
    } else {
        "Owner: read only"
    };
    assert!(said.iter().any(|l| l.contains(changed)), "{said:?}");
    s.key("home");
    let said = s.key("enter");
    assert!(
        said.last()
            .is_some_and(|q| q.ends_with("No is the default. y or n")),
        "{said:?}"
    );
    let listing = match s.dialog() {
        Some(diroxide_app::Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        listing
            .iter()
            .any(|l| l.starts_with("Apply to: this item only")),
        "{listing:?}"
    );
    s.key("y");
    s.key("ctrl+z");
}
