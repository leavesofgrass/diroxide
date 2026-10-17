//! Spike P1 (research/alpha2-plan.md 4.3): the seekable read over split
//! parts on real files, under the target's temporary folder.
//!
//! - A 25 MB zip-shaped and a 25 MB 7z-shaped stream, split into 64 KiB
//!   parts with a last part of three bytes, read as one: every byte
//!   compared with the whole by SHA-256, the zip's end record found by a
//!   backward scan that crosses parts, the 7z's next header read from the
//!   offset its start header names.
//! - Ten thousand tiny parts read through a provider that counts its open
//!   readers: never more than two at once.
//! - A part cut short while it is open gives "part changed", never other
//!   bytes.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use diroxide_core::{Entry, Place};
use diroxide_ops::{PartsError, PartsReader};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, LocalProvider, OpContext, Provider, ReadSeek,
    VfsError, WriteMode,
};
use sha2::{Digest, Sha256};

const PART: usize = 64 * 1024;

fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

fn place_of(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

/// Bytes that do not repeat at a part's length.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// A zip-shaped stream: noise, then an end-of-central-directory record
/// (`PK 05 06`, 22 bytes, no comment) as the last bytes.
fn zip_shaped(len: usize) -> Vec<u8> {
    let mut bytes = noise(len - 22, 7);
    // Keep the noise free of the signature, so the scan finds the tail.
    for window in 0..bytes.len().saturating_sub(3) {
        if bytes[window..window + 4] == *b"PK\x05\x06" {
            bytes[window] = b'Q';
        }
    }
    let central_offset = (len as u32).saturating_sub(4000);
    bytes.extend_from_slice(b"PK\x05\x06");
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&3978u32.to_le_bytes());
    bytes.extend_from_slice(&central_offset.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    assert_eq!(bytes.len(), len);
    bytes
}

/// A 7z-shaped stream: the 32-byte start header (signature, version, the
/// next header's offset after the start header and its size), noise, and
/// the next header as the last bytes.
fn seven_shaped(len: usize, next_size: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, 0, 4]);
    bytes.extend_from_slice(&[0; 4]);
    let next_offset = (len - 32 - next_size) as u64;
    bytes.extend_from_slice(&next_offset.to_le_bytes());
    bytes.extend_from_slice(&(next_size as u64).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend(noise(len - 32 - next_size, 11));
    let mut header = vec![0x01, 0x04, 0x06];
    header.extend(noise(next_size - 3, 13));
    bytes.extend(header);
    assert_eq!(bytes.len(), len);
    bytes
}

/// Writes `whole` as `name.001` and on, `PART` bytes each, the last with
/// what is left; returns the parts' places in order.
fn split_to_files(folder: &Path, name: &str, whole: &[u8]) -> Vec<Place> {
    whole
        .chunks(PART)
        .enumerate()
        .map(|(n, chunk)| {
            let path = folder.join(format!("{name}.{:03}", n + 1));
            fs::write(&path, chunk).unwrap();
            place_of(&path)
        })
        .collect()
}

fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

/// The 25 MB length: 381 parts of 64 KiB and a last part of 3 bytes.
const LEN: usize = 381 * PART + 3;

#[test]
fn a_split_zip_reads_as_one_and_its_tail_is_found_across_parts() {
    let folder = temp();
    let whole = zip_shaped(LEN);
    let parts = split_to_files(folder.path(), "course.zip", &whole);
    assert_eq!(parts.len(), 382);
    assert_eq!(
        fs::metadata(folder.path().join("course.zip.382"))
            .unwrap()
            .len(),
        3
    );
    let provider: Arc<dyn Provider> = Arc::new(LocalProvider::new());
    let mut reader = PartsReader::open(provider, &parts, &OpContext::new()).unwrap();
    assert_eq!(reader.len(), LEN as u64);
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher).unwrap();
    assert_eq!(hasher.finalize().to_vec(), sha(&whole), "every byte");
    // The zip reader's way in: the end record, searched backward from the
    // end in blocks that cross the 3-byte last part.
    let mut found = None;
    let mut end = reader.len();
    while end > 0 && found.is_none() {
        let start = end.saturating_sub(1000);
        reader.seek(SeekFrom::Start(start)).unwrap();
        let mut block = vec![0u8; (end - start) as usize + 3];
        let read = reader.read(&mut block).unwrap();
        let mut got = read;
        while got < block.len() {
            let more = reader.read(&mut block[got..]).unwrap();
            if more == 0 {
                break;
            }
            got += more;
        }
        block.truncate(got);
        found = block
            .windows(4)
            .rposition(|w| w == b"PK\x05\x06")
            .map(|at| start + at as u64);
        end = start;
    }
    assert_eq!(found, Some(LEN as u64 - 22));
    reader.seek(SeekFrom::Start(LEN as u64 - 6)).unwrap();
    let mut offset = [0u8; 4];
    reader.read_exact(&mut offset).unwrap();
    assert_eq!(u32::from_le_bytes(offset), LEN as u32 - 4000);
    assert!(reader.open_handles() <= PartsReader::MAX_OPEN);
}

