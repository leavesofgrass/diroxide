//! Where the viewer's bytes come from: a seekable read (a local file, a
//! spooled member), read in blocks with a small cache, or a forward-only
//! stream (a deflate member inside a zip), whose decoded bytes are kept up
//! to a cap and said beyond it ("Showing the first 64 MB of 800 MB.").
//! Nothing here reads a whole file: a 1 GiB log shows its first screen
//! after one block.

use std::io::{self, Read, Seek, SeekFrom};

use diroxide_vfs::ReadSeek;

/// The bytes one cached block holds.
pub(crate) const BLOCK: usize = 64 * 1024;

/// How many blocks a seekable source keeps: 2 MiB.
const CACHED_BLOCKS: usize = 32;

/// The bytes the viewer shows. Built by [`crate::viewer::Viewer::open`]
/// from a provider, or by hand for tests and command output.
pub struct ViewerSource {
    kind: Kind,
    /// A buffer for reads larger than a block, reused.
    scratch: Vec<u8>,
}

impl std::fmt::Debug for ViewerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("ViewerSource");
        match &self.kind {
            Kind::Seekable { len, .. } => s.field("seekable", len),
            Kind::Stream {
                bytes, cap, total, ..
            } => s
                .field("stream", &bytes.len())
                .field("cap", cap)
                .field("total", total),
        };
        s.finish_non_exhaustive()
    }
}

enum Kind {
    Seekable {
        reader: Box<dyn ReadSeek>,
        len: u64,
        /// Cached blocks by index, the most recently used last.
        blocks: Vec<(u64, Vec<u8>)>,
    },
    Stream {
        reader: Option<Box<dyn Read + Send>>,
        bytes: Vec<u8>,
        cap: u64,
        total: Option<u64>,
        /// A byte read past the cap to learn that more follows.
        pending: Option<u8>,
    },
}

impl ViewerSource {
    /// A source over a seekable read; its length is read once here and
    /// again by [`ViewerSource::refresh`].
    pub fn seekable(mut reader: Box<dyn ReadSeek>) -> io::Result<Self> {
        let len = reader.seek(SeekFrom::End(0))?;
        Ok(Self {
            kind: Kind::Seekable {
                reader,
                len,
                blocks: Vec::new(),
            },
            scratch: Vec::new(),
        })
    }

    /// A source over a forward-only read, keeping at most `cap` decoded
    /// bytes; `total` is the size the listing gave, when it gave one.
    pub fn stream(reader: Box<dyn Read + Send>, total: Option<u64>, cap: u64) -> Self {
        Self {
            kind: Kind::Stream {
                reader: Some(reader),
                bytes: Vec::new(),
                cap: cap.max(1),
                total,
                pending: None,
            },
            scratch: Vec::new(),
        }
    }

    /// A source over bytes already in memory: a command's output, a test.
    pub fn bytes(bytes: Vec<u8>) -> Self {
        let total = bytes.len() as u64;
        Self {
            kind: Kind::Stream {
                reader: None,
                bytes,
                cap: total.max(1),
                total: Some(total),
                pending: None,
            },
            scratch: Vec::new(),
        }
    }

    /// Whether the source reads with random access.
    pub fn is_seekable(&self) -> bool {
        matches!(self.kind, Kind::Seekable { .. })
    }

    /// The bytes the viewer can show: a seekable file's length, or the
    /// decoded bytes a stream has kept so far.
    pub fn available(&self) -> u64 {
        match &self.kind {
            Kind::Seekable { len, .. } => *len,
            Kind::Stream { bytes, .. } => bytes.len() as u64,
        }
    }

    /// The whole size, when known: a seekable file's length, or the size
    /// a stream's listing gave.
    pub fn total(&self) -> Option<u64> {
        match &self.kind {
            Kind::Seekable { len, .. } => Some(*len),
            // Read to its end, a stream knows; before, only its listing.
            Kind::Stream {
                reader: None,
                pending: None,
                bytes,
                ..
            } => Some(bytes.len() as u64),
            Kind::Stream { total, .. } => *total,
        }
    }

