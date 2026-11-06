//! The one runner (alpha.7, R30 commit 18): every helper program diroxide
//! runs under limits goes through [`run_limited`], whoever asks for it:
//! the plugin host's `tools/run`, a manifest's previewer and converter
//! recipes, the registry's own runs, and a version command.
//!
//! It was the plugin host's runner (`diroxide-plugins/src/tools.rs`)
//! until this commit; the host now calls it, so the rules are the host's:
//!
//! - **An argument list,** through the process helper: no shell, no
//!   console window, a tree killed as a whole.
//! - **Both streams pumped** on threads of their own, so a program that
//!   fills one pipe never stalls on the other.
//! - **A time limit, a cancel, an output cap,** checked every
//!   [`POLL`] while the program runs; past the time or on a cancel the
//!   program and every child it started are stopped. At a shared cap the
//!   tree is stopped too ([`AtCap::Stop`], the host's rule); a cap per
//!   stream may instead keep the first bytes and drop the rest while the
//!   program finishes ([`AtCap::Drop`], the recipes' rule).
//! - **A program that ends** has its tree stopped at once, so a child it
//!   left behind cannot hold the pipes open.
//! - **Streamed records** are cut at a byte (a line feed, a NUL) or at a
//!   line feed or a lone carriage return ([`Cut::CarriageReturn`], for
//!   git's `clone --progress`, R29 2.1), handed over as they arrive and
//!   not kept.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use diroxide_vfs::ToolStream;

use crate::process::{ProcessError, ProcessSpec, Stream};

/// How often a run looks at its program, its time, its cap and its
/// cancel: the plugin host's interval.
pub const POLL: Duration = Duration::from_millis(10);

/// The size of each read from a pipe.
const CHUNK: usize = 64 * 1024;

/// Where a streamed record ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cut {
    /// At this byte, which is not kept: a line feed, or a NUL.
    Byte(u8),
    /// At a line feed or a carriage return, so each rewrite of a progress
    /// line is a record; the empty record between a carriage return and
    /// its line feed is dropped.
    CarriageReturn,
}

impl Cut {
    fn find(self, bytes: &[u8]) -> Option<usize> {
        match self {
            Cut::Byte(byte) => bytes.iter().position(|b| *b == byte),
            Cut::CarriageReturn => bytes.iter().position(|b| *b == b'\n' || *b == b'\r'),
        }
    }
}

/// What happens when output passes a cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AtCap {
    /// The program and its children are stopped (the plugin host's
    /// rule, so a tool that floods its pipes cannot run on).
    Stop,
    /// The bytes past the cap are read and dropped, and the program is
    /// left to finish (the manifest recipes' rule: a preview shows its
    /// first 64 KiB and says it was cut).
    Drop,
}

/// The output caps of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Caps {
    /// Bytes kept from both streams together.
    pub total: u64,
    /// Bytes kept from standard output.
    pub stdout: u64,
    /// Bytes kept from standard error.
    pub stderr: u64,
    /// What passing a cap does.
    pub at_cap: AtCap,
}

impl Caps {
    /// One cap for both streams together; past it the program is stopped.
    pub const fn shared(total: u64) -> Self {
        Self {
            total,
            stdout: u64::MAX,
            stderr: u64::MAX,
            at_cap: AtCap::Stop,
        }
    }

    /// A cap per stream; past one, the rest of that stream is dropped and
    /// the program left to finish.
    pub const fn each(stdout: u64, stderr: u64) -> Self {
        Self {
            total: u64::MAX,
            stdout,
            stderr,
            at_cap: AtCap::Drop,
        }
    }
}

/// Sends one streamed record: its stream, and its bytes without the cut.
pub type Emit = Arc<dyn Fn(ToolStream, &[u8]) + Send + Sync>;

/// Which streams are handed over record by record, cut how, and to whom.
#[derive(Clone)]
pub struct Streaming {
    /// Standard output's records are handed over, not kept.
    pub stdout: bool,
    /// Standard error's records are handed over, not kept.
    pub stderr: bool,
    /// Where a record ends.
    pub cut: Cut,
    /// Who takes each record.
    pub emit: Emit,
}

impl std::fmt::Debug for Streaming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Streaming")
            .field("stdout", &self.stdout)
            .field("stderr", &self.stderr)
            .field("cut", &self.cut)
            .finish_non_exhaustive()
    }
}

