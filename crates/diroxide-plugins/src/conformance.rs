//! The conformance suite: a fake host that drives any process plugin
//! through what the protocol asks of it (R22, sections 8.1 and 8.2), for
//! `diro plugin conformance PATH`, the examples' CI job and a plugin
//! author.
//!
//! It reads the plugin's manifest, starts the plugin through the real
//! process helper and transport, and plays scripted exchanges against it,
//! answering the plugin's own requests from stand-ins that record
//! everything. Each check group ends in one line, its meaning first:
//! "Pass: Lifecycle", "Fail: Framing: a line that is not JSON was not
//! answered -32700", or "Skipped: Providers: none declared". The run
//! passes when no group fails.
//!
//! The groups: the manifest and its accessibility lint; the lifecycle
//! (-32002 before `initialize`, the answer's shape, `shutdown` answered
//! null, exit code 0 after `exit`, exit at the end of input); framing (a
//! line that is not JSON answered -32700 with a null id, a batch refused
//! with -32600, CRLF accepted); methods (an unknown request -32601, an
//! unknown notification ignored, unknown fields ignored); each command
//! with no selection, one file, many, a name that is not Unicode, a name
//! with a line break and one starting with a dash, and its progress in
//! order; cancel; settings; providers and hooks when declared; the
//! services the plugin calls, each within its manifest; and output
//! hygiene (stray lines are counted, never fatal).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_core::{CommandId, Entry, EntryKind, Place};
use serde_json::{Value, json};

use crate::error::PluginError;
use crate::framing::Frame;
use crate::host::{Launcher, PluginInfo, ProcessLauncher};
use crate::lines;
use crate::log::PluginLog;
use crate::manifest::{Access, Manifest, Severity};
use crate::protocol::methods::{self, Availability, Call, Sender};
use crate::protocol::{
    ErrorKind, Id, Incoming, Notification, Params, Request, Response, parse_line, to_line,
};
use crate::session::InitializeParams;
use crate::shapes::{PanelRef, WireContext, WireEntry};
use crate::transport::PluginTransport;

/// How long the suite waits for an answer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(10);

/// How long it listens for what must not come (an answer to a
/// notification).
pub const QUIET_WAIT: Duration = Duration::from_millis(300);

/// A group of checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// The manifest reads.
    Manifest,
    /// Names and descriptions keep the 40-cell rule.
    Accessibility,
    /// Starting and stopping.
    Lifecycle,
    /// Lines in and out.
    Framing,
    /// Unknown methods and fields.
    Methods,
    /// Each declared command.
    Commands,
    /// `$/cancel_request`.
    Cancel,
    /// `settings/changed`.
    Settings,
    /// The declared providers.
    Providers,
    /// The declared hooks.
    Hooks,
    /// The services the plugin calls.
    Services,
    /// Stray output.
    Hygiene,
}

impl Group {
    /// Every group, in the report's order.
    pub const ALL: [Group; 12] = [
        Group::Manifest,
        Group::Accessibility,
        Group::Lifecycle,
        Group::Framing,
        Group::Methods,
        Group::Commands,
        Group::Cancel,
        Group::Settings,
        Group::Providers,
        Group::Hooks,
        Group::Services,
        Group::Hygiene,
    ];

    /// The group's name in the report.
    pub const fn name(self) -> &'static str {
        match self {
            Group::Manifest => "Manifest",
            Group::Accessibility => "Accessibility",
            Group::Lifecycle => "Lifecycle",
            Group::Framing => "Framing",
            Group::Methods => "Methods",
            Group::Commands => "Commands",
            Group::Cancel => "Cancel",
            Group::Settings => "Settings",
            Group::Providers => "Providers",
            Group::Hooks => "Hooks",
            Group::Services => "Services",
            Group::Hygiene => "Output hygiene",
        }
    }
}

