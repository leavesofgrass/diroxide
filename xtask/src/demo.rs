//! `cargo xtask demo PATH`: the owner's demo set for the alpha.2 check
//! (research/alpha2-plan.md part six), written into a new or empty folder
//! on drive D: with plain names only:
//!
//! - `course.zip`: folders `week1` and `week2` with three text files
//!   each, plus `notes.txt` and `README.md`, ten entries in all;
//! - `backup.7z`, solid; `logs.tar.xz`;
//! - `locked.zip`, AES-256, with a password made up when the command runs
//!   and said once on its output, never kept anywhere;
//! - `hostile.zip`, whose first member's name would leave the archive,
//!   built in memory, so the name never touches the disk;
//! - `big.bin` (60 MB, binary), `main.rs`, `report.md` (twelve headings
//!   and a table), `notes.org`, `big.log` (over 600 KiB);
//! - an empty folder `out`.
//!
//! The archives are written with diroxide's own writers, through `diro
//! archive pack` (the `diro` binary is built first, with this task's
//! profile), except `locked.zip`, which the zip crate writes directly
//! because `diro` takes a password only from its terminal, and
//! `hostile.zip`, which no writer of diroxide's will make. Then each is
//! read back: `diro archive test`, the zip crate with the password, and
//! the oracle tools found on PATH (unzip, xz, tar, and 7z in the
//! container). The command lists what it made, one line each with the
//! size.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use diroxide_core::format::human_size;
use diroxide_core::{Timestamp, UtcOffset};

use crate::bench::{diro_binary, log_file, run_diro};
use crate::sample;

/// What the demo folder holds, in the order the command lists it.
pub const DEMO_NAMES: [&str; 11] = [
    "course.zip",
    "backup.7z",
    "logs.tar.xz",
    "locked.zip",
    "hostile.zip",
    "big.bin",
    "main.rs",
    "report.md",
    "notes.org",
    "big.log",
    "out",
];

/// The size of `big.bin`: about 60 MB, four parts at the email preset.
pub const BIG_BIN_BYTES: u64 = 60_000_000;

/// The size of `big.log`: over 600 KiB, past the viewer's whole-file cap.
pub const BIG_LOG_BYTES: u64 = 700_000;

const USAGE: &str =
    "Usage: cargo xtask demo PATH, a new or empty folder on drive D:, such as D:\\demo";

/// Whether a name is plain: letters, digits, `-`, `_` and `.`, not only
/// dots, and not ending in a dot (the owner's rule 1a).
pub fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Checks the folder named for the demo set, without touching the disk:
/// on Windows a full path on drive D: and nowhere else; elsewhere a full
/// path from `/`; every name in it plain, with no `.` or `..`. The error
/// says why, in words.
pub fn checked_folder(text: &str) -> Result<PathBuf> {
    let rest = if cfg!(windows) {
        let bytes = text.as_bytes();
        let on_d = bytes.len() >= 3
            && bytes[0].eq_ignore_ascii_case(&b'D')
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/');
        if !on_d {
            bail!(
                "{text} is not on drive D:; the demo set is written on D: only, such as D:\\demo"
            );
        }
        &text[3..]
    } else {
        match text.strip_prefix('/') {
            Some(rest) => rest,
            None => bail!("{text} is not a full path; start it with /"),
        }
    };
    let names: Vec<&str> = rest
        .split(['/', '\\'])
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        bail!("{text} is the top of a drive; name a folder below it, such as D:\\demo");
    }
    if let Some(odd) = names.iter().find(|name| !plain(name)) {
        bail!("{odd} is not a plain name; use letters, digits, -, _ and . only, as in D:\\demo");
    }
    let path = PathBuf::from(text);
    if path
        .components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        bail!("{text} has . or .. in it; write the folder's full path");
    }
    Ok(path)
}

/// A system time from a civil date and time in UTC.
fn utc(day: u8, hour: u8, minute: u8) -> SystemTime {
    let seconds = Timestamp::from_civil(2026, 10, day, hour, minute, 0, UtcOffset::UTC)
        .map_or(0, Timestamp::unix_seconds);
    UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).unwrap_or(0))
}