/// The limits and the input of one run.
pub struct Limited {
    /// How long the program may run; `None` for no limit (a manifest
    /// converter that gives none).
    pub timeout: Option<Duration>,
    /// The output caps.
    pub caps: Caps,
    /// Standard input's bytes, written on a thread of their own; `None`
    /// closes standard input, so a program that asks a question gets the
    /// end of input and never waits.
    pub input: Option<Box<dyn Read + Send>>,
    /// The streams handed over record by record, if any.
    pub streaming: Option<Streaming>,
}

impl Limited {
    /// A run with this time limit and these caps, standard input closed,
    /// nothing streamed.
    pub fn new(timeout: Option<Duration>, caps: Caps) -> Self {
        Self {
            timeout,
            caps,
            input: None,
            streaming: None,
        }
    }

    /// The same run with this standard input.
    pub fn with_input(mut self, input: Box<dyn Read + Send>) -> Self {
        self.input = Some(input);
        self
    }

    /// The same run streaming records.
    pub fn streaming(mut self, streaming: Streaming) -> Self {
        self.streaming = Some(streaming);
        self
    }
}

impl std::fmt::Debug for Limited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limited")
            .field("timeout", &self.timeout)
            .field("caps", &self.caps)
            .field("input", &self.input.is_some())
            .field("streaming", &self.streaming)
            .finish()
    }
}

/// A run that came to its end, or was stopped at its shared cap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Finished {
    /// The exit code; `None` when it was stopped (at the cap) or ended by
    /// a signal.
    pub status: Option<i32>,
    /// Standard output, at most its cap; empty when it was streamed.
    pub stdout: Vec<u8>,
    /// Standard error, likewise.
    pub stderr: Vec<u8>,
    /// Whether standard output passed a cap.
    pub stdout_cut: bool,
    /// Whether standard error passed a cap.
    pub stderr_cut: bool,
    /// Whether the program was stopped at a cap ([`AtCap::Stop`]).
    pub stopped_at_cap: bool,
}

/// Why a run gave no [`Finished`]. Each caller says it in its own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunFailure {
    /// The program could not be started, or its input could not be fed.
    Start(ProcessError),
    /// The cancel was raised; the program and its children were stopped.
    Cancelled,
    /// The program ran past this limit; it and its children were stopped.
    TimedOut(Duration),
}

/// What one pump kept.
struct Pump {
    bytes: Vec<u8>,
    cut: bool,
}

/// What every pump of a run shares.
#[derive(Clone)]
struct Shared {
    total: Arc<AtomicU64>,
    over: Arc<AtomicBool>,
    at_cap: AtCap,
}

/// Runs a program under limits, until it ends, is cancelled, runs past
/// its time, or passes a stopping cap. `spec` names the program, its
/// arguments, folder and environment; its standard streams are set here.
/// `cancelled` is asked every [`POLL`].
pub fn run_limited(
    spec: ProcessSpec,
    limited: Limited,
    cancelled: &dyn Fn() -> bool,
) -> Result<Finished, RunFailure> {
    let Limited {
        timeout,
        caps,
        input,
        streaming,
    } = limited;
    let spec = spec
        .stdin(if input.is_some() {
            Stream::Piped
        } else {
            Stream::Null
        })
        .stdout(Stream::Piped)
        .stderr(Stream::Piped);
    let mut child = spec.spawn().map_err(RunFailure::Start)?;
    if let (Some(mut input), Some(mut stdin)) = (input, child.take_stdin()) {
        thread::Builder::new()
            .name("tool-stdin".to_owned())
            .spawn(move || {
                let _ = std::io::copy(&mut input, &mut stdin);
                let _ = stdin.flush();
            })
            .map_err(|e| RunFailure::Start(ProcessError::Io(e.to_string())))?;
    }
    let shared = Shared {
        total: Arc::new(AtomicU64::new(caps.total)),
        over: Arc::new(AtomicBool::new(false)),
        at_cap: caps.at_cap,
    };
    let start_pump = |reader: Option<Box<dyn Read + Send>>, stream: ToolStream, cap: u64| {
        let shared = shared.clone();
        let records = streaming
            .as_ref()
            .filter(|s| match stream {
                ToolStream::Stdout => s.stdout,
                ToolStream::Stderr => s.stderr,
            })
            .map(|s| (Arc::clone(&s.emit), s.cut));
        let name = match stream {
            ToolStream::Stdout => "tool-stdout",
            ToolStream::Stderr => "tool-stderr",
        };
        reader.and_then(|reader| {
            thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || pump(reader, stream, cap, &shared, records))
                .ok()
        })
    };
    let stdout = start_pump(
        child
            .take_stdout()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
        ToolStream::Stdout,
        caps.stdout,
    );
    let stderr = start_pump(
        child
            .take_stderr()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
        ToolStream::Stderr,
        caps.stderr,
    );
    let started = Instant::now();
    let ending = loop {
        if let Ok(Some(status)) = child.try_wait() {
            // A child it left behind must not hold the pipes open.
            let _ = child.start_kill();
            break Ok(status.code());
        }
        if cancelled() {
            let _ = child.kill_tree();
            break Err(RunFailure::Cancelled);
        }
        if let Some(limit) = timeout
            && started.elapsed() >= limit
        {
            let _ = child.kill_tree();
            break Err(RunFailure::TimedOut(limit));
        }
        if shared.over.load(Ordering::SeqCst) {
            let _ = child.kill_tree();
            break Ok(None);
        }
        thread::sleep(POLL);
    };
    let (stdout, stderr) = (joined(stdout), joined(stderr));
    let status = ending?;
    Ok(Finished {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_cut: stdout.cut,
        stderr_cut: stderr.cut,
        stopped_at_cap: shared.over.load(Ordering::SeqCst),
    })
}

