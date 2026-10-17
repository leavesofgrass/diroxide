//! The parts reader: numbered parts read as one stream, with random access
//! (plan section 5.4; research/alpha2-plan.md, spike P1 and commit 20).
//!
//! A split file (`report.pdf.001`, `report.pdf.002`) or a split archive
//! (`backup.7z.001` and its siblings) is read through one [`PartsReader`],
//! so nothing is joined just to browse it, and a join reads its parts the
//! same way. The rules:
//!
//! - **A size snapshot.** The parts and their sizes are fixed when the
//!   reader is made; every offset is computed from them in unsigned 64
//!   bits, so a set past 4 GiB seeks exactly. A part of zero bytes is
//!   refused: every joiner would read the set differently.
//! - **Changes are said, never read.** Before a part is opened it is read
//!   again and compared with the snapshot; a part that ends early while it
//!   is read is the same. Both give [`PartsError::Changed`], "part
//!   report.pdf.004 changed since it was listed", and never other bytes.
//! - **At most two handles.** Only [`PartsReader::MAX_OPEN`] parts are open
//!   at once, the least recently used closed first, so a set of ten
//!   thousand parts never runs the process out of handles, and a zip
//!   reader that goes back and forth between its central directory in the
//!   last part and a member in the first keeps both open.
//! - **Any provider.** A part is opened with random access where its
//!   provider has it, and read forward otherwise (a forward seek skips, a
//!   backward one opens the part again).

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use diroxide_core::names::spell_out;
use diroxide_core::{Entry, Place, Timestamp};
use diroxide_vfs::{OpContext, Provider, ReadSeek, VfsError};
use thiserror::Error;

/// One part as the snapshot saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartInfo {
    /// Where the part is.
    pub place: Place,
    /// Its size in bytes when the parts were listed.
    pub size: u64,
    /// Its modified time when the parts were listed, when known.
    pub modified: Option<Timestamp>,
}

impl PartInfo {
    /// A part of this size, its time not known.
    pub fn new(place: Place, size: u64) -> Self {
        Self {
            place,
            size,
            modified: None,
        }
    }

    /// A part as an entry describes it.
    pub fn of(place: &Place, entry: &Entry) -> Self {
        Self {
            place: place.clone(),
            size: entry.size.unwrap_or(0),
            modified: entry.modified,
        }
    }

    /// The part's name, with hidden characters spelled out.
    pub fn name(&self) -> String {
        self.place.name().map_or_else(
            || spell_out(&self.place.to_string()).into_owned(),
            |name| spell_out(name).into_owned(),
        )
    }
}

/// Why parts could not be read as one. The text is the cause in lowercase
/// words, for "Could not VERB OBJECT: CAUSE." messages.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PartsError {
    /// There are no parts at all.
    #[error("there are no parts")]
    NoParts,
    /// A part has no bytes.
    #[error("part {name} is empty")]
    Empty {
        /// The part's index, from 0.
        part: usize,
        /// Its name.
        name: String,
    },
    /// A part is not what the snapshot saw: a new size or time before it
    /// was opened, or an early end while it was read.
    #[error("part {name} changed since it was listed")]
    Changed {
        /// The part's index, from 0.
        part: usize,
        /// Its name.
        name: String,
    },
    /// The parts add up to more bytes than 64 bits count.
    #[error("the parts add up to more bytes than can be counted")]
    TooLarge,
    /// A provider failed.
    #[error(transparent)]
    Vfs(#[from] VfsError),
}

impl PartsError {
    /// The parts error an I/O error from a [`PartsReader`] carries, if it
    /// carries one.
    pub fn of(error: &io::Error) -> Option<&PartsError> {
        error.get_ref().and_then(|e| e.downcast_ref::<PartsError>())
    }

    /// The error as a provider's error, for a provider that reads through
    /// the parts (a split archive): a change is [`VfsError::Changed`], an
    /// empty part damage with its name.
    pub fn into_vfs(self) -> VfsError {
        match self {
            PartsError::NoParts => VfsError::NotFound,
            PartsError::Empty { name, .. } => VfsError::Corrupt {
                what: format!("part {name} is empty"),
            },
            PartsError::Changed { .. } => VfsError::Changed,
            PartsError::TooLarge => VfsError::LimitReached {
                what: "the size of a set of parts".to_owned(),
                limit: "16 EB".to_owned(),
            },
            PartsError::Vfs(error) => error,
        }
    }
}

