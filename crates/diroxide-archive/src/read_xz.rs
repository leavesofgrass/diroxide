//! xz reading on `lzma-rust2` (research/archives.md 2 and 3): a tar.xz
//! listed through the tar walker, and a plain `.xz` shown as an archive of
//! one member whose size comes from the stream's own index at its end.
//!
//! Spike 4.1 measured the risk: `XzReader` without a limit took 4,012 MiB
//! for a block that asks for a 4 GiB dictionary. So every decoder here is
//! made with a memory limit (the smaller of the dictionary and memory
//! limits), and the first block's dictionary is read from its header
//! before anything is decoded, so a dictionary over the hard limit is
//! refused in words and one over the soft limit asks first.
//!
//! The xz project's own test files (`fixtures/archives/xz/`) showed that
//! `lzma-rust2` 0.21's reader takes some broken files whole: a wrong
//! backward size, an index that disagrees with its blocks, a size written
//! longer than it needs, a zero or impossible compressed size, sizes that
//! add up past the format's limit. So before decoding, the provider walks
//! the file itself: every stream's footer, index and header, and each
//! block's header and LZMA2 chunk headers against its index record
//! ([`verify`]). It reads headers only, never the compressed data, within
//! a budget of reads; past the budget the decoder's own checks remain.
//! The same walk says in words what an unsupported file uses: a check
//! type, a filter or a header field the format does not define.

use std::io::{self, Read};

use diroxide_core::format::grouped;
use diroxide_vfs::VfsError;

use crate::error::ArchiveError;
use crate::guard::{Allowances, carries, check_dictionary, damaged, to_io};
use crate::limits::{Limits, limit_words};
use crate::source::Source;

/// The xz stream header's magic.
const MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];

/// The footer's magic.
const FOOTER_MAGIC: [u8; 2] = [b'Y', b'Z'];

/// The most streams an index walk follows back from the end.
const STREAMS_MAX: usize = 1_000;

/// The largest number the format's variable-length integers hold.
const VLI_MAX: u64 = u64::MAX >> 1;

/// The largest unpadded size of a block: the whole block stays under
/// 2^63 bytes.
const UNPADDED_MAX: u64 = VLI_MAX & !3;

/// The smallest unpadded size of a block: a header and one byte of data.
const UNPADDED_MIN: u64 = 5;

/// How many reads the block walk makes in one file (a block header or an
/// LZMA2 chunk header each); past it, the decoder's own checks remain.
/// At about 64 KiB of compressed data per chunk, a little over 1 GiB is
/// walked.
const BLOCK_READS_MAX: usize = 20_000;

/// The filters the decoder reads: delta, the eight branch converters
/// (x86, PowerPC, IA-64, ARM, ARM-Thumb, SPARC, ARM64, RISC-V) and LZMA2.
const FILTERS_KNOWN: [u64; 10] = [0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x21];

/// The LZMA2 filter's id.
const LZMA2: u64 = 0x21;

/// The cause when the index and the blocks disagree.
const MISMATCH: &str = "its index does not match its blocks";

/// The cause when a block header breaks the format's rules.
const BAD_BLOCK_HEADER: &str = "a block header is damaged";

/// The cause when a stream's index breaks the format's rules.
const BAD_INDEX: &str = "its index is damaged";

/// The decoder's memory limit for these limits, in KiB: the smaller of
/// the dictionary and the memory limits.
pub(crate) fn mem_limit_kb(limits: &Limits) -> u32 {
    let bytes = limits.max_dictionary.min(limits.max_memory_bytes);
    u32::try_from(bytes / 1024).unwrap_or(u32::MAX)
}

