//! 7z reading on `sevenz-rust2` (research/archives.md 2 and 6): the start
//! header checked and the encoded header's coders read by the provider
//! before the crate decodes anything, every block's dictionary checked
//! before any member is read, header encryption asking for the password,
//! and members read through the crate's block decoder on a thread of their
//! own: in a solid block, everything before the member is decoded and set
//! aside, so reading one member alone asks first when that is a lot, and
//! the in-order visit ([`crate::ArchiveProvider::visit`]) reads a whole
//! solid archive in one pass.
//!
//! From spike 4.1: `LzmaReader` allocates the smaller of its dictionary
//! and the stream's stated size, both read from the archive, so a header
//! that states a 4 GiB dictionary and a large size would take 4 GiB at
//! open. The pre-scan refuses it in words first.

use std::fmt;
use std::io::{self, Read};
use std::sync::Arc;

use diroxide_core::Secret;
use diroxide_core::format::grouped;
use diroxide_vfs::{CancelToken, VfsError};
use sevenz_rust2::{Archive, BlockDecoder, Password};

use crate::error::ArchiveError;
use crate::guard::{Allowances, check_dictionary, damaged, refuse};
use crate::index::{Index, IndexBuilder, Locator, Member, RecordFlags, RecordKind};
use crate::limits::{Limits, limit_words};
use crate::pipe::{self, PipeReader};
use crate::source::Source;

/// The signature header's size.
const START: u64 = 32;

/// The 7z signature.
const SIGNATURE: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];

/// Method ids.
const ID_LZMA: &[u8] = &[0x03, 0x01, 0x01];
const ID_LZMA2: &[u8] = &[0x21];
const ID_PPMD: &[u8] = &[0x03, 0x04, 0x01];
const ID_AES: &[u8] = &[0x06, 0xF1, 0x07, 0x01];

/// The memory a coder asks for: an LZMA or LZMA2 dictionary, or PPMd's
/// model, read from its properties; `None` for a coder that asks for
/// little.
pub fn coder_memory(id: &[u8], props: &[u8]) -> Option<u64> {
    if id == ID_LZMA && props.len() >= 5 {
        return Some(u64::from(u32::from_le_bytes([
            props[1], props[2], props[3], props[4],
        ])));
    }
    if id == ID_LZMA2 {
        return props
            .first()
            .and_then(|&code| crate::read_xz::lzma2_dictionary(code));
    }
    if id == ID_PPMD && props.len() >= 5 {
        return Some(u64::from(u32::from_le_bytes([
            props[1], props[2], props[3], props[4],
        ])));
    }
    None
}

/// A reader over a header's bytes, for the pre-scan.
struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bytes<'_> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let slice = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(slice)
    }

    /// 7z's number: the first byte's high bits count the bytes after it.
    fn number(&mut self) -> Option<u64> {
        let first = self.byte()?;
        let mut mask = 0x80u8;
        let mut value = 0u64;
        for i in 0..8 {
            if first & mask == 0 {
                let high = u64::from(first & mask.wrapping_sub(1));
                return Some(value | (high << (8 * i)));
            }
            value |= u64::from(self.byte()?) << (8 * i);
            mask >>= 1;
        }
        Some(value)
    }
}

/// What the encoded header's folders say: their coders, and the size the
/// header decodes to.
#[derive(Debug, Default)]
struct HeaderPlan {
    coders: Vec<(Vec<u8>, Vec<u8>)>,
    unpack_size: u64,
}