impl From<PartsError> for io::Error {
    fn from(error: PartsError) -> Self {
        match error {
            // A provider's own error travels as it came, so the engine
            // reads the same error back from the stream.
            PartsError::Vfs(inner) => io::Error::other(inner),
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        }
    }
}

/// A part opened for reading, from its start.
pub enum Opened {
    /// With random access.
    Seekable(Box<dyn ReadSeek>),
    /// Forward only: a seek ahead skips, a seek back opens it again.
    Forward(Box<dyn Read + Send>),
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Opened::Seekable(_) => "Opened::Seekable",
            Opened::Forward(_) => "Opened::Forward",
        })
    }
}

/// Opens parts for a [`PartsReader`]: a provider, or a test's own.
pub trait PartOpener: Send {
    /// Opens a part from its start.
    fn open(&self, part: &PartInfo) -> Result<Opened, VfsError>;

    /// Whether a part is still what the snapshot saw, asked before it is
    /// opened. The default trusts the snapshot.
    fn unchanged(&self, part: &PartInfo) -> Result<bool, VfsError> {
        let _ = part;
        Ok(true)
    }
}

/// Opens parts through their provider: with random access where it has
/// it, forward otherwise; a part is read again before it is opened.
struct ProviderOpener {
    provider: Arc<dyn Provider>,
    ctx: OpContext,
}

impl PartOpener for ProviderOpener {
    fn open(&self, part: &PartInfo) -> Result<Opened, VfsError> {
        match self.provider.open_seekable(&part.place, &self.ctx) {
            Ok(reader) => Ok(Opened::Seekable(reader)),
            Err(VfsError::Unsupported { .. }) => {
                Ok(Opened::Forward(self.provider.read(&part.place, &self.ctx)?))
            }
            Err(error) => Err(error),
        }
    }

    fn unchanged(&self, part: &PartInfo) -> Result<bool, VfsError> {
        let entry = match self.provider.stat(&part.place, &self.ctx) {
            Ok(entry) => entry,
            Err(VfsError::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        let same_time = match (part.modified, entry.modified) {
            (Some(then), Some(now)) => then == now,
            _ => true,
        };
        Ok(entry.size == Some(part.size) && same_time)
    }
}

/// One open part and where its reader is within it.
struct Handle {
    part: usize,
    reader: Opened,
    at: u64,
}

/// Numbered parts read as one stream, with random access: seeks across
/// part boundaries return the same bytes as the whole would. Every
/// `Read + Seek + Send` caller, a zip or 7z reader among them, can read
/// through it.
///
/// Errors come as I/O errors carrying a [`PartsError`]
/// ([`PartsError::of`]) or, from a provider, the provider's own
/// [`VfsError`].
pub struct PartsReader {
    parts: Vec<PartInfo>,
    starts: Vec<u64>,
    total: u64,
    position: u64,
    handles: Vec<Handle>,
    opener: Box<dyn PartOpener>,
}

impl std::fmt::Debug for PartsReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartsReader")
            .field("parts", &self.parts.len())
            .field("total", &self.total)
            .field("position", &self.position)
            .field("open", &self.handles.len())
            .finish_non_exhaustive()
    }
}

impl PartsReader {
    /// The most parts open at once.
    pub const MAX_OPEN: usize = 2;

    /// The parts at these places, in this order, read through their
    /// provider. Each is read once now for the size snapshot.
    pub fn open(
        provider: Arc<dyn Provider>,
        places: &[Place],
        ctx: &OpContext,
    ) -> Result<Self, PartsError> {
        let mut parts = Vec::with_capacity(places.len());
        for place in places {
            ctx.check()?;
            let entry = provider.stat(place, ctx)?;
            parts.push(PartInfo::of(place, &entry));
        }
        Self::with_parts(provider, parts, ctx)
    }

