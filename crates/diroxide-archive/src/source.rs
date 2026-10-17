//! A container's bytes, shared: one handle under a lock, any number of
//! views with their own positions and their own small buffers, cheap to
//! clone. The zip crate wants a reader it can clone for each member read;
//! a stored member of an outer archive is a window of the outer bytes (the
//! sub-range shortcut of research/archives.md 5.4).

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use diroxide_vfs::ReadSeek;

/// How much one view reads ahead.
const BUFFER: usize = 64 << 10;

struct Shared {
    handle: Mutex<Box<dyn ReadSeek>>,
}

/// A view of a container's bytes: [`Read`] and [`Seek`] within its window,
/// [`Clone`] for another view with its own position.
pub struct Source {
    shared: Arc<Shared>,
    start: u64,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    buf_at: u64,
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("start", &self.start)
            .field("len", &self.len)
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

impl Clone for Source {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            start: self.start,
            len: self.len,
            pos: self.pos,
            buf: Vec::new(),
            buf_at: 0,
        }
    }
}

impl Source {
    /// A view of the whole of a reader, which is measured once.
    pub fn new(mut reader: Box<dyn ReadSeek>) -> io::Result<Self> {
        let len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self {
            shared: Arc::new(Shared {
                handle: Mutex::new(reader),
            }),
            start: 0,
            len,
            pos: 0,
            buf: Vec::new(),
            buf_at: 0,
        })
    }

    /// A view of bytes held in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let len = bytes.len() as u64;
        Self {
            shared: Arc::new(Shared {
                handle: Mutex::new(Box::new(io::Cursor::new(bytes))),
            }),
            start: 0,
            len,
            pos: 0,
            buf: Vec::new(),
            buf_at: 0,
        }
    }

    /// A window of this view, `len` bytes from `offset`, cut to what
    /// exists.
    pub fn window(&self, offset: u64, len: u64) -> Self {
        let offset = offset.min(self.len);
        let len = len.min(self.len - offset);
        Self {
            shared: self.shared.clone(),
            start: self.start + offset,
            len,
            pos: 0,
            buf: Vec::new(),
            buf_at: 0,
        }
    }

    /// The bytes in this view.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Reads at an offset of the view, without moving the position: as
    /// many bytes as fit, fewer at the end.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        read_shared(&self.shared, self.start, self.len, offset, buf)
    }

    /// Reads exactly `buf.len()` bytes at an offset, or fails as damaged
    /// data does: the bytes end early.
    pub fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if self.read_at(offset, buf)? == buf.len() {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::UnexpectedEof))
        }
    }
}

/// Reads at `offset` of a window of the shared handle.
fn read_shared(
    shared: &Shared,
    start: u64,
    len: u64,
    offset: u64,
    buf: &mut [u8],
) -> io::Result<usize> {
    if offset >= len {
        return Ok(0);
    }
    let room = usize::try_from(len - offset).unwrap_or(usize::MAX);
    let want = buf.len().min(room);
    let mut handle = shared
        .handle
        .lock()
        .map_err(|_| io::Error::other("the archive's reader failed"))?;
    handle.seek(SeekFrom::Start(start + offset))?;
    let mut done = 0;
    while done < want {
        let n = handle.read(&mut buf[done..want])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Ok(done)
}

impl Read for Source {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let in_buffer = self.pos >= self.buf_at && self.pos < self.buf_at + self.buf.len() as u64;
        if !in_buffer {
            if out.len() >= BUFFER {
                let n = read_shared(&self.shared, self.start, self.len, self.pos, out)?;
                self.pos += n as u64;
                return Ok(n);
            }
            let mut buf = std::mem::take(&mut self.buf);
            buf.resize(BUFFER, 0);
            let n = read_shared(&self.shared, self.start, self.len, self.pos, &mut buf)?;
            buf.truncate(n);
            self.buf = buf;
            self.buf_at = self.pos;
            if self.buf.is_empty() {
                return Ok(0);
            }
        }
        let skip = (self.pos - self.buf_at) as usize;
        let available = &self.buf[skip..];
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Source {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
        };
        let target = target.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        self.pos = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_read_and_seek_on_their_own() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
        let mut a = Source::from_bytes(bytes.clone());
        let mut b = a.clone();
        let mut first = [0u8; 10];
        a.read_exact(&mut first).unwrap();
        assert_eq!(first, bytes[..10]);
        b.seek(SeekFrom::Start(150_000)).unwrap();
        let mut rest = Vec::new();
        b.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, bytes[150_000..]);
        let mut all = Vec::new();
        a.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes[10..]);
        let window = a.window(100, 50);
        let mut w = Vec::new();
        window.clone().read_to_end(&mut w).unwrap();
        assert_eq!(w, bytes[100..150]);
        let mut at = [0u8; 4];
        assert_eq!(window.read_at(48, &mut at).unwrap(), 2);
        assert!(window.read_exact_at(48, &mut at).is_err());
        let mut end = a.clone();
        assert_eq!(end.seek(SeekFrom::End(-1)).unwrap(), 199_999);
        assert!(end.seek(SeekFrom::Current(-300_000)).is_err());
    }
}
