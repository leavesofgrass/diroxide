//! The details view, Say Details, Copy details and Show all (alpha 4,
//! lane VA, R30 commit 41): every fact a row in the order of facts, the
//! summary said with the focus kept, the rows copied with the private
//! ones left out, and ExifTool's tags through a recording runner.

use std::fs;
use std::path::Path;

use diroxide_app::commands::ids;
use diroxide_core::Effect;
use diroxide_core::format::within_cells;
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, place, png_bytes, temp};

fn session(work: &Path) -> Session {
    Session::open(
        place(work),
        place(work),
        Registry::with_local(),
        Setup::default(),
    )
}

fn work(root: &Path) -> std::path::PathBuf {
    let work = root.join("work");
    fs::create_dir_all(work.join("Shots")).unwrap();
    fs::write(work.join("photo.png"), png_bytes(4, 3)).unwrap();
    fs::write(work.join("notes.txt"), "Crows meet at dawn.\n").unwrap();
    work
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => {
            list.rows().iter().map(|r| r.label.clone()).collect()
        }
        other => panic!("{other:?}"),
    }
}

fn focused_row(s: &Session) -> String {
    match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => {
            list.focused().map(|r| r.label.clone()).unwrap_or_default()
        }
        other => panic!("{other:?}"),
    }
}

fn clipboard(s: &Session) -> Option<String> {
    s.effects.iter().rev().find_map(|e| match e {
        Effect::SetClipboardText(text) => Some(text.clone()),
        _ => None,
    })
}

/// ExifTool's answer for one file, as `-json -G -s` writes it, its tags
/// in ExifTool's own order.
fn exiftool_json(file: &Path) -> String {
    let source = serde_json::to_string(file.to_str().unwrap()).unwrap();
    format!(
        "[{{\"SourceFile\": {source}, \"ExifTool:ExifToolVersion\": 13.25, \
         \"EXIF:Make\": \"Example Camera\", \
         \"Composite:GPSPosition\": \"45 deg 30' N, 122 deg 41' W\"}}]"
    )
}

#[test]
fn details_list_a_pictures_facts_in_the_order_of_facts() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("photo.png");
    let said = s.command(ids::DETAILS);
    assert!(
        said[0].starts_with("Details of photo.png, Type: PNG"),
        "{said:?}"
    );
    let rows = rows(&s);
    let at = |start: &str| {
        rows.iter()
            .position(|r| r.starts_with(start))
            .unwrap_or_else(|| panic!("no {start}: {rows:?}"))
    };
    // What it is, what is in it, size, dates, privacy, place, system,
    // then Show all.
    let order = [
        "Type: ",
        "Name: photo.png",
        "Dimensions: 4 by 3",
        "Size: ",
        "Modified: ",
        "Privacy: nothing private found",
        "Path: ",
        "Checksum: not computed",
        "Show all: every tag, read by ExifTool",
    ];
    let places: Vec<usize> = order.iter().map(|o| at(o)).collect();
    assert!(places.windows(2).all(|w| w[0] < w[1]), "{rows:?}");
    assert_eq!(places.last(), Some(&(rows.len() - 1)), "{rows:?}");
    for row in &rows {
        let label = row.split(':').next().unwrap();
        assert!(within_cells(row, label), "{row}");
    }
    // Nothing ran: a picture's words come from its header.
    assert!(s.tools.ran().is_empty(), "{:?}", s.tools.ran());
    s.key("escape");
}

#[test]
fn a_folder_lists_no_privacy_checksum_or_show_all() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("Shots");
    s.command(ids::DETAILS);
    let rows = rows(&s);
    assert_eq!(rows[0], "Type: folder", "{rows:?}");
    assert!(rows.contains(&"Size: folder size not computed".to_owned()));
    assert!(!rows.iter().any(|r| r.starts_with("Privacy: ")
        || r.starts_with("Checksum: ")
        || r.starts_with("Show all")));
    assert!(!rows.iter().any(|r| r.ends_with("not known")), "{rows:?}");
}

#[test]
fn say_details_says_the_summary_and_the_focus_stays() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("photo.png");
    let said = s.command(ids::SAY_DETAILS);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("PNG, 4 by 3, 8-bit RGBA, "), "{said:?}");
    assert!(said[0].chars().count() <= 40, "{said:?}");
    assert_eq!(s.focused(), "photo.png");
    assert!(s.dialog().is_none());
    s.focus("Shots");
    assert_eq!(s.command(ids::SAY_DETAILS), ["folder"]);
}

#[test]
fn copy_details_from_the_panel_reads_first_and_leaves_private_rows_out() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("notes.txt");
    let said = s.command(ids::COPY_DETAILS);
    let copied = said.last().unwrap();
    assert!(copied.starts_with("Copied "), "{said:?}");
    assert!(copied.ends_with(" details"), "{said:?}");
    let text = clipboard(&s).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Details of notes.txt");
    assert!(
        lines.contains(&"Privacy: nothing private found"),
        "{lines:?}"
    );
    assert!(lines.contains(&"Checksum: not computed"), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.starts_with("Show all")),
        "{lines:?}"
    );
    assert!(s.dialog().is_none());
}

#[test]
fn show_all_adds_exiftools_tags_and_copy_leaves_the_place_out() {
    let dir = temp();
    let work = work(dir.path());
    let photo = work.join("photo.png");
    let mut s = session(&work);
    s.tools.answer("exiftool", exiftool_json(&photo).as_bytes());
    s.focus("photo.png");
    s.command(ids::DETAILS);
    s.key("end");
    assert_eq!(focused_row(&s), "Show all: every tag, read by ExifTool");
    let said = s.key("enter");
    assert!(
        said.contains(&"2 tags more, from ExifTool".to_owned()),
        "{said:?}"
    );
    assert_eq!(said.last().unwrap(), "Make (EXIF): Example Camera");
    let rows = rows(&s);
    let show = rows
        .iter()
        .position(|r| r == "Show all: 2 tags below, from ExifTool")
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert_eq!(
        &rows[show + 1..],
        [
            "Make (EXIF): Example Camera",
            "GPSPosition (Composite): 45 deg 30' N, 122 deg 41' W",
        ]
    );
    assert_eq!(s.tools.ran(), ["exiftool"]);
    let run = s.tools.runs.lock().unwrap()[0].clone();
    assert_eq!(run.args, ["-@", "-"]);
    // The place is private: Copy details leaves it out, Copy all keeps it.
    let said = s.command(ids::COPY_DETAILS);
    assert!(
        said.last().unwrap().ends_with(", 1 private left out"),
        "{said:?}"
    );
    let text = clipboard(&s).unwrap();
    assert!(text.contains("Make (EXIF): Example Camera"), "{text}");
    assert!(!text.contains("GPSPosition"), "{text}");
    let said = s.command(ids::COPY_ALL_DETAILS);
    assert!(said.last().unwrap().starts_with("Copied all "), "{said:?}");
    assert!(clipboard(&s).unwrap().contains("GPSPosition"));
    // Show all again goes to the tags; ExifTool is not run twice.
    s.key("home");
    s.key("end");
    s.key("up");
    s.key("up");
    s.key("enter");
    assert_eq!(s.tools.ran(), ["exiftool"]);
}

#[test]
fn show_all_without_exiftool_says_so_and_runs_nothing() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = session(&work);
    s.focus("photo.png");
    s.command(ids::DETAILS);
    s.key("end");
    let said = s.key("enter");
    assert_eq!(said.last().unwrap(), "More details: ExifTool not found");
    assert_eq!(rows(&s).last().unwrap(), "More details: ExifTool not found");
    assert!(s.tools.ran().is_empty());
}
