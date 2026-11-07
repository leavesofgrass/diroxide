//! The 7z tool backend (alpha 4, R30 commit 80; R29 1.4 and 6.1): RAR,
//! CAB, ARJ, LZH, RPM, cpio and ar, a spanned zip set, and a disk image no
//! plugin claims, read through the 7-Zip program the tools registry finds
//! (`7z`, `7za` or `7zz`), always through the job engine's one runner
//! ([`diroxide_vfs::ToolRunner`]), never a shell line. Read only: diroxide
//! never writes RAR (the owner's answer 116).
//!
//! - **The index** comes from `7z l -slt -spd -sccUTF-8 -- ARCHIVE`: the
//!   banner first (the program's version), then the archive's own block
//!   (its type, whether it is solid), then a block of `Key = value` lines
//!   an item ([`parse_slt`], fuzzed). Spike DS3 fixed the forms: the same
//!   keys on 7-Zip 24.08, 25.01 and 26.05, `\` between names on Windows,
//!   CR LF line ends there, and `-sccUTF-8` for names that are not ASCII.
//!   The listing is run without `-ba`, so one run gives the version, the
//!   type and the solid flag as well as the items.
//! - **A member** comes back on standard output from `x -so`, which
//!   writes no file, when it fits [`IN_MEMORY`]; a larger one is unpacked
//!   alone into a staging folder diroxide makes inside the spool. Members
//!   are sequential (`random_read` false), so the "Read 3.2 GB first to
//!   open note.txt?" question applies to a solid archive.
//! - **A whole unpack** runs `x -o STAGING -y` once into a staging folder
//!   inside the spool; the unpack runner then reads each member from it
//!   through the limiter chain and writes it with its usual name checks.
//!   A link in the staging folder is refused, never followed, and the
//!   staging folder is removed when the pass ends.
//! - **The version gate.** Anything that writes files (the staging road)
//!   needs 7-Zip 25.01 or later, which fixed two link path-traversal bugs
//!   (R7); an older program still lists and reads members through the
//!   pipe, and the unpack says "7-Zip 24.08 is too old: needs 25.01".
//! - **A password** is never on a command line or in a log: spike DS4
//!   found that with no `-p` switch 7-Zip 24.08, 25.01 and 26.05 read it
//!   from standard input, for `x -so` and for a listing whose names are
//!   locked, so the masked prompt's answer goes there. `-p` alone would
//!   mean an empty password, so it is never passed.
//! - **A hang or a crash** of the program becomes words through the
//!   runner's limits: a time limit and an output cap on every run.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use diroxide_core::{Secret, SecretKind, SecretRequest};
use diroxide_vfs::{
    OpContext, RunLimits, ToolInput, ToolOutput, ToolRequest, ToolRunError, VfsError,
};

use crate::error::ArchiveError;
use crate::guard::refuse;
use crate::index::{Index, IndexBuilder, Locator, Member, RecordFlags, RecordKind};
use crate::limits::{Limits, limit_words};
use crate::sniff::{DISK_IMAGE_NEEDS, Signature};
use crate::spool::SpoolFile;

/// The tools registry's id of 7-Zip.
pub const TOOL: &str = "7z";

/// The largest member read through the pipe into memory; a larger one,
/// or one whose size is not listed, takes the staging road.
pub const IN_MEMORY: u64 = 64 << 20;

/// The start of every staging folder's name, inside the spool.
pub const STAGING_PREFIX: &str = "diroxide-stage-";

/// How long a listing may run.
const LIST_TIME: Duration = Duration::from_secs(300);

/// The most a listing may print: about 600,000 items.
const LIST_OUTPUT: usize = 128 << 20;

/// A version of 7-Zip, from its banner: "7-Zip 24.08 (x64) : ...",
/// "7-Zip (a) 26.05 (x64) : ...", "p7zip Version 16.02 (locale=...".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SevenZipVersion {
    /// The major number.
    pub major: u32,
    /// The minor number.
    pub minor: u32,
}

impl SevenZipVersion {
    /// The first version whose unpacking to disk is trusted: 25.00 fixed
    /// two link path-traversal bugs (R7), and R29 names 25.01.
    pub const EXTRACT: SevenZipVersion = SevenZipVersion {
        major: 25,
        minor: 1,
    };

    /// The version in its usual words: "24.08".
    pub fn words(self) -> String {
        format!("{}.{:02}", self.major, self.minor)
    }

    /// Whether it may unpack to disk.
    pub fn extracts(self) -> bool {
        self >= Self::EXTRACT
    }

    /// The version a banner names, from the first line that starts with
    /// "7-Zip" or "p7zip Version".
    pub fn from_banner(banner: &str) -> Option<Self> {
        let rest = banner.lines().map(str::trim).find_map(|line| {
            line.strip_prefix("7-Zip")
                .or_else(|| line.strip_prefix("p7zip Version"))
        })?;
        let number = rest
            .split_whitespace()
            .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
        let (major, minor) = number.split_once('.')?;
        let minor: String = minor.chars().take_while(char::is_ascii_digit).collect();
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }
}