/// Reads the coders of an encoded header's streams info, as far as the
/// pre-scan needs. `None` when the bytes are not what the format says;
/// the crate then decides, under the limits already checked.
fn header_plan(next: &[u8]) -> Option<HeaderPlan> {
    let mut b = Bytes { data: next, at: 0 };
    if b.byte()? != 0x17 {
        return None;
    }
    let mut plan = HeaderPlan::default();
    let mut outs_per_folder: Vec<u64> = Vec::new();
    loop {
        match b.byte()? {
            0x00 => break,
            0x06 => {
                // Pack info: position, count, then sizes and CRCs.
                let _position = b.number()?;
                let count = b.number()?;
                loop {
                    match b.byte()? {
                        0x00 => break,
                        0x09 => {
                            for _ in 0..count.min(1 << 16) {
                                b.number()?;
                            }
                        }
                        0x0A => {
                            let all = b.byte()?;
                            if all == 0 {
                                return None;
                            }
                            b.take(usize::try_from(count).ok()?.checked_mul(4)?)?;
                        }
                        _ => return None,
                    }
                }
            }
            0x07 => {
                // Unpack info: the folders and their coders.
                if b.byte()? != 0x0B {
                    return None;
                }
                let folders = b.number()?.min(64);
                if b.byte()? != 0 {
                    return None;
                }
                for _ in 0..folders {
                    let coders = b.number()?.min(64);
                    let mut outs = 0u64;
                    let mut inputs = 0u64;
                    for _ in 0..coders {
                        let flags = b.byte()?;
                        let id = b.take(usize::from(flags & 0x0F))?.to_vec();
                        let (ins, out) = if flags & 0x10 != 0 {
                            (b.number()?, b.number()?)
                        } else {
                            (1, 1)
                        };
                        inputs += ins;
                        outs += out;
                        let props = if flags & 0x20 != 0 {
                            let size = usize::try_from(b.number()?).ok()?;
                            b.take(size)?.to_vec()
                        } else {
                            Vec::new()
                        };
                        plan.coders.push((id, props));
                    }
                    for _ in 1..outs {
                        b.number()?;
                        b.number()?;
                    }
                    let packed = inputs.checked_sub(outs.saturating_sub(1))?;
                    if packed > 1 {
                        for _ in 0..packed.min(64) {
                            b.number()?;
                        }
                    }
                    outs_per_folder.push(outs);
                }
                if b.byte()? != 0x0C {
                    return None;
                }
                let mut sizes = Vec::new();
                for &outs in &outs_per_folder {
                    for _ in 0..outs {
                        sizes.push(b.number()?);
                    }
                }
                plan.unpack_size = sizes.into_iter().max().unwrap_or(0);
                return Some(plan);
            }
            _ => return None,
        }
    }
    Some(plan)
}

/// Holds the start header to the file and the limits, and reads the
/// encoded header's coders: whether the header is encrypted, and the
/// memory its decoder asks for.
fn pre_scan(
    source: &Source,
    name: &str,
    limits: &Limits,
    allow: Allowances,
) -> Result<bool, ArchiveError> {
    let mut start = [0u8; START as usize];
    source
        .read_exact_at(0, &mut start)
        .map_err(|_| damaged("its start header is cut short"))?;
    if start[..6] != SIGNATURE {
        return Err(damaged("its start header is not a 7z header").into());
    }
    if crc32fast::hash(&start[12..32])
        != u32::from_le_bytes([start[8], start[9], start[10], start[11]])
    {
        return Err(damaged("its start header's checksum failed").into());
    }
    let offset = u64::from_le_bytes(start[12..20].try_into().unwrap_or([0; 8]));
    let size = u64::from_le_bytes(start[20..28].try_into().unwrap_or([0; 8]));
    if size > limits.max_header_bytes {
        return Err(refuse("header size", limit_words(limits.max_header_bytes)).into());
    }
    let at = START
        .checked_add(offset)
        .filter(|at| at.checked_add(size).is_some_and(|end| end <= source.len()))
        .ok_or_else(|| damaged("its header is not where it says"))?;
    let mut next = vec![0u8; usize::try_from(size).unwrap_or(0)];
    source
        .read_exact_at(at, &mut next)
        .map_err(|_| damaged("its header is cut short"))?;
    let Some(plan) = header_plan(&next) else {
        // A plain header decodes nothing. An encoded one the pre-scan
        // cannot read is refused: its decoder's memory is unknown.
        if next.first() == Some(&0x17) {
            return Err(VfsError::UnsupportedFeature {
                what: "a header in a form diroxide does not read".to_owned(),
            }
            .into());
        }
        return Ok(false);
    };
    if plan.unpack_size > limits.max_header_bytes {
        return Err(refuse("header size", limit_words(limits.max_header_bytes)).into());
    }
    let mut encrypted = false;
    for (id, props) in &plan.coders {
        if id.as_slice() == ID_AES {
            encrypted = true;
        }
        if let Some(memory) = coder_memory(id, props) {
            check_dictionary(name, memory, limits, allow)?;
        }
    }
    Ok(encrypted)
}

