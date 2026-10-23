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

pub mod memfs;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ask::{Answer, Question, TextDocument};
use crate::error::PluginError;
use crate::framing::{Frame, RawLine, classify};
use crate::host::{HostServices, Launcher, PluginInfo};
use crate::log::PluginLog;
use crate::manifest::Manifest;
use crate::panels::{PanelChange, PanelChoice, PanelSnapshot};
use crate::protocol::{
    ErrorKind, Id, Incoming, Notification, Params, Request, Response, RpcError, methods,
    parse_line, to_line,
};
use crate::router::CancelFlag;
use crate::session::{
    Gate, InitializeResult, Lifecycle, Peer, PluginCapabilities, ProviderCapability,
};
use crate::transport::{MemoryPeer, MemoryTransport, PluginTransport};
use diroxide_core::Message;
use diroxide_core::{ConflictAnswer, ErrorAction, JobRequest};
use diroxide_ops::{
    Conflict, Engine, ItemError, JobControl, JobProgress, JobReport, JobSink, Plan,
};
use memfs::MemFs;

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
    /// Says its provider lost a device while it runs a command.
    Lost,
    /// Makes the host call its command's arguments name (`call.method` and
    /// `call.params`) and answers the command with what the host answered,
    /// under `host` or `host_error`.
    Call,
}

/// Starts in-process fake plugins in a mode, over memory transports, and
/// keeps what each one saw.
#[derive(Debug)]
pub struct FakeLauncher {
    mode: Mutex<Mode>,
    launches: Mutex<usize>,
    seen: Arc<Mutex<Vec<Transcript>>>,
    memfs: Mutex<Option<Arc<Mutex<MemFs>>>>,
}

