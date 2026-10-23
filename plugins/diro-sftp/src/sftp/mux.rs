//! The request multiplexer (R24 section 2.5; answer 88): many requests in
//! flight on one SFTP session, their answers matched by id.
//!
//! - **A writer thread** with a bounded queue sends the packets in the
//!   order they were made; a full queue makes the caller wait, so memory
//!   stays bounded.
//! - **A reader thread** reads answers with the codec's bound and hands
//!   each to the request that waits for its id. Replies may come in any
//!   order. An answer for an id nobody waits for (a server that answers
//!   twice, a request already cancelled) is dropped and counted, never
//!   given to another request.
//! - **Ids** count up and wrap, skipping any still waiting.
//! - **Every call has a time limit and may be cancelled**; the late answer
//!   is then dropped. When the stream ends or a packet cannot be read the
//!   session is lost: every waiting call and every later one gets
//!   [`SftpError::Lost`] with the reason.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::codec::{CodecError, Request, Response, frame};
use crate::rpc::Cancel;

/// How many packets may wait for the writer.
pub const WRITE_QUEUE: usize = 256;

/// Why an SFTP call did not answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SftpError {
    /// The session ended; the reason in words.
    #[error("the connection was lost: {0}")]
    Lost(String),
    /// No answer in time.
    #[error("the server did not answer in time")]
    TimedOut,
    /// The call was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The server printed text before the protocol began: a shell startup
    /// file that writes to its output. The first bytes, cleaned.
    #[error("the server printed text first")]
    ShellChatter(String),
    /// The server's first answer was not a version, or a packet could not
    /// be read.
    #[error("the server did not speak SFTP: {0}")]
    Protocol(String),
    /// The server answered with a status other than OK where an answer was
    /// wanted.
    #[error("the server refused: status {code}")]
    Status {
        /// The status code.
        code: u32,
        /// The server's words, never shown as they are.
        message: Vec<u8>,
    },
}