/// The words when 7-Zip is too old to unpack to disk, the program first:
/// "7-Zip 24.08 is too old: needs 25.01".
pub fn too_old_line(version: SevenZipVersion) -> String {
    format!(
        "7-Zip {} is too old: needs {}",
        version.words(),
        SevenZipVersion::EXTRACT.words()
    )
}

/// The title's kind for an archive read through 7-Zip: "Backup.rar, read
/// with 7-Zip".
pub fn read_with_line(name: &str) -> String {
    format!("{name}, read with 7-Zip")
}

/// One item of a `-slt` listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SltItem {
    /// Its path as 7-Zip printed it (`\` between names on Windows).
    pub path: String,
    /// Whether it is a folder.
    pub folder: bool,
    /// Its size, when listed.
    pub size: Option<u64>,
    /// The bytes it takes packed, when listed.
    pub packed: Option<u64>,
    /// When it last changed, in seconds since the epoch, read as UTC.
    pub modified: Option<i64>,
    /// Its Unix mode bits, when listed.
    pub mode: Option<u32>,
    /// A symbolic link's target.
    pub link: Option<String>,
    /// Whether it is a hard link, which is listed and never read.
    pub hard_link: bool,
    /// Whether its data is encrypted.
    pub encrypted: bool,
    /// Its CRC-32, when listed.
    pub crc32: Option<u32>,
}

/// A `-slt` listing: the program's version, the archive's type and solid
/// flag, and the items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SltListing {
    /// The version the banner names.
    pub version: Option<SevenZipVersion>,
    /// The archive's type as 7-Zip names it: "Rar5", "Cab", "Iso".
    pub kind: Option<String>,
    /// Whether the archive is solid, so reading one member decodes those
    /// before it.
    pub solid: bool,
    /// The items, in the archive's order.
    pub items: Vec<SltItem>,
}

/// The line that ends the archive's own block and starts the items.
const ITEMS_START: &str = "----------";

/// Parses a `7z l -slt` listing, with or without its banner and archive
/// block. A block of lines between blank lines is an item when it has a
/// `Path`; a line without ` = ` is passed over, so nothing 7-Zip adds stops
/// the parse; nothing in the text can make it panic.
pub fn parse_slt(text: &str) -> SltListing {
    let mut listing = SltListing::default();
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let start = lines.iter().position(|l| *l == ITEMS_START);
    let (head, body) = match start {
        Some(at) => (&lines[..at], &lines[at + 1..]),
        None => (&lines[..0], &lines[..]),
    };
    if start.is_some() {
        listing.version = SevenZipVersion::from_banner(&head.join("\n"));
        for line in head {
            match line.split_once(" = ") {
                Some(("Type", kind)) => listing.kind = Some(kind.to_owned()),
                Some(("Solid", flag)) => listing.solid = flag == "+",
                _ => {}
            }
        }
    }
    let mut item: Option<SltItem> = None;
    for line in body.iter().copied().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if let Some(done) = item.take()
                && !done.path.is_empty()
            {
                listing.items.push(done);
            }
            continue;
        }
        let (key, value) = match line.split_once(" = ") {
            Some(pair) => pair,
            None => match line.strip_suffix(" =") {
                Some(key) => (key, ""),
                None => continue,
            },
        };
        let current = item.get_or_insert_with(SltItem::default);
        match key {
            "Path" => current.path = value.to_owned(),
            "Folder" => current.folder = value == "+",
            "Size" => current.size = value.parse().ok(),
            "Packed Size" => current.packed = value.parse().ok(),
            "Modified" => current.modified = slt_time(value),
            "Mode" => current.mode = current.mode.or_else(|| unix_mode(value)),
            "Attributes" => {
                current.folder |= value.split_whitespace().next().is_some_and(|f| {
                    f.contains('D') && f.chars().all(|c| c.is_ascii_uppercase() || c == '.')
                });
                if current.mode.is_none() {
                    current.mode = value.split_whitespace().find_map(unix_mode);
                }
            }
            "Symbolic Link" if !value.is_empty() => current.link = Some(value.to_owned()),
            "Hard Link" if !value.is_empty() => current.hard_link = true,
            "Encrypted" => current.encrypted = value == "+",
            // Eight hex digits are a CRC-32; LZH lists a CRC-16 in four,
            // which the limiter chain must not hold a CRC-32 to.
            "CRC" if value.len() == 8 => current.crc32 = u32::from_str_radix(value, 16).ok(),
            _ => {}
        }
    }
    listing
}

/// Unix mode bits from `drwxr-xr-x` or `-rw-r--r--`; a folder's or link's
/// type letter is left to the other keys.
fn unix_mode(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || !matches!(bytes[0], b'-' | b'd' | b'l' | b'b' | b'c' | b'p' | b's') {
        return None;
    }
    let mut mode = 0u32;
    for (i, &b) in bytes[1..].iter().enumerate() {
        let want = [b'r', b'w', b'x'][i % 3];
        let set = match b {
            b'-' => false,
            b's' | b't' if i % 3 == 2 => true,
            b'S' | b'T' if i % 3 == 2 => false,
            other if other == want => true,
            _ => return None,
        };
        if set {
            mode |= 1 << (8 - i);
        }
    }
    Some(mode)
}

