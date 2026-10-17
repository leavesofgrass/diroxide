//! The 7z writer (plan commit 34): LZMA2 at the level's preset, solid
//! blocks of at most the block limit (1 GiB by default, so reading one
//! file later costs at most about a gigabyte of decoding), or one block
//! per file; AES-256 when a password is set, with the names hidden too
//! when asked; LZMA2's own threads, as many as fit in 1 GiB.
//!
//! sevenz-rust2's writer seeks twice: past the 32-byte start header when
//! it starts, and back to it when it ends. A committing writer that seeks
//! (`FileWriter::seek_to`, the local one since P2b) takes it straight,
//! through [`Seeking`]; [`HeadHolder`] lets it write to a sink that cannot
//! seek: the body streams through, and the start header is held back, for
//! the runner to put in front.
//!
//! The crate copies the password into its own `Password` (UTF-16 bytes in
//! a plain vector, not wiped) for as long as the writer lives; diroxide's
//! copy stays in its wiped `Secret`.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use diroxide_vfs::{FileWriter, VfsError};
use sevenz_rust2::encoder_options::{AesEncoderOptions, EncoderOptions, Lzma2Options};
use sevenz_rust2::{
    ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, NtTime, Password,
    SourceReader,
};

use crate::format::ArchiveFormat;
use crate::write::{Contents, Feed, ItemKind, PackFailure, Records, Settings, Watch, WriteItem};

/// The 7z start header's size.
pub(crate) const START_HEADER: u64 = 32;

/// A sink for sevenz-rust2's writer that never seeks its own sink: the
/// body streams through, the 32-byte start header is held back.
pub(crate) struct HeadHolder<W> {
    body: W,
    pos: u64,
    head: Option<Vec<u8>>,
}

impl<W: Write> HeadHolder<W> {
    /// A holder whose body streams into `body`.
    pub(crate) fn new(body: W) -> Self {
        Self {
            body,
            pos: 0,
            head: None,
        }
    }

    /// The start header and the body's sink, once the writer finished.
    pub(crate) fn into_parts(self) -> (Vec<u8>, W) {
        (self.head.unwrap_or_default(), self.body)
    }
}

impl<W: Write> Write for HeadHolder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(head) = self.head.as_mut() {
            let room = (START_HEADER as usize).saturating_sub(head.len());
            let n = buf.len().min(room);
            if n == 0 {
                return Err(io::Error::other("7z wrote past its start header"));
            }
            head.extend_from_slice(&buf[..n]);
            return Ok(n);
        }
        let n = self.body.write(buf)?;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.body.flush()
    }
}

impl<W: Write> Seek for HeadHolder<W> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match to {
            SeekFrom::Start(START_HEADER) if self.pos == 0 && self.head.is_none() => {
                self.pos = START_HEADER;
                Ok(START_HEADER)
            }
            SeekFrom::Current(0) => Ok(self.pos),
            SeekFrom::Start(0) if self.head.is_none() => {
                self.head = Some(Vec::with_capacity(START_HEADER as usize));
                Ok(0)
            }
            _ => Err(io::Error::from(io::ErrorKind::Unsupported)),
        }
    }
}

/// A committing writer that seeks, as the 7z writer wants its sink: the
/// whole archive goes straight into the unfinished file, header last.
pub(crate) struct Seeking(pub(crate) Box<dyn FileWriter>);

impl Write for Seeking {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Seek for Seeking {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.0.seek_to(to).map_err(crate::guard::to_io)
    }
}

/// A member's bytes opened only when the writer first reads them, so a
/// solid block of ten thousand files holds one open at a time.
struct Lazy<'a> {
    contents: &'a dyn Contents,
    n: usize,
    watch: &'a dyn Watch,
    records: &'a Records,
    feed: Option<Feed<'a, Box<dyn Read + Send + 'a>>>,
    failed: &'a Mutex<Option<(usize, VfsError)>>,
    finished: bool,
}

