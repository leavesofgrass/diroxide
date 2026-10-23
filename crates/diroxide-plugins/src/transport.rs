//! How the host talks to a plugin: lines out, frames in. Today a process's
//! standard input and output ([`StdioTransport`]); in tests an in-process
//! pair of channels ([`MemoryTransport`]); a WebAssembly component can be
//! a further transport later without changing the plugin API.
//!
//! Every transport reads with the bounded reader of [`crate::framing`] and
//! writes from a queue, so neither a plugin that prints without end nor a
//! plugin that stops reading can block the host's threads.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::error::PluginError;
use crate::framing::{Frame, LineReader, LineWriter, SendError};
use crate::protocol::MAX_LINE_BYTES;

/// Sends lines to a plugin and receives its frames. Shared by the host's
/// threads: sending never blocks, and one thread receives.
pub trait PluginTransport: Send + Sync {
    /// Queues one line for the plugin; the transport adds the line break.
    /// Never blocks: a full queue is [`PluginError::Limit`].
    fn send(&self, line: String) -> Result<(), PluginError>;

    /// Waits up to `timeout` for the plugin's next frame; `Ok(None)` when
    /// none came. [`PluginError::Stopped`] once the plugin's output has
    /// ended and every frame was received.
    fn receive(&self, timeout: Duration) -> Result<Option<Frame>, PluginError>;

    /// Closes the plugin's input, after what is queued is written: the
    /// polite end of the stream, at which a plugin exits.
    fn close_input(&self);

    /// Stops the plugin, and everything it started where the transport
    /// can, without waiting.
    fn kill(&self) -> Result<(), PluginError>;
}

/// What the reader thread sends: a frame, or why reading stopped.
type FrameResult = Result<Frame, String>;

/// Starts a thread that reads frames from an output into a channel, until
/// the output ends or the receiver is gone.
fn spawn_reader<R: Read + Send + 'static>(
    output: R,
    limit: usize,
) -> std::io::Result<Receiver<FrameResult>> {
    let (sender, frames) = mpsc::channel();
    thread::Builder::new()
        .name("plugin-reader".to_owned())
        .spawn(move || {
            let mut reader = LineReader::with_limit(output, limit);
            loop {
                let next = match reader.next_frame() {
                    Ok(Some(frame)) => Ok(frame),
                    Ok(None) => break,
                    Err(e) => Err(e.to_string()),
                };
                let failed = next.is_err();
                if sender.send(next).is_err() || failed {
                    break;
                }
            }
        })?;
    Ok(frames)
}

fn receive_from(
    frames: &Mutex<Receiver<FrameResult>>,
    name: &str,
    timeout: Duration,
) -> Result<Option<Frame>, PluginError> {
    let frames = frames
        .lock()
        .map_err(|_| PluginError::Io("the reader is poisoned".to_owned()))?;
    match frames.recv_timeout(timeout) {
        Ok(Ok(frame)) => Ok(Some(frame)),
        Ok(Err(message)) => Err(PluginError::Io(message)),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => Err(PluginError::Stopped(name.to_owned())),
    }
}

fn send_error(name: &str, error: SendError) -> PluginError {
    match error {
        SendError::QueueFull => PluginError::Limit(format!("{name} is not reading its input")),
        SendError::Closed => PluginError::Stopped(name.to_owned()),
    }
}

/// A plugin process spoken to over its standard input and output, with a
/// reader thread and a writer thread.
#[derive(Debug)]
pub struct StdioTransport {
    name: String,
    child: Mutex<Child>,
    writer: Mutex<Option<LineWriter>>,
    frames: Mutex<Receiver<FrameResult>>,
}

impl StdioTransport {
    /// Starts a plugin's program in its folder, with no shell: the first
    /// item of `program` is the executable and the rest its arguments.
    pub fn spawn(name: &str, program: &[String], folder: &Path) -> Result<Self, PluginError> {
        Self::spawn_with_limit(name, program, folder, MAX_LINE_BYTES)
    }