/// A `-slt` time, `2026-11-06 12:48:43` with an optional fraction, read as
/// UTC (7-Zip prints the archive's own time, whose zone it does not know).
pub fn slt_time(text: &str) -> Option<i64> {
    let (date, time) = text.trim().split_once(' ')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let time = time.split('.').next()?;
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
        || !(1..=9999).contains(&year)
    {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Builds the index of a listing. Each file's record points to its item
/// by number, so a member is asked for by the path 7-Zip printed.
pub(crate) fn index_of(listing: &SltListing, limits: &Limits) -> Result<Index, ArchiveError> {
    let mut builder = IndexBuilder::new(*limits);
    for (n, item) in listing.items.iter().enumerate() {
        let kind = if item.folder {
            RecordKind::Folder
        } else if item.link.is_some() {
            RecordKind::Link
        } else if item.hard_link {
            RecordKind::Special
        } else {
            RecordKind::File
        };
        let mut flags = RecordFlags::NONE;
        if item.encrypted {
            flags.insert(RecordFlags::ENCRYPTED);
        }
        if kind == RecordKind::File && item.size.is_none() {
            flags.insert(RecordFlags::SIZE_UNKNOWN);
        }
        builder.add(Member {
            name: item.path.clone(),
            not_unicode: item.path.contains('\u{FFFD}'),
            kind: Some(kind),
            size: item.size.unwrap_or(0),
            packed: item.packed,
            modified: item.modified,
            mode: item.mode,
            crc32: item.crc32,
            link_target: item.link.clone(),
            locator: if kind == RecordKind::File {
                Locator::Tool(n as u32)
            } else {
                Locator::None
            },
            flags,
            note: None,
        })?;
    }
    Ok(builder.finish())
}

/// An archive read through 7-Zip: the file it reads (a spool copy of a
/// container that is not a local file), each item's path as printed, and
/// what the listing said.
pub(crate) struct ToolBackend {
    path: PathBuf,
    _spooled: Option<SpoolFile>,
    paths: Vec<String>,
    ahead: Vec<u64>,
    solid: bool,
    version: Option<SevenZipVersion>,
    kind: Option<String>,
    signature: Signature,
    password: Option<Secret>,
    spool_dir: Option<PathBuf>,
}

impl fmt::Debug for ToolBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolBackend")
            .field("path", &self.path)
            .field("items", &self.paths.len())
            .field("solid", &self.solid)
            .field("version", &self.version)
            .field("signature", &self.signature)
            .field("holds_a_password", &self.password.is_some())
            .finish_non_exhaustive()
    }
}

/// The text a run printed, both streams.
fn said(output: &ToolOutput) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

/// Whether a failed run failed for want of the right password.
fn wants_password(output: &ToolOutput) -> bool {
    let text = said(output).to_lowercase();
    text.contains("wrong password")
        || text.contains("encrypted archive")
        || text.contains("enter password")
}

/// A failed run in words, for "Could not open Backup.rar: CAUSE.".
fn failure(output: &ToolOutput, signature: Signature) -> VfsError {
    let text = said(output).to_lowercase();
    if text.contains("can not open the file as archive")
        || text.contains("cannot open the file as archive")
        || text.contains("is not archive")
    {
        return VfsError::NeedsHelper {
            what: format!("a 7-Zip that reads {}", signature.short()),
        };
    }
    if text.contains("unsupported method") {
        return VfsError::UnsupportedFeature {
            what: "a method this 7-Zip does not read".to_owned(),
        };
    }
    if text.contains("data error")
        || text.contains("crc failed")
        || text.contains("headers error")
        || text.contains("unexpected end")
    {
        return VfsError::Corrupt {
            what: "7-Zip found damaged data".to_owned(),
        };
    }
    VfsError::Io {
        kind: io::ErrorKind::Other,
        message: match output.status {
            Some(code) => format!("7-Zip stopped with exit code {code}"),
            None => "7-Zip stopped unexpectedly".to_owned(),
        },
    }
}

/// A runner error in words; "not installed" names what the format needs.
fn run_failure(error: ToolRunError, signature: Signature) -> VfsError {
    match error {
        ToolRunError::NotInstalled { .. } => VfsError::NeedsHelper {
            what: if signature.is_disk_image() {
                DISK_IMAGE_NEEDS.to_owned()
            } else {
                "7-Zip".to_owned()
            },
        },
        other => other.into(),
    }
}

/// Standard input for a run: the password and a line feed, or nothing.
fn input_of(password: Option<&Secret>) -> ToolInput {
    match password {
        Some(secret) => ToolInput::Bytes(format!("{}\n", secret.expose()).into_bytes()),
        None => ToolInput::Closed,
    }
}