/// How a group ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every check held; with notes that are not failures.
    Pass(Vec<String>),
    /// What failed, each in words.
    Fail(Vec<String>),
    /// Why it was not run.
    Skipped(String),
}

/// The suite's report on one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The plugin's name, or its folder when the manifest does not read.
    pub plugin: String,
    /// Each group's verdict, in order.
    pub groups: Vec<(Group, Verdict)>,
}

impl Report {
    /// Whether no group failed.
    pub fn passed(&self) -> bool {
        !self
            .groups
            .iter()
            .any(|(_, v)| matches!(v, Verdict::Fail(_)))
    }

    /// A group's verdict.
    pub fn verdict(&self, group: Group) -> Option<&Verdict> {
        self.groups
            .iter()
            .find(|(g, _)| *g == group)
            .map(|(_, v)| v)
    }

    /// The report in lines, each group's meaning first: "Pass: Lifecycle",
    /// "Fail: Methods: an unknown request was not answered -32601", then
    /// the totals with the plugin's name last.
    pub fn text(&self) -> String {
        let mut text = String::new();
        let (mut passed, mut failed, mut skipped) = (0, 0, 0);
        for (group, verdict) in &self.groups {
            match verdict {
                Verdict::Pass(notes) => {
                    passed += 1;
                    let _ = writeln!(text, "Pass: {}", group.name());
                    for note in notes {
                        let _ = writeln!(text, "  Note: {note}");
                    }
                }
                Verdict::Fail(problems) => {
                    failed += 1;
                    for problem in problems {
                        let _ = writeln!(text, "Fail: {}: {problem}", group.name());
                    }
                }
                Verdict::Skipped(why) => {
                    skipped += 1;
                    let _ = writeln!(text, "Skipped: {}: {why}", group.name());
                }
            }
        }
        let _ = writeln!(
            text,
            "{}",
            lines::with_source(
                &format!("Conformance: {passed} passed, {failed} failed, {skipped} skipped"),
                &self.plugin
            )
        );
        text
    }
}

/// The plugin's process as the fake host sees it.
struct Driver {
    transport: Arc<dyn PluginTransport>,
    manifest: Manifest,
    next: i64,
    stray: usize,
    unreadable: usize,
    /// The plugin's own requests, by method.
    called: Vec<String>,
    /// Calls outside the manifest's permissions.
    outside: Vec<String>,
    /// Progress kinds per token, in order.
    progress: HashMap<String, Vec<String>>,
    folder: String,
}

/// What a wait for an answer found.
enum Waited {
    Answer(Response),
    Nothing,
    Ended,
}

impl Driver {
    fn send_line(&self, line: String) -> bool {
        self.transport.send(line).is_ok()
    }

    fn send<T: serde::Serialize>(&self, message: &T) -> bool {
        to_line(message).is_ok_and(|line| self.send_line(line))
    }

    fn request(&mut self, method: &str, params: Params) -> Id {
        self.next += 1;
        let id = Id::Text(format!("c{}", self.next));
        self.send(&Request::new(id.clone(), method, params));
        id
    }

    fn notify(&self, method: &str, params: Params) -> bool {
        self.send(&Notification::new(method, params))
    }

