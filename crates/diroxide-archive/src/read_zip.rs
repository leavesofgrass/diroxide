//! Zip reading (research/archives.md 2 and 6): the end record and the
//! central directory checked by the provider before the `zip` crate parses
//! them, the crate's entries turned into the index, and member reads
//! through the limiter chain on a thread of their own.
//!
//! What the provider adds to the crate, from spike 4.1: the end record's
//! claims are held to the file before anything is allocated; a walk of the
//! central directory counts duplicate names (the crate keeps only the last
//! copy of each) and finds overlapping members in sorted order, so the
//! quoted-overlap bomb is refused without the crate's quadratic check;
//! every local header is held to its central record (up to 100,000
//! members at listing, and always at read), so a member whose two names
//! differ is refused; an encrypted member is never read without a
//! password; and ZipCrypto is read with a warning, never written.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::io::{Read, Seek, SeekFrom};

use diroxide_core::clock::UtcOffset;
use diroxide_core::format::grouped;
use diroxide_core::{Secret, Timestamp};
use diroxide_vfs::{CancelToken, VfsError};
use zip::result::ZipError;
use zip::{CompressionMethod, ZipArchive, ZipReadOptions};

use crate::error::ArchiveError;
use crate::guard::{damaged, refuse};
use crate::index::{Index, IndexBuilder, Locator, Member, RecordFlags, RecordKind};
use crate::limits::{Limits, limit_words};
use crate::pipe::{self, PipeReader};
use crate::source::Source;

/// The most members whose local headers are checked at listing; past it
/// they are checked when read.
pub const LOCAL_CHECK_MAX: usize = 100_000;

/// The most link targets read at listing.
const LINK_READS_MAX: usize = 10_000;

/// The method number of AES in the central directory.
const METHOD_AES: u16 = 99;

/// What the end records say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EndRecord {
    /// Where the central directory really starts.
    pub cd_start: u64,
    /// Its size.
    pub cd_size: u64,
    /// The entries it claims.
    pub count: u64,
    /// The bytes before the archive (a self-extracting stub).
    pub shift: u64,
}

fn le16(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 2)
        .map_or(0, |s| u64::from(u16::from_le_bytes([s[0], s[1]])))
}

fn le32(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 4).map_or(0, |s| {
        u64::from(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    })
}

fn le64(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 8).map_or(0, |s| {
        u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
    })
}

/// Finds and checks the end records, before the crate allocates anything
/// from them.
pub(crate) fn end_record(source: &Source, limits: &Limits) -> Result<EndRecord, ArchiveError> {
    let len = source.len();
    let window = len.min(65_535 + 22);
    let mut tail = vec![0u8; window as usize];
    source
        .read_exact_at(len - window, &mut tail)
        .map_err(|_| damaged("its end record cannot be read"))?;
    let base = len - window;
    // The last end-record signature whose comment ends the file; failing
    // that, the last one whose comment fits (a signature inside a
    // comment is no way round).
    let candidates: Vec<usize> = (0..tail.len().saturating_sub(21))
        .rev()
        .filter(|&i| tail[i..i + 4] == [0x50, 0x4B, 0x05, 0x06])
        .collect();
    let fits = |i: usize| i + 22 + le16(&tail, i + 20) as usize;
    let at = candidates
        .iter()
        .copied()
        .find(|&i| fits(i) == tail.len())
        .or_else(|| candidates.iter().copied().find(|&i| fits(i) <= tail.len()))
        .ok_or_else(|| damaged("it has no end record, so it is not a whole zip"))?;
    let eocd = &tail[at..];
    let mut count = le16(eocd, 10);
    let mut cd_size = le32(eocd, 12);
    let mut cd_offset = le32(eocd, 16);
    let mut record_at = base + at as u64;
    // A zip64 locator just before the end record.
    if at >= 20 && tail[at - 20..at - 16] == [0x50, 0x4B, 0x06, 0x07] {
        let locator_at = base + at as u64 - 20;
        let mut record = [0u8; 56];
        let guess = locator_at.checked_sub(56);
        let mut found = None;
        if let Some(guess) = guess
            && source.read_exact_at(guess, &mut record).is_ok()
            && record[..4] == [0x50, 0x4B, 0x06, 0x06]
        {
            found = Some(guess);
        }
        if found.is_none() {
            let stated = le64(&tail, at - 20 + 8);
            if source.read_exact_at(stated, &mut record).is_ok()
                && record[..4] == [0x50, 0x4B, 0x06, 0x06]
            {
                found = Some(stated);
            }
        }
        let z64 = found.ok_or_else(|| damaged("its zip64 end record is missing"))?;
        count = le64(&record, 32);
        cd_size = le64(&record, 40);
        cd_offset = le64(&record, 48);
        record_at = z64;
    }
    if cd_size > limits.max_header_bytes {
        return Err(refuse("header size", limit_words(limits.max_header_bytes)).into());
    }
    if count > limits.max_entries as u64 {
        return Err(refuse("entries", grouped(limits.max_entries as u64)).into());
    }
    if count > cd_size / 46 {
        return Err(damaged(&format!(
            "it claims {} entries, more than its directory holds",
            grouped(count)
        ))
        .into());
    }
    let cd_start = record_at
        .checked_sub(cd_size)
        .ok_or_else(|| damaged("its directory runs past its start"))?;
    let shift = cd_start
        .checked_sub(cd_offset)
        .ok_or_else(|| damaged("its directory is not where it says"))?;
    Ok(EndRecord {
        cd_start,
        cd_size,
        count,
        shift,
    })
}