/// Writes a text file with a fixed modified time.
fn write_text(path: &Path, text: &str, modified: SystemTime) -> Result<()> {
    fs::write(path, text).with_context(|| format!("{} could not be written", path.display()))?;
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(modified))
        .with_context(|| format!("the time of {} could not be set", path.display()))
}

/// Makes a folder and its parents.
fn make_folder(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("{} could not be made", path.display()))
}

/// A password made up now: three words and four digits from the system's
/// random keys, easy to type and to hear; said once, never kept.
pub fn demo_password() -> String {
    use std::hash::{BuildHasher, Hasher};
    const WORDS: [&str; 32] = [
        "amber", "badger", "cedar", "crow", "delta", "ember", "ferry", "fern", "glacier", "harbor",
        "heron", "island", "juniper", "kelp", "lantern", "maple", "meadow", "nimbus", "otter",
        "pebble", "quartz", "raccoon", "river", "salmon", "spruce", "summit", "thistle", "tram",
        "umber", "willow", "yarrow", "zephyr",
    ];
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.write_u128(nanos);
    hasher.write_u32(std::process::id());
    let mut bits = hasher.finish();
    let mut word = || {
        let w = WORDS[(bits % 32) as usize];
        bits /= 32;
        w
    };
    let (a, b, c) = (word(), word(), word());
    format!("{a}-{b}-{c}-{:04}", bits % 10_000)
}

/// A zip of stored members built in memory, the first named so it would
/// leave the archive.
pub fn hostile_zip() -> Vec<u8> {
    let members: [(&str, &[u8]); 3] = [
        ("../escape.txt", b"This file tried to leave the archive.\n"),
        ("safe.txt", b"This one stays where it belongs.\n"),
        (
            "readme.txt",
            b"hostile.zip: one name here would leave the archive.\n",
        ),
    ];
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let offset = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let crc = crc32fast_free(data);
        let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);
        let header = |signature: u32, in_central: bool| {
            let mut h = signature.to_le_bytes().to_vec();
            if in_central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&20u16.to_le_bytes());
            // UTF-8 names; stored; 2:05 PM on Friday, October 16, 2026.
            h.extend_from_slice(&0x0800u16.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(&0x70A0u16.to_le_bytes());
            h.extend_from_slice(&0x5D50u16.to_le_bytes());
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&name_len.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if in_central {
                // The comment's length, the disk and the attributes.
                h.extend_from_slice(&[0; 6]);
                h.extend_from_slice(&0u32.to_le_bytes());
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4B50, false));
        out.extend_from_slice(data);
        central.extend(header(0x0201_4B50, true));
    }
    let start = u32::try_from(out.len()).unwrap_or(u32::MAX);
    let length = u32::try_from(central.len()).unwrap_or(u32::MAX);
    out.extend(central);
    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The CRC-32 of zip and gzip, bit by bit: a few lines, no crate.
