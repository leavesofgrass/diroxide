//! One seekable source over every backing an image's bytes come from
//! (R29 1.9, R30 commit 86).
//!
//! - **[`Backing`]** is a positional read: `read_at(offset, buffer)`, no
//!   seek state, so one image serves many requests at once and every
//!   reader crate gets a [`Source`] of its own over the same bytes.
//! - **The backings:** the host's ranged `fs/read` ([`crate::host::HostFile`],
//!   an image on a local drive, inside an archive or on SFTP); a file in
//!   the host's spool ([`SpoolFile`], what 7-Zip or `qemu-img` made); a
//!   window into another backing ([`Slice`], a partition, or an image
//!   inside an image); a virtual disk a container crate decodes
//!   ([`ReaderBacking`]); and bytes in memory ([`Memory`], the tests and
//!   the fuzz targets). The spool and the host's ranged read are the only
//!   ways the plugin reaches an image's bytes.
//! - **The block cache** ([`Cached`]): 16 blocks of 64 KiB, least recently
//!   used out first. Tables, directories and inodes are small and
//!   scattered, so a listing costs a handful of host reads (spike DS1 in
//!   G5a: about 320 KiB in about 5 reads for every container). A read of
//!   four blocks or more goes past the cache, so copying a file out does
//!   not push the tables out.
//! - **[`Counted`]** counts reads and bytes, for the tests that prove a
//!   listing reads kilobytes, not the image.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// A cache block: 64 KiB.
pub const BLOCK: u64 = 64 * 1024;

/// Blocks the cache keeps.
pub const BLOCKS: usize = 16;

/// Bytes an image is read from, by position.
pub trait Backing: Send + Sync {
    /// The length in bytes.
    fn len(&self) -> u64;

    /// Whether it holds no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads into `buffer` from `offset`: as much as there is, fewer bytes
    /// only at the end, none at or past the end.
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize>;
}

/// Reads exactly `buffer.len()` bytes at `offset`, or fails with
/// `UnexpectedEof`.
pub fn read_exact_at(backing: &dyn Backing, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    let mut done = 0;
    while done < buffer.len() {
        let n = backing.read_at(offset + done as u64, &mut buffer[done..])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the image ends early",
            ));
        }
        done += n;
    }
    Ok(())
}

/// Reads up to `length` bytes at `offset` into a new buffer, fewer at the
/// end; never more than the backing holds, whatever `length` claims.
pub fn read_vec(backing: &dyn Backing, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    let left = backing.len().saturating_sub(offset);
    let length = length.min(usize::try_from(left).unwrap_or(usize::MAX));
    let mut buffer = vec![0; length];
    let mut done = 0;
    while done < length {
        let n = backing.read_at(offset + done as u64, &mut buffer[done..])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    buffer.truncate(done);
    Ok(buffer)
}

/// Bytes in memory.
#[derive(Clone, Debug, Default)]
pub struct Memory(Arc<Vec<u8>>);

impl Memory {
    /// A backing over these bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Arc::new(bytes))
    }
}

impl Backing for Memory {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let Ok(start) = usize::try_from(offset) else {
            return Ok(0);
        };
        let Some(rest) = self.0.get(start..) else {
            return Ok(0);
        };
        let n = rest.len().min(buffer.len());
        buffer[..n].copy_from_slice(&rest[..n]);
        Ok(n)
    }
}

/// A window into another backing: a partition, or an image inside an
/// image. Reads past its end read nothing, whatever the parent holds.
#[derive(Clone)]
pub struct Slice {
    inner: Arc<dyn Backing>,
    start: u64,
    len: u64,
}

impl Slice {
    /// The `len` bytes of `inner` from `start`, cut to what `inner` holds.
    pub fn new(inner: Arc<dyn Backing>, start: u64, len: u64) -> Self {
        let held = inner.len().saturating_sub(start);
        Self {
            inner,
            start,
            len: len.min(held),
        }
    }
}

impl Backing for Slice {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let n = buffer.len().min(room);
        self.inner.read_at(self.start + offset, &mut buffer[..n])
    }
}