/// Runs 7-Zip with these arguments through the context's runner.
fn run(
    ctx: &OpContext,
    args: Vec<OsString>,
    password: Option<&Secret>,
    limits: RunLimits,
    signature: Signature,
) -> Result<ToolOutput, VfsError> {
    let request = ToolRequest::new(TOOL, args)
        .with_input(input_of(password))
        .with_limits(limits);
    ctx.tools
        .run(&request, &ctx.cancel)
        .map_err(|e| run_failure(e, signature))
}

/// The listing's arguments.
fn list_args(path: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["l", "-slt", "-spd", "-sccUTF-8", "--"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push(path.as_os_str().to_owned());
    args
}

/// Seconds a run may take for this many bytes: a base and one second for
/// every 2 MiB.
fn time_for(base: u64, bytes: u64) -> Duration {
    Duration::from_secs(base.saturating_add(bytes / (2 << 20)))
}

impl ToolBackend {
    /// Lists `path` through 7-Zip and builds the index. A listing whose
    /// names are locked asks the password once, through the context.
    /// `spooled` keeps a spool copy alive while the archive is open.
    pub(crate) fn open(
        path: PathBuf,
        spooled: Option<SpoolFile>,
        signature: Signature,
        name: &str,
        limits: &Limits,
        ctx: &OpContext,
        spool_dir: Option<PathBuf>,
    ) -> Result<(Index, Self), ArchiveError> {
        let list_limits = RunLimits::new(LIST_TIME, LIST_OUTPUT);
        let mut password = None;
        let mut output = run(ctx, list_args(&path), None, list_limits, signature)?;
        if !output.succeeded() && wants_password(&output) {
            let request = SecretRequest::new(SecretKind::ArchivePassword, name);
            let secret = ctx.ask_secret(&request).ok_or(VfsError::PasswordRequired)?;
            output = run(ctx, list_args(&path), Some(&secret), list_limits, signature)?;
            if !output.succeeded() && wants_password(&output) {
                return Err(VfsError::WrongPassword.into());
            }
            password = Some(secret);
        }
        // Exit code 1 is a warning: the listing stands.
        if !matches!(output.status, Some(0 | 1)) {
            return Err(failure(&output, signature).into());
        }
        let listing = parse_slt(&String::from_utf8_lossy(&output.stdout));
        let index = index_of(&listing, limits)?;
        let mut ahead = Vec::with_capacity(listing.items.len());
        let mut before = 0u64;
        for item in &listing.items {
            ahead.push(if listing.solid { before } else { 0 });
            if !item.folder {
                before = before.saturating_add(item.size.unwrap_or(0));
            }
        }
        let backend = Self {
            path,
            _spooled: spooled,
            paths: listing.items.into_iter().map(|i| i.path).collect(),
            ahead,
            solid: listing.solid,
            version: listing.version,
            kind: listing.kind,
            signature,
            password,
            spool_dir,
        };
        Ok((index, backend))
    }

    /// The program's version, when its banner said it.
    pub(crate) fn version(&self) -> Option<SevenZipVersion> {
        self.version
    }

    /// The archive's type as 7-Zip named it.
    pub(crate) fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }

    /// The password the listing needed, for the members too.
    pub(crate) fn password(&self) -> Option<&Secret> {
        self.password.as_ref()
    }

    /// The bytes decoded before item `n` when it is read alone: those
    /// before it in a solid archive, none otherwise.
    pub(crate) fn ahead(&self, n: u32) -> u64 {
        self.ahead.get(n as usize).copied().unwrap_or(0)
    }

    /// Why this program may not unpack to disk, in words: too old, or a
    /// version its banner did not say.
    pub(crate) fn unpack_refusal(&self) -> Option<String> {
        match self.version {
            Some(version) if version.extracts() => None,
            Some(version) => Some(too_old_line(version)),
            None => Some(format!(
                "7-Zip's version is unknown: needs {}",
                SevenZipVersion::EXTRACT.words()
            )),
        }
    }

    /// The refusal as an error, whose next step is the components list:
    /// "it needs 7-Zip 25.01; 24.08 is too old".
    fn gate(&self) -> Result<(), VfsError> {
        let needed = SevenZipVersion::EXTRACT.words();
        match self.version {
            Some(version) if version.extracts() => Ok(()),
            Some(version) => Err(VfsError::NeedsHelper {
                what: format!("7-Zip {needed}; {} is too old", version.words()),
            }),
            None => Err(VfsError::NeedsHelper {
                what: format!("7-Zip {needed}; this one does not say its version"),
            }),
        }
    }

    /// Reads item `n` (claimed `size` bytes): through the pipe into memory
    /// when it fits, else unpacked alone into a staging folder.
    pub(crate) fn read(
        &self,
        n: u32,
        size: Option<u64>,
        password: Option<&Secret>,
        ctx: &OpContext,
    ) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let raw = self.paths.get(n as usize).ok_or(VfsError::NotFound)?;
        match size {
            Some(size) if size <= IN_MEMORY => {
                let mut args: Vec<OsString> = ["x", "-so", "-spd", "-sccUTF-8", "-bd", "--"]
                    .iter()
                    .map(OsString::from)
                    .collect();
                args.push(self.path.as_os_str().to_owned());
                args.push(OsString::from(raw));
                let limits = RunLimits::new(
                    time_for(120, size),
                    usize::try_from(size + (1 << 20)).unwrap_or(usize::MAX),
                );
                let output = run(ctx, args, password, limits, self.signature)?;
                if !output.succeeded() {
                    return Err(self.member_failure(&output, password).into());
                }
                Ok(Box::new(Cursor::new(output.stdout)))
            }
            _ => self.read_staged(raw, size.unwrap_or(0), password, ctx),
        }
    }

    /// A failed member read in words: the password first.
    fn member_failure(&self, output: &ToolOutput, password: Option<&Secret>) -> VfsError {
        if wants_password(output) {
            if password.is_some() {
                VfsError::WrongPassword
            } else {
                VfsError::PasswordRequired
            }
        } else {
            failure(output, self.signature)
        }
    }

    /// Makes a new staging folder in the spool.
    fn staging(&self) -> Result<Staging, VfsError> {
        let Some(dir) = self.spool_dir.as_deref() else {
            return Err(refuse(
                "holding a member in memory",
                format!("{}, and no spool folder is set", limit_words(IN_MEMORY)),
            )
            .into_error());
        };
        Staging::new(dir)
    }

    /// Unpacks item `raw` alone into a staging folder (25.01 or later) and
    /// reads it from there; the folder goes when the reader is dropped.
    fn read_staged(
        &self,
        raw: &str,
        size: u64,
        password: Option<&Secret>,
        ctx: &OpContext,
    ) -> Result<Box<dyn Read + Send>, ArchiveError> {
        self.gate()?;
        let staging = self.staging()?;
        let output = self.extract(ctx, &staging, Some(raw), size, password)?;
        if !output.succeeded() {
            return Err(self.member_failure(&output, password).into());
        }
        let files = staging.files()?;
        match files.as_slice() {
            [one] => {
                let file = File::open(one).map_err(VfsError::from)?;
                Ok(Box::new(StagedFile {
                    file,
                    _staging: staging,
                }))
            }
            _ => Err(VfsError::Corrupt {
                what: "7-Zip did not unpack the member alone".to_owned(),
            }
            .into()),
        }
    }

    /// Runs `x -o STAGING -y` for one item, or for all of them.
    fn extract(
        &self,
        ctx: &OpContext,
        staging: &Staging,
        raw: Option<&str>,
        bytes: u64,
        password: Option<&Secret>,
    ) -> Result<ToolOutput, VfsError> {
        let mut out = OsString::from("-o");
        out.push(staging.path.as_os_str());
        let mut args: Vec<OsString> = vec![OsString::from("x"), out];
        args.extend(
            ["-y", "-bd", "-bso0", "-bsp0", "-spd", "-sccUTF-8", "--"]
                .iter()
                .map(OsString::from),
        );
        args.push(self.path.as_os_str().to_owned());
        if let Some(raw) = raw {
            args.push(OsString::from(raw));
        }
        let limits = RunLimits::new(time_for(600, bytes), 1 << 20);
        run(ctx, args, password, limits, self.signature)
    }

    /// Unpacks every item into a new staging folder for a whole unpack
    /// (25.01 or later): the folder the visit reads from, removed when it
    /// is dropped.
    pub(crate) fn extract_all(
        &self,
        ctx: &OpContext,
        bytes: u64,
        password: Option<&Secret>,
    ) -> Result<Staging, VfsError> {
        self.gate()?;
        let staging = self.staging()?;
        let output = self.extract(ctx, &staging, None, bytes, password)?;
        // Exit code 1 is a warning; the members it wrote are read, each
        // checked against its listed size and CRC on the way.
        if !matches!(output.status, Some(0 | 1)) {
            return Err(self.member_failure(&output, password));
        }
        Ok(staging)
    }
}