fn crc32fast_free(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The text files of `course.zip`, by path.
const COURSE: [(&str, &str); 8] = [
    (
        "week1/a.txt",
        "Week 1, lesson A: the two panels.\nThe active panel is the source; the other is the target.\n",
    ),
    (
        "week1/b.txt",
        "Week 1, lesson B: moving around.\nLeft goes to the parent, Right enters a folder or an archive.\n",
    ),
    (
        "week1/c.txt",
        "Week 1, lesson C: marking.\nSpace marks a row and moves on; Ctrl+A marks all.\n",
    ),
    (
        "week2/d.txt",
        "Week 2, lesson D: copying.\nF5 copies the marked entries to the other panel.\n",
    ),
    (
        "week2/e.txt",
        "Week 2, lesson E: archives.\nEnter opens a zip like a folder; nothing is unpacked to browse.\n",
    ),
    (
        "week2/f.txt",
        "Week 2, lesson F: undo.\nCtrl+Z undoes the last job; a copy goes to the trash.\n",
    ),
    (
        "notes.txt",
        "Notes for the course\n\
         \n\
         A crow sat on the tram cable this morning.\n\
         The second crow came at nine, and a third crow at ten.\n\
         Jimothy the raccoon was not seen, which is suspicious.\n\
         Remember: the crow count goes in the weekly report.\n\
         Portland weather: rain, then more rain, then a crow.\n",
    ),
    (
        "README.md",
        "# The course\n\nTwo weeks of short lessons, three a week, plus notes.\n\n## Weeks\n\n- week1: panels, moving, marking\n- week2: copying, archives, undo\n",
    ),
];

/// The files of `backup.7z`, by path.
const BACKUP: [(&str, &str); 5] = [
    (
        "budget.csv",
        "item,amount\nbirdseed,12.50\nraccoon-proof lid,24.00\numbrella,18.00\n",
    ),
    (
        "todo.txt",
        "Fix the gate.\nCount the crows.\nBuy a bigger umbrella.\n",
    ),
    (
        "old/draft1.txt",
        "First draft of the letter to the neighborhood association.\n",
    ),
    (
        "old/draft2.txt",
        "Second draft, with fewer complaints about the raccoon.\n",
    ),
    (
        "photos.txt",
        "A list of photos: crow1.jpg, crow2.jpg, tram.jpg.\n",
    ),
];

/// The files of `locked.zip`, by path.
const LOCKED: [(&str, &str); 3] = [
    (
        "plans.txt",
        "The plans: feed the crows at seven, never at night.\n",
    ),
    (
        "recipe.txt",
        "Marionberry pie: berries, sugar, butter, and patience.\n",
    ),
    (
        "notes/crows.txt",
        "Crows remember faces. Be kind to them.\n",
    ),
];

/// `main.rs`: a small program with the shapes the viewer's outline and
/// token kinds name.
const MAIN_RS: &str = r#"//! A small program for the viewer: functions, a type, a module.

use std::env;

/// How the greeting is said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Plain,
    Loud,
}

/// What the command line asked for.
struct Config {
    name: String,
    mode: Mode,
}

impl Config {
    /// Reads the arguments: a name, then `--loud` if wanted.
    fn new(args: &[String]) -> Self {
        let name = args.get(1).cloned().unwrap_or_else(|| "crow".to_owned());
        let mode = if args.iter().any(|a| a == "--loud") {
            Mode::Loud
        } else {
            Mode::Plain
        };
        Self { name, mode }
    }

    /// The greeting, in words.
    fn describe(&self) -> String {
        greeting::hello(&self.name, self.mode == Mode::Loud)
    }
}

mod greeting {
    /// Says hello to someone.
    pub fn hello(name: &str, loud: bool) -> String {
        let text = format!("Hello, {name}.");
        if loud { text.to_uppercase() } else { text }
    }
}

fn parse_args() -> Config {
    let args: Vec<String> = env::args().collect();
    Config::new(&args)
}

fn main() {
    let config = parse_args();
    println!("{}", config.describe());
}
"#;

/// `report.md`: twelve headings and a table.
const REPORT_MD: &str = "# Quarterly report

A short report for the quick view: twelve headings and one table.

## Summary

The crow count rose again this quarter.

## Counts

| Month | Crows | Raccoons |
| --- | --- | --- |
| July | 14 | 1 |
| August | 19 | 0 |
| September | 23 | 1 |

## Places

### The tram

Most crows were seen near the upper tram terminal.

### The river

Fewer crows by the river, more gulls.

### The garden

One raccoon, named Jimothy by the neighbors.

## Weather

Rain on most days.

## Plans

### Next quarter

Count again, with a second observer.

### Next year

A crow-friendly feeder, raccoon-proof.

## Thanks

To the neighbors who counted.
";

/// `notes.org`: an Org file for carta's reader.
const NOTES_ORG: &str = "#+TITLE: Field notes

* Crows
  Seen on the tram cable, three at a time.
** TODO Count them again on Saturday
** DONE Ask the neighbors

* Jimothy
  The raccoon of the garden, short in the spine, long on charm.
  - Visits after dark
  - Prefers the green bin

* Portland
  | Day      | Weather    |
  |----------+------------|
  | Monday   | rain       |
  | Tuesday  | more rain  |
  | Saturday | sun, maybe |
";

/// Writes `files` under `root`, every file and folder made, each file's
/// time fixed.
fn stage(root: &Path, files: &[(&str, &str)]) -> Result<()> {
    for (n, (path, text)) in files.iter().enumerate() {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            make_folder(parent)?;
        }
        write_text(&target, text, utc(16, 9, u8::try_from(n % 60).unwrap_or(0)))?;
    }
    Ok(())
}

