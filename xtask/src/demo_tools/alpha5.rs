//! The alpha 5 fixtures (R34's section 5.2): what the owner's check of
//! tabs, batch rename, compare and synchronize, duplicates, the flat
//! view, collections, a list from a pipe and recent files reads, with
//! plain names, fixed bytes and fixed modified times, nothing downloaded
//! and no hostile name on the disk.
//!
//! - `rename`: `IMG_0001.jpg` to `IMG_0050.jpg`, a few bytes each, and
//!   `a.txt` and `b.txt`;
//! - `sync/here` and `sync/there`, ten relative paths: `a.txt`, `b.txt`,
//!   `c.txt` and `docs/guide.md` the same on both sides; `d.txt` the same
//!   with times one second apart; `notes.txt` newer and larger here;
//!   `todo.txt` newer there; `plan.md` only here; `old.txt` only there;
//!   `thing`, a file here and an empty folder there;
//! - `dups`: three groups (`crow.png` three times at 16,000 bytes, the
//!   oldest in `photos/a`; `song.txt` twice at 8,000; `report.md` twice
//!   at 4,000), `near-a.bin` and `near-b.bin` (alike at both ends,
//!   different inside), two empty files, and `linked/one.txt` and
//!   `linked/two.txt`, one file under two names where the file system
//!   makes a hard link;
//! - `flat/work`: 14 files in 4 folders three levels deep, `readme.txt`
//!   in `a` and in `b/deep`;
//! - `state/saved`: the collection "Trip photos" (three items, one of
//!   them `old/place/gone.txt`, which does not exist) and the saved
//!   search "Large images", in the shape `diroxide-search` reads;
//! - `open`: `report.md` and `notes.txt` for recent files;
//! - `paths.nul`: three paths into the fixtures, each ended by a NUL.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use diroxide_core::Place;
use diroxide_search::{SAVED_FOLDER, SAVED_FORMAT, Saved, SavedItem, SavedKind, file_stem_for};

use super::{make_folder, taken, utc, write_file};
use crate::demo::REPORT_MD;
use crate::sample::png;

/// The number of pictures in `rename`.
pub const PICTURES: u32 = 50;

/// The size of each `crow.png` in `dups`: 16 KB in the app's words.
pub const CROW_BYTES: usize = 16_000;

/// The size of each `song.txt` in `dups`.
pub const SONG_BYTES: usize = 8_000;

/// The size of each `report.md` in `dups`.
pub const REPORT_BYTES: usize = 4_000;

/// The size of the hard-linked `linked/one.txt`.
pub const LINKED_BYTES: usize = 2_000;

/// The size of `near-a.bin` and `near-b.bin`: three blocks of 64 KiB, so
/// the first and last 64 KiB are alike and the middle one is not.
pub const NEAR_BYTES: usize = 3 * NEAR_BLOCK;

/// The block a quick duplicate check reads at each end of a file.
pub const NEAR_BLOCK: usize = 64 * 1024;

/// The one byte where `near-b.bin` differs from `near-a.bin`.
pub const NEAR_DIFFERS_AT: usize = NEAR_BYTES / 2;

/// The display name of the demo's collection.
pub const COLLECTION: &str = "Trip photos";

/// The display name of the demo's saved search.
pub const SAVED_SEARCH: &str = "Large images";

/// The saved search's filter, as typed.
pub const SAVED_EXPRESSION: &str = "group images and size over 10 KB";

/// The collection's item that does not exist, below the demo folder.
pub const GONE: [&str; 3] = ["old", "place", "gone.txt"];

/// The files `paths.nul` names, below the demo folder.
pub const PIPED: [[&str; 4]; 3] = [
    ["open", "report.md", "", ""],
    ["open", "notes.txt", "", ""],
    ["flat", "work", "a", "readme.txt"],
];

/// What the alpha 5 part of the set made that its lines must say.
pub struct Made5 {
    /// Whether `dups/linked/two.txt` is a hard link to `one.txt`.
    pub linked: bool,
}

/// A time `seconds` after another.
fn after(time: SystemTime, seconds: u64) -> SystemTime {
    time + Duration::from_secs(seconds)
}

/// One day in seconds.
const DAY: u64 = 24 * 60 * 60;

/// Some text of exactly `size` bytes: `line` again and again, cut.
pub fn text_of_size(line: &str, size: usize) -> Vec<u8> {
    line.bytes().cycle().take(size).collect()
}

/// `size` bytes of fixed noise from a seed (xorshift, never zero).
pub fn noise(seed: u32, size: usize) -> Vec<u8> {
    let mut x = seed.max(1);
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out.push(x.to_be_bytes()[0]);
    }
    out
}

// ---------------------------------------------------------------------
// rename

/// The name of picture `n` in `rename`: `IMG_0007.jpg`.
pub fn picture_name(n: u32) -> String {
    format!("IMG_{n:04}.jpg")
}

