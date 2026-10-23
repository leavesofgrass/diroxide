//! The session: `initialize`, `initialized`, `shutdown` and `exit`, in both
//! directions (R22, section 2.5).
//!
//! - **The initiating side** sends `initialize` and, when it accepts the
//!   answer, `initialized`; at the end it sends `shutdown`, waits for the
//!   null answer, sends `exit` and closes the stream. The host does this
//!   to a plugin ([`handshake`], [`polite_stop`]); a program connecting to
//!   `diro serve` does it to diroxide.
//! - **The answering side** keeps a [`Lifecycle`]: before `initialize` is
//!   answered every other request gets -32002 and every other notification
//!   is dropped; after `shutdown` only `exit` counts. A plugin keeps one,
//!   and `diro serve` keeps one for the program connected to it, so the
//!   same fixture dialogues check both.
//!
//! Both answers use one shape: the protocol's major version, the sender's
//! own name and version, and its capabilities, where absence means
//! unsupported. Nothing that names the owner travels: the locale and the
//! folders, never a user name.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::framing::Frame;
use crate::lines;
use crate::protocol::{
    ErrorKind, Id, Incoming, Notification, PROTOCOL, Params, Request, RpcError, methods,
    parse_line, to_line,
};
use crate::transport::PluginTransport;

/// Who is speaking: a name and a version.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    /// The program's name: "diroxide", or the plugin's name.
    #[serde(default)]
    pub name: String,
    /// Its version.
    #[serde(default)]
    pub version: String,
}

/// What the host offers a plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilities {
    /// The services on offer, by method name.
    #[serde(default)]
    pub services: Vec<String>,
    /// How file contents may travel: `inline` and `file`.
    #[serde(default)]
    pub transfer: Vec<String>,
    /// The longest line the host reads, in bytes.
    #[serde(default)]
    pub max_message_bytes: u64,
}

/// The folders a plugin may use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dirs {
    /// Its own folder.
    #[serde(default)]
    pub plugin: String,
    /// A private writable folder inside the state folder.
    #[serde(default)]
    pub state: String,
    /// Where file transfers land.
    #[serde(default)]
    pub spool: String,
}

/// The parameters of `initialize`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InitializeParams {
    /// The protocol's major version.
    pub protocol: u32,
    /// The host's name and version.
    #[serde(default)]
    pub host: Peer,
    /// The plugin's id, from its manifest.
    #[serde(default)]
    pub plugin_id: String,
    /// The host's process id; a plugin exits when it vanishes.
    #[serde(default)]
    pub process_id: u32,
    /// The owner's locale, such as `en-US`.
    #[serde(default)]
    pub locale: String,
    /// What the host offers.
    #[serde(default)]
    pub host_capabilities: HostCapabilities,
    /// The current values of the plugin's declared settings.
    #[serde(default)]
    pub settings: Map<String, Value>,
    /// The plugin's folders.
    #[serde(default)]
    pub dirs: Dirs,
}

impl InitializeParams {
    /// Parameters for a plugin, with this host's name, version and process
    /// id, and the services this version serves.
    pub fn for_plugin(plugin_id: &str) -> Self {
        Self {
            protocol: PROTOCOL,
            host: Peer {
                name: "diroxide".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            plugin_id: plugin_id.to_owned(),
            process_id: std::process::id(),
            locale: "en-US".to_owned(),
            host_capabilities: HostCapabilities {
                services: methods::services().map(|m| m.name.to_owned()).collect(),
                transfer: vec!["inline".to_owned(), "file".to_owned()],
                max_message_bytes: crate::protocol::MAX_LINE_BYTES as u64,
            },
            settings: Map::new(),
            dirs: Dirs::default(),
        }
    }
}

/// A provider the plugin implements, with its methods.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapability {
    /// The scheme.
    pub scheme: String,
    /// The methods, without the `provider/` prefix.
    #[serde(default)]
    pub methods: Vec<String>,
}