/// The top-level names of staged files, each once, in order.
fn tops<'a>(files: &[(&'a str, &str)]) -> Vec<&'a str> {
    let mut found: Vec<&str> = Vec::new();
    for (path, _) in files {
        let top = path.split('/').next().unwrap_or(path);
        if !found.contains(&top) {
            found.push(top);
        }
    }
    found
}

/// Packs the staged files of `staged` into `archive` with `diro archive
/// pack`; returns what it said.
fn pack(diro: &Path, staged: &Path, archive: &Path, files: &[(&str, &str)]) -> Result<String> {
    let target = archive
        .to_str()
        .with_context(|| format!("{} is not valid Unicode", archive.display()))?;
    let mut args = vec!["archive", "pack", target];
    args.extend(tops(files));
    let said = run_diro(diro, staged, &args, b"")?;
    Ok(String::from_utf8_lossy(&said).into_owned())
}

/// Writes `locked.zip` with the zip crate, every file AES-256 under the
/// password, folders as their own entries.
fn write_locked(archive: &Path, staged: &Path, password: &str) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let file = fs::File::create(archive)
        .with_context(|| format!("{} could not be made", archive.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let base = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(
            zip::DateTime::from_date_and_time(2026, 10, 16, 14, 5, 0).unwrap_or_default(),
        );
    let mut folders: Vec<String> = Vec::new();
    for (path, _) in LOCKED {
        if let Some((folder, _)) = path.rsplit_once('/')
            && !folders.iter().any(|f| f == folder)
        {
            folders.push(folder.to_owned());
            zip.add_directory(folder, base)
                .with_context(|| format!("{folder} could not be added"))?;
        }
        let bytes =
            fs::read(staged.join(path)).with_context(|| format!("{path} could not be read"))?;
        zip.start_file(
            path,
            base.with_aes_encryption(zip::AesMode::Aes256, password),
        )
        .with_context(|| format!("{path} could not be added"))?;
        zip.write_all(&bytes)
            .with_context(|| format!("{path} could not be written"))?;
    }
    zip.finish()
        .with_context(|| format!("{} could not be finished", archive.display()))?;
    Ok(())
}

/// Reads every member of `locked.zip` back with the password: true when
/// every one opens and reads whole.
fn locked_opens(archive: &Path, password: &str) -> Result<bool> {
    let file = fs::File::open(archive)
        .with_context(|| format!("{} could not be read", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("{} is not a zip", archive.display()))?;
    for n in 0..zip.len() {
        let mut member = match zip.by_index_decrypt(n, password.as_bytes()) {
            Ok(member) => member,
            Err(_) => return Ok(false),
        };
        if member.is_dir() {
            continue;
        }
        if !member.encrypted() {
            return Ok(false);
        }
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut member, &mut bytes).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Runs a program with arguments in `folder`; its exit status and output,
/// or `None` when it could not start.
fn run_tool(tool: &str, args: &[&str], folder: &Path) -> Option<(bool, String)> {
    let output = Command::new(tool)
        .current_dir(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some((output.status.success(), text))
}

/// The first line of a tool's output, for a problem's sentence.
fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

/// Checks the archives with the oracle tools found on PATH: unzip, xz,
/// tar (bsdtar reads the zip and the 7z too), and 7z. Returns the lines
/// and whether every check passed.
fn oracle_checks(folder: &Path) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut all_good = true;
    let mut missing = Vec::new();
    let tar_version = run_tool("tar", &["--version"], folder);
    let bsdtar = tar_version
        .as_ref()
        .is_some_and(|(_, text)| text.contains("bsdtar"));
    let checks: Vec<(&str, Vec<&str>, &str)> = {
        let mut checks = vec![
            ("unzip", vec!["-tq", "course.zip"], "course.zip"),
            ("xz", vec!["-t", "logs.tar.xz"], "logs.tar.xz"),
            ("tar", vec!["-tf", "logs.tar.xz"], "logs.tar.xz"),
        ];
        if bsdtar {
            checks.push(("tar", vec!["-tf", "course.zip"], "course.zip"));
            checks.push(("tar", vec!["-tf", "backup.7z"], "backup.7z"));
        }
        for archive in ["course.zip", "backup.7z", "logs.tar.xz"] {
            checks.push(("7z", vec!["t", archive], archive));
        }
        checks
    };
    for (tool, args, archive) in checks {
        if missing.contains(&tool) {
            continue;
        }
        let present = match tool {
            "tar" => tar_version.is_some(),
            "7z" => run_tool("7z", &["i"], folder).is_some(),
            other => {
                run_tool(other, &["--version"], folder).is_some()
                    || run_tool(other, &["-v"], folder).is_some()
            }
        };
        if !present {
            missing.push(tool);
            continue;
        }
        match run_tool(tool, &args, folder) {
            Some((true, _)) => lines.push(format!("{archive} reads whole with {tool}.")),
            Some((false, text)) => {
                all_good = false;
                lines.push(format!(
                    "{archive} failed with {tool}: {}",
                    first_line(&text)
                ));
            }
            None => missing.push(tool),
        }
    }
    if !missing.is_empty() {
        lines.push(format!(
            "Not on PATH, so not checked with: {}.",
            missing.join(", ")
        ));
    }
    (lines, all_good)
}

/// What the demo made: the lines to say, and whether every check passed.
#[derive(Debug)]
pub struct Made {
    /// What was made, one line each with the size; the password; the
    /// checks.
    pub lines: Vec<String>,
    /// Whether every check passed.
    pub all_good: bool,
}

/// Writes the demo set into `folder` (which must exist and be empty),
/// staging the archives' contents in `staging`, packing with `diro`, and
/// locking `locked.zip` with `password`. Then reads every archive back.
pub fn build(folder: &Path, staging: &Path, diro: &Path, password: &str) -> Result<Made> {
    let mut checks = Vec::new();
    let mut all_good = true;
    // The archives, from their staged contents.
    let course = staging.join("course");
    stage(&course, &COURSE)?;
    pack(diro, &course, &folder.join("course.zip"), &COURSE)?;
    let backup = staging.join("backup");
    stage(&backup, &BACKUP)?;
    let said = pack(diro, &backup, &folder.join("backup.7z"), &BACKUP)?;
    if !said.contains("Solid: on") {
        all_good = false;
        checks.push("backup.7z was not packed solid.".to_owned());
    }
    let logs = staging.join("logs");
    make_folder(&logs)?;
    log_file(&logs.join("app.log"), 150_000)?;
    log_file(&logs.join("access.log"), 120_000)?;
    write_text(
        &logs.join("error.log"),
        "2026-10-16 14:05:00 ERROR the crow feeder is empty\n",
        utc(16, 14, 5),
    )?;
    let log_files: [(&str, &str); 3] = [("app.log", ""), ("access.log", ""), ("error.log", "")];
    pack(diro, &logs, &folder.join("logs.tar.xz"), &log_files)?;
    let locked = staging.join("locked");
    stage(&locked, &LOCKED)?;
    write_locked(&folder.join("locked.zip"), &locked, password)?;
    fs::write(folder.join("hostile.zip"), hostile_zip())
        .context("hostile.zip could not be written")?;
    // The plain files.
    sample::big_file(&folder.join("big.bin"), BIG_BIN_BYTES, Some(utc(16, 14, 5)))?;
    write_text(&folder.join("main.rs"), MAIN_RS, utc(15, 10, 30))?;
    write_text(&folder.join("report.md"), REPORT_MD, utc(14, 16, 0))?;
    write_text(&folder.join("notes.org"), NOTES_ORG, utc(13, 8, 15))?;
    log_file(&folder.join("big.log"), BIG_LOG_BYTES)?;
    make_folder(&folder.join("out"))?;

    // What was made, with the sizes.
    let mut lines = Vec::new();
    for name in DEMO_NAMES {
        let path = folder.join(name);
        let meta = fs::metadata(&path).with_context(|| format!("{name} was not made"))?;
        lines.push(if meta.is_dir() {
            format!("{name}, empty folder")
        } else {
            format!("{name}, {}", human_size(meta.len()))
        });
    }
    lines.push(format!("Password for locked.zip: {password}"));

    // Read back with diroxide's own reader.
    let course_count = run_diro(diro, folder, &["archive", "list", "-0", "course.zip"], b"")?
        .iter()
        .filter(|b| **b == 0)
        .count();
    if course_count != 10 {
        all_good = false;
        checks.push(format!("course.zip has {course_count} entries, not 10."));
    }
    for archive in ["course.zip", "backup.7z", "logs.tar.xz"] {
        match run_diro(diro, folder, &["archive", "test", archive], b"") {
            Ok(said) => checks.push(format!(
                "{archive}: {}",
                String::from_utf8_lossy(&said).trim()
            )),
            Err(error) => {
                all_good = false;
                checks.push(format!("{archive} failed its test: {error:#}"));
            }
        }
    }
    if locked_opens(&folder.join("locked.zip"), password)? {
        checks.push("locked.zip: every member opens with the password.".to_owned());
    } else {
        all_good = false;
        checks.push("locked.zip did not open with its password.".to_owned());
    }
    let hostile = Command::new(diro)
        .current_dir(folder)
        .args(["archive", "list", "hostile.zip"])
        .stdin(Stdio::null())
        .output()
        .context("diro could not be started")?;
    let hostile_said = String::from_utf8_lossy(&hostile.stdout);
    match hostile_said
        .lines()
        .find(|l| l.contains("name leaves the archive"))
    {
        Some(skipped) if hostile.status.code() == Some(3) => {
            checks.push(format!("hostile.zip: {skipped}"));
        }
        _ => {
            all_good = false;
            checks.push(format!(
                "hostile.zip did not refuse its escaping name: {}",
                first_line(&hostile_said)
            ));
        }
    }
    let (oracle, oracle_good) = oracle_checks(folder);
    checks.extend(oracle);
    lines.extend(checks);
    Ok(Made {
        lines,
        all_good: all_good && oracle_good,
    })
}

/// Whether a folder has nothing in it, or does not exist yet.
fn is_empty_or_new(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if !path.is_dir() {
        bail!("{} is a file; name a folder", path.display());
    }
    Ok(fs::read_dir(path)
        .with_context(|| format!("{} could not be read", path.display()))?
        .next()
        .is_none())
}

/// `cargo xtask demo PATH`.
pub fn run(root: &Path, args: &[&str]) -> Result<bool> {
    let [text] = args else {
        eprintln!("{USAGE}");
        return Ok(false);
    };
    let folder = checked_folder(text)?;
    let parent = folder
        .parent()
        .context("the folder has no parent; name one below the top of D:")?;
    if !parent.is_dir() {
        bail!(
            "{} does not exist; make it first, or choose a folder that does",
            parent.display()
        );
    }
    if !is_empty_or_new(&folder)? {
        bail!(
            "{} has things in it; name a new or empty folder, so nothing there is replaced",
            folder.display()
        );
    }
    println!("demo: building diro.");
    let diro = diro_binary(root)?;
    make_folder(&folder)?;
    let work = crate::bench::target_folder(root).join("demo");
    make_folder(&work)?;
    let staging = tempfile::Builder::new()
        .prefix("staging-")
        .tempdir_in(&work)
        .with_context(|| format!("no staging folder could be made in {}", work.display()))?;
    println!("demo: writing the demo set in {}.", folder.display());
    let made = build(&folder, staging.path(), &diro, &demo_password())?;
    staging
        .close()
        .context("the staging folder could not be removed")?;
    for line in &made.lines {
        println!("{line}");
    }
    if made.all_good {
        println!(
            "demo: Pass. {} items in {}.",
            DEMO_NAMES.len(),
            folder.display()
        );
    } else {
        println!("demo: Fail. A check above found a problem.");
    }
    Ok(made.all_good)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_and_the_folder_rules() {
        for name in ["demo", "demo-2", "a_b.c", "alpha2"] {
            assert!(plain(name), "{name}");
        }
        for name in ["", "a b", "a(2)", "x.", "$x", "a:b", "caf\u{e9}"] {
            assert!(!plain(name), "{name}");
        }
        if cfg!(windows) {
            assert!(checked_folder("D:\\demo").is_ok());
            assert!(checked_folder("d:/dirocheck/demo-2").is_ok());
            for (text, why) in [
                ("C:\\demo", "not on drive D:"),
                ("E:\\demo", "not on drive D:"),
                ("demo", "not on drive D:"),
                ("\\\\server\\share\\demo", "not on drive D:"),
                ("D:demo", "not on drive D:"),
                ("D:\\", "the top of a drive"),
                ("D:\\my demo", "not a plain name"),
                ("D:\\a\\..\\demo", "not a plain name"),
                ("D:\\demo.", "not a plain name"),
            ] {
                let error = checked_folder(text).unwrap_err().to_string();
                assert!(error.contains(why), "{text}: {error}");
            }
        } else {
            assert!(checked_folder("/srv/demo").is_ok());
            for (text, why) in [
                ("demo", "not a full path"),
                ("/", "the top of a drive"),
                ("/srv/my demo", "not a plain name"),
                ("/srv/../demo", "not a plain name"),
            ] {
                let error = checked_folder(text).unwrap_err().to_string();
                assert!(error.contains(why), "{text}: {error}");
            }
        }
    }

    #[test]
    fn the_password_is_words_and_digits_and_new_each_time() {
        let one = demo_password();
        let parts: Vec<&str> = one.split('-').collect();
        assert_eq!(parts.len(), 4, "{one}");
        assert!(parts[3].len() == 4 && parts[3].chars().all(|c| c.is_ascii_digit()));
        assert!(plain(&one));
        let many: std::collections::HashSet<String> = (0..20).map(|_| demo_password()).collect();
        assert!(many.len() > 15, "{many:?}");
    }

    #[test]
    fn the_texts_have_what_the_check_needs() {
        let headings = REPORT_MD.lines().filter(|l| l.starts_with('#')).count();
        assert_eq!(headings, 12);
        assert!(REPORT_MD.contains("| Month | Crows | Raccoons |"));
        assert!(MAIN_RS.contains("fn parse_args"));
        assert!(NOTES_ORG.starts_with("#+TITLE"));
        let crows = COURSE[6].1.matches("crow").count();
        assert!(crows >= 4, "notes.txt holds the word to search for");
        assert_eq!(tops(&COURSE), ["week1", "week2", "notes.txt", "README.md"]);
        let entries = COURSE.len() + 2;
        assert_eq!(entries, 10, "eight files and two folders");
    }

    #[test]
    fn the_demo_set_holds_every_file_part_six_names() {
        let root = crate::bench::workspace_root().unwrap();
        let diro = diro_binary(&root).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("demo");
        fs::create_dir(&folder).unwrap();
        let staging = tempfile::tempdir().unwrap();
        let made = build(&folder, staging.path(), &diro, "test-only-password-1").unwrap();
        assert!(made.all_good, "{:#?}", made.lines);
        let mut names: Vec<String> = fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let mut expected: Vec<String> = DEMO_NAMES.iter().map(|n| (*n).to_owned()).collect();
        expected.sort();
        assert_eq!(names, expected, "nothing more, nothing less");
        for (line, name) in made.lines.iter().zip(DEMO_NAMES) {
            assert!(line.starts_with(&format!("{name}, ")), "{line}");
        }
        assert_eq!(made.lines[10], "out, empty folder");
        assert_eq!(
            made.lines[11],
            "Password for locked.zip: test-only-password-1"
        );
        assert!(
            made.lines
                .iter()
                .any(|l| l == "course.zip: Archive is sound: 10 members.")
        );
        assert!(
            made.lines
                .iter()
                .any(|l| l == "hostile.zip: Skipped ../escape.txt: name leaves the archive."),
            "{:#?}",
            made.lines
        );
        assert_eq!(
            fs::metadata(folder.join("big.bin")).unwrap().len(),
            BIG_BIN_BYTES
        );
        assert!(fs::metadata(folder.join("big.log")).unwrap().len() > 600 * 1024);
        assert_eq!(fs::read_dir(folder.join("out")).unwrap().count(), 0);
        assert!(locked_opens(&folder.join("locked.zip"), "test-only-password-1").unwrap());
        assert!(!locked_opens(&folder.join("locked.zip"), "wrong").unwrap());
    }
}