    /// Reads until the answer for `id` (or, with `None`, an answer with a
    /// null id), serving the plugin's own requests on the way.
    fn wait(&mut self, id: Option<&Id>, timeout: Duration) -> Waited {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Waited::Nothing;
            }
            let frame = match self.transport.receive(left.min(Duration::from_millis(100))) {
                Ok(Some(frame)) => frame,
                Ok(None) => continue,
                Err(_) => return Waited::Ended,
            };
            let text = match frame {
                Frame::Message(text) => text,
                Frame::Stray(_) => {
                    self.stray += 1;
                    continue;
                }
                Frame::NotUtf8(_) | Frame::TooLong { .. } => {
                    self.unreadable += 1;
                    continue;
                }
                Frame::Blank => continue,
            };
            match parse_line(&text) {
                Ok(Incoming::Response(response)) => {
                    if response.id.as_ref() == id {
                        return Waited::Answer(response);
                    }
                }
                Ok(Incoming::Request(request)) => self.serve(request),
                Ok(Incoming::Notification(notification)) => self.heard(&notification),
                Err(_) => self.unreadable += 1,
            }
        }
    }

    /// Waits up to `timeout` for the plugin's output to end, whatever it
    /// writes meanwhile.
    fn ends_within(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            if matches!(self.wait(None, left), Waited::Ended) {
                return true;
            }
        }
    }

    /// Listens for `period`; true when no answer came.
    fn quiet(&mut self, period: Duration) -> bool {
        matches!(self.wait(None, period), Waited::Nothing)
    }

    fn heard(&mut self, notification: &Notification) {
        if notification.method == methods::PROGRESS
            && let (Some(token), Some(kind)) = (
                notification.params.get("token").and_then(Value::as_str),
                notification
                    .params
                    .get("value")
                    .and_then(|v| v.get("kind"))
                    .and_then(Value::as_str),
            )
        {
            self.progress
                .entry(token.to_owned())
                .or_default()
                .push(kind.to_owned());
        }
    }

    /// The stand-in services: everything recorded, anything outside the
    /// manifest noted, the owner's dialogs answered with Escape.
    fn serve(&mut self, request: Request) {
        let method = request.method.clone();
        self.called.push(method.clone());
        let permissions = &self.manifest.permissions;
        let named = |key: &str| {
            request
                .params
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let allowed = match method.as_str() {
            methods::PANEL_GET => permissions.panel >= Access::Read,
            methods::PANEL_SET => permissions.panel == Access::Write,
            methods::COMMAND_EXECUTE => permissions.allows_command(&named("command")),
            methods::FS_LIST | methods::FS_STAT | methods::FS_READ => {
                permissions.fs >= Access::Read
            }
            m if m.starts_with("fs/") => permissions.fs == Access::Write,
            methods::TOOLS_RUN => permissions.tools.contains(&named("tool")),
            methods::TOOLS_LAUNCH => permissions.launch.contains(&named("program")),
            _ => true,
        };
        let offered = methods::find(&method).is_some_and(|m| {
            m.sender != Sender::Host
                && m.call == Call::Request
                && m.availability == Availability::Served
        });
        let outcome = if !offered {
            self.outside
                .push(format!("called {method}, which the host never offered"));
            Err(crate::protocol::RpcError::new(
                ErrorKind::MethodNotFound,
                format!("the host does not serve {method}"),
            ))
        } else if !allowed {
            self.outside
                .push(format!("called {method} without its permission"));
            Err(crate::protocol::RpcError::new(
                ErrorKind::DeniedByHost,
                format!("the manifest does not allow {method} here"),
            ))
        } else {
            Ok(match method.as_str() {
                methods::UI_ASK => json!({"cancelled": true}),
                methods::UI_SHOW_TEXT => json!({"shown": true}),
                methods::PANEL_GET => json!({"panel": "left", "active": true,
                    "place": self.folder, "provider": "local", "cursor": null,
                    "selection": [], "selection_count": 0}),
                methods::PANEL_SET => json!({"applied": true}),
                methods::LISTER_OPEN => json!({"opened": true, "rows": 0}),
                methods::TOOLS_FIND => json!({"found": false, "path": null, "version": null}),
                methods::COMMAND_EXECUTE => Value::Null,
                _ => {
                    let response = Response::failure(
                        Some(request.id),
                        crate::protocol::RpcError::new(
                            ErrorKind::Unsupported,
                            "the conformance host reaches no files or tools",
                        ),
                    );
                    self.send(&response);
                    return;
                }
            })
        };
        let response = match outcome {
            Ok(value) => Response::success(request.id, value),
            Err(error) => Response::failure(Some(request.id), error),
        };
        self.send(&response);
    }
}

