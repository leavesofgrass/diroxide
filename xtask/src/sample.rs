//! Trees on disk, written only under a folder the caller names:
//!
//! - `cargo xtask sample-folder PATH [--big-mb N]`: the owner's check
//!   folder for the ten-minute check (research/alpha1-plan.md part six):
//!   thirty-one items at the top, `Backup` with an older, smaller
//!   `report.pdf` for the conflict, `Names` with an accented, a Japanese
//!   and an emoji name, `Photos` with twelve pictures, and a large
//!   `big.bin` for progress. Plain names only: letters, digits, `-`, `_`
//!   and `.`, with accented letters, Japanese and one emoji where the
//!   check needs them; no hostile name is ever written to disk.
//! - `cargo xtask search-tree PATH [--small]`: the 1 GB, 100,000-file
//!   tree of alpha.3's search spike, with a known number of content and
//!   name hits, from `bench/search-tree.toml`.
//! - The bench's own trees (a flat folder, one large file), written under
//!   the target folder.
//!
//! Every tree is the same on every run: fixed names, contents and
//! modified times from a seeded generator. Nothing here deletes anything:
//! a second run over a sample folder rewrites the sample's own files and
//! lists, in words, what else it found there.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use diroxide_core::{Timestamp, UtcOffset};
use serde::Deserialize;

/// The default size of `big.bin`: 2 GB, so a copy to a slow drive lasts
/// long enough for progress to be heard.
pub const DEFAULT_BIG_MB: u64 = 2000;

/// A small, fast, seeded generator (SplitMix64), so every tree is the
/// same on every run and on every platform.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n` (0 when `n` is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    /// Fills a buffer with random bytes.
    pub fn fill(&mut self, buffer: &mut [u8]) {
        for chunk in buffer.chunks_mut(8) {
            let bytes = self.next().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

/// Checks a folder named for generated files and returns it: a full path
/// with no `.` or `..`, below a drive's or the file system's top, whose
/// parent exists; on Windows a drive-letter path that is not on drive C
/// or the system drive (the owner's rule: nothing is written to drive C).
pub fn checked_target(text: &str) -> Result<PathBuf> {
    let path = PathBuf::from(text);
    if cfg!(windows) {
        let bytes = text.as_bytes();
        let drive_path = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/');
        if !drive_path {
            bail!(
                "{text} is not a full path that starts with a drive letter, such as D:\\dirocheck"
            );
        }
        let letter = bytes[0].to_ascii_uppercase();
        let system = std::env::var("SystemDrive")
            .ok()
            .and_then(|drive| drive.bytes().next())
            .map(|b| b.to_ascii_uppercase());
        if letter == b'C' || Some(letter) == system {
            bail!(
                "{text} is on drive {}, where nothing is written; choose a folder on another drive, such as D:",
                letter as char
            );
        }
    } else if !text.starts_with('/') {
        bail!("{text} is not a full path; start it with /");
    }
    if path
        .components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        bail!("{text} has . or .. in it; write the folder's full path");
    }
    let Some(parent) = path.parent().filter(|_| path.file_name().is_some()) else {
        bail!("{text} is the top of a drive; name a folder below it");
    };
    if !parent.is_dir() {
        bail!(
            "{} does not exist; make it first, or choose a folder that does",
            parent.display()
        );
    }
    if path.exists() && !path.is_dir() {
        bail!("{text} is a file; name a folder");
    }
    Ok(path)
}

/// Whether a folder has nothing in it (or does not exist yet).
fn is_empty_or_new(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    Ok(fs::read_dir(path)
        .with_context(|| format!("{} could not be read", path.display()))?
        .next()
        .is_none())
}

/// A system time from a civil date and time in UTC.
fn utc(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> SystemTime {
    let seconds = Timestamp::from_civil(year, month, day, hour, minute, 0, UtcOffset::UTC)
        .map_or(0, Timestamp::unix_seconds);
    UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).unwrap_or(0))
}

/// Writes a file with these bytes and this modified time, replacing what
/// was there.
fn write_file(path: &Path, bytes: &[u8], modified: SystemTime) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("{} could not be written", path.display()))?;
    set_modified(path, modified)
}

fn set_modified(path: &Path, modified: SystemTime) -> Result<()> {
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(modified))
        .with_context(|| format!("the time of {} could not be set", path.display()))
}