/// A staging folder in the spool, made new for one run and removed with
/// everything in it when dropped. Nothing in it is followed: a link is
/// refused where a member is looked for.
pub(crate) struct Staging {
    path: PathBuf,
}

impl Staging {
    /// Makes a new staging folder with a plain name in `dir`.
    fn new(dir: &Path) -> Result<Self, VfsError> {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        for attempt in 0..64u32 {
            let name = format!(
                "{STAGING_PREFIX}{}-{stamp:08x}-{attempt}",
                std::process::id()
            );
            let path = dir.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(VfsError::from(e)),
            }
        }
        Err(VfsError::AlreadyExists)
    }

    /// Every regular file in the folder, walked without following links;
    /// a link anywhere is refused.
    fn files(&self) -> Result<Vec<PathBuf>, VfsError> {
        let mut found = Vec::new();
        let mut stack = vec![self.path.clone()];
        while let Some(folder) = stack.pop() {
            for item in fs::read_dir(&folder).map_err(VfsError::from)? {
                let item = item.map_err(VfsError::from)?;
                let kind = item.file_type().map_err(VfsError::from)?;
                if kind.is_symlink() {
                    return Err(link_refused());
                } else if kind.is_dir() {
                    stack.push(item.path());
                } else if kind.is_file() {
                    found.push(item.path());
                }
            }
        }
        Ok(found)
    }

    /// Opens a member's file by its safe path in the index (`/` between
    /// names): each folder on the way and the file itself checked, a link
    /// refused, never followed.
    pub(crate) fn open(&self, member: &str) -> Result<File, VfsError> {
        let mut at = self.path.clone();
        for part in member.split('/').filter(|p| !p.is_empty()) {
            at.push(part);
            let meta = fs::symlink_metadata(&at).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => VfsError::Corrupt {
                    what: "7-Zip did not unpack it".to_owned(),
                },
                _ => VfsError::from(e),
            })?;
            if meta.file_type().is_symlink() {
                return Err(link_refused());
            }
        }
        File::open(&at).map_err(VfsError::from)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // The folder this backend made, by the path it made it at; the
        // standard library's removal does not follow links.
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// The refusal of a link 7-Zip made in a staging folder.
fn link_refused() -> VfsError {
    VfsError::Unsafe {
        why: "7-Zip made a link in the staging folder".to_owned(),
    }
}