/// What a plugin confirms it implements; the host calls nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCapabilities {
    /// Command ids (plugin-local).
    #[serde(default)]
    pub commands: Vec<String>,
    /// Providers.
    #[serde(default)]
    pub providers: Vec<ProviderCapability>,
    /// Previewer ids.
    #[serde(default)]
    pub previewers: Vec<String>,
    /// Converter ids.
    #[serde(default)]
    pub converters: Vec<String>,
    /// Hooked events.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Lister ids.
    #[serde(default)]
    pub listers: Vec<String>,
    /// Column ids.
    #[serde(default)]
    pub columns: Vec<String>,
    /// Whether it describes places.
    #[serde(default)]
    pub place_info: bool,
    /// Whether it answers `completion/query`.
    #[serde(default)]
    pub completion: bool,
    /// How many host requests it takes at once, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    /// How file contents may travel to it.
    #[serde(default)]
    pub transfer: Vec<String>,
}

/// The answer to `initialize`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeResult {
    /// The protocol's major version the answering side speaks.
    pub protocol: u32,
    /// Its name and version.
    #[serde(default)]
    pub plugin: Peer,
    /// What it implements.
    #[serde(default)]
    pub plugin_capabilities: PluginCapabilities,
    /// Words for the log.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// What the answering side does with a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    /// Handle it.
    Proceed,
    /// Answer the request with this error and do nothing else.
    Refuse(RpcError),
    /// Drop it silently (a notification that came too early or too late).
    Drop,
    /// End the process: `exit` came.
    Exit,
}

/// Where the answering side is in the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for `initialize`.
    #[default]
    New,
    /// `initialize` was answered; requests may flow both ways.
    Running,
    /// `shutdown` was answered; only `exit` counts.
    ShuttingDown,
    /// `exit` came.
    Exited,
}

/// The answering side's half of the session.
#[derive(Clone, Debug, Default)]
pub struct Lifecycle {
    phase: Phase,
    initialized: bool,
}

impl Lifecycle {
    /// A session waiting for `initialize`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where it is.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether `initialized` has come.
    pub fn initialized(&self) -> bool {
        self.initialized
    }

    /// Decides what to do with an incoming message, before it is handled.
    /// Responses always proceed (they answer requests this side sent).
    pub fn gate(&mut self, incoming: &Incoming) -> Gate {
        match incoming {
            Incoming::Response(_) => Gate::Proceed,
            Incoming::Request(request) => self.gate_request(&request.method),
            Incoming::Notification(notification) => self.gate_notification(&notification.method),
        }
    }

    fn gate_request(&self, method: &str) -> Gate {
        match (self.phase, method) {
            (Phase::New, methods::INITIALIZE) => Gate::Proceed,
            (Phase::New, _) => Gate::Refuse(RpcError::new(
                ErrorKind::NotInitialized,
                "initialize has not been answered yet",
            )),
            (Phase::Running, methods::INITIALIZE) => Gate::Refuse(RpcError::new(
                ErrorKind::InvalidRequest,
                "initialize was already answered",
            )),
            (Phase::Running, _) => Gate::Proceed,
            (Phase::ShuttingDown | Phase::Exited, _) => Gate::Refuse(RpcError::new(
                ErrorKind::InvalidRequest,
                "shutting down, only exit is accepted",
            )),
        }
    }

    fn gate_notification(&mut self, method: &str) -> Gate {
        match (self.phase, method) {
            (_, methods::EXIT) => {
                self.phase = Phase::Exited;
                Gate::Exit
            }
            (Phase::New | Phase::ShuttingDown | Phase::Exited, _) => Gate::Drop,
            (Phase::Running, methods::INITIALIZED) => {
                self.initialized = true;
                Gate::Drop
            }
            (Phase::Running, _) => Gate::Proceed,
        }
    }

    /// Reads `initialize`'s parameters and checks the protocol; on success
    /// the session is running once the answer is sent.
    pub fn initialize(&mut self, params: &Params) -> Result<InitializeParams, RpcError> {
        let params: InitializeParams = serde_json::from_value(Value::Object(params.clone()))
            .map_err(|e| RpcError::new(ErrorKind::InvalidParams, format!("initialize: {e}")))?;
        if params.protocol != PROTOCOL {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!(
                    "the other side speaks protocol {}; this side speaks {PROTOCOL}",
                    params.protocol
                ),
            ));
        }
        self.phase = Phase::Running;
        Ok(params)
    }

    /// Records that `shutdown` was answered.
    pub fn shut_down(&mut self) {
        self.phase = Phase::ShuttingDown;
    }
}