/// Writes a large file of `bytes` bytes in 1 MiB blocks: one seeded random
/// block with each block's number stamped at its start, so no two blocks
/// are alike and no file system can fold them together.
pub fn big_file(path: &Path, bytes: u64, modified: Option<SystemTime>) -> Result<()> {
    const BLOCK: usize = 1024 * 1024;
    let mut block = vec![0u8; BLOCK];
    Rng::new(0xB16_F11E).fill(&mut block);
    let file =
        File::create(path).with_context(|| format!("{} could not be made", path.display()))?;
    let mut out = BufWriter::with_capacity(BLOCK, file);
    let mut left = bytes;
    let mut number: u64 = 0;
    while left > 0 {
        block[..8].copy_from_slice(&number.to_le_bytes());
        let take = usize::try_from(left.min(BLOCK as u64)).unwrap_or(BLOCK);
        out.write_all(&block[..take])
            .with_context(|| format!("{} could not be written", path.display()))?;
        left -= take as u64;
        number += 1;
    }
    out.flush()
        .with_context(|| format!("{} could not be written", path.display()))?;
    drop(out);
    if let Some(modified) = modified {
        set_modified(path, modified)?;
    }
    Ok(())
}

/// Makes `count` empty files in `folder` (which must exist), named
/// `entry-000001.txt` and on, and returns their names: the bench's
/// listing.
pub fn flat_folder(folder: &Path, count: usize) -> Result<Vec<String>> {
    let mut names = Vec::with_capacity(count);
    for n in 1..=count {
        let name = format!("entry-{n:06}.txt");
        let path = folder.join(&name);
        File::create(&path).with_context(|| format!("{} could not be made", path.display()))?;
        names.push(name);
    }
    Ok(names)
}

// ---------------------------------------------------------------------
// Small valid files: a PDF, a PNG and an empty zip, so the system's own
// programs open what the owner opens during the check.