#[derive(Default)]
struct Shared {
    pending: Mutex<HashMap<u32, Sender<Response>>>,
    lost: Mutex<Option<String>>,
    is_lost: AtomicBool,
    strays: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl Shared {
    fn lose(&self, why: &str) {
        let mut lost = self.lost.lock().unwrap_or_else(PoisonError::into_inner);
        if lost.is_none() {
            *lost = Some(why.to_owned());
        }
        self.is_lost.store(true, Ordering::SeqCst);
        // Dropping the senders wakes every waiting call.
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    fn lost_error(&self) -> SftpError {
        SftpError::Lost(
            self.lost
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_else(|| "the stream ended".to_owned()),
        )
    }
}

/// A request sent and not yet answered. Dropped unanswered, its late
/// answer is dropped and counted.
pub struct Ticket {
    id: u32,
    rx: Receiver<Response>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ticket").field("id", &self.id).finish()
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Ticket {
    /// The request's id.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Waits for the answer, up to `timeout`, stopping early on a cancel.
    pub fn wait(self, cancel: Option<&Cancel>, timeout: Duration) -> Result<Response, SftpError> {
        const STEP: Duration = Duration::from_millis(25);
        let deadline = Instant::now() + timeout;
        let shared = &self.shared;
        loop {
            if cancel.is_some_and(Cancel::is_cancelled) {
                break Err(SftpError::Cancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                break Err(SftpError::TimedOut);
            }
            match self.rx.recv_timeout(STEP.min(deadline - now)) {
                Ok(response) => break Ok(response),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break Err(shared.lost_error()),
            }
        }
    }
}

/// The multiplexer of one session.
pub struct Mux {
    shared: Arc<Shared>,
    queue: SyncSender<Vec<u8>>,
    next: AtomicU32,
}

impl std::fmt::Debug for Mux {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mux")
            .field("in_flight", &self.in_flight())
            .field("lost", &self.is_lost())
            .finish_non_exhaustive()
    }
}

impl Mux {
    /// Starts the threads over a session's streams; `max_packet` bounds
    /// every answer read.
    pub fn start(
        input: impl Read + Send + 'static,
        output: impl Write + Send + 'static,
        max_packet: usize,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let (queue, packets) = mpsc::sync_channel::<Vec<u8>>(WRITE_QUEUE);
        {
            let shared = Arc::clone(&shared);
            let mut output = output;
            let _ = thread::Builder::new()
                .name("sftp-writer".to_owned())
                .spawn(move || {
                    for packet in packets {
                        if output
                            .write_all(&packet)
                            .and_then(|()| output.flush())
                            .is_err()
                        {
                            shared.lose("the stream to the server closed");
                            return;
                        }
                    }
                });
        }
        {
            let shared = Arc::clone(&shared);
            let _ = thread::Builder::new()
                .name("sftp-reader".to_owned())
                .spawn(move || read_loop(input, &shared, max_packet));
        }
        Self {
            shared,
            queue,
            next: AtomicU32::new(1),
        }
    }

    /// Whether the session is lost.
    pub fn is_lost(&self) -> bool {
        self.shared.is_lost.load(Ordering::SeqCst)
    }

    /// How many answers were dropped because nobody waited for their id.
    pub fn strays(&self) -> usize {
        self.shared.strays.load(Ordering::SeqCst)
    }

    /// How many requests wait for answers now.
    pub fn in_flight(&self) -> usize {
        self.shared.in_flight.load(Ordering::SeqCst)
    }

    /// The most that ever waited at once.
    pub fn max_in_flight(&self) -> usize {
        self.shared.max_in_flight.load(Ordering::SeqCst)
    }

    fn take_id(&self, pending: &HashMap<u32, Sender<Response>>) -> u32 {
        loop {
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            if id != 0 && !pending.contains_key(&id) {
                return id;
            }
        }
    }

    /// Sends a request made for a fresh id; the ticket waits for its
    /// answer.
    pub fn send(&self, make: impl FnOnce(u32) -> Request) -> Result<Ticket, SftpError> {
        if self.is_lost() {
            return Err(self.shared.lost_error());
        }
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let id = self.take_id(&pending);
            pending.insert(id, tx);
            id
        };
        let now = self.shared.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.max_in_flight.fetch_max(now, Ordering::SeqCst);
        let ticket = Ticket {
            id,
            rx,
            shared: Arc::clone(&self.shared),
        };
        if self.queue.send(make(id).encode()).is_err() {
            self.shared.lose("the stream to the server closed");
        }
        Ok(ticket)
    }

    /// Sends a request and waits for its answer.
    pub fn call(
        &self,
        make: impl FnOnce(u32) -> Request,
        cancel: Option<&Cancel>,
        timeout: Duration,
    ) -> Result<Response, SftpError> {
        self.send(make)?.wait(cancel, timeout)
    }
}

fn read_loop(mut input: impl Read, shared: &Shared, max_packet: usize) {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        loop {
            match frame(&buffer, max_packet) {
                Ok(Some((body, used))) => {
                    let decoded = Response::decode(body);
                    buffer.drain(..used);
                    match decoded {
                        Ok(response) => route(shared, response),
                        Err(error) => {
                            shared.lose(&format!("a packet could not be read: {error}"));
                            return;
                        }
                    }
                }
                Ok(None) => break,
                Err(CodecError::Length(len)) => {
                    shared.lose(&format!("a packet of {len} bytes is too long"));
                    return;
                }
                Err(error) => {
                    shared.lose(&error.to_string());
                    return;
                }
            }
        }
        match input.read(&mut chunk) {
            Ok(0) => {
                let why = if buffer.is_empty() {
                    "the server closed the session"
                } else {
                    "the session ended in the middle of a packet"
                };
                shared.lose(why);
                return;
            }
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(error) => {
                shared.lose(&error.to_string());
                return;
            }
        }
    }
}

fn route(shared: &Shared, response: Response) {
    let Some(id) = response.id() else {
        shared.strays.fetch_add(1, Ordering::SeqCst);
        return;
    };
    let waiting = shared
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    match waiting {
        Some(tx) => {
            let _ = tx.send(response);
        }
        None => {
            shared.strays.fetch_add(1, Ordering::SeqCst);
        }
    }
}
