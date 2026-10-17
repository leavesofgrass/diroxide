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

use crate::error::ArchiveError;
use diroxide_vfs::VfsError;

use crate::guard::{Allowances, check_dictionary, damaged};
use crate::limits::Limits;
use crate::source::Source;

/// The xz stream header's magic.
const MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];

/// The footer's magic.
const FOOTER_MAGIC: [u8; 2] = [b'Y', b'Z'];

/// The most streams an index walk follows back from the end.
const STREAMS_MAX: usize = 1_000;

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
/// dictionary against the limits, and the file's end.
pub(crate) fn check(
    source: &Source,
    name: &str,
    limits: &Limits,
    allow: Allowances,
) -> Result<(), ArchiveError> {
    if let Some(dictionary) = first_dictionary(source, 0) {
        check_dictionary(name, dictionary, limits, allow)?;
    }
    check_end(source)?;
    Ok(())
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
/// the indexes cannot be read (a cut-off file), so the size is said to be
/// unknown rather than guessed.
pub(crate) fn uncompressed_size(source: &Source, limits: &Limits) -> Option<u64> {
    let mut end = source.len();
    let mut total = 0u64;
    let mut records_seen = 0u64;
    for _ in 0..STREAMS_MAX {
        // Stream padding: zero bytes in multiples of four.
        while end >= 4 {
            let mut four = [0u8; 4];
            source.read_exact_at(end - 4, &mut four).ok()?;
            if four != [0; 4] {
                break;
            }
            end -= 4;
        }
        if end == 0 {
            return Some(total);
        }
        let mut footer = [0u8; 12];
        source
            .read_exact_at(end.checked_sub(12)?, &mut footer)
            .ok()?;
        if footer[10..12] != FOOTER_MAGIC || crc(&footer[4..10]) != le32(&footer[..4]) {
            return None;
        }
        let backward = u64::from(le32(&footer[4..8]));
        let index_size = (backward + 1) * 4;
        let index_at = end.checked_sub(12)?.checked_sub(index_size)?;
        if index_size > limits.max_header_bytes {
            return None;
        }
        let mut index = vec![0u8; usize::try_from(index_size).ok()?];
        source.read_exact_at(index_at, &mut index).ok()?;
        if index.first() != Some(&0)
            || crc(&index[..index.len() - 4]) != le32(&index[index.len() - 4..])
        {
            return None;
        }
        let mut at = 1;
        let (records, used) = number(&index[at..])?;
        at += used;
        records_seen = records_seen.checked_add(records)?;
        if records_seen > limits.max_entries as u64 {
            return None;
        }
        let mut blocks = 0u64;
        for _ in 0..records {
            let (unpadded, used) = number(index.get(at..)?)?;
            at += used;
            let (size, used) = number(index.get(at..)?)?;
            at += used;
            total = total.checked_add(size)?;
            blocks = blocks.checked_add(unpadded.checked_add(3)? / 4 * 4)?;
        }
        let start = index_at.checked_sub(blocks)?.checked_sub(12)?;
        let mut header = [0u8; 6];
        source.read_exact_at(start, &mut header).ok()?;
        if header != MAGIC {
            return None;
        }
        end = start;
    }
    None
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
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
}