/// The crate's error, in the provider's words. 7z cannot tell a wrong
/// password from damage, so its message says both.
fn sevenz_error(error: sevenz_rust2::Error, with_password: bool) -> VfsError {
    use sevenz_rust2::Error as E;
    match error {
        E::PasswordRequired => VfsError::PasswordRequired,
        E::MaybeBadPassword(_) => VfsError::WrongPassword,
        E::BadSignature(_) => damaged("its start header is not a 7z header"),
        E::UnsupportedVersion { major, minor } => VfsError::UnsupportedFeature {
            what: format!("7z format version {major}.{minor}"),
        },
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch if with_password => {
            VfsError::WrongPassword
        }
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch => damaged("a checksum failed"),
        E::UnsupportedCompressionMethod(method) => VfsError::UnsupportedFeature {
            what: format!("compression method {method}"),
        },
        E::MaxMemLimited { .. } => VfsError::LimitReached {
            what: "memory for one operation".to_owned(),
            limit: "the decoder's limit".to_owned(),
        },
        E::Io(error, _) => crate::guard::from_io(error),
        E::FileNotFound => VfsError::NotFound,
        other => damaged(&crate::guard::lowercase_first(&other.to_string())),
    }
}

/// An NT time (100 ns since 1601) in seconds since the Unix epoch.
fn unix_seconds(nt: sevenz_rust2::NtTime) -> Option<i64> {
    let ticks = u64::from(nt);
    i64::try_from(ticks / 10_000_000)
        .ok()?
        .checked_sub(11_644_473_600)
}

/// A 7z opened: the crate's archive, shared with every read.
pub(crate) struct SevenZBackend {
    archive: Arc<Archive>,
    source: Source,
    password: Option<Secret>,
    solid: bool,
    threads: u32,
}

impl fmt::Debug for SevenZBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SevenZBackend")
            .field("solid", &self.solid)
            .finish_non_exhaustive()
    }
}