/// A one-page PDF that shows `lines` of text, with a correct cross
/// reference table.
fn pdf(lines: &[&str]) -> Vec<u8> {
    let mut text = String::from("BT /F1 14 Tf 72 720 Td 18 TL\n");
    for line in lines {
        let clean: String = line
            .chars()
            .filter(|c| c.is_ascii() && !matches!(c, '(' | ')' | '\\'))
            .collect();
        text.push_str(&format!("({clean}) Tj T*\n"));
    }
    text.push_str("ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{text}endstream", text.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn crc32(bytes: &[u8]) -> u32 {
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

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    let mut checked = kind.to_vec();
    checked.extend_from_slice(data);
    out.extend_from_slice(&checked);
    out.extend_from_slice(&crc32(&checked).to_be_bytes());
}

/// An RGB picture of `width` by `height`, a gradient from `color`, in a
/// valid PNG with stored (uncompressed) deflate blocks.
fn png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let shade = u8::try_from((x + y) * 255 / (width + height).max(1)).unwrap_or(255);
            raw.extend_from_slice(&[
                color[0].wrapping_add(shade / 2),
                color[1].wrapping_add(shade / 3),
                color[2].wrapping_add(shade / 4),
            ]);
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = u16::try_from(block.len()).unwrap_or(u16::MAX);
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(&mut out, b"IHDR", &header);
    png_chunk(&mut out, b"IDAT", &zlib);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

/// An empty zip archive: the end-of-central-directory record alone.
fn empty_zip() -> Vec<u8> {
    let mut out = b"PK\x05\x06".to_vec();
    out.extend_from_slice(&[0; 18]);
    out
}

// ---------------------------------------------------------------------
// The owner's check folder.

/// What one file of the check folder holds.
enum Content {
    Text(String),
    Bytes(Vec<u8>),
    Big(u64),
}

/// One file of the check folder: its path below the folder (with `/`),
/// what it holds, and its modified time.
struct SampleFile {
    path: String,
    content: Content,
    modified: SystemTime,
}

/// The folders of the check folder.
pub const SAMPLE_FOLDERS: [&str; 3] = ["Backup", "Names", "Photos"];

/// The top-level files other than `report.pdf` and `big.bin`: name, and
/// the line it holds.
const TOP_FILES: [(&str, &str); 26] = [
    (
        "agenda.md",
        "# Agenda\n\n- Linux User Net, Monday 8:10 PM\n- HEART drill\n",
    ),
    (
        "bike-route.gpx",
        "<?xml version=\"1.0\"?>\n<gpx version=\"1.1\" creator=\"diroxide sample\"></gpx>\n",
    ),
    (
        "budget-2026.csv",
        "month,amount\nJanuary,120\nFebruary,95\n",
    ),
    (
        "config-example.toml",
        "# An example setting\nlayout = \"side_by_side\"\n",
    ),
    (
        "crow-facts.txt",
        "Crows remember faces.\nA group of crows is a murder.\n",
    ),
    (
        "garden-plan.txt",
        "Tomatoes by the fence. Marionberries along the back.\n",
    ),
    ("hello.py", "print(\"Hello from the sample folder\")\n"),
    (
        "index.html",
        "<!doctype html>\n<title>Sample</title>\n<p>A sample page.</p>\n",
    ),
    (
        "Jimothy-sightings.txt",
        "Jimothy the raccoon, seen near the bins again.\n",
    ),
    (
        "letter-to-the-editor.txt",
        "Dear editor,\nThe tram was on time all week.\n",
    ),
    (
        "linux-net-links.md",
        "# Links\n\n- The kernel release notes\n",
    ),
    (
        "marionberry-pie-recipe.txt",
        "Four cups of marionberries, one cup of sugar.\n",
    ),
    (
        "meeting-notes.md",
        "# Meeting notes\n\nNothing decided yet.\n",
    ),
    (
        "net-script.md",
        "# Net script\n\nThis is the Linux User Net.\n",
    ),
    ("notes.txt", "A plain text note.\n"),
    ("old-photos.zip", ""),
    ("packing-list.txt", "Radio, battery, charger, water.\n"),
    ("playlist.m3u", "#EXTM3U\n"),
    ("quotes.txt", "Keep Portland weird.\n"),
    ("reading-list.md", "# Reading list\n\n- Academic Ableism\n"),
    ("repeater-list.csv", "name,frequency\nK7RPT,147.040\n"),
    ("shopping-list.txt", "Coffee, bread, apples.\n"),
    ("todo.txt", "Run the ten-minute check.\n"),
    (
        "tram-schedule.csv",
        "leaves,arrives\n7:00,7:04\n7:06,7:10\n",
    ),
    (
        "weather-log.csv",
        "day,weather\nMonday,rain\nTuesday,rain\n",
    ),
    (
        "zebra-crossing.txt",
        "The last name in the list, for End and letter jumps.\n",
    ),
];

/// The pictures in `Photos`, with their sizes in pixels.
const PHOTOS: [(&str, u32); 12] = [
    ("aerial-tram-at-dusk.png", 48),
    ("crow-on-the-wire.png", 32),
    ("crows-at-sunset.png", 40),
    ("food-carts.png", 24),
    ("forest-park-trail.png", 56),
    ("Jimothy-at-night.png", 16),
    ("marquam-hill-fog.png", 44),
    ("mount-hood.png", 64),
    ("rose-garden.png", 28),
    ("south-waterfront.png", 36),
    ("steel-bridge.png", 20),
    ("willamette-river.png", 52),
];

/// The names in `Names`: an accented name, a Japanese name and an emoji
/// name (a raccoon, a single code point, with no joiner or variation
/// selector, which are invisible characters).
const NAMES: [(&str, &str); 3] = [
    (
        "R\u{e9}sum\u{e9}.txt",
        "A r\u{e9}sum\u{e9}, with its accents.\n",
    ),
    (
        "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30e1}\u{30e2}.txt",
        "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30e1}\u{30e2}\n",
    ),
    ("Jimothy-\u{1f99d}.txt", "A name with an emoji.\n"),
];

fn sample_files(big_bytes: u64) -> Vec<SampleFile> {
    let mut files = Vec::new();
    for (i, (name, text)) in TOP_FILES.iter().enumerate() {
        let month = 1 + u8::try_from(i % 9).unwrap_or(0);
        let day = 1 + u8::try_from((i * 7) % 27).unwrap_or(0);
        let content = if *name == "old-photos.zip" {
            Content::Bytes(empty_zip())
        } else {
            Content::Text((*text).to_owned())
        };
        files.push(SampleFile {
            path: (*name).to_owned(),
            content,
            modified: utc(2026, month, day, 9 + u8::try_from(i % 8).unwrap_or(0), 15),
        });
    }
    files.push(SampleFile {
        path: "report.pdf".to_owned(),
        content: Content::Bytes(pdf(&[
            "Quarterly report, final version",
            "This is the newer, larger copy of the report.",
            "Its older, smaller copy is in the Backup folder,",
            "so copying it there asks what to do with the name.",
            "Section one: the tram ran on time.",
            "Section two: the crows were well fed.",
            "Section three: Jimothy was seen twice.",
        ])),
        modified: utc(2026, 10, 1, 16, 30),
    });
    files.push(SampleFile {
        path: "Backup/report.pdf".to_owned(),
        content: Content::Bytes(pdf(&["Quarterly report, draft", "The older copy."])),
        modified: utc(2026, 9, 1, 10, 0),
    });
    files.push(SampleFile {
        path: "big.bin".to_owned(),
        content: Content::Big(big_bytes),
        modified: utc(2026, 8, 20, 12, 0),
    });
    for (i, (name, text)) in NAMES.iter().enumerate() {
        files.push(SampleFile {
            path: format!("Names/{name}"),
            content: Content::Text((*text).to_owned()),
            modified: utc(2026, 7, 10 + u8::try_from(i).unwrap_or(0), 11, 0),
        });
    }
    let colors = [[40, 60, 90], [90, 40, 30], [30, 90, 50], [70, 70, 20]];
    for (i, (name, side)) in PHOTOS.iter().enumerate() {
        files.push(SampleFile {
            path: format!("Photos/{name}"),
            content: Content::Bytes(png(*side, *side, colors[i % colors.len()])),
            modified: utc(2026, 6, 1 + u8::try_from(i * 2).unwrap_or(0), 18, 45),
        });
    }
    files
}

/// Everything below `root`, as paths relative to it with `/`, folders
/// with a trailing `/`.
fn walk(root: &Path) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for item in fs::read_dir(&folder)
            .with_context(|| format!("{} could not be read", folder.display()))?
        {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            if item.file_type()?.is_dir() {
                found.insert(format!("{prefix}{name}/"));
                stack.push((item.path(), format!("{prefix}{name}/")));
            } else {
                found.insert(format!("{prefix}{name}"));
            }
        }
    }
    Ok(found)
}

