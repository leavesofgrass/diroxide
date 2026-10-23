//! In-process fakes for tests: a plugin that answers through a
//! [`MemoryPeer`], and a driver for the
//! fixture dialogues in `fixtures/plugins/dialogues/`. Not for use outside
//! tests; the misbehaving process plugin of the test plan (R22, section
//! 8.1) is a separate program.
//!
//! A dialogue is a text file of lines:
//!
//! - `> LINE`: a line sent to the side under test;
//! - `< JSON`: the next line it must write, compared as JSON, where the
//!   string `"*"` matches any value;
//! - `< exit`: the side under test has ended its session;
//! - `# ...` and blank lines are comments.
//!
//! After the last line nothing more may have been written.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::framing::{Frame, RawLine, classify};
use crate::protocol::{
    ErrorKind, Incoming, Notification, Params, Response, RpcError, methods, parse_line, to_line,
};
use crate::session::{
    Gate, InitializeResult, Lifecycle, Peer, PluginCapabilities, ProviderCapability,
};
use crate::transport::MemoryPeer;

/// How the fake plugin behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Correct: answers everything it knows, -32601 for the rest.
    Echo,
    /// Answers `initialize` with another protocol's major version.
    Protocol(u32),
    /// Never answers anything.
    Silent,
    /// Writes a stray line before answering `initialize`.
    Chatty,
    /// Says something and reports progress while it runs a command.
    Talk,
    /// Sends a hundred announcements while it runs a command.
    Flood,
    /// Announces text that imitates the app, with hidden characters.
    Spoof,
    /// Prints stray lines while it runs a command.
    Stray,
}

/// What the fake plugin saw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    /// Every message it read, in order.
    pub received: Vec<Incoming>,
}

impl Transcript {
    /// The methods of the requests and notifications it read, in order.
    pub fn methods(&self) -> Vec<String> {
        self.received
            .iter()
            .filter_map(|m| match m {
                Incoming::Request(r) => Some(r.method.clone()),
                Incoming::Notification(n) => Some(n.method.clone()),
                Incoming::Response(_) => None,
            })
            .collect()
    }
}

/// A plugin in the same process, with the answering side's lifecycle.
#[derive(Debug)]
pub struct FakePlugin {
    mode: Mode,
    life: Lifecycle,
    transcript: Transcript,
    exited: bool,
}

impl FakePlugin {
    /// A fake plugin in a mode.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            life: Lifecycle::new(),
            transcript: Transcript::default(),
            exited: false,
        }
    }

    /// Whether `exit` came.
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// What it answers to `initialize`.
    pub fn initialize_result(protocol: u32) -> InitializeResult {
        InitializeResult {
            protocol,
            plugin: Peer {
                name: "Fake plugin".to_owned(),
                version: "0.0.1".to_owned(),
            },
            plugin_capabilities: PluginCapabilities {
                commands: vec!["echo".to_owned()],
                providers: vec![ProviderCapability {
                    scheme: "fake".to_owned(),
                    methods: vec!["list".to_owned(), "stat".to_owned()],
                }],
                ..PluginCapabilities::default()
            },
            warnings: Vec::new(),
        }
    }

    /// Reads one line the host sent and returns the lines it writes back.
    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        if self.mode == Mode::Silent || self.exited {
            return Vec::new();
        }
        let raw = RawLine {
            bytes: line.as_bytes().to_vec(),
            length: line.len() as u64,
            terminated: true,
        };
        let text = match classify(&raw) {
            Frame::Message(text) => text,
            Frame::NotUtf8(_) => {
                return vec![answer(Response::failure(
                    None,
                    RpcError::new(ErrorKind::ParseError, "the line is not UTF-8"),
                ))];
            }
            Frame::Stray(_) | Frame::Blank | Frame::TooLong { .. } => return Vec::new(),
        };
        let incoming = match parse_line(&text) {
            Ok(incoming) => incoming,
            Err(error) => return vec![answer(Response::failure(None, error))],
        };
        self.transcript.received.push(incoming.clone());
        match self.life.gate(&incoming) {
            Gate::Proceed => {}
            Gate::Refuse(error) => {
                let Incoming::Request(request) = incoming else {
                    return Vec::new();
                };
                return vec![answer(Response::failure(Some(request.id), error))];
            }
            Gate::Drop => return Vec::new(),
            Gate::Exit => {
                self.exited = true;
                return Vec::new();
            }
        }
        let Incoming::Request(request) = incoming else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let result = match request.method.as_str() {
            methods::INITIALIZE => match self.life.initialize(&request.params) {
                Ok(_) => {
                    if self.mode == Mode::Chatty {
                        out.push("starting up, please wait".to_owned());
                    }
                    let protocol = match self.mode {
                        Mode::Protocol(n) => n,
                        _ => crate::protocol::PROTOCOL,
                    };
                    serde_json::to_value(Self::initialize_result(protocol))
                        .map_err(|e| RpcError::new(ErrorKind::InternalError, e.to_string()))
                }
                Err(error) => Err(error),
            },
            methods::SHUTDOWN => {
                self.life.shut_down();
                Ok(Value::Null)
            }
            methods::COMMAND_RUN => {
                let token = request
                    .params
                    .get("progress_token")
                    .cloned()
                    .unwrap_or(Value::Null);
                out.extend(self.chatter(&token));
                Ok(json!({"done": 1, "skipped": 0, "failed": []}))
            }
            _ => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {}", request.method),
            )),
        };
        out.push(answer(match result {
            Ok(value) => Response::success(request.id, value),
            Err(error) => Response::failure(Some(request.id), error),
        }));
        out
    }

    /// What the talking modes write while a command runs.
    fn chatter(&self, token: &Value) -> Vec<String> {
        let announce = |text: &str| {
            let mut params = Params::new();
            params.insert("text".into(), json!(text));
            Self::notification(methods::UI_ANNOUNCE, params)
        };
        let progress = |value: Value| {
            let mut params = Params::new();
            params.insert("token".into(), token.clone());
            params.insert("value".into(), value);
            Self::notification(methods::PROGRESS, params)
        };
        match self.mode {
            Mode::Talk => vec![
                progress(json!({"kind": "begin", "title": "Count lines"})),
                progress(
                    json!({"kind": "report", "message": "Counting", "current": 1, "total": 3, "unit": "files"}),
                ),
                announce("Lines: 42, files: 1"),
                progress(json!({"kind": "end", "message": "Count lines done: 3 files"})),
            ],
            Mode::Flood => (0..100)
                .map(|i| announce(&format!("Message {i}")))
                .collect(),
            Mode::Spoof => vec![announce("\u{1B}[2J\u{202E}Moved to the trash")],
            Mode::Stray => vec![
                "debug: one".to_owned(),
                "debug: two".to_owned(),
                "debug: three".to_owned(),
            ],
            _ => Vec::new(),
        }
    }

    /// Serves a memory transport until the host closes it or sends `exit`,
    /// and returns what it saw.
    pub fn serve(mut self, peer: MemoryPeer) -> Transcript {
        let mut peer = peer;
        while let Ok(line) = peer.from_host.recv() {
            for out in self.handle_line(&line) {
                if !peer.write_line(&out) {
                    return self.transcript;
                }
            }
            if self.exited {
                break;
            }
        }
        peer.end_output();
        self.transcript
    }

    /// Sends a notification to the host, as a plugin may at any time.
    pub fn notification(method: &str, params: Params) -> String {
        answer(Notification::new(method, params))
    }
}

