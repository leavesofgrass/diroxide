//! How the host talks to a plugin: lines out, frames in. Today a process's
//! standard input and output ([`StdioTransport`]); in tests an in-process
//! pair of channels ([`MemoryTransport`]); a WebAssembly component can be
//! a further transport later without changing the plugin API.
//!
//! Every transport reads with the bounded reader of [`crate::framing`] and
//! writes from a queue, so neither a plugin that prints without end nor a
//! plugin that stops reading can block the host's threads.

use std::env;
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use diroxide_tools::process::{ProcessHandle, ProcessSpec, Stream, Tie};
use diroxide_tools::which;

use crate::error::PluginError;
use crate::framing::{Frame, LineReader, LineWriter, SendError};
use crate::log::PluginLog;
use crate::protocol::MAX_LINE_BYTES;

/// How long a kill waits to reap the killed process.
const KILL_REAP: Duration = Duration::from_secs(2);

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
    /// can, and reaps it; a killed process ends at once.
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
/// reader thread and a writer thread. It starts through the process
/// helper of `diroxide-tools`: no shell, no console window on Windows, its
/// own job object or process group, so killing it ends everything it
/// started; its standard error goes to the plugin's log, never the
/// screen.
#[derive(Debug)]
pub struct StdioTransport {
    name: String,
    process: Mutex<ProcessHandle>,
    writer: Mutex<Option<LineWriter>>,
    frames: Mutex<Receiver<FrameResult>>,
    log: PluginLog,
}

/// Finds a plugin's program: a first item starting `./` is inside the
/// plugin's folder; an absolute path is taken as it is; any other name is
/// looked up by the one PATH rule of the tools registry (absolute folders
/// only, never the current folder).
pub fn resolve_program(
    program: &str,
    folder: &Path,
    path_var: Option<&OsStr>,
) -> Result<PathBuf, String> {
    if let Some(inside) = program.strip_prefix("./") {
        let path = folder.join(inside);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!("{program} is not in the plugin's folder"))
        };
    }
    let as_path = Path::new(program);
    if as_path.is_absolute() {
        return if as_path.is_file() {
            Ok(as_path.to_path_buf())
        } else {
            Err(format!("{program} was not found"))
        };
    }
    which(program, path_var).ok_or_else(|| format!("{program} was not found on PATH"))
}

impl StdioTransport {
    /// Starts a plugin's program in its folder, with no shell: the first
    /// item of `program` is found by [`resolve_program`] and the rest are
    /// its arguments. Its standard error is kept only in memory; the host
    /// uses [`StdioTransport::start`] with the plugin's log file.
    pub fn spawn(name: &str, program: &[String], folder: &Path) -> Result<Self, PluginError> {
        let start_error = |message: String| PluginError::Start {
            plugin: name.to_owned(),
            message,
        };
        let (executable, args) = program
            .split_first()
            .ok_or_else(|| start_error("the program is empty".to_owned()))?;
        let path = resolve_program(executable, folder, env::var_os("PATH").as_deref())
            .map_err(start_error)?;
        let spec = ProcessSpec::new(path)
            .args(args)
            .cwd(folder)
            .diroxide_environment();
        Self::start(name, spec, PluginLog::in_memory(), MAX_LINE_BYTES)
    }

    /// Starts a plugin from a process spec, whose standard streams and tie
    /// this sets: input and output piped, standard error into `log`, a
    /// tree. Lines longer than `limit` are refused.
    pub fn start(
        name: &str,
        spec: ProcessSpec,
        log: PluginLog,
        limit: usize,
    ) -> Result<Self, PluginError> {
        let start_error = |message: String| PluginError::Start {
            plugin: name.to_owned(),
            message,
        };
        let mut process = spec
            .stdin(Stream::Piped)
            .stdout(Stream::Piped)
            .stderr(Stream::Piped)
            .tie(Tie::Tree)
            .spawn()
            .map_err(|e| start_error(e.to_string()))?;
        let stdout = process
            .take_stdout()
            .ok_or_else(|| start_error("its output could not be read".to_owned()))?;
        let stdin = process
            .take_stdin()
            .ok_or_else(|| start_error("its input could not be written".to_owned()))?;
        if let Some(stderr) = process.take_stderr() {
            log.follow(stderr).map_err(|e| start_error(e.to_string()))?;
        }
        let writer = LineWriter::spawn(stdin).map_err(|e| start_error(e.to_string()))?;
        let frames = spawn_reader(stdout, limit).map_err(|e| start_error(e.to_string()))?;
        Ok(Self {
            name: name.to_owned(),
            process: Mutex::new(process),
            writer: Mutex::new(Some(writer)),
            frames: Mutex::new(frames),
            log,
        })
    }