    /// These parts, their snapshot already taken (from a listing), read
    /// through their provider.
    pub fn with_parts(
        provider: Arc<dyn Provider>,
        parts: Vec<PartInfo>,
        ctx: &OpContext,
    ) -> Result<Self, PartsError> {
        Self::with_opener(
            parts,
            ProviderOpener {
                provider,
                ctx: ctx.clone(),
            },
        )
    }

    /// These parts, opened by `opener`.
    pub fn with_opener(
        parts: Vec<PartInfo>,
        opener: impl PartOpener + 'static,
    ) -> Result<Self, PartsError> {
        if parts.is_empty() {
            return Err(PartsError::NoParts);
        }
        let mut starts = Vec::with_capacity(parts.len());
        let mut total: u64 = 0;
        for (index, part) in parts.iter().enumerate() {
            if part.size == 0 {
                return Err(PartsError::Empty {
                    part: index,
                    name: part.name(),
                });
            }
            starts.push(total);
            total = total.checked_add(part.size).ok_or(PartsError::TooLarge)?;
        }
        Ok(Self {
            parts,
            starts,
            total,
            position: 0,
            handles: Vec::with_capacity(Self::MAX_OPEN),
            opener: Box::new(opener),
        })
    }

    /// The bytes of all the parts together, as the snapshot saw them.
    pub fn len(&self) -> u64 {
        self.total
    }

    /// Whether there are no bytes; never, since empty parts are refused.
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// The parts, in order.
    pub fn parts(&self) -> &[PartInfo] {
        &self.parts
    }

    /// The index of the part that holds the byte at `offset`, or `None`
    /// past the end.
    pub fn part_at(&self, offset: u64) -> Option<usize> {
        if offset >= self.total {
            return None;
        }
        Some(self.starts.partition_point(|start| *start <= offset) - 1)
    }

    /// Where a part starts in the whole.
    pub fn start_of(&self, part: usize) -> Option<u64> {
        self.starts.get(part).copied()
    }

    /// The position in the whole.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// How many parts are open now: never more than
    /// [`PartsReader::MAX_OPEN`].
    pub fn open_handles(&self) -> usize {
        self.handles.len()
    }

    fn changed(&self, part: usize) -> PartsError {
        PartsError::Changed {
            part,
            name: self.parts[part].name(),
        }
    }

