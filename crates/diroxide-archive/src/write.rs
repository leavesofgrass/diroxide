//! The writers of the pack runner, over any byte sink: what one member is
//! ([`WriteItem`]), where its bytes come from ([`Contents`]), how hard to
//! compress and with what password ([`Settings`]), and what each member's
//! bytes were, counted and checksummed on their way in, for the check
//! after packing ([`Records`]).
//!
//! Every format is written in one forward pass over its sink: zip with
//! data descriptors (its writer needs no seek), tar as it is or through
//! gzip or xz, plain xz of one file, and 7z through a sink that holds back
//! only its 32-byte start header. ZipCrypto is never written: a zip
//! password is AES-256.
//!
//! The same functions serve the runner (contents from the providers, the
//! job's progress and cancel) and the round-trip fuzz target (contents in
//! memory, no job).

use std::io::{self, Read, Write};
use std::sync::Mutex;

use diroxide_core::{Timestamp, UtcOffset};
use diroxide_vfs::VfsError;

use crate::format::ArchiveFormat;
use crate::guard::to_io;
use crate::request::CompressionLevel;

/// What one member of a new archive is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ItemKind {
    /// A file, its bytes from the contents.
    File,
    /// A folder.
    Folder,
    /// A symbolic link, stored with its target.
    Link(String),
}

/// One member of a new archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WriteItem {
    /// Its path in the archive: forward slashes, no leading slash, no `.`
    /// or `..`.
    pub(crate) name: String,
    /// What it is.
    pub(crate) kind: ItemKind,
    /// Its size, as listed (a file's bytes may differ if it changes while
    /// being read; the records say what was written).
    pub(crate) size: u64,
    /// When it last changed.
    pub(crate) modified: Option<Timestamp>,
    /// Its Unix mode bits, when the source has them.
    pub(crate) mode: Option<u32>,
    /// Its owner's and group's names, when kept (tar only).
    pub(crate) owner: Option<(String, String)>,
}

impl WriteItem {
    /// The mode stored for it: the source's bits, or the usual ones.
    pub(crate) fn stored_mode(&self) -> u32 {
        match (&self.kind, self.mode) {
            (_, Some(mode)) => mode & 0o7777,
            (ItemKind::Folder, None) => 0o755,
            (ItemKind::Link(_), None) => 0o777,
            (ItemKind::File, None) => 0o644,
        }
    }
}

/// Where the members' bytes come from.
pub(crate) trait Contents {
    /// Opens member `n`'s bytes.
    fn open(&self, n: usize) -> Result<Box<dyn Read + Send + '_>, VfsError>;
}

/// The bytes of members held in memory, for the fuzz target and tests.
#[allow(dead_code)] // The round-trip fuzz target's (plan commit 37).
pub(crate) struct InMemory<'a>(pub(crate) &'a [Vec<u8>]);

impl Contents for InMemory<'_> {
    fn open(&self, n: usize) -> Result<Box<dyn Read + Send + '_>, VfsError> {
        let data = self.0.get(n).ok_or(VfsError::NotFound)?;
        Ok(Box::new(data.as_slice()))
    }
}

/// How a new archive is written.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // The tar and 7z writers read the rest (commits 33, 34).
pub(crate) struct Settings<'a> {
    /// Its format.
    pub(crate) format: ArchiveFormat,
    /// How hard to compress.
    pub(crate) level: CompressionLevel,
    /// A solid 7z.
    pub(crate) solid: bool,
    /// The largest solid block.
    pub(crate) block_bytes: u64,
    /// The password: AES-256 for zip and 7z.
    pub(crate) password: Option<&'a str>,
    /// Whether a 7z hides its names behind the password.
    pub(crate) encrypt_names: bool,
    /// Threads for the encoders that can use them.
    pub(crate) threads: u32,
    /// Whether tar keeps owner names.
    pub(crate) keep_owner: bool,
}

