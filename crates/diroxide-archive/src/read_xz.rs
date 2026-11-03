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
//!
//! The stream path has a guard of its own ([`check_indexes`]), run each
//! time a decoder is made, before `lzma-rust2` reads a byte: `lzma-rust2`
//! 0.21 reserves room for an index's records from the count written in
//! the file before it reads one, so 25 bytes found by fuzzing asked it
//! for 506 GB. The guard walks every stream's footer and index back from
//! the end and holds each record count to the bytes its index has (two a
//! record at the least) and to the decoder's memory limit. What it leaves
//! to the decoder is said on [`check_indexes`].

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

/// The most streams the stream path's guard follows back from the end
/// ([`check_indexes`]): it keeps nothing per stream, so it goes further
/// than [`STREAMS_MAX`]; past it, the file is refused as unsupported. No
/// tool writes so many: each stream is a whole compressed file.
const GUARD_STREAMS_MAX: usize = 100_000;

/// The memory `lzma-rust2` keeps for each index record while it decodes:
/// two sizes of eight bytes.
const RECORD_MEMORY: u64 = 16;

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
    let mut end = source.len();
    let mut found = Vec::new();
    let mut records_seen = 0u64;
    for _ in 0..STREAMS_MAX {
        end = before_padding(source, end)?;
        if end == 0 {
            if found.is_empty() {
                return Err(damaged("it holds no xz stream"));
            }
            found.reverse();
            return Ok(Some(found));
        }
        let mut records = Vec::new();
        let ends = stream_ending_at(
            source,
            end,
            limits.max_header_bytes,
            // The entries are counted before a record is kept.
            |count| {
                records_seen = records_seen.saturating_add(count);
                if records_seen > limits.max_entries as u64 {
                    return Err(VfsError::LimitReached {
                        what: "entries".to_owned(),
                        limit: grouped(limits.max_entries as u64),
                    });
                }
                Ok(())
            },
            |unpadded, uncompressed| records.push((unpadded, uncompressed)),
        )?;
        found.push(Stream {
            start: ends.start,
            check: ends.check,
            records,
        });
        end = ends.start;
    }
    Ok(None)
}

/// The guard of the stream path, run each time a decoder is made, before
/// `lzma-rust2` reads a byte: every stream's footer and index, walked
/// back from the end over stream padding as [`streams`] walks them, with
/// each index's record count held to the bytes the index has (each
/// record takes two at the least, so a count past them is damage) and to
/// the decoder's memory limit (`lzma-rust2` 0.21 reserves 16 bytes a
/// record from the count before it reads one). It keeps nothing per
/// stream, and past [`GUARD_STREAMS_MAX`] streams the file is refused as
/// unsupported.
///
/// What it leaves to the decoder: a block whose compressed data ends
/// before its index record says could make `lzma-rust2` read an index
/// from inside the block, which no walk from the end sees. The provider's
/// walk at open ([`verify`]) checks each block's chunk headers against
/// its record within its read budget; past the budget, `lzma-rust2`
/// asks with `try_reserve_exact`, which fails cleanly rather than end the
/// program, and never touches the room it gets.
pub(crate) fn check_indexes(source: &Source, mem_limit_kb: u32) -> Result<(), VfsError> {
    let memory = u64::from(mem_limit_kb).saturating_mul(1024);
    let mut end = source.len();
    for walked in 0..GUARD_STREAMS_MAX {
        end = before_padding(source, end)?;
        if end == 0 {
            if walked == 0 {
                return Err(damaged("it holds no xz stream"));
            }
            return Ok(());
        }
        let ends = stream_ending_at(
            source,
            end,
            u64::MAX,
            |count| {
                if count.saturating_mul(RECORD_MEMORY) > memory {
                    return Err(VfsError::LimitReached {
                        what: "memory for one operation".to_owned(),
                        limit: limit_words(memory),
                    });
                }
                Ok(())
            },
            |_, _| {},
        )?;
        end = ends.start;
    }
    Err(unsupported(&format!(
        "more than {} xz streams",
        grouped(GUARD_STREAMS_MAX as u64)
    )))
}