/// Writes the fifty pictures and the two names to swap.
fn rename(folder: &Path) -> Result<()> {
    make_folder(folder)?;
    let start = utc(10, 11, 9, 30);
    for n in 1..=PICTURES {
        write_file(
            &folder.join(picture_name(n)),
            format!("Example picture {n}.\n").as_bytes(),
            after(start, u64::from(n) * 60),
        )?;
    }
    write_file(
        &folder.join("a.txt"),
        b"This is the first file, a.\n",
        start,
    )?;
    write_file(
        &folder.join("b.txt"),
        b"This is the second file, b.\n",
        start,
    )
}

// ---------------------------------------------------------------------
// sync

/// The relative paths of `sync`, each side's entries together: ten.
#[cfg(test)]
pub const SYNC_PATHS: [&str; 10] = [
    "a.txt",
    "b.txt",
    "c.txt",
    "docs/guide.md",
    "d.txt",
    "notes.txt",
    "todo.txt",
    "plan.md",
    "old.txt",
    "thing",
];

/// Writes `sync/here` and `sync/there`.
fn sync(folder: &Path) -> Result<()> {
    let here = folder.join("here");
    let there = folder.join("there");
    let base = utc(10, 20, 9, 0);
    let newer = after(base, DAY);
    for side in [&here, &there] {
        make_folder(&side.join("docs"))?;
        write_file(
            &side.join("a.txt"),
            b"Alpha, the same on both sides.\n",
            base,
        )?;
        write_file(
            &side.join("b.txt"),
            b"Bravo, the same on both sides.\n",
            base,
        )?;
        write_file(
            &side.join("c.txt"),
            b"Charlie, the same on both sides.\n",
            base,
        )?;
        write_file(
            &side.join("docs").join("guide.md"),
            b"# Guide\n\nThe same on both sides.\n",
            base,
        )?;
    }
    // One second apart: within the two seconds a compare allows.
    let delta = b"Delta, the same bytes, one second apart.\n";
    write_file(&here.join("d.txt"), delta, base)?;
    write_file(&there.join("d.txt"), delta, after(base, 1))?;
    write_file(
        &here.join("notes.txt"),
        b"Notes, version 2: newer and longer here.\nA second line.\n",
        newer,
    )?;
    write_file(&there.join("notes.txt"), b"Notes, version 1.\n", base)?;
    write_file(&here.join("todo.txt"), b"To do, version 1.\n", base)?;
    write_file(
        &there.join("todo.txt"),
        b"To do, version 2: newer there.\n",
        newer,
    )?;
    write_file(&here.join("plan.md"), b"# Plan\n\nOnly here.\n", base)?;
    write_file(&there.join("old.txt"), b"Old, only there.\n", base)?;
    write_file(&here.join("thing"), b"A file here, a folder there.\n", base)?;
    make_folder(&there.join("thing"))
}

// ---------------------------------------------------------------------
// dups

/// Writes `dups`; says whether the hard link was made.
fn dups(folder: &Path) -> Result<bool> {
    let oldest = utc(9, 1, 12, 0);
    let mut crow = png(32, 32, [24, 24, 32]);
    crow.resize(CROW_BYTES, 0);
    for (at, (part, name)) in [("a", "crow.png"), ("b", "crow.png"), ("c", "crow-copy.png")]
        .into_iter()
        .enumerate()
    {
        let place = folder.join("photos").join(part);
        make_folder(&place)?;
        write_file(&place.join(name), &crow, after(oldest, at as u64 * DAY))?;
    }
    let song = text_of_size("La la la, the example song's words. ", SONG_BYTES);
    let report = text_of_size("The example report, saved twice. ", REPORT_BYTES);
    for (at, part) in ["", "backup"].into_iter().enumerate() {
        let music = folder.join("music").join(part);
        let docs = folder.join("docs").join(part);
        make_folder(&music)?;
        make_folder(&docs)?;
        let time = after(oldest, at as u64 * DAY);
        write_file(&music.join("song.txt"), &song, time)?;
        write_file(&docs.join("report.md"), &report, time)?;
    }
    let near = noise(0x5EED_0001, NEAR_BYTES);
    let mut other = near.clone();
    other[NEAR_DIFFERS_AT] ^= 0xFF;
    write_file(&folder.join("near-a.bin"), &near, oldest)?;
    write_file(&folder.join("near-b.bin"), &other, oldest)?;
    write_file(&folder.join("empty-1.txt"), b"", oldest)?;
    write_file(&folder.join("empty-2.txt"), b"", oldest)?;
    let linked = folder.join("linked");
    make_folder(&linked)?;
    let one = linked.join("one.txt");
    write_file(
        &one,
        &text_of_size("One file under two names. ", LINKED_BYTES),
        oldest,
    )?;
    Ok(fs::hard_link(&one, linked.join("two.txt")).is_ok())
}

// ---------------------------------------------------------------------
// flat

/// The files of `flat/work`, by folder: 14 in 4 folders, three deep.
pub const FLAT: [(&str, &[&str]); 4] = [
    ("a", &["readme.txt", "one.txt", "two.txt", "three.txt"]),
    ("b", &["four.txt", "five.txt", "six.txt"]),
    (
        "b/deep",
        &["readme.txt", "seven.txt", "eight.txt", "nine.txt"],
    ),
    ("b/deep/old", &["ten.txt", "eleven.txt", "twelve.txt"]),
];

