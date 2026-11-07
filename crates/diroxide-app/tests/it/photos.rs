//! The photo helpers through the app (alpha 4, lane AP, R30 commits 83 to
//! 85): pictures made at test time, their Exif dates and cameras written
//! by the test, never a picture of the owner's; every change is to the
//! test's own folder, and Undo puts it back.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_config::Settings;
use diroxide_core::format::within_cells;
use diroxide_core::{Side, Timestamp, UtcOffset};

use crate::support::{Session, place, png_bytes, registry_with_trash, settings, temp};

/// A TIFF block, little-endian: IFD0 with the maker and the model and a
/// pointer to the Exif folder, which holds DateTimeOriginal.
fn tiff(make: &str, model: &str, taken: &str) -> Vec<u8> {
    let ascii = |text: &str| {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        if bytes.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    };
    let (make, model, taken) = (ascii(make), ascii(model), ascii(taken));
    let entry = |out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    };
    let ifd0 = 8u32;
    let ifd0_size = 2 + 3 * 12 + 4;
    let make_at = ifd0 + ifd0_size;
    let model_at = make_at + make.len() as u32;
    let exif_at = model_at + model.len() as u32;
    let exif_size = 2 + 12 + 4;
    let taken_at = exif_at + exif_size;
    let mut out = b"II".to_vec();
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&ifd0.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    entry(&mut out, 0x010F, 2, make.len() as u32, make_at);
    entry(&mut out, 0x0110, 2, model.len() as u32, model_at);
    entry(&mut out, 0x8769, 4, 1, exif_at);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&make);
    out.extend_from_slice(&model);
    out.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut out, 0x9003, 2, 20, taken_at);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&taken);
    out
}

