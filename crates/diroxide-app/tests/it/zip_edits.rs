//! Editing inside a zip (alpha 4, lane AW, R30 commit 82; the owner's
//! answer 114): Delete, Rename and Make folder in a panel inside a zip
//! plan a rewrite on a worker, ask with its cost and what it loses
//! (starting on No), write the new zip beside the old one, test it, and
//! only then send the old one to the trash, so Undo restores it. A 7z
//! keeps the read-only sentence. Every archive is made at test time in
//! the test's own folder, and the trash is a test folder.

use std::fs;
use std::path::Path;

use diroxide_core::EntryKind;
use diroxide_vfs::{OpContext, Registry};

use crate::archives::{demo, pack_into};
use crate::support::{Session, Setup, place, registry_with_trash, temp};

/// A session in the demo folder whose trash is a test folder, with the
/// trash itself for counting.
fn session(dir: &Path, demo: &Path) -> (Session, std::sync::Arc<crate::support::TestTrash>) {
    let bin = dir.join("bin");
    let (registry, trash) = registry_with_trash(&bin);
    let s = Session::open(
        place(demo),
        place(&demo.join("out")),
        registry,
        Setup::default(),
    );
    (s, trash)
}

/// The member paths of a zip, read by the archive provider, folders with
/// a slash at the end, in name order.
fn members(zip: &Path) -> Vec<String> {
    let settings = diroxide_config::ArchiveSettings::default();
    let provider = diroxide_app::archives::provider(&settings, None);
    let (registry, _provider) = diroxide_app::archives::install(&Registry::with_local(), provider);
    let mut out = Vec::new();
    let mut folders = vec![place(zip).into_archive()];
    while let Some(folder) = folders.pop() {
        let resolved = registry.resolve(&folder).unwrap();
        for entry in resolved.list(&folder, &OpContext::new()).unwrap() {
            let child = folder.join(&entry.name).unwrap();
            let (_, member) = child.archive().unwrap();
            if entry.kind == EntryKind::Folder {
                out.push(format!("{member}/"));
                folders.push(child);
            } else {
                out.push(member.to_owned());
            }
        }
    }
    out.sort();
    out
}

/// The confirmation's listing, for the open dialog.
fn listing(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(diroxide_app::Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    }
}

/// Every quoted line fits a Braille line with its meaning first.
fn fits(line: &str) {
    assert!(line.chars().count() <= 40, "{line}");
}

#[test]
fn delete_inside_a_zip_asks_with_the_cost_then_undo_restores_the_old_copy() {
    let dir = temp();
    let demo = demo(dir.path());
    let zip = demo.join("course.zip");
    let before = fs::read(&zip).unwrap();
    let (mut s, trash) = session(dir.path(), &demo);
    s.focus("course.zip");
    s.key("enter");
    s.focus("notes.txt");
    let said = s.key("delete");
    let question = said.last().cloned().unwrap_or_default();
    assert_eq!(
        question,
        "Remove notes.txt from course.zip? No is the default. y or n"
    );
    let rows = listing(&s);
    assert!(
        rows.iter()
            .any(|r| r.starts_with("Rewrites all of course.zip: ")),
        "{rows:?}"
    );
    for row in [
        "Zip comments and extra fields are lost",
        "Compression redone: stored stays stored",
        "Old copy: to the trash, Undo restores",
    ] {
        assert!(rows.iter().any(|r| r == row), "{row}: {rows:?}");
        fits(row);
    }
    for row in &rows {
        fits(row);
    }
    let said = s.key("y");
    let joined = said.join(" | ");
    assert!(
        joined.contains("Removed notes.txt from course.zip"),
        "{said:?}"
    );
    assert!(
        joined.contains("Old copy of course.zip is in the trash"),
        "{said:?}"
    );
    fits("Removed notes.txt from course.zip");
    fits("Old copy of course.zip is in the trash");
    assert_eq!(trash.count(), 1);
    let now = members(&zip);
    assert!(!now.contains(&"notes.txt".to_owned()), "{now:?}");
    assert!(now.contains(&"week1/a.txt".to_owned()), "{now:?}");
    assert!(now.contains(&"README.md".to_owned()), "{now:?}");
    // Undo: the new archive moves aside first, then the old one comes
    // back to the same name (Z's request 8), then the new one goes to the
    // trash.
    s.key("backspace");
    let said = s.key("ctrl+z");
    assert!(
        said.iter()
            .any(|l| l == "Undid zip change: the old copy is back"),
        "{said:?}"
    );
    fits("Undid zip change: the old copy is back");
    assert_eq!(fs::read(&zip).unwrap(), before, "the old copy is back");
    assert!(members(&zip).contains(&"notes.txt".to_owned()));
    let leftovers: Vec<String> = fs::read_dir(&demo)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("diro-undo-") || n.contains(".diro-pack"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(trash.count(), 1, "the new copy went to the trash");
}

#[test]
fn rename_and_make_folder_inside_a_zip_rewrite_it() {
    let dir = temp();
    let demo = demo(dir.path());
    let zip = demo.join("course.zip");
    let (mut s, _trash) = session(dir.path(), &demo);
    s.focus("course.zip");
    s.key("enter");
    s.focus("week1");
    s.key("enter");
    s.focus("a.txt");
    // The prompt holds "a.txt" with the cursor before the extension.
    s.key("f2");
    s.key("backspace");
    s.type_text("first");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Rename a.txt in course.zip? No is the default. y or n")
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Renamed a.txt in course.zip")),
        "{said:?}"
    );
    let now = members(&zip);
    assert!(now.contains(&"week1/first.txt".to_owned()), "{now:?}");
    assert!(!now.contains(&"week1/a.txt".to_owned()), "{now:?}");
    s.key("f7");
    s.type_text("drafts");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Make folder drafts in course.zip? No is the default. y or n")
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Made folder drafts in course.zip")),
        "{said:?}"
    );
    assert!(members(&zip).contains(&"week1/drafts/".to_owned()));
}

