//! Tar reading, plain and inside gzip or xz ([`crate::read_xz`]):
//! research/archives.md 2, 4 and 6. The headers are parsed one by one with
//! the `tar` crate's header type, under the provider's own caps (the crate's
//! entry walk reads a long name or a PAX header whole, whatever it claims),
//! and the data is never handed to the crate: the provider seeks over it in
//! a plain tar and reads past it in a compressed one.
//!
//! A compressed tar is sequential: listing it is a pass over the whole
//! decompressed stream, a cancellable job with a progress line, and
//! reading one member means reading everything before it. The provider
//! keeps one forward cursor per archive: the next member read in archive
//! order continues the same decoder, an earlier one starts again, and a
//! read that must first pass more than [`COST_ASK_BYTES`] asks ("Read 3.2
//! GB first to open note.txt? y or n").

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use diroxide_core::format::grouped;
use diroxide_vfs::{CancelToken, OpContext, ProgressUpdate, VfsError};
use flate2::bufread::GzDecoder;

use crate::error::ArchiveError;
use crate::guard::{
    Allowances, Counting, LimitHit, Question, QuestionKind, check_cost, damaged, refuse, to_io,
};
use crate::index::{Index, IndexBuilder, Locator, Member, RecordFlags, RecordKind};
use crate::limits::{Limits, limit_words};
use crate::sniff::tar_checksum_ok;
use crate::source::Source;

/// A tar block.
const BLOCK: u64 = 512;

/// The most bytes a PAX or GNU extension header may hold (R7 group A).
const EXTENSION_MAX: u64 = 1 << 20;

/// The most extension headers in a row before a member.
const EXTENSIONS_IN_A_ROW: usize = 8;

/// Past this much to read first, reading one member alone asks.
pub const COST_ASK_BYTES: u64 = 256 << 20;

/// How often a listing reports progress, in decompressed bytes.
const PROGRESS_EVERY: u64 = 4 << 20;

/// How a tar is wrapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrapper {
    /// No compression: a plain tar, seekable.
    Plain,
    /// gzip, read member after member.
    Gzip,
    /// xz, with a memory limit for each block.
    Xz {
        /// The decoder's memory limit, in KiB.
        mem_limit_kb: u32,
    },
}

/// gzip members one after another, as one stream (RFC 1952 2.2): zero
/// padding after the last member is accepted, any other trailing bytes end
/// the stream and are noted ("extra data after the end, ignored").
pub(crate) struct GzStream<R: Read> {
    decoder: Option<GzDecoder<io::BufReader<R>>>,
    reader: Option<io::BufReader<R>>,
    trailing: Arc<AtomicBool>,
}

impl<R: Read> GzStream<R> {
    /// A stream over a reader; `trailing` is set when bytes that are not
    /// gzip follow the last member.
    pub(crate) fn new(inner: R, trailing: Arc<AtomicBool>) -> Self {
        Self {
            decoder: Some(GzDecoder::new(io::BufReader::with_capacity(
                64 << 10,
                inner,
            ))),
            reader: None,
            trailing,
        }
    }
}

impl<R: Read> Read for GzStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if let Some(decoder) = self.decoder.as_mut() {
                let n = decoder.read(buf)?;
                if n > 0 || buf.is_empty() {
                    return Ok(n);
                }
                let Some(decoder) = self.decoder.take() else {
                    return Ok(0);
                };
                self.reader = Some(decoder.into_inner());
            }
            let Some(mut reader) = self.reader.take() else {
                return Ok(0);
            };
            let next = io::BufRead::fill_buf(&mut reader)?;
            if next.is_empty() {
                return Ok(0);
            }
            if next.starts_with(&[0x1F, 0x8B]) {
                self.decoder = Some(GzDecoder::new(reader));
                continue;
            }
            if next.len() < 2 && next[0] == 0x1F {
                // One byte left that could start a header: not a member.
                self.trailing.store(true, Ordering::Relaxed);
                return Ok(0);
            }
            if !next.iter().all(|&b| b == 0) {
                self.trailing.store(true, Ordering::Relaxed);
                return Ok(0);
            }
            // Zero padding: consume it and look again.
            let len = next.len();
            io::BufRead::consume(&mut reader, len);
            self.reader = Some(reader);
        }
    }
}