/// Opens a 7z: the pre-scan, the password when its header is encrypted,
/// the crate's parse, every block's dictionary, the index.
pub(crate) fn open(
    source: Source,
    container: &str,
    limits: &Limits,
    allow: Allowances,
    ask_password: &dyn Fn() -> Option<Secret>,
) -> Result<(Index, SevenZBackend), ArchiveError> {
    let encrypted_header = pre_scan(&source, container, limits, allow)?;
    let password = if encrypted_header {
        Some(ask_password().ok_or(VfsError::PasswordRequired)?)
    } else {
        None
    };
    let crate_password = password
        .as_ref()
        .map_or_else(Password::empty, |p| Password::new(p.expose()));
    let mut reader = source.clone();
    let archive = Archive::read(&mut reader, &crate_password)
        .map_err(|e| sevenz_error(e, password.is_some()))?;
    drop(crate_password);
    // Every block's coders, before any member is decoded.
    let mut largest = 0u64;
    let mut block_encrypted = vec![false; archive.blocks.len()];
    for (i, block) in archive.blocks.iter().enumerate() {
        for coder in &block.coders {
            if coder.encoder_method_id() == ID_AES {
                block_encrypted[i] = true;
            }
            if let Some(memory) = coder_memory(coder.encoder_method_id(), coder.properties()) {
                largest = largest.max(memory);
            }
        }
    }
    if largest > 0 {
        check_dictionary(container, largest, limits, allow)?;
    }
    if archive.files.len() > limits.max_entries {
        return Err(refuse("entries", grouped(limits.max_entries as u64)).into());
    }
    let mut builder = IndexBuilder::new(*limits);
    let solid = archive.is_solid;
    for (i, file) in archive.files.iter().enumerate() {
        if file.is_anti_item() {
            continue;
        }
        let block = archive
            .stream_map
            .file_block_index
            .get(i)
            .copied()
            .flatten();
        let mut flags = RecordFlags::NONE;
        if block.is_some_and(|b| block_encrypted.get(b).copied().unwrap_or(false)) {
            flags.insert(RecordFlags::ENCRYPTED);
        }
        let attributes = file.windows_attributes();
        let mode =
            (file.has_windows_attributes && attributes & 0x8000 != 0).then_some(attributes >> 16);
        let kind = if file.is_directory() {
            RecordKind::Folder
        } else {
            match mode.map(|m| m & 0o170_000) {
                Some(0o120_000) => RecordKind::Link,
                None | Some(0 | 0o100_000) => RecordKind::File,
                Some(0o040_000) => RecordKind::Folder,
                Some(_) => RecordKind::Special,
            }
        };
        builder.add(Member {
            name: file.name().to_owned(),
            not_unicode: false,
            kind: Some(kind),
            size: file.size(),
            packed: None,
            modified: file
                .has_last_modified_date
                .then(|| unix_seconds(file.last_modified_date()))
                .flatten(),
            mode: mode.map(|m| m & 0o7777),
            crc32: (file.has_crc && file.has_stream()).then_some(file.crc as u32),
            link_target: None,
            locator: Locator::SevenZ(i as u32),
            flags,
            note: None,
        })?;
    }
    if encrypted_header {
        builder.note(format!("{container} hides its names behind its password."));
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8)) as u32;
    Ok((
        builder.finish(),
        SevenZBackend {
            archive: Arc::new(archive),
            source,
            password,
            solid,
            threads,
        },
    ))
}

impl SevenZBackend {
    /// Whether members are read with random access: not in a solid one.
    pub(crate) fn random_read(&self) -> bool {
        !self.solid
    }

    /// The password the archive was opened with, for its locked members.
    pub(crate) fn password(&self) -> Option<&Secret> {
        self.password.as_ref()
    }

    /// The bytes decoded and set aside before a member in its block.
    pub(crate) fn ahead(&self, file: usize) -> u64 {
        let map = &self.archive.stream_map;
        let Some(Some(block)) = map.file_block_index.get(file) else {
            return 0;
        };
        let first = map
            .block_first_file_index
            .get(*block)
            .copied()
            .unwrap_or(file);
        self.archive.files[first..file.min(self.archive.files.len())]
            .iter()
            .map(sevenz_rust2::ArchiveEntry::size)
            .sum()
    }

    /// Reads one member on a thread of its own: its block decoded from the
    /// start, the members before it set aside, then the member itself.
    pub(crate) fn read(
        &self,
        file: usize,
        password: Option<Secret>,
        cancel: CancelToken,
    ) -> Result<PipeReader, VfsError> {
        let archive = self.archive.clone();
        let mut source = self.source.clone();
        let threads = self.threads;
        let Some(entry) = archive.files.get(file) else {
            return Err(VfsError::NotFound);
        };
        let Some(Some(block)) = archive.stream_map.file_block_index.get(file).copied() else {
            // No stream: an empty file.
            if entry.size() > 0 {
                return Err(damaged("the member's data is missing"));
            }
            return pipe::spawn(cancel, |feed| {
                feed.started();
                Ok(())
            });
        };
        let with_password = password.is_some();
        let set_aside_cancel = cancel.clone();
        let first = archive
            .stream_map
            .block_first_file_index
            .get(block)
            .copied()
            .unwrap_or(file);
        pipe::spawn(cancel, move |feed| {
            let crate_password = password
                .as_ref()
                .map_or_else(Password::empty, |p| Password::new(p.expose()));
            let decoder = BlockDecoder::new(threads, block, &archive, &crate_password, &mut source);
            let mut failure = None;
            // The block hands its members over in file order, from its
            // first; the member wanted is the one at its own number.
            let mut at = first;
            let result = decoder.for_each_entries(&mut |_entry, reader| {
                let this = at;
                at += 1;
                if this == file {
                    feed.pump(reader);
                    return Ok(false);
                }
                if let Err(error) = set_aside(reader, &set_aside_cancel) {
                    failure = Some(error);
                    return Ok(false);
                }
                Ok(true)
            });
            if let Some(error) = failure {
                return Err(error);
            }
            result
                .map(|_| ())
                .map_err(|e| sevenz_error(e, with_password))
        })
    }