/// Why a plugin could not be started or stopped; the text is the owner's
/// line, meaning first, the plugin's name first.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// No answer to `initialize` in time.
    #[error("{0}")]
    StartTimedOut(String),
    /// The plugin answered `initialize` with an error.
    #[error("{0}")]
    Refused(String),
    /// The plugin speaks another protocol.
    #[error("{0}")]
    Protocol(String),
    /// The plugin's output ended before it answered.
    #[error("{0}")]
    Ended(String),
    /// The answer could not be read, or a line could not be sent.
    #[error("{0}")]
    Broken(String),
}

/// What the host saw while waiting: lines it set aside for the router.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Started {
    /// The plugin's answer.
    pub result: InitializeResult,
    /// Messages that came after the answer, in order, for the router.
    pub early: Vec<Incoming>,
    /// Stray lines, for the log.
    pub stray: Vec<String>,
}

fn send_message<T: Serialize>(transport: &dyn PluginTransport, message: &T) -> Result<(), String> {
    let line = to_line(message).map_err(|e| e.to_string())?;
    transport.send(line).map_err(|e| e.to_string())
}

/// The host's side of the start: sends `initialize`, waits up to `timeout`
/// for the answer, checks the protocol, and sends `initialized`. Stray
/// lines are set aside for the log; anything else that comes before the
/// answer is a protocol error. `name` is the plugin's name for the lines.
pub fn handshake(
    transport: &dyn PluginTransport,
    name: &str,
    params: &InitializeParams,
    timeout: Duration,
) -> Result<Started, SessionError> {
    let id = Id::Number(0);
    let params = match serde_json::to_value(params) {
        Ok(Value::Object(map)) => map,
        _ => Params::new(),
    };
    send_message(
        transport,
        &Request::new(id.clone(), methods::INITIALIZE, params),
    )
    .map_err(|e| SessionError::Broken(lines::could_not_start(name, &e)))?;
    let deadline = Instant::now() + timeout;
    let mut started = Started::default();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(SessionError::StartTimedOut(lines::did_not_start(
                name, timeout,
            )));
        }
        let frame = match transport.receive(left) {
            Ok(Some(frame)) => frame,
            Ok(None) => continue,
            Err(_) => return Err(SessionError::Ended(lines::stopped_while_starting(name))),
        };
        let text = match frame {
            Frame::Message(text) => text,
            Frame::Stray(text) => {
                started.stray.push(text);
                continue;
            }
            Frame::Blank => continue,
            Frame::NotUtf8(_) | Frame::TooLong { .. } => {
                return Err(SessionError::Broken(lines::could_not_start(
                    name,
                    "its answer could not be read",
                )));
            }
        };
        match parse_line(&text) {
            Ok(Incoming::Response(response)) if response.id.as_ref() == Some(&id) => {
                if let Some(error) = response.error {
                    return Err(SessionError::Refused(lines::could_not_start(
                        name,
                        &error.message,
                    )));
                }
                let result: InitializeResult = serde_json::from_value(
                    response.result.unwrap_or(Value::Null),
                )
                .map_err(|e| SessionError::Broken(lines::could_not_start(name, &e.to_string())))?;
                if result.protocol != PROTOCOL {
                    return Err(SessionError::Protocol(lines::other_protocol(
                        name,
                        result.protocol,
                    )));
                }
                send_message(
                    transport,
                    &Notification::new(methods::INITIALIZED, Params::new()),
                )
                .map_err(|e| SessionError::Broken(lines::could_not_start(name, &e)))?;
                started.result = result;
                return Ok(started);
            }
            Ok(other) => started.early.push(other),
            Err(_) => {
                return Err(SessionError::Broken(lines::could_not_start(
                    name,
                    "its answer could not be read",
                )));
            }
        }
    }
}

/// How a plugin ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// It answered `shutdown` and exited when told to.
    Polite,
    /// It had to be killed.
    Killed,
}

