//! "Are these two files equal?" with an early exit: both are read in
//! blocks side by side, and the reading stops at the first difference
//! (R18: early exit, not hashes, for a pair; hashes belong to many-to-many
//! work). Compare uses it, and the duplicate finder of a later alpha will
//! use it for its final byte comparison.

use std::io::{ErrorKind, Read};

use diroxide_vfs::{CancelToken, VfsError};

use crate::error::DiffError;

/// How much of each file one step reads: 64 KiB.
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

/// Fills `buffer` from `reader` as far as it can: short only at the end.
pub(crate) fn fill(reader: &mut dyn Read, buffer: &mut [u8]) -> Result<usize, DiffError> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(DiffError::Read(VfsError::from(error))),
        }
    }
    Ok(filled)
}

/// Reads two streams side by side and says whether they are equal,
/// stopping at the first difference and between blocks when `cancel` is
/// set ([`DiffError::Cancelled`]).
pub fn equal(
    before: &mut dyn Read,
    after: &mut dyn Read,
    cancel: &CancelToken,
) -> Result<Equality, DiffError> {
    let mut one = vec![0u8; BLOCK_BYTES];
    let mut other = vec![0u8; BLOCK_BYTES];
    let mut offset = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(DiffError::Cancelled);
        }
        let got_one = fill(before, &mut one)?;
        let got_other = fill(after, &mut other)?;
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
        if got_one < BLOCK_BYTES {
            return Ok(Equality::Same { bytes: offset });
        }
    }
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

    /// A reader that hands out one byte at a time, as a slow source may.
    struct Trickle(Cursor<Vec<u8>>);

    impl Read for Trickle {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let end = buffer.len().min(1);
            self.0.read(&mut buffer[..end])
        }
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
    fn equal_streams_are_the_same() {
        assert_eq!(check(b"", b""), Equality::Same { bytes: 0 });
        assert_eq!(check(b"abc", b"abc"), Equality::Same { bytes: 3 });
        let big = vec![7u8; BLOCK_BYTES * 2 + 5];
        assert_eq!(
            check(&big, &big),
            Equality::Same {
                bytes: big.len() as u64
            }
        );
        let exact = vec![1u8; BLOCK_BYTES];
        assert!(check(&exact, &exact).is_same());
    }

    #[test]
    fn the_first_difference_is_found() {
        assert_eq!(check(b"abc", b"abd"), Equality::Different { at: 2 });
        assert_eq!(check(b"abc", b"abcd"), Equality::Different { at: 3 });
        assert_eq!(check(b"", b"x"), Equality::Different { at: 0 });
        let mut late = vec![0u8; BLOCK_BYTES + 10];
        let mut other = late.clone();
        other[BLOCK_BYTES + 3] = 1;
        assert_eq!(
            check(&late, &other),
            Equality::Different {
                at: BLOCK_BYTES as u64 + 3
            }
        );
        late.truncate(BLOCK_BYTES);
        assert_eq!(
            check(&late, &other),
            Equality::Different {
                at: BLOCK_BYTES as u64
            }
        );
    }

    #[test]
    fn short_reads_do_not_fool_it() {
        let data = b"hello, world".to_vec();
        let got = equal(
            &mut Trickle(Cursor::new(data.clone())),
            &mut Cursor::new(data),
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(got, Equality::Same { bytes: 12 });
    }

    #[test]
    fn it_stops_at_the_first_differing_block() {
        let size = BLOCK_BYTES * 4;
        let one = vec![0u8; size];
        let mut other = vec![0u8; size];
        other[10] = 1;
        let got = equal(
            &mut Fence {
                data: Cursor::new(one),
                limit: BLOCK_BYTES as u64,
            },
            &mut Cursor::new(other),
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(got, Equality::Different { at: 10 });
    }

    #[test]
    fn a_cancel_is_honored_and_read_errors_say_so() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            equal(
                &mut Cursor::new(vec![1u8]),
                &mut Cursor::new(vec![1u8]),
                &cancel
            ),
            Err(DiffError::Cancelled)
        );
        let failing = equal(
            &mut Fence {
                data: Cursor::new(vec![0u8; 4]),
                limit: 0,
            },
            &mut Cursor::new(vec![0u8; 4]),
            &CancelToken::new(),
        );
        assert!(matches!(failing, Err(DiffError::Read(_))));
    }
}