impl FakeLauncher {
    /// A launcher of fakes in a mode.
    pub fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode: Mutex::new(mode),
            launches: Mutex::new(0),
            seen: Arc::new(Mutex::new(Vec::new())),
            memfs: Mutex::new(None),
        })
    }

    /// Serves this toy file system in every fake it starts, so its state
    /// outlives a plugin that is killed.
    pub fn with_memfs(&self, memfs: Arc<Mutex<MemFs>>) {
        *self.memfs.lock().unwrap_or_else(PoisonError::into_inner) = Some(memfs);
    }

    /// The mode for the next start.
    pub fn set_mode(&self, mode: Mode) {
        *self.mode.lock().unwrap_or_else(PoisonError::into_inner) = mode;
    }

    /// How many fakes it started.
    pub fn launches(&self) -> usize {
        *self.launches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the fakes saw, once at least `count` have ended (waiting up to
    /// five seconds).
    pub fn transcripts_after(&self, count: usize) -> Vec<Transcript> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
            if seen.len() >= count || Instant::now() >= deadline {
                return seen.clone();
            }
            drop(seen);
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Launcher for FakeLauncher {
    fn launch(
        &self,
        info: &PluginInfo,
        _log: &PluginLog,
    ) -> Result<Arc<dyn PluginTransport>, PluginError> {
        *self.launches.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        let mode = *self.mode.lock().unwrap_or_else(PoisonError::into_inner);
        let (host, peer) = MemoryTransport::pair(&info.manifest.name)?;
        let seen = Arc::clone(&self.seen);
        let memfs = self
            .memfs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(fs) = &memfs {
            fs.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .forget_sessions();
        }
        thread::Builder::new()
            .name("fake-plugin".to_owned())
            .spawn(move || {
                let mut fake = FakePlugin::new(mode);
                fake.memfs = memfs;
                let transcript = fake.serve(peer);
                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(transcript);
            })
            .map_err(|e| PluginError::Io(e.to_string()))?;
        Ok(Arc::new(host))
    }
}

/// The manifest of the fake plugin, `line-counter` ("Line counter"), with
/// one command `echo` ("Echo") and `extra` lines in its `[run]` table and
/// after it.
pub fn plugin_info(extra: &str) -> PluginInfo {
    let text = format!(
        "[plugin]\nid = \"line-counter\"\nname = \"Line counter\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[run]\ncommand = [\"x\"]\nstartup_timeout = \"1s\"\n{extra}\n[[command]]\nid = \"echo\"\nname = \"Echo\"\n"
    );
    let report = Manifest::check(&text);
    let problems = report.text();
    let manifest = report
        .manifest
        .unwrap_or_else(|| panic!("the fake manifest does not read: {problems}"));
    PluginInfo {
        manifest,
        folder: std::path::PathBuf::from("."),
        enabled: true,
    }
}

type Responder = Box<dyn FnMut(&Question) -> Answer + Send>;

/// Host services that record everything a plugin causes and answer from a
/// script: the stand-ins of the fake host (R22, section 8.1). Questions are
/// answered by the responder when one is set, else from the queue of
/// answers, else with Escape.
#[derive(Default)]
pub struct RecordingServices {
    said: Mutex<Vec<Message>>,
    questions: Mutex<Vec<Question>>,
    documents: Mutex<Vec<TextDocument>>,
    answers: Mutex<VecDeque<Answer>>,
    responder: Mutex<Option<Responder>>,
    panels: Mutex<Vec<PanelSnapshot>>,
    changes: Mutex<Vec<PanelChange>>,
    executed: Mutex<Vec<(String, Params)>>,
    engine: Mutex<Option<(Engine, bool)>>,
    jobs: Mutex<Vec<JobRequest>>,
    plans: Mutex<Vec<Plan>>,
}

/// A job sink that skips every conflict and every error, for the stand-in
/// host's jobs.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkippingSink;

impl JobSink for SkippingSink {
    fn progress(&self, _progress: &JobProgress) {}

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

impl std::fmt::Debug for RecordingServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingServices").finish_non_exhaustive()
    }
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl RecordingServices {
    /// Services with nothing recorded and no answers.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Queues an answer for the next question.
    pub fn answer(&self, answer: Answer) {
        locked(&self.answers).push_back(answer);
    }

    /// Answers every question through `responder` (it may block, as the
    /// owner does).
    pub fn respond(&self, responder: impl FnMut(&Question) -> Answer + Send + 'static) {
        *locked(&self.responder) = Some(Box::new(responder));
    }

    /// What was said, in order.
    pub fn said(&self) -> Vec<String> {
        locked(&self.said).iter().map(|m| m.text.clone()).collect()
    }

    /// The questions asked, in order.
    pub fn questions(&self) -> Vec<Question> {
        locked(&self.questions).clone()
    }

    /// The documents shown, in order.
    pub fn documents(&self) -> Vec<TextDocument> {
        locked(&self.documents).clone()
    }

    /// Sets the panels `panel/get` reads: the first is active and on the
    /// left, the second the other.
    pub fn set_panels(&self, panels: Vec<PanelSnapshot>) {
        *locked(&self.panels) = panels;
    }

    /// The panel changes asked for, in order.
    pub fn changes(&self) -> Vec<PanelChange> {
        locked(&self.changes).clone()
    }

    /// The commands run, by id, with their arguments.
    pub fn executed(&self) -> Vec<(String, Params)> {
        locked(&self.executed).clone()
    }

    /// Plans the plugins' file jobs with this engine, the built-in jobs'
    /// own planner, and runs them when `run` is set (never a trash job in
    /// a test: the trash is the real one).
    pub fn plan_with(&self, engine: Engine, run: bool) {
        *locked(&self.engine) = Some((engine, run));
    }

    /// The file jobs the plugins asked for, in order.
    pub fn jobs(&self) -> Vec<JobRequest> {
        locked(&self.jobs).clone()
    }

    /// Their plans, in order.
    pub fn plans(&self) -> Vec<Plan> {
        locked(&self.plans).clone()
    }
}

impl HostServices for RecordingServices {
    fn announce(&self, message: Message) {
        locked(&self.said).push(message);
    }

    fn ask(&self, question: &Question) -> Answer {
        locked(&self.questions).push(question.clone());
        if let Some(responder) = locked(&self.responder).as_mut() {
            return responder(question);
        }
        locked(&self.answers)
            .pop_front()
            .unwrap_or(Answer::Cancelled)
    }

    fn show_text(&self, document: &TextDocument) -> Result<(), RpcError> {
        locked(&self.documents).push(document.clone());
        Ok(())
    }

    fn panel(&self, which: PanelChoice) -> Option<PanelSnapshot> {
        let panels = locked(&self.panels);
        let index = match which {
            PanelChoice::Active | PanelChoice::Left => 0,
            PanelChoice::Other | PanelChoice::Right => 1,
        };
        panels.get(index).cloned()
    }

    fn run_job(&self, request: &JobRequest, cancel: &CancelFlag) -> Result<JobReport, RpcError> {
        locked(&self.jobs).push(request.clone());
        let engine = locked(&self.engine).clone();
        let Some((engine, run)) = engine else {
            return Ok(JobReport::new(request.id, request.kind));
        };
        let plan = engine
            .plan(request)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        locked(&self.plans).push(plan.clone());
        if !run {
            return Ok(JobReport::new(request.id, request.kind));
        }
        let mut control = JobControl::new();
        control.cancel = cancel.token();
        Ok(engine.run(&plan, &control, &SkippingSink))
    }

    fn set_panel(&self, change: &PanelChange) -> Result<(), RpcError> {
        locked(&self.changes).push(change.clone());
        Ok(())
    }

    fn execute(
        &self,
        command: &diroxide_core::CommandId,
        args: &Params,
    ) -> Result<Value, RpcError> {
        locked(&self.executed).push((command.to_string(), args.clone()));
        Ok(Value::Null)
    }
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
    /// The command waiting for the host's answer, and the id of the call
    /// it made.
    waiting: Option<(Id, Id)>,
    calls: u64,
    /// Completion queries held until they are cancelled.
    held: Vec<Id>,
    /// The toy file system it serves as `memfs`, when it has one.
    pub memfs: Option<Arc<Mutex<MemFs>>>,
}

impl FakePlugin {
    /// A fake plugin in a mode.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            life: Lifecycle::new(),
            transcript: Transcript::default(),
            exited: false,
            waiting: None,
            calls: 0,
            held: Vec::new(),
            memfs: None,
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
                providers: vec![
                    ProviderCapability {
                        scheme: "fake".to_owned(),
                        methods: vec!["list".to_owned(), "stat".to_owned()],
                    },
                    ProviderCapability {
                        scheme: memfs::SCHEME.to_owned(),
                        methods: memfs::METHODS.iter().map(|m| (*m).to_owned()).collect(),
                    },
                ],
                completion: true,
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
        let request = match incoming {
            Incoming::Request(request) => request,
            Incoming::Response(response) => return self.host_answered(response),
            Incoming::Notification(notification) => return self.notified(&notification),
        };
        let mut out = Vec::new();
        let result = match request.method.as_str() {
            methods::COMPLETION_QUERY => {
                let text = request
                    .params
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if text.ends_with("slow") {
                    self.held.push(request.id);
                    return Vec::new();
                }
                Ok(json!({"candidates": [
                    {"text": format!("{text}-one"), "kind": "branch"},
                    {"text": format!("{text}-two"), "kind": "branch"}
                ]}))
            }
            name if name.starts_with("provider/") && self.memfs.is_some() => {
                let token = request
                    .params
                    .get("partial_token")
                    .cloned()
                    .unwrap_or(Value::Null);
                let mut partials = Vec::new();
                let result = match &self.memfs {
                    Some(fs) => fs.lock().unwrap_or_else(PoisonError::into_inner).serve(
                        &name["provider/".len()..],
                        &request.params,
                        &mut partials,
                    ),
                    None => Err(RpcError::new(ErrorKind::MethodNotFound, "no provider")),
                };
                for value in partials {
                    let mut params = Params::new();
                    params.insert("token".into(), token.clone());
                    params.insert("value".into(), value);
                    out.push(Self::notification(methods::PROGRESS, params));
                }
                result
            }
            methods::COMMAND_RUN if self.mode == Mode::Call => {
                let call = request
                    .params
                    .get("args")
                    .and_then(|a| a.get("call"))
                    .or_else(|| request.params.get("call"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let method = call
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or(methods::UI_ASK)
                    .to_owned();
                let params = call
                    .get("params")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                self.calls += 1;
                let id = Id::Text(format!("host-{}", self.calls));
                self.waiting = Some((request.id, id.clone()));
                return vec![answer(Request::new(id, &method, params))];
            }
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

    /// The host answered a call the plugin made: the command that waited
    /// for it is answered now.
    fn host_answered(&mut self, response: Response) -> Vec<String> {
        let Some((command, call)) = self.waiting.take() else {
            return Vec::new();
        };
        if response.id.as_ref() != Some(&call) {
            self.waiting = Some((command, call));
            return Vec::new();
        }
        let mut value = json!({"done": 1, "skipped": 0, "failed": []});
        if let Some(fields) = value.as_object_mut() {
            match (response.result, response.error) {
                (_, Some(error)) => {
                    fields.insert("host_error".into(), json!(error));
                }
                (result, None) => {
                    fields.insert("host".into(), result.unwrap_or(Value::Null));
                }
            }
        }
        vec![answer(Response::success(command, value))]
    }

    /// A notification from the host: a cancel answers a held query.
    fn notified(&mut self, notification: &Notification) -> Vec<String> {
        if notification.method != methods::CANCEL_REQUEST {
            return Vec::new();
        }
        let Some(id) = notification
            .params
            .get("id")
            .and_then(|v| serde_json::from_value::<Id>(v.clone()).ok())
        else {
            return Vec::new();
        };
        let before = self.held.len();
        self.held.retain(|held| held != &id);
        if self.held.len() == before {
            return Vec::new();
        }
        vec![answer(Response::failure(
            Some(id),
            RpcError::new(ErrorKind::RequestCancelled, "cancelled"),
        ))]
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
            Mode::Lost => {
                let mut params = Params::new();
                params.insert("authority".into(), json!("memfs://box"));
                params.insert("reason".into(), json!("Box unplugged\u{7}"));
                vec![Self::notification(methods::PROVIDER_LOST, params)]
            }
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