/// Watches the bytes going in: cancel and pause between reads, progress.
pub(crate) trait Watch: Sync {
    /// Waits while paused; an error once cancelled.
    fn check(&self) -> io::Result<()>;
    /// Counts bytes read from a member.
    fn bytes(&self, bytes: u64);
    /// Says which member is at work.
    fn member(&self, name: &str);
}

/// A watch that never stops and counts nothing.
#[allow(dead_code)] // The round-trip fuzz target's (plan commit 37).
pub(crate) struct NoWatch;

impl Watch for NoWatch {
    fn check(&self) -> io::Result<()> {
        Ok(())
    }

    fn bytes(&self, _bytes: u64) {}

    fn member(&self, _name: &str) {}
}

/// What each member's bytes were, by item: the size and CRC-32 of what
/// went into the archive.
#[derive(Debug, Default)]
pub(crate) struct Records(Mutex<Vec<Option<(u64, u32)>>>);

impl Records {
    /// Room for `items`.
    pub(crate) fn new(items: usize) -> Self {
        Self(Mutex::new(vec![None; items]))
    }

    fn set(&self, n: usize, size: u64, crc: u32) {
        if let Ok(mut slots) = self.0.lock()
            && let Some(slot) = slots.get_mut(n)
        {
            *slot = Some((size, crc));
        }
    }

    /// Member `n`'s size and CRC-32, once its bytes were read to the end.
    pub(crate) fn get(&self, n: usize) -> Option<(u64, u32)> {
        self.0
            .lock()
            .ok()
            .and_then(|slots| slots.get(n).copied().flatten())
    }
}

/// A member's bytes on their way in: the watch asked at every read, the
/// size and CRC-32 kept in the records at the end.
pub(crate) struct Feed<'a, R> {
    inner: R,
    n: usize,
    size: u64,
    crc: crc32fast::Hasher,
    watch: &'a dyn Watch,
    records: &'a Records,
    done: bool,
    failure: Option<VfsError>,
}

impl<'a, R: Read> Feed<'a, R> {
    /// Feeds member `n`.
    pub(crate) fn new(inner: R, n: usize, watch: &'a dyn Watch, records: &'a Records) -> Self {
        Self {
            inner,
            n,
            size: 0,
            crc: crc32fast::Hasher::new(),
            watch,
            records,
            done: false,
            failure: None,
        }
    }

    /// Copies the member into `sink`: an error reading the member is the
    /// member's, any other the archive's.
    pub(crate) fn copy_into(&mut self, sink: &mut dyn Write) -> Result<(), PackFailure> {
        match io::copy(self, sink) {
            Ok(_) => Ok(()),
            Err(error) => Err(match self.failure.take() {
                Some(failure) => PackFailure::Source(self.n, failure),
                None => PackFailure::from_io(error),
            }),
        }
    }
}

impl<R: Read> Read for Feed<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        self.watch.check()?;
        let n = match self.inner.read(buf) {
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                let failure = crate::guard::from_io(error);
                self.failure = Some(failure.clone());
                return Err(to_io(failure));
            }
        };
        if n == 0 {
            self.done = true;
            let crc = std::mem::take(&mut self.crc).finalize();
            self.records.set(self.n, self.size, crc);
            return Ok(0);
        }
        self.crc.update(&buf[..n]);
        self.size += n as u64;
        self.watch.bytes(n as u64);
        Ok(n)
    }
}

/// Opens member `n` through a feed, or says why it could not be read.
pub(crate) fn feed<'a>(
    contents: &'a dyn Contents,
    n: usize,
    watch: &'a dyn Watch,
    records: &'a Records,
) -> Result<Feed<'a, Box<dyn Read + Send + 'a>>, PackFailure> {
    let inner = contents
        .open(n)
        .map_err(|error| PackFailure::Source(n, error))?;
    Ok(Feed::new(inner, n, watch, records))
}