/// Writes `flat/work`.
fn flat(folder: &Path) -> Result<()> {
    let work = folder.join("work");
    let start = utc(10, 15, 14, 0);
    let mut n = 0;
    for (part, names) in FLAT {
        let place = part
            .split('/')
            .fold(work.clone(), |path, name| path.join(name));
        make_folder(&place)?;
        for name in names {
            n += 1;
            write_file(
                &place.join(name),
                format!("{name} in work/{part}.\n").as_bytes(),
                after(start, n * 60),
            )?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// state

/// A place for a path below the demo folder.
fn place_of(folder: &Path, parts: &[&str]) -> Result<Place> {
    let path = parts
        .iter()
        .filter(|part| !part.is_empty())
        .fold(folder.to_path_buf(), |path, part| path.join(part));
    let text = path.display().to_string();
    Place::parse(&text).with_context(|| format!("{text} could not be read as a place"))
}

/// The demo's collection: two crows and one file that is gone.
pub fn collection(folder: &Path) -> Result<Saved> {
    let items = [
        place_of(folder, &["dups", "photos", "a", "crow.png"])?,
        place_of(folder, &["dups", "photos", "b", "crow.png"])?,
        place_of(folder, &GONE)?,
    ];
    let mut saved = blank(COLLECTION, SavedKind::Collection);
    saved.items = items
        .into_iter()
        .map(|place| SavedItem { place, label: None })
        .collect();
    Ok(saved)
}

/// The demo's saved search: large images anywhere in the demo folder.
pub fn saved_search(folder: &Path) -> Result<Saved> {
    let mut saved = blank(SAVED_SEARCH, SavedKind::Search);
    saved.places = vec![place_of(folder, &[])?];
    saved.expression = Some(SAVED_EXPRESSION.to_owned());
    saved.options = Some(diroxide_search::SavedOptions::default());
    Ok(saved)
}

/// A saved file with a name and kind, made at the fixed time.
fn blank(name: &str, kind: SavedKind) -> Saved {
    Saved {
        format: SAVED_FORMAT,
        name: name.to_owned(),
        kind,
        created: Some("2026-10-11T09:30:00Z".to_owned()),
        places: Vec::new(),
        expression: None,
        case: None,
        content: None,
        options: None,
        sort: None,
        items: Vec::new(),
        extra: std::collections::BTreeMap::new(),
    }
}

/// The file a saved search or collection is kept in, below `state`.
pub fn saved_file(state: &Path, name: &str) -> std::path::PathBuf {
    state
        .join(SAVED_FOLDER)
        .join(format!("{}.toml", file_stem_for(name)))
}

/// Writes `state/saved` with the collection and the saved search.
fn state(folder: &Path, demo: &Path) -> Result<()> {
    make_folder(&folder.join(SAVED_FOLDER))?;
    for saved in [collection(demo)?, saved_search(demo)?] {
        let text = saved
            .to_toml()
            .with_context(|| format!("{} could not be written as TOML", saved.name))?;
        write_file(&saved_file(folder, &saved.name), text.as_bytes(), taken())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// open and paths.nul

/// Writes `open/report.md` and `open/notes.txt`.
fn open(folder: &Path) -> Result<()> {
    make_folder(folder)?;
    write_file(
        &folder.join("report.md"),
        REPORT_MD.as_bytes(),
        utc(10, 14, 16, 0),
    )?;
    write_file(
        &folder.join("notes.txt"),
        b"Example notes to edit with F4.\n",
        utc(10, 14, 16, 5),
    )
}

/// The bytes of `paths.nul`: each piped path, ended by a NUL.
pub fn paths_nul(demo: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    for parts in PIPED {
        let path = parts
            .iter()
            .filter(|part| !part.is_empty())
            .fold(demo.to_path_buf(), |path, part| path.join(part));
        out.extend_from_slice(path.display().to_string().as_bytes());
        out.push(0);
    }
    out
}

// ---------------------------------------------------------------------
// the whole alpha 5 part

/// Writes every alpha 5 fixture into the demo folder.
pub fn build(demo: &Path) -> Result<Made5> {
    rename(&demo.join("rename"))?;
    sync(&demo.join("sync"))?;
    let linked = dups(&demo.join("dups"))?;
    flat(&demo.join("flat"))?;
    state(&demo.join("state"), demo)?;
    open(&demo.join("open"))?;
    write_file(&demo.join("paths.nul"), &paths_nul(demo), taken())?;
    Ok(Made5 { linked })
}

/// What a fixture's line adds after its size, if anything.
pub fn note(name: &str, made: &Made5) -> Option<&'static str> {
    match name {
        "dups" if made.linked => Some("linked/one.txt and two.txt are one file, a hard link"),
        "dups" => Some("no hard link: this file system made none"),
        "state" => Some("copy its saved folder into the state folder beside diro"),
        "paths.nul" => Some("three paths, each ended by a NUL"),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
