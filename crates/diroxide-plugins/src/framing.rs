//! Framing: one message per line, read with a bound and written by a
//! thread of its own.
//!
//! The reader ([`LineReader`]) reads bytes up to `\n` and enforces the
//! line limit while reading, so a plugin that prints a gigabyte without a
//! line break costs the limit, not the memory; the rest of that line is
//! skipped and the next line is read as usual. Each line is then sorted
//! ([`classify`]) into a message, stray text, a message that is not valid
//! UTF-8, or a line over the limit. A stray line is never fatal: bytes in
//! the console's code page are logged as lossy text and the conversation
//! goes on.
//!
//! The writer ([`LineWriter`]) is a thread with a bounded queue, so a
//! plugin that stops reading fills the queue, not the caller's thread:
//! a send that finds the queue full fails at once with
//! [`SendError::QueueFull`], which the router answers with
//! `limit_exceeded`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use crate::protocol::MAX_LINE_BYTES;

/// How much of a stray line is kept for the log, in characters.
pub const STRAY_PREVIEW_CHARS: usize = 200;

/// How many lines may wait for the writer thread.
pub const WRITE_QUEUE: usize = 256;

/// One line as read, before it is sorted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLine {
    /// The line's bytes without the `\n`, at most the limit.
    pub bytes: Vec<u8>,
    /// The line's whole length in bytes, which is more than `bytes` holds
    /// when the line was over the limit.
    pub length: u64,
    /// Whether the line ended with `\n` (the last line of a stream may
    /// not).
    pub terminated: bool,
}

impl RawLine {
    /// Whether the line was longer than the limit.
    pub fn over_limit(&self) -> bool {
        self.length > self.bytes.len() as u64
    }
}

/// What a line turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// A line meant as a message: it starts with `{` (or `[`, a batch,
    /// which is refused) after spaces and is
    /// valid UTF-8. The `\r` of a `\r\n` ending is removed.
    Message(String),
    /// A line that is not meant as a message: logged, counted, never
    /// answered. Lossy, cut to [`STRAY_PREVIEW_CHARS`].
    Stray(String),
    /// A message line that is not valid UTF-8: answered as a
    /// parse error. The text is a lossy preview for the log.
    NotUtf8(String),
    /// A line longer than the limit: refused, and the next line is still
    /// read.
    TooLong {
        /// The line's length in bytes.
        length: u64,
        /// Whether it started as a message.
        message: bool,
    },
    /// An empty line, or only spaces: ignored.
    Blank,
}

/// Reads lines with a bound on each line's length.
#[derive(Debug)]
pub struct LineReader<R> {
    inner: BufReader<R>,
    limit: usize,
}

impl<R: Read> LineReader<R> {
    /// A reader with the default limit of 8 MiB.
    pub fn new(inner: R) -> Self {
        Self::with_limit(inner, MAX_LINE_BYTES)
    }

    /// A reader with another limit (at least one byte).
    pub fn with_limit(inner: R, limit: usize) -> Self {
        Self {
            inner: BufReader::with_capacity(64 * 1024, inner),
            limit: limit.max(1),
        }
    }

    /// The limit in bytes.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Reads the next line's bytes; `None` at the end of the stream.
    /// Never holds more than the limit in memory.
    pub fn read_raw(&mut self) -> io::Result<Option<RawLine>> {
        let mut bytes = Vec::new();
        let mut length: u64 = 0;
        loop {
            let available = match self.inner.fill_buf() {
                Ok(available) => available,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                if length == 0 {
                    return Ok(None);
                }
                return Ok(Some(RawLine {
                    bytes,
                    length,
                    terminated: false,
                }));
            }
            let (chunk, used, done) = match available.iter().position(|&b| b == b'\n') {
                Some(end) => (&available[..end], end + 1, true),
                None => (available, available.len(), false),
            };
            let room = self.limit.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
            length += chunk.len() as u64;
            self.inner.consume(used);
            if done {
                return Ok(Some(RawLine {
                    bytes,
                    length,
                    terminated: true,
                }));
            }
        }
    }

    /// Reads and sorts the next line; `None` at the end of the stream.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        Ok(self.read_raw()?.map(|line| classify(&line)))
    }
}