/// Writes the owner's check folder at `path` with a `big.bin` of
/// `big_bytes`. Returns the lines to say: what was written, then anything
/// else found there (never removed).
pub fn write_sample_folder(path: &Path, big_bytes: u64) -> Result<Vec<String>> {
    let is_sample = SAMPLE_FOLDERS.iter().all(|f| path.join(f).is_dir());
    if !is_empty_or_new(path)? && !is_sample {
        bail!(
            "{} has other things in it; name a new or empty folder",
            path.display()
        );
    }
    if !path.exists() {
        fs::create_dir(path).with_context(|| format!("{} could not be made", path.display()))?;
    }
    for folder in SAMPLE_FOLDERS {
        let folder = path.join(folder);
        if !folder.is_dir() {
            fs::create_dir(&folder)
                .with_context(|| format!("{} could not be made", folder.display()))?;
        }
    }
    let files = sample_files(big_bytes);
    let mut expected: BTreeSet<String> = SAMPLE_FOLDERS.iter().map(|f| format!("{f}/")).collect();
    for file in &files {
        let target = path.join(&file.path);
        match &file.content {
            Content::Text(text) => write_file(&target, text.as_bytes(), file.modified)?,
            Content::Bytes(bytes) => write_file(&target, bytes, file.modified)?,
            Content::Big(bytes) => big_file(&target, *bytes, Some(file.modified))?,
        }
        expected.insert(file.path.clone());
    }
    let mut lines = vec![format!(
        "sample-folder: wrote {} files and {} folders in {}.",
        files.len(),
        SAMPLE_FOLDERS.len(),
        path.display()
    )];
    lines.push(format!(
        "Top: 31 items. big.bin is {} MB; report.pdf conflicts with Backup.",
        big_bytes / 1_000_000
    ));
    let extra: Vec<String> = walk(path)?.difference(&expected).cloned().collect();
    if !extra.is_empty() {
        lines.push(format!(
            "Not part of the sample, left as they are: {}.",
            extra.join(", ")
        ));
    }
    Ok(lines)
}