/// A member unpacked alone: its file, and the staging folder it is in,
/// removed when the reader is dropped.
struct StagedFile {
    file: File,
    _staging: Staging,
}

impl Read for StagedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

/// A reader that gives one error at its first read: a member the visit
/// could not open, said through the runner's usual failure road.
pub(crate) struct Failing(pub(crate) Option<VfsError>);

impl Read for Failing {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.0.take() {
            Some(error) => Err(crate::guard::to_io(error)),
            None => Ok(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A RAR 5 block as 7-Zip 24.08 prints it on Windows (the shape of
    /// DS3's samples), CR LF and backslashes.
    const RAR: &str = "\r\n7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11\r\n\r\nScanning the drive for archives:\r\n1 file, 610 bytes (1 KiB)\r\n\r\nListing archive: D:\\x\\Backup.rar\r\n\r\n--\r\nPath = D:\\x\\Backup.rar\r\nType = Rar5\r\nPhysical Size = 610\r\nSolid = +\r\nBlocks = 1\r\nEncrypted = -\r\n\r\n----------\r\nPath = docs\r\nFolder = +\r\nSize = 0\r\nPacked Size = 0\r\nModified = 2026-11-06 12:48:43.1234567\r\nAttributes = D\r\nEncrypted = -\r\n\r\nPath = docs\\notes.txt\r\nFolder = -\r\nSize = 13\r\nPacked Size = 13\r\nModified = 2026-11-06 12:48:43\r\nAttributes = A -rw-r--r--\r\nEncrypted = +\r\nCRC = 0A1B2C3D\r\n\r\nPath = a = b.txt\r\nSize = 2\r\nSymbolic Link = docs\r\n\r\n";

    #[test]
    fn a_listing_gives_its_version_type_and_items() {
        let listing = parse_slt(RAR);
        assert_eq!(
            listing.version.map(SevenZipVersion::words).as_deref(),
            Some("24.08")
        );
        assert_eq!(listing.kind.as_deref(), Some("Rar5"));
        assert!(listing.solid);
        let paths: Vec<_> = listing.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["docs", "docs\\notes.txt", "a = b.txt"]);
        assert!(listing.items[0].folder);
        let notes = &listing.items[1];
        assert_eq!(notes.size, Some(13));
        assert!(notes.encrypted);
        assert_eq!(notes.crc32, Some(0x0A1B_2C3D));
        assert_eq!(notes.mode, Some(0o644));
        assert_eq!(notes.modified, Some(1_793_969_323));
        assert_eq!(listing.items[2].link.as_deref(), Some("docs"));
        // An LZH's CRC-16 is not taken for a CRC-32.
        let lzh = parse_slt("----------\nPath = a.txt\nSize = 1\nCRC = 1A2B\n");
        assert_eq!(lzh.items[0].crc32, None);
        let index = index_of(&listing, &Limits::default()).unwrap();
        assert_eq!(index.summary().members, 2);
        let id = index.find("docs/notes.txt").unwrap();
        assert_eq!(index.record(id).unwrap().locator, Locator::Tool(1));
        assert!(
            index
                .record(id)
                .unwrap()
                .flags
                .contains(RecordFlags::ENCRYPTED)
        );
    }

    #[test]
    fn hostile_listings_never_panic_and_never_climb() {
        for text in [
            "",
            "----------",
            "----------\nPath = ../../etc/passwd\nSize = 1\n",
            "Path = \nSize = 99999999999999999999999\n",
            "----------\nPath = x\nModified = 9999-99-99 99:99:99\nMode = drwxrwxrwxx\n",
            "7-Zip\n----------\n=\n = \nPath = a\nCRC = zz\n",
        ] {
            let listing = parse_slt(text);
            let index = index_of(&listing, &Limits::default()).unwrap();
            assert!(index.find("etc/passwd").is_none() || index.summary().refused == 0);
            assert!(index.find("../../etc/passwd").is_none());
        }
    }

    #[test]
    fn banners_and_modes_and_times() {
        let cases = [
            ("7-Zip 25.01 (x64) : Copyright", (25, 1)),
            ("\n7-Zip (a) 26.05 (x64) : Copyright", (26, 5)),
            ("p7zip Version 16.02 (locale=C.UTF-8)", (16, 2)),
        ];
        for (banner, (major, minor)) in cases {
            assert_eq!(
                SevenZipVersion::from_banner(banner),
                Some(SevenZipVersion { major, minor })
            );
        }
        assert!(SevenZipVersion::from_banner("nothing").is_none());
        let old = SevenZipVersion {
            major: 24,
            minor: 9,
        };
        assert!(!old.extracts() && SevenZipVersion::EXTRACT.extracts());
        let line = too_old_line(old);
        assert_eq!(line, "7-Zip 24.09 is too old: needs 25.01");
        assert_eq!(line.chars().count(), 35);
        assert_eq!(unix_mode("drwxr-xr-x"), Some(0o755));
        assert_eq!(unix_mode("-rwsr-xr-t"), Some(0o755));
        assert_eq!(unix_mode("A"), None);
        assert_eq!(slt_time("1970-01-01 00:00:00"), Some(0));
        assert_eq!(slt_time("2026-02-30 00:00:00").map(|_| ()), Some(()));
        assert_eq!(slt_time("garbage"), None);
        let title = read_with_line("Backup.rar");
        assert_eq!(title, "Backup.rar, read with 7-Zip");
        assert_eq!(title.chars().count(), 27);
    }

    #[test]
    fn failures_are_said_in_words() {
        let out = |text: &str| ToolOutput::new(Some(2), Vec::new(), text.as_bytes().to_vec());
        let rar = Signature::Rar;
        assert_eq!(
            failure(&out("ERROR: x.rar\nCan not open the file as archive"), rar).to_string(),
            "it needs a 7-Zip that reads RAR"
        );
        assert_eq!(
            failure(&out("ERROR: Data Error : notes.txt"), rar).to_string(),
            "it is damaged: 7-Zip found damaged data"
        );
        assert_eq!(
            failure(&out("something else"), rar).to_string(),
            "7-Zip stopped with exit code 2"
        );
        assert!(wants_password(&out(
            "ERROR: x.rar\nCan not open encrypted archive. Wrong password?"
        )));
        let missing = ToolRunError::NotInstalled {
            tool: "7z".to_owned(),
            name: "7-Zip".to_owned(),
        };
        assert_eq!(
            run_failure(missing.clone(), rar).to_string(),
            "it needs 7-Zip"
        );
        assert_eq!(
            run_failure(missing, Signature::Iso).to_string(),
            "it needs the disk image plugin or 7-Zip"
        );
    }

    /// 7-Zip as a plain process for the tests that need the real program,
    /// recording every argument it was given.
    struct Program {
        path: PathBuf,
        seen: std::sync::Mutex<Vec<OsString>>,
    }

    impl diroxide_vfs::ToolRunner for Program {
        fn has(&self, tool: &str) -> bool {
            tool == TOOL
        }

        fn run(
            &self,
            request: &ToolRequest,
            _cancel: &diroxide_vfs::CancelToken,
        ) -> Result<ToolOutput, ToolRunError> {
            use std::io::Write;
            use std::process::{Command, Stdio};
            self.seen
                .lock()
                .unwrap()
                .extend(request.args.iter().cloned());
            let mut command = Command::new(&self.path);
            command
                .args(&request.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000);
            }
            let mut child = command.spawn().map_err(|e| ToolRunError::NotStarted {
                name: "7-Zip".to_owned(),
                message: e.to_string(),
            })?;
            let mut stdin = child.stdin.take();
            if let (ToolInput::Bytes(bytes), Some(stdin)) = (&request.input, stdin.as_mut()) {
                let _ = stdin.write_all(bytes);
            }
            drop(stdin);
            let output = child
                .wait_with_output()
                .map_err(|e| ToolRunError::NotStarted {
                    name: "7-Zip".to_owned(),
                    message: e.to_string(),
                })?;
            Ok(ToolOutput::new(
                output.status.code(),
                output.stdout,
                output.stderr,
            ))
        }

        fn run_streaming(
            &self,
            request: &ToolRequest,
            cancel: &diroxide_vfs::CancelToken,
            _record: &mut dyn FnMut(diroxide_vfs::ToolStream, &str) -> std::ops::ControlFlow<()>,
        ) -> Result<ToolOutput, ToolRunError> {
            self.run(request, cancel)
        }
    }

