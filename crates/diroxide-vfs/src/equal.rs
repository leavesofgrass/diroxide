//! "Are these two streams equal?" with an early exit: both are read in
//! blocks side by side, and the reading stops at the first difference
//! (R18: early exit, not hashes, for a pair; hashes belong to
//! many-to-many work). One block compare for every caller: the file
//! compare in `diroxide-diff` (which re-exports it), the folder compare
//! of a synchronize, and the duplicate finder's byte check.

use std::io::{ErrorKind, Read};

use crate::cancel::CancelToken;
use crate::error::VfsError;

/// How much of each stream one step reads by default: 64 KiB.
pub const BLOCK_BYTES: usize = 64 * 1024;

/// Whether two byte streams are equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Equality {
    /// The same bytes, this many of them.
    Same {
        /// The length of each stream.
        bytes: u64,
    },
    /// The streams differ, first at this offset, counted from 0: the
    /// first byte that differs, or the end of the shorter stream.
    Different {
        /// Where they first differ.
        at: u64,
    },
}

impl Equality {
    /// Whether the streams were equal.
    pub const fn is_same(self) -> bool {
        matches!(self, Equality::Same { .. })
    }
}

/// Which of the two streams a read error came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stream {
    /// The first stream given.
    First,
    /// The second stream given.
    Second,
}

/// Why a block compare stopped before its answer.
#[derive(Debug, PartialEq, Eq)]
pub enum EqualStop {
    /// The caller's step said stop ([`VfsError::Cancelled`] for a
    /// cancel), or the token was cancelled between blocks.
    Stopped(VfsError),
    /// A stream could not be read.
    Read(Stream, VfsError),
}

impl From<EqualStop> for VfsError {
    fn from(stop: EqualStop) -> Self {
        match stop {
            EqualStop::Stopped(error) | EqualStop::Read(_, error) => error,
        }
    }
}

/// Fills `buffer` from `reader` as far as it can: short only at the end.
/// An interrupted read is tried again.
pub fn fill(reader: &mut dyn Read, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Reads two streams side by side in blocks of `block` bytes (at least
/// one) and says whether they are equal, stopping at the first
/// difference. Before each block `cancel` is checked; after each block is
/// read, `step` hears the bytes read from both streams so far, and may
/// stop the compare (a pause waits inside it; a stop returns
/// [`EqualStop::Stopped`]).
pub fn equal_with(
    first: &mut dyn Read,
    second: &mut dyn Read,
    block: usize,
    cancel: &CancelToken,
    step: &mut dyn FnMut(u64) -> Result<(), VfsError>,
) -> Result<Equality, EqualStop> {
    let block = block.max(1);
    let mut one = vec![0u8; block];
    let mut other = vec![0u8; block];
    let mut offset = 0u64;
    let mut read = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(EqualStop::Stopped(VfsError::Cancelled));
        }
        let got_one =
            fill(first, &mut one).map_err(|e| EqualStop::Read(Stream::First, e.into()))?;
        let got_other =
            fill(second, &mut other).map_err(|e| EqualStop::Read(Stream::Second, e.into()))?;
        if got_one + got_other > 0 {
            read += (got_one + got_other) as u64;
            step(read).map_err(EqualStop::Stopped)?;
        }
        let common = got_one.min(got_other);
        if let Some(at) = one[..common]
            .iter()
            .zip(&other[..common])
            .position(|(a, b)| a != b)
        {
            return Ok(Equality::Different {
                at: offset + at as u64,
            });
        }
        if got_one != got_other {
            return Ok(Equality::Different {
                at: offset + common as u64,
            });
        }
        offset += common as u64;
        if got_one < block {
            return Ok(Equality::Same { bytes: offset });
        }
    }
}

/// [`equal_with`] in [`BLOCK_BYTES`] blocks with no step: the plain
/// question.
pub fn equal(
    first: &mut dyn Read,
    second: &mut dyn Read,
    cancel: &CancelToken,
) -> Result<Equality, EqualStop> {
    equal_with(first, second, BLOCK_BYTES, cancel, &mut |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn check(a: &[u8], b: &[u8]) -> Equality {
        equal(
            &mut Cursor::new(a.to_vec()),
            &mut Cursor::new(b.to_vec()),
            &CancelToken::new(),
        )
        .unwrap()
    }

    /// A reader that fails if read past `limit`, to prove the early exit.
    struct Fence {
        data: Cursor<Vec<u8>>,
        limit: u64,
    }

    impl Read for Fence {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.data.position() >= self.limit {
                return Err(std::io::Error::other("read past the first difference"));
            }
            self.data.read(buffer)
        }
    }

    #[test]
    fn equal_and_different_streams() {
        assert_eq!(check(b"", b""), Equality::Same { bytes: 0 });
        assert_eq!(check(b"abc", b"abc"), Equality::Same { bytes: 3 });
        assert_eq!(check(b"abc", b"abd"), Equality::Different { at: 2 });
        assert_eq!(check(b"abc", b"abcd"), Equality::Different { at: 3 });
        let exact = vec![1u8; BLOCK_BYTES];
        assert!(check(&exact, &exact).is_same());
    }

    #[test]
    fn the_step_hears_both_sides_and_can_stop() {
        let data = vec![3u8; 10];
        let mut heard = Vec::new();
        let got = equal_with(
            &mut Cursor::new(data.clone()),
            &mut Cursor::new(data.clone()),
            4,
            &CancelToken::new(),
            &mut |done| {
                heard.push(done);
                Ok(())
            },
        );
        assert_eq!(got, Ok(Equality::Same { bytes: 10 }));
        assert_eq!(heard, [8, 16, 20]);
        let stopped = equal_with(
            &mut Cursor::new(data.clone()),
            &mut Cursor::new(data),
            4,
            &CancelToken::new(),
            &mut |_| Err(VfsError::Cancelled),
        );
        assert_eq!(stopped, Err(EqualStop::Stopped(VfsError::Cancelled)));
    }

    #[test]
    fn a_read_error_names_its_stream_and_a_cancel_stops() {
        let failing = equal(
            &mut Cursor::new(vec![0u8; 4]),
            &mut Fence {
                data: Cursor::new(vec![0u8; 4]),
                limit: 0,
            },
            &CancelToken::new(),
        );
        assert!(matches!(failing, Err(EqualStop::Read(Stream::Second, _))));
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            equal(&mut Cursor::new(vec![1]), &mut Cursor::new(vec![1]), &cancel),
            Err(EqualStop::Stopped(VfsError::Cancelled))
        );
    }
}