/// A one-pixel JPEG made in memory, with an Exif block written here.
fn photo(make: &str, model: &str, taken: &str) -> Vec<u8> {
    let mut exif = b"Exif\0\0".to_vec();
    exif.extend(tiff(make, model, taken));
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    out.extend(exif);
    out.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
    out.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// Sets the modified time of a test's own file.
fn set_modified(path: &Path, year: i32, month: u8, day: u8, hour: u8) {
    let at = Timestamp::from_civil(year, month, day, hour, 0, 0, UtcOffset::UTC).unwrap();
    let time = UNIX_EPOCH + Duration::from_secs(u64::try_from(at.unix_seconds()).unwrap());
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

/// The pictures: three from one camera within two seconds (a burst), one
/// the day before, one in December 2025, a screenshot with no Exif, and
/// a text file.
fn pictures(root: &Path) -> PathBuf {
    let folder = root.join("pics");
    fs::create_dir_all(&folder).unwrap();
    let shots = [
        ("IMG_0001.jpg", "2026:10:11 09:00:00"),
        ("IMG_0002.jpg", "2026:10:12 14:03:00"),
        ("IMG_0003.jpg", "2026:10:12 14:03:01"),
        ("IMG_0004.jpg", "2026:10:12 14:03:02"),
        ("old.jpg", "2025:12:31 10:00:00"),
    ];
    for (name, taken) in shots {
        let path = folder.join(name);
        fs::write(&path, photo("Google", "Pixel 8", taken)).unwrap();
        set_modified(&path, 2026, 11, 1, 12);
    }
    let shot = folder.join("Screenshot (3).png");
    fs::write(&shot, png_bytes(4, 3)).unwrap();
    set_modified(&shot, 2026, 10, 13, 10);
    fs::write(folder.join("notes.txt"), b"notes\n").unwrap();
    folder
}

fn session(folder: &Path, settings: Settings) -> Session {
    let (registry, _) = registry_with_trash(&folder.parent().unwrap().join("bin"));
    Session::start(place(folder), place(folder), registry, None, settings)
}

fn labels(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("no list: {other:?}"),
    }
}

fn focused(s: &Session) -> String {
    s.app
        .panel(Side::Left)
        .list()
        .focused()
        .map(|r| r.id.clone())
        .unwrap_or_default()
}

/// Commit 83: Next day and Previous day, the date from the pictures and
/// the weekday from the calendar; a screenshot with no date taken goes by
/// its modified time.
#[test]
fn next_day_and_previous_day_step_through_the_dates() {
    let dir = temp();
    let folder = pictures(dir.path());
    let mut s = session(&folder, settings());
    s.focus("IMG_0001.jpg");
    let said = s.command(ids::NEXT_DAY);
    assert_eq!(said[0], "Reading 6 pictures", "{said:?}");
    // October 12, 2026 was a Monday: computed, never typed.
    assert!(
        said.iter()
            .any(|l| l == "Next day: October 12, 3 pictures, Monday"),
        "{said:?}"
    );
    assert_eq!(focused(&s), "IMG_0002.jpg");
    let said = s.command(ids::NEXT_DAY);
    assert_eq!(said[0], "Next day: October 13, 1 picture, Tuesday");
    assert_eq!(focused(&s), "Screenshot (3).png");
    assert_eq!(s.command(ids::NEXT_DAY), ["No later day here"]);
    s.focus("IMG_0001.jpg");
    let said = s.command(ids::PREVIOUS_DAY);
    assert_eq!(
        said[0],
        "Previous day: December 31, 2025, 1 picture, Wednesday"
    );
    assert!(within_cells(&said[0], "Previous day: December 31, 2025"));
    assert_eq!(focused(&s), "old.jpg");
}

/// Commit 83: the three selections, each said by count first, the marks
/// before kept for Restore selection.
#[test]
fn the_screenshots_the_bursts_and_the_pictures_with_no_date_are_selected() {
    let dir = temp();
    let folder = pictures(dir.path());
    let mut s = session(&folder, settings());
    let said = s.command(ids::SELECT_BURSTS);
    let line = said.last().unwrap();
    assert_eq!(line, "Burst, 3 pictures in 2 seconds. 3 selected.");
    assert!(within_cells(line, "Burst, 3 pictures in 2 seconds"));
    let said = s.command(ids::SELECT_SCREENSHOTS);
    assert_eq!(said, ["Selected 1 screenshot. 1 selected."]);
    let said = s.command(ids::SELECT_NO_DATE);
    assert_eq!(said, ["Selected 1 picture with no date taken. 1 selected."]);
    // A gap of a single second still holds the three, one second apart.
    let mut narrow = settings();
    narrow.photos.burst_gap_seconds = 1;
    let mut s = session(&folder, narrow);
    let said = s.command(ids::SELECT_BURSTS);
    assert_eq!(
        said.last().unwrap(),
        "Burst, 3 pictures in 2 seconds. 3 selected."
    );
}

/// Commit 83: Sort by date taken, the modified time where a picture has
/// none, said; and the photo columns in the rows.
#[test]
fn sort_by_date_taken_and_the_photo_columns() {
    let dir = temp();
    let folder = pictures(dir.path());
    let mut columns = settings();
    columns.columns = ["photo.taken", "photo.camera", "photo.pixels", "photo.kind"]
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
    let mut s = session(&folder, columns);
    let said = s.command(ids::SORT_DATE_TAKEN);
    assert_eq!(
        said.last().unwrap(),
        "Sorted by date taken, oldest first. Date taken; 1 uses modified time."
    );
    let names = s.names(Side::Left);
    let at = |name: &str| names.iter().position(|n| n == name).unwrap();
    assert!(at("old.jpg") < at("IMG_0001.jpg"), "{names:?}");
    assert!(at("IMG_0001.jpg") < at("IMG_0002.jpg"), "{names:?}");
    assert!(at("IMG_0004.jpg") < at("Screenshot (3).png"), "{names:?}");
    // The columns fill near the focus, off the input thread.
    let mut rows = Vec::new();
    for _ in 0..50 {
        rows = s
            .app
            .panel(Side::Left)
            .list()
            .rows()
            .iter()
            .map(|r| r.spoken())
            .collect();
        if rows.iter().any(|r| r.contains("camera Google Pixel 8"))
            && rows.iter().any(|r| r.contains("kind screenshot"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        let events = s.runtime.poll();
        let effects: Vec<_> = events
            .into_iter()
            .flat_map(|event| s.app.handle(event))
            .collect();
        s.run(effects);
    }
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.starts_with(name))
            .cloned()
            .unwrap_or_else(|| panic!("no row {name}: {rows:?}"))
    };
    let one = row("IMG_0002.jpg");
    assert!(one.contains("taken 2026-10-12 14:03"), "{one}");
    assert!(one.contains("camera Google Pixel 8"), "{one}");
    assert!(one.contains("pixels 1 by 1"), "{one}");
    assert!(one.contains("kind photo"), "{one}");
    let shot = row("Screenshot (3).png");
    assert!(shot.contains("taken 2026-10-13 10:00 (modified)"), "{shot}");
    assert!(shot.contains("kind screenshot"), "{shot}");
    assert!(
        !row("notes.txt").contains("kind"),
        "a text file is no picture"
    );
}

/// Commit 84: the review steps through the pictures one key an answer;
/// T sends to the trash as a job Undo takes back.
#[test]
fn the_review_answers_with_one_key_each() {
    let dir = temp();
    let folder = pictures(dir.path());
    let mut s = session(&folder, settings());
    s.focus("IMG_0002.jpg");
    let said = s.command(ids::REVIEW_PICTURES);
    assert!(
        said.iter()
            .any(|l| l.starts_with("Review: 2 of 6, IMG_0002.jpg, photo")),
        "{said:?}"
    );
    assert_eq!(
        labels(&s),
        [
            "Keep, K",
            "Label, L",
            "Move to a place you pick, M",
            "Trash, T",
            "Skip, S",
            "Back, B"
        ]
    );
    let said = s.key("k");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Review: 3 of 6, IMG_0003.jpg, photo")),
        "{said:?}"
    );
    s.key("t");
    assert!(!folder.join("IMG_0003.jpg").exists());
    let said = s.key("b");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Review: 3 of 6, IMG_0003.jpg, photo, in the trash")),
        "{said:?}"
    );
    let said = s.key("escape");
    assert_eq!(
        said.last().unwrap(),
        "Review ended at 3 of 6, 1 kept, 1 in the trash"
    );
    s.key("ctrl+z");
    assert!(folder.join("IMG_0003.jpg").exists(), "Undo brought it back");
}