/// Why writing a new archive stopped.
#[derive(Debug)]
pub(crate) enum PackFailure {
    /// A member's bytes could not be read: its number and the cause.
    Source(usize, VfsError),
    /// The archive could not be written.
    Write(VfsError),
    /// Cancelled.
    Cancelled,
}

impl PackFailure {
    /// From an I/O error of the writer, which may carry a cancel or a
    /// member's read error.
    pub(crate) fn from_io(error: io::Error) -> Self {
        match crate::guard::from_io(error) {
            VfsError::Cancelled => PackFailure::Cancelled,
            other => PackFailure::Write(other),
        }
    }
}

/// The error a watch gives once cancelled.
pub(crate) fn cancelled() -> io::Error {
    to_io(VfsError::Cancelled)
}

/// A time as the zip's DOS date and time (whole seconds rounded down to
/// even, 1980 to 2107), in UTC, as the reader reads it.
pub(crate) fn dos_time(time: Option<Timestamp>) -> zip::DateTime {
    let Some(time) = time else {
        return zip::DateTime::default();
    };
    let civil = time.to_civil(UtcOffset::UTC);
    u16::try_from(civil.year)
        .ok()
        .and_then(|year| {
            zip::DateTime::from_date_and_time(
                year,
                civil.month,
                civil.day,
                civil.hour,
                civil.minute,
                civil.second,
            )
            .ok()
        })
        .unwrap_or_default()
}

/// Writes a zip: deflate at the level's number, AES-256 when a password
/// is set (ZipCrypto never), data descriptors so the sink needs no seek,
/// zip64 for a member of 4 GiB or more, times from the sources, Unix
/// modes, links as links.
pub(crate) fn write_zip<W: Write>(
    out: W,
    items: &[WriteItem],
    contents: &dyn Contents,
    settings: &Settings<'_>,
    watch: &dyn Watch,
    records: &Records,
) -> Result<W, PackFailure> {
    use zip::write::SimpleFileOptions;
    let level = settings.level.number_for(ArchiveFormat::Zip).map(i64::from);
    let mut zip = zip::ZipWriter::new_stream(out);
    for (n, item) in items.iter().enumerate() {
        watch.check().map_err(PackFailure::from_io)?;
        watch.member(&item.name);
        let base = SimpleFileOptions::default()
            .system(zip::System::Unix)
            .last_modified_time(dos_time(item.modified))
            .unix_permissions(item.stored_mode());
        let written = match &item.kind {
            ItemKind::Folder => zip.add_directory(item.name.as_str(), base),
            ItemKind::Link(target) => zip.add_symlink(item.name.as_str(), target.as_str(), base),
            ItemKind::File => {
                let mut source = feed(contents, n, watch, records)?;
                let options = base
                    .compression_method(zip::CompressionMethod::Deflated)
                    .compression_level(level)
                    .large_file(item.size >= u64::from(u32::MAX));
                let started = match settings.password {
                    Some(password) => zip.start_file(
                        item.name.as_str(),
                        options.with_aes_encryption(zip::AesMode::Aes256, password),
                    ),
                    None => zip.start_file(item.name.as_str(), options),
                };
                started.map_err(zip_failure)?;
                source.copy_into(&mut zip)?;
                Ok(())
            }
        };
        written.map_err(zip_failure)?;
    }
    let stream = zip.finish().map_err(zip_failure)?;
    Ok(stream.into_inner())
}

/// The zip crate's error, in the provider's words.
fn zip_failure(error: zip::result::ZipError) -> PackFailure {
    match error {
        zip::result::ZipError::Io(error) => PackFailure::from_io(error),
        other => PackFailure::Write(VfsError::Corrupt {
            what: crate::guard::lowercase_first(&other.to_string()),
        }),
    }
}