/// `cargo xtask sample-folder PATH [--big-mb N]`.
pub fn sample_folder(args: &[&str]) -> Result<bool> {
    let (path, big_mb) = match args {
        [path] => (*path, DEFAULT_BIG_MB),
        [path, "--big-mb", mb] | ["--big-mb", mb, path] => (
            *path,
            mb.parse::<u64>()
                .ok()
                .filter(|mb| *mb > 0)
                .with_context(|| format!("--big-mb takes a number of megabytes, not {mb}"))?,
        ),
        _ => {
            eprintln!("Usage: cargo xtask sample-folder PATH [--big-mb N]");
            return Ok(false);
        }
    };
    let path = checked_target(path)?;
    println!(
        "sample-folder: writing the check folder in {}, big.bin {big_mb} MB.",
        path.display()
    );
    for line in write_sample_folder(&path, big_mb * 1_000_000)? {
        println!("{line}");
    }
    Ok(true)
}

// ---------------------------------------------------------------------
// The search tree.

/// The search tree's parameters, from `bench/search-tree.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TreeParams {
    /// Files in the tree.
    pub files: u64,
    /// Folders in the tree, below its top.
    pub folders: u64,
    /// The deepest a folder goes below the top.
    pub depth: u32,
    /// Bytes of text and code, spread over the text and code files.
    pub text_bytes: u64,
    /// Binary files among the files.
    pub binary_files: u64,
    /// Bytes of the binary files together.
    pub binary_bytes: u64,
    /// Of the files that are not binary, the percent that are code.
    pub code_percent: u64,
    /// The word a known number of files hold, once each, and have in
    /// their names.
    pub hit_word: String,
    /// How many files hold it.
    pub hit_files: u64,
    /// The word every file holds.
    pub every_word: String,
    /// The word no file holds.
    pub missing_word: String,
    /// The seed of the generator.
    pub seed: u64,
}

/// The search tree's parameter file, both sizes in one.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeFile {
    full: TreeParams,
    small: TreeParams,
}

/// Reads `bench/search-tree.toml`: the full tree, or the small one.
pub fn tree_params(root: &Path, small: bool) -> Result<TreeParams> {
    let file = root.join("bench").join("search-tree.toml");
    let text = fs::read_to_string(&file)
        .with_context(|| format!("{} could not be read", file.display()))?;
    let parsed: TreeFile =
        toml::from_str(&text).with_context(|| format!("{} is not valid", file.display()))?;
    Ok(if small { parsed.small } else { parsed.full })
}

/// Words for text, none of which holds the hit, every or missing word.
const VOCABULARY: [&str; 48] = [
    "river", "bridge", "tram", "harbor", "cedar", "maple", "rain", "cloud", "lantern", "window",
    "garden", "kettle", "ledger", "orchard", "pepper", "quiet", "signal", "radio", "battery",
    "folder", "summit", "valley", "copper", "meadow", "anchor", "basket", "candle", "dinner",
    "engine", "feather", "glacier", "hammer", "island", "jacket", "kitchen", "letter", "mirror",
    "needle", "oyster", "pocket", "ribbon", "saddle", "ticket", "violin", "walnut", "yellow",
    "zipper", "crow",
];

/// What a search tree holds, in numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeFacts {
    /// Files written.
    pub files: u64,
    /// Folders written below the top.
    pub folders: u64,
    /// Bytes written.
    pub bytes: u64,
    /// Text files.
    pub text_files: u64,
    /// Code files.
    pub code_files: u64,
    /// Binary files.
    pub binary_files: u64,
    /// Files that hold the hit word, and have it in their names.
    pub hit_files: u64,
}

impl TreeFacts {
    /// The facts in words, the counts first.
    pub fn describe(&self, params: &TreeParams) -> Vec<String> {
        vec![
            format!(
                "{} files in {} folders, {}.",
                self.files,
                self.folders,
                diroxide_core::format::human_size(self.bytes)
            ),
            format!(
                "{} text, {} code (.rs), {} binary (.bin).",
                self.text_files, self.code_files, self.binary_files
            ),
            format!(
                "{} files hold {} once, and have it in their names.",
                self.hit_files, params.hit_word
            ),
            format!(
                "Every file holds {}; no file holds {}.",
                params.every_word, params.missing_word
            ),
        ]
    }
}