/// A decoder of the whole stream from its start, counting the compressed
/// bytes it takes.
pub(crate) fn decoder(
    source: &Source,
    wrapper: Wrapper,
    consumed: Arc<AtomicU64>,
    trailing: Arc<AtomicBool>,
) -> Box<dyn Read + Send> {
    let mut start = source.clone();
    let _ = start.seek(SeekFrom::Start(0));
    let counted = Counting::new(start, consumed);
    match wrapper {
        Wrapper::Plain => Box::new(counted),
        Wrapper::Gzip => Box::new(GzStream::new(counted, trailing)),
        Wrapper::Xz { mem_limit_kb } => Box::new(lzma_rust2::XzReader::new_mem_limit(
            counted,
            true,
            mem_limit_kb,
        )),
    }
}

/// Reads until `buf` is full or the stream ends: the bytes read.
fn read_full(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match reader.read(&mut buf[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// What the listing pass watches as it goes: the limits, the answers, the
/// cancel token and the progress sink, and the bytes in and out.
struct Pass<'a> {
    limits: &'a Limits,
    allow: Allowances,
    ctx: &'a OpContext,
    name: &'a str,
    consumed: Arc<AtomicU64>,
    container_len: u64,
    out: u64,
    reported: u64,
    asked_total: bool,
}

impl Pass<'_> {
    /// Counts decompressed bytes: the budget, the ratio, the progress.
    fn count(&mut self, bytes: u64, current: &str) -> Result<(), ArchiveError> {
        self.out = self.out.saturating_add(bytes);
        if self.out > self.limits.max_total_bytes {
            return Err(refuse("bytes unpacked", limit_words(self.limits.max_total_bytes)).into());
        }
        if self.out > self.limits.ask_total_bytes && !self.allow.total && !self.asked_total {
            self.asked_total = true;
            return Err(LimitHit::Ask(Question {
                kind: QuestionKind::Total,
                text: format!(
                    "Reading {} means unpacking over {}. Go on",
                    self.name,
                    diroxide_core::format::human_size(self.limits.ask_total_bytes)
                ),
            })
            .into());
        }
        let packed = self.consumed.load(Ordering::Relaxed).max(1);
        if !self.allow.ratio
            && self.out > self.limits.ratio_floor_bytes
            && self.out / packed > self.limits.max_ratio
        {
            return Err(LimitHit::Ask(Question {
                kind: QuestionKind::Ratio,
                text: format!(
                    "Expands to {} from {}. Read {}",
                    diroxide_core::format::human_size(self.out),
                    diroxide_core::format::human_size(packed),
                    self.name
                ),
            })
            .into());
        }
        if self.out - self.reported >= PROGRESS_EVERY {
            self.reported = self.out;
            self.ctx.progress.report(ProgressUpdate {
                items_done: 0,
                items_total: None,
                bytes_done: self.consumed.load(Ordering::Relaxed),
                bytes_total: Some(self.container_len),
                current: Some(current.to_owned()),
            });
        }
        Ok(())
    }
}

/// The input of a listing pass: a plain tar seeks over the data, a
/// compressed one reads past it.
enum Input {
    Seekable(Source),
    Stream(Box<dyn Read + Send>),
}

impl Input {
    fn reader(&mut self) -> &mut dyn Read {
        match self {
            Input::Seekable(source) => source,
            Input::Stream(stream) => stream.as_mut(),
        }
    }

    /// Moves past `bytes` of data, checking the cancel flag every 64 KiB.
    fn skip(&mut self, bytes: u64, pass: &mut Pass<'_>, current: &str) -> Result<(), ArchiveError> {
        match self {
            Input::Seekable(source) => {
                let at = source.stream_position().map_err(VfsError::from)?;
                let to = at
                    .checked_add(bytes)
                    .filter(|&to| to <= source.len())
                    .ok_or_else(|| {
                        damaged(&format!("{current} runs past the end of the archive"))
                    })?;
                source.seek(SeekFrom::Start(to)).map_err(VfsError::from)?;
                pass.ctx.check()?;
                Ok(())
            }
            Input::Stream(stream) => {
                let mut left = bytes;
                let mut chunk = vec![0u8; 64 << 10];
                while left > 0 {
                    pass.ctx.check()?;
                    let want = usize::try_from(left.min(chunk.len() as u64)).unwrap_or(chunk.len());
                    let n = read_full(stream.as_mut(), &mut chunk[..want])
                        .map_err(crate::guard::from_io)?;
                    if n == 0 {
                        return Err(damaged(&format!(
                            "{current} runs past the end of the archive"
                        ))
                        .into());
                    }
                    left -= n as u64;
                    pass.count(n as u64, current)?;
                }
                Ok(())
            }
        }
    }
}

/// Rounds a size up to whole blocks.
fn padded(size: u64) -> Option<u64> {
    size.checked_add(BLOCK - 1).map(|n| n / BLOCK * BLOCK)
}

/// The text of a NUL-ended field.
fn text_of(bytes: &[u8]) -> (String, bool) {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), false),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
    }
}

