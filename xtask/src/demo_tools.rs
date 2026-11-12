//! `cargo xtask demo-tools PATH`: the owner's demo set, written into a
//! new or empty folder on drive D: with plain names only. The alpha 5
//! check's fixtures (R34's commits 37 and 38, section 5.2) are in
//! [`alpha5`]: `rename`, `sync`, `dups`, `flat`, `state`, `open` and
//! `paths.nul`. Alpha 4's (R30's commits 118 and 119) stay beside them:
//!
//! - `photos`: the sample folder's twelve PNGs and `broken.png`, the PNG
//!   signature then noise;
//! - `mixed`: `portrait.jpg`, a drawn picture (no photograph) stored 400
//!   by 300 with Exif orientation 6, the camera "Example Camera" and its
//!   serial, taken Sunday, October 11, 2026 at 09:30, a place and the
//!   description "Crow on a wire"; `liar.png`, the same picture as JPEG
//!   bytes; `cp1250.txt`, Polish text in Windows-1250;
//! - `music/song.mp3`: twelve seconds of silent frames at 128 kbps with
//!   its title, artist and album tags; nothing in it can sound;
//! - `docs`: `report.pdf`, three pages written by hand with the title
//!   "Example Report" and the author "Ada Example"; `report.md` and
//!   `notes.org`, alpha 2's demo texts;
//! - `download`: `example-tool.zip`, packed by diroxide's own pack runner;
//!   `example-notes.txt`, edited after it was hashed; `SHA256SUMS`, whose
//!   three lines are the zip's right value, the notes' stale one and a
//!   missing `example-extra.txt`; `example-tool.zip.sha256`;
//! - `repo`, made by git when git is there, with the placeholder
//!   identity "Ada Example", no configuration of the computer's, and
//!   fixed dates: one commit, then `hello.txt` changed and `notes.txt`
//!   new; `repo-hook`, whose `.git/config` names a program that does not
//!   exist as its `core.fsmonitor`. Without git both are left out with a
//!   line;
//! - `disk`: `example.iso`, ISO 9660 with Joliet and Rock Ridge holding
//!   `EFI/BOOT/BOOTX64.EFI` and `README.TXT`; `two-parts.img`, an MBR
//!   disk of two FAT16 partitions of 8 MB labeled BOOT and DATA, each
//!   holding `hello.txt`; both written here byte by byte, in pure Rust;
//! - `hot`: the watched folder `scans` with its `in`, `out`, `processed`
//!   and `failed`, and its task file `scans.toml` for the config folder's
//!   `tasks`;
//! - `out`, empty;
//! - `check-steps.txt`: the alpha 5 check's steps in plain lines, one a
//!   line.
//!
//! Every file has fixed bytes and a fixed modified time, nothing is
//! downloaded, and no hostile name is on the disk. The command lists each
//! fixture with its size, one line each, the name first, then the shell
//! lines of steps 2 and 8, and ends with the one `diro` line that starts
//! the check.

use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use diroxide_core::format::human_size;
use diroxide_core::{Timestamp, UtcOffset};
use sha2::{Digest, Sha256};

use crate::bench::{pack_items, target_folder};
use crate::demo::{NOTES_ORG, REPORT_MD, checked_folder};
use crate::sample::{PHOTOS, png};

mod alpha5;

/// The fixtures, in the order the command lists them: alpha 5's, then
/// alpha 4's.
pub const FIXTURES: [&str; 17] = [
    "rename",
    "sync",
    "dups",
    "flat",
    "state",
    "open",
    "paths.nul",
    "photos",
    "mixed",
    "music",
    "docs",
    "download",
    "repo",
    "repo-hook",
    "disk",
    "hot",
    "out",
];

/// The owner's steps, written beside the fixtures.
pub const STEPS_FILE: &str = "check-steps.txt";

const USAGE: &str =
    "Usage: cargo xtask demo-tools PATH, a new or empty folder on drive D:, such as D:\\dirocheck5";

/// The placeholder identity of the demo repositories.
const GIT_NAME: &str = "Ada Example";

/// Its address, on a domain that never resolves.
const GIT_EMAIL: &str = "ada@example.invalid";

/// The program `repo-hook` names as its file system monitor: it does not
/// exist anywhere, so nothing runs even if a tool trusted the repository.
const MISSING_MONITOR: &str = "example-fsmonitor-missing";

/// A system time from a civil date and time in UTC.
fn utc(month: u8, day: u8, hour: u8, minute: u8) -> SystemTime {
    let seconds = Timestamp::from_civil(2026, month, day, hour, minute, 0, UtcOffset::UTC)
        .map_or(0, Timestamp::unix_seconds);
    UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).unwrap_or(0))
}

/// The modified time of most fixtures: Sunday, October 11, 2026, 09:30
/// UTC, when the portrait was taken.
fn taken() -> SystemTime {
    utc(10, 11, 9, 30)
}

/// Makes a folder and its parents.
fn make_folder(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("{} could not be made", path.display()))
}

/// Writes a file and sets its modified time.
fn write_file(path: &Path, bytes: &[u8], modified: SystemTime) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("{} could not be written", path.display()))?;
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(modified))
        .with_context(|| format!("the time of {} could not be set", path.display()))
}

/// The SHA-256 of some bytes, in lowercase hex.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------
// photos

/// Writes the twelve sample PNGs and `broken.png`.
fn photos(folder: &Path) -> Result<()> {
    make_folder(folder)?;
    let colors = [[40, 60, 90], [90, 40, 30], [30, 90, 50], [70, 70, 20]];
    for (i, (name, side)) in PHOTOS.iter().enumerate() {
        let day = 1 + u8::try_from(i * 2).unwrap_or(0);
        write_file(
            &folder.join(name),
            &png(*side, *side, colors[i % colors.len()]),
            utc(6, day, 18, 45),
        )?;
    }
    write_file(
        &folder.join("broken.png"),
        &broken_png(),
        utc(6, 30, 18, 45),
    )
}

/// The PNG signature, then 200 bytes of fixed noise: a picture no reader
/// can read.
pub fn broken_png() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut x: u32 = 0x2545_F491;
    for _ in 0..200 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out.push(x.to_be_bytes()[0]);
    }
    out
}

// ---------------------------------------------------------------------
// mixed