/// The host's polite ending on a transport no router is reading: sends
/// `shutdown`, waits up to `grace` for the null answer, sends `exit`,
/// closes the input, waits up to two seconds for the output to end, and
/// kills the plugin's tree if it has not. Never waits longer than `grace`
/// and two seconds.
pub fn polite_stop(transport: &dyn PluginTransport, grace: Duration) -> Ending {
    let id = Id::Text("shutdown".to_owned());
    let mut answered = false;
    if send_message(
        transport,
        &Request::new(id.clone(), methods::SHUTDOWN, Params::new()),
    )
    .is_ok()
    {
        let deadline = Instant::now() + grace;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match transport.receive(left) {
                Ok(Some(Frame::Message(text))) => {
                    if let Ok(Incoming::Response(r)) = parse_line(&text)
                        && r.id.as_ref() == Some(&id)
                    {
                        answered = true;
                        break;
                    }
                }
                Ok(Some(_)) | Ok(None) => continue,
                Err(_) => break,
            }
        }
    }
    let _ = send_message(transport, &Notification::new(methods::EXIT, Params::new()));
    transport.close_input();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut ended = false;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match transport.receive(left) {
            Err(_) => {
                ended = true;
                break;
            }
            Ok(_) => continue,
        }
    }
    if answered && ended {
        Ending::Polite
    } else {
        let _ = transport.kill();
        Ending::Killed
    }
}