    /// Whether every byte of the source is in hand or can be read: a
    /// seekable file, or a stream read to its end.
    pub fn is_complete(&self) -> bool {
        match &self.kind {
            Kind::Seekable { .. } => true,
            Kind::Stream {
                reader, pending, ..
            } => reader.is_none() && pending.is_none(),
        }
    }

    /// The cap on a stream's decoded bytes, `None` for a seekable source.
    pub fn cap(&self) -> Option<u64> {
        match &self.kind {
            Kind::Seekable { .. } => None,
            Kind::Stream { cap, .. } => Some(*cap),
        }
    }

    /// Whether a stream stopped at its cap with more to come: the
    /// large-file policy's case.
    pub fn is_capped(&self) -> bool {
        match &self.kind {
            Kind::Seekable { .. } => false,
            Kind::Stream {
                bytes,
                cap,
                total,
                pending,
                reader,
            } => {
                bytes.len() as u64 >= *cap
                    && (pending.is_some()
                        || total.is_some_and(|t| t > *cap)
                        || (reader.is_some() && total.is_none()))
            }
        }
    }

    /// Whether the cap would be reached: a stream whose listed size is
    /// over its cap.
    pub fn will_cap(&self) -> bool {
        match &self.kind {
            Kind::Seekable { .. } => false,
            Kind::Stream { cap, total, .. } => total.is_some_and(|t| t > *cap),
        }
    }

    /// Raises a stream's cap by `more` bytes ("read more").
    pub fn raise_cap(&mut self, more: u64) {
        if let Kind::Stream { cap, .. } = &mut self.kind {
            *cap = cap.saturating_add(more);
        }
    }

    /// Reads the length again (follow, reload): the new length of a
    /// seekable file, with the cache dropped when it shrank.
    pub fn refresh(&mut self) -> io::Result<u64> {
        match &mut self.kind {
            Kind::Seekable {
                reader,
                len,
                blocks,
            } => {
                let now = reader.seek(SeekFrom::End(0))?;
                if now < *len {
                    blocks.clear();
                } else {
                    // The last block may have grown.
                    let last = len.saturating_sub(1) / BLOCK as u64;
                    blocks.retain(|(i, _)| *i < last);
                }
                *len = now;
                Ok(now)
            }
            Kind::Stream { bytes, .. } => Ok(bytes.len() as u64),
        }
    }

    /// Up to `want` bytes from `offset`: fewer at the end, none past it.
    pub fn chunk(&mut self, offset: u64, want: usize) -> io::Result<&[u8]> {
        match &mut self.kind {
            Kind::Seekable {
                reader,
                len,
                blocks,
            } => {
                if offset >= *len || want == 0 {
                    return Ok(&[]);
                }
                let want = want.min((*len - offset) as usize);
                let block = offset / BLOCK as u64;
                let within = (offset % BLOCK as u64) as usize;
                if within + want <= BLOCK {
                    let at = cached_block(reader.as_mut(), *len, blocks, block)?;
                    let data = &blocks[at].1;
                    let end = (within + want).min(data.len());
                    return Ok(&data[within.min(end)..end]);
                }
                // Larger than a block: read straight through, uncached.
                self.scratch.resize(want, 0);
                reader.seek(SeekFrom::Start(offset))?;
                let got = read_full(reader.as_mut(), &mut self.scratch)?;
                Ok(&self.scratch[..got])
            }
            Kind::Stream {
                reader,
                bytes,
                cap,
                pending,
                ..
            } => {
                let wanted_end = offset.saturating_add(want as u64).min(*cap);
                if (bytes.len() as u64) < wanted_end {
                    fill(reader, bytes, pending, wanted_end, *cap)?;
                }
                let start = (offset as usize).min(bytes.len());
                let end = (wanted_end as usize).min(bytes.len()).max(start);
                Ok(&bytes[start..end])
            }
        }
    }
}

/// The index in `blocks` of block `block`, read and cached if needed.
fn cached_block(
    reader: &mut dyn ReadSeek,
    len: u64,
    blocks: &mut Vec<(u64, Vec<u8>)>,
    block: u64,
) -> io::Result<usize> {
    if let Some(at) = blocks.iter().position(|(i, _)| *i == block) {
        let entry = blocks.remove(at);
        blocks.push(entry);
        return Ok(blocks.len() - 1);
    }
    let start = block * BLOCK as u64;
    let size = (len - start).min(BLOCK as u64) as usize;
    let mut data = vec![0u8; size];
    reader.seek(SeekFrom::Start(start))?;
    let got = read_full(reader, &mut data)?;
    data.truncate(got);
    if blocks.len() >= CACHED_BLOCKS {
        blocks.remove(0);
    }
    blocks.push((block, data));
    Ok(blocks.len() - 1)
}