/// Reads until `buf` is full or the reader ends: the bytes read.
fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match reader.read(&mut buf[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// What one central record says, for the walk.
#[derive(Clone, Copy, Debug, Default)]
struct Walked {
    times: u32,
    method: u16,
    flags: u16,
}

/// What the walk of the central directory found.
struct Walk {
    by_name: HashMap<u64, Walked>,
    first_start: Option<u64>,
    records: u64,
    overlap: bool,
    past_directory: bool,
}

/// Walks the central directory: the names (for duplicates), the methods
/// (for ZipCrypto), and the data ranges (for the overlap).
fn walk(
    source: &Source,
    end: &EndRecord,
    hasher: &RandomState,
    cancel: &CancelToken,
) -> Result<Walk, ArchiveError> {
    let mut view = source.window(end.cd_start, end.cd_size);
    let mut by_name: HashMap<u64, Walked> = HashMap::new();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut records = 0u64;
    let mut past_directory = false;
    let mut fixed = [0u8; 46];
    let mut variable = Vec::new();
    loop {
        if records.is_multiple_of(4_096) {
            cancel.check()?;
        }
        match read_full(&mut view, &mut fixed[..4]) {
            Ok(0) => break,
            Ok(4) => {}
            Ok(_) | Err(_) => {
                return Err(damaged("its directory ends in the middle of an entry").into());
            }
        }
        if fixed[..4] != [0x50, 0x4B, 0x01, 0x02] {
            break;
        }
        view.read_exact(&mut fixed[4..])
            .map_err(|_| damaged("its directory ends in the middle of an entry"))?;
        let flags = le16(&fixed, 8) as u16;
        let method = le16(&fixed, 10) as u16;
        let mut csize = le32(&fixed, 20);
        let mut usize_ = le32(&fixed, 24);
        let name_len = le16(&fixed, 28) as usize;
        let extra_len = le16(&fixed, 30) as usize;
        let comment_len = le16(&fixed, 32) as usize;
        let mut offset = le32(&fixed, 42);
        variable.resize(name_len + extra_len + comment_len, 0);
        view.read_exact(&mut variable)
            .map_err(|_| damaged("its directory ends in the middle of a name"))?;
        let name = &variable[..name_len];
        // The zip64 extra holds the sizes and the offset the classic
        // fields could not.
        let extra = &variable[name_len..name_len + extra_len];
        let mut at = 0;
        while at + 4 <= extra.len() {
            let id = le16(extra, at);
            let size = le16(extra, at + 2) as usize;
            let body = extra.get(at + 4..at + 4 + size).unwrap_or(&[]);
            if id == 1 {
                let mut field = 0;
                for value in [&mut usize_, &mut csize, &mut offset] {
                    if *value == 0xFFFF_FFFF && field + 8 <= body.len() {
                        *value = le64(body, field);
                        field += 8;
                    }
                }
            }
            at += 4 + size;
        }
        let _ = usize_;
        let key = hasher.hash_one(name);
        let walked = by_name.entry(key).or_default();
        walked.times += 1;
        walked.method = method;
        walked.flags = flags;
        let start = offset.saturating_add(end.shift);
        let end_at = start
            .saturating_add(30)
            .saturating_add(name_len as u64)
            .saturating_add(csize);
        if end_at > end.cd_start {
            past_directory = true;
        }
        ranges.push((start, end_at));
        records += 1;
    }
    ranges.sort_unstable();
    let overlap = ranges.windows(2).any(|w| w[1].0 < w[0].1);
    Ok(Walk {
        by_name,
        first_start: ranges.first().map(|r| r.0),
        records,
        overlap,
        past_directory,
    })
}

/// A zip opened: the crate's archive (its metadata shared by every
/// clone), and what the provider found.
#[derive(Debug)]
pub(crate) struct ZipBackend {
    archive: ZipArchive<Source>,
    source: Source,
    overlap: bool,
    locals_checked: bool,
}

/// The crate's error, as the provider says it.
fn zip_error(error: ZipError) -> VfsError {
    match error {
        ZipError::Io(error) => crate::guard::from_io(error),
        ZipError::InvalidArchive(why) => damaged(&crate::guard::lowercase_first(&why)),
        ZipError::UnsupportedArchive(why) if why == ZipError::PASSWORD_REQUIRED => {
            VfsError::PasswordRequired
        }
        ZipError::UnsupportedArchive(why) => VfsError::UnsupportedFeature {
            what: crate::guard::lowercase_first(why),
        },
        ZipError::FileNotFound => VfsError::NotFound,
        ZipError::InvalidPassword => VfsError::WrongPassword,
        ZipError::CompressionMethodNotSupported(id) => VfsError::UnsupportedFeature {
            what: format!("compression method {id}"),
        },
        other => damaged(&crate::guard::lowercase_first(&other.to_string())),
    }
}

/// A DOS time as a timestamp, read as UTC (zip stores no zone).
fn timestamp(time: zip::DateTime) -> Option<i64> {
    Timestamp::from_civil(
        i32::from(time.year()),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second(),
        UtcOffset::UTC,
    )
    .map(Timestamp::unix_seconds)
}

/// Whether the crate can read a method.
fn readable(method: CompressionMethod) -> bool {
    matches!(
        method,
        CompressionMethod::Stored
            | CompressionMethod::Deflated
            | CompressionMethod::Deflate64
            | CompressionMethod::Bzip2
            | CompressionMethod::Lzma
            | CompressionMethod::Xz
            | CompressionMethod::Ppmd
    )
}

/// What a local header says.
struct Local {
    name: Vec<u8>,
}

/// Reads a local header through a buffered view: the headers of a zip
/// lie in order, so one buffer serves many.
fn local_header(view: &mut Source, at: u64) -> Option<Local> {
    let mut fixed = [0u8; 30];
    view.seek(SeekFrom::Start(at)).ok()?;
    if read_full(view, &mut fixed).ok()? != fixed.len() || fixed[..4] != [0x50, 0x4B, 0x03, 0x04] {
        return None;
    }
    let name_len = le16(&fixed, 26) as usize;
    let mut name = vec![0u8; name_len];
    if read_full(view, &mut name).ok()? != name_len {
        return None;
    }
    Some(Local { name })
}

/// Opens a zip: the checks, the walk, the crate's parse, the index.
pub(crate) fn open(
    source: Source,
    container: &str,
    limits: &Limits,
    cancel: &CancelToken,
) -> Result<(Index, ZipBackend), ArchiveError> {
    let end = end_record(&source, limits)?;
    let hasher = RandomState::new();
    let walked = walk(&source, &end, &hasher, cancel)?;
    let mut archive = ZipArchive::new(source.clone()).map_err(zip_error)?;
    let mut builder = IndexBuilder::new(*limits);
    if walked.overlap {
        builder.note(format!(
            "Members of {container} overlap, as a zip bomb's do: nothing is read from it."
        ));
    }
    if walked.past_directory {
        builder.note(format!("A member of {container} runs into its directory."));
    }
    if walked.records != end.count {
        builder.note(format!(
            "{container} says it holds {} entries and holds {}.",
            grouped(end.count),
            grouped(walked.records)
        ));
    }
    let before = walked.first_start.unwrap_or(0).max(archive.offset());
    if before > 0 {
        builder.note(format!(
            "{container} has {} bytes before the archive, which are ignored.",
            grouped(before)
        ));
    }
    let count = archive.len();
    let check_locals = count <= LOCAL_CHECK_MAX;
    let mut locals = source.clone();
    let mut links_read = 0usize;
    let mut weak = false;
    for i in 0..count {
        if i.is_multiple_of(4_096) {
            cancel.check()?;
        }
        let entry = archive.by_index_data(i).map_err(zip_error)?;
        let raw = entry.name_raw().to_vec();
        let (name, not_unicode) = match entry.name() {
            Ok(name) => (name.into_owned(), false),
            Err(_) => (String::from_utf8_lossy(&raw).into_owned(), true),
        };
        let walked_entry = walked
            .by_name
            .get(&hasher.hash_one(&raw))
            .copied()
            .unwrap_or_default();
        let mode = entry.unix_mode();
        let kind = if entry.is_dir() {
            RecordKind::Folder
        } else if entry.is_symlink() {
            RecordKind::Link
        } else {
            match mode.map(|m| m & 0o170_000) {
                None | Some(0 | 0o100_000) => RecordKind::File,
                Some(0o040_000) => RecordKind::Folder,
                Some(_) => RecordKind::Special,
            }
        };
        let mut flags = RecordFlags::NONE;
        let mut note = None;
        let encrypted = entry.encrypted();
        if encrypted {
            flags.insert(RecordFlags::ENCRYPTED);
            if walked_entry.method != METHOD_AES {
                flags.insert(RecordFlags::WEAK_CRYPTO);
                weak = true;
            }
        }
        if kind == RecordKind::File && !readable(entry.compression()) {
            flags.insert(RecordFlags::UNREADABLE);
            note = Some(format!(
                "Compression method {} is not supported.",
                walked_entry.method
            ));
        }
        if check_locals && let Some(local) = local_header(&mut locals, entry.header_start()) {
            if local.name != raw {
                flags.insert(RecordFlags::MISMATCH);
                note = Some(format!(
                    "Its local header names it {}.",
                    diroxide_core::names::display_name(&String::from_utf8_lossy(&local.name))
                ));
            }
        } else if check_locals && kind != RecordKind::Folder {
            flags.insert(RecordFlags::MISMATCH);
            note = Some("Its local header is missing.".to_owned());
        }
        let size = entry.size();
        let packed = entry.compressed_size();
        let crc32 = entry.crc32();
        let modified = entry.last_modified().and_then(timestamp);
        drop(entry);
        let mut link_target = None;
        if kind == RecordKind::Link && !encrypted && links_read < LINK_READS_MAX {
            links_read += 1;
            if let Ok(file) = archive.by_index(i) {
                let mut target = Vec::new();
                let cap = limits.max_link_target_bytes as u64 + 1;
                if file.take(cap).read_to_end(&mut target).is_ok() {
                    link_target = Some(String::from_utf8_lossy(&target).into_owned());
                }
            }
        }
        if walked_entry.times > 1 && kind != RecordKind::Folder {
            // The crate kept one copy of the name; the walk saw them all.
            // Count them as the index would.
            for _ in 1..walked_entry.times {
                let mut ghost = Member {
                    name: name.clone(),
                    not_unicode,
                    kind: Some(kind),
                    ..Member::default()
                };
                ghost.locator = Locator::None;
                builder.add(ghost)?;
            }
        }
        builder.add(Member {
            name,
            not_unicode,
            kind: Some(kind),
            size,
            packed: Some(packed),
            modified,
            mode: mode.map(|m| m & 0o7777),
            // AES (AE-2) stores no CRC; its own code authenticates instead.
            crc32: (kind == RecordKind::File && walked_entry.method != METHOD_AES).then_some(crc32),
            link_target,
            locator: Locator::Zip(i as u32),
            flags,
            note,
        })?;
    }
    if weak {
        builder.note(format!("Weak encryption in {container} (ZipCrypto)."));
    }
    let index = builder.finish();
    Ok((
        index,
        ZipBackend {
            archive,
            source,
            overlap: walked.overlap,
            locals_checked: check_locals,
        },
    ))
}

impl ZipBackend {
    /// A stored, unencrypted member as a window of the archive's bytes:
    /// the shortcut a nested archive takes, with no spool and no copy.
    /// `None` for any other member, and for every member of an archive
    /// whose members overlap or whose local header disagrees.
    pub(crate) fn stored_window(&self, index: &Index, id: u32) -> Option<Source> {
        let record = index.record(id)?;
        let Locator::Zip(i) = record.locator else {
            return None;
        };
        if self.overlap
            || record.flags.contains(RecordFlags::ENCRYPTED)
            || record.flags.contains(RecordFlags::MISMATCH)
        {
            return None;
        }
        let entry = self.archive.by_index_data(i as usize).ok()?;
        if entry.compression() != CompressionMethod::Stored
            || entry.size() != entry.compressed_size()
        {
            return None;
        }
        let mut fixed = [0u8; 30];
        self.source
            .read_exact_at(entry.header_start(), &mut fixed)
            .ok()?;
        if fixed[..4] != [0x50, 0x4B, 0x03, 0x04] {
            return None;
        }
        let data = entry.header_start() + 30 + le16(&fixed, 26) + le16(&fixed, 28);
        let end = data.checked_add(entry.size())?;
        if end > self.source.len() {
            return None;
        }
        Some(self.source.window(data, entry.size()))
    }

    /// Reads one member on a thread of its own, through the crate.
    pub(crate) fn read(
        &self,
        index: &Index,
        id: u32,
        password: Option<Secret>,
        cancel: CancelToken,
    ) -> Result<PipeReader, VfsError> {
        let record = index.record(id).ok_or(VfsError::NotFound)?;
        if self.overlap {
            return Err(VfsError::Unsafe {
                why: "its members overlap, as a zip bomb's do".to_owned(),
            });
        }
        let Locator::Zip(i) = record.locator else {
            return Err(VfsError::NotFound);
        };
        let i = i as usize;
        if record.flags.contains(RecordFlags::MISMATCH) {
            return Err(VfsError::Unsafe {
                why: "its local and central names differ".to_owned(),
            });
        }
        if !self.locals_checked {
            let entry = self.archive.by_index_data(i).map_err(zip_error)?;
            let raw = entry.name_raw().to_vec();
            match local_header(&mut self.source.clone(), entry.header_start()) {
                Some(local) if local.name == raw => {}
                _ => {
                    return Err(VfsError::Unsafe {
                        why: "its local and central names differ".to_owned(),
                    });
                }
            }
        }
        if record.flags.contains(RecordFlags::UNREADABLE) {
            return Err(VfsError::UnsupportedFeature {
                what: index
                    .note(id)
                    .map_or("a compression method", |n| n)
                    .trim_end_matches(" is not supported.")
                    .to_lowercase(),
            });
        }
        let encrypted = record.flags.contains(RecordFlags::ENCRYPTED);
        if encrypted && password.is_none() {
            return Err(VfsError::PasswordRequired);
        }
        let weak = record.flags.contains(RecordFlags::WEAK_CRYPTO);
        let name = index.name(id).to_owned();
        let size = record.size;
        let mut archive = self.archive.clone();
        pipe::spawn(cancel, move |feed| {
            let password = if encrypted { password } else { None };
            let bytes = password.as_ref().map(|p| p.expose().as_bytes());
            let options = ZipReadOptions::new().password(bytes);
            let mut file =
                archive
                    .by_index_with_options(i, options)
                    .map_err(|e| match zip_error(e) {
                        // ZipCrypto checks one byte: a wrong password and damage
                        // look the same.
                        VfsError::WrongPassword if weak => VfsError::WrongPassword,
                        other => other,
                    })?;
            let mut said = SaidPlainly {
                inner: &mut file,
                name: &name,
                size,
            };
            feed.pump(&mut said);
            Ok(())
        })
    }
}

/// The crate's own read errors, in the provider's words: a member that
/// runs past its stated size says so as the limiter chain would.
struct SaidPlainly<'a, R> {
    inner: &'a mut R,
    name: &'a str,
    size: u64,
}

impl<R: Read> Read for SaidPlainly<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf).map_err(|error| {
            if error.to_string().contains("larger than its declared") {
                crate::guard::to_io(damaged(&format!(
                    "{} is longer than its stated size of {} bytes",
                    self.name,
                    grouped(self.size)
                )))
            } else {
                error
            }
        })
    }
}
