//! A fetcher for tests: it serves fixtures by address and records every
//! request, so a test can fail if anything is fetched before the owner's
//! yes (R22 section 6.1, textweaver's `FakeFetcher`). Public, so the app's
//! tests can use it too.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};
use std::sync::{Mutex, PoisonError};

use crate::fetch::{FetchError, Fetched, Fetcher, Request};

/// What the fake answers for one address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeAnswer {
    /// These bytes, ranges honored.
    Bytes(Vec<u8>),
    /// These bytes, whole, whatever range was asked.
    IgnoresRange(Vec<u8>),
    /// These bytes, cut with "the connection was lost" after this many.
    CutAfter(Vec<u8>, usize),
    /// This error from `open`.
    Fail(FetchError),
}

/// One request the fake saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeRequest {
    /// The address.
    pub address: String,
    /// The byte asked from.
    pub from: u64,
    /// The `If-Range` validator sent.
    pub if_range: Option<String>,
    /// Whether a token came with it (never its text).
    pub token: bool,
}

/// Serves fixtures and records requests. An address it has no answer for
/// is "not there".
#[derive(Debug, Default)]
pub struct FakeFetcher {
    answers: Mutex<HashMap<String, FakeAnswer>>,
    requests: Mutex<Vec<FakeRequest>>,
}

impl FakeFetcher {
    /// A fake with no files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `address` with `answer` from now on.
    pub fn serve(&self, address: impl Into<String>, answer: FakeAnswer) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(address.into(), answer);
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<FakeRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A reader that gives its bytes, then an error.
struct Cut {
    bytes: Cursor<Vec<u8>>,
}

impl Read for Cut {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.bytes.read(buf)? {
            0 => Err(FetchError::Lost.into_io()),
            n => Ok(n),
        }
    }
}

impl Fetcher for FakeFetcher {
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FakeRequest {
                address: request.address.to_owned(),
                from: request.from,
                if_range: request.if_range.map(str::to_owned),
                token: request.token.is_some(),
            });
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(request.address)
            .cloned()
            .ok_or(FetchError::Missing)?;
        let ranged = |bytes: Vec<u8>| {
            let from = request.from;
            if from > 0 && from < bytes.len() as u64 {
                (from, bytes[from as usize..].to_vec())
            } else {
                (0, bytes)
            }
        };
        let (start, reader): (u64, Box<dyn Read + Send>) = match answer {
            FakeAnswer::Fail(error) => return Err(error),
            FakeAnswer::Bytes(bytes) => {
                let (start, rest) = ranged(bytes);
                (start, Box::new(Cursor::new(rest)))
            }
            FakeAnswer::IgnoresRange(bytes) => (0, Box::new(Cursor::new(bytes))),
            FakeAnswer::CutAfter(bytes, keep) => {
                let (start, rest) = ranged(bytes);
                let keep = keep.saturating_sub(start as usize).min(rest.len());
                (
                    start,
                    Box::new(Cut {
                        bytes: Cursor::new(rest[..keep].to_vec()),
                    }),
                )
            }
        };
        Ok(Fetched {
            reader,
            start,
            validator: None,
        })
    }
}