#[test]
fn a_split_7z_reads_its_next_header_from_the_last_parts() {
    let folder = temp();
    let whole = seven_shaped(LEN, 200);
    let parts = split_to_files(folder.path(), "backup.7z", &whole);
    let provider: Arc<dyn Provider> = Arc::new(LocalProvider::new());
    let mut reader = PartsReader::open(provider, &parts, &OpContext::new()).unwrap();
    let mut start = [0u8; 32];
    reader.read_exact(&mut start).unwrap();
    assert_eq!(&start[..6], &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]);
    let next_offset = u64::from_le_bytes(start[12..20].try_into().unwrap());
    let next_size = u64::from_le_bytes(start[20..28].try_into().unwrap());
    reader.seek(SeekFrom::Start(32 + next_offset)).unwrap();
    let mut header = vec![0u8; next_size as usize];
    reader.read_exact(&mut header).unwrap();
    assert_eq!(&header[..3], &[0x01, 0x04, 0x06]);
    assert_eq!(header, whole[LEN - 200..]);
    reader.seek(SeekFrom::Start(0)).unwrap();
    let mut all = Vec::with_capacity(LEN);
    reader.read_to_end(&mut all).unwrap();
    assert_eq!(sha(&all), sha(&whole), "every byte");
}

/// The local provider, counting the readers it has open.
#[derive(Debug, Default)]
struct Counting {
    inner: LocalProvider,
    live: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
}

struct Counted {
    file: Box<dyn ReadSeek>,
    live: Arc<AtomicUsize>,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for Counted {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Provider for Counting {
    fn scheme(&self) -> &str {
        self.inner.scheme()
    }
    fn capabilities(&self, place: &Place) -> Capabilities {
        self.inner.capabilities(place)
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.inner.list(place, ctx)
    }
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.inner.stat(place, ctx)
    }
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.inner.read(place, ctx)
    }
    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        let file = self.inner.open_seekable(place, ctx)?;
        let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        Ok(Box::new(Counted {
            file,
            live: Arc::clone(&self.live),
        }))
    }
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.inner.write(place, mode, ctx)
    }
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.create_dir(place, ctx)
    }
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.rename(from, to, ctx)
    }
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.remove(place, ctx)
    }
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.inner.set_attributes(place, change, ctx)
    }
}

#[test]
fn ten_thousand_parts_never_hold_more_than_two_handles() {
    let folder = temp();
    let mut whole = Vec::with_capacity(30_000);
    let mut parts = Vec::with_capacity(10_000);
    for n in 1..=10_000u32 {
        let bytes = [(n % 251) as u8, (n / 251) as u8, 0x5A];
        whole.extend_from_slice(&bytes);
        let path = folder.path().join(format!("many.bin.{n:03}"));
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        parts.push(place_of(&path));
    }
    let counting = Arc::new(Counting::default());
    let provider: Arc<dyn Provider> = Arc::clone(&counting) as Arc<dyn Provider>;
    let mut reader = PartsReader::open(provider, &parts, &OpContext::new()).unwrap();
    let mut all = Vec::new();
    reader.read_to_end(&mut all).unwrap();
    assert_eq!(all, whole);
    // Back and forth between the first and the last parts, and about.
    for offset in [0u64, 29_997, 3, 29_990, 15_000, 1, 29_999] {
        reader.seek(SeekFrom::Start(offset)).unwrap();
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], whole[offset as usize]);
    }
    assert!(counting.most.load(Ordering::SeqCst) <= PartsReader::MAX_OPEN);
    drop(reader);
    assert_eq!(counting.live.load(Ordering::SeqCst), 0, "all closed");
}

#[test]
fn a_part_cut_short_while_open_says_it_changed() {
    let folder = temp();
    let whole = noise(3 * 1000, 3);
    let parts: Vec<Place> = whole
        .chunks(1000)
        .enumerate()
        .map(|(n, chunk)| {
            let path = folder.path().join(format!("report.pdf.{:03}", n + 1));
            fs::write(&path, chunk).unwrap();
            place_of(&path)
        })
        .collect();
    let provider: Arc<dyn Provider> = Arc::new(LocalProvider::new());
    let mut reader = PartsReader::open(Arc::clone(&provider), &parts, &OpContext::new()).unwrap();
    let mut first = vec![0u8; 1500];
    reader.read_exact(&mut first).unwrap();
    assert_eq!(first, whole[..1500], "the second part is open, half read");
    let cut = fs::OpenOptions::new()
        .write(true)
        .open(folder.path().join("report.pdf.002"))
        .unwrap();
    cut.set_len(600).unwrap();
    drop(cut);
    let mut rest = Vec::new();
    let error = reader.read_to_end(&mut rest).unwrap_err();
    assert!(
        rest.is_empty() || whole[1500..1500 + rest.len()] == rest[..],
        "no other bytes"
    );
    assert!(
        matches!(
            PartsError::of(&error),
            Some(PartsError::Changed { part: 1, .. })
        ),
        "{error}"
    );
    assert!(error.to_string().starts_with("part report.pdf.002 changed"));
    // A new reader over the same snapshot sees the change before it reads.
    let snapshot = reader.parts().to_vec();
    let mut again = PartsReader::with_parts(provider, snapshot, &OpContext::new()).unwrap();
    again.seek(SeekFrom::Start(1200)).unwrap();
    let error = again.read(&mut [0u8; 10]).unwrap_err();
    assert!(matches!(
        PartsError::of(&error),
        Some(PartsError::Changed { part: 1, .. })
    ));
}
