//! The plugin's side of the protocol: JSON-RPC 2.0, one object per line,
//! over standard input and output (R3; the host's `protocol` module holds
//! the envelopes and error kinds this module shares).
//!
//! - **One reader thread** reads the host's lines with a bound, answers
//!   what is wrong with a line at once (-32700, -32600), keeps the
//!   session's [`Lifecycle`] (-32002 before `initialize`, only `exit`
//!   after `shutdown`), routes answers to the plugin's own requests, and
//!   hands every other request to a pool of [`WORKERS`] threads.
//! - **Every request has a cancel flag** that `$/cancel_request` sets; a
//!   cancelled request is still answered, by the handler or with -32800.
//! - **The plugin calls the host** ([`Peer::call`]): `ui/ask` for a host
//!   key question or a password, `ui/announce`, `$/progress`. The worker
//!   that calls waits; the reader thread delivers the answer.
//! - **Nothing secret is ever written to standard error**, which is the
//!   plugin's log in the host.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use diroxide_plugins::protocol::{
    ErrorKind, Id, Incoming, MAX_LINE_BYTES, Notification, Params, Request, Response, RpcError,
    methods, parse_line, to_line,
};
use diroxide_plugins::session::{Gate, InitializeParams, InitializeResult, Lifecycle};
use serde_json::{Value, json};

/// How many host requests run at once: the manifest's `concurrency`.
pub const WORKERS: usize = 16;

/// The method the host sends to cancel one of its requests.
pub const CANCEL_REQUEST: &str = "$/cancel_request";