    /// Reads and drops `count` bytes from a forward reader.
    fn skip(&self, part: usize, reader: &mut dyn Read, mut count: u64) -> io::Result<()> {
        let mut scratch = [0u8; 8192];
        while count > 0 {
            let want = usize::try_from(count)
                .unwrap_or(usize::MAX)
                .min(scratch.len());
            match reader.read(&mut scratch[..want]) {
                Ok(0) => return Err(self.changed(part).into()),
                Ok(read) => count -= read as u64,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Opens a part, after checking it against the snapshot, closing the
    /// least recently used part first when two are open.
    fn open_part(&mut self, part: usize) -> io::Result<Handle> {
        let info = &self.parts[part];
        if !self.opener.unchanged(info).map_err(PartsError::from)? {
            return Err(self.changed(part).into());
        }
        while self.handles.len() >= Self::MAX_OPEN {
            self.handles.remove(0);
        }
        let reader = self.opener.open(info).map_err(PartsError::from)?;
        Ok(Handle {
            part,
            reader,
            at: 0,
        })
    }

    /// The handle of a part, open and placed at `within`, as the most
    /// recently used.
    fn place_handle(&mut self, part: usize, within: u64) -> io::Result<&mut Handle> {
        let mut handle = match self.handles.iter().position(|h| h.part == part) {
            Some(index) => self.handles.remove(index),
            None => self.open_part(part)?,
        };
        if handle.at != within {
            if matches!(handle.reader, Opened::Forward(_)) && within < handle.at {
                // A forward reader cannot go back: open the part again.
                drop(handle);
                handle = self.open_part(part)?;
            }
            let ahead = within.saturating_sub(handle.at);
            match &mut handle.reader {
                Opened::Seekable(reader) => {
                    reader.seek(SeekFrom::Start(within))?;
                }
                Opened::Forward(reader) => self.skip(part, reader.as_mut(), ahead)?,
            }
            handle.at = within;
        }
        self.handles.push(handle);
        let last = self.handles.len() - 1;
        Ok(&mut self.handles[last])
    }
}

impl Read for PartsReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.position >= self.total {
            return Ok(0);
        }
        let Some(part) = self.part_at(self.position) else {
            return Ok(0);
        };
        let within = self.position - self.starts[part];
        let left = self.parts[part].size - within;
        let want = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
        let handle = self.place_handle(part, within)?;
        let read = loop {
            let result = match &mut handle.reader {
                Opened::Seekable(reader) => reader.read(&mut buf[..want]),
                Opened::Forward(reader) => reader.read(&mut buf[..want]),
            };
            match result {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                other => break other?,
            }
        };
        if read == 0 {
            // The part ends before the snapshot said: it was cut short.
            self.handles.retain(|h| h.part != part);
            return Err(self.changed(part).into());
        }
        handle.at += read as u64;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for PartsReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.total.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        match target {
            Some(offset) => {
                self.position = offset;
                Ok(offset)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a seek before the start of the parts",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use proptest::prelude::*;

    fn place(n: usize) -> Place {
        let text = if cfg!(windows) {
            format!("D:\\set\\whole.bin.{n:03}")
        } else {
            format!("/set/whole.bin.{n:03}")
        };
        Place::parse(&text).unwrap()
    }

    /// Counts the parts open at once, through a guard on each reader.
    #[derive(Clone, Default)]
    struct Counter {
        live: Arc<AtomicUsize>,
        most: Arc<AtomicUsize>,
        opened: Arc<AtomicUsize>,
    }

    struct Counted<R> {
        inner: R,
        live: Arc<AtomicUsize>,
    }

    impl<R: Read> Read for Counted<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl<R: Seek> Seek for Counted<R> {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    impl<R> Drop for Counted<R> {
        fn drop(&mut self) {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Counter {
        fn guard<R>(&self, inner: R) -> Counted<R> {
            let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            self.opened.fetch_add(1, Ordering::SeqCst);
            Counted {
                inner,
                live: Arc::clone(&self.live),
            }
        }
    }

    /// Parts in memory, opened with random access or forward only; their
    /// bytes can be swapped under the reader.
    struct Memory {
        data: Arc<Mutex<Vec<Vec<u8>>>>,
        forward: bool,
        counter: Counter,
    }

    type Shared = Arc<Mutex<Vec<Vec<u8>>>>;

    impl Memory {
        fn reader(parts: &[Vec<u8>], forward: bool) -> (PartsReader, Counter) {
            let (reader, counter, _) = Self::shared(parts, forward);
            (reader, counter)
        }

        /// A reader, its counter, and the bytes behind it, to change.
        fn shared(parts: &[Vec<u8>], forward: bool) -> (PartsReader, Counter, Shared) {
            let counter = Counter::default();
            let infos = parts
                .iter()
                .enumerate()
                .map(|(n, bytes)| PartInfo::new(place(n + 1), bytes.len() as u64))
                .collect();
            let data = Arc::new(Mutex::new(parts.to_vec()));
            let memory = Memory {
                data: Arc::clone(&data),
                forward,
                counter: counter.clone(),
            };
            (
                PartsReader::with_opener(infos, memory).unwrap(),
                counter,
                data,
            )
        }
    }

    impl PartOpener for Memory {
        fn open(&self, part: &PartInfo) -> Result<Opened, VfsError> {
            let index = (1..=self.data.lock().unwrap().len())
                .find(|n| place(*n) == part.place)
                .ok_or(VfsError::NotFound)?;
            let bytes = self.data.lock().unwrap()[index - 1].clone();
            let reader = self.counter.guard(Cursor::new(bytes));
            Ok(if self.forward {
                Opened::Forward(Box::new(reader))
            } else {
                Opened::Seekable(Box::new(reader))
            })
        }
    }

    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    #[test]
    fn reads_across_boundaries_equal_the_whole() {
        let parts: Vec<Vec<u8>> = vec![b"abc".to_vec(), b"defg".to_vec(), b"h".to_vec()];
        for forward in [false, true] {
            let (mut reader, counter) = Memory::reader(&parts, forward);
            assert_eq!(reader.len(), 8);
            let mut all = Vec::new();
            reader.read_to_end(&mut all).unwrap();
            assert_eq!(all, b"abcdefgh");
            reader.seek(SeekFrom::Start(2)).unwrap();
            let mut five = [0u8; 5];
            reader.read_exact(&mut five).unwrap();
            assert_eq!(&five, b"cdefg");
            reader.seek(SeekFrom::End(-1)).unwrap();
            let mut last = [0u8; 4];
            assert_eq!(reader.read(&mut last).unwrap(), 1);
            assert_eq!(last[0], b'h');
            assert_eq!(reader.read(&mut last).unwrap(), 0, "the end");
            assert!(counter.most.load(Ordering::SeqCst) <= PartsReader::MAX_OPEN);
            assert_eq!(reader.part_at(3), Some(1));
            assert_eq!(reader.part_at(8), None);
            assert_eq!(reader.start_of(2), Some(7));
        }
    }

    #[test]
    fn empty_parts_and_no_parts_are_refused() {
        let infos = vec![PartInfo::new(place(1), 4), PartInfo::new(place(2), 0)];
        let refused = PartsReader::with_opener(
            infos,
            Memory {
                data: Arc::new(Mutex::new(Vec::new())),
                forward: false,
                counter: Counter::default(),
            },
        )
        .unwrap_err();
        assert_eq!(
            refused,
            PartsError::Empty {
                part: 1,
                name: "whole.bin.002".into()
            }
        );
        assert_eq!(refused.to_string(), "part whole.bin.002 is empty");
        let none = PartsReader::with_opener(
            Vec::new(),
            Memory {
                data: Arc::new(Mutex::new(Vec::new())),
                forward: false,
                counter: Counter::default(),
            },
        )
        .unwrap_err();
        assert_eq!(none, PartsError::NoParts);
    }

    #[test]
    fn a_seek_before_the_start_is_an_error_and_past_the_end_reads_nothing() {
        let (mut reader, _) = Memory::reader(&[b"ab".to_vec()], false);
        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
        assert_eq!(reader.seek(SeekFrom::Start(10)).unwrap(), 10);
        let mut buf = [0u8; 2];
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn a_part_cut_short_says_it_changed_and_gives_no_other_bytes() {
        let parts = vec![b"aaaa".to_vec(), b"bbbb".to_vec(), b"cccc".to_vec()];
        let (mut reader, _, data) = Memory::shared(&parts, false);
        // Cut short after the snapshot, before the part is opened.
        data.lock().unwrap()[1] = b"bb".to_vec();
        let mut all = Vec::new();
        let error = reader.read_to_end(&mut all).unwrap_err();
        assert_eq!(all, b"aaaabb", "the bytes before the cut, nothing else");
        let parts_error = PartsError::of(&error).unwrap();
        assert!(matches!(parts_error, PartsError::Changed { part: 1, .. }));
        assert_eq!(
            parts_error.to_string(),
            "part whole.bin.002 changed since it was listed"
        );
        assert_eq!(parts_error.clone().into_vfs(), VfsError::Changed);
    }

    /// Parts whose bytes are a function of their offset in the whole, of
    /// any size, with nothing stored: for offsets past 4 GiB.
    struct Synthetic {
        starts: Vec<(Place, u64, u64)>,
    }

    fn byte_at(offset: u64) -> u8 {
        (offset.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as u8
    }

    struct Pattern {
        start: u64,
        size: u64,
        at: u64,
    }

    impl Read for Pattern {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let left = self.size.saturating_sub(self.at);
            let count = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
            for (i, byte) in buf[..count].iter_mut().enumerate() {
                *byte = byte_at(self.start + self.at + i as u64);
            }
            self.at += count as u64;
            Ok(count)
        }
    }

    impl Seek for Pattern {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            if let SeekFrom::Start(offset) = pos {
                self.at = offset;
            }
            Ok(self.at)
        }
    }

    impl PartOpener for Synthetic {
        fn open(&self, part: &PartInfo) -> Result<Opened, VfsError> {
            let (_, start, size) = self
                .starts
                .iter()
                .find(|(place, _, _)| *place == part.place)
                .cloned()
                .ok_or(VfsError::NotFound)?;
            Ok(Opened::Seekable(Box::new(Pattern { start, size, at: 0 })))
        }
    }

    #[test]
    fn offsets_past_4_gib_are_counted_in_64_bits() {
        const GIB: u64 = 1 << 30;
        let sizes = [3 * GIB, 3 * GIB, 3 * GIB + 7];
        let mut starts = Vec::new();
        let mut at = 0;
        let mut infos = Vec::new();
        for (n, size) in sizes.iter().enumerate() {
            starts.push((place(n + 1), at, *size));
            infos.push(PartInfo::new(place(n + 1), *size));
            at += size;
        }
        let mut reader = PartsReader::with_opener(infos, Synthetic { starts }).unwrap();
        assert_eq!(reader.len(), 9 * GIB + 7);
        assert_eq!(reader.part_at(5 * GIB), Some(1));
        assert_eq!(reader.start_of(2), Some(6 * GIB));
        for offset in [4 * GIB + 5, 6 * GIB - 50, 9 * GIB - 93] {
            reader.seek(SeekFrom::Start(offset)).unwrap();
            let mut buf = [0u8; 100];
            reader.read_exact(&mut buf).unwrap();
            let expected: Vec<u8> = (0..100).map(|i| byte_at(offset + i)).collect();
            assert_eq!(buf.to_vec(), expected, "at {offset}");
        }
        reader.seek(SeekFrom::End(-3)).unwrap();
        assert_eq!(reader.position(), 9 * GIB + 4);
        let huge = vec![
            PartInfo::new(place(1), u64::MAX / 2 + 1),
            PartInfo::new(place(2), u64::MAX / 2 + 1),
        ];
        assert_eq!(
            PartsReader::with_opener(huge, Synthetic { starts: Vec::new() }).unwrap_err(),
            PartsError::TooLarge
        );
    }

    #[derive(Clone, Debug)]
    enum Op {
        Seek(SeekFrom),
        Read(usize),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0u64..80).prop_map(|n| Op::Seek(SeekFrom::Start(n))),
            (-40i64..40).prop_map(|n| Op::Seek(SeekFrom::Current(n))),
            (-80i64..5).prop_map(|n| Op::Seek(SeekFrom::End(n))),
            (0usize..30).prop_map(Op::Read),
        ]
    }

    fn parts_strategy() -> impl Strategy<Value = Vec<Vec<u8>>> {
        prop::collection::vec(prop::collection::vec(any::<u8>(), 1..12), 1..8)
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Any seeks and reads give what a cursor over the whole gives.
        #[test]
        fn random_seeks_and_reads_equal_a_cursor(
            parts in parts_strategy(),
            ops in prop::collection::vec(op(), 1..40),
            forward in any::<bool>(),
        ) {
            let whole = concat(&parts);
            let (mut reader, counter) = Memory::reader(&parts, forward);
            let mut cursor = Cursor::new(whole.clone());
            for op in ops {
                match op {
                    Op::Seek(pos) => {
                        let ours = reader.seek(pos).ok();
                        let theirs = cursor.seek(pos).ok();
                        prop_assert_eq!(ours, theirs);
                    }
                    Op::Read(n) => {
                        let mut a = vec![0u8; n];
                        let mut b = vec![0u8; n];
                        let mut got_a = 0;
                        while got_a < n {
                            let read = reader.read(&mut a[got_a..]).unwrap();
                            if read == 0 { break; }
                            got_a += read;
                        }
                        let got_b = cursor.read(&mut b).unwrap();
                        prop_assert_eq!(got_a, got_b);
                        prop_assert_eq!(&a[..got_a], &b[..got_b]);
                    }
                }
                prop_assert!(reader.open_handles() <= PartsReader::MAX_OPEN);
            }
            prop_assert!(counter.most.load(Ordering::SeqCst) <= PartsReader::MAX_OPEN);
        }
    }
}