fn joined(handle: Option<JoinHandle<Pump>>) -> Pump {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or(Pump {
            bytes: Vec::new(),
            cut: false,
        })
}

/// Reads one stream until it ends, keeping what fits under its own cap
/// and the shared one, and handing records over when asked.
fn pump(
    mut reader: impl Read,
    stream: ToolStream,
    cap: u64,
    shared: &Shared,
    records: Option<(Emit, Cut)>,
) -> Pump {
    let mut kept = Vec::new();
    let mut pending = Vec::new();
    let mut own_left = cap;
    let mut cut = false;
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let shared_left = shared
            .total
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                Some(left.saturating_sub(n as u64))
            })
            .unwrap_or(0);
        let allowed = shared_left.min(own_left).min(n as u64) as usize;
        own_left -= allowed as u64;
        if allowed < n {
            cut = true;
            if shared.at_cap == AtCap::Stop {
                shared.over.store(true, Ordering::SeqCst);
            }
        }
        let chunk = &buffer[..allowed];
        match &records {
            Some((emit, rule)) => {
                pending.extend_from_slice(chunk);
                while let Some(at) = rule.find(&pending) {
                    let record: Vec<u8> = pending.drain(..=at).collect();
                    let record = &record[..record.len() - 1];
                    if *rule == Cut::CarriageReturn && record.is_empty() {
                        continue;
                    }
                    emit(stream, record);
                }
            }
            None => kept.extend_from_slice(chunk),
        }
        if shared.over.load(Ordering::SeqCst) {
            break;
        }
    }
    if let Some((emit, _)) = &records
        && !pending.is_empty()
    {
        emit(stream, &pending);
    }
    Pump { bytes: kept, cut }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::io::Write;
    use std::sync::Mutex;

    use super::*;

    const HELPER: &str = "DIROXIDE_RUN_HELPER";

    /// This test binary run again as a tool in one mode.
    fn tool(mode: &str) -> ProcessSpec {
        ProcessSpec::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "run::tests::helper",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env(HELPER, mode)
    }

    /// The tool's behavior, chosen by the environment; an ordinary test
    /// run does nothing here.
    #[test]
    fn helper() {
        let Ok(mode) = env::var(HELPER) else {
            return;
        };
        let mut out = std::io::stdout();
        let mut err = std::io::stderr();
        match mode.as_str() {
            "both" => {
                writeln!(out, "to standard output").unwrap();
                writeln!(err, "to standard error").unwrap();
            }
            "big" => {
                let line = vec![b'x'; 1024 * 1024];
                for _ in 0..100 {
                    if out.write_all(&line).is_err() {
                        break;
                    }
                }
            }
            "forever" => {
                writeln!(out, "started").unwrap();
                out.flush().unwrap();
                thread::sleep(Duration::from_secs(600));
            }
            "echo" => {
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text).unwrap();
                writeln!(out, "read:{}", text.trim()).unwrap();
            }
            "progress" => {
                err.write_all(
                    b"Receiving objects:  50% (1/2)\rReceiving objects: 100% (2/2)\r\ndone.\n",
                )
                .unwrap();
            }
            "code" => {
                out.flush().unwrap();
                std::process::exit(3);
            }
            _ => {}
        }
        out.flush().unwrap();
        err.flush().unwrap();
        std::process::exit(0);
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn both_streams_and_the_exit_code_come_back() {
        let done = run_limited(
            tool("both"),
            Limited::new(Some(Duration::from_secs(60)), Caps::shared(1 << 20)),
            &never,
        )
        .unwrap();
        assert_eq!(done.status, Some(0));
        assert!(String::from_utf8_lossy(&done.stdout).contains("to standard output"));
        assert!(String::from_utf8_lossy(&done.stderr).contains("to standard error"));
        assert!(!done.stopped_at_cap);
        let failed = run_limited(
            tool("code"),
            Limited::new(None, Caps::shared(1 << 20)),
            &never,
        )
        .unwrap();
        assert_eq!(failed.status, Some(3));
    }

    /// The host's rule: past the shared cap the tree is stopped. (A
    /// program whose writes fail once its pipe closes may end on its own
    /// first; its exit code is then kept, as the host always did.)
    #[test]
    fn a_shared_cap_stops_the_program() {
        let started = Instant::now();
        let done = run_limited(
            tool("big"),
            Limited::new(Some(Duration::from_secs(60)), Caps::shared(2_000_000)),
            &never,
        )
        .unwrap();
        assert!(done.stopped_at_cap);
        assert!(done.stdout_cut);
        assert!(done.stdout.len() <= 2_000_000);
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    /// The recipes' rule: past a stream's cap the rest is dropped and the
    /// program finishes.
    #[test]
    fn a_cap_per_stream_keeps_the_first_bytes_and_lets_it_finish() {
        let done = run_limited(
            tool("big"),
            Limited::new(
                Some(Duration::from_secs(60)),
                Caps::each(64 * 1024, 64 * 1024),
            ),
            &never,
        )
        .unwrap();
        assert_eq!(done.status, Some(0));
        assert_eq!(done.stdout.len(), 64 * 1024);
        assert!(done.stdout_cut);
        assert!(!done.stopped_at_cap);
    }

    #[test]
    fn the_time_limit_and_the_cancel_stop_the_tree() {
        let started = Instant::now();
        let timed = run_limited(
            tool("forever"),
            Limited::new(Some(Duration::from_millis(1500)), Caps::shared(1 << 20)),
            &never,
        )
        .unwrap_err();
        assert_eq!(timed, RunFailure::TimedOut(Duration::from_millis(1500)));
        assert!(started.elapsed() < Duration::from_secs(20));
        let raised = Instant::now();
        let cancelled = run_limited(
            tool("forever"),
            Limited::new(None, Caps::shared(1 << 20)),
            &|| raised.elapsed() > Duration::from_millis(500),
        )
        .unwrap_err();
        assert_eq!(cancelled, RunFailure::Cancelled);
    }

    #[test]
    fn standard_input_reaches_the_program() {
        let done = run_limited(
            tool("echo"),
            Limited::new(Some(Duration::from_secs(60)), Caps::shared(1 << 20))
                .with_input(Box::new(std::io::Cursor::new(b"hello".to_vec()))),
            &never,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&done.stdout).contains("read:hello"));
    }

    /// Each rewrite of a progress line is a record, a CR LF ending makes
    /// no empty one, and streamed records are not kept.
    #[test]
    fn records_stream_cut_at_carriage_returns() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&seen);
        let emit: Emit = Arc::new(move |stream, bytes| {
            into.lock()
                .unwrap()
                .push((stream, String::from_utf8_lossy(bytes).into_owned()));
        });
        let done = run_limited(
            tool("progress"),
            Limited::new(Some(Duration::from_secs(60)), Caps::shared(1 << 20)).streaming(
                Streaming {
                    stdout: false,
                    stderr: true,
                    cut: Cut::CarriageReturn,
                    emit,
                },
            ),
            &never,
        )
        .unwrap();
        assert!(done.stderr.is_empty());
        let seen = seen.lock().unwrap();
        let progress: Vec<&str> = seen
            .iter()
            .filter(|(s, _)| *s == ToolStream::Stderr)
            .map(|(_, r)| r.as_str())
            .filter(|r| r.starts_with("Receiving") || *r == "done.")
            .collect();
        assert_eq!(
            progress,
            [
                "Receiving objects:  50% (1/2)",
                "Receiving objects: 100% (2/2)",
                "done."
            ]
        );
    }

    #[test]
    fn a_program_that_cannot_start_says_so() {
        let failure = run_limited(
            ProcessSpec::new("no-such-program-anywhere"),
            Limited::new(None, Caps::shared(1)),
            &never,
        )
        .unwrap_err();
        assert!(matches!(failure, RunFailure::Start(_)), "{failure:?}");
    }
}