/// The byte-order mark, tolerated at the start of a line although the
/// protocol says not to send it (Windows PowerShell 5.1 may).
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Sorts a line: a message, stray text, a message that is not UTF-8, a
/// line over the limit, or a blank line.
pub fn classify(line: &RawLine) -> Frame {
    let mut bytes = line.bytes.as_slice();
    if let Some(rest) = bytes.strip_prefix(BOM) {
        bytes = rest;
    }
    if let Some(rest) = bytes.strip_suffix(b"\r") {
        bytes = rest;
    }
    let first = bytes.iter().position(|b| !matches!(b, b' ' | b'\t'));
    let message = first.is_some_and(|i| matches!(bytes[i], b'{' | b'['));
    if line.over_limit() {
        return Frame::TooLong {
            length: line.length,
            message,
        };
    }
    if first.is_none() {
        return Frame::Blank;
    }
    if !message {
        return Frame::Stray(preview(bytes));
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Frame::Message(text.to_owned()),
        Err(_) => Frame::NotUtf8(preview(bytes)),
    }
}

/// Lossy text of at most [`STRAY_PREVIEW_CHARS`] characters, for the log.
pub fn preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    match text.char_indices().nth(STRAY_PREVIEW_CHARS) {
        Some((cut, _)) => text[..cut].to_owned(),
        None => text.into_owned(),
    }
}

/// Why a line could not be queued for writing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// The queue is full: the plugin is not reading.
    #[error("the plugin is not reading its input")]
    QueueFull,
    /// The writer has stopped: the plugin's input is closed.
    #[error("the plugin's input is closed")]
    Closed,
}

/// A thread that writes lines, each followed by `\n` and a flush, from a
/// bounded queue. Dropping it (or [`LineWriter::close`]) lets the thread
/// write what is queued and then close the output.
#[derive(Debug)]
pub struct LineWriter {
    queue: Option<SyncSender<String>>,
    broken: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LineWriter {
    /// Starts the thread over an output, with a queue of [`WRITE_QUEUE`].
    pub fn spawn<W: Write + Send + 'static>(output: W) -> io::Result<Self> {
        Self::with_capacity(output, WRITE_QUEUE)
    }