/// A file in the host's spool folder, which 7-Zip or `qemu-img` wrote.
pub struct SpoolFile {
    file: Mutex<File>,
    len: u64,
}

impl SpoolFile {
    /// The file, opened to read.
    pub fn open(file: File) -> io::Result<Self> {
        let len = file.metadata()?.len();
        Ok(Self {
            file: Mutex::new(file),
            len,
        })
    }
}

impl Backing for SpoolFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        file.seek(SeekFrom::Start(offset))?;
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let n = buffer.len().min(room);
        let mut done = 0;
        while done < n {
            match file.read(&mut buffer[done..n])? {
                0 => break,
                got => done += got,
            }
        }
        Ok(done)
    }
}

/// A reader a container crate made (a VHDX's or a QCOW2's virtual disk),
/// as a backing: one at a time, by seeking.
pub struct ReaderBacking<R> {
    reader: Mutex<R>,
    len: u64,
}

impl<R: Read + Seek + Send> ReaderBacking<R> {
    /// The reader, whose virtual size is `len`.
    pub fn new(reader: R, len: u64) -> Self {
        Self {
            reader: Mutex::new(reader),
            len,
        }
    }
}

impl<R: Read + Seek + Send> Backing for ReaderBacking<R> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let n = buffer.len().min(room);
        let mut reader = self.reader.lock().unwrap_or_else(PoisonError::into_inner);
        reader.seek(SeekFrom::Start(offset))?;
        let mut done = 0;
        while done < n {
            match reader.read(&mut buffer[done..n])? {
                0 => break,
                got => done += got,
            }
        }
        Ok(done)
    }
}

/// Counts the reads and bytes that reach a backing.
pub struct Counted {
    inner: Arc<dyn Backing>,
    reads: AtomicU64,
    bytes: AtomicU64,
}

impl Counted {
    /// A counter over `inner`.
    pub fn new(inner: Arc<dyn Backing>) -> Self {
        Self {
            inner,
            reads: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    /// Reads so far.
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }

    /// Bytes read so far.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::SeqCst)
    }
}

impl Backing for Counted {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read_at(offset, buffer)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.bytes.fetch_add(n as u64, Ordering::SeqCst);
        Ok(n)
    }
}

/// The block cache: [`BLOCKS`] blocks of [`BLOCK`] bytes, least recently
/// used out first.
pub struct Cached {
    inner: Arc<dyn Backing>,
    blocks: Mutex<Vec<(u64, Arc<Vec<u8>>)>>,
}

impl Cached {
    /// A cache over `inner`.
    pub fn new(inner: Arc<dyn Backing>) -> Self {
        Self {
            inner,
            blocks: Mutex::new(Vec::with_capacity(BLOCKS)),
        }
    }

    fn block(&self, index: u64) -> io::Result<Arc<Vec<u8>>> {
        let mut blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = blocks.iter().position(|(i, _)| *i == index) {
            let hit = blocks.remove(at);
            let bytes = Arc::clone(&hit.1);
            blocks.push(hit);
            return Ok(bytes);
        }
        let bytes = Arc::new(read_vec(
            self.inner.as_ref(),
            index * BLOCK,
            BLOCK as usize,
        )?);
        if blocks.len() >= BLOCKS {
            blocks.remove(0);
        }
        blocks.push((index, Arc::clone(&bytes)));
        Ok(bytes)
    }
}

impl Backing for Cached {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len() {
            return Ok(0);
        }
        if buffer.len() as u64 >= 4 * BLOCK {
            return self.inner.read_at(offset, buffer);
        }
        let mut done = 0;
        while done < buffer.len() {
            let at = offset + done as u64;
            let block = self.block(at / BLOCK)?;
            let within = (at % BLOCK) as usize;
            let Some(rest) = block.get(within..) else {
                break;
            };
            if rest.is_empty() {
                break;
            }
            let n = rest.len().min(buffer.len() - done);
            buffer[done..done + n].copy_from_slice(&rest[..n]);
            done += n;
        }
        Ok(done)
    }
}

/// A seekable reader over a backing, for the reader crates: each has its
/// own position over the same bytes.
#[derive(Clone)]
pub struct Source {
    backing: Arc<dyn Backing>,
    at: u64,
}