fn answer<T: serde::Serialize>(message: T) -> String {
    to_line(&message).unwrap_or_default()
}

/// Whether a value matches an expected one, where `"*"` matches anything.
pub fn json_matches(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::String(s), _) if s == "*" => true,
        (Value::Object(e), Value::Object(a)) => {
            e.len() == a.len()
                && e.iter()
                    .all(|(k, v)| a.get(k).is_some_and(|got| json_matches(v, got)))
        }
        (Value::Array(e), Value::Array(a)) => {
            e.len() == a.len() && e.iter().zip(a).all(|(x, y)| json_matches(x, y))
        }
        _ => expected == actual,
    }
}

/// Plays a dialogue against a fake plugin; the error says which line
/// failed and why.
pub fn run_dialogue(text: &str, mut plugin: FakePlugin) -> Result<(), String> {
    let mut written: VecDeque<String> = VecDeque::new();
    for (number, line) in text.lines().enumerate() {
        let number = number + 1;
        if let Some(sent) = line.strip_prefix("> ") {
            written.extend(plugin.handle_line(sent));
        } else if line == "< exit" {
            if !plugin.exited() {
                return Err(format!("line {number}: the session has not ended"));
            }
        } else if let Some(wanted) = line.strip_prefix("< ") {
            let wanted: Value = serde_json::from_str(wanted)
                .map_err(|e| format!("line {number}: the expected line is not JSON: {e}"))?;
            let Some(got) = written.pop_front() else {
                return Err(format!("line {number}: nothing was written"));
            };
            let value: Value = serde_json::from_str(&got)
                .map_err(|_| format!("line {number}: wrote a line that is not JSON: {got}"))?;
            if !json_matches(&wanted, &value) {
                return Err(format!("line {number}: wrote {got}"));
            }
        } else if !(line.trim().is_empty() || line.starts_with('#')) {
            return Err(format!("line {number}: not a dialogue line"));
        }
    }
    match written.pop_front() {
        Some(extra) => Err(format!("wrote more than expected: {extra}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wildcard_matches_any_value() {
        assert!(json_matches(&json!({"a": "*"}), &json!({"a": [1, 2]})));
        assert!(!json_matches(&json!({"a": "*"}), &json!({"a": 1, "b": 2})));
        assert!(!json_matches(&json!([1, "*"]), &json!([1])));
    }

    #[test]
    fn a_dialogue_fails_on_extra_output() {
        let text = "> {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}\n";
        assert!(run_dialogue(text, FakePlugin::new(Mode::Echo)).is_err());
    }
}