impl Read for Lazy<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.finished {
            return Ok(0);
        }
        if self.feed.is_none() {
            match self.contents.open(self.n) {
                Ok(inner) => {
                    self.feed = Some(Feed::new(inner, self.n, self.watch, self.records));
                }
                Err(error) => {
                    self.note(error.clone());
                    return Err(crate::guard::to_io(error));
                }
            }
        }
        let Some(feed) = self.feed.as_mut() else {
            return Ok(0);
        };
        match feed.read(buf) {
            Ok(0) => {
                self.finished = true;
                self.feed = None;
                Ok(0)
            }
            Ok(n) => Ok(n),
            Err(error) => {
                if let Some(failure) = feed.failure.take() {
                    self.note(failure);
                }
                Err(error)
            }
        }
    }
}

impl Lazy<'_> {
    fn note(&self, error: VfsError) {
        if let Ok(mut slot) = self.failed.lock()
            && slot.is_none()
        {
            *slot = Some((self.n, error));
        }
    }
}

/// An NT time from a timestamp.
fn nt_time(time: diroxide_core::Timestamp) -> Option<NtTime> {
    let secs = u64::try_from(time.unix_seconds().checked_add(11_644_473_600)?).ok()?;
    let ticks = secs
        .checked_mul(10_000_000)?
        .checked_add(u64::from(time.subsec_nanos() / 100))?;
    Some(NtTime::from(ticks))
}

/// The archive entry for an item: its name, time, and Unix mode in the
/// high half of the attributes (7-Zip's own way, which the reader reads).
fn entry_of(item: &WriteItem) -> ArchiveEntry {
    let mut entry = match item.kind {
        ItemKind::Folder => ArchiveEntry::new_directory(&item.name),
        _ => ArchiveEntry::new_file(&item.name),
    };
    if let Some(time) = item.modified.and_then(nt_time) {
        entry.has_last_modified_date = true;
        entry.last_modified_date = time;
    }
    let (kind_bits, dos) = match item.kind {
        ItemKind::Folder => (0o040_000, 0x10),
        ItemKind::Link(_) => (0o120_000, 0x20),
        ItemKind::File => (0o100_000, 0x20),
    };
    entry.has_windows_attributes = true;
    entry.windows_attributes = ((kind_bits | item.stored_mode()) << 16) | 0x8000 | dos;
    entry
}

/// The 7z crate's error, in the provider's words: a member that could not
/// be read is that member's, a cancel a cancel.
fn failure_of(
    error: sevenz_rust2::Error,
    failed: &Mutex<Option<(usize, VfsError)>>,
) -> PackFailure {
    if let Some((n, cause)) = failed.lock().ok().and_then(|mut slot| slot.take()) {
        return match cause {
            VfsError::Cancelled => PackFailure::Cancelled,
            cause => PackFailure::Source(n, cause),
        };
    }
    match error {
        sevenz_rust2::Error::Io(error, _) => PackFailure::from_io(error),
        other => PackFailure::Write(VfsError::Corrupt {
            what: crate::guard::lowercase_first(&other.to_string()),
        }),
    }
}