fn code_of(response: &Response) -> Option<i64> {
    response.error.as_ref().map(|e| e.code)
}

/// Starts the plugin as the host would, in a scratch state folder.
fn start(info: &PluginInfo, state: &Path) -> Result<Arc<dyn PluginTransport>, PluginError> {
    let launcher = ProcessLauncher {
        state_dir: Some(state.to_path_buf()),
    };
    let log = PluginLog::at(state.join(&info.manifest.id).join("conformance.log"));
    launcher.launch(info, &log)
}

fn entry(folder: &Place, name: &str) -> WireEntry {
    let place = folder.join(name).ok();
    WireEntry::from_entry(&Entry::new(name, EntryKind::File), place.as_ref())
}

/// The selections each command is run with: none, one file, many, a name
/// that is not Unicode, a name with a line break, a name starting with a
/// dash. The names exist only in the messages, never on disk.
fn selections(folder: &Place) -> Vec<(&'static str, Vec<WireEntry>)> {
    let mut raw = entry(folder, "caf\u{FFFD}.txt");
    raw.name_raw = Some(crate::base64::encode(b"caf\xe9.txt"));
    let many = (1..=5)
        .map(|i| entry(folder, &format!("file{i}.txt")))
        .collect();
    vec![
        ("no selection", Vec::new()),
        ("one file", vec![entry(folder, "a.txt")]),
        ("many files", many),
        ("a name that is not Unicode", vec![raw]),
        (
            "a name with a line break",
            vec![entry(folder, "two\nlines.txt")],
        ),
        (
            "a name starting with a dash",
            vec![entry(folder, "-rf.txt")],
        ),
    ]
}

fn context(manifest: &Manifest, command: &str, folder: &Place, selection: Vec<WireEntry>) -> Value {
    let id = CommandId::new(format!("{}.{command}", manifest.id))
        .unwrap_or_else(|_| CommandId::from_static("plugin.command"));
    let reference = PanelRef::for_place(folder);
    let context =
        WireContext::new(&id, reference.clone(), reference).with_selection(None, selection);
    serde_json::to_value(context).unwrap_or(Value::Null)
}

/// Runs the suite on the plugin in `folder`.
pub fn run(folder: &Path) -> Report {
    let label = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".to_owned());
    let mut report = Report {
        plugin: label,
        groups: Vec::new(),
    };
    let text = match std::fs::read_to_string(folder.join(Manifest::FILE)) {
        Ok(text) => text,
        Err(e) => {
            report.groups.push((
                Group::Manifest,
                Verdict::Fail(vec![format!("plugin.toml could not be read: {e}")]),
            ));
            return report;
        }
    };
    let checked = Manifest::check(&text);
    let Some(manifest) = checked.manifest.clone() else {
        report.groups.push((
            Group::Manifest,
            Verdict::Fail(checked.errors().map(ToString::to_string).collect()),
        ));
        return report;
    };
    report.plugin = manifest.name.clone();
    report
        .groups
        .push((Group::Manifest, Verdict::Pass(Vec::new())));
    let lint: Vec<String> = checked
        .problems
        .iter()
        .filter(|p| p.severity == Severity::Warning)
        .map(ToString::to_string)
        .collect();
    report
        .groups
        .push((Group::Accessibility, Verdict::Pass(lint)));
    if !manifest.is_process() {
        for group in &Group::ALL[2..] {
            report.groups.push((
                *group,
                Verdict::Skipped("a manifest-only plugin runs no process".to_owned()),
            ));
        }
        return report;
    }
    let scratch = match tempfile_folder() {
        Ok(scratch) => scratch,
        Err(e) => {
            report.groups.push((
                Group::Lifecycle,
                Verdict::Fail(vec![format!("no scratch folder: {e}")]),
            ));
            return report;
        }
    };
    let info = PluginInfo {
        manifest: manifest.clone(),
        folder: folder.to_path_buf(),
        enabled: true,
    };
    let groups = drive(&info, scratch.path());
    report.groups.extend(groups);
    report
}