/// Where a stream starts and its check type, read back from its end.
struct Ends {
    /// Where its stream header starts.
    start: u64,
    /// Its check type.
    check: u8,
}

/// A run of a source's bytes, from `at` to `end`, read where they are
/// with no buffer of its own (a view's buffer is 64 KiB, too much for an
/// index of eight bytes in each of many streams).
struct Span<'a> {
    source: &'a Source,
    at: u64,
    end: u64,
}

impl Read for Span<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let room = usize::try_from(self.end.saturating_sub(self.at)).unwrap_or(usize::MAX);
        let want = buf.len().min(room);
        let n = self.source.read_at(self.at, &mut buf[..want])?;
        self.at += n as u64;
        Ok(n)
    }
}

/// The end of the data before stream padding (zero bytes in fours) that
/// ends at `end`.
fn before_padding(source: &Source, mut end: u64) -> Result<u64, VfsError> {
    while end >= 4 {
        let mut four = [0u8; 4];
        source.read_exact_at(end - 4, &mut four)?;
        if four != [0; 4] {
            break;
        }
        end -= 4;
    }
    Ok(end)
}

/// The stream whose footer ends at `end`, read back from there as the
/// format says: the footer, its index (refused as "header size" past
/// `index_max` bytes), and its header, which must agree with the footer.
/// The index is read as it goes ([`read_index`]): `allow` sees its record
/// count before a record is read, and `each` gets every record.
fn stream_ending_at(
    source: &Source,
    end: u64,
    index_max: u64,
    allow: impl FnOnce(u64) -> Result<(), VfsError>,
    each: impl FnMut(u64, u64),
) -> Result<Ends, VfsError> {
    let footer_damaged = || damaged("its stream footer is damaged");
    let footer_at = end.checked_sub(12).ok_or_else(footer_damaged)?;
    let mut footer = [0u8; 12];
    source.read_exact_at(footer_at, &mut footer)?;
    if footer[10..12] != FOOTER_MAGIC || crc(&footer[4..10]) != le32(&footer[..4]) {
        return Err(footer_damaged());
    }
    let flags = [footer[8], footer[9]];
    let index_size = (u64::from(le32(&footer[4..8])) + 1) * 4;
    if index_size > index_max {
        return Err(VfsError::LimitReached {
            what: "header size".to_owned(),
            limit: limit_words(index_max),
        });
    }
    let index_at = footer_at
        .checked_sub(index_size)
        .filter(|&at| at >= 12)
        .ok_or_else(footer_damaged)?;
    let span = Span {
        source,
        at: index_at,
        end: footer_at,
    };
    let sums = read_index(span, index_size, allow, each)?.ok_or_else(|| damaged(BAD_INDEX))?;
    let start = index_at
        .checked_sub(sums.padded)
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
    Ok(Ends { start, check })
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

/// What an index adds up to.
#[derive(Debug, PartialEq, Eq)]
struct IndexSums {
    /// How many records it holds.
    count: u64,
    /// The blocks' sizes with their padding, added up.
    padded: u64,
}

/// An index of `len` bytes read whole from `reader` as it goes, keeping
/// nothing but what `each` keeps: its indicator, a record count the
/// index can hold (each record takes two bytes at the least, so a count
/// past the bytes left is refused before a record is read), its records
/// (unpadded and uncompressed sizes, each in range and written as short
/// as it can be), zero padding, and its CRC-32. `allow` sees the count
/// before a record is read and may refuse it; `each` gets every record.
/// `Ok(None)` for any break of the format's rules.
fn read_index<R: Read>(
    reader: R,
    len: u64,
    allow: impl FnOnce(u64) -> Result<(), VfsError>,
    mut each: impl FnMut(u64, u64),
) -> Result<Option<IndexSums>, VfsError> {
    if len < 8 || !len.is_multiple_of(4) {
        return Ok(None);
    }
    // The records and the padding come before the CRC-32.
    let body = len - 4;
    let mut tally = Tally::new(reader);
    if tally.byte()? != Some(0) {
        return Ok(None);
    }
    let Some(count) = tally.number()? else {
        return Ok(None);
    };
    if tally.taken > body || count > (body - tally.taken) / 2 {
        return Ok(None);
    }
    allow(count)?;
    let mut padded = 0u64;
    for _ in 0..count {
        let Some(unpadded) = tally.number()? else {
            return Ok(None);
        };
        let Some(uncompressed) = tally.number()? else {
            return Ok(None);
        };
        if tally.taken > body || !(UNPADDED_MIN..=UNPADDED_MAX).contains(&unpadded) {
            return Ok(None);
        }
        let Some(sum) = padded
            .checked_add(unpadded.div_ceil(4) * 4)
            .filter(|&sum| sum <= VLI_MAX)
        else {
            return Ok(None);
        };
        padded = sum;
        each(unpadded, uncompressed);
    }
    // Zero padding to a multiple of four: three bytes at the most.
    if body - tally.taken > 3 {
        return Ok(None);
    }
    while tally.taken < body {
        if tally.byte()? != Some(0) {
            return Ok(None);
        }
    }
    let expected = tally.sum();
    let mut stored = [0u8; 4];
    for slot in &mut stored {
        let Some(b) = tally.byte()? else {
            return Ok(None);
        };
        *slot = b;
    }
    if u32::from_le_bytes(stored) != expected {
        return Ok(None);
    }
    Ok(Some(IndexSums { count, padded }))
}

/// A reader taken a byte at a time through a small buffer of its own,
/// counting what it hands out and adding it to a CRC-32.
struct Tally<R> {
    reader: R,
    crc: crc32fast::Hasher,
    /// The bytes handed out.
    taken: u64,
    buf: [u8; 1024],
    /// The bytes in `buf`.
    filled: usize,
    /// The next byte of `buf` to hand out.
    next: usize,
    /// The bytes of `buf` already added to the CRC.
    hashed: usize,
}

impl<R: Read> Tally<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            crc: crc32fast::Hasher::new(),
            taken: 0,
            buf: [0; 1024],
            filled: 0,
            next: 0,
            hashed: 0,
        }
    }

    /// The next byte, or `None` at the end.
    fn byte(&mut self) -> Result<Option<u8>, VfsError> {
        if self.next == self.filled {
            self.crc.update(&self.buf[self.hashed..self.filled]);
            self.filled = loop {
                match self.reader.read(&mut self.buf) {
                    Ok(n) => break n,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(VfsError::from(error)),
                }
            };
            self.next = 0;
            self.hashed = 0;
            if self.filled == 0 {
                return Ok(None);
            }
        }
        let b = self.buf[self.next];
        self.next += 1;
        self.taken += 1;
        Ok(Some(b))
    }

    /// The CRC-32 of every byte handed out so far.
    fn sum(&mut self) -> u32 {
        self.crc.update(&self.buf[self.hashed..self.next]);
        self.hashed = self.next;
        self.crc.clone().finalize()
    }

    /// A variable-length number as [`strict_number`] allows it: at most
    /// nine bytes, never longer than it needs to be; `None` for one the
    /// format does not allow, or the end.
    fn number(&mut self) -> Result<Option<u64>, VfsError> {
        let mut value = 0u64;
        for i in 0..9 {
            let Some(b) = self.byte()? else {
                return Ok(None);
            };
            value |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok((i == 0 || b != 0).then_some(value));
            }
        }
        Ok(None)
    }
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
pub(crate) mod tests {
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