    /// The 7-Zip of `DIRO_TEST_7Z`, the image's `7z`, or the program
    /// folder's on Windows, with its version.
    fn seven_zip() -> Option<(PathBuf, SevenZipVersion)> {
        let mut tries: Vec<PathBuf> = Vec::new();
        if let Some(named) = std::env::var_os("DIRO_TEST_7Z") {
            tries.push(PathBuf::from(named));
        }
        tries.push(PathBuf::from("7z"));
        if cfg!(windows) {
            tries.push(PathBuf::from(r"C:\Program Files\7-Zip\7z.exe"));
        }
        tries.into_iter().find_map(|path| {
            let mut command = std::process::Command::new(&path);
            command.stdin(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000);
            }
            let output = command.output().ok()?;
            let version = SevenZipVersion::from_banner(&String::from_utf8_lossy(&output.stdout))?;
            Some((path, version))
        })
    }

    /// A prompt that types one password and counts its asks.
    struct Typed(&'static str, std::sync::atomic::AtomicU32);

    impl diroxide_vfs::SecretSource for Typed {
        fn ask(
            &self,
            _request: &SecretRequest,
            _cancel: &diroxide_vfs::CancelToken,
        ) -> Option<Secret> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(Secret::new(self.0.to_owned()))
        }
    }