/// Reads an extension header's data, under its cap.
fn extension(
    input: &mut Input,
    size: u64,
    what: &str,
    pass: &mut Pass<'_>,
) -> Result<Vec<u8>, ArchiveError> {
    if size > EXTENSION_MAX {
        return Err(refuse(what, limit_words(EXTENSION_MAX)).into());
    }
    let padded = padded(size).ok_or_else(|| damaged("an extension header is too large"))?;
    let mut data = vec![0u8; padded as usize];
    let n = read_full(input.reader(), &mut data).map_err(crate::guard::from_io)?;
    if n != data.len() {
        return Err(damaged("it ends in the middle of an extension header").into());
    }
    pass.count(padded, what)?;
    data.truncate(size as usize);
    Ok(data)
}

/// Walks a tar's headers into the index.
pub(crate) fn list(
    source: &Source,
    wrapper: Wrapper,
    container: &str,
    limits: &Limits,
    ctx: &OpContext,
    allow: Allowances,
) -> Result<Index, ArchiveError> {
    let consumed = Arc::new(AtomicU64::new(0));
    let trailing = Arc::new(AtomicBool::new(false));
    let mut input = match wrapper {
        Wrapper::Plain => {
            let mut start = source.clone();
            let _ = start.seek(SeekFrom::Start(0));
            Input::Seekable(start)
        }
        _ => Input::Stream(decoder(source, wrapper, consumed.clone(), trailing.clone())),
    };
    let mut pass = Pass {
        limits,
        allow,
        ctx,
        name: container,
        consumed: consumed.clone(),
        container_len: source.len(),
        out: 0,
        reported: 0,
        asked_total: false,
    };
    let mut builder = IndexBuilder::new(*limits);
    let mut offset = 0u64;
    let mut entries = 0u64;
    let mut long_name: Option<Vec<u8>> = None;
    let mut long_link: Option<Vec<u8>> = None;
    let mut pax: Option<Vec<u8>> = None;
    let mut in_a_row = 0usize;
    let mut block = [0u8; BLOCK as usize];
    let mut ended = false;
    loop {
        ctx.check()?;
        let n = read_full(input.reader(), &mut block).map_err(crate::guard::from_io)?;
        if n == 0 {
            break;
        }
        if n < block.len() {
            builder.note(format!("{container} ends in the middle of a header."));
            break;
        }
        if matches!(input, Input::Stream(_)) {
            pass.count(BLOCK, container)?;
        }
        offset += BLOCK;
        if block.iter().all(|&b| b == 0) {
            ended = true;
            break;
        }
        if !tar_checksum_ok(&block) {
            if entries == 0 && long_name.is_none() && pax.is_none() {
                return Err(damaged("its first header is not a tar header").into());
            }
            builder.note(format!(
                "{container} is damaged after {} entries: the rest is not listed.",
                grouped(entries)
            ));
            break;
        }
        let header = tar::Header::from_byte_slice(&block);
        let kind = header.entry_type();
        let stored = header
            .entry_size()
            .map_err(|_| damaged("a header's size is not a number"))?;
        let is_extension = kind.is_gnu_longname()
            || kind.is_gnu_longlink()
            || kind.is_pax_local_extensions()
            || kind.is_pax_global_extensions();
        if is_extension {
            in_a_row += 1;
            if in_a_row > EXTENSIONS_IN_A_ROW {
                return Err(refuse(
                    "extension headers in a row",
                    EXTENSIONS_IN_A_ROW.to_string(),
                )
                .into());
            }
            let data = extension(&mut input, stored, "extension header size", &mut pass)?;
            offset += padded(stored).unwrap_or(0);
            if kind.is_gnu_longname() {
                long_name = Some(data);
            } else if kind.is_gnu_longlink() {
                long_link = Some(data);
            } else if kind.is_pax_local_extensions() {
                pax = Some(data);
            }
            continue;
        }
        in_a_row = 0;
        entries += 1;
        // The PAX records, if any: path, linkpath and size.
        let mut pax_path = None;
        let mut pax_link = None;
        let mut pax_size = None;
        let mut pax_time = None;
        if let Some(records) = pax.take() {
            for record in tar::PaxExtensions::new(&records).flatten() {
                match record.key() {
                    Ok("path") => pax_path = Some(record.value_bytes().to_vec()),
                    Ok("linkpath") => pax_link = Some(record.value_bytes().to_vec()),
                    Ok("size") => {
                        pax_size = record.value().ok().and_then(|v| v.parse::<u64>().ok())
                    }
                    Ok("mtime") => {
                        pax_time = record
                            .value()
                            .ok()
                            .and_then(|v| v.split('.').next().and_then(|s| s.parse::<i64>().ok()));
                    }
                    _ => {}
                }
            }
        }
        let name_bytes = pax_path
            .or_else(|| long_name.take())
            .unwrap_or_else(|| header.path_bytes().into_owned());
        long_name = None;
        let (mut name, not_unicode) = text_of(&name_bytes);
        let mut flags = RecordFlags::NONE;
        let mut note = None;
        let size = match pax_size {
            Some(pax) if pax != stored => {
                flags.insert(RecordFlags::SIZES_DISAGREE);
                pax
            }
            Some(pax) => pax,
            None => stored,
        };
        let link = pax_link
            .or_else(|| long_link.take())
            .or_else(|| header.link_name_bytes().map(|l| l.into_owned()));
        long_link = None;
        let record_kind = match kind.as_byte() {
            b'0' | b'\0' | b'7' => RecordKind::File,
            b'5' => RecordKind::Folder,
            b'2' => RecordKind::Link,
            b'1' => {
                let target = link.as_deref().map(|l| text_of(l).0).unwrap_or_default();
                note = Some(format!("A hard link to {target}."));
                RecordKind::Special
            }
            b'S' => {
                flags.insert(RecordFlags::UNREADABLE);
                note = Some("Sparse data is not read here.".to_owned());
                RecordKind::File
            }
            _ => RecordKind::Special,
        };
        if record_kind == RecordKind::Folder && !name.ends_with('/') {
            name.push('/');
        }
        let data_at = offset;
        let mode = header.mode().ok();
        let modified = pax_time.or_else(|| header.mtime().ok().and_then(|t| i64::try_from(t).ok()));
        let link_target = (record_kind == RecordKind::Link)
            .then(|| link.as_deref().map(|l| text_of(l).0))
            .flatten();
        builder.add(Member {
            name: name.clone(),
            not_unicode,
            kind: Some(record_kind),
            size,
            packed: matches!(wrapper, Wrapper::Plain).then_some(size),
            modified,
            mode,
            crc32: None,
            link_target,
            locator: Locator::Tar(data_at),
            flags,
            note,
        })?;
        // The data that follows: a folder, link or device has none, whatever
        // its size field says, except in a sparse member's map.
        let data = match record_kind {
            RecordKind::File => size,
            _ if kind.as_byte() == b'S' => stored,
            _ => 0,
        };
        let skip = padded(data).ok_or_else(|| damaged("a member's size is too large"))?;
        input.skip(skip, &mut pass, &name)?;
        offset = offset
            .checked_add(skip)
            .ok_or_else(|| damaged("a member's size is too large"))?;
    }
    if !ended {
        builder.note(format!("{container} has no end marker."));
    }
    if trailing.load(Ordering::Relaxed) {
        builder.note(format!(
            "{container} has extra data after the end, ignored."
        ));
    }
    ctx.progress.report(ProgressUpdate {
        items_done: entries,
        items_total: Some(entries),
        bytes_done: consumed
            .load(Ordering::Relaxed)
            .max(source.len().min(offset)),
        bytes_total: Some(source.len()),
        current: None,
    });
    Ok(builder.finish())
}