/// The portrait as stored: 400 wide and 300 tall, a crow on a wire drawn
/// in a few flat shapes against an evening sky. No photograph.
fn crow_picture() -> image::RgbImage {
    image::RgbImage::from_fn(400, 300, |x, y| {
        let (x, y) = (i64::from(x), i64::from(y));
        // The crow: a body, a head and a tail, all dark.
        let body = (x - 200).pow(2) * 4 + (y - 150).pow(2) * 9 <= 60 * 60 * 9;
        let head = (x - 250).pow(2) + (y - 120).pow(2) <= 22 * 22;
        let tail = (130..170).contains(&x) && (150..170).contains(&y) && y - 150 <= (170 - x) / 2;
        // The wire: a dark line across, sagging a little.
        let sag = (x - 200).pow(2) / 4000;
        let wire = (y - (205 + sag)).abs() <= 1;
        if body || head || tail {
            image::Rgb([24, 24, 32])
        } else if wire {
            image::Rgb([40, 40, 40])
        } else {
            let shade = u8::try_from(120 + y * 100 / 300).unwrap_or(220);
            image::Rgb([shade / 2 + 60, shade / 2 + 40, shade])
        }
    })
}

/// A picture as JPEG bytes at quality 85.
fn jpeg_of(picture: &image::RgbImage) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .write_image(
            picture.as_raw(),
            picture.width(),
            picture.height(),
            image::ExtendedColorType::Rgb8,
        )
        .context("the portrait could not be written as JPEG")?;
    Ok(out)
}

/// A TIFF value, as an Exif block holds it.
enum Value {
    Bytes(Vec<u8>),
    Ascii(&'static str),
    Short(u16),
    Long(u32),
    Rationals(Vec<(u32, u32)>),
}

impl Value {
    /// The value's TIFF type, count and big-endian bytes.
    fn encoded(&self) -> (u16, u32, Vec<u8>) {
        match self {
            Value::Bytes(bytes) => (1, len32(bytes.len()), bytes.clone()),
            Value::Ascii(text) => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                (2, len32(bytes.len()), bytes)
            }
            Value::Short(n) => (3, 1, n.to_be_bytes().to_vec()),
            Value::Long(n) => (4, 1, n.to_be_bytes().to_vec()),
            Value::Rationals(list) => {
                let mut bytes = Vec::new();
                for (top, bottom) in list {
                    bytes.extend_from_slice(&top.to_be_bytes());
                    bytes.extend_from_slice(&bottom.to_be_bytes());
                }
                (5, len32(list.len()), bytes)
            }
        }
    }
}

/// A length as a TIFF count; the demo's are all small.
fn len32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// One IFD at offset `at` of the TIFF block, its larger values after its
/// entries; the next IFD is none. Entries must be in tag order.
fn ifd(at: u32, entries: &[(u16, Value)]) -> Vec<u8> {
    let count = u16::try_from(entries.len()).unwrap_or(u16::MAX);
    let data_at = at + 2 + 12 * u32::from(count) + 4;
    let mut head = count.to_be_bytes().to_vec();
    let mut data: Vec<u8> = Vec::new();
    for (tag, value) in entries {
        let (kind, n, mut bytes) = value.encoded();
        head.extend_from_slice(&tag.to_be_bytes());
        head.extend_from_slice(&kind.to_be_bytes());
        head.extend_from_slice(&n.to_be_bytes());
        if bytes.len() <= 4 {
            bytes.resize(4, 0);
            head.extend_from_slice(&bytes);
        } else {
            head.extend_from_slice(&(data_at + len32(data.len())).to_be_bytes());
            data.extend_from_slice(&bytes);
            if data.len() % 2 == 1 {
                data.push(0);
            }
        }
    }
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&data);
    head
}

/// The portrait's Exif block, a big-endian TIFF: the description, the
/// camera, orientation 6 (turned a quarter right to be shown), the time
/// taken, the camera's serial and a place in Portland.
pub fn portrait_exif() -> Vec<u8> {
    let first = |exif_at: u32, gps_at: u32| {
        vec![
            (0x010E, Value::Ascii("Crow on a wire")),
            (0x010F, Value::Ascii("Example")),
            (0x0110, Value::Ascii("Example Camera")),
            (0x0112, Value::Short(6)),
            (0x8769, Value::Long(exif_at)),
            (0x8825, Value::Long(gps_at)),
        ]
    };
    let exif = vec![
        (0x9003, Value::Ascii("2026:10:11 09:30:00")),
        (0xA431, Value::Ascii("EX-0001234")),
    ];
    let gps = vec![
        (0x0000, Value::Bytes(vec![2, 3, 0, 0])),
        (0x0001, Value::Ascii("N")),
        (0x0002, Value::Rationals(vec![(45, 1), (30, 1), (0, 1)])),
        (0x0003, Value::Ascii("W")),
        (0x0004, Value::Rationals(vec![(122, 1), (40, 1), (0, 1)])),
    ];
    let first_len = len32(ifd(8, &first(0, 0)).len());
    let exif_at = 8 + first_len;
    let exif_len = len32(ifd(exif_at, &exif).len());
    let gps_at = exif_at + exif_len;
    let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08".to_vec();
    tiff.extend_from_slice(&ifd(8, &first(exif_at, gps_at)));
    tiff.extend_from_slice(&ifd(exif_at, &exif));
    tiff.extend_from_slice(&ifd(gps_at, &gps));
    tiff
}

/// A JPEG with an Exif segment put in right after its start marker.
pub fn with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut segment = b"Exif\x00\x00".to_vec();
    segment.extend_from_slice(tiff);
    let length = u16::try_from(segment.len() + 2).unwrap_or(u16::MAX);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&segment);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// "Zażółć gęślą jaźń. Król żyje, a wrona śpi na drucie." in two lines,
/// in Windows-1250, the code page of Central Europe.
pub const CP1250_TEXT: [u8; 53] = [
    90, 97, 191, 243, 179, 230, 32, 103, 234, 156, 108, 185, 32, 106, 97, 159, 241, 46, 10, 75,
    114, 243, 108, 32, 191, 121, 106, 101, 44, 32, 97, 32, 119, 114, 111, 110, 97, 32, 156, 112,
    105, 32, 110, 97, 32, 100, 114, 117, 99, 105, 101, 46, 10,
];

/// Writes the portrait, the JPEG named .png and the Polish text.
fn mixed(folder: &Path) -> Result<()> {
    make_folder(folder)?;
    let jpeg = jpeg_of(&crow_picture())?;
    write_file(
        &folder.join("portrait.jpg"),
        &with_exif(&jpeg, &portrait_exif()),
        taken(),
    )?;
    write_file(&folder.join("liar.png"), &jpeg, taken())?;
    write_file(&folder.join("cp1250.txt"), &CP1250_TEXT, utc(10, 12, 8, 0))
}

// ---------------------------------------------------------------------
// music

/// The number of frames in the song: 462 frames of 1,152 samples at
/// 44.1 kHz is 12.07 seconds, and its 192,654 bytes at 128 kbps are 12.04
/// seconds, so a reader that counts either way says 12.
pub const SONG_FRAMES: usize = 462;