/// A scratch folder for the plugin's state while it is checked.
fn tempfile_folder() -> std::io::Result<ScratchFolder> {
    let base = std::env::temp_dir().join(format!(
        "diroxide-conformance-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&base)?;
    Ok(ScratchFolder(base))
}

/// A scratch folder the suite made; it removes it, and only it, when
/// done.
struct ScratchFolder(std::path::PathBuf);

impl ScratchFolder {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchFolder {
    fn drop(&mut self) {
        let name = self.0.file_name().map(|n| n.to_string_lossy().into_owned());
        if name.is_some_and(|n| n.starts_with("diroxide-conformance-")) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn verdict(problems: Vec<String>) -> Verdict {
    if problems.is_empty() {
        Verdict::Pass(Vec::new())
    } else {
        Verdict::Fail(problems)
    }
}

fn params(value: Value) -> Params {
    match value {
        Value::Object(map) => map,
        _ => Params::new(),
    }
}

/// Plays every group against fresh processes of the plugin.
fn drive(info: &PluginInfo, state: &Path) -> Vec<(Group, Verdict)> {
    let manifest = &info.manifest;
    let startup = manifest.run.clone().unwrap_or_default().startup_timeout;
    let mut out = Vec::new();
    let transport = match start(info, state) {
        Ok(transport) => transport,
        Err(e) => {
            out.push((Group::Lifecycle, Verdict::Fail(vec![e.to_string()])));
            return out;
        }
    };
    let folder_place = Place::parse(&info.folder.display().to_string())
        .or_else(|_| Place::parse(if cfg!(windows) { "C:\\" } else { "/" }));
    let Ok(folder_place) = folder_place else {
        out.push((
            Group::Lifecycle,
            Verdict::Fail(vec!["the plugin's folder is not a place".to_owned()]),
        ));
        return out;
    };
    let mut d = Driver {
        transport,
        manifest: manifest.clone(),
        next: 0,
        stray: 0,
        unreadable: 0,
        called: Vec::new(),
        outside: Vec::new(),
        progress: HashMap::new(),
        folder: folder_place.to_string(),
    };

    // Lifecycle, part one: -32002 before initialize, then the handshake.
    let mut life = Vec::new();
    let early = d.request(methods::COMMAND_RUN, Params::new());
    match d.wait(Some(&early), startup) {
        Waited::Answer(r) if code_of(&r) == Some(ErrorKind::NotInitialized.code()) => {}
        Waited::Answer(_) => {
            life.push("a request before initialize was not answered -32002".to_owned())
        }
        Waited::Nothing => life.push("a request before initialize was not answered".to_owned()),
        Waited::Ended => life.push("it stopped before initialize".to_owned()),
    }
    let mut init = InitializeParams::for_plugin(&manifest.id);
    init.dirs.plugin = info.folder.display().to_string();
    let init_id = Id::Number(0);
    d.send(&Request::new(
        init_id.clone(),
        methods::INITIALIZE,
        params(serde_json::to_value(&init).unwrap_or(Value::Null)),
    ));
    let mut started = false;
    match d.wait(Some(&init_id), startup) {
        Waited::Answer(r) => match (&r.result, &r.error) {
            (Some(result), None)
                if result.get("protocol").and_then(Value::as_u64)
                    == Some(u64::from(crate::protocol::PROTOCOL)) =>
            {
                started = true;
                if !result
                    .get("plugin_capabilities")
                    .is_none_or(Value::is_object)
                {
                    life.push("plugin_capabilities is not an object".to_owned());
                }
            }
            (_, Some(error)) => life.push(format!("initialize failed: {}", error.message)),
            _ => life.push("initialize was not answered with protocol 1".to_owned()),
        },
        Waited::Nothing => life.push(format!(
            "initialize was not answered in {}",
            lines::duration_words(startup)
        )),
        Waited::Ended => life.push("it stopped before answering initialize".to_owned()),
    }
    if !started {
        out.push((Group::Lifecycle, Verdict::Fail(life)));
        let _ = d.transport.kill();
        for group in &Group::ALL[3..] {
            out.push((
                *group,
                Verdict::Skipped("the plugin did not start".to_owned()),
            ));
        }
        return out;
    }
    d.notify(methods::INITIALIZED, Params::new());

    // Framing.
    let mut framing = Vec::new();
    d.send_line("{not json".to_owned());
    match d.wait(None, ANSWER_WAIT) {
        Waited::Answer(r) if code_of(&r) == Some(ErrorKind::ParseError.code()) => {}
        _ => framing.push("a line that is not JSON was not answered -32700".to_owned()),
    }
    d.send_line(r#"[{"jsonrpc":"2.0","id":"batch","method":"x"}]"#.to_owned());
    match d.wait(None, ANSWER_WAIT) {
        Waited::Answer(r) if code_of(&r) == Some(ErrorKind::InvalidRequest.code()) => {}
        _ => framing.push("a batch was not refused with -32600".to_owned()),
    }
    let crlf = Id::Text("crlf".to_owned());
    if let Ok(line) = to_line(&Request::new(
        crlf.clone(),
        "diroxide/conformance_crlf",
        Params::new(),
    )) {
        d.send_line(format!("{line}\r"));
    }
    if !matches!(d.wait(Some(&crlf), ANSWER_WAIT), Waited::Answer(_)) {
        framing.push("a line ending in CR LF was not answered".to_owned());
    }
    out.push((Group::Framing, verdict(framing)));

    // Methods.
    let mut method_problems = Vec::new();
    let unknown = d.request("diroxide/conformance_unknown", Params::new());
    match d.wait(Some(&unknown), ANSWER_WAIT) {
        Waited::Answer(r) if code_of(&r) == Some(ErrorKind::MethodNotFound.code()) => {}
        Waited::Answer(_) => {
            method_problems.push("an unknown request was not answered -32601".to_owned());
        }
        _ => method_problems.push("an unknown request was not answered".to_owned()),
    }
    d.notify("diroxide/conformance_notice", Params::new());
    if !d.quiet(QUIET_WAIT) {
        method_problems.push("an unknown notification was answered".to_owned());
    }
    out.push((Group::Methods, verdict(method_problems)));

    // Commands, with unknown fields on the way.
    let commands: Vec<String> = manifest
        .commands
        .iter()
        .filter(|c| c.argv.is_empty())
        .map(|c| c.id.clone())
        .collect();
    if commands.is_empty() {
        out.push((
            Group::Commands,
            Verdict::Skipped("none declared".to_owned()),
        ));
    } else {
        let mut problems = Vec::new();
        for command in &commands {
            for (case, selection) in selections(&folder_place) {
                let mut p = Params::new();
                p.insert("command".into(), json!(command));
                p.insert(
                    "context".into(),
                    context(manifest, command, &folder_place, selection),
                );
                p.insert("args".into(), json!({}));
                p.insert(
                    "progress_token".into(),
                    json!(format!("p-{command}-{case}")),
                );
                p.insert("diroxide_conformance_unknown_field".into(), json!(true));
                let id = d.request(methods::COMMAND_RUN, p);
                match d.wait(Some(&id), ANSWER_WAIT) {
                    Waited::Answer(r) => {
                        if let Some(result) = &r.result
                            && !(result.is_null() || result.is_object())
                        {
                            problems.push(format!(
                                "{command} with {case}: the answer is not an object"
                            ));
                        }
                    }
                    Waited::Nothing => {
                        problems.push(format!("{command} with {case}: no answer"));
                    }
                    Waited::Ended => {
                        problems.push(format!("{command} with {case}: it stopped"));
                        break;
                    }
                }
            }
        }
        for (token, kinds) in &d.progress {
            let reports: Vec<&String> = kinds.iter().filter(|k| *k != "partial").collect();
            let ordered = reports.first().is_none_or(|k| *k == "begin")
                && reports
                    .iter()
                    .position(|k| *k == "end")
                    .is_none_or(|i| i + 1 == reports.len());
            if !ordered {
                problems.push(format!("progress {token} is not begin, reports, end"));
            }
        }
        out.push((Group::Commands, verdict(problems)));
    }

    // Cancel.
    if let Some(command) = commands.first() {
        let mut p = Params::new();
        p.insert("command".into(), json!(command));
        p.insert(
            "context".into(),
            context(manifest, command, &folder_place, Vec::new()),
        );
        let id = d.request(methods::COMMAND_RUN, p);
        let mut cancel = Params::new();
        cancel.insert("id".into(), json!(id));
        d.notify(methods::CANCEL_REQUEST, cancel);
        let problems = match d.wait(Some(&id), ANSWER_WAIT) {
            Waited::Answer(r)
                if r.error.is_none() || code_of(&r) == Some(ErrorKind::RequestCancelled.code()) =>
            {
                Vec::new()
            }
            Waited::Answer(_) => {
                vec!["a cancelled request was answered with an error other than -32800".to_owned()]
            }
            _ => vec!["a cancelled request was not answered".to_owned()],
        };
        out.push((Group::Cancel, verdict(problems)));
    } else {
        out.push((
            Group::Cancel,
            Verdict::Skipped("no command to cancel".to_owned()),
        ));
    }

    // Settings.
    let settings = crate::services::settings_for(manifest, &serde_json::Map::new());
    let mut p = Params::new();
    p.insert("settings".into(), Value::Object(settings));
    d.notify(methods::SETTINGS_CHANGED, p);
    let after = d.request("diroxide/conformance_after_settings", Params::new());
    let settings_problems = match d.wait(Some(&after), ANSWER_WAIT) {
        Waited::Answer(_) => Vec::new(),
        _ => vec!["it stopped answering after settings/changed".to_owned()],
    };
    out.push((Group::Settings, verdict(settings_problems)));

    // Providers.
    if manifest.providers.is_empty() {
        out.push((
            Group::Providers,
            Verdict::Skipped("none declared".to_owned()),
        ));
    } else {
        let mut problems = Vec::new();
        for provider in &manifest.providers {
            let root = format!("{}://conformance/", provider.scheme);
            let id = d.request(methods::PROVIDER_LIST, params(json!({"place": root})));
            match d.wait(Some(&id), ANSWER_WAIT) {
                Waited::Answer(r) => {
                    let shaped = r.result.as_ref().is_some_and(|v| {
                        v.get("entries").is_some_and(Value::is_array) || v.get("complete").is_some()
                    }) || r.error.is_some();
                    if !shaped {
                        problems.push(format!(
                            "{}: list is not entries or complete",
                            provider.scheme
                        ));
                    }
                }
                _ => problems.push(format!("{}: list was not answered", provider.scheme)),
            }
            let missing = format!("{root}diroxide-conformance-missing");
            let id = d.request(methods::PROVIDER_STAT, params(json!({"place": missing})));
            match d.wait(Some(&id), ANSWER_WAIT) {
                Waited::Answer(r)
                    if [ErrorKind::NotFound.code(), ErrorKind::AuthRequired.code()]
                        .contains(&code_of(&r).unwrap_or(0)) => {}
                _ => problems.push(format!(
                    "{}: a missing name was not answered -32010",
                    provider.scheme
                )),
            }
        }
        out.push((Group::Providers, verdict(problems)));
    }

    // Hooks.
    if manifest.hooks.is_empty() {
        out.push((Group::Hooks, Verdict::Skipped("none declared".to_owned())));
    } else {
        let mut problems = Vec::new();
        for hook in manifest.hooks.iter().filter(|h| h.is_before()) {
            let before = json!({"event": hook.event, "job": 1, "items": [], "count": 0,
                "summary": "Nothing, a conformance check"});
            let id = d.request(methods::HOOK_BEFORE, params(before));
            match d.wait(Some(&id), ANSWER_WAIT) {
                Waited::Answer(r) => {
                    let action = r
                        .result
                        .as_ref()
                        .and_then(|v| v.get("action"))
                        .and_then(Value::as_str);
                    if !matches!(action, Some("continue" | "cancel")) {
                        problems.push(format!(
                            "{}: the answer is not continue or cancel",
                            hook.event
                        ));
                    }
                }
                _ => problems.push(format!("{}: no answer", hook.event)),
            }
        }
        for hook in manifest.hooks.iter().filter(|h| !h.is_before()) {
            d.notify(methods::HOOK_AFTER, params(json!({"event": hook.event})));
        }
        if !d.quiet(QUIET_WAIT) {
            problems.push("an after event was answered".to_owned());
        }
        out.push((Group::Hooks, verdict(problems)));
    }

    // Lifecycle, part two: shutdown, exit, and the exit code.
    let shutdown = d.request(methods::SHUTDOWN, Params::new());
    match d.wait(Some(&shutdown), ANSWER_WAIT) {
        Waited::Answer(r) if r.error.is_none() && r.result.as_ref().is_none_or(Value::is_null) => {}
        _ => life.push("shutdown was not answered with null".to_owned()),
    }
    d.notify(methods::EXIT, Params::new());
    if !d.ends_within(Duration::from_secs(3)) {
        life.push("it did not exit after exit".to_owned());
        let _ = d.transport.kill();
    }
    // A second process: it must end when its input ends.
    if let Ok(second) = start(info, state) {
        let mut p = Driver {
            transport: second,
            manifest: manifest.clone(),
            next: 100,
            stray: 0,
            unreadable: 0,
            called: Vec::new(),
            outside: Vec::new(),
            progress: HashMap::new(),
            folder: d.folder.clone(),
        };
        p.send(&Request::new(
            init_id.clone(),
            methods::INITIALIZE,
            params(serde_json::to_value(&init).unwrap_or(Value::Null)),
        ));
        let _ = p.wait(Some(&init_id), startup);
        p.notify(methods::INITIALIZED, Params::new());
        p.transport.close_input();
        if !p.ends_within(Duration::from_secs(3)) {
            life.push("it did not exit when its input ended".to_owned());
            let _ = p.transport.kill();
        }
        d.stray += p.stray;
    }
    out.insert(0, (Group::Lifecycle, verdict(life)));

    // Services and hygiene.
    let services = if d.outside.is_empty() {
        let notes = if d.called.is_empty() {
            vec!["it called no services".to_owned()]
        } else {
            vec![format!("it called {}", d.called.join(", "))]
        };
        Verdict::Pass(notes)
    } else {
        Verdict::Fail(d.outside.clone())
    };
    out.push((Group::Services, services));
    let mut hygiene_problems = Vec::new();
    if d.unreadable > 0 {
        hygiene_problems.push(format!("{} lines could not be read", d.unreadable));
    }
    out.push((
        Group::Hygiene,
        if hygiene_problems.is_empty() {
            let notes = if d.stray == 0 {
                Vec::new()
            } else {
                vec![format!("{} stray lines on standard output", d.stray)]
            };
            Verdict::Pass(notes)
        } else {
            Verdict::Fail(hygiene_problems)
        },
    ));
    out
}

/// `diro plugin conformance PATH`: runs the suite, writes the report and
/// answers the exit code (0 when every group passed, 1 otherwise).
pub fn run_cli(folder: &Path, out: &mut dyn Write) -> i32 {
    let report = run(folder);
    let _ = out.write_all(report.text().as_bytes());
    i32::from(!report.passed())
}