/// Writes a 7z to a sink that seeks only as [`HeadHolder`] allows.
pub(crate) fn write_7z<W: Write + Seek>(
    out: W,
    items: &[WriteItem],
    contents: &dyn Contents,
    settings: &Settings<'_>,
    watch: &dyn Watch,
    records: &Records,
) -> Result<W, PackFailure> {
    let failed = Mutex::new(None);
    let mut writer = ArchiveWriter::new(out).map_err(|e| failure_of(e, &failed))?;
    let level = settings
        .level
        .number_for(ArchiveFormat::SevenZ)
        .unwrap_or(6);
    let mut methods = Vec::new();
    if let Some(password) = settings.password {
        methods.push(EncoderConfiguration::from(AesEncoderOptions::new(
            Password::new(password),
        )));
    }
    let dictionary = u64::from(
        lzma_rust2::Lzma2Options::with_preset(level)
            .lzma_options
            .dict_size,
    );
    let fit = (1024 / thread_mib(level)).max(1) as u32;
    let threads = settings.threads.min(fit);
    let lzma2 = if threads > 1 {
        Lzma2Options::from_level_mt(level, threads, dictionary.saturating_mul(4))
    } else {
        Lzma2Options::from_level(level)
    };
    methods.push(
        EncoderConfiguration::new(EncoderMethod::LZMA2).with_options(EncoderOptions::Lzma2(lzma2)),
    );
    writer.set_content_methods(methods);
    writer.set_encrypt_header(settings.password.is_some() && settings.encrypt_names);
    let mut block: Vec<(ArchiveEntry, usize)> = Vec::new();
    let mut block_bytes = 0u64;
    let flush = |writer: &mut ArchiveWriter<W>,
                 block: &mut Vec<(ArchiveEntry, usize)>|
     -> Result<(), PackFailure> {
        if block.is_empty() {
            return Ok(());
        }
        let (entries, members): (Vec<ArchiveEntry>, Vec<usize>) = block.drain(..).unzip();
        let readers: Vec<SourceReader<Lazy<'_>>> = members
            .into_iter()
            .map(|n| {
                SourceReader::new(Lazy {
                    contents,
                    n,
                    watch,
                    records,
                    feed: None,
                    failed: &failed,
                    finished: false,
                })
            })
            .collect();
        writer
            .push_archive_entries(entries, readers)
            .map(|_| ())
            .map_err(|e| failure_of(e, &failed))
    };
    for (n, item) in items.iter().enumerate() {
        watch.check().map_err(PackFailure::from_io)?;
        watch.member(&item.name);
        let entry = entry_of(item);
        match &item.kind {
            ItemKind::Folder => {
                writer
                    .push_archive_entry::<&[u8]>(entry, None)
                    .map_err(|e| failure_of(e, &failed))?;
            }
            ItemKind::Link(target) => {
                writer
                    .push_archive_entry(entry, Some(target.as_bytes()))
                    .map_err(|e| failure_of(e, &failed))?;
            }
            ItemKind::File if item.size == 0 => {
                // An empty file has no stream; its record says it is
                // there and empty. It is read once, for its record.
                let mut source = crate::write::feed(contents, n, watch, records)?;
                source.copy_into(&mut io::sink())?;
                writer
                    .push_archive_entry::<&[u8]>(entry, None)
                    .map_err(|e| failure_of(e, &failed))?;
            }
            ItemKind::File if settings.solid => {
                if !block.is_empty() && block_bytes.saturating_add(item.size) > settings.block_bytes
                {
                    flush(&mut writer, &mut block)?;
                    block_bytes = 0;
                }
                block.push((entry, n));
                block_bytes = block_bytes.saturating_add(item.size);
            }
            ItemKind::File => {
                let source = Lazy {
                    contents,
                    n,
                    watch,
                    records,
                    feed: None,
                    failed: &failed,
                    finished: false,
                };
                writer
                    .push_archive_entry(entry, Some(source))
                    .map_err(|e| failure_of(e, &failed))?;
            }
        }
    }
    flush(&mut writer, &mut block)?;
    writer
        .finish()
        .map_err(|e| failure_of(sevenz_rust2::Error::Io(e, "".into()), &failed))
}

/// The memory one LZMA2 encoder thread takes at a preset, in MiB.
fn thread_mib(preset: u32) -> u64 {
    match preset {
        0..=3 => 32,
        4..=6 => 100,
        7 => 190,
        8 => 370,
        _ => 680,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_header_is_held_back_and_the_body_streams() {
        let mut holder = HeadHolder::new(Vec::new());
        assert_eq!(holder.seek(SeekFrom::Start(32)).unwrap(), 32);
        holder.write_all(b"body bytes").unwrap();
        assert_eq!(holder.stream_position().unwrap(), 42);
        assert_eq!(holder.seek(SeekFrom::Start(0)).unwrap(), 0);
        holder.write_all(&[7u8; 32]).unwrap();
        assert!(holder.write_all(b"x").is_err(), "nothing past the header");
        assert!(holder.seek(SeekFrom::Start(5)).is_err());
        let (head, body) = holder.into_parts();
        assert_eq!(head, [7u8; 32]);
        assert_eq!(body, b"body bytes");
    }

    #[test]
    fn nt_times_count_from_1601() {
        let time = nt_time(diroxide_core::Timestamp::from_unix(0, 0)).unwrap();
        assert_eq!(u64::from(time), 116_444_736_000_000_000);
    }
}