/// An xz variable-length number from a slice, with how many bytes it took.
fn number(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (i, &b) in bytes.iter().enumerate().take(9) {
        value |= u64::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// An xz variable-length number as the format allows it: at most nine
/// bytes, and never longer than it needs to be (no final zero byte after
/// the first).
fn strict_number(bytes: &[u8]) -> Option<(u64, usize)> {
    let (value, used) = number(bytes)?;
    if used > 1 && bytes[used - 1] == 0 {
        return None;
    }
    Some((value, used))
}

/// The dictionary of an LZMA2 filter's property byte (40 is the largest,
/// 4 GiB less one), or `None` for a byte the format does not allow.
pub fn lzma2_dictionary(code: u8) -> Option<u64> {
    match code {
        0..=39 => Some(u64::from(2 | (code & 1)) << (code / 2 + 11)),
        40 => Some(u64::from(u32::MAX)),
        _ => None,
    }
}

/// The dictionary the first block of the stream at `at` asks for, read
/// from its header; `None` when the stream has no block or no LZMA2 filter
/// is found (the decoder's own limit still holds).
pub(crate) fn first_dictionary(source: &Source, at: u64) -> Option<u64> {
    let mut head = [0u8; 12 + 1024];
    let got = source.read_at(at, &mut head).ok()?;
    let head = &head[..got];
    if head.get(..6) != Some(&MAGIC[..]) {
        return None;
    }
    let block = head.get(12..)?;
    let size_byte = *block.first()?;
    if size_byte == 0 {
        return None;
    }
    let header_size = (usize::from(size_byte) + 1) * 4;
    let header = block.get(..header_size)?;
    let flags = *header.get(1)?;
    let filters = usize::from(flags & 0x03) + 1;
    let mut at = 2;
    if flags & 0x40 != 0 {
        at += number(header.get(at..)?)?.1;
    }
    if flags & 0x80 != 0 {
        at += number(header.get(at..)?)?.1;
    }
    for _ in 0..filters {
        let (id, used) = number(header.get(at..)?)?;
        at += used;
        let (size, used) = number(header.get(at..)?)?;
        at += used;
        let props = header.get(at..at + usize::try_from(size).ok()?)?;
        at += props.len();
        if id == 0x21 {
            return lzma2_dictionary(*props.first()?);
        }
    }
    None
}

/// The checks before an xz stream is decoded: its first block's
/// dictionary against the limits, the file's end, and the walk of its
/// streams and blocks ([`verify`]). Gives the uncompressed size the
/// indexes add up to, or `None` when the walk could not follow every
/// stream.
pub(crate) fn check(
    source: &Source,
    name: &str,
    limits: &Limits,
    allow: Allowances,
) -> Result<Option<u64>, ArchiveError> {
    if let Some(dictionary) = first_dictionary(source, 0) {
        check_dictionary(name, dictionary, limits, allow)?;
    }
    check_end(source)?;
    Ok(verify(source, limits)?)
}

/// The most stream padding read back from the end.
const PADDING_MAX: u64 = 16 << 20;

/// The end of an xz file: after its last stream footer, only stream
/// padding, zero bytes in a multiple of four (the format's rule, which
/// `lzma-rust2` 0.21 does not hold a file to: it accepts three).
pub(crate) fn check_end(source: &Source) -> Result<(), VfsError> {
    let mut end = source.len();
    let mut zeros = 0u64;
    let mut chunk = vec![0u8; 4096];
    'back: while end > 0 && zeros <= PADDING_MAX {
        let take = end.min(chunk.len() as u64);
        let part = &mut chunk[..take as usize];
        source
            .read_exact_at(end - take, part)
            .map_err(VfsError::from)?;
        for (i, &b) in part.iter().enumerate().rev() {
            if b != 0 {
                end -= take - i as u64 - 1;
                break 'back;
            }
            zeros += 1;
        }
        end -= take;
    }
    if zeros > PADDING_MAX {
        return Err(damaged("its stream padding is too long"));
    }
    if !zeros.is_multiple_of(4) {
        return Err(damaged(
            "its stream padding is not a multiple of four bytes",
        ));
    }
    let mut footer = [0u8; 2];
    if end < 2 || source.read_exact_at(end - 2, &mut footer).is_err() || footer != FOOTER_MAGIC {
        return Err(damaged(
            "it ends without a stream footer: it is cut short or has data after its end",
        ));
    }
    Ok(())
}

/// The uncompressed size of a whole `.xz` file, from the index of each of
/// its streams, walked back from the end over stream padding; `None` when
/// the indexes cannot be read (a cut-off file) or break the format's
/// rules, so the size is said to be unknown rather than guessed.
pub(crate) fn uncompressed_size(source: &Source, limits: &Limits) -> Option<u64> {
    let streams = streams(source, limits).ok()??;
    total_of(&streams).ok()
}

/// Walks an xz file before it is decoded: every stream's footer, index
/// and header back from the end, then each block's header and LZMA2
/// chunk headers against its index record, within [`BLOCK_READS_MAX`]
/// reads. Damage is refused as "it is damaged: CAUSE", a feature the
/// format does not define or this version does not read as "it uses
/// WHAT, which is not supported". Gives the uncompressed size, or `None`
/// when there are more streams than the walk follows.
pub(crate) fn verify(source: &Source, limits: &Limits) -> Result<Option<u64>, VfsError> {
    let Some(streams) = streams(source, limits)? else {
        return Ok(None);
    };
    let total = total_of(&streams)?;
    let mut reads = 0;
    for stream in &streams {
        let check_size = check_size(stream.check);
        let mut at = stream.start + 12;
        for &(unpadded, uncompressed) in &stream.records {
            let record = Record {
                unpadded,
                uncompressed,
                check_size,
            };
            if !check_block(source, at, record, &mut reads)? {
                return Ok(Some(total));
            }
            at += unpadded.div_ceil(4) * 4;
        }
    }
    Ok(Some(total))
}

/// One stream of an xz file, as its footer, index and header describe it.
struct Stream {
    /// Where its stream header starts.
    start: u64,
    /// Its check type.
    check: u8,
    /// Each block's unpadded and uncompressed sizes, in order.
    records: Vec<(u64, u64)>,
}

/// The uncompressed size of every stream together, which the format
/// holds under 2^63.
fn total_of(streams: &[Stream]) -> Result<u64, VfsError> {
    streams
        .iter()
        .flat_map(|stream| stream.records.iter().map(|&(_, size)| size))
        .try_fold(0u64, |total, size| {
            total.checked_add(size).filter(|&total| total <= VLI_MAX)
        })
        .ok_or_else(|| damaged("its sizes add up to more than the format allows"))
}

/// The streams of an xz file, first to last, walked back from the end
/// over stream padding: each footer, index and header checked as the
/// format says. `None` when there are more than [`STREAMS_MAX`].
fn streams(source: &Source, limits: &Limits) -> Result<Option<Vec<Stream>>, VfsError> {
    let footer_damaged = || damaged("its stream footer is damaged");
    let mut end = source.len();
    let mut found = Vec::new();
    let mut records_seen = 0u64;
    for _ in 0..STREAMS_MAX {
        // Stream padding: zero bytes in multiples of four.
        while end >= 4 {
            let mut four = [0u8; 4];
            source.read_exact_at(end - 4, &mut four)?;
            if four != [0; 4] {
                break;
            }
            end -= 4;
        }
        if end == 0 {
            if found.is_empty() {
                return Err(damaged("it holds no xz stream"));
            }
            found.reverse();
            return Ok(Some(found));
        }
        let footer_at = end.checked_sub(12).ok_or_else(footer_damaged)?;
        let mut footer = [0u8; 12];
        source.read_exact_at(footer_at, &mut footer)?;
        if footer[10..12] != FOOTER_MAGIC || crc(&footer[4..10]) != le32(&footer[..4]) {
            return Err(footer_damaged());
        }
        let flags = [footer[8], footer[9]];
        let index_size = (u64::from(le32(&footer[4..8])) + 1) * 4;
        if index_size > limits.max_header_bytes {
            return Err(VfsError::LimitReached {
                what: "header size".to_owned(),
                limit: limit_words(limits.max_header_bytes),
            });
        }
        let index_at = footer_at
            .checked_sub(index_size)
            .filter(|&at| at >= 12)
            .ok_or_else(footer_damaged)?;
        let mut index = vec![0u8; usize::try_from(index_size).map_err(|_| footer_damaged())?];
        source.read_exact_at(index_at, &mut index)?;
        let (records, padded) = parse_index(&index).ok_or_else(|| damaged(BAD_INDEX))?;
        records_seen = records_seen.saturating_add(records.len() as u64);
        if records_seen > limits.max_entries as u64 {
            return Err(VfsError::LimitReached {
                what: "entries".to_owned(),
                limit: grouped(limits.max_entries as u64),
            });
        }
        let start = index_at
            .checked_sub(padded)
            .and_then(|at| at.checked_sub(12))
            .ok_or_else(|| damaged(MISMATCH))?;
        let mut header = [0u8; 12];
        source.read_exact_at(start, &mut header)?;
        if header[..6] != MAGIC {
            return Err(damaged(MISMATCH));
        }
        if crc(&header[6..8]) != le32(&header[8..12]) {
            return Err(damaged("its stream header is damaged"));
        }
        if header[6..8] != flags {
            return Err(damaged("its stream header and footer disagree"));
        }
        // Only flags both ends agree on can say what is unsupported.
        let check = check_of(flags)?;
        found.push(Stream {
            start,
            check,
            records,
        });
        end = start;
    }
    Ok(None)
}

/// A stream's check type from its flags; a check or a flag the format
/// does not define is refused as unsupported, in words.
fn check_of(flags: [u8; 2]) -> Result<u8, VfsError> {
    if flags[0] != 0 || flags[1] & 0xF0 != 0 {
        return Err(unsupported("xz stream flags newer than this reader"));
    }
    match flags[1] {
        id @ (0x00 | 0x01 | 0x04 | 0x0A) => Ok(id),
        id => Err(unsupported(&format!("xz check type {id}"))),
    }
}

/// The size of a check of this type, in bytes, as the format reserves it.
fn check_size(id: u8) -> u64 {
    match id {
        0 => 0,
        1..=3 => 4,
        4..=6 => 8,
        7..=9 => 16,
        10..=12 => 32,
        _ => 64,
    }
}

/// An index whole: its indicator, its records (unpadded and uncompressed
/// sizes, each in range and written as short as it can be), zero padding,
/// and its CRC-32. Gives the records and the blocks' padded sizes added
/// up; `None` for any break of the rules.
fn parse_index(index: &[u8]) -> Option<(Vec<(u64, u64)>, u64)> {
    let len = index.len();
    if len < 8 || index[0] != 0 || crc(&index[..len - 4]) != le32(&index[len - 4..]) {
        return None;
    }
    let body = &index[..len - 4];
    let (count, used) = strict_number(body.get(1..)?)?;
    let mut at = 1 + used;
    // Each record takes at least two bytes, so a count the index cannot
    // hold is refused before anything is reserved.
    if count > (body.len().saturating_sub(at) / 2) as u64 {
        return None;
    }
    let mut records = Vec::with_capacity(usize::try_from(count).ok()?);
    let mut padded = 0u64;
    for _ in 0..count {
        let (unpadded, used) = strict_number(body.get(at..)?)?;
        at += used;
        let (uncompressed, used) = strict_number(body.get(at..)?)?;
        at += used;
        if !(UNPADDED_MIN..=UNPADDED_MAX).contains(&unpadded) {
            return None;
        }
        padded = padded
            .checked_add(unpadded.div_ceil(4) * 4)
            .filter(|&sum| sum <= VLI_MAX)?;
        records.push((unpadded, uncompressed));
    }
    if at.next_multiple_of(4) != body.len() || body[at..].iter().any(|&b| b != 0) {
        return None;
    }
    Some((records, padded))
}

/// A block's index record, with the size of its stream's check.
#[derive(Clone, Copy)]
struct Record {
    /// The header, the data and the check, without the padding.
    unpadded: u64,
    /// The size it decodes to.
    uncompressed: u64,
    /// The check's size, in bytes.
    check_size: u64,
}

/// Checks one block against its index record: the header's CRC, flags,
/// sizes and filter chain, then the LZMA2 chunk headers, which must end
/// exactly where the record says with the uncompressed size it says.
/// `false` when the read budget ran out, so the walk stops there.
fn check_block(
    source: &Source,
    at: u64,
    record: Record,
    reads: &mut usize,
) -> Result<bool, VfsError> {
    if *reads >= BLOCK_READS_MAX {
        return Ok(false);
    }
    *reads += 1;
    let mut buf = [0u8; 1024];
    let got = source.read_at(at, &mut buf)?;
    let size_byte = *buf[..got].first().ok_or_else(|| damaged(MISMATCH))?;
    if size_byte == 0 {
        // The index indicator where a block should start.
        return Err(damaged(MISMATCH));
    }
    let header_size = (usize::from(size_byte) + 1) * 4;
    let header = buf[..got]
        .get(..header_size)
        .ok_or_else(|| damaged(MISMATCH))?;
    let body = &header[..header_size - 4];
    if crc(body) != le32(&header[header_size - 4..]) {
        return Err(damaged(BAD_BLOCK_HEADER));
    }
    let fields = BlockFields::parse(body)?;
    // The record's unpadded size holds the header, the data and the check.
    let compressed = record
        .unpadded
        .checked_sub(header_size as u64 + record.check_size)
        .filter(|&compressed| compressed > 0)
        .ok_or_else(|| damaged(MISMATCH))?;
    if fields
        .compressed
        .is_some_and(|declared| declared != compressed)
        || fields
            .uncompressed
            .is_some_and(|declared| declared != record.uncompressed)
    {
        return Err(damaged(MISMATCH));
    }
    let data = at + header_size as u64;
    match lzma2_size(source, data, compressed, reads)? {
        Some(size) if size == record.uncompressed => Ok(true),
        Some(_) => Err(damaged(MISMATCH)),
        None => Ok(false),
    }
}

/// What a block header says, read as the format allows.
struct BlockFields {
    /// The compressed size, when the header records it.
    compressed: Option<u64>,
    /// The uncompressed size, when the header records it.
    uncompressed: Option<u64>,
}

impl BlockFields {
    /// Reads a block header's body (without its CRC): the flags, the
    /// optional sizes, the filter flags and the padding. Damage is "a
    /// block header is damaged"; a field, filter or chain the format does
    /// not define is refused as unsupported, in words.
    fn parse(body: &[u8]) -> Result<Self, VfsError> {
        let bad = || damaged(BAD_BLOCK_HEADER);
        let field = |at: &mut usize| -> Result<u64, VfsError> {
            let (value, used) = body.get(*at..).and_then(strict_number).ok_or_else(bad)?;
            *at += used;
            Ok(value)
        };
        let flags = *body.get(1).ok_or_else(bad)?;
        if flags & 0x3C != 0 {
            return Err(unsupported(
                "an xz block header flag newer than this reader",
            ));
        }
        let mut at = 2;
        let compressed = if flags & 0x40 != 0 {
            match field(&mut at)? {
                0 => return Err(bad()),
                size => Some(size),
            }
        } else {
            None
        };
        let uncompressed = if flags & 0x80 != 0 {
            Some(field(&mut at)?)
        } else {
            None
        };
        let filters = usize::from(flags & 0x03) + 1;
        let mut chain = Vec::with_capacity(filters);
        for _ in 0..filters {
            let id = field(&mut at)?;
            let size = usize::try_from(field(&mut at)?).map_err(|_| bad())?;
            let props = body.get(at..at.saturating_add(size)).ok_or_else(bad)?;
            at += size;
            if id == LZMA2 && (props.len() != 1 || lzma2_dictionary(props[0]).is_none()) {
                return Err(bad());
            }
            chain.push(id);
        }
        if body[at..].iter().any(|&b| b != 0) {
            return Err(unsupported(
                "an xz block header field newer than this reader",
            ));
        }
        if let Some(&id) = chain.iter().find(|id| !FILTERS_KNOWN.contains(id)) {
            return Err(unsupported(&format!("xz filter number {id}")));
        }
        if chain.last() != Some(&LZMA2) {
            return Err(unsupported("an xz filter chain that does not end in LZMA2"));
        }
        if chain[..chain.len() - 1].contains(&LZMA2) {
            return Err(unsupported("an xz filter chain with LZMA2 before its end"));
        }
        Ok(Self {
            compressed,
            uncompressed,
        })
    }
}

/// Walks the LZMA2 chunk headers of a block's data at `at`, which must
/// take exactly `len` bytes and end with the end marker: the
/// uncompressed size the chunks add up to (the filters before LZMA2 keep
/// the size), or `None` when the read budget ran out. Only the chunk
/// headers are read, never the compressed data.
fn lzma2_size(
    source: &Source,
    at: u64,
    len: u64,
    reads: &mut usize,
) -> Result<Option<u64>, VfsError> {
    let end = at.checked_add(len).ok_or_else(|| damaged(MISMATCH))?;
    let mut pos = at;
    let mut size = 0u64;
    loop {
        if pos >= end {
            return Err(damaged(MISMATCH));
        }
        if *reads >= BLOCK_READS_MAX {
            return Ok(None);
        }
        *reads += 1;
        let mut buf = [0u8; 6];
        let got = source.read_at(pos, &mut buf)?;
        let room = usize::try_from(end - pos).unwrap_or(usize::MAX);
        let head = &buf[..got.min(room)];
        let control = *head.first().ok_or_else(|| damaged(MISMATCH))?;
        let (taken, unpacked) = match control {
            0x00 => {
                return if pos + 1 == end {
                    Ok(Some(size))
                } else {
                    Err(damaged(MISMATCH))
                };
            }
            0x01 | 0x02 => {
                let n = be16(head.get(1..3).ok_or_else(|| damaged(MISMATCH))?) + 1;
                (3 + n, n)
            }
            0x03..=0x7F => return Err(damaged("its compressed data is damaged")),
            _ => {
                let sizes = head.get(1..5).ok_or_else(|| damaged(MISMATCH))?;
                let unpacked = ((u64::from(control & 0x1F) << 16) | be16(&sizes[..2])) + 1;
                let packed = be16(&sizes[2..]) + 1;
                let props = u64::from((control >> 5) & 0x03 >= 2);
                (5 + props + packed, unpacked)
            }
        };
        pos = pos.checked_add(taken).ok_or_else(|| damaged(MISMATCH))?;
        size = size
            .checked_add(unpacked)
            .ok_or_else(|| damaged(MISMATCH))?;
    }
}

/// "it uses WHAT, which is not supported".
fn unsupported(what: &str) -> VfsError {
    VfsError::UnsupportedFeature {
        what: what.to_owned(),
    }
}

/// The xz decoder the provider reads through: `lzma-rust2`'s reader with
/// several streams allowed and the memory limit. Its source's errors pass
/// through whole; an error of its own that names no kind the provider
/// knows (the crate's "dist overflow") is said as damage, never as a disk
/// error with "check the disk".
pub(crate) struct Decoder<R: Read> {
    inner: lzma_rust2::XzReader<Marked<R>>,
}

impl<R: Read> Decoder<R> {
    /// A decoder over a source with this memory limit, in KiB.
    pub(crate) fn new(source: R, mem_limit_kb: u32) -> Self {
        Self {
            inner: lzma_rust2::XzReader::new_mem_limit(Marked(source), true, mem_limit_kb),
        }
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf).map_err(|error| {
            if error.kind() == io::ErrorKind::Other && !carries(&error) {
                to_io(damaged("its compressed data does not decode"))
            } else {
                error
            }
        })
    }
}