/// Reads until the buffer is full or the reader ends; the bytes read.
fn read_full(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match reader.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

/// Reads a stream forward until `bytes` reaches `until` (at most `cap`)
/// or the stream ends; at the cap, one byte more says whether more
/// follows.
fn fill(
    reader: &mut Option<Box<dyn Read + Send>>,
    bytes: &mut Vec<u8>,
    pending: &mut Option<u8>,
    until: u64,
    cap: u64,
) -> io::Result<()> {
    if let Some(byte) = pending.take() {
        bytes.push(byte);
    }
    let Some(stream) = reader.as_mut() else {
        return Ok(());
    };
    let mut buf = vec![0u8; BLOCK];
    while (bytes.len() as u64) < until {
        let room = ((until - bytes.len() as u64) as usize).min(BLOCK);
        let n = match stream.read(&mut buf[..room]) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if n == 0 {
            *reader = None;
            return Ok(());
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    if bytes.len() as u64 >= cap {
        let mut one = [0u8; 1];
        loop {
            match stream.read(&mut one) {
                Ok(0) => {
                    *reader = None;
                    break;
                }
                Ok(_) => {
                    *pending = Some(one[0]);
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn text(n: usize) -> Vec<u8> {
        (0..n).map(|i| b'a' + (i % 26) as u8).collect()
    }

    #[test]
    fn a_seekable_source_reads_blocks_and_long_runs() {
        let bytes = text(BLOCK * 3 + 10);
        let mut source = ViewerSource::seekable(Box::new(Cursor::new(bytes.clone()))).unwrap();
        assert!(source.is_seekable());
        assert_eq!(source.available(), bytes.len() as u64);
        assert_eq!(source.chunk(5, 4).unwrap(), &bytes[5..9]);
        let across = source.chunk(BLOCK as u64 - 2, 4).unwrap().to_vec();
        assert_eq!(across, &bytes[BLOCK - 2..BLOCK + 2]);
        let long = source.chunk(10, BLOCK * 2).unwrap().to_vec();
        assert_eq!(long, &bytes[10..10 + BLOCK * 2]);
        let end = source.chunk(bytes.len() as u64 - 3, 100).unwrap().to_vec();
        assert_eq!(end, &bytes[bytes.len() - 3..]);
        assert!(source.chunk(bytes.len() as u64, 10).unwrap().is_empty());
        assert!(!source.is_capped());
    }

    #[test]
    fn a_stream_keeps_bytes_up_to_its_cap_and_says_more_follows() {
        let bytes = text(1000);
        let mut source = ViewerSource::stream(Box::new(Cursor::new(bytes.clone())), None, 600);
        assert!(!source.is_seekable());
        assert_eq!(source.chunk(0, 10).unwrap(), &bytes[..10]);
        assert_eq!(source.available(), 10);
        assert_eq!(
            source.chunk(500, 500).unwrap().len(),
            100,
            "stops at the cap"
        );
        assert!(source.is_capped());
        assert_eq!(source.total(), None, "no listed size, not read to the end");
        source.raise_cap(1000);
        assert_eq!(source.chunk(590, 20).unwrap(), &bytes[590..610]);
        assert_eq!(source.chunk(0, 5000).unwrap().len(), 1000);
        assert!(!source.is_capped());
        assert_eq!(source.total(), Some(1000));
        let listed = ViewerSource::stream(Box::new(Cursor::new(text(10))), Some(800), 600);
        assert!(listed.will_cap());
    }

    #[test]
    fn bytes_in_memory_are_a_finished_stream() {
        let mut source = ViewerSource::bytes(b"one\ntwo\n".to_vec());
        assert_eq!(source.total(), Some(8));
        assert_eq!(source.chunk(4, 100).unwrap(), b"two\n");
        assert!(!source.is_capped() && !source.will_cap());
    }
}