    /// Starts the thread with another queue length (at least one).
    pub fn with_capacity<W: Write + Send + 'static>(
        mut output: W,
        capacity: usize,
    ) -> io::Result<Self> {
        let (queue, lines) = mpsc::sync_channel::<String>(capacity.max(1));
        let broken = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&broken);
        let thread = thread::Builder::new()
            .name("plugin-writer".to_owned())
            .spawn(move || {
                for line in lines {
                    let written = output
                        .write_all(line.as_bytes())
                        .and_then(|()| output.write_all(b"\n"))
                        .and_then(|()| output.flush());
                    if written.is_err() {
                        flag.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            })?;
        Ok(Self {
            queue: Some(queue),
            broken,
            thread: Some(thread),
        })
    }

    /// Queues one line; the writer adds the line break. Never blocks.
    pub fn send(&self, line: String) -> Result<(), SendError> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(SendError::Closed);
        }
        let queue = self.queue.as_ref().ok_or(SendError::Closed)?;
        queue.try_send(line).map_err(|e| match e {
            TrySendError::Full(_) => SendError::QueueFull,
            TrySendError::Disconnected(_) => SendError::Closed,
        })
    }

    /// Whether a write failed, so the plugin's input is gone.
    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    /// Stops taking lines; the thread writes what is queued and closes the
    /// output. Returns at once; [`LineWriter::join`] waits.
    pub fn close(&mut self) {
        self.queue = None;
    }

    /// Closes and waits for the thread to finish writing.
    pub fn join(mut self) {
        self.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Mutex;

    use proptest::prelude::*;

    use super::*;

    fn frames(input: &[u8], limit: usize) -> Vec<Frame> {
        let mut reader = LineReader::with_limit(Cursor::new(input.to_vec()), limit);
        let mut out = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            out.push(frame);
        }
        out
    }

    #[test]
    fn messages_stray_lines_and_crlf_are_told_apart() {
        let input = b"{\"a\":1}\r\nhello there\n\n  {\"b\":2}\n{\"c\":3}";
        assert_eq!(
            frames(input, 1024),
            vec![
                Frame::Message("{\"a\":1}".into()),
                Frame::Stray("hello there".into()),
                Frame::Blank,
                Frame::Message("  {\"b\":2}".into()),
                Frame::Message("{\"c\":3}".into()),
            ]
        );
    }

    #[test]
    fn a_line_over_the_limit_is_refused_and_the_next_is_read() {
        let mut input = vec![b'{'];
        input.extend(std::iter::repeat_n(b'x', 9 * 1024 * 1024));
        input.extend_from_slice(b"\n{\"next\":true}\n");
        let mut reader = LineReader::new(Cursor::new(input));
        let first = reader.read_raw().unwrap().unwrap();
        assert!(first.over_limit());
        assert_eq!(first.bytes.len(), MAX_LINE_BYTES);
        assert_eq!(first.length, 9 * 1024 * 1024 + 1);
        assert_eq!(
            classify(&first),
            Frame::TooLong {
                length: 9 * 1024 * 1024 + 1,
                message: true
            }
        );
        assert_eq!(
            reader.next_frame().unwrap(),
            Some(Frame::Message("{\"next\":true}".into()))
        );
        assert_eq!(reader.next_frame().unwrap(), None);
    }

    #[test]
    fn bytes_that_are_not_utf8_never_stop_the_reader() {
        let input = b"caf\xE9 au lait\n{\"x\":\"\xFF\"}\n{\"ok\":1}\n";
        assert_eq!(
            frames(input, 1024),
            vec![
                Frame::Stray("caf\u{FFFD} au lait".into()),
                Frame::NotUtf8("{\"x\":\"\u{FFFD}\"}".into()),
                Frame::Message("{\"ok\":1}".into()),
            ]
        );
    }

    #[test]
    fn a_byte_order_mark_is_tolerated() {
        assert_eq!(
            frames(b"\xEF\xBB\xBF{\"a\":1}\n", 64),
            vec![Frame::Message("{\"a\":1}".into())]
        );
    }

    #[test]
    fn stray_text_is_cut_for_the_log() {
        let long = "x".repeat(500);
        match &frames(long.as_bytes(), 1024)[..] {
            [Frame::Stray(text)] => assert_eq!(text.chars().count(), STRAY_PREVIEW_CHARS),
            other => panic!("{other:?}"),
        }
    }

    /// A writer that records what it is given, shared with the test.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_writer_thread_writes_lines_in_order() {
        let sink = Sink::default();
        let writer = LineWriter::spawn(sink.clone()).unwrap();
        writer.send("{\"a\":1}".into()).unwrap();
        writer.send("{\"b\":2}".into()).unwrap();
        writer.join();
        assert_eq!(&*sink.0.lock().unwrap(), b"{\"a\":1}\n{\"b\":2}\n");
    }

    /// A writer that never returns until it is told to.
    struct Stuck(mpsc::Receiver<()>);

    impl Write for Stuck {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let _ = self.0.recv();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_plugin_that_stops_reading_fills_the_queue_not_the_caller() {
        let (release, wait) = mpsc::channel();
        let writer = LineWriter::with_capacity(Stuck(wait), 2).unwrap();
        // The thread takes the first line and blocks in write; two more
        // fill the queue; the next fails at once instead of blocking.
        let mut results = Vec::new();
        for i in 0..6 {
            results.push(writer.send(format!("{{\"n\":{i}}}")));
        }
        assert!(results.contains(&Err(SendError::QueueFull)), "{results:?}");
        drop(release);
        writer.join();
    }

    #[test]
    fn a_closed_output_says_so() {
        struct Gone;
        impl Write for Gone {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let writer = LineWriter::spawn(Gone).unwrap();
        writer.send("{}".into()).unwrap();
        for _ in 0..200 {
            if writer.is_broken() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(writer.send("{}".into()), Err(SendError::Closed));
    }

    proptest! {
        /// Any byte stream: no panic, and the lines join back into the
        /// input when no line is over the limit.
        #[test]
        fn lines_join_back_into_the_input(input in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let mut reader = LineReader::with_limit(Cursor::new(input.clone()), 4096);
            let mut joined = Vec::new();
            while let Some(line) = reader.read_raw().unwrap() {
                prop_assert!(!line.over_limit());
                let _ = classify(&line);
                joined.extend_from_slice(&line.bytes);
                if line.terminated {
                    joined.push(b'\n');
                }
            }
            prop_assert_eq!(joined, input);
        }

        /// With a small limit, every line is cut to the limit and its
        /// length still counts every byte.
        #[test]
        fn the_limit_holds_for_any_input(input in proptest::collection::vec(any::<u8>(), 0..2048), limit in 1usize..64) {
            let mut reader = LineReader::with_limit(Cursor::new(input.clone()), limit);
            let mut total = 0u64;
            let mut lines = 0u64;
            while let Some(line) = reader.read_raw().unwrap() {
                prop_assert!(line.bytes.len() <= limit);
                total += line.length + u64::from(line.terminated);
                lines += 1;
            }
            prop_assert_eq!(total, input.len() as u64);
            prop_assert!(lines <= input.len() as u64);
        }
    }
}