    /// Visits every member in archive order, one pass per block: what an
    /// unpack runner uses. `each` gets the file number and a reader of its
    /// data, and returns whether to go on.
    pub(crate) fn visit(
        &self,
        password: Option<&Secret>,
        cancel: &CancelToken,
        each: &mut dyn FnMut(usize, &mut dyn Read) -> Result<bool, VfsError>,
    ) -> Result<(), VfsError> {
        let crate_password = password.map_or_else(Password::empty, |p| Password::new(p.expose()));
        let archive = &self.archive;
        let mut source = self.source.clone();
        let mut failure = None;
        for block in 0..archive.blocks.len() {
            cancel.check()?;
            let mut at = archive
                .stream_map
                .block_first_file_index
                .get(block)
                .copied()
                .unwrap_or(0);
            let decoder =
                BlockDecoder::new(self.threads, block, archive, &crate_password, &mut source);
            let go_on = decoder
                .for_each_entries(&mut |_entry, reader| {
                    let i = at;
                    at += 1;
                    let go_on = match each(i, reader) {
                        Ok(go_on) => go_on,
                        Err(error) => {
                            failure = Some(error);
                            return Ok(false);
                        }
                    };
                    // What `each` left unread is set aside, or the next
                    // member would start in the wrong place.
                    if let Err(error) = set_aside(reader, cancel) {
                        failure = Some(error);
                        return Ok(false);
                    }
                    Ok(go_on)
                })
                .map_err(|e| sevenz_error(e, password.is_some()))?;
            if let Some(error) = failure.take() {
                return Err(error);
            }
            if !go_on {
                return Ok(());
            }
        }
        for (i, file) in archive.files.iter().enumerate() {
            if archive
                .stream_map
                .file_block_index
                .get(i)
                .copied()
                .flatten()
                .is_none()
                && !file.is_anti_item()
                && !each(i, &mut io::empty())?
            {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Decodes a member and throws its bytes away, checking the cancel flag
/// every 64 KiB.
fn set_aside(reader: &mut dyn Read, cancel: &CancelToken) -> Result<(), VfsError> {
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        cancel.check()?;
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(crate::guard::from_io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coders_say_their_memory() {
        assert_eq!(coder_memory(ID_LZMA, &[0x5D, 0, 0, 0, 1]), Some(1 << 24));
        assert_eq!(
            coder_memory(ID_LZMA, &[0x5D, 0xFF, 0xFF, 0xFF, 0xFF]),
            Some(u64::from(u32::MAX))
        );
        assert_eq!(coder_memory(ID_LZMA2, &[40]), Some(u64::from(u32::MAX)));
        assert_eq!(coder_memory(ID_LZMA2, &[22]), Some(8 << 20));
        assert_eq!(coder_memory(ID_PPMD, &[6, 0, 0, 0, 1]), Some(1 << 24));
        assert_eq!(coder_memory(ID_AES, &[1, 2, 3]), None);
        assert_eq!(coder_memory(ID_LZMA, &[0x5D]), None, "too short");
        assert_eq!(
            unix_seconds(sevenz_rust2::NtTime::from(116_444_736_000_000_000)),
            Some(0)
        );
    }

    #[test]
    fn the_number_reader_follows_the_format() {
        let mut b = Bytes {
            data: &[0x05, 0x80, 0x80, 0xC0, 0x01, 0x02],
            at: 0,
        };
        assert_eq!(b.number(), Some(5));
        assert_eq!(b.number(), Some(0x80));
        assert_eq!(b.number(), Some(0x0201));
        assert_eq!(b.number(), None);
    }
}