    /// The same, with another line limit.
    pub fn spawn_with_limit(
        name: &str,
        program: &[String],
        folder: &Path,
        limit: usize,
    ) -> Result<Self, PluginError> {
        let start_error = |message: String| PluginError::Start {
            plugin: name.to_owned(),
            message,
        };
        let (executable, args) = program
            .split_first()
            .ok_or_else(|| start_error("the program is empty".to_owned()))?;
        let mut child = Command::new(executable)
            .args(args)
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| start_error(e.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| start_error("its output could not be read".to_owned()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| start_error("its input could not be written".to_owned()))?;
        let writer = LineWriter::spawn(stdin).map_err(|e| start_error(e.to_string()))?;
        let frames = spawn_reader(stdout, limit).map_err(|e| start_error(e.to_string()))?;
        Ok(Self {
            name: name.to_owned(),
            child: Mutex::new(child),
            writer: Mutex::new(Some(writer)),
            frames: Mutex::new(frames),
        })
    }
}

impl PluginTransport for StdioTransport {
    fn send(&self, line: String) -> Result<(), PluginError> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| PluginError::Stopped(self.name.clone()))?;
        let writer = writer
            .as_ref()
            .ok_or_else(|| PluginError::Stopped(self.name.clone()))?;
        writer.send(line).map_err(|e| send_error(&self.name, e))
    }

    fn receive(&self, timeout: Duration) -> Result<Option<Frame>, PluginError> {
        receive_from(&self.frames, &self.name, timeout)
    }

    fn close_input(&self) {
        if let Ok(mut writer) = self.writer.lock() {
            writer.take();
        }
    }

    fn kill(&self) -> Result<(), PluginError> {
        self.close_input();
        let mut child = self
            .child
            .lock()
            .map_err(|_| PluginError::Io("the process is poisoned".to_owned()))?;
        match child.try_wait() {
            Ok(Some(_)) => Ok(()),
            _ => child
                .kill()
                .and_then(|()| child.wait().map(|_| ()))
                .map_err(|e| PluginError::Io(e.to_string())),
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

/// An in-process transport for tests and fakes: the host's side of a pair
/// of channels. The other side is a [`MemoryPeer`].
#[derive(Debug)]
pub struct MemoryTransport {
    name: String,
    to_peer: Mutex<Option<Sender<String>>>,
    frames: Mutex<Receiver<FrameResult>>,
    killed: Sender<()>,
}

/// The plugin's side of a [`MemoryTransport`]: it reads the host's lines
/// and writes raw output, which the host reads through the same bounded
/// reader and sorting as a process's output.
#[derive(Debug)]
pub struct MemoryPeer {
    /// The lines the host sent, without line breaks.
    pub from_host: Receiver<String>,
    output: Option<Sender<Vec<u8>>>,
    /// Receives once when the host kills the transport.
    pub killed: Receiver<()>,
}

impl MemoryPeer {
    /// Writes raw bytes as the plugin's output; `false` once the host is
    /// gone.
    pub fn write(&self, bytes: &[u8]) -> bool {
        self.output
            .as_ref()
            .is_some_and(|out| out.send(bytes.to_vec()).is_ok())
    }

    /// Writes one line and its line break.
    pub fn write_line(&self, line: &str) -> bool {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        self.write(&bytes)
    }

    /// Ends the plugin's output, as a process that exits does.
    pub fn end_output(&mut self) {
        self.output = None;
    }
}

/// Reads the chunks a [`MemoryPeer`] writes as one byte stream.
struct ChunkReader {
    chunks: Receiver<Vec<u8>>,
    current: Vec<u8>,
    at: usize,
}

impl Read for ChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.at >= self.current.len() {
            match self.chunks.recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.current.len() - self.at);
        buf[..n].copy_from_slice(&self.current[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl MemoryTransport {
    /// A connected pair with the default line limit.
    pub fn pair(name: &str) -> Result<(Self, MemoryPeer), PluginError> {
        Self::pair_with_limit(name, MAX_LINE_BYTES)
    }

    /// A connected pair with another line limit.
    pub fn pair_with_limit(name: &str, limit: usize) -> Result<(Self, MemoryPeer), PluginError> {
        let (to_peer, from_host) = mpsc::channel();
        let (output, chunks) = mpsc::channel();
        let (killed_tx, killed_rx) = mpsc::channel();
        let reader = ChunkReader {
            chunks,
            current: Vec::new(),
            at: 0,
        };
        let frames = spawn_reader(reader, limit).map_err(|e| PluginError::Io(e.to_string()))?;
        Ok((
            Self {
                name: name.to_owned(),
                to_peer: Mutex::new(Some(to_peer)),
                frames: Mutex::new(frames),
                killed: killed_tx,
            },
            MemoryPeer {
                from_host,
                output: Some(output),
                killed: killed_rx,
            },
        ))
    }
}

impl PluginTransport for MemoryTransport {
    fn send(&self, line: String) -> Result<(), PluginError> {
        let to_peer = self
            .to_peer
            .lock()
            .map_err(|_| PluginError::Stopped(self.name.clone()))?;
        to_peer
            .as_ref()
            .ok_or_else(|| PluginError::Stopped(self.name.clone()))?
            .send(line)
            .map_err(|_| PluginError::Stopped(self.name.clone()))
    }

    fn receive(&self, timeout: Duration) -> Result<Option<Frame>, PluginError> {
        receive_from(&self.frames, &self.name, timeout)
    }

    fn close_input(&self) {
        if let Ok(mut to_peer) = self.to_peer.lock() {
            to_peer.take();
        }
    }

    fn kill(&self) -> Result<(), PluginError> {
        self.close_input();
        let _ = self.killed.send(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn the_memory_pair_carries_lines_and_frames() {
        let (host, mut peer) = MemoryTransport::pair("echo").unwrap();
        host.send("{\"jsonrpc\":\"2.0\",\"method\":\"x\"}".into())
            .unwrap();
        assert_eq!(
            peer.from_host.recv_timeout(WAIT).unwrap(),
            "{\"jsonrpc\":\"2.0\",\"method\":\"x\"}"
        );
        assert!(peer.write(b"stray\r\n{\"a\""));
        assert!(peer.write(b":1}\n"));
        assert_eq!(
            host.receive(WAIT).unwrap(),
            Some(Frame::Stray("stray".into()))
        );
        assert_eq!(
            host.receive(WAIT).unwrap(),
            Some(Frame::Message("{\"a\":1}".into()))
        );
        assert_eq!(host.receive(Duration::from_millis(10)).unwrap(), None);
        peer.end_output();
        assert!(matches!(
            host.receive(WAIT),
            Err(PluginError::Stopped(name)) if name == "echo"
        ));
        host.kill().unwrap();
        assert!(peer.killed.recv_timeout(WAIT).is_ok());
        assert!(host.send("{}".into()).is_err());
    }

    #[test]
    fn a_partial_line_before_the_end_is_still_read() {
        let (host, mut peer) = MemoryTransport::pair("half").unwrap();
        assert!(peer.write(b"{\"partial\":true}"));
        peer.end_output();
        assert_eq!(
            host.receive(WAIT).unwrap(),
            Some(Frame::Message("{\"partial\":true}".into()))
        );
        assert!(host.receive(WAIT).is_err());
    }
}