/// The decoder's source, whose errors are carried whole, so they are
/// told apart from the decoder's own.
struct Marked<R>(R);

impl<R: Read> Read for Marked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf).map_err(|error| {
            if carries(&error) || error.kind() == io::ErrorKind::Interrupted {
                error
            } else {
                to_io(VfsError::from(error))
            }
        })
    }
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn be16(b: &[u8]) -> u64 {
    u64::from(u16::from_be_bytes([b[0], b[1]]))
}

fn crc(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_codes_read_as_the_format_says() {
        assert_eq!(lzma2_dictionary(0), Some(4 << 10));
        assert_eq!(lzma2_dictionary(1), Some(6 << 10));
        assert_eq!(lzma2_dictionary(22), Some(8 << 20));
        assert_eq!(lzma2_dictionary(40), Some(u64::from(u32::MAX)));
        assert_eq!(lzma2_dictionary(41), None);
        assert_eq!(number(&[0x80, 0x01]), Some((128, 2)));
        assert_eq!(number(&[0xFF; 10]), None);
    }

    #[test]
    fn strict_numbers_are_as_short_as_they_can_be() {
        assert_eq!(strict_number(&[0x0D]), Some((13, 1)));
        assert_eq!(strict_number(&[0x00]), Some((0, 1)));
        assert_eq!(strict_number(&[0x80, 0x01]), Some((128, 2)));
        // 13 written in two bytes, as bad-1-vli-1.xz does.
        assert_eq!(number(&[0x8D, 0x00]), Some((13, 2)));
        assert_eq!(strict_number(&[0x8D, 0x00]), None);
    }

    #[test]
    fn check_types_and_flags_read_in_words() {
        assert_eq!(check_of([0, 0x01]), Ok(0x01));
        assert_eq!(check_of([0, 0x0A]), Ok(0x0A));
        assert_eq!(
            check_of([0, 0x02]).unwrap_err().to_string(),
            "it uses xz check type 2, which is not supported"
        );
        assert_eq!(
            check_of([1, 0x01]).unwrap_err().to_string(),
            "it uses xz stream flags newer than this reader, which is not supported"
        );
        assert_eq!(check_size(0x0A), 32);
        assert_eq!(check_size(0x02), 4);
    }

    /// A one-block xz file of `data`, from the crate's encoder.
    fn encoded(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut writer =
            lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(0)).unwrap();
        writer.write_all(data).unwrap();
        writer.finish().unwrap()
    }

    #[test]
    fn the_walk_checks_blocks_and_stops_at_its_budget() {
        let data: Vec<u8> = (0..200_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect();
        let source = Source::from_bytes(encoded(&data));
        let limits = Limits::default();
        assert_eq!(verify(&source, &limits), Ok(Some(data.len() as u64)));
        let streams = streams(&source, &limits).unwrap().unwrap();
        let &(unpadded, uncompressed) = &streams[0].records[0];
        let record = Record {
            unpadded,
            uncompressed,
            check_size: check_size(streams[0].check),
        };
        let mut reads = 0;
        assert_eq!(check_block(&source, 12, record, &mut reads), Ok(true));
        assert!(reads > 2, "{reads} reads: a header and several chunks");
        // At the budget, the walk stops and leaves the rest to the decoder.
        let mut reads = BLOCK_READS_MAX;
        assert_eq!(check_block(&source, 12, record, &mut reads), Ok(false));
        let mut reads = BLOCK_READS_MAX - 2;
        assert_eq!(check_block(&source, 12, record, &mut reads), Ok(false));
        // A record one byte off is found.
        let off = Record {
            uncompressed: uncompressed + 1,
            ..record
        };
        let mut reads = 0;
        assert_eq!(
            check_block(&source, 12, off, &mut reads),
            Err(damaged(MISMATCH))
        );
    }

    #[test]
    fn an_index_is_read_whole_or_not_at_all() {
        // No records: indicator, count 0, two bytes of padding, the CRC.
        let mut index = vec![0, 0, 0, 0];
        index.extend_from_slice(&crc(&index).to_le_bytes());
        assert_eq!(parse_index(&index), Some((Vec::new(), 0)));
        // A count the index cannot hold is refused before reserving.
        let mut huge = vec![0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F];
        huge.extend_from_slice(&crc(&huge).to_le_bytes());
        assert_eq!(parse_index(&huge), None);
        // A wrong CRC.
        let mut wrong = index.clone();
        wrong[4] ^= 1;
        assert_eq!(parse_index(&wrong), None);
    }
}
