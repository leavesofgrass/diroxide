//! A decoder that borrows its archive, turned into a reader the caller
//! owns: the decoder runs on a thread of its own and sends its output in
//! 64 KiB pieces through a channel of four, so at most a quarter of a MiB
//! waits. The zip and 7z crates hand out member readers that borrow the
//! archive; a provider's `read` must return one that stands alone.
//!
//! The thread stops when the reader is dropped (its next send fails) or
//! the cancel token is set. An error before the first byte (a wrong
//! password, a method not supported) is returned when the pipe opens, not
//! at the first read.

use std::fmt;
use std::io::{self, Read};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use diroxide_vfs::{CancelToken, VfsError};

use crate::guard::{from_io, to_io};

/// How much one message carries.
pub const PIECE: usize = 64 << 10;

enum Message {
    Started,
    Data(Vec<u8>),
    Failed(io::Error),
}

/// The sending end, for the decoder's thread.
pub struct Feed {
    send: SyncSender<Message>,
    cancel: CancelToken,
    started: bool,
}

impl Feed {
    /// Says the decoder is ready; the pipe opens.
    pub fn started(&mut self) -> bool {
        self.started = true;
        self.send.send(Message::Started).is_ok()
    }

    /// Copies a reader into the pipe, piece by piece, until its end, an
    /// error, a cancel, or a dropped reader. Returns whether the reader
    /// wants more.
    pub fn pump(&mut self, reader: &mut dyn Read) -> bool {
        if !self.started && !self.started() {
            return false;
        }
        loop {
            if self.cancel.is_cancelled() {
                let _ = self.send.send(Message::Failed(to_io(VfsError::Cancelled)));
                return false;
            }
            let mut piece = vec![0u8; PIECE];
            match reader.read(&mut piece) {
                Ok(0) => return true,
                Ok(n) => {
                    piece.truncate(n);
                    if self.send.send(Message::Data(piece)).is_err() {
                        return false;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    let _ = self.send.send(Message::Failed(e));
                    return false;
                }
            }
        }
    }

    /// Ends the pipe with an error.
    pub fn fail(self, error: VfsError) {
        let _ = self.send.send(Message::Failed(to_io(error)));
    }
}

/// The receiving end: an owned reader.
pub struct PipeReader {
    receive: Receiver<Message>,
    piece: Vec<u8>,
    at: usize,
    done: bool,
}

impl fmt::Debug for PipeReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PipeReader")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.piece.len() {
            if self.done {
                return Ok(0);
            }
            match self.receive.recv() {
                Ok(Message::Data(piece)) => {
                    self.piece = piece;
                    self.at = 0;
                }
                Ok(Message::Started) => {}
                Ok(Message::Failed(error)) => {
                    self.done = true;
                    return Err(error);
                }
                Err(_) => {
                    self.done = true;
                    return Ok(0);
                }
            }
        }
        let n = (self.piece.len() - self.at).min(out.len());
        out[..n].copy_from_slice(&self.piece[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Runs `decode` on a thread with the sending end and waits for it to
/// start: the reader, or the error it failed with first. `decode` calls
/// [`Feed::started`] (or [`Feed::pump`], which starts the pipe) once the
/// decoder is ready, and returns its error, if any, which the reader
/// gives at its end.
pub fn spawn(
    cancel: CancelToken,
    decode: impl FnOnce(&mut Feed) -> Result<(), VfsError> + Send + 'static,
) -> Result<PipeReader, VfsError> {
    let (send, receive) = sync_channel(4);
    let builder = thread::Builder::new().name("diroxide-archive-read".to_owned());
    builder
        .spawn(move || {
            let mut feed = Feed {
                send,
                cancel,
                started: false,
            };
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&mut feed)));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => feed.fail(error),
                Err(_) => feed.fail(VfsError::Corrupt {
                    what: "the archive made its reader fail".to_owned(),
                }),
            }
        })
        .map_err(VfsError::from)?;
    let mut reader = PipeReader {
        receive,
        piece: Vec::new(),
        at: 0,
        done: false,
    };
    match reader.receive.recv() {
        Ok(Message::Started) => Ok(reader),
        Ok(Message::Data(piece)) => {
            reader.piece = piece;
            Ok(reader)
        }
        Ok(Message::Failed(error)) => Err(from_io(error)),
        Err(_) => {
            reader.done = true;
            Ok(reader)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_carries_data_and_errors() {
        let data: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
        let copy = data.clone();
        let mut reader = spawn(CancelToken::new(), move |feed| {
            feed.pump(&mut copy.as_slice());
            Ok(())
        })
        .unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, data);
        let failed = spawn(CancelToken::new(), |_| Err(VfsError::WrongPassword)).unwrap_err();
        assert_eq!(failed, VfsError::WrongPassword);
        let mut late = spawn(CancelToken::new(), |feed| {
            feed.started();
            Err(VfsError::Corrupt {
                what: "late".to_owned(),
            })
        })
        .unwrap();
        let error = late.read_to_end(&mut Vec::new()).unwrap_err();
        assert_eq!(from_io(error).to_string(), "it is damaged: late");
    }

    #[test]
    fn a_panic_in_the_decoder_is_an_error() {
        let failed = spawn(CancelToken::new(), |_| panic!("a crate bug")).unwrap_err();
        assert!(failed.to_string().contains("made its reader fail"));
    }

    #[test]
    fn dropping_the_reader_stops_the_thread() {
        let (done_send, done) = std::sync::mpsc::channel();
        let reader = spawn(CancelToken::new(), move |feed| {
            let mut endless = io::repeat(1);
            let wanted = feed.pump(&mut endless);
            let _ = done_send.send(wanted);
            Ok(())
        })
        .unwrap();
        drop(reader);
        assert_eq!(
            done.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(false)
        );
    }
}