/// One MPEG-1 Layer III frame at 128 kbps and 44.1 kHz, stereo, with no
/// padding: 417 bytes.
pub const FRAME_BYTES: usize = 417;

/// An ID3v2.3 tag of text frames, each in ISO-8859-1.
fn id3(frames: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, text) in frames {
        body.extend_from_slice(id.as_bytes());
        body.extend_from_slice(&len32(text.len() + 1).to_be_bytes());
        body.extend_from_slice(&[0, 0, 0]);
        body.extend_from_slice(text.as_bytes());
    }
    let size = len32(body.len());
    let syncsafe = [
        u8::try_from((size >> 21) & 0x7F).unwrap_or(0),
        u8::try_from((size >> 14) & 0x7F).unwrap_or(0),
        u8::try_from((size >> 7) & 0x7F).unwrap_or(0),
        u8::try_from(size & 0x7F).unwrap_or(0),
    ];
    let mut out = b"ID3\x03\x00\x00".to_vec();
    out.extend_from_slice(&syncsafe);
    out.extend_from_slice(&body);
    out
}

/// Twelve seconds of silence: every frame's side information and data
/// are zero, so a decoder makes no sound. Its tags come first.
pub fn song_mp3() -> Vec<u8> {
    let mut out = id3(&[
        ("TIT2", "Example Song"),
        ("TPE1", GIT_NAME),
        ("TALB", "Example Album"),
    ]);
    for _ in 0..SONG_FRAMES {
        out.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        out.extend(std::iter::repeat_n(0u8, FRAME_BYTES - 4));
    }
    out
}

// ---------------------------------------------------------------------
// docs

/// A PDF of three pages written by hand: one line of Helvetica on each,
/// the title "Example Report" and the author "Ada Example" in its
/// information, and a cross-reference table whose offsets are counted.
pub fn report_pdf() -> Vec<u8> {
    let page_text =
        |n: usize| format!("BT /F1 24 Tf 72 700 Td (Example Report, page {n} of 3) Tj ET");
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_owned(),
    ];
    for n in 0..3 {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 9 0 R >> >> >>",
            6 + n
        ));
    }
    for n in 1..=3 {
        let text = page_text(n);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{text}\nendstream",
            text.len()
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());
    objects.push(
        "<< /Title (Example Report) /Author (Ada Example) /CreationDate (D:20261011093000Z) >>"
            .to_owned(),
    );
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        table.push_str(&format!("{offset:010} 00000 n \n"));
    }
    table.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1,
        objects.len()
    ));
    out.extend_from_slice(table.as_bytes());
    out
}

/// Writes the report, its Markdown and the Org notes.
fn docs(folder: &Path) -> Result<()> {
    make_folder(folder)?;
    write_file(&folder.join("report.pdf"), &report_pdf(), taken())?;
    write_file(
        &folder.join("report.md"),
        REPORT_MD.as_bytes(),
        utc(10, 14, 16, 0),
    )?;
    write_file(
        &folder.join("notes.org"),
        NOTES_ORG.as_bytes(),
        utc(10, 13, 8, 15),
    )
}

// ---------------------------------------------------------------------
// download

/// The notes as they were hashed.
const NOTES_BEFORE: &str = "Example notes, version 1.\n";

/// The notes as they are now, edited after the list was made.
const NOTES_AFTER: &str = "Example notes, version 1, edited after the list was made.\n";

/// Writes the zip (packed from `staging`), the notes, and the two lists.
fn download(folder: &Path, staging: &Path) -> Result<()> {
    make_folder(folder)?;
    let tool = staging.join("example-tool");
    make_folder(&tool)?;
    write_file(
        &tool.join("README.txt"),
        b"Example Tool 1.0: a made-up download for the checksum check.\n",
        taken(),
    )?;
    write_file(
        &tool.join("tool.txt"),
        b"This is not a program; it only stands in for one.\n",
        taken(),
    )?;
    // The two files, not their folder: a folder's own modified time
    // would go into the zip and change its bytes on every run.
    let zip = folder.join("example-tool.zip");
    pack_items(
        &zip,
        &[tool.join("README.txt"), tool.join("tool.txt")],
        None,
    )?;
    set_time(&zip, taken())?;
    let zip_hash = sha256_hex(&fs::read(&zip).context("example-tool.zip could not be read")?);
    write_file(
        &folder.join("example-notes.txt"),
        NOTES_AFTER.as_bytes(),
        utc(10, 12, 14, 0),
    )?;
    let sums = format!(
        "{zip_hash}  example-tool.zip\n{}  example-notes.txt\n{}  example-extra.txt\n",
        sha256_hex(NOTES_BEFORE.as_bytes()),
        sha256_hex(b"Example extra, never shipped.\n"),
    );
    write_file(&folder.join("SHA256SUMS"), sums.as_bytes(), taken())?;
    write_file(
        &folder.join("example-tool.zip.sha256"),
        format!("{zip_hash}  example-tool.zip\n").as_bytes(),
        taken(),
    )
}

/// Sets a file's modified time.
fn set_time(path: &Path, modified: SystemTime) -> Result<()> {
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(modified))
        .with_context(|| format!("the time of {} could not be set", path.display()))
}

// ---------------------------------------------------------------------
// repo and repo-hook