/// Commit 85: Move into month folders asks first, starting on No, moves
/// every picture into its month's folder as one job, and one Undo puts
/// them all back and removes the folders it made.
#[test]
fn move_into_month_folders_asks_moves_and_undoes_in_one_step() {
    let dir = temp();
    let folder = pictures(dir.path());
    let mut s = session(&folder, settings());
    let said = s.command(ids::MONTH_FOLDERS);
    let question = said.last().unwrap();
    assert!(
        question.starts_with("Move 6 pictures into 2 month folders? No is the default."),
        "{said:?}"
    );
    let listing = match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        listing,
        [
            "2025-12: 1 picture, a new folder",
            "2026-10: 5 pictures, a new folder",
            "Date taken; 1 uses modified time.",
            "No date taken: using modified time",
        ]
    );
    let said = s.key("n");
    assert_eq!(said.last().unwrap(), "Nothing moved.");
    assert!(!folder.join("2026-10").exists());
    s.command(ids::MONTH_FOLDERS);
    s.key("y");
    assert!(folder.join("2026-10").join("IMG_0002.jpg").exists());
    assert!(folder.join("2026-10").join("Screenshot (3).png").exists());
    assert!(folder.join("2025-12").join("old.jpg").exists());
    assert!(folder.join("notes.txt").exists(), "a text file stays");
    s.key("ctrl+z");
    assert!(folder.join("IMG_0002.jpg").exists());
    assert!(folder.join("old.jpg").exists());
    assert!(
        !folder.join("2026-10").exists(),
        "the folder it made is gone"
    );
    assert!(!folder.join("2025-12").exists());
}