impl Source {
    /// A reader at the start of `backing`.
    pub fn new(backing: Arc<dyn Backing>) -> Self {
        Self { backing, at: 0 }
    }

    /// The backing.
    pub fn backing(&self) -> &Arc<dyn Backing> {
        &self.backing
    }

    /// The length in bytes.
    pub fn len(&self) -> u64 {
        self.backing.len()
    }

    /// Whether it holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.backing.is_empty()
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.backing.read_at(self.at, buffer)?;
        self.at += n as u64;
        Ok(n)
    }
}

impl Seek for Source {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(d) => self.at.checked_add_signed(d),
            SeekFrom::End(d) => self.backing.len().checked_add_signed(d),
        };
        let target = target.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a seek before the start")
        })?;
        self.at = target;
        Ok(target)
    }
}

/// A source for a `fatfs` volume, which asks for `Write` too: every write
/// is refused, so the image is never changed.
#[derive(Clone)]
pub struct ReadOnly(pub Source);

impl Read for ReadOnly {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Seek for ReadOnly {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.0.seek(to)
    }
}

impl io::Write for ReadOnly {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "disk images are read only here",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_source_reads_and_seeks_like_a_file() {
        let data = bytes(10_000);
        let mut source = Source::new(Arc::new(Memory::new(data.clone())));
        let mut head = [0u8; 10];
        source.read_exact(&mut head).unwrap();
        assert_eq!(&head, &data[..10]);
        source.seek(SeekFrom::End(-5)).unwrap();
        let mut tail = Vec::new();
        source.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &data[9_995..]);
        assert!(source.seek(SeekFrom::Current(-20_000)).is_err());
        source.seek(SeekFrom::Start(20_000)).unwrap();
        assert_eq!(source.read(&mut head).unwrap(), 0);
    }

    #[test]
    fn a_slice_never_reads_past_its_end() {
        let data = bytes(1_000);
        let slice = Slice::new(Arc::new(Memory::new(data.clone())), 100, 50);
        assert_eq!(slice.len(), 50);
        let got = read_vec(&slice, 40, 100).unwrap();
        assert_eq!(got, &data[140..150]);
        let claimed = Slice::new(Arc::new(Memory::new(data)), 900, 10_000);
        assert_eq!(claimed.len(), 100);
    }

    #[test]
    fn the_cache_reads_a_block_once_and_large_reads_go_past_it() {
        let counted = Arc::new(Counted::new(Arc::new(Memory::new(bytes(
            40 * BLOCK as usize,
        )))));
        let cached = Cached::new(Arc::clone(&counted) as Arc<dyn Backing>);
        let mut small = [0u8; 512];
        for _ in 0..10 {
            cached.read_at(1_024, &mut small).unwrap();
        }
        assert_eq!(counted.reads(), 1);
        assert_eq!(counted.bytes(), BLOCK);
        // A read across a block edge takes both blocks.
        cached.read_at(BLOCK - 100, &mut small).unwrap();
        assert_eq!(counted.reads(), 2);
        // Seventeen other blocks push the first out.
        for i in 2..19 {
            cached.read_at(i * BLOCK, &mut small).unwrap();
        }
        let before = counted.reads();
        cached.read_at(0, &mut small).unwrap();
        assert_eq!(counted.reads(), before + 1);
        let mut large = vec![0u8; 4 * BLOCK as usize];
        cached.read_at(0, &mut large).unwrap();
        assert_eq!(counted.reads(), before + 2);
        assert_eq!(large[1], 1);
    }

    #[test]
    fn a_read_vec_never_allocates_by_a_claimed_length() {
        let memory = Memory::new(bytes(100));
        let got = read_vec(&memory, 50, usize::MAX).unwrap();
        assert_eq!(got.len(), 50);
        assert!(read_vec(&memory, 500, 10).unwrap().is_empty());
    }

    #[test]
    fn the_read_only_source_refuses_every_write() {
        use std::io::Write;
        let mut source = ReadOnly(Source::new(Arc::new(Memory::new(bytes(10)))));
        let error = source.write(b"x").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