/// Writes a new archive of `settings.format`.
pub(crate) fn write_archive<W: Write>(
    out: W,
    items: &[WriteItem],
    contents: &dyn Contents,
    settings: &Settings<'_>,
    watch: &dyn Watch,
    records: &Records,
) -> Result<W, PackFailure> {
    match settings.format {
        ArchiveFormat::Zip => write_zip(out, items, contents, settings, watch, records),
        ArchiveFormat::Tar => write_tar(out, items, contents, settings, watch, records),
        ArchiveFormat::TarGz => {
            let level = settings.level.number_for(ArchiveFormat::TarGz).unwrap_or(6);
            let gzip = flate2::write::GzEncoder::new(out, flate2::Compression::new(level));
            let gzip = write_tar(gzip, items, contents, settings, watch, records)?;
            gzip.finish().map_err(PackFailure::from_io)
        }
        ArchiveFormat::TarXz => {
            let xz = XzSink::new(out, settings)?;
            let xz = write_tar(xz, items, contents, settings, watch, records)?;
            xz.finish()
        }
        ArchiveFormat::Xz => {
            let mut xz = XzSink::new(out, settings)?;
            match items {
                [item] if item.kind == ItemKind::File => {
                    watch.check().map_err(PackFailure::from_io)?;
                    watch.member(&item.name);
                    feed(contents, 0, watch, records)?.copy_into(&mut xz)?;
                }
                _ => {
                    return Err(PackFailure::Write(VfsError::UnsupportedFeature {
                        what: "more than one file in xz, which holds one; use tar.xz".to_owned(),
                    }));
                }
            }
            xz.finish()
        }
        _ => Err(PackFailure::Write(VfsError::unsupported(
            "archive",
            "pack to this format yet",
        ))),
    }
}

/// The memory one xz encoder thread takes at a preset, in MiB (the xz
/// manual's figures, rounded up).
fn xz_thread_mib(preset: u32) -> u64 {
    match preset {
        0..=3 => 32,
        4..=6 => 100,
        7 => 190,
        8 => 370,
        _ => 680,
    }
}

/// An xz stream being written: one thread, or several over blocks of
/// three dictionaries each (xz's own default), as many as fit in 1 GiB.
enum XzSink<W: Write> {
    One(lzma_rust2::XzWriter<W>),
    Many(lzma_rust2::XzWriterMt<W>),
}

impl<W: Write> XzSink<W> {
    fn new(out: W, settings: &Settings<'_>) -> Result<Self, PackFailure> {
        let preset = settings.level.number_for(ArchiveFormat::Xz).unwrap_or(6);
        let mut options = lzma_rust2::XzOptions::with_preset(preset);
        let fit = (1024 / xz_thread_mib(preset)).max(1) as u32;
        let threads = settings.threads.min(fit);
        if threads > 1 {
            let block = u64::from(options.lzma_options.dict_size).saturating_mul(3);
            options.set_block_size(std::num::NonZeroU64::new(block.max(1 << 20)));
            lzma_rust2::XzWriterMt::new(out, options, threads)
                .map(XzSink::Many)
                .map_err(PackFailure::from_io)
        } else {
            lzma_rust2::XzWriter::new(out, options)
                .map(XzSink::One)
                .map_err(PackFailure::from_io)
        }
    }

    fn finish(self) -> Result<W, PackFailure> {
        match self {
            XzSink::One(writer) => writer.finish(),
            XzSink::Many(writer) => writer.finish(),
        }
        .map_err(PackFailure::from_io)
    }
}

impl<W: Write> Write for XzSink<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            XzSink::One(writer) => writer.write(buf),
            XzSink::Many(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            XzSink::One(writer) => writer.flush(),
            XzSink::Many(writer) => writer.flush(),
        }
    }
}

/// A member's bytes held to the size its header already says: fewer
/// bytes, or more, mean the file changed while it was packed.
struct Exactly<'f, R> {
    inner: &'f mut R,
    left: u64,
    name: &'f str,
}

impl<R: Read> Read for Exactly<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            let mut one = [0u8; 1];
            return match self.inner.read(&mut one)? {
                0 => Ok(0),
                _ => Err(to_io(changed(self.name))),
            };
        }
        let want = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(to_io(changed(self.name)));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// The error of a file that changed while it was packed.