/// Looks inside a gzip or xz stream: whether its first block is a tar
/// header.
pub(crate) fn holds_tar(source: &Source, wrapper: Wrapper) -> bool {
    let mut stream = decoder(
        source,
        wrapper,
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let mut block = [0u8; BLOCK as usize];
    match read_full(stream.as_mut(), &mut block) {
        Ok(512) => block.get(257..262) == Some(b"ustar".as_slice()) || tar_checksum_ok(&block),
        _ => false,
    }
}

/// A decoder and how far it has got.
struct Cursor {
    decoder: Box<dyn Read + Send>,
    pos: u64,
}

/// A tar opened: its bytes, its wrapper, and the forward cursor.
pub(crate) struct TarBackend {
    source: Source,
    wrapper: Wrapper,
    cursor: Arc<Mutex<Option<Cursor>>>,
    starts: AtomicU64,
}

impl fmt::Debug for TarBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TarBackend")
            .field("wrapper", &self.wrapper)
            .finish_non_exhaustive()
    }
}

impl TarBackend {
    /// A backend over a listed tar.
    pub(crate) fn new(source: Source, wrapper: Wrapper) -> Self {
        Self {
            source,
            wrapper,
            cursor: Arc::new(Mutex::new(None)),
            starts: AtomicU64::new(0),
        }
    }