/// Fills `out` with text of about `size` bytes: lines of words, the every
/// word on the first line, the hit word once in the middle when `hit`.
fn text_content(rng: &mut Rng, size: usize, params: &TreeParams, hit: bool, code: bool) -> Vec<u8> {
    let mut out = String::with_capacity(size + 64);
    if code {
        out.push_str(&format!("// {}\n", params.every_word));
    } else {
        out.push_str(&format!("{}\n", params.every_word));
    }
    let mut hit_done = !hit;
    let mut item = 0u64;
    while out.len() < size || !hit_done {
        if !hit_done && out.len() >= size / 2 {
            out.push_str(&params.hit_word);
            out.push('\n');
            hit_done = true;
            continue;
        }
        if code {
            item += 1;
            let word = VOCABULARY[usize::try_from(rng.below(VOCABULARY.len() as u64)).unwrap_or(0)];
            out.push_str(&format!(
                "fn {word}_{item}() -> u64 {{ {} }}\n",
                rng.below(1000)
            ));
        } else {
            for _ in 0..10 {
                let word =
                    VOCABULARY[usize::try_from(rng.below(VOCABULARY.len() as u64)).unwrap_or(0)];
                out.push_str(word);
                out.push(' ');
            }
            out.push('\n');
        }
    }
    let mut bytes = out.into_bytes();
    if bytes.len() > size && !hit {
        // Cut back to the size, at a line's end so no word is split.
        let cut = bytes[..size]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(bytes.len(), |at| at + 1);
        bytes.truncate(cut.max(params.every_word.len() + 1));
    }
    bytes
}

/// Writes the search tree under `path`, which must be new or empty.
pub fn write_search_tree(path: &Path, params: &TreeParams) -> Result<TreeFacts> {
    if !is_empty_or_new(path)? {
        bail!(
            "{} has other things in it; name a new or empty folder",
            path.display()
        );
    }
    if params.folders == 0 || params.files == 0 || params.depth == 0 {
        bail!("the tree needs at least one file, one folder and a depth of one");
    }
    if !path.exists() {
        fs::create_dir(path).with_context(|| format!("{} could not be made", path.display()))?;
    }
    // A b-ary tree of folders, b the smallest that fits the depth.
    let mut branching = 2u64;
    loop {
        let mut reach = 0u64;
        let mut level = 1u64;
        for _ in 0..params.depth {
            level = level.saturating_mul(branching);
            reach = reach.saturating_add(level);
        }
        if reach >= params.folders {
            break;
        }
        branching += 1;
    }
    let mut rng = Rng::new(params.seed);
    let mut folders: Vec<PathBuf> = vec![path.to_path_buf()];
    for k in 1..=params.folders {
        let parent = usize::try_from((k - 1) / branching).unwrap_or(0);
        let word = VOCABULARY[usize::try_from(rng.below(VOCABULARY.len() as u64)).unwrap_or(0)];
        let folder = folders[parent].join(format!("{word}-{k:04}"));
        fs::create_dir(&folder)
            .with_context(|| format!("{} could not be made", folder.display()))?;
        folders.push(folder);
    }
    let binary_files = params.binary_files.min(params.files);
    let binary_stride = params.files.checked_div(binary_files).unwrap_or(u64::MAX);
    let plain_files = params.files - binary_files;
    let hit_files = params.hit_files.min(plain_files);
    let hit_stride = plain_files.checked_div(hit_files).unwrap_or(u64::MAX);
    let text_average = params.text_bytes.checked_div(plain_files).unwrap_or(0);
    let binary_size = params.binary_bytes.checked_div(binary_files).unwrap_or(0);
    let mut facts = TreeFacts {
        folders: params.folders,
        ..TreeFacts::default()
    };
    let mut plain_index = 0u64;
    for j in 0..params.files {
        let folder = &folders[1 + usize::try_from(j % params.folders).unwrap_or(0)];
        let binary = j % binary_stride == 0 && facts.binary_files < binary_files;
        let (name, bytes) = if binary {
            let mut bytes = vec![0u8; usize::try_from(binary_size).unwrap_or(0)];
            rng.fill(&mut bytes);
            let word = params.every_word.as_bytes();
            let n = word.len().min(bytes.len());
            bytes[..n].copy_from_slice(&word[..n]);
            facts.binary_files += 1;
            (format!("data-{j:06}.bin"), bytes)
        } else {
            let hit = plain_index.is_multiple_of(hit_stride) && facts.hit_files < hit_files;
            plain_index += 1;
            let code = (j % 100) < params.code_percent;
            let size = text_average / 2 + rng.below(text_average.max(1));
            let bytes = text_content(
                &mut rng,
                usize::try_from(size).unwrap_or(0),
                params,
                hit,
                code,
            );
            let extension = if code { "rs" } else { "txt" };
            let stem = if hit {
                facts.hit_files += 1;
                params.hit_word.clone()
            } else {
                VOCABULARY[usize::try_from(rng.below(VOCABULARY.len() as u64)).unwrap_or(0)]
                    .to_owned()
            };
            if code {
                facts.code_files += 1;
            } else {
                facts.text_files += 1;
            }
            (format!("{stem}-{j:06}.{extension}"), bytes)
        };
        let file = folder.join(name);
        fs::write(&file, &bytes)
            .with_context(|| format!("{} could not be written", file.display()))?;
        facts.files += 1;
        facts.bytes += bytes.len() as u64;
    }
    Ok(facts)
}