/// What a manifest declares that the plugin did not confirm in its
/// `initialize` answer, in words for the log; the host leaves those out.
pub fn unconfirmed(manifest: &crate::manifest::Manifest, caps: &PluginCapabilities) -> Vec<String> {
    let mut missing = Vec::new();
    for command in &manifest.commands {
        if command.argv.is_empty() && !caps.commands.contains(&command.id) {
            missing.push(format!("command {}", command.id));
        }
    }
    let confirmed: BTreeMap<&str, &Vec<String>> = caps
        .providers
        .iter()
        .map(|p| (p.scheme.as_str(), &p.methods))
        .collect();
    for provider in &manifest.providers {
        match confirmed.get(provider.scheme.as_str()) {
            None => missing.push(format!("provider {}", provider.scheme)),
            Some(methods) => {
                for method in &provider.methods {
                    if !methods.contains(method) {
                        missing.push(format!("provider {} method {method}", provider.scheme));
                    }
                }
            }
        }
    }
    for p in manifest.previewers.iter().filter(|p| p.argv.is_empty()) {
        if !caps.previewers.contains(&p.id) {
            missing.push(format!("previewer {}", p.id));
        }
    }
    for c in manifest.converters.iter().filter(|c| c.argv.is_empty()) {
        if !caps.converters.contains(&c.id) {
            missing.push(format!("converter {}", c.id));
        }
    }
    for hook in &manifest.hooks {
        if !caps.hooks.contains(&hook.event) {
            missing.push(format!("hook {}", hook.event));
        }
    }
    for lister in &manifest.listers {
        if !caps.listers.contains(&lister.id) {
            missing.push(format!("lister {}", lister.id));
        }
    }
    for column in &manifest.columns {
        if !caps.columns.contains(&column.id) {
            missing.push(format!("column {}", column.id));
        }
    }
    if !manifest.place_info.is_empty() && !caps.place_info {
        missing.push("place descriptions".to_owned());
    }
    missing
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::thread;

    use serde_json::json;

    use super::*;
    use crate::testing::{FakePlugin, Mode, run_dialogue};
    use crate::transport::MemoryTransport;

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn the_host_starts_and_stops_a_plugin_politely() {
        let (host, peer) = MemoryTransport::pair("Line counter").unwrap();
        let plugin = thread::spawn(move || FakePlugin::new(Mode::Echo).serve(peer));
        let started = handshake(
            &host,
            "Line counter",
            &InitializeParams::for_plugin("line-counter"),
            WAIT,
        )
        .unwrap();
        assert_eq!(started.result.protocol, PROTOCOL);
        assert_eq!(started.result.plugin.name, "Fake plugin");
        assert_eq!(started.result.plugin_capabilities.commands, ["echo"]);
        assert_eq!(polite_stop(&host, WAIT), Ending::Polite);
        let seen = plugin.join().unwrap();
        assert_eq!(
            seen.methods(),
            [
                methods::INITIALIZE,
                methods::INITIALIZED,
                methods::SHUTDOWN,
                methods::EXIT
            ]
        );
    }

    #[test]
    fn a_plugin_of_another_protocol_is_refused_in_words() {
        let (host, peer) = MemoryTransport::pair("Line counter").unwrap();
        let plugin = thread::spawn(move || FakePlugin::new(Mode::Protocol(2)).serve(peer));
        let err = handshake(
            &host,
            "Line counter",
            &InitializeParams::for_plugin("line-counter"),
            WAIT,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Line counter needs a newer diroxide: it speaks protocol 2."
        );
        let _ = polite_stop(&host, Duration::from_millis(200));
        drop(plugin);
    }

    #[test]
    fn a_silent_plugin_did_not_start() {
        let (host, peer) = MemoryTransport::pair("Line counter").unwrap();
        let plugin = thread::spawn(move || FakePlugin::new(Mode::Silent).serve(peer));
        let err = handshake(
            &host,
            "Line counter",
            &InitializeParams::for_plugin("line-counter"),
            Duration::from_millis(150),
        )
        .unwrap_err();
        assert_eq!(
            err,
            SessionError::StartTimedOut("Line counter did not start in under a second.".to_owned())
        );
        // A silent plugin never answers shutdown, so it is killed.
        assert_eq!(
            polite_stop(&host, Duration::from_millis(100)),
            Ending::Killed
        );
        drop(plugin);
    }

    #[test]
    fn stray_lines_before_the_answer_are_set_aside() {
        let (host, peer) = MemoryTransport::pair("Chatty").unwrap();
        let plugin = thread::spawn(move || FakePlugin::new(Mode::Chatty).serve(peer));
        let started = handshake(
            &host,
            "Chatty",
            &InitializeParams::for_plugin("chatty"),
            WAIT,
        )
        .unwrap();
        assert_eq!(started.stray, ["starting up, please wait"]);
        assert_eq!(polite_stop(&host, WAIT), Ending::Polite);
        drop(plugin);
    }

    #[test]
    fn initialize_params_carry_no_identity() {
        let params = InitializeParams::for_plugin("line-counter");
        let text = serde_json::to_string(&params).unwrap();
        for word in ["user", "USERNAME", "HOME", "USERPROFILE", "email"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
        assert!(
            params
                .host_capabilities
                .services
                .contains(&"ui/announce".to_owned())
        );
        assert!(
            !params
                .host_capabilities
                .services
                .contains(&"ui/form".to_owned())
        );
    }

    #[test]
    fn the_lifecycle_refuses_and_drops_in_order() {
        let mut life = Lifecycle::new();
        let request =
            |method: &str| Incoming::Request(Request::new(Id::Number(1), method, Params::new()));
        let note = |method: &str| Incoming::Notification(Notification::new(method, Params::new()));
        assert!(matches!(
            life.gate(&request("command/run")),
            Gate::Refuse(e) if e.code == -32002
        ));
        assert_eq!(life.gate(&note("settings/changed")), Gate::Drop);
        assert_eq!(life.gate(&request(methods::INITIALIZE)), Gate::Proceed);
        let mut params = Params::new();
        params.insert("protocol".into(), json!(1));
        life.initialize(&params).unwrap();
        assert_eq!(life.phase(), Phase::Running);
        assert_eq!(life.gate(&note(methods::INITIALIZED)), Gate::Drop);
        assert!(life.initialized());
        assert_eq!(life.gate(&request("command/run")), Gate::Proceed);
        assert!(matches!(
            life.gate(&request(methods::INITIALIZE)),
            Gate::Refuse(e) if e.code == -32600
        ));
        life.shut_down();
        assert!(matches!(
            life.gate(&request("command/run")),
            Gate::Refuse(_)
        ));
        assert_eq!(life.gate(&note("settings/changed")), Gate::Drop);
        assert_eq!(life.gate(&note(methods::EXIT)), Gate::Exit);
        let mut other = Lifecycle::new();
        params.insert("protocol".into(), json!(2));
        assert_eq!(
            other.initialize(&params).unwrap_err().message,
            "the other side speaks protocol 2; this side speaks 1"
        );
        assert_eq!(other.phase(), Phase::New);
    }

    /// The fixture dialogues, against the in-process fake plugin: the same
    /// files serve `diro serve`, whose answering side keeps a [`Lifecycle`]
    /// too.
    #[test]
    fn the_session_dialogues_hold() {
        let folder =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plugins/dialogues");
        let mut count = 0;
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "txt") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            if let Err(e) = run_dialogue(&text, FakePlugin::new(Mode::Echo)) {
                panic!("{}: {e}", path.display());
            }
            count += 1;
        }
        assert!(count >= 4, "only {count} dialogues");
    }
}