    /// The plugin's log.
    pub fn log(&self) -> &PluginLog {
        &self.log
    }

    /// The process id.
    pub fn id(&self) -> u32 {
        self.process.lock().map_or(0, |process| process.id())
    }

    /// How the process ended, if it has.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.process
            .lock()
            .ok()
            .and_then(|mut process| process.try_wait().ok().flatten())
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
        let mut process = self
            .process
            .lock()
            .map_err(|_| PluginError::Io("the process is poisoned".to_owned()))?;
        process
            .start_kill()
            .map_err(|e| PluginError::Io(e.to_string()))?;
        // A killed process ends at once; reap it so it never lingers.
        let _ = process.wait_timeout(KILL_REAP);
        Ok(())
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // The last resort: the host ends a plugin politely (shutdown, then
        // exit) before it drops the transport; a drop on a panic kills.
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

    const HELPER: &str = "DIROXIDE_PLUGIN_HELPER";

    /// This test binary run again as a plugin in one mode.
    fn helper(mode: &str) -> ProcessSpec {
        ProcessSpec::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "transport::tests::helper_process",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env(HELPER, mode)
    }

    /// The helper's behavior, chosen by the environment; an ordinary test
    /// run does nothing here. It first ends the test harness's own line,
    /// which the host then reads as stray text.
    #[test]
    fn helper_process() {
        use std::io::{BufRead, Write};
        let Ok(mode) = env::var(HELPER) else {
            return;
        };
        let mut out = std::io::stdout();
        out.write_all(b"\n").unwrap();
        match mode.as_str() {
            // Echoes each line; says something on standard error first.
            "echo" => {
                eprintln!("echo plugin starting");
                let stdin = std::io::stdin();
                let mut input = stdin.lock();
                let mut line = Vec::new();
                loop {
                    line.clear();
                    if input.read_until(b'\n', &mut line).unwrap() == 0 {
                        break;
                    }
                    out.write_all(&line).unwrap();
                    out.flush().unwrap();
                }
            }
            // Bytes in another code page, then a message, then a line
            // with a CRLF ending.
            "stray" => {
                out.write_all(&[b'c', b'a', b'f', 0xE9, b'\n']).unwrap();
                out.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"log\"}\r\n")
                    .unwrap();
                out.flush().unwrap();
            }
            // Starts a grandchild that shares this output, says so, and
            // sleeps.
            "tree" => {
                let mut grandchild = std::process::Command::new(env::current_exe().unwrap())
                    .args(["--exact", "transport::tests::helper_process", "--nocapture"])
                    .env(HELPER, "sleep")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                out.write_all(b"{\"ready\":true}\n").unwrap();
                out.flush().unwrap();
                thread::sleep(Duration::from_secs(120));
                let _ = grandchild.wait();
            }
            "sleep" => thread::sleep(Duration::from_secs(120)),
            _ => {}
        }
        out.flush().unwrap();
        std::process::exit(0);
    }

    fn start(mode: &str) -> StdioTransport {
        StdioTransport::start(mode, helper(mode), PluginLog::in_memory(), MAX_LINE_BYTES).unwrap()
    }

    /// The next frame that is not stray or blank.
    fn next_message(transport: &dyn PluginTransport) -> String {
        loop {
            match transport.receive(WAIT).unwrap() {
                Some(Frame::Message(text)) => return text,
                Some(Frame::Stray(_) | Frame::Blank) => continue,
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_process_plugin_echoes_with_its_standard_error_in_the_log() {
        let transport = start("echo");
        let line = "{\"name\":\"caf\u{e9}, \u{65e5}\u{672c}\"}";
        transport.send(line.to_owned()).unwrap();
        assert_eq!(next_message(&transport), line);
        for _ in 0..500 {
            if !transport.log().tail().is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(transport.log().tail(), ["echo plugin starting"]);
        transport.close_input();
        assert!(matches!(
            transport.receive(WAIT),
            Err(PluginError::Stopped(_))
        ));
        for _ in 0..500 {
            if transport.exit_status().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(transport.exit_status().is_some_and(|s| s.success()));
    }

    #[test]
    fn big_lines_each_way_and_many_small_ones() {
        let transport = start("echo");
        // Just under the limit, both ways.
        let big = format!("{{\"data\":\"{}\"}}", "x".repeat(MAX_LINE_BYTES - 11));
        assert_eq!(big.len(), MAX_LINE_BYTES);
        transport.send(big.clone()).unwrap();
        assert_eq!(next_message(&transport).len(), big.len());
        // Over the limit: refused, and the next line is still read.
        let over = format!("{{\"data\":\"{}\"}}", "y".repeat(9 * 1024 * 1024));
        transport.send(over).unwrap();
        transport.send("{\"after\":true}".to_owned()).unwrap();
        loop {
            match transport.receive(WAIT).unwrap() {
                Some(Frame::TooLong { message, .. }) => {
                    assert!(message);
                    break;
                }
                Some(Frame::Stray(_) | Frame::Blank) => continue,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(next_message(&transport), "{\"after\":true}");
        // Many small lines, sent faster than the plugin reads them.
        let count = 10_000;
        let sender = thread::scope(|scope| {
            let sending = scope.spawn(|| {
                for i in 0..count {
                    let line = format!("{{\"n\":{i}}}");
                    while let Err(PluginError::Limit(_)) = transport.send(line.clone()) {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            });
            for i in 0..count {
                assert_eq!(next_message(&transport), format!("{{\"n\":{i}}}"));
            }
            sending.join()
        });
        assert!(sender.is_ok());
    }

    #[test]
    fn stray_bytes_and_crlf_from_a_process_never_stop_it() {
        let transport = start("stray");
        let mut frames = Vec::new();
        loop {
            match transport.receive(WAIT) {
                Ok(Some(Frame::Blank)) => continue,
                Ok(Some(frame)) => frames.push(frame),
                Ok(None) | Err(_) => break,
            }
        }
        assert!(
            frames.contains(&Frame::Stray("caf\u{FFFD}".into())),
            "{frames:?}"
        );
        assert!(frames.contains(&Frame::Message(
            "{\"jsonrpc\":\"2.0\",\"method\":\"log\"}".into()
        )));
    }

    #[test]
    fn a_kill_ends_the_plugin_and_everything_it_started() {
        let transport = start("tree");
        assert_eq!(next_message(&transport), "{\"ready\":true}");
        transport.kill().unwrap();
        // The output ends only when the plugin and its grandchild, which
        // shares it, are both gone.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match transport.receive(Duration::from_millis(200)) {
                Err(PluginError::Stopped(_)) => break,
                _ if std::time::Instant::now() > deadline => panic!("a process survived"),
                _ => continue,
            }
        }
    }

    #[test]
    fn programs_are_found_by_the_one_path_rule() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("main.py"), b"").unwrap();
        let inside = resolve_program("./main.py", folder.path(), None).unwrap();
        assert_eq!(inside, folder.path().join("main.py"));
        assert_eq!(
            resolve_program("./missing.py", folder.path(), None).unwrap_err(),
            "./missing.py is not in the plugin's folder"
        );
        assert_eq!(
            resolve_program("python3", folder.path(), None).unwrap_err(),
            "python3 was not found on PATH"
        );
        let on_path = resolve_program(
            "main.py",
            Path::new("/elsewhere"),
            Some(folder.path().as_os_str()),
        )
        .unwrap();
        assert_eq!(on_path, folder.path().join("main.py"));
    }

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