    /// A 7z whose names and data are locked, by this crate's own writer.
    fn locked_7z(password: &str) -> Vec<u8> {
        use crate::request::CompressionLevel;
        use crate::write::{InMemory, ItemKind, NoWatch, Records, Settings, WriteItem};
        let data = vec![b"locked notes".to_vec(), b"more".to_vec()];
        let items: Vec<WriteItem> = ["notes.txt", "more.txt"]
            .iter()
            .zip(&data)
            .map(|(name, bytes)| WriteItem {
                name: (*name).to_owned(),
                kind: ItemKind::File,
                size: bytes.len() as u64,
                modified: None,
                mode: None,
                owner: None,
            })
            .collect();
        let settings = Settings {
            format: crate::ArchiveFormat::SevenZ,
            level: CompressionLevel::Normal,
            solid: true,
            block_bytes: 1 << 20,
            password: Some(password),
            encrypt_names: true,
            threads: 1,
            keep_owner: false,
        };
        let records = Records::new(items.len());
        crate::write::write_archive_seekable(
            Cursor::new(Vec::new()),
            &items,
            &InMemory(&data),
            &settings,
            &NoWatch,
            &records,
        )
        .unwrap()
        .into_inner()
    }

    /// Spike DS4's finding, held: a password reaches 7-Zip on standard
    /// input, for a listing whose names are locked and for a member read
    /// through the pipe; never in the arguments; a wrong one says so.
    #[test]
    fn a_password_reaches_7_zip_on_standard_input() {
        let Some((program, version)) = seven_zip() else {
            eprintln!("7-Zip not found: the password road did not run");
            return;
        };
        let canary = "canary-7f3a-not-a-password";
        // The build's own folder, beside this test program.
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("z-password-road");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locked.7z");
        fs::write(&path, locked_7z(canary)).unwrap();
        let runner = std::sync::Arc::new(Program {
            path: program,
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let prompt = std::sync::Arc::new(Typed(canary, Default::default()));
        let ctx = OpContext::new()
            .with_tools(runner.clone())
            .with_secrets(prompt.clone());
        let (index, backend) = ToolBackend::open(
            path.clone(),
            None,
            Signature::SevenZ,
            "locked.7z",
            &Limits::default(),
            &ctx,
            Some(dir.clone()),
        )
        .unwrap();
        assert_eq!(prompt.1.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(backend.password().is_some());
        let id = index.find("notes.txt").unwrap();
        if version.extracts() {
            let n = match index.record(id).unwrap().locator {
                Locator::Tool(n) => n,
                other => panic!("{other:?}"),
            };
            let mut text = String::new();
            backend
                .read(n, Some(12), backend.password(), &ctx)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            assert_eq!(text, "locked notes");
        }
        let wrong = OpContext::new()
            .with_tools(runner.clone())
            .with_secrets(std::sync::Arc::new(Typed("wrong", Default::default())));
        let error = ToolBackend::open(
            path.clone(),
            None,
            Signature::SevenZ,
            "locked.7z",
            &Limits::default(),
            &wrong,
            None,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), VfsError::WrongPassword.to_string());
        let seen = runner.seen.lock().unwrap();
        assert!(seen.iter().all(|a| !a.to_string_lossy().contains(canary)));
        assert!(seen.iter().all(|a| !a.to_string_lossy().starts_with("-p")));
    }

    #[test]
    fn a_password_goes_to_standard_input_never_the_arguments() {
        let secret = Secret::new("canary-7f3a-not-a-password".to_owned());
        let ToolInput::Bytes(bytes) = input_of(Some(&secret)) else {
            panic!("the password goes to standard input");
        };
        assert_eq!(bytes, b"canary-7f3a-not-a-password\n");
        assert_eq!(input_of(None), ToolInput::Closed);
        let args = list_args(Path::new("Backup.rar"));
        assert!(args.iter().all(|a| !a.to_string_lossy().starts_with("-p")));
        assert!(args.iter().all(|a| !a.to_string_lossy().contains("canary")));
    }
}