#[test]
fn no_leaves_the_zip_as_it_was_and_a_7z_keeps_the_old_sentence() {
    let dir = temp();
    let demo = demo(dir.path());
    let zip = demo.join("course.zip");
    let before = fs::read(&zip).unwrap();
    fs::write(demo.join("out").join("one.txt"), "one").unwrap();
    pack_into(&demo.join("out"), "set.7z", &["one.txt"], None);
    let (mut s, trash) = session(dir.path(), &demo);
    s.focus("course.zip");
    s.key("enter");
    s.focus("README.md");
    s.key("delete");
    let said = s.key("n");
    assert_eq!(said, ["Nothing changed."]);
    assert_eq!(fs::read(&zip).unwrap(), before);
    assert_eq!(trash.count(), 0);
    s.key("tab");
    s.focus("set.7z");
    s.key("enter");
    s.focus("one.txt");
    assert_eq!(
        s.key("delete"),
        ["Archives are read only here. Use Pack to make a new one."]
    );
    let said = s.key("f7");
    assert!(
        said.iter().any(|l| l.starts_with("New folder in set.7z")),
        "the name is asked first: {said:?}"
    );
    s.type_text("new");
    assert_eq!(
        s.key("enter"),
        ["Archives are read only here. Use Pack to make a new one."]
    );
}

#[test]
fn a_name_already_in_the_zip_is_refused_whole() {
    let dir = temp();
    let demo = demo(dir.path());
    let (mut s, _trash) = session(dir.path(), &demo);
    s.focus("course.zip");
    s.key("enter");
    s.key("f7");
    s.type_text("WEEK1");
    let said = s.key("enter");
    assert_eq!(said, ["WEEK1 is in course.zip already"]);
    assert!(s.dialog().is_none());
}

/// A made-up password, never anyone's.
const CANARY: &str = "canary-aw-not-a-password";

/// A locked zip: the password is asked once, through the masked prompt,
/// for reading the old members and locking the new ones; it is never
/// said or kept in a dialog.
#[test]
fn a_locked_zip_asks_its_password_once_and_stays_locked() {
    let dir = temp();
    let demo = demo(dir.path());
    let build = dir.path().join("build");
    fs::write(build.join("secret.txt"), "the secret\n").unwrap();
    fs::write(build.join("plain.txt"), "plain\n").unwrap();
    pack_into(
        &build,
        "locked.zip",
        &["secret.txt", "plain.txt"],
        Some(CANARY),
    );
    fs::rename(build.join("locked.zip"), demo.join("locked.zip")).unwrap();
    let (mut s, trash) = session(dir.path(), &demo);
    s.focus("locked.zip");
    s.key("enter");
    s.focus("plain.txt");
    s.key("delete");
    let rows = listing(&s);
    assert!(
        rows.iter()
            .any(|r| r == "Password: asked once, kept for the job"),
        "{rows:?}"
    );
    let said = s.key("y");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Password for locked.zip (hidden):")
    );
    s.type_text(CANARY);
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Removed plain.txt from locked.zip")),
        "{said:?}"
    );
    assert_eq!(members(&demo.join("locked.zip")), ["secret.txt"]);
    assert_eq!(trash.count(), 1);
    for message in &s.said {
        assert!(!message.text.contains(CANARY), "{}", message.text);
    }
    assert!(!format!("{:?}", s.app.dialogs()).contains(CANARY));
}

/// A RAR 5 listing as 7-Zip 24.08 prints it, one file, and that 7-Zip's
/// banner: what the fake 7-Zip answers. No real 7-Zip runs.
const RAR_LISTING: &str = "\n7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11\n\nListing archive: Backup.rar\n\n--\nPath = Backup.rar\nType = Rar5\nPhysical Size = 64\nSolid = -\nBlocks = 1\nEncrypted = -\n\n----------\nPath = notes.txt\nFolder = -\nSize = 13\nPacked Size = 13\nModified = 2026-11-06 12:48:43\nAttributes = A\nEncrypted = -\nCRC = 0A1B2C3D\n\n";

#[test]
fn a_rar_opens_through_7_zip_says_its_note_and_is_never_written() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut rar = b"Rar!\x1a\x07\x01\x00".to_vec();
    rar.resize(64, 0);
    fs::write(demo.join("Backup.rar"), &rar).unwrap();
    let bin = dir.path().join("bin");
    let (registry, _trash) = registry_with_trash(&bin);
    let setup = Setup::default();
    setup.tools.answer("7z", RAR_LISTING.as_bytes());
    let tools = setup.tools.clone();
    let mut s = Session::open(place(&demo), place(&demo.join("out")), registry, setup);
    s.focus("Backup.rar");
    let said = s.key("enter").join(" | ");
    assert!(said.contains("Opened Backup.rar"), "{said}");
    assert!(
        said.contains("7-Zip 24.08 is too old: needs 25.01"),
        "{said}"
    );
    fits("7-Zip 24.08 is too old: needs 25.01");
    assert!(tools.ran().iter().all(|t| t == "7z"));
    s.focus("notes.txt");
    assert_eq!(s.key("delete"), ["RAR is read only in diroxide"]);
    // Z's sentence is 42 cells; its meaning is in the first 16.
    assert!(diroxide_core::format::within_cells(
        "RAR is read only in diroxide",
        "RAR is read only"
    ));
}