    /// How many times the decoder started from the archive's start to read
    /// members: one for a whole unpack in archive order.
    pub(crate) fn passes(&self) -> u64 {
        self.starts.load(Ordering::Relaxed)
    }

    /// Whether members are read with random access: a plain tar only.
    pub(crate) fn random_read(&self) -> bool {
        self.wrapper == Wrapper::Plain
    }

    /// The decompressed bytes that must be read before a member: what the
    /// cost question weighs.
    pub(crate) fn ahead(&self, index: &Index, id: u32) -> u64 {
        let Some(Locator::Tar(data)) = index.record(id).map(|r| r.locator) else {
            return 0;
        };
        if self.random_read() {
            return 0;
        }
        let pos = self
            .cursor
            .lock()
            .ok()
            .and_then(|c| c.as_ref().map(|c| c.pos))
            .filter(|&pos| pos <= data)
            .unwrap_or(0);
        data - pos
    }

    /// A member of a plain tar as a window of the archive's bytes, for a
    /// random-access read; `None` in a compressed tar.
    pub(crate) fn window(&self, index: &Index, id: u32) -> Option<Source> {
        if !self.random_read() {
            return None;
        }
        let record = index.record(id)?;
        let Locator::Tar(data) = record.locator else {
            return None;
        };
        if record.flags.contains(RecordFlags::UNREADABLE)
            || data.checked_add(record.size)? > self.source.len()
        {
            return None;
        }
        Some(self.source.window(data, record.size))
    }

    /// Opens a member: a window of a plain tar, or the forward cursor moved
    /// to it in a compressed one, after the cost question.
    pub(crate) fn read(
        &self,
        index: &Index,
        id: u32,
        cancel: &CancelToken,
        allow: Allowances,
        cost_ask_bytes: u64,
    ) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let record = index.record(id).ok_or(VfsError::NotFound)?;
        if record.flags.contains(RecordFlags::UNREADABLE) {
            return Err(VfsError::UnsupportedFeature {
                what: "sparse tar data".to_owned(),
            }
            .into());
        }
        let Locator::Tar(data) = record.locator else {
            return Err(VfsError::NotFound.into());
        };
        let size = record.size;
        if self.random_read() {
            if data
                .checked_add(size)
                .is_none_or(|end| end > self.source.len())
            {
                return Err(damaged("the member runs past the end of the archive").into());
            }
            let mut window = self.source.window(data, size);
            let _ = window.seek(SeekFrom::Start(0));
            return Ok(Box::new(window));
        }
        check_cost(index.name(id), self.ahead(index, id), cost_ask_bytes, allow)?;
        let taken = self.cursor.lock().ok().and_then(|mut slot| slot.take());
        let mut cursor = match taken {
            Some(cursor) if cursor.pos <= data => cursor,
            _ => {
                self.starts.fetch_add(1, Ordering::Relaxed);
                Cursor {
                    decoder: decoder(
                        &self.source,
                        self.wrapper,
                        Arc::new(AtomicU64::new(0)),
                        Arc::new(AtomicBool::new(false)),
                    ),
                    pos: 0,
                }
            }
        };
        let mut chunk = vec![0u8; 64 << 10];
        while cursor.pos < data {
            cancel.check()?;
            let want =
                usize::try_from((data - cursor.pos).min(chunk.len() as u64)).unwrap_or(chunk.len());
            let n = read_full(cursor.decoder.as_mut(), &mut chunk[..want])
                .map_err(crate::guard::from_io)?;
            if n == 0 {
                return Err(damaged("the archive ends before the member").into());
            }
            cursor.pos += n as u64;
        }
        Ok(Box::new(CursorReader {
            cursor: Some(cursor),
            left: size,
            slot: self.cursor.clone(),
        }))
    }
}