    /// An index's records and sums, read from its bytes; `None` for a
    /// break of the rules. Panics when `allow` is reached for a count it
    /// should not see.
    fn index_of(bytes: &[u8], allowed: bool) -> Option<(Vec<(u64, u64)>, IndexSums)> {
        let mut records = Vec::new();
        let sums = read_index(
            bytes,
            bytes.len() as u64,
            |count| {
                assert!(allowed, "allow saw the count {count}");
                Ok(())
            },
            |unpadded, uncompressed| records.push((unpadded, uncompressed)),
        )
        .unwrap()?;
        Some((records, sums))
    }

    /// The bytes with their CRC-32 after them.
    fn with_crc(mut bytes: Vec<u8>) -> Vec<u8> {
        let sum = crc(&bytes);
        bytes.extend_from_slice(&sum.to_le_bytes());
        bytes
    }

    #[test]
    fn an_index_is_read_whole_or_not_at_all() {
        // No records: indicator, count 0, two bytes of padding, the CRC.
        let index = with_crc(vec![0, 0, 0, 0]);
        let none = IndexSums {
            count: 0,
            padded: 0,
        };
        assert_eq!(index_of(&index, true), Some((Vec::new(), none)));
        // One record of five bytes that decode to one: padded to eight.
        let one = with_crc(vec![0, 1, 5, 1]);
        let sums = IndexSums {
            count: 1,
            padded: 8,
        };
        assert_eq!(index_of(&one, true), Some((vec![(5, 1)], sums)));
        // A count the index cannot hold is refused before a record is
        // read and before `allow` sees it, whatever the CRC says.
        let huge = with_crc(vec![0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
        assert_eq!(index_of(&huge, false), None);
        let two = with_crc(vec![0, 2, 5, 1]);
        assert_eq!(index_of(&two, false), None);
        // A wrong CRC, padding that is not zero, a record under five.
        let mut wrong = index.clone();
        wrong[4] ^= 1;
        assert_eq!(index_of(&wrong, true), None);
        assert_eq!(index_of(&with_crc(vec![0, 0, 0, 1]), true), None);
        assert_eq!(index_of(&with_crc(vec![0, 1, 4, 1]), true), None);
        // A number written longer than it needs.
        assert_eq!(index_of(&with_crc(vec![0, 0x81, 0x00, 0]), true), None);
        // `allow` may refuse the count, and its words come through.
        let refused = read_index(
            &one[..],
            one.len() as u64,
            |_| Err(damaged("no")),
            |_, _| {},
        );
        assert_eq!(refused, Err(damaged("no")));
    }

    /// The 25 bytes fuzzing found (F's notes, Tuesday, November 3, 2026):
    /// a stream header, then an index whose record count is 31,650,202,408,
    /// for which `lzma-rust2` 0.21 asks 506 GB.
    pub(crate) const FOUND: [u8; 25] = [
        0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, 0x00, 0x04, 0xE6, 0xD6, 0xB4, 0x46, 0x00, 0xA8, 0x86,
        0xFF, 0xF3, 0x75, 0x7D, 0x01, 0x00, 0x00, 0x04, 0x00, 0x5A,
    ];

    /// A stream whose footer and index CRC are right but whose index
    /// says `count` records after `room` bytes of zeros: the shape of
    /// [`FOUND`] with a footer, so only the count is wrong.
    pub(crate) fn counted(count: u64, room: usize) -> Vec<u8> {
        let flags = [0u8, 0x01];
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&flags);
        out.extend_from_slice(&crc(&flags).to_le_bytes());
        let mut index = vec![0u8];
        let mut value = count;
        while value >= 0x80 {
            index.push((value as u8 & 0x7F) | 0x80);
            value >>= 7;
        }
        index.push(value as u8);
        index.resize(index.len() + room, 0);
        index.resize(index.len().next_multiple_of(4), 0);
        let index = with_crc(index);
        let backward = (index.len() / 4 - 1) as u32;
        out.extend_from_slice(&index);
        let mut tail = backward.to_le_bytes().to_vec();
        tail.extend_from_slice(&flags);
        out.extend_from_slice(&crc(&tail).to_le_bytes());
        out.extend_from_slice(&tail);
        out.extend_from_slice(&FOOTER_MAGIC);
        out
    }

    #[test]
    fn the_fuzz_find_is_refused_on_both_paths_before_the_decoder() {
        let limits = Limits::strict();
        let kb = mem_limit_kb(&limits);
        let allow = Allowances::default();
        // The provider's checks at open, then the stream path's guard.
        let found = Source::from_bytes(FOUND.to_vec());
        assert_eq!(
            check(&found, "found.xz", &limits, allow)
                .unwrap_err()
                .to_string(),
            "it is damaged: it ends without a stream footer: \
             it is cut short or has data after its end"
        );
        assert_eq!(
            check_indexes(&found, kb),
            Err(damaged("its stream footer is damaged"))
        );
        // With a footer, the count alone is refused: 31,650,202,408
        // records in an index of a few bytes.
        let shaped = Source::from_bytes(counted(31_650_202_408, 0));
        assert_eq!(
            check(&shaped, "shaped.xz", &limits, allow)
                .unwrap_err()
                .to_string(),
            "it is damaged: its index is damaged"
        );
        assert_eq!(check_indexes(&shaped, kb), Err(damaged(BAD_INDEX)));
        // One record more than the index's bytes hold, at any size.
        let one_over = Source::from_bytes(counted(1_001, 2_000));
        assert_eq!(check_indexes(&one_over, kb), Err(damaged(BAD_INDEX)));
    }

    #[test]
    fn the_guard_holds_a_count_to_the_decoders_memory() {
        // Room for 100 records, which at 16 bytes each pass 1 KiB.
        let source = Source::from_bytes(counted(100, 200));
        assert_eq!(
            check_indexes(&source, 1).unwrap_err().to_string(),
            "it passes the limit for memory for one operation: 1 KiB"
        );
        // Under the limit, the count is fine and the records decide.
        assert_eq!(check_indexes(&source, 64), Err(damaged(BAD_INDEX)));
        // A good file passes, with one stream or several.
        let good = encoded(b"guarded");
        assert_eq!(check_indexes(&Source::from_bytes(good.clone()), 1), Ok(()));
        let mut joined = good.clone();
        joined.extend_from_slice(&[0; 8]);
        joined.extend_from_slice(&good);
        assert_eq!(check_indexes(&Source::from_bytes(joined), 1), Ok(()));
        assert_eq!(
            check_indexes(&Source::from_bytes(Vec::new()), 1),
            Err(damaged("it holds no xz stream"))
        );
    }

    #[test]
    fn the_guard_walks_past_the_index_walks_reach() {
        // A bad index in the first of 1,001 streams: the walk at open
        // stops at 1,000 and leaves the size unknown, the guard finds it.
        let empty = encoded(b"");
        let mut many = counted(31_650_202_408, 0);
        for _ in 0..STREAMS_MAX {
            many.extend_from_slice(&empty);
        }
        let source = Source::from_bytes(many);
        assert_eq!(verify(&source, &Limits::default()), Ok(None));
        assert_eq!(check_indexes(&source, 1 << 20), Err(damaged(BAD_INDEX)));
        // Past its own cap, the guard refuses in words.
        let mut more = Vec::with_capacity(empty.len() * (GUARD_STREAMS_MAX + 1));
        for _ in 0..=GUARD_STREAMS_MAX {
            more.extend_from_slice(&empty);
        }
        assert_eq!(
            check_indexes(&Source::from_bytes(more), 1 << 20)
                .unwrap_err()
                .to_string(),
            "it uses more than 100,000 xz streams, which is not supported"
        );
    }
}