fn changed(name: &str) -> VfsError {
    VfsError::Unsafe {
        why: format!("{name} changed while it was packed; pack it again"),
    }
}

/// Writes a tar in GNU format: long names as GNU long-name entries, no
/// sparse entries, owner and group ids of 0 and no owner names unless
/// they are kept, times and modes from the sources, links as links.
pub(crate) fn write_tar<W: Write>(
    out: W,
    items: &[WriteItem],
    contents: &dyn Contents,
    settings: &Settings<'_>,
    watch: &dyn Watch,
    records: &Records,
) -> Result<W, PackFailure> {
    let mut builder = tar::Builder::new(out);
    builder.follow_symlinks(false);
    builder.sparse(false);
    for (n, item) in items.iter().enumerate() {
        watch.check().map_err(PackFailure::from_io)?;
        watch.member(&item.name);
        let mut header = tar::Header::new_gnu();
        header.set_mtime(
            item.modified
                .map_or(0, |t| u64::try_from(t.unix_seconds()).unwrap_or(0)),
        );
        header.set_mode(item.stored_mode());
        header.set_uid(0);
        header.set_gid(0);
        if settings.keep_owner
            && let Some((owner, group)) = &item.owner
        {
            let _ = header.set_username(owner);
            let _ = header.set_groupname(group);
        }
        let appended = match &item.kind {
            ItemKind::Folder => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                builder.append_data(&mut header, format!("{}/", item.name), io::empty())
            }
            ItemKind::Link(target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                builder.append_link(&mut header, &item.name, target)
            }
            ItemKind::File => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(item.size);
                let mut source = feed(contents, n, watch, records)?;
                let result = {
                    let mut exact = Exactly {
                        inner: &mut source,
                        left: item.size,
                        name: &item.name,
                    };
                    builder.append_data(&mut header, &item.name, &mut exact)
                };
                if result.is_err()
                    && let Some(failure) = source.failure.take()
                {
                    return Err(PackFailure::Source(n, failure));
                }
                result
            }
        };
        appended.map_err(PackFailure::from_io)?;
    }
    builder.into_inner().map_err(PackFailure::from_io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> WriteItem {
        WriteItem {
            name: name.to_owned(),
            kind: ItemKind::File,
            size,
            modified: Some(Timestamp::from_unix(1_792_152_000, 0)),
            mode: Some(0o640),
            owner: None,
        }
    }

    #[test]
    fn a_zip_is_written_without_a_seek_and_reads_back() {
        let data = vec![b"hello".to_vec(), b"world, again".to_vec()];
        let items = vec![file("a.txt", 5), file("dir/b.txt", 12)];
        let records = Records::new(2);
        let settings = Settings {
            format: ArchiveFormat::Zip,
            level: CompressionLevel::Normal,
            solid: false,
            block_bytes: 1 << 30,
            password: None,
            encrypt_names: false,
            threads: 1,
            keep_owner: false,
        };
        let out = write_archive(
            Vec::new(),
            &items,
            &InMemory(&data),
            &settings,
            &NoWatch,
            &records,
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(io::Cursor::new(out)).unwrap();
        let mut text = String::new();
        zip.by_name("dir/b.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "world, again");
        assert_eq!(zip.by_name("a.txt").unwrap().unix_mode(), Some(0o100_640));
        assert_eq!(records.get(1), Some((12, crc32fast::hash(b"world, again"))));
    }

    #[test]
    fn dos_times_are_utc_and_clamped() {
        let time = dos_time(Some(Timestamp::from_unix(1_792_152_000, 0)));
        assert_eq!((time.year(), time.month(), time.day()), (2026, 10, 16));
        assert_eq!((time.hour(), time.minute()), (12, 0));
        assert_eq!(dos_time(Some(Timestamp::from_unix(0, 0))).year(), 1980);
    }
}