/// A member read from the forward cursor; when dropped, the cursor goes
/// back for the next read in order.
struct CursorReader {
    cursor: Option<Cursor>,
    left: u64,
    slot: Arc<Mutex<Option<Cursor>>>,
}

impl Read for CursorReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(cursor) = self.cursor.as_mut() else {
            return Ok(0);
        };
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let want = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = cursor.decoder.read(&mut buf[..want])?;
        if n == 0 {
            return Err(to_io(damaged("the archive ends in the middle of a member")));
        }
        cursor.pos += n as u64;
        self.left -= n as u64;
        Ok(n)
    }
}

impl Drop for CursorReader {
    fn drop(&mut self) {
        if let (Some(cursor), Ok(mut slot)) = (self.cursor.take(), self.slot.lock()) {
            *slot = Some(cursor);
        }
    }
}

/// One compressed file shown as an archive of one member: a plain xz, or
/// a gzip that holds no tar.
pub(crate) struct SingleBackend {
    source: Source,
    wrapper: Wrapper,
}

impl fmt::Debug for SingleBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SingleBackend")
            .field("wrapper", &self.wrapper)
            .finish_non_exhaustive()
    }
}

impl SingleBackend {
    /// A backend over a compressed file.
    pub(crate) fn new(source: Source, wrapper: Wrapper) -> Self {
        Self { source, wrapper }
    }

    /// The whole decompressed stream, from its start.
    pub(crate) fn read(&self) -> Box<dyn Read + Send> {
        decoder(
            &self.source,
            self.wrapper,
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        )
    }
}

/// The member name of a compressed file: its name without the last
/// extension ("notes.txt.gz" holds "notes.txt", "logs.tgz" holds
/// "logs.tar"), or the name and ".data" when nothing would be left.
pub(crate) fn single_name(container: &str) -> String {
    let lower = container.to_lowercase();
    for (suffix, replacement) in [(".tgz", ".tar"), (".txz", ".tar"), (".gz", ""), (".xz", "")] {
        if lower.ends_with(suffix) && container.len() > suffix.len() {
            return format!(
                "{}{replacement}",
                &container[..container.len() - suffix.len()]
            );
        }
    }
    format!("{container}.data")
}

/// The index of one compressed file: one member, its size when known.
pub(crate) fn single_index(
    container: &str,
    size: Option<u64>,
    limits: &Limits,
) -> Result<Index, ArchiveError> {
    let mut builder = IndexBuilder::new(*limits);
    let mut flags = RecordFlags::NONE;
    if size.is_none() {
        flags.insert(RecordFlags::SIZE_UNKNOWN);
    }
    builder.add(Member {
        name: single_name(container),
        kind: Some(RecordKind::File),
        size: size.unwrap_or(0),
        packed: None,
        locator: Locator::Whole,
        flags,
        ..Member::default()
    })?;
    Ok(builder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_members_are_named_by_their_container() {
        assert_eq!(single_name("notes.txt.gz"), "notes.txt");
        assert_eq!(single_name("logs.TGZ"), "logs.tar");
        assert_eq!(single_name("big.xz"), "big");
        assert_eq!(single_name("odd"), "odd.data");
        assert_eq!(single_name(".gz"), ".gz.data");
    }

    fn member(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn gzip_members_read_as_one_with_padding_and_garbage_said() {
        let mut two = member(b"first ");
        two.extend_from_slice(&member(b"second"));
        let trailing = Arc::new(AtomicBool::new(false));
        let mut out = String::new();
        GzStream::new(two.as_slice(), trailing.clone())
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "first second");
        assert!(!trailing.load(Ordering::Relaxed));
        let mut padded = member(b"padded");
        padded.extend_from_slice(&[0u8; 100]);
        let mut out = String::new();
        GzStream::new(padded.as_slice(), trailing.clone())
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "padded");
        assert!(!trailing.load(Ordering::Relaxed), "zero padding is fine");
        let mut garbage = member(b"kept");
        garbage.extend_from_slice(b"GARBAGE");
        let mut out = String::new();
        GzStream::new(garbage.as_slice(), trailing.clone())
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "kept");
        assert!(trailing.load(Ordering::Relaxed), "the garbage is noted");
    }
}