/// Whether git answers here.
pub fn git_present() -> bool {
    Command::new("git")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Runs git in `folder` with nothing of the computer's configuration, the
/// placeholder identity and the fixed date; fails with git's own words.
fn git(folder: &Path, empty_config: &Path, args: &[&str]) -> Result<()> {
    let date = "2026-10-11T09:30:00Z";
    let output = Command::new("git")
        .args(["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(folder)
        .env("GIT_CONFIG_GLOBAL", empty_config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", GIT_NAME)
        .env("GIT_AUTHOR_EMAIL", GIT_EMAIL)
        .env("GIT_COMMITTER_NAME", GIT_NAME)
        .env("GIT_COMMITTER_EMAIL", GIT_EMAIL)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .output()
        .context("git could not be started")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Makes `repo` and `repo-hook` with git; `staging` holds the empty
/// configuration file git is pointed at.
fn repositories(folder: &Path, staging: &Path) -> Result<()> {
    let empty_config = staging.join("empty-gitconfig");
    fs::write(&empty_config, b"").context("the empty git configuration could not be written")?;
    let repo = folder.join("repo");
    make_folder(&repo)?;
    git(&repo, &empty_config, &["init", "-q", "-b", "main"])?;
    write_file(&repo.join("hello.txt"), b"Hello, crows.\n", taken())?;
    git(&repo, &empty_config, &["add", "hello.txt"])?;
    git(
        &repo,
        &empty_config,
        &["commit", "-q", "-m", "First commit"],
    )?;
    write_file(
        &repo.join("hello.txt"),
        b"Hello, crows and raccoons.\n",
        utc(10, 12, 10, 0),
    )?;
    write_file(
        &repo.join("notes.txt"),
        b"Notes not added to git yet.\n",
        utc(10, 12, 10, 5),
    )?;
    let hook = folder.join("repo-hook");
    make_folder(&hook)?;
    git(&hook, &empty_config, &["init", "-q", "-b", "main"])?;
    git(
        &hook,
        &empty_config,
        &["config", "core.fsmonitor", MISSING_MONITOR],
    )?;
    write_file(
        &hook.join("README.txt"),
        b"This repository's configuration names a program to run. It does not exist.\n",
        taken(),
    )
}

// ---------------------------------------------------------------------
// disk: the ISO

/// An ISO 9660 sector.
const SECTOR: usize = 2048;

/// The ISO's date and time in a directory record: 2026-10-11 09:30 UTC.
const ISO_DATE: [u8; 7] = [126, 10, 11, 9, 30, 0, 0];

/// The ISO's date and time in a volume descriptor.
const ISO_LONG_DATE: &[u8; 16] = b"2026101109300000";

/// The ISO's sectors, in order.
mod iso_at {
    pub const PRIMARY: usize = 16;
    pub const JOLIET: usize = 17;
    pub const END: usize = 18;
    pub const PATHS_L: u32 = 19;
    pub const PATHS_M: u32 = 20;
    pub const JPATHS_L: u32 = 21;
    pub const JPATHS_M: u32 = 22;
    pub const ROOT: u32 = 23;
    pub const EFI: u32 = 24;
    pub const BOOT: u32 = 25;
    pub const JROOT: u32 = 26;
    pub const JEFI: u32 = 27;
    pub const JBOOT: u32 = 28;
    pub const BOOT_FILE: u32 = 29;
    pub const README: u32 = 30;
    pub const TOTAL: u32 = 31;
}

/// `BOOTX64.EFI`'s few bytes: not a program.
pub const BOOT_FILE_BYTES: &[u8] = b"MZ example boot file, not a program\r\n";

/// `README.TXT`'s bytes.
pub const README_BYTES: &[u8] = b"Example disc made by cargo xtask demo-tools.\r\n";

fn both16(n: u16) -> [u8; 4] {
    let (l, b) = (n.to_le_bytes(), n.to_be_bytes());
    [l[0], l[1], b[0], b[1]]
}

fn both32(n: u32) -> [u8; 8] {
    let (l, b) = (n.to_le_bytes(), n.to_be_bytes());
    [l[0], l[1], l[2], l[3], b[0], b[1], b[2], b[3]]
}

/// UCS-2 big-endian, as Joliet writes names.
fn ucs2(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// One directory record; its system use (Rock Ridge's entries) after the
/// name and the pad byte an even name length needs; its length even.
fn dir_record(extent: u32, size: u32, folder: bool, id: &[u8], system_use: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; 33];
    r[2..10].copy_from_slice(&both32(extent));
    r[10..18].copy_from_slice(&both32(size));
    r[18..25].copy_from_slice(&ISO_DATE);
    r[25] = if folder { 2 } else { 0 };
    r[28..32].copy_from_slice(&both16(1));
    r[32] = u8::try_from(id.len()).unwrap_or(0);
    r.extend_from_slice(id);
    if id.len().is_multiple_of(2) {
        r.push(0);
    }
    r.extend_from_slice(system_use);
    if r.len() % 2 == 1 {
        r.push(0);
    }
    r[0] = u8::try_from(r.len()).unwrap_or(0);
    r
}

/// Rock Ridge's SUSP indicator, first in the root's own record.
fn rr_sp() -> Vec<u8> {
    vec![b'S', b'P', 7, 1, 0xBE, 0xEF, 0]
}

/// Rock Ridge's extension reference, in the root's own record.
fn rr_er() -> Vec<u8> {
    let (id, about, source) = (
        &b"RRIP_1991A"[..],
        &b"ROCK RIDGE"[..],
        &b"SEE PUBLISHER"[..],
    );
    let mut out = vec![
        b'E',
        b'R',
        u8::try_from(8 + id.len() + about.len() + source.len()).unwrap_or(0),
        1,
        u8::try_from(id.len()).unwrap_or(0),
        u8::try_from(about.len()).unwrap_or(0),
        u8::try_from(source.len()).unwrap_or(0),
        1,
    ];
    out.extend_from_slice(id);
    out.extend_from_slice(about);
    out.extend_from_slice(source);
    out
}

/// Rock Ridge's POSIX attributes: the mode, the links, owner and group 0.
fn rr_px(mode: u32, links: u32) -> Vec<u8> {
    let mut out = vec![b'P', b'X', 36, 1];
    out.extend_from_slice(&both32(mode));
    out.extend_from_slice(&both32(links));
    out.extend_from_slice(&both32(0));
    out.extend_from_slice(&both32(0));
    out
}

/// Rock Ridge's name, as the name is.
fn rr_nm(name: &str) -> Vec<u8> {
    let mut out = vec![b'N', b'M', u8::try_from(5 + name.len()).unwrap_or(0), 1, 0];
    out.extend_from_slice(name.as_bytes());
    out
}

/// A folder's mode in Rock Ridge: read and enter for all.
const DIR_MODE: u32 = 0o040_555;

/// A file's mode in Rock Ridge: read for all.
const FILE_MODE: u32 = 0o100_444;

/// A folder's extent: its records, the rest zero.
fn directory(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = records.iter().flatten().copied().collect();
    out.resize(SECTOR, 0);
    out
}

/// A path table: each folder's name, extent and parent, little-endian or
/// big-endian.
fn path_table(entries: &[(Vec<u8>, u32, u16)], big: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, extent, parent) in entries {
        out.push(u8::try_from(id.len()).unwrap_or(0));
        out.push(0);
        if big {
            out.extend_from_slice(&extent.to_be_bytes());
            out.extend_from_slice(&parent.to_be_bytes());
        } else {
            out.extend_from_slice(&extent.to_le_bytes());
            out.extend_from_slice(&parent.to_le_bytes());
        }
        out.extend_from_slice(id);
        if id.len() % 2 == 1 {
            out.push(0);
        }
    }
    out
}

/// A volume descriptor: the primary (type 1) or Joliet's (type 2), with
/// its root's record and its path tables.
fn volume_descriptor(
    joliet: bool,
    root: &[u8],
    paths_size: u32,
    paths_l: u32,
    paths_m: u32,
) -> Vec<u8> {
    let mut d = vec![0u8; SECTOR];
    d[0] = if joliet { 2 } else { 1 };
    d[1..6].copy_from_slice(b"CD001");
    d[6] = 1;
    let text = |d: &mut Vec<u8>, at: usize, len: usize, value: &str| {
        let bytes = if joliet {
            ucs2(value)
        } else {
            value.as_bytes().to_vec()
        };
        for i in 0..len {
            d[at + i] = if joliet { [0, b' '][i % 2] } else { b' ' };
        }
        let n = bytes.len().min(len);
        d[at..at + n].copy_from_slice(&bytes[..n]);
    };
    text(&mut d, 8, 32, "");
    text(&mut d, 40, 32, "EXAMPLE");
    d[80..88].copy_from_slice(&both32(iso_at::TOTAL));
    if joliet {
        d[88..91].copy_from_slice(b"%/E");
    }
    d[120..124].copy_from_slice(&both16(1));
    d[124..128].copy_from_slice(&both16(1));
    d[128..132].copy_from_slice(&both16(2048));
    d[132..140].copy_from_slice(&both32(paths_size));
    d[140..144].copy_from_slice(&paths_l.to_le_bytes());
    d[148..152].copy_from_slice(&paths_m.to_be_bytes());
    d[156..156 + root.len()].copy_from_slice(root);
    text(&mut d, 190, 128, "");
    text(&mut d, 318, 128, "");
    text(&mut d, 446, 128, "");
    text(&mut d, 574, 128, "DIROXIDE XTASK DEMO-TOOLS");
    text(&mut d, 702, 37, "");
    text(&mut d, 739, 37, "");
    text(&mut d, 776, 37, "");
    for at in [813, 830, 864] {
        d[at..at + 16].copy_from_slice(ISO_LONG_DATE);
    }
    d[847..863].copy_from_slice(b"0000000000000000");
    d[881] = 1;
    d
}

/// `example.iso`: ISO 9660 with Joliet's names and Rock Ridge's, 31
/// sectors, holding `EFI/BOOT/BOOTX64.EFI` and `README.TXT`.
pub fn example_iso() -> Vec<u8> {
    use iso_at::*;
    let mut image = vec![0u8; TOTAL as usize * SECTOR];
    let mut put = |sector: usize, bytes: &[u8]| {
        image[sector * SECTOR..sector * SECTOR + bytes.len()].copy_from_slice(bytes);
    };
    let full = len32(SECTOR);
    // The primary tree, with Rock Ridge.
    let root_self = {
        let mut su = rr_sp();
        su.extend(rr_px(DIR_MODE, 3));
        su.extend(rr_er());
        dir_record(ROOT, full, true, &[0], &su)
    };
    let px_dir = |links| rr_px(DIR_MODE, links);
    let named = |mode: u32, links: u32, name: &str| {
        let mut su = rr_px(mode, links);
        su.extend(rr_nm(name));
        su
    };
    put(
        ROOT as usize,
        &directory(&[
            root_self,
            dir_record(ROOT, full, true, &[1], &px_dir(3)),
            dir_record(EFI, full, true, b"EFI", &named(DIR_MODE, 3, "EFI")),
            dir_record(
                README,
                len32(README_BYTES.len()),
                false,
                b"README.TXT;1",
                &named(FILE_MODE, 1, "README.TXT"),
            ),
        ]),
    );
    put(
        EFI as usize,
        &directory(&[
            dir_record(EFI, full, true, &[0], &px_dir(3)),
            dir_record(ROOT, full, true, &[1], &px_dir(3)),
            dir_record(BOOT, full, true, b"BOOT", &named(DIR_MODE, 2, "BOOT")),
        ]),
    );
    put(
        BOOT as usize,
        &directory(&[
            dir_record(BOOT, full, true, &[0], &px_dir(2)),
            dir_record(EFI, full, true, &[1], &px_dir(3)),
            dir_record(
                BOOT_FILE,
                len32(BOOT_FILE_BYTES.len()),
                false,
                b"BOOTX64.EFI;1",
                &named(FILE_MODE, 1, "BOOTX64.EFI"),
            ),
        ]),
    );
    // Joliet's tree: the same folders and files, the names in UCS-2.
    put(
        JROOT as usize,
        &directory(&[
            dir_record(JROOT, full, true, &[0], &[]),
            dir_record(JROOT, full, true, &[1], &[]),
            dir_record(JEFI, full, true, &ucs2("EFI"), &[]),
            dir_record(
                README,
                len32(README_BYTES.len()),
                false,
                &ucs2("README.TXT;1"),
                &[],
            ),
        ]),
    );
    put(
        JEFI as usize,
        &directory(&[
            dir_record(JEFI, full, true, &[0], &[]),
            dir_record(JROOT, full, true, &[1], &[]),
            dir_record(JBOOT, full, true, &ucs2("BOOT"), &[]),
        ]),
    );
    put(
        JBOOT as usize,
        &directory(&[
            dir_record(JBOOT, full, true, &[0], &[]),
            dir_record(JEFI, full, true, &[1], &[]),
            dir_record(
                BOOT_FILE,
                len32(BOOT_FILE_BYTES.len()),
                false,
                &ucs2("BOOTX64.EFI;1"),
                &[],
            ),
        ]),
    );
    put(BOOT_FILE as usize, BOOT_FILE_BYTES);
    put(README as usize, README_BYTES);
    // The path tables, then the descriptors that point at them.
    let primary_paths = vec![
        (vec![0], ROOT, 1),
        (b"EFI".to_vec(), EFI, 1),
        (b"BOOT".to_vec(), BOOT, 2),
    ];
    let joliet_paths = vec![
        (vec![0], JROOT, 1),
        (ucs2("EFI"), JEFI, 1),
        (ucs2("BOOT"), JBOOT, 2),
    ];
    let primary_l = path_table(&primary_paths, false);
    put(PATHS_L as usize, &primary_l);
    put(PATHS_M as usize, &path_table(&primary_paths, true));
    let joliet_l = path_table(&joliet_paths, false);
    put(JPATHS_L as usize, &joliet_l);
    put(JPATHS_M as usize, &path_table(&joliet_paths, true));
    put(
        PRIMARY,
        &volume_descriptor(
            false,
            &dir_record(ROOT, full, true, &[0], &[]),
            len32(primary_l.len()),
            PATHS_L,
            PATHS_M,
        ),
    );
    put(
        JOLIET,
        &volume_descriptor(
            true,
            &dir_record(JROOT, full, true, &[0], &[]),
            len32(joliet_l.len()),
            JPATHS_L,
            JPATHS_M,
        ),
    );
    let mut end = vec![0u8; 7];
    end[0] = 255;
    end[1..6].copy_from_slice(b"CD001");
    end[6] = 1;
    put(END, &end);
    image
}

// ---------------------------------------------------------------------
// disk: the two-partition image

/// A disk sector.
const DISK_SECTOR: usize = 512;

/// Each partition's size in sectors: 8 MiB.
pub const PART_SECTORS: u32 = 16_384;

/// Where the partitions start: at 1 MiB, then right after the first.
pub const PART_STARTS: [u32; 2] = [2048, 2048 + PART_SECTORS];

/// The disk's size in sectors.
pub const DISK_SECTORS: u32 = 2048 + 2 * PART_SECTORS;

/// FAT16's layout in each partition: one reserved sector, two FATs of 32
/// sectors, a root folder of 512 entries (32 sectors), clusters of two
/// sectors from sector 97.
const FAT_SECTORS: u32 = 32;
const ROOT_ENTRIES: u16 = 512;
const DATA_START: u32 = 1 + 2 * FAT_SECTORS + 32;

/// A time and a date as FAT writes them: Sunday, October 11, 2026, 09:30.
const FAT_TIME: u16 = (9 << 11) | (30 << 5);
const FAT_DATE: u16 = ((2026 - 1980) << 9) | (10 << 5) | 11;

/// A short name's checksum, which its long name's entries carry.
fn short_name_sum(name: &[u8; 11]) -> u8 {
    name.iter().fold(0u8, |sum, b| {
        ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(*b)
    })
}

/// One FAT16 file system of `PART_SECTORS` sectors, `hidden` sectors from
/// the disk's start, with a volume label and `hello.txt` (its long name
/// lower case, its short name HELLO.TXT).
pub fn fat16(label: &str, serial: u32, hidden: u32, text: &[u8]) -> Vec<u8> {
    let mut fs = vec![0u8; PART_SECTORS as usize * DISK_SECTOR];
    let mut label11 = [b' '; 11];
    label11[..label.len().min(11)].copy_from_slice(&label.as_bytes()[..label.len().min(11)]);
    // The boot sector and its BIOS parameter block.
    let b = &mut fs[..DISK_SECTOR];
    b[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    b[3..11].copy_from_slice(b"DIROXIDE");
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[13] = 2;
    b[14..16].copy_from_slice(&1u16.to_le_bytes());
    b[16] = 2;
    b[17..19].copy_from_slice(&ROOT_ENTRIES.to_le_bytes());
    b[19..21].copy_from_slice(&u16::try_from(PART_SECTORS).unwrap_or(0).to_le_bytes());
    b[21] = 0xF8;
    b[22..24].copy_from_slice(&u16::try_from(FAT_SECTORS).unwrap_or(0).to_le_bytes());
    b[24..26].copy_from_slice(&63u16.to_le_bytes());
    b[26..28].copy_from_slice(&255u16.to_le_bytes());
    b[28..32].copy_from_slice(&hidden.to_le_bytes());
    b[36] = 0x80;
    b[38] = 0x29;
    b[39..43].copy_from_slice(&serial.to_le_bytes());
    b[43..54].copy_from_slice(&label11);
    b[54..62].copy_from_slice(b"FAT16   ");
    b[510] = 0x55;
    b[511] = 0xAA;
    // Both FATs: the media and end entries, then hello.txt's one cluster.
    for fat in 0..2 {
        let at = (1 + fat * FAT_SECTORS) as usize * DISK_SECTOR;
        fs[at..at + 6].copy_from_slice(&[0xF8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }
    // The root folder: the label, then hello.txt's long and short names.
    let root = (1 + 2 * FAT_SECTORS) as usize * DISK_SECTOR;
    let entry = |name: &[u8; 11], attr: u8, case: u8, cluster: u16, size: u32| {
        let mut e = [0u8; 32];
        e[0..11].copy_from_slice(name);
        e[11] = attr;
        e[12] = case;
        e[14..16].copy_from_slice(&FAT_TIME.to_le_bytes());
        e[16..18].copy_from_slice(&FAT_DATE.to_le_bytes());
        e[18..20].copy_from_slice(&FAT_DATE.to_le_bytes());
        e[22..24].copy_from_slice(&FAT_TIME.to_le_bytes());
        e[24..26].copy_from_slice(&FAT_DATE.to_le_bytes());
        e[26..28].copy_from_slice(&cluster.to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        e
    };
    let short = *b"HELLO   TXT";
    let mut long = [0u8; 32];
    long[0] = 0x41;
    long[11] = 0x0F;
    long[13] = short_name_sum(&short);
    let chars: Vec<u16> = "hello.txt"
        .encode_utf16()
        .chain([0])
        .chain(std::iter::repeat(0xFFFF))
        .take(13)
        .collect();
    let slots: Vec<usize> = (1..11)
        .step_by(2)
        .chain((14..26).step_by(2))
        .chain((28..32).step_by(2))
        .collect();
    for (slot, c) in slots.iter().zip(chars) {
        long[*slot..*slot + 2].copy_from_slice(&c.to_le_bytes());
    }
    fs[root..root + 32].copy_from_slice(&entry(&label11, 0x08, 0, 0, 0));
    fs[root + 32..root + 64].copy_from_slice(&long);
    fs[root + 64..root + 96].copy_from_slice(&entry(&short, 0x20, 0x18, 2, len32(text.len())));
    // Cluster 2, the first of the data area.
    let data = DATA_START as usize * DISK_SECTOR;
    fs[data..data + text.len()].copy_from_slice(text);
    fs
}

/// `two-parts.img`: an MBR and two FAT16 partitions of 8 MB, labeled
/// BOOT and DATA, each holding `hello.txt`.
pub fn two_parts_img() -> Vec<u8> {
    let mut disk = vec![0u8; DISK_SECTORS as usize * DISK_SECTOR];
    disk[440..444].copy_from_slice(b"DIRX");
    for (i, start) in PART_STARTS.iter().enumerate() {
        let at = 446 + 16 * i;
        disk[at] = if i == 0 { 0x80 } else { 0 };
        disk[at + 1..at + 4].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
        disk[at + 4] = 0x06;
        disk[at + 5..at + 8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
        disk[at + 8..at + 12].copy_from_slice(&start.to_le_bytes());
        disk[at + 12..at + 16].copy_from_slice(&PART_SECTORS.to_le_bytes());
    }
    disk[510] = 0x55;
    disk[511] = 0xAA;
    let parts: [(&str, u32, &[u8]); 2] = [
        ("BOOT", 0x2026_1011, b"Hello from partition 1, BOOT.\r\n"),
        ("DATA", 0x2026_1012, b"Hello from partition 2, DATA.\r\n"),
    ];
    for ((label, serial, text), start) in parts.iter().zip(PART_STARTS) {
        let at = start as usize * DISK_SECTOR;
        let fs = fat16(label, *serial, start, text);
        disk[at..at + fs.len()].copy_from_slice(&fs);
    }
    disk
}

/// Writes the ISO and the two-partition image.
fn disk(folder: &Path) -> Result<()> {
    make_folder(folder)?;
    write_file(&folder.join("example.iso"), &example_iso(), taken())?;
    write_file(&folder.join("two-parts.img"), &two_parts_img(), taken())
}

// ---------------------------------------------------------------------
// hot

/// The task file of the watched folder `scans`, its home given.
pub fn scans_task(home: &Path) -> String {
    format!(
        "# The demo's watched folder (cargo xtask demo-tools): a PNG that\n\
         # arrives in in and stops changing for 2 seconds becomes a JPEG in\n\
         # out; the PNG moves to processed, or to failed when it fails.\n\
         name = \"scans\"\n\
         about = \"Scans become JPEG pictures.\"\n\
         kind = \"convert\"\n\
         recipe = \"image-to-jpeg\"\n\
         \n\
         [trigger]\n\
         kind = \"folder\"\n\
         home = '{}'\n\
         match = [\"*.png\"]\n\
         settle = \"2s\"\n\
         \n\
         [run]\n\
         after = \"move\"\n\
         conflicts = \"keep-both\"\n",
        home.display()
    )
}

/// Writes the watched folder and its task file.
fn hot(folder: &Path) -> Result<()> {
    let home = folder.join("scans");
    for part in ["in", "out", "processed", "failed"] {
        make_folder(&home.join(part))?;
    }
    write_file(
        &folder.join("scans.toml"),
        scans_task(&home).as_bytes(),
        taken(),
    )
}

// ---------------------------------------------------------------------
// the steps

/// The owner's steps for the alpha 5 check (R34 section 5.1's ten), one
/// plain line each: `{PATH}` the demo folder, `{START}`, `{RENAME}` and
/// `{PIPE}` the lines of [`start_line`], [`rename_line`] and
/// [`pipe_line`]. The words are the plan's; a step whose lanes had not
/// reported their words when this was written ends with a line starting
/// "To be checked", naming them.
const STEPS: &[&str] = &[
    "The alpha 5 check, about 30 minutes, steps 0 to 9.",
    "NVDA for every step; JAWS for 0, 1, 3, 4 and 5; the Mantis for 1, 3, 4, 5 and 7.",
    "Each line in quotes is what to hear; a different line is a finding.",
    "A line starting To be checked: those lanes had not reported their words yet.",
    "First copy the folder {PATH}\\state\\saved into the state folder beside diro.",
    "Start: {START}",
    "",
    "Step 0, tabs. New tab: Ctrl+T in the window, the View menu in a terminal.",
    "Hear: \"here, tab 2 of 2\". Go to {PATH}\\rename. Previous tab, Next tab.",
    "The palette, Tab list: rows name first. Lock the tab:",
    "Hear: \"rename, tab 2 of 2, locked\". Close tab:",
    "Hear: \"Tab is locked. Unlock it to close.\"",
    "Unlock, close, then Undo close tab: \"Reopened rename, tab 2 of 2\".",
    "Save the tab set as Work, quit, start again: the tabs are back.",
    "Load it by name: \"Tab set Work loaded, 2 tabs\".",
    "Listen for: no tab words while only one tab is open.",
    "To be checked: TB's tab words.",
    "",
    "Step 1, rename fifty files by a rule. In rename, mark all 52, F2.",
    "Add the rule Replace IMG_ with trip-.",
    "Hear: \"50 of 52 files will change. No problems.\"",
    "Rows: \"IMG_0001.jpg becomes trip-0001.jpg\".",
    "Space on a row: \"Skipped by you: IMG_0002.jpg\". Space again.",
    "Apply: \"Rename 50 files? y or n\", No the default. Then y.",
    "Hear: \"Renamed 50 files.\" Ctrl+Z: \"Undid rename of 50 files.\"",
    "To be checked: BR's and RA's rename words.",
    "",
    "Step 2, swap two names in the editor. Mark a.txt and b.txt, F2,",
    "then Edit names in the editor. Lines: \"a.txt (was a.txt)\", \"b.txt (was b.txt)\".",
    "Swap the first words, save, close: \"2 of 2 files will change. No problems.\"",
    "Row: \"Swap with b.txt: a.txt becomes b.txt\".",
    "Open the editor again and give both the name b.txt.",
    "Hear: \"1 problem. 0 of 2 files will change.\"",
    "Hear: \"Problem, name taken: a.txt becomes b.txt\".",
    "Apply: \"Cannot rename: 1 problem.\" Fix it, apply, undo.",
    "From a shell: {RENAME}",
    "It ends: \"Nothing was renamed.\"",
    "To be checked: RA's editor words and CX's diro rename.",
    "",
    "Step 3, compare two folders. The palette, Compare folders.",
    "Hear: \"5 differences in 10 files\", \"2 different, 1 only here, 1 only there\".",
    "Hear: \"1 type differs. 5 same.\"",
    "Rows: \"Different, notes.txt, newer here\", \"Different, todo.txt, newer there\".",
    "Rows: \"Only here, plan.md\", \"Only there, old.txt\", \"Type differs, thing, file here\".",
    "Enter on the first: the file compare's verdict line, as alpha 3 says it.",
    "Listen for: d.txt, one second apart, counted as same.",
    "To be checked: CS's and SA's compare words.",
    "",
    "Step 4, synchronize from the plan. The palette, Synchronize.",
    "Choose \"Update the other panel\". Hear: \"Plan: 1 copy, 1 replace, 3 skipped\".",
    "Rows: \"Copy there: plan.md, only here\", \"Replace there: notes.txt, newer here\".",
    "Row: \"Skip: todo.txt, newer there\".",
    "Space on the replace row: \"Skipped by you: notes.txt\". Space again.",
    "Run: \"Synchronize 2 items? y or n\". y: \"Synchronize finished: 2 done\".",
    "The old notes.txt is in the trash; Ctrl+Z puts both sides back.",
    "Choose \"Mirror to the other panel\": \"Trash there: old.txt, only there\".",
    "Press Escape: nothing changes.",
    "To be checked: CS's and SA's synchronize words.",
    "",
    "Step 5, duplicates. In {PATH}\\dups, the palette, Find duplicates, This folder.",
    "Hear: \"3 groups, 7 files, 44 KB to free\", \"2 empty files left out\".",
    "Hear: \"Group 1 of 3, 3 copies, 16 KB each\".",
    "Enter: \"crow.png, copy 1 of 3, in photos/a\".",
    "Keep oldest: \"Marked 4 files, 44 KB. 3 kept.\"",
    "Space on a group's last unmarked copy: \"One copy must stay.\"",
    "F8: the plan lists four files and the question starts on No. y.",
    "Ctrl+Z restores them.",
    "Listen for: near-a.bin and near-b.bin, alike at both ends, in no group.",
    "Listen for: linked/one.txt and linked/two.txt, one file, in no group.",
    "To be checked: DU's and DA's duplicates words.",
    "",
    "Step 6, flat view. In {PATH}\\flat\\work, the palette, Flat view.",
    "Hear: \"Flat view on, 14 files in 4 folders\".",
    "Two rows share a name: \"readme.txt, in a\", \"readme.txt, in b/deep\".",
    "Show in its folder on the second. Off: \"Flat view off. work.\"",
    "To be checked: FC's and LA's flat view words.",
    "",
    "Step 7, collections. Go menu, Collections.",
    "Hear: \"Trip photos, collection, 3 items\".",
    "Hear: \"Large images, saved search, runs on open\".",
    "Open the first: \"Missing: gone.txt, in old/place\".",
    "F8 on an item: \"Removed 1 from Trip photos.\", \"The file was not touched.\"",
    "The file is still there. Mark two files in a panel, Save as collection.",
    "To be checked: FC's and LA's collection words.",
    "",
    "Step 8, a list from a pipe. In cmd.exe: {PIPE}",
    "Hear: \"3 paths from standard input\".",
    "To be checked: FC's NUL reader and CX's diro -.",
    "",
    "Step 9, recent files. In {PATH}\\open, Enter on report.md, F4 on notes.txt.",
    "The palette, Recent files: \"Recent: 2 files, newest notes.txt\".",
    "Clear: \"Recent files cleared.\" Settings: turn the list off.",
    "To be checked: FC's and LA's recent files words.",
];

/// The steps' text for the demo folder.
pub fn steps_text(folder: &Path) -> String {
    let path = folder.display().to_string();
    let mut text = String::new();
    for line in STEPS {
        let line = line
            .replace("{START}", &start_line(folder))
            .replace("{RENAME}", &rename_line(folder))
            .replace("{PIPE}", &pipe_line(folder))
            .replace("{PATH}", &path);
        text.push_str(&line);
        text.push('\n');
    }
    text
}

// ---------------------------------------------------------------------
// the command

/// What the command made: its lines and whether every part was made.
pub struct Made {
    /// One line a fixture, then the steps file and the start line.
    pub lines: Vec<String>,
    /// Whether every fixture was made as asked.
    pub all_good: bool,
}

/// The files under `path` and their total size, the `.git` folders left
/// out.
fn files_under(path: &Path) -> (u64, u64) {
    let Ok(read) = fs::read_dir(path) else {
        return (0, 0);
    };
    let mut files = 0;
    let mut bytes = 0;
    for item in read.flatten() {
        let Ok(meta) = item.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if item.file_name() != ".git" {
                let (f, b) = files_under(&item.path());
                files += f;
                bytes += b;
            }
        } else {
            files += 1;
            bytes += meta.len();
        }
    }
    (files, bytes)
}

/// A fixture's line: its name, how many files, their size.
fn fixture_line(folder: &Path, name: &str) -> String {
    let path = folder.join(name);
    if path.is_file() {
        let bytes = fs::metadata(&path).map_or(0, |meta| meta.len());
        return format!("{name}, a file, {}", human_size(bytes));
    }
    let (files, bytes) = files_under(&path);
    match files {
        0 => format!("{name}, empty folder"),
        1 => format!("{name}, 1 file, {}", human_size(bytes)),
        n => format!("{name}, {n} files, {}", human_size(bytes)),
    }
}

/// Step 2's shell line: a rename tried without renaming.
pub fn rename_line(folder: &Path) -> String {
    format!(
        "diro rename --in {} --find IMG_ --replace trip- --dry-run",
        folder.join("rename").display()
    )
}

/// Step 8's cmd.exe line: `paths.nul` fed to `diro -`.
pub fn pipe_line(folder: &Path) -> String {
    format!("diro - < {}", folder.join("paths.nul").display())
}

/// The line that starts the check: the two sides of `sync`.
pub fn start_line(folder: &Path) -> String {
    let sync = folder.join("sync");
    format!(
        "diro --portable {} {}",
        sync.join("here").display(),
        sync.join("there").display()
    )
}

/// Writes every fixture into `folder`, staging in `staging`, git's only
/// when `with_git` and git is there.
pub fn build(folder: &Path, staging: &Path, with_git: bool) -> Result<Made> {
    let made5 = alpha5::build(folder)?;
    photos(&folder.join("photos"))?;
    mixed(&folder.join("mixed"))?;
    let music = folder.join("music");
    make_folder(&music)?;
    write_file(&music.join("song.mp3"), &song_mp3(), taken())?;
    docs(&folder.join("docs"))?;
    download(&folder.join("download"), staging)?;
    let git_made = if with_git && git_present() {
        repositories(folder, staging)?;
        true
    } else {
        false
    };
    disk(&folder.join("disk"))?;
    hot(&folder.join("hot"))?;
    make_folder(&folder.join("out"))?;
    write_file(
        &folder.join(STEPS_FILE),
        steps_text(folder).as_bytes(),
        taken(),
    )?;
    let mut lines = Vec::new();
    for name in FIXTURES {
        if !git_made && matches!(name, "repo" | "repo-hook") {
            lines.push(format!(
                "{name}, left out: git was not found, so step 8 cannot be heard"
            ));
            continue;
        }
        let line = fixture_line(folder, name);
        lines.push(match alpha5::note(name, &made5) {
            Some(note) => format!("{line}; {note}"),
            None => line,
        });
    }
    lines.push(format!("{STEPS_FILE}, the owner's steps in plain lines"));
    lines.push(format!("Step 2's shell line: {}", rename_line(folder)));
    lines.push(format!("Step 8's cmd.exe line: {}", pipe_line(folder)));
    lines.push(format!("Start the check: {}", start_line(folder)));
    Ok(Made {
        lines,
        all_good: true,
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

/// `cargo xtask demo-tools PATH`.
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
    make_folder(&folder)?;
    let work = target_folder(root).join("demo-tools");
    make_folder(&work)?;
    let staging = tempfile::Builder::new()
        .prefix("staging-")
        .tempdir_in(&work)
        .with_context(|| format!("no staging folder could be made in {}", work.display()))?;
    println!("demo-tools: writing the demo set in {}.", folder.display());
    let made = build(&folder, staging.path(), true)?;
    staging
        .close()
        .context("the staging folder could not be removed")?;
    for line in &made.lines {
        println!("{line}");
    }
    Ok(made.all_good)
}

/// The demo folder's every file, by its path relative to it with `/`.
#[cfg(test)]
fn every_file(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in fs::read_dir(&dir).unwrap().flatten() {
            let path = item.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests;
