//! Strip metadata into a copy, Check before sharing and Remove the
//! internet mark through the app (alpha 4, lane MA, R30 commit 56): each
//! asks first where it changes anything, starting on No; a strip writes a
//! copy beside the original, never over a file, and Undo trashes it; a
//! check reads only; a removed mark comes back with Undo.

use std::fs;
use std::path::Path;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_core::format::within_cells;
use diroxide_vfs::{OpContext, Provider};

use crate::support::{Session, place, registry_with_trash, settings, temp};

/// A small JPEG made in memory: a comment, XMP, a 1 by 1 frame.
fn beach_jpeg() -> Vec<u8> {
    let segments: [(u8, &[u8]); 2] = [
        (0xE1, b"http://ns.adobe.com/xap/1.0/\0<x/>"),
        (0xFE, b"from the beach"),
    ];
    let mut out = vec![0xFF, 0xD8];
    for (marker, body) in segments {
        out.extend_from_slice(&[0xFF, marker]);
        out.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(body);
    }
    out.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
    out.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn work(root: &Path) -> std::path::PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("beach.jpg"), beach_jpeg()).unwrap();
    fs::write(folder.join("notes.txt"), b"plain notes").unwrap();
    folder
}

fn labels(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("no list: {other:?}"),
    }
}

#[test]
fn strip_asks_first_writes_a_copy_beside_it_and_undo_trashes_the_copy() {
    let dir = temp();
    let work = work(dir.path());
    let (registry, trash) = registry_with_trash(&dir.path().join("bin"));
    let mut s = Session::start(place(&work), place(&work), registry, None, settings());
    let original = fs::read(work.join("beach.jpg")).unwrap();
    s.focus("beach.jpg");
    let said = s.command(ids::STRIP_METADATA);
    assert!(
        said[0].starts_with("Strip beach.jpg into a copy? No is the default. y or n"),
        "{said:?}"
    );
    // No changes nothing.
    let said = s.key("n");
    assert_eq!(said.last().unwrap(), "Nothing stripped.");
    assert!(!work.join("beach-clean.jpg").exists());
    s.command(ids::STRIP_METADATA);
    s.key("y");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Stripped into beach-clean.jpg")
    );
    let rows = labels(&s);
    assert_eq!(rows[0], "Removed: XMP data, comments");
    assert_eq!(rows[1], "Left in the copy: image size");
    for row in &rows {
        assert!(within_cells(row, row), "{row}");
    }
    s.key("escape");
    assert_eq!(fs::read(work.join("beach.jpg")).unwrap(), original);
    assert!(work.join("beach-clean.jpg").exists());
    assert!(
        s.names(diroxide_core::Side::Left)
            .contains(&"beach-clean.jpg".to_owned())
    );
    // A second strip never writes over the first copy.
    s.focus("beach.jpg");
    s.command(ids::STRIP_METADATA);
    s.key("y");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Stripped into beach-clean (2).jpg")
    );
    s.key("escape");
    // Undo puts the newest copy in the trash.
    let said = s.key("ctrl+z");
    assert!(
        said.iter()
            .any(|l| l == "Undid strip: 1 copy is in the trash"),
        "{said:?}"
    );
    assert!(!work.join("beach-clean (2).jpg").exists());
    assert_eq!(trash.count(), 1);
    // A file Strip does not take is said in words.
    s.focus("notes.txt");
    let said = s.command(ids::STRIP_METADATA);
    assert_eq!(
        said,
        ["Strip works on JPEG, PNG and WebP pictures; none is chosen"]
    );
}

#[test]
fn check_before_sharing_says_the_summary_then_each_file() {
    let dir = temp();
    let work = work(dir.path());
    let mut s = Session::start(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        None,
        settings(),
    );
    s.focus("beach.jpg");
    s.key("space");
    s.focus("notes.txt");
    s.key("space");
    let said = s.command(ids::CHECK_BEFORE_SHARING);
    assert_eq!(said[0], "Checking 2 files before sharing");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("1 of 2 files hold private details")
    );
    let rows = labels(&s);
    assert_eq!(rows[0], "beach.jpg: comments, hidden data", "{rows:?}");
    assert!(
        rows.contains(&"Comment: from the beach".to_owned()),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"notes.txt: nothing private found".to_owned()),
        "{rows:?}"
    );
    // Nothing was written.
    let names: Vec<String> = fs::read_dir(&work)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn the_internet_mark_is_asked_about_removed_and_put_back_by_undo() {
    let dir = temp();
    let work = work(dir.path());
    fs::write(work.join("setup.exe"), b"not really a program").unwrap();
    let file = place(&work.join("setup.exe"));
    let provider = diroxide_vfs::LocalProvider::new();
    let ctx = OpContext::new();
    if !provider.capabilities(&file).extended_attributes {
        return;
    }
    let extended = provider.as_extended().unwrap();
    let (name, bytes): (&str, &[u8]) = if cfg!(windows) {
        (
            diroxide_meta::origin::ZONE_STREAM,
            b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.org/setup.exe\r\n",
        )
    } else if cfg!(target_vendor = "apple") {
        (
            diroxide_meta::origin::QUARANTINE,
            b"0083;6542a1b2;Safari;0B7B4E1D",
        )
    } else {
        (
            diroxide_meta::origin::XDG_ORIGIN,
            b"https://example.org/setup.exe",
        )
    };
    if extended.write_extended(&file, name, bytes, &ctx).is_err() {
        return;
    }
    let mut s = Session::start(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        None,
        settings(),
    );
    s.focus("notes.txt");
    let said = s.command(ids::REMOVE_INTERNET_MARK);
    assert_eq!(said.last().unwrap(), "No internet mark on notes.txt");
    s.focus("setup.exe");
    let said = s.command(ids::REMOVE_INTERNET_MARK);
    let question = said.last().unwrap();
    assert!(
        question.starts_with("Remove the internet mark from setup.exe? "),
        "{question}"
    );
    assert!(
        question.ends_with("No is the default. y or n"),
        "{question}"
    );
    let said = s.key("n");
    assert_eq!(said.last().unwrap(), "The internet mark stays.");
    assert!(
        diroxide_meta::origin::read_mark(&provider, &file, &ctx)
            .unwrap()
            .is_some()
    );
    s.command(ids::REMOVE_INTERNET_MARK);
    let said = s.key("y");
    assert_eq!(
        said.last().unwrap(),
        "Removed the internet mark from setup.exe"
    );
    assert_eq!(
        diroxide_meta::origin::read_mark(&provider, &file, &ctx).unwrap(),
        None
    );
    assert_eq!(
        fs::read(work.join("setup.exe")).unwrap(),
        b"not really a program"
    );
    // Undo writes it back through the engine's undo record (lane AF).
    let said = s.key("ctrl+z");
    let undid = said.last().unwrap();
    assert_eq!(undid, "Undid mark removal: 1 item is back");
    assert!(undid.chars().count() <= 40, "{undid}");
    assert!(
        diroxide_meta::origin::read_mark(&provider, &file, &ctx)
            .unwrap()
            .is_some()
    );
}