/// A request's cancel flag, set by `$/cancel_request` or by the plugin's
/// own stop.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag that is not set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether it is set.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(-32800)` when it is set, for an early return.
    pub fn check(&self) -> Result<(), RpcError> {
        if self.is_cancelled() {
            Err(RpcError::new(
                ErrorKind::RequestCancelled,
                "the request was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

/// The plugin's connection to the host: it writes lines and waits for the
/// answers to the plugin's own requests.
pub struct Peer {
    out: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<i64, Sender<Response>>>,
    next: AtomicI64,
    closed: AtomicBool,
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer")
            .field("closed", &self.closed.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl Peer {
    /// A peer that writes to `out`.
    pub fn new(out: Box<dyn Write + Send>) -> Self {
        Self {
            out: Mutex::new(out),
            pending: Mutex::new(HashMap::new()),
            next: AtomicI64::new(1),
            closed: AtomicBool::new(false),
        }
    }

    fn send<T: serde::Serialize>(&self, message: &T) {
        let Ok(line) = to_line(message) else {
            return;
        };
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        // A host that is gone cannot be told; the reader sees the end.
        let _ = out.write_all(line.as_bytes());
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Params) {
        self.send(&Notification::new(method, params));
    }

    /// Calls the host and waits for its answer, up to `timeout`; a cancel
    /// stops the wait (the host's answer is then dropped).
    pub fn call(
        &self,
        method: &str,
        params: Params,
        cancel: Option<&Cancel>,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(gone());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        self.send(&Request::new(Id::Number(id), method, params));
        let answer = wait(&rx, cancel, timeout);
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        let response = answer?;
        match (response.result, response.error) {
            (_, Some(error)) => Err(error),
            (Some(result), None) => Ok(result),
            (None, None) => Ok(Value::Null),
        }
    }

    fn answer(&self, id: Id, result: Result<Value, RpcError>) {
        let response = match result {
            Ok(value) => Response::success(id, value),
            Err(error) => Response::failure(Some(id), error),
        };
        self.send(&response);
    }

    /// Hands an answer from the host to the request that waits for it (the
    /// reader thread does this; a test's fake host too).
    pub fn deliver(&self, response: Response) {
        let Some(Id::Number(id)) = response.id else {
            return;
        };
        let waiting = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        if let Some(waiting) = waiting {
            let _ = waiting.send(response);
        }
    }

    /// Ends every wait: the host is gone.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

fn gone() -> RpcError {
    RpcError::new(ErrorKind::PluginUnavailable, "the host is gone")
}

fn wait(
    rx: &Receiver<Response>,
    cancel: Option<&Cancel>,
    timeout: Duration,
) -> Result<Response, RpcError> {
    const STEP: Duration = Duration::from_millis(50);
    let mut left = timeout;
    loop {
        if let Some(cancel) = cancel {
            cancel.check()?;
        }
        let step = left.min(STEP);
        match rx.recv_timeout(step) {
            Ok(response) => return Ok(response),
            Err(RecvTimeoutError::Disconnected) => return Err(gone()),
            Err(RecvTimeoutError::Timeout) => {
                left = left.saturating_sub(step);
                if left.is_zero() {
                    return Err(RpcError::new(
                        ErrorKind::TimedOut,
                        format!("the host did not answer in {} seconds", timeout.as_secs()),
                    ));
                }
            }
        }
    }
}

/// What the plugin does with the host's messages. Requests run on worker
/// threads; `initialize` and `shutdown` on the reader thread.
pub trait Handler: Send + Sync + 'static {
    /// Answers `initialize`: the plugin's name, version and capabilities.
    fn initialize(&self, params: &InitializeParams) -> InitializeResult;

    /// Answers one request.
    fn request(
        &self,
        peer: &Arc<Peer>,
        method: &str,
        params: Params,
        cancel: &Cancel,
    ) -> Result<Value, RpcError>;

    /// Takes one notification (`settings/changed`, `provider/disconnect`).
    fn notification(&self, peer: &Arc<Peer>, method: &str, params: Params) {
        let _ = (peer, method, params);
    }

    /// Called once when `shutdown` comes, before it is answered: close
    /// every connection.
    fn shutdown(&self) {}
}

/// A line read with a bound.
enum Line {
    /// A whole line, without its end.
    Text(String),
    /// A line that was not UTF-8 or was longer than the bound; it was read
    /// to its end and dropped.
    Bad(&'static str),
}

/// Reads one line of at most `max` bytes; `None` at the end of input.
fn read_line(input: &mut dyn BufRead, max: usize) -> io::Result<Option<Line>> {
    let mut bytes = Vec::new();
    let mut too_long = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            if bytes.is_empty() && !too_long {
                return Ok(None);
            }
            break;
        }
        let (take, done) = match buffer.iter().position(|b| *b == b'\n') {
            Some(at) => (at + 1, true),
            None => (buffer.len(), false),
        };
        if !too_long {
            if bytes.len() + take > max + 2 {
                too_long = true;
                bytes.clear();
            } else {
                bytes.extend_from_slice(&buffer[..take]);
            }
        }
        input.consume(take);
        if done {
            break;
        }
    }
    if too_long {
        return Ok(Some(Line::Bad("the line is longer than the limit")));
    }
    while bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        bytes.pop();
    }
    Ok(Some(match String::from_utf8(bytes) {
        Ok(text) => Line::Text(text),
        Err(_) => Line::Bad("the line is not UTF-8"),
    }))
}

type Job = (Request, Cancel);

/// Serves the host until `exit` or the end of input; the answer is the
/// process's exit code (0 after `shutdown` and `exit` or at the end of
/// input, 1 for an `exit` that came without `shutdown`).
pub fn serve(
    mut input: impl BufRead,
    output: Box<dyn Write + Send>,
    handler: Arc<dyn Handler>,
) -> i32 {
    let peer = Arc::new(Peer::new(output));
    let cancels: Arc<Mutex<HashMap<Id, Cancel>>> = Arc::new(Mutex::new(HashMap::new()));
    let (jobs, queue) = mpsc::channel::<Job>();
    let queue = Arc::new(Mutex::new(queue));
    let mut workers = Vec::new();
    for n in 0..WORKERS {
        let queue = Arc::clone(&queue);
        let handler = Arc::clone(&handler);
        let peer = Arc::clone(&peer);
        let cancels = Arc::clone(&cancels);
        let worker = thread::Builder::new()
            .name(format!("rpc-worker-{n}"))
            .spawn(move || {
                loop {
                    let job = queue.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    let Ok((request, cancel)) = job else {
                        return;
                    };
                    let result = if cancel.is_cancelled() {
                        Err(RpcError::new(
                            ErrorKind::RequestCancelled,
                            "the request was cancelled",
                        ))
                    } else {
                        handler.request(&peer, &request.method, request.params, &cancel)
                    };
                    let id = request.id;
                    peer.answer(id.clone(), result);
                    cancels
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                }
            });
        if let Ok(worker) = worker {
            workers.push(worker);
        }
    }
    let mut lifecycle = Lifecycle::new();
    let mut shut_down = false;
    let code = loop {
        let line = match read_line(&mut input, MAX_LINE_BYTES) {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break 0,
        };
        let text = match line {
            Line::Text(text) => text,
            Line::Bad(why) => {
                peer.send(&Response::failure(
                    None,
                    RpcError::new(ErrorKind::ParseError, why),
                ));
                continue;
            }
        };
        if text.trim().is_empty() {
            continue;
        }
        let incoming = match parse_line(&text) {
            Ok(incoming) => incoming,
            Err(error) => {
                peer.send(&Response::failure(None, error));
                continue;
            }
        };
        match lifecycle.gate(&incoming) {
            Gate::Exit => break if shut_down { 0 } else { 1 },
            Gate::Drop => continue,
            Gate::Refuse(error) => {
                if let Incoming::Request(request) = incoming {
                    peer.answer(request.id, Err(error));
                }
                continue;
            }
            Gate::Proceed => {}
        }
        match incoming {
            Incoming::Response(response) => peer.deliver(response),
            Incoming::Notification(note) if note.method == CANCEL_REQUEST => {
                let id = note
                    .params
                    .get("id")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Id>(v).ok());
                if let Some(id) = id
                    && let Some(cancel) = cancels
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&id)
                {
                    cancel.cancel();
                }
            }
            Incoming::Notification(note) => handler.notification(&peer, &note.method, note.params),
            Incoming::Request(request) if request.method == methods::INITIALIZE => {
                let answer = lifecycle
                    .initialize(&request.params)
                    .map(|params| json!(handler.initialize(&params)));
                peer.answer(request.id, answer);
            }
            Incoming::Request(request) if request.method == methods::SHUTDOWN => {
                handler.shutdown();
                lifecycle.shut_down();
                shut_down = true;
                peer.answer(request.id, Ok(Value::Null));
            }
            Incoming::Request(request) => {
                let cancel = Cancel::new();
                cancels
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(request.id.clone(), cancel.clone());
                if jobs.send((request, cancel)).is_err() {
                    break 1;
                }
            }
        }
    };
    // Every running request is told to stop and nothing waits for the
    // host any more; the answers already being made are given a moment to
    // be written.
    for cancel in cancels
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
    {
        cancel.cancel();
    }
    peer.close();
    drop(jobs);
    let deadline = std::time::Instant::now() + FINISH_WAIT;
    while std::time::Instant::now() < deadline
        && !cancels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    {
        thread::sleep(Duration::from_millis(5));
    }
    code
}

/// How long the end of input waits for requests that are still being
/// answered.
pub const FINISH_WAIT: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use diroxide_plugins::session::{Peer as Who, PluginCapabilities};

    use super::*;

    /// Collects what the plugin writes.
    #[derive(Clone, Default)]
    struct Collected(Arc<Mutex<Vec<u8>>>);

    impl Write for Collected {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Collected {
        fn lines(&self) -> Vec<Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
    }

    struct Echo;

    impl Handler for Echo {
        fn initialize(&self, _: &InitializeParams) -> InitializeResult {
            InitializeResult {
                protocol: 1,
                plugin: Who {
                    name: "Echo".into(),
                    version: "1".into(),
                },
                plugin_capabilities: PluginCapabilities::default(),
                warnings: Vec::new(),
            }
        }

        fn request(
            &self,
            _: &Arc<Peer>,
            method: &str,
            params: Params,
            cancel: &Cancel,
        ) -> Result<Value, RpcError> {
            match method {
                "echo" => Ok(Value::Object(params)),
                "wait" => {
                    for _ in 0..200 {
                        cancel.check()?;
                        thread::sleep(Duration::from_millis(10));
                    }
                    Ok(json!("waited"))
                }
                _ => Err(RpcError::new(ErrorKind::MethodNotFound, "no such method")),
            }
        }
    }

    fn run(input: &str) -> (i32, Vec<Value>) {
        let out = Collected::default();
        let code = serve(
            Cursor::new(input.as_bytes().to_vec()),
            Box::new(out.clone()),
            Arc::new(Echo),
        );
        (code, out.lines())
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":1}}"#;

    #[test]
    fn the_lifecycle_is_kept() {
        let input = format!(
            "{}\n{INIT}\n{}\n{}\n{}\n",
            r#"{"jsonrpc":"2.0","id":0,"method":"echo"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"echo"}"#,
            r#"{"jsonrpc":"2.0","method":"exit"}"#,
        );
        let (code, lines) = run(&input);
        assert_eq!(code, 0);
        assert_eq!(lines[0]["error"]["code"], -32002);
        assert_eq!(lines[1]["result"]["plugin"]["name"], "Echo");
        assert_eq!(lines[2]["result"], Value::Null);
        assert_eq!(lines[2]["id"], 2);
        assert_eq!(lines[3]["error"]["code"], -32600);
    }

    #[test]
    fn bad_lines_are_answered_with_a_null_id() {
        let input = format!("{INIT}\nnot json\n[1,2]\n\u{FEFF}\n");
        let (code, lines) = run(&input);
        assert_eq!(code, 0);
        assert_eq!(lines[1]["error"]["code"], -32700);
        assert_eq!(lines[1]["id"], Value::Null);
        assert_eq!(lines[2]["error"]["code"], -32600);
    }

    #[test]
    fn a_line_over_the_limit_or_not_utf8_is_refused_and_the_next_read() {
        let mut input = Vec::new();
        input.extend_from_slice(INIT.as_bytes());
        input.extend_from_slice(b"\n{\"x\":\"\xff\xfe\"}\n");
        input.extend_from_slice(
            r#"{"jsonrpc":"2.0","id":5,"method":"echo","params":{"a":1}}"#.as_bytes(),
        );
        input.extend_from_slice(b"\r\n");
        let out = Collected::default();
        let mut reader = Cursor::new(input);
        let code = serve(&mut reader, Box::new(out.clone()), Arc::new(Echo));
        assert_eq!(code, 0);
        let lines = out.lines();
        assert_eq!(lines[1]["error"]["code"], -32700);
        // The echo runs on a worker; the input ended right after it, so it
        // is answered, with its result or as cancelled, never dropped.
        assert!(lines.iter().any(|l| l["id"] == 5));
        let mut long = Cursor::new(vec![b'a'; 40]);
        assert!(matches!(
            read_line(&mut long, 10).unwrap(),
            Some(Line::Bad(_))
        ));
        assert!(read_line(&mut long, 10).unwrap().is_none());
    }

    #[test]
    fn exit_without_shutdown_is_code_one() {
        let (code, _) = run(&format!(
            "{INIT}\n{}\n",
            r#"{"jsonrpc":"2.0","method":"exit"}"#
        ));
        assert_eq!(code, 1);
    }

    #[test]
    fn a_cancel_reaches_the_running_request() {
        // The input stays open until the cancel was sent, so the request
        // is still running when it comes.
        let (mut tx, rx) = pipe();
        let out = Collected::default();
        let reader = io::BufReader::new(rx);
        let server = {
            let out = out.clone();
            thread::spawn(move || serve(reader, Box::new(out), Arc::new(Echo)))
        };
        writeln!(tx, "{INIT}").unwrap();
        writeln!(tx, r#"{{"jsonrpc":"2.0","id":7,"method":"wait"}}"#).unwrap();
        thread::sleep(Duration::from_millis(100));
        writeln!(
            tx,
            r#"{{"jsonrpc":"2.0","method":"$/cancel_request","params":{{"id":7}}}}"#
        )
        .unwrap();
        let mut answered = false;
        for _ in 0..200 {
            if out.lines().iter().any(|l| l["id"] == 7) {
                answered = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(tx);
        assert_eq!(server.join().unwrap(), 0);
        assert!(answered);
        let lines = out.lines();
        let seven = lines.iter().find(|l| l["id"] == 7).unwrap();
        assert_eq!(seven["error"]["code"], -32800);
    }

    #[test]
    fn the_plugin_calls_the_host_and_gets_its_answer() {
        let out = Collected::default();
        let peer = Arc::new(Peer::new(Box::new(out.clone())));
        let caller = {
            let peer = Arc::clone(&peer);
            thread::spawn(move || peer.call("ui/ask", Params::new(), None, Duration::from_secs(5)))
        };
        let mut sent = Vec::new();
        for _ in 0..200 {
            sent = out.lines();
            if !sent.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(sent[0]["method"], "ui/ask");
        let id = sent[0]["id"].as_i64().unwrap();
        peer.deliver(Response::success(Id::Number(id), json!({"answer": true})));
        assert_eq!(caller.join().unwrap().unwrap(), json!({"answer": true}));
        let cancel = Cancel::new();
        cancel.cancel();
        assert_eq!(
            peer.call(
                "ui/ask",
                Params::new(),
                Some(&cancel),
                Duration::from_secs(5)
            )
            .unwrap_err()
            .code,
            -32800
        );
        assert_eq!(
            peer.call("ui/ask", Params::new(), None, Duration::from_millis(60))
                .unwrap_err()
                .code,
            ErrorKind::TimedOut.code()
        );
        peer.close();
        assert_eq!(
            peer.call("ui/ask", Params::new(), None, Duration::from_secs(1))
                .unwrap_err()
                .code,
            ErrorKind::PluginUnavailable.code()
        );
    }

    /// A pipe made of a channel: writes on one end are read on the other.
    fn pipe() -> (PipeWriter, PipeReader) {
        let (tx, rx) = mpsc::channel();
        (
            PipeWriter(tx),
            PipeReader {
                rx,
                buffer: Vec::new(),
            },
        )
    }

    struct PipeWriter(Sender<Vec<u8>>);

    impl Write for PipeWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.0.send(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct PipeReader {
        rx: Receiver<Vec<u8>>,
        buffer: Vec<u8>,
    }

    impl io::Read for PipeReader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.buffer.is_empty() {
                match self.rx.recv() {
                    Ok(bytes) => self.buffer = bytes,
                    Err(_) => return Ok(0),
                }
            }
            let n = out.len().min(self.buffer.len());
            out[..n].copy_from_slice(&self.buffer[..n]);
            self.buffer.drain(..n);
            Ok(n)
        }
    }
}
