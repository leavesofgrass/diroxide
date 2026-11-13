//! The full hash behind the duplicate finder, as a trait the caller
//! fills. [`Blake3Hasher`] is the default, over the same `blake3` crate
//! `diroxide-meta`'s Checksum command uses, so a duplicate's hash and
//! its checksum agree. [`Sha256Hasher`] is this crate's other, over the
//! `sha2` it already has.

use std::io::{ErrorKind, Read};

use diroxide_vfs::{CancelToken, VfsError};

/// The step of a read: 1 MiB.
pub const STEP: usize = 1024 * 1024;

/// A hash of a whole stream, the deciding step of the duplicate finder.
pub trait ContentHasher {
    /// The hash's name as said: "BLAKE3", "SHA-256".
    fn name(&self) -> &'static str;

    /// Hashes everything `reader` gives, checking `cancel` between steps
    /// and telling `on_step` the bytes read so far after each.
    fn hash(
        &self,
        reader: &mut dyn Read,
        cancel: &CancelToken,
        on_step: &mut dyn FnMut(u64),
    ) -> Result<Vec<u8>, VfsError>;
}

/// BLAKE3, read in [`STEP`] steps: the duplicate finder's default.
#[derive(Clone, Copy, Debug, Default)]
pub struct Blake3Hasher;

impl ContentHasher for Blake3Hasher {
    fn name(&self) -> &'static str {
        "BLAKE3"
    }

    fn hash(
        &self,
        reader: &mut dyn Read,
        cancel: &CancelToken,
        on_step: &mut dyn FnMut(u64),
    ) -> Result<Vec<u8>, VfsError> {
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; STEP];
        let mut done = 0u64;
        loop {
            cancel.check()?;
            let filled = fill(reader, &mut buffer, cancel)?;
            if filled == 0 {
                break;
            }
            hasher.update(&buffer[..filled]);
            done += filled as u64;
            on_step(done);
            if filled < buffer.len() {
                break;
            }
        }
        Ok(hasher.finalize().as_bytes().to_vec())
    }
}

/// SHA-256 over `sha2`, read in [`STEP`] steps.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sha256Hasher;

impl ContentHasher for Sha256Hasher {
    fn name(&self) -> &'static str {
        "SHA-256"
    }

    fn hash(
        &self,
        reader: &mut dyn Read,
        cancel: &CancelToken,
        on_step: &mut dyn FnMut(u64),
    ) -> Result<Vec<u8>, VfsError> {
        use sha2::Digest as _;
        let mut hasher = sha2::Sha256::new();
        let mut buffer = vec![0u8; STEP];
        let mut done = 0u64;
        loop {
            cancel.check()?;
            let filled = fill(reader, &mut buffer, cancel)?;
            if filled == 0 {
                break;
            }
            hasher.update(&buffer[..filled]);
            done += filled as u64;
            on_step(done);
            if filled < buffer.len() {
                break;
            }
        }
        Ok(hasher.finalize().to_vec())
    }
}

/// Reads until `buffer` is full or the stream ends; the count read.
pub(super) fn fill(
    reader: &mut dyn Read,
    buffer: &mut [u8],
    cancel: &CancelToken,
) -> Result<usize, VfsError> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => cancel.check()?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(filled)
}