/// `cargo xtask search-tree PATH [--small]`.
pub fn search_tree(root: &Path, args: &[&str]) -> Result<bool> {
    let (path, small) = match args {
        [path] => (*path, false),
        [path, "--small"] | ["--small", path] => (*path, true),
        _ => {
            eprintln!("Usage: cargo xtask search-tree PATH [--small]");
            return Ok(false);
        }
    };
    let path = checked_target(path)?;
    let params = tree_params(root, small)?;
    println!(
        "search-tree: writing {} files in {} folders in {}.",
        params.files,
        params.folders,
        path.display()
    );
    let facts = write_search_tree(&path, &params)?;
    for line in facts.describe(&params) {
        println!("{line}");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// Every file below a folder: its path, its bytes and its modified
    /// time in seconds.
    fn snapshot(path: &Path) -> BTreeMap<String, (Vec<u8>, u64)> {
        walk(path)
            .unwrap()
            .into_iter()
            .filter(|p| !p.ends_with('/'))
            .map(|p| {
                let full = path.join(&p);
                let modified = fs::metadata(&full)
                    .unwrap()
                    .modified()
                    .unwrap()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                (p, (fs::read(&full).unwrap(), modified))
            })
            .collect()
    }

    /// The owner's rule 1a for names written to disk: no control,
    /// private-use, invisible, reserved or shell-special character, no
    /// trailing dot or space, no two names in a folder that differ only
    /// by case.
    fn assert_plain_names(path: &Path) {
        let mut by_folder: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in walk(path).unwrap() {
            let trimmed = entry.trim_end_matches('/');
            let (folder, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
            for c in name.chars() {
                let invisible = matches!(c,
                    '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
                    | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{E000}'..='\u{F8FF}');
                assert!(
                    !c.is_control() && !invisible && !" :*?\"<>|\\/$`!&;#~'()[]{}".contains(c),
                    "{entry} holds {c:?}"
                );
            }
            assert!(!name.ends_with('.') && !name.is_empty(), "{entry}");
            assert!(diroxide_vfs::check_name(name, &[]).is_empty(), "{entry}");
            by_folder
                .entry(folder.to_owned())
                .or_default()
                .push(name.to_lowercase());
        }
        for (folder, names) in by_folder {
            let unique: BTreeSet<&String> = names.iter().collect();
            assert_eq!(unique.len(), names.len(), "case twins in {folder}");
        }
    }

    #[test]
    fn targets_on_drive_c_and_at_the_top_are_refused() {
        if cfg!(windows) {
            assert!(checked_target("C:\\dirocheck").is_err());
            assert!(checked_target("c:/dirocheck").is_err());
            assert!(checked_target("D:\\").is_err());
            assert!(checked_target("D:dirocheck").is_err());
            assert!(checked_target("\\\\server\\share\\x").is_err());
            assert!(checked_target("dirocheck").is_err());
        } else {
            assert!(checked_target("/").is_err());
            assert!(checked_target("dirocheck").is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("a").join("..").join("b");
        assert!(checked_target(inside.to_str().unwrap()).is_err());
        let deep = dir.path().join("missing").join("x");
        assert!(checked_target(deep.to_str().unwrap()).is_err());
        let fine = dir.path().join("check");
        if !cfg!(windows)
            || !dir
                .path()
                .to_string_lossy()
                .to_uppercase()
                .starts_with("C:")
        {
            assert_eq!(checked_target(fine.to_str().unwrap()).unwrap(), fine);
        }
    }

    #[test]
    fn two_runs_write_the_same_check_folder_with_plain_names() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = (dir.path().join("one"), dir.path().join("two"));
        let said = write_sample_folder(&one, 3_000_000).unwrap();
        assert!(said[0].starts_with("sample-folder: wrote 44 files and 3 folders in "));
        write_sample_folder(&two, 3_000_000).unwrap();
        assert_eq!(snapshot(&one), snapshot(&two));
        assert_plain_names(&one);
        let top = fs::read_dir(&one).unwrap().count();
        assert_eq!(top, 31);
        assert_eq!(fs::read_dir(one.join("Photos")).unwrap().count(), 12);
        assert_eq!(fs::metadata(one.join("big.bin")).unwrap().len(), 3_000_000);
        let newer = fs::metadata(one.join("report.pdf")).unwrap();
        let older = fs::metadata(one.join("Backup").join("report.pdf")).unwrap();
        assert!(newer.len() > older.len());
        assert!(newer.modified().unwrap() > older.modified().unwrap());
    }

    #[test]
    fn a_second_run_restores_the_sample_and_names_what_else_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("check");
        write_sample_folder(&path, 1_000_000).unwrap();
        let before = snapshot(&path);
        fs::write(path.join("notes.txt"), b"changed").unwrap();
        fs::write(path.join("report-2026.pdf"), b"renamed by the owner").unwrap();
        let said = write_sample_folder(&path, 1_000_000).unwrap();
        assert_eq!(
            said.last().unwrap(),
            "Not part of the sample, left as they are: report-2026.pdf."
        );
        let mut after = snapshot(&path);
        assert!(after.remove("report-2026.pdf").is_some());
        assert_eq!(after, before);
        let other = dir.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("mine.txt"), b"x").unwrap();
        assert!(write_sample_folder(&other, 1_000_000).is_err());
    }

    #[test]
    fn the_small_files_are_valid() {
        let doc = pdf(&["Hello"]);
        let text = String::from_utf8_lossy(&doc);
        let xref: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(text[xref..].starts_with("xref"));
        assert!(text.contains("1 0 obj") && text.ends_with("%%EOF\n"));
        let picture = png(3, 2, [1, 2, 3]);
        assert!(picture.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(empty_zip().len(), 22);
    }

    #[test]
    fn the_search_tree_holds_what_it_says() {
        let params = tree_params(&root(), true).unwrap();
        for word in VOCABULARY {
            for special in [&params.hit_word, &params.every_word, &params.missing_word] {
                assert!(!word.contains(special.as_str()) && !special.contains(word));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tree");
        let facts = write_search_tree(&path, &params).unwrap();
        assert_eq!(facts.files, params.files);
        assert_eq!(facts.hit_files, params.hit_files);
        assert_eq!(facts.binary_files, params.binary_files);
        let all = walk(&path).unwrap();
        let files: Vec<&String> = all.iter().filter(|p| !p.ends_with('/')).collect();
        let folders = all.iter().filter(|p| p.ends_with('/')).count() as u64;
        assert_eq!(files.len() as u64, params.files);
        assert_eq!(folders, params.folders);
        let deepest = all
            .iter()
            .filter(|p| p.ends_with('/'))
            .map(|p| p.matches('/').count())
            .max()
            .unwrap();
        assert!(deepest <= params.depth as usize, "{deepest}");
        let holds =
            |bytes: &[u8], word: &str| bytes.windows(word.len()).any(|w| w == word.as_bytes());
        let mut hits = 0;
        let mut bytes_total = 0u64;
        for file in &files {
            let bytes = fs::read(path.join(file)).unwrap();
            bytes_total += bytes.len() as u64;
            assert!(holds(&bytes, &params.every_word), "{file}");
            assert!(!holds(&bytes, &params.missing_word), "{file}");
            if holds(&bytes, &params.hit_word) {
                hits += 1;
                assert!(file.contains(&params.hit_word), "{file}");
            }
        }
        assert_eq!(hits, params.hit_files);
        assert_eq!(bytes_total, facts.bytes);
        assert_plain_names(&path);
        assert!(write_search_tree(&path, &params).is_err(), "not empty");
    }

    #[test]
    fn big_files_have_their_size_and_differ_block_by_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        big_file(&path, 2 * 1024 * 1024 + 5, None).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 2 * 1024 * 1024 + 5);
        assert_ne!(bytes[..1024 * 1024], bytes[1024 * 1024..2 * 1024 * 1024]);
        let names = flat_folder(dir.path(), 3).unwrap();
        assert_eq!(
            names,
            ["entry-000001.txt", "entry-000002.txt", "entry-000003.txt"]
        );
    }
}
