//! The host: finds plugins, starts them on demand, routes calls, keeps
//! their timers, and serves their requests through [`HostServices`],
//! which the app implements.
//!
//! - A plugin starts the first time it is used, through a [`Launcher`]
//!   (a process by default), with the handshake of [`crate::session`].
//! - [`Host::call`] runs on the caller's worker thread; the input thread
//!   never waits on a plugin.
//! - [`Host::tick`], every tenth of a second from [`Host::start_supervisor`]
//!   or by hand in tests, applies the timeouts, the idle stop and the crash
//!   policy, and turns what happened into [`HostEvent`]s for the app.
//! - [`Host::shutdown_all`] ends every plugin in parallel within one budget
//!   of two seconds; dropping the host kills what is left without waiting.

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use diroxide_config::PluginSettings;
use diroxide_core::{CommandId, Entry, JobRequest, Message, Place};
use diroxide_ops::JobReport;
use diroxide_tools::process::ProcessSpec;
use diroxide_vfs::{Registry, WatchEvent, WatchSink};
use serde_json::{Map, Value};

use crate::ask::{Answer, PromptHistory, Question, TextDocument};
use crate::calls::Calls;
use crate::error::PluginError;
use crate::files::Files;
use crate::hooks::{HookAfter, HookBefore, HookDecision, decide};
use crate::lines;
use crate::listers::{
    ColumnValue, ListerView, MAX_LISTER_ROWS, column_params, read_column_values, read_rows,
};
use crate::live::{CallOptions, LivePlugin, Notice, RequestKind};
use crate::log::{PluginLog, Source};
use crate::manifest::{HookSpec, Manifest};
use crate::panels::{Denials, PanelChange, PanelChoice, PanelSnapshot};
use crate::protocol::{ErrorKind, Notification, PROTOCOL, Params, Request, RpcError, methods};
use crate::router::{CancelFlag, Connection, Event, Inbound, Outcome};
use crate::services::{
    AnnounceLimiter, MAX_ANNOUNCE_CHARS, ProgressBoard, Waiting, announcement, clean_text,
    settings_for,
};
use crate::session::{InitializeParams, SessionError, handshake, unconfirmed};
use crate::spool::Spool;
use crate::supervisor::{
    Action, CrashPolicy, CrashVerdict, Flood, FloodGuard, SystemTicker, Ticker,
};
use crate::tools::ToolBox;
use crate::transport::{PluginTransport, StdioTransport, resolve_program};

/// How long quitting waits for every plugin in all.
pub const QUIT_BUDGET: Duration = Duration::from_secs(2);

/// A plugin found on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    /// Its manifest.
    pub manifest: Manifest,
    /// Its folder.
    pub folder: PathBuf,
    /// Whether the user enabled it.
    pub enabled: bool,
}

/// What the host offers plugins, implemented by the app over its own state,
/// dialogs and job engine, so a plugin's operations get the same safety,
/// trash, progress and undo as built-in ones. Every method runs on one of
/// the router's worker threads, never the input thread, and may block
/// while the owner answers; the time does not count against the plugin.
pub trait HostServices: Send + Sync {
    /// `ui/announce`: says a message, already cleaned and attributed.
    fn announce(&self, message: Message);

    /// `$/progress`, as the job's sentence: shown in the jobs line, said on
    /// Say Status, or at once when it says the plugin waits for the owner.
    /// By default it is announced like any message.
    fn progress(&self, message: Message) {
        self.announce(message);
    }

    /// `ui/ask`: one accessible dialog, the app's own: modal, titled with
    /// [`Question::title`] (the plugin's name last), closed by Escape
    /// ([`Answer::Cancelled`]), focus returned. A destructive confirm
    /// starts on No. A typed kind offers [`Question::history`] and
    /// [`Question::completion`]; a secret is never echoed.
    fn ask(&self, question: &Question) -> Answer;

    /// `ui/show_text`: the document in the viewer, titled with its
    /// title. When [`TextDocument::wait`] is set, returns when the owner
    /// closes it. The default says this host has no viewer.
    fn show_text(&self, document: &TextDocument) -> Result<(), RpcError> {
        let _ = document;
        Err(RpcError::new(
            ErrorKind::Unsupported,
            "this host has no viewer",
        ))
    }

    /// `panel/get`: a panel's state; `None` where there are no panels
    /// (`diro serve` with no frontend), which the default says.
    fn panel(&self, which: PanelChoice) -> Option<PanelSnapshot> {
        let _ = which;
        None
    }

    /// `panel/set`: the change made through ordinary app commands, so the
    /// app says its usual position message. The default has no panels.
    fn set_panel(&self, change: &PanelChange) -> Result<(), RpcError> {
        let _ = change;
        Err(RpcError::new(
            ErrorKind::Unsupported,
            "this host has no panels",
        ))
    }

    /// `fs/mkdir`, `fs/rename`, `fs/copy`, `fs/move` and `fs/delete`: runs
    /// the job a plugin asked for exactly as a built-in one, with the plan,
    /// the listing, the confirmation that starts on No, the trash by
    /// default and the undo log; the request's origin names the plugin, and
    /// the app gives it a job id. `cancel` rises when the plugin cancels
    /// its request or stops; the job stops with it. The default runs
    /// nothing.
    fn run_job(&self, request: &JobRequest, cancel: &CancelFlag) -> Result<JobReport, RpcError> {
        let _ = (request, cancel);
        Err(RpcError::new(
            ErrorKind::Unsupported,
            "this host runs no file jobs for plugins",
        ))
    }

    /// `lister/open`: shows a plugin's lister in a panel, its rows item
    /// rows that no file command takes. Enter runs
    /// [`ListerView::activate`], keys run [`ListerView::action_for`];
    /// more rows come from [`Host::lister_rows`], and leaving it calls
    /// [`Host::lister_closed`]. The default has no panels.
    fn open_lister(&self, lister: &ListerView) -> Result<(), RpcError> {
        let _ = lister;
        Err(RpcError::new(
            ErrorKind::Unsupported,
            "this host has no panels",
        ))
    }

    /// `command/execute`: runs a diroxide command by id with its
    /// arguments, as a key would; the manifest's patterns were checked
    /// first. The result is the command's, or null. The default runs
    /// nothing.
    fn execute(&self, command: &CommandId, args: &Params) -> Result<Value, RpcError> {
        let _ = args;
        Err(RpcError::new(
            ErrorKind::Unsupported,
            format!("this host cannot run {command}"),
        ))
    }
}

/// Something the app says or shows about a plugin. Each carries its
/// sentence, meaning first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// A plugin could not start.
    StartFailed {
        /// The plugin's id.
        plugin: String,
        /// "Could not start Line counter: CAUSE."
        message: Message,
    },
    /// A plugin's request has waited two seconds.
    Waiting {
        /// The plugin's id.
        plugin: String,
        /// "Count lines: waiting for the plugin"
        message: Message,
    },
    /// Something a plugin said or reported, when the host has no services
    /// to say it through (tests, `diro plugin run`).
    Said {
        /// The plugin's id.
        plugin: String,
        /// The cleaned, attributed message.
        message: Message,
    },
    /// A request timed out.
    TimedOut {
        /// The plugin's id.
        plugin: String,
        /// "Line counter timed out after 30 seconds."
        message: Message,
    },
    /// A plugin stopped without being asked.
    Crashed {
        /// The plugin's id.
        plugin: String,
        /// "Line counter stopped unexpectedly." and the next step.
        message: Message,
    },
    /// A plugin was turned off by a policy (crashes, failed starts,
    /// floods).
    TurnedOff {
        /// The plugin's id.
        plugin: String,
        /// "Line counter is off: it stopped 3 times."
        message: Message,
    },
    /// A plugin was stopped for flooding.
    Flooded {
        /// The plugin's id.
        plugin: String,
        /// "Line counter was stopped for flooding."
        message: Message,
    },
    /// A plugin was stopped after its idle time.
    IdleStopped {
        /// The plugin's id.
        plugin: String,
    },
    /// A plugin's provider lost a device or a session
    /// (`provider/lost`): panels at the authority show it in words.
    ProviderLost {
        /// The plugin's id.
        plugin: String,
        /// The authority lost, such as `mtp://phone`.
        authority: String,
        /// "Phone disconnected (MTP)", the plugin's name last.
        message: Message,
    },
}

/// Starts a plugin's program and returns its transport.
pub trait Launcher: Send + Sync {
    /// Starts the plugin; its standard error goes to `log`.
    fn launch(
        &self,
        info: &PluginInfo,
        log: &PluginLog,
    ) -> Result<Arc<dyn PluginTransport>, PluginError>;
}

/// The default launcher: a process, found by the one PATH rule or inside
/// the plugin's folder, with diroxide's environment and the plugin's own.
#[derive(Clone, Debug, Default)]
pub struct ProcessLauncher {
    /// The plugin state folder, named in `DIRO_STATE_DIR`.
    pub state_dir: Option<PathBuf>,
}

impl Launcher for ProcessLauncher {
    fn launch(
        &self,
        info: &PluginInfo,
        log: &PluginLog,
    ) -> Result<Arc<dyn PluginTransport>, PluginError> {
        let manifest = &info.manifest;
        let start_error = |message: String| PluginError::Start {
            plugin: manifest.name.clone(),
            message,
        };
        let run = manifest
            .run
            .as_ref()
            .ok_or_else(|| start_error("it has no program to run".to_owned()))?;
        let (program, args) = run
            .command
            .split_first()
            .ok_or_else(|| start_error("its program is empty".to_owned()))?;
        let path = resolve_program(program, &info.folder, env::var_os("PATH").as_deref())
            .map_err(start_error)?;
        let mut spec = ProcessSpec::new(path)
            .args(args)
            .cwd(info.folder.join(&run.cwd))
            .diroxide_environment()
            .env("DIRO_PROTOCOL", PROTOCOL.to_string())
            .env("DIRO_PLUGIN_ID", &manifest.id)
            .env("DIRO_PLUGIN_DIR", &info.folder)
            .env("DIRO_PARENT_PID", std::process::id().to_string());
        if let Some(state) = &self.state_dir {
            spec = spec.env("DIRO_STATE_DIR", state.join(&manifest.id));
        }
        for (name, value) in &run.env {
            spec = spec.env(name, value);
        }
        let transport =
            StdioTransport::start(&manifest.name, spec, log.clone(), run.max_message_bytes)?;
        Ok(Arc::new(transport))
    }
}

/// One plugin's place in the host: running or not, and its policies.
struct Slot {
    live: Option<Arc<LivePlugin>>,
    crashes: CrashPolicy,
    starts: CrashPolicy,
    off: Option<String>,
    log: PluginLog,
}

/// Where a plugin's words go: the app's services, or, without them (tests,
/// `diro plugin run`), the host's events.
#[derive(Clone)]
pub(crate) struct Voice {
    services: Option<Arc<dyn HostServices>>,
    events: Arc<Mutex<Vec<HostEvent>>>,
    plugin: String,
}

impl Voice {
    pub(crate) fn say(&self, message: Message) {
        match &self.services {
            Some(services) => services.announce(message),
            None => self.push(message),
        }
    }

    fn progress(&self, message: Message) {
        match &self.services {
            Some(services) => services.progress(message),
            None => self.push(message),
        }
    }

    pub(crate) fn events_push(&self, event: HostEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn push(&self, message: Message) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(HostEvent::Said {
                plugin: self.plugin.clone(),
                message,
            });
    }
}

/// The handler of one connection's inbound traffic: announcements, log
/// lines and progress (host services, part one); progress and log lines
/// also feed the supervisor; floods stop the plugin.
struct PluginInbound {
    name: String,
    live: Arc<OnceLock<Weak<LivePlugin>>>,
    calls: Calls,
    log: PluginLog,
    flood: Mutex<FloodGuard>,
    flooded: Mutex<bool>,
    ticker: Arc<dyn Ticker>,
    voice: Voice,
    limiter: Mutex<AnnounceLimiter>,
    board: Mutex<ProgressBoard>,
    stray_said: AtomicBool,
    watches: Mutex<HashMap<String, (Place, WatchSink)>>,
}

impl PluginInbound {
    /// `provider/changed`: each change to the watch's sink, as the
    /// provider trait's events; an unknown watch or change is dropped.
    fn changed(&self, params: &Params) {
        let Some(id) = params.get("watch_id").and_then(Value::as_str) else {
            return;
        };
        let watches = self.watches.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((folder, sink)) = watches.get(id) else {
            return;
        };
        let changes = params
            .get("changes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for change in changes {
            let place = change
                .get("place")
                .and_then(Value::as_str)
                .and_then(|p| Place::parse(p).ok())
                .filter(|p| p.parent().as_ref() == Some(folder));
            let event = match (change.get("kind").and_then(Value::as_str), place) {
                (Some("created"), Some(place)) => WatchEvent::Created(place),
                (Some("modified"), Some(place)) => WatchEvent::Changed(place),
                (Some("removed"), Some(place)) => WatchEvent::Removed(place),
                (Some("renamed"), Some(place)) => WatchEvent::RenamedIn(place),
                _ => WatchEvent::Rescan,
            };
            sink(event);
        }
    }

    /// `provider/lost`: said once, with the plugin's name last.
    fn lost(&self, params: &Params) {
        let authority = params
            .get("authority")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(|r| clean_text(r, MAX_ANNOUNCE_CHARS))
            .filter(|r| !r.is_empty())
            .unwrap_or_else(lines::connection_lost);
        self.voice.events_push(HostEvent::ProviderLost {
            plugin: self.voice.plugin.clone(),
            authority,
            message: Message::warning(lines::with_source(&reason, &self.name)),
        });
    }

    fn live(&self) -> Option<Arc<LivePlugin>> {
        self.live.get().and_then(Weak::upgrade)
    }

    fn count(&self, kind: Flood) {
        let stop = self
            .flood
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(kind, self.ticker.now());
        if stop {
            *self.flooded.lock().unwrap_or_else(PoisonError::into_inner) = true;
            self.log
                .write(Source::Host, &format!("{} flooded the host", self.name));
            if let Some(live) = self.live() {
                live.kill();
            }
        }
    }

    fn announce(&self, params: &Params) {
        let Some(message) = announcement(params, &self.name) else {
            return;
        };
        let admitted = self
            .limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(self.ticker.now());
        if admitted {
            self.voice.say(message);
        } else {
            self.count(Flood::Folded);
        }
    }

    fn progress(&self, params: &Params) {
        let Some(token) = params.get("token").and_then(Value::as_str) else {
            return;
        };
        let value = params.get("value").cloned().unwrap_or(Value::Null);
        if value.get("kind").and_then(Value::as_str) == Some("partial") {
            if let Some(live) = self.live() {
                let entries = value
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                live.partial(token, entries);
                live.progress(token, None);
            }
            return;
        }
        let (said, waiting) = self
            .board
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .update(token, &value, &self.name, self.ticker.now());
        if let Some(live) = self.live() {
            let state = match waiting {
                Waiting::Began => Some(true),
                Waiting::Ended => Some(false),
                Waiting::Same => None,
            };
            live.progress(token, state);
        }
        if let Some(message) = said {
            self.voice.progress(message);
        }
    }

    /// The fold line and the progress held back, once their time comes;
    /// tokens of answered requests are forgotten.
    fn tick(&self) {
        let now = self.ticker.now();
        let fold = self
            .limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fold(now, &self.name);
        if let Some(message) = fold {
            self.voice.say(message);
        }
        let held = self
            .board
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush(&self.name, now);
        for message in held {
            self.voice.progress(message);
        }
        if let Some(live) = self.live() {
            let mut board = self.board.lock().unwrap_or_else(PoisonError::into_inner);
            for token in board.tokens() {
                if live.request_of(&token).is_none() {
                    board.forget(&token);
                }
            }
        }
    }
}

impl Inbound for PluginInbound {
    fn request(&self, request: &Request, cancel: &CancelFlag) -> Outcome {
        self.calls.serve(request, cancel)
    }

    fn notification(&self, notification: &Notification) {
        match notification.method.as_str() {
            methods::UI_ANNOUNCE => self.announce(&notification.params),
            methods::PROVIDER_CHANGED => self.changed(&notification.params),
            methods::PROVIDER_LOST => self.lost(&notification.params),
            methods::PROGRESS => self.progress(&notification.params),
            methods::LOG => {
                let level = match notification.params.get("level").and_then(Value::as_str) {
                    Some("error") => "error",
                    Some("warn") => "warn",
                    Some("debug") => "debug",
                    _ => "info",
                };
                let text = notification
                    .params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.log.write(Source::Plugin(level), text);
                if let Some(live) = self.live() {
                    live.logged();
                }
            }
            _ => {}
        }
    }

    fn event(&self, event: &Event) {
        match event {
            Event::ParseFailure(_) | Event::TooLong(_) => self.count(Flood::ParseFailures),
            Event::Stray(_) => {
                self.count(Flood::Stray);
                if !self.stray_said.swap(true, Ordering::SeqCst) {
                    self.voice
                        .say(Message::info(lines::wrote_stray_text(&self.name)));
                }
            }
            Event::Ended => {
                // The host removes the spool files it made, and only
                // those, when the session ends.
                if let Some(spool) = &self.calls.files.spool {
                    spool.remove_made();
                }
            }
            Event::UnknownAnswer(_) => {}
        }
    }
}

/// The plugin host.
pub struct Host {
    plugins: Vec<PluginInfo>,
    enabled: Vec<String>,
    timeout: Duration,
    state_dir: Option<PathBuf>,
    launcher: Arc<dyn Launcher>,
    ticker: Arc<dyn Ticker>,
    slots: Mutex<HashMap<String, Arc<Mutex<Slot>>>>,
    inbounds: Mutex<HashMap<String, Arc<PluginInbound>>>,
    events: Arc<Mutex<Vec<HostEvent>>>,
    services: Option<Arc<dyn HostServices>>,
    settings: Mutex<HashMap<String, Map<String, Value>>>,
    history: Arc<Mutex<PromptHistory>>,
    registry: Option<Arc<Registry>>,
    tools: ToolBox,
    off: bool,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("plugins", &self.plugins.len())
            .field("off", &self.off)
            .finish()
    }
}

impl Host {
    /// A host with the user's plugin settings and no plugins found yet.
    pub fn new(settings: &PluginSettings) -> Self {
        Self {
            plugins: Vec::new(),
            enabled: settings.enabled.clone(),
            timeout: Duration::from_millis(settings.timeout_ms),
            state_dir: None,
            launcher: Arc::new(ProcessLauncher::default()),
            ticker: Arc::new(SystemTicker::new()),
            slots: Mutex::new(HashMap::new()),
            inbounds: Mutex::new(HashMap::new()),
            events: Arc::new(Mutex::new(Vec::new())),
            services: None,
            settings: Mutex::new(HashMap::new()),
            history: Arc::new(Mutex::new(PromptHistory::default())),
            registry: None,
            tools: ToolBox::default(),
            off: false,
        }
    }

    /// The same host with the plugin state folder, where each plugin's log
    /// and private folder live.
    pub fn with_state_dir(mut self, state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        self.launcher = Arc::new(ProcessLauncher {
            state_dir: Some(state_dir.clone()),
        });
        self.state_dir = Some(state_dir);
        self
    }

    /// The same host with another launcher (tests start fakes).
    pub fn with_launcher(mut self, launcher: Arc<dyn Launcher>) -> Self {
        self.launcher = launcher;
        self
    }

    /// The same host with another clock (tests move it by hand).
    pub fn with_ticker(mut self, ticker: Arc<dyn Ticker>) -> Self {
        self.ticker = ticker;
        self
    }

    /// The same host reaching files through these providers, for the
    /// plugins' `fs/*` calls.
    pub fn with_registry(mut self, registry: Arc<Registry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// The same host finding the plugins' tools here (tests put fakes
    /// on a test PATH).
    pub fn with_tools(mut self, tools: ToolBox) -> Self {
        self.tools = tools;
        self
    }

    /// The same host speaking through the app's services.
    pub fn with_services(mut self, services: Arc<dyn HostServices>) -> Self {
        self.services = Some(services);
        self
    }

    /// The owner's values for a plugin's settings. They go to the plugin
    /// in `initialize`, and at once in `settings/changed` (the whole
    /// object, defaults included) when it is running.
    pub fn set_settings(&self, plugin: &str, values: Map<String, Value>) {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(plugin.to_owned(), values);
        let Some(info) = self.plugin(plugin) else {
            return;
        };
        let live = self
            .slot(info)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .live
            .clone();
        if let Some(live) = live
            && !live.is_ended()
        {
            let mut params = Params::new();
            params.insert(
                "settings".to_owned(),
                Value::Object(self.settings_of(&info.manifest)),
            );
            let _ = live.connection().notify(methods::SETTINGS_CHANGED, params);
        }
    }

    fn settings_of(&self, manifest: &Manifest) -> Map<String, Value> {
        let settings = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let empty = Map::new();
        settings_for(manifest, settings.get(&manifest.id).unwrap_or(&empty))
    }

    /// Turns every plugin off for this session (`--no-plugins`).
    pub fn plugins_off(mut self) -> Self {
        self.off = true;
        self
    }

    /// How long a plugin may take to answer.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Finds plugins: every folder under `plugins_dir` with a readable
    /// `plugin.toml`. Folders whose manifest cannot be read are returned as
    /// errors beside the plugins found; a missing folder finds nothing.
    pub fn discover(&mut self, plugins_dir: &Path) -> Vec<PluginError> {
        self.plugins.clear();
        let mut problems = Vec::new();
        let Ok(folders) = fs::read_dir(plugins_dir) else {
            return problems;
        };
        for folder in folders
            .flatten()
            .map(|item| item.path())
            .filter(|p| p.is_dir())
        {
            let label = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(text) = fs::read_to_string(folder.join(Manifest::FILE)) else {
                continue;
            };
            match Manifest::from_toml(&text) {
                Ok(manifest) => {
                    let enabled = self.enabled.contains(&manifest.id);
                    self.plugins.push(PluginInfo {
                        manifest,
                        folder,
                        enabled,
                    });
                }
                Err(PluginError::Manifest { message, .. }) => {
                    problems.push(PluginError::Manifest {
                        plugin: label,
                        message,
                    })
                }
                Err(other) => problems.push(other),
            }
        }
        self.plugins
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        problems
    }

    /// Adds a plugin found elsewhere (tests, `diro plugin run PATH`).
    pub fn add(&mut self, info: PluginInfo) {
        self.plugins.retain(|p| p.manifest.id != info.manifest.id);
        self.plugins.push(info);
        self.plugins
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    }

    /// The plugins found, by id.
    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugins
    }

    /// A plugin by id.
    pub fn plugin(&self, id: &str) -> Option<&PluginInfo> {
        self.plugins.iter().find(|p| p.manifest.id == id)
    }

    fn slot(&self, info: &PluginInfo) -> Arc<Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slots.entry(info.manifest.id.clone()).or_insert_with(|| {
            let log = match &self.state_dir {
                Some(state) => PluginLog::at(
                    state
                        .join(&info.manifest.id)
                        .join(format!("{}.log", info.manifest.id)),
                ),
                None => PluginLog::in_memory(),
            };
            Arc::new(Mutex::new(Slot {
                live: None,
                crashes: CrashPolicy::default(),
                starts: CrashPolicy::for_starts(),
                off: None,
                log,
            }))
        }))
    }

    fn event(&self, event: HostEvent) {
        self.events
            .as_ref()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    /// The events since the last look, for the app to say and show.
    pub fn take_events(&self) -> Vec<HostEvent> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The running plugin, started if needed.
    fn live(&self, info: &PluginInfo) -> Result<Arc<LivePlugin>, PluginError> {
        let name = &info.manifest.name;
        if self.off {
            return Err(PluginError::Off(lines::plugins_off()));
        }
        if !info.enabled {
            return Err(PluginError::NotEnabled(name.clone()));
        }
        let slot = self.slot(info);
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(reason) = &slot.off {
            return Err(PluginError::Off(reason.clone()));
        }
        if let Some(live) = &slot.live
            && !live.is_ended()
        {
            return Ok(Arc::clone(live));
        }
        slot.live = None;
        match self.start(info, &slot.log) {
            Ok(live) => {
                slot.live = Some(Arc::clone(&live));
                Ok(live)
            }
            Err(error) => {
                let sentence = error.to_string();
                slot.log.write(Source::Host, &sentence);
                if slot.starts.record(self.ticker.now()) == CrashVerdict::Disabled {
                    let off = lines::off_after_failed_starts(name);
                    slot.off = Some(off.clone());
                    self.event(HostEvent::TurnedOff {
                        plugin: info.manifest.id.clone(),
                        message: Message::warning(off),
                    });
                } else {
                    self.event(HostEvent::StartFailed {
                        plugin: info.manifest.id.clone(),
                        message: Message::error(sentence.clone()),
                    });
                }
                Err(PluginError::Start {
                    plugin: name.clone(),
                    message: sentence,
                })
            }
        }
    }

    fn start(&self, info: &PluginInfo, log: &PluginLog) -> Result<Arc<LivePlugin>, SessionError> {
        let manifest = &info.manifest;
        let name = &manifest.name;
        if let Some(sentence) = manifest.needs_newer_host(env!("CARGO_PKG_VERSION")) {
            return Err(SessionError::Protocol(sentence));
        }
        let run = manifest.run.clone().unwrap_or_default();
        let transport = self
            .launcher
            .launch(info, log)
            .map_err(|e| SessionError::Broken(lines::could_not_start(name, &e.to_string())))?;
        let mut params = InitializeParams::for_plugin(&manifest.id);
        params.settings = self.settings_of(manifest);
        params.dirs.plugin = info.folder.display().to_string();
        if let Some(state) = &self.state_dir {
            params.dirs.state = state.join(&manifest.id).display().to_string();
            params.dirs.spool = state.join(&manifest.id).join("spool").display().to_string();
        }
        let started = match handshake(transport.as_ref(), name, &params, run.startup_timeout) {
            Ok(started) => started,
            Err(e) => {
                let _ = transport.kill();
                return Err(e);
            }
        };
        for text in &started.stray {
            log.write(Source::Stray, text);
        }
        for missing in unconfirmed(manifest, &started.result.plugin_capabilities) {
            log.write(
                Source::Host,
                &format!("declared in the manifest but not confirmed, left out: {missing}"),
            );
        }
        for warning in &started.result.warnings {
            log.write(Source::Plugin("warn"), warning);
        }
        let spool = self.state_dir.as_ref().and_then(|state| {
            match Spool::create(state.join(&manifest.id).join("spool")) {
                Ok(spool) => Some(Arc::new(spool)),
                Err(e) => {
                    log.write(Source::Host, &format!("the spool could not be made: {e}"));
                    None
                }
            }
        });
        let live_cell = Arc::new(OnceLock::new());
        let inbound = Arc::new(PluginInbound {
            name: name.clone(),
            live: Arc::clone(&live_cell),
            calls: Calls {
                plugin_id: manifest.id.clone(),
                name: name.clone(),
                manifest: manifest.clone(),
                confirmed: started.result.plugin_capabilities.clone(),
                services: self.services.clone(),
                history: Arc::clone(&self.history),
                live: live_cell,
                log: log.clone(),
                voice: Voice {
                    services: self.services.clone(),
                    events: Arc::clone(&self.events),
                    plugin: manifest.id.clone(),
                },
                denials: Denials::default(),
                files: Files {
                    registry: self.registry.clone(),
                    spool,
                    own_schemes: manifest
                        .providers
                        .iter()
                        .map(|p| p.scheme.clone())
                        .collect(),
                    plugin: name.clone(),
                },
                tools: self.tools.clone(),
            },
            log: log.clone(),
            flood: Mutex::new(FloodGuard::default()),
            flooded: Mutex::new(false),
            ticker: Arc::clone(&self.ticker),
            voice: Voice {
                services: self.services.clone(),
                events: Arc::clone(&self.events),
                plugin: manifest.id.clone(),
            },
            limiter: Mutex::new(AnnounceLimiter::new(self.ticker.now())),
            board: Mutex::new(ProgressBoard::default()),
            stray_said: AtomicBool::new(false),
            watches: Mutex::new(HashMap::new()),
        });
        let connection = Connection::open(
            name,
            Arc::clone(&transport),
            Arc::clone(&inbound) as Arc<dyn Inbound>,
            log.clone(),
            started.early,
        )
        .map_err(|e| SessionError::Broken(lines::could_not_start(name, &e.to_string())))?;
        let live = Arc::new(LivePlugin::new(
            manifest,
            connection,
            Arc::clone(&self.ticker),
        ));
        let _ = inbound.live.set(Arc::downgrade(&live));
        self.inbounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(manifest.id.clone(), inbound);
        Ok(live)
    }

    /// Calls a method on a plugin, starting it if needed, on the caller's
    /// thread. The method's kind decides its inactivity limit.
    pub fn call(&self, plugin: &str, method: &str, params: Value) -> Result<Value, PluginError> {
        self.call_with(plugin, method, params, CallOptions::default())
    }

    /// The same, with a cancel token and a taker of partial batches (a
    /// provider's streamed listing).
    pub fn call_with(
        &self,
        plugin: &str,
        method: &str,
        params: Value,
        options: CallOptions<'_>,
    ) -> Result<Value, PluginError> {
        let info = self
            .plugin(plugin)
            .ok_or_else(|| PluginError::Unknown(plugin.to_owned()))?;
        let params = match params {
            Value::Object(map) => map,
            Value::Null => Params::new(),
            _ => {
                return Err(PluginError::Protocol(
                    "the parameters are not an object".to_owned(),
                ));
            }
        };
        let kind = match method {
            methods::COMMAND_RUN => RequestKind::Command,
            methods::PREVIEW_RENDER => RequestKind::Preview,
            methods::HOOK_BEFORE => RequestKind::Hook,
            methods::CONVERT_RUN => RequestKind::Convert,
            _ => RequestKind::Request,
        };
        // The sentences name the command when there is one: "Count lines:
        // waiting for the plugin".
        let label = params
            .get("command")
            .and_then(Value::as_str)
            .filter(|_| method == methods::COMMAND_RUN)
            .and_then(|id| info.manifest.command(id))
            .map_or_else(|| info.manifest.name.clone(), |c| c.name.clone());
        let live = self.live(info)?;
        let name = info.manifest.name.clone();
        live.call_with(method, params, kind, &label, options)
            .map_err(|error| match ErrorKind::from_code(error.code) {
                Some(ErrorKind::TimedOut) => PluginError::Timeout(name),
                Some(ErrorKind::PluginUnavailable) => PluginError::Stopped(name),
                _ => PluginError::Remote {
                    plugin: name,
                    code: error.code,
                    hint: error.hint().map(str::to_owned),
                    message: error.message,
                },
            })
    }

    /// Whether a plugin is running now.
    pub fn is_running(&self, plugin: &str) -> bool {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.get(plugin).is_some_and(|slot| {
            slot.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .live
                .as_ref()
                .is_some_and(|live| !live.is_ended())
        })
    }

    /// Why a plugin is off for this session, if it is.
    pub fn off_reason(&self, plugin: &str) -> Option<String> {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.get(plugin).and_then(|slot| {
            slot.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .off
                .clone()
        })
    }

    /// Turns a plugin on again after a policy turned it off, forgetting
    /// its crashes.
    pub fn turn_on(&self, plugin: &str) {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = slots.get(plugin) {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            slot.off = None;
            slot.crashes.reset();
            slot.starts.reset();
        }
    }

    /// `hook/before`: asks a hooked plugin whether a job may go on, with
    /// the hook's own time limit, and decides by its answer or, when it
    /// is silent, by its `on_timeout`. Not wired to the job engine in this
    /// alpha.
    pub fn hook_before(
        &self,
        plugin: &str,
        hook: &HookSpec,
        before: &HookBefore,
        verb: &str,
    ) -> HookDecision {
        let name = self
            .plugin(plugin)
            .map_or_else(|| plugin.to_owned(), |p| p.manifest.name.clone());
        let outcome = serde_json::to_value(before)
            .map_err(|e| PluginError::Protocol(e.to_string()))
            .and_then(|params| self.call(plugin, methods::HOOK_BEFORE, params));
        decide(hook, &outcome, verb, &name)
    }

    /// `hook/after`: tells a hooked plugin what happened, starting it if
    /// needed; never waited for.
    pub fn hook_after(&self, plugin: &str, after: &HookAfter) -> Result<(), PluginError> {
        let info = self
            .plugin(plugin)
            .ok_or_else(|| PluginError::Unknown(plugin.to_owned()))?;
        let live = self.live(info)?;
        match serde_json::to_value(after) {
            Ok(Value::Object(params)) => live.connection().notify(methods::HOOK_AFTER, params),
            _ => Err(PluginError::Protocol(
                "the event could not be written".to_owned(),
            )),
        }
    }

    /// `lister/rows`: more rows of a lister from `offset`, or all of them
    /// again when `refresh`; at most [`MAX_LISTER_ROWS`].
    pub fn lister_rows(
        &self,
        plugin: &str,
        lister: &str,
        offset: u64,
        refresh: bool,
    ) -> Result<Vec<Entry>, PluginError> {
        let answer = self.call(
            plugin,
            methods::LISTER_ROWS,
            serde_json::json!({"lister": lister, "offset": offset, "limit": MAX_LISTER_ROWS, "refresh": refresh}),
        )?;
        read_rows(answer.get("rows"), MAX_LISTER_ROWS)
            .map(|(rows, _)| rows)
            .map_err(|e| PluginError::Protocol(e.message))
    }

    /// `lister/closed`: the owner left a lister, so the plugin can free
    /// it. Nothing is sent to a plugin that is not running.
    pub fn lister_closed(&self, plugin: &str, lister: &str) {
        self.notify_running(
            plugin,
            methods::LISTER_CLOSED,
            serde_json::json!({"lister": lister}),
        );
    }

    /// `column/format`: a batch of a declared column's values for places
    /// of one folder, each cleaned to 40 cells, "pending" where the plugin
    /// answered none.
    pub fn column_values(
        &self,
        plugin: &str,
        column: &str,
        folder: &Place,
        places: &[Place],
    ) -> Result<BTreeMap<String, ColumnValue>, PluginError> {
        let declared = self
            .plugin(plugin)
            .is_some_and(|p| p.manifest.columns.iter().any(|c| c.id == column));
        if !declared {
            return Err(PluginError::Protocol(format!(
                "the manifest declares no column {column}"
            )));
        }
        let texts: Vec<String> = places.iter().map(ToString::to_string).collect();
        let params = column_params(column, &folder.to_string(), &texts)
            .map_err(|e| PluginError::Limit(e.message))?;
        let answer = self.call(plugin, methods::COLUMN_FORMAT, params)?;
        Ok(read_column_values(&answer, &texts))
    }

    fn notify_running(&self, plugin: &str, method: &str, params: Value) {
        let live = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
            .and_then(|slot| {
                slot.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .live
                    .clone()
            });
        if let (Some(live), Value::Object(params)) = (live, params)
            && !live.is_ended()
        {
            let _ = live.connection().notify(method, params);
        }
    }

    /// Kills a running plugin at once, its whole tree, without the polite
    /// ending: what it was serving fails, and nothing it had not committed
    /// is committed. It starts again at its next use.
    pub fn kill(&self, plugin: &str) {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
            .cloned();
        if let Some(slot) = slot
            && let Some(live) = slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .live
                .take()
        {
            live.kill();
        }
    }

    /// Sends a running plugin's `provider/changed` for a watch to `sink`,
    /// until [`Host::remove_watch`].
    pub fn add_watch(&self, plugin: &str, watch_id: &str, folder: Place, sink: WatchSink) {
        if let Some(inbound) = self
            .inbounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
        {
            inbound
                .watches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(watch_id.to_owned(), (folder, sink));
        }
    }

    /// Forgets a watch.
    pub fn remove_watch(&self, plugin: &str, watch_id: &str) {
        if let Some(inbound) = self
            .inbounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
        {
            inbound
                .watches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(watch_id);
        }
    }

    /// How many of a plugin's calls its manifest refused while it ran
    /// last.
    pub fn refusals(&self, plugin: &str) -> usize {
        self.inbounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(plugin)
            .map_or(0, |inbound| inbound.calls.denials.count())
    }

    /// The plugin's log.
    pub fn log(&self, plugin: &str) -> Option<PluginLog> {
        let info = self.plugin(plugin)?;
        let slot = self.slot(info);
        let log = slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .log
            .clone();
        Some(log)
    }

    /// Looks at every running plugin: timeouts, the idle stop, crashes and
    /// floods. Called every tenth of a second by the supervisor thread.
    pub fn tick(&self) {
        let slots: Vec<(String, Arc<Mutex<Slot>>)> = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(id, slot)| (id.clone(), Arc::clone(slot)))
            .collect();
        for (id, slot) in slots {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(live) = slot.live.clone() else {
                continue;
            };
            let name = live.name().to_owned();
            if live.is_ended() {
                slot.live = None;
                let flooded = self
                    .inbounds
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&id)
                    .is_some_and(|inbound| {
                        *inbound
                            .flooded
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                    });
                if flooded {
                    self.event(HostEvent::Flooded {
                        plugin: id.clone(),
                        message: Message::warning(lines::flooded(&name)),
                    });
                    continue;
                }
                if live.connection().stopped_by_host() {
                    continue;
                }
                let tail = slot.log.tail();
                slot.log
                    .write(Source::Host, &lines::stopped_unexpectedly(&name));
                for line in &tail {
                    slot.log
                        .write(Source::Host, &format!("last error output: {line}"));
                }
                match slot.crashes.record(self.ticker.now()) {
                    CrashVerdict::Disabled => {
                        let off = lines::off_after_crashes(&name, slot.crashes.count());
                        slot.off = Some(off.clone());
                        self.event(HostEvent::TurnedOff {
                            plugin: id.clone(),
                            message: Message::warning(off),
                        });
                    }
                    CrashVerdict::Restartable => self.event(HostEvent::Crashed {
                        plugin: id.clone(),
                        message: Message::warning(format!(
                            "{} {}",
                            lines::stopped_unexpectedly(&name),
                            lines::starts_again()
                        )),
                    }),
                }
                continue;
            }
            for action in live.tick() {
                match action {
                    Action::Restart => {
                        slot.live = None;
                        slot.log
                            .write(Source::Host, "a second time-out in five minutes: restarted");
                    }
                    Action::IdleStop => {
                        slot.live = None;
                        let stopping = Arc::clone(&live);
                        let _ = thread::Builder::new()
                            .name("plugin-idle-stop".to_owned())
                            .spawn(move || stopping.stop());
                        self.event(HostEvent::IdleStopped { plugin: id.clone() });
                    }
                    Action::Slow(_) | Action::Cancel(_) | Action::TimedOut(_) => {}
                }
            }
            for notice in live.take_notices() {
                match notice {
                    Notice::Slow { label, .. } => self.event(HostEvent::Waiting {
                        plugin: id.clone(),
                        message: Message::progress(lines::waiting_for_plugin(&label)),
                    }),
                    Notice::TimedOut { label, after, .. } => self.event(HostEvent::TimedOut {
                        plugin: id.clone(),
                        message: Message::error(lines::timed_out(&label, after)),
                    }),
                }
            }
            let inbound = self
                .inbounds
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id)
                .cloned();
            if let Some(inbound) = inbound {
                inbound.tick();
            }
        }
    }

    /// Starts the thread that ticks every tenth of a second while the host
    /// lives.
    pub fn start_supervisor(host: &Arc<Host>) -> std::io::Result<JoinHandle<()>> {
        let weak = Arc::downgrade(host);
        thread::Builder::new()
            .name("plugin-supervisor".to_owned())
            .spawn(move || {
                while let Some(host) = weak.upgrade() {
                    host.tick();
                    drop(host);
                    thread::sleep(Duration::from_millis(100));
                }
            })
    }

    /// Ends every running plugin in parallel, politely, within one budget;
    /// whatever is left after it is killed. Never waits longer than the
    /// budget and a moment.
    pub fn shutdown_all(&self, budget: Duration) {
        let live: Vec<Arc<LivePlugin>> = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter_map(|slot| {
                slot.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .live
                    .take()
            })
            .collect();
        let deadline = Instant::now() + budget;
        let handles: Vec<JoinHandle<()>> = live
            .iter()
            .filter_map(|plugin| {
                let plugin = Arc::clone(plugin);
                thread::Builder::new()
                    .name("plugin-stop".to_owned())
                    .spawn(move || {
                        plugin.stop_within(budget);
                    })
                    .ok()
            })
            .collect();
        while Instant::now() < deadline && handles.iter().any(|h| !h.is_finished()) {
            thread::sleep(Duration::from_millis(10));
        }
        for plugin in &live {
            if !plugin.is_ended() {
                plugin.kill();
            }
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // A panic or a quit that skipped the polite ending: kill what is
        // left, without waiting.
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        for slot in slots.values() {
            if let Some(live) = &slot.lock().unwrap_or_else(PoisonError::into_inner).live {
                live.kill();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::ManualTicker;
    use crate::testing::{FakeLauncher, Mode};

    fn info(extra: &str) -> PluginInfo {
        let text = format!(
            "[plugin]\nid = \"line-counter\"\nname = \"Line counter\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[run]\ncommand = [\"x\"]\nstartup_timeout = \"1s\"\n{extra}\n[[command]]\nid = \"echo\"\nname = \"Echo\"\n"
        );
        PluginInfo {
            manifest: Manifest::check(&text).manifest.unwrap(),
            folder: PathBuf::from("."),
            enabled: true,
        }
    }

    fn host(launcher: Arc<FakeLauncher>, ticker: &Arc<ManualTicker>, extra: &str) -> Host {
        let mut host = Host::new(&PluginSettings::default())
            .with_launcher(launcher)
            .with_ticker(Arc::clone(ticker) as Arc<dyn Ticker>);
        host.add(info(extra));
        host
    }

    fn run_echo(host: &Host) {
        let mut params = Map::new();
        params.insert("command".into(), serde_json::json!("echo"));
        host.call("line-counter", methods::COMMAND_RUN, Value::Object(params))
            .unwrap();
    }

    fn said(host: &Host) -> Vec<String> {
        host.take_events()
            .into_iter()
            .filter_map(|e| match e {
                HostEvent::Said { message, .. } => Some(message.text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_plugins_words_and_progress_are_said_with_its_name_last() {
        let ticker = Arc::new(ManualTicker::new());
        let host = host(FakeLauncher::new(Mode::Talk), &ticker, "");
        run_echo(&host);
        // The report came within half a second of the start, so only the
        // start, the announcement and the end are said.
        assert_eq!(
            said(&host),
            [
                "Count lines: started (Line counter)",
                "Lines: 42, files: 1. By Line counter.",
                "Count lines done: 3 files (Line counter)"
            ]
        );
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn a_spoof_is_cleaned_and_attributed() {
        let ticker = Arc::new(ManualTicker::new());
        let host = host(FakeLauncher::new(Mode::Spoof), &ticker, "");
        run_echo(&host);
        assert_eq!(said(&host), ["Moved to the trash. By Line counter."]);
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn a_flood_of_announcements_is_folded_into_one_line() {
        let ticker = Arc::new(ManualTicker::new());
        let host = host(FakeLauncher::new(Mode::Flood), &ticker, "");
        run_echo(&host);
        let first = said(&host);
        assert_eq!(first.len(), 20);
        assert_eq!(first[0], "Message 0. By Line counter.");
        ticker.advance(Duration::from_secs(1));
        host.tick();
        assert_eq!(said(&host), ["80 more messages from Line counter"]);
        assert!(host.is_running("line-counter"));
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn a_lost_connection_is_an_event_with_the_plugin_last() {
        let ticker = Arc::new(ManualTicker::new());
        let host = host(FakeLauncher::new(Mode::Lost), &ticker, "");
        run_echo(&host);
        let lost: Vec<HostEvent> = host
            .take_events()
            .into_iter()
            .filter(|e| matches!(e, HostEvent::ProviderLost { .. }))
            .collect();
        assert_eq!(
            lost,
            [HostEvent::ProviderLost {
                plugin: "line-counter".into(),
                authority: "memfs://box".into(),
                message: Message::warning("Box unplugged (Line counter)"),
            }]
        );
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn stray_text_is_said_once_a_session() {
        let ticker = Arc::new(ManualTicker::new());
        let host = host(FakeLauncher::new(Mode::Stray), &ticker, "");
        run_echo(&host);
        run_echo(&host);
        assert_eq!(said(&host), ["Line counter wrote stray text; see its log."]);
        host.shutdown_all(QUIT_BUDGET);
    }

    /// Records what the host says through the app's services.
    #[derive(Default)]
    struct Services {
        said: Mutex<Vec<Message>>,
    }

    impl HostServices for Services {
        fn announce(&self, message: Message) {
            self.said.lock().unwrap().push(message);
        }
        fn ask(&self, _: &Question) -> Answer {
            Answer::Cancelled
        }
    }

    #[test]
    fn with_services_the_app_hears_the_plugin() {
        let ticker = Arc::new(ManualTicker::new());
        let services = Arc::new(Services::default());
        let host = host(FakeLauncher::new(Mode::Talk), &ticker, "")
            .with_services(Arc::clone(&services) as Arc<dyn HostServices>);
        run_echo(&host);
        let said: Vec<String> = services
            .said
            .lock()
            .unwrap()
            .iter()
            .map(|m| m.text.clone())
            .collect();
        assert_eq!(said.len(), 3);
        assert_eq!(said[1], "Lines: 42, files: 1. By Line counter.");
        assert!(host.take_events().is_empty());
        host.shutdown_all(QUIT_BUDGET);
    }

    #[test]
    fn settings_go_in_initialize_and_settings_changed() {
        let launcher = FakeLauncher::new(Mode::Echo);
        let ticker = Arc::new(ManualTicker::new());
        let host = host(
            Arc::clone(&launcher),
            &ticker,
            "\n[[setting]]\nkey = \"depth\"\ntype = \"int\"\ndefault = 3\n",
        );
        let mut values = Map::new();
        values.insert("depth".into(), serde_json::json!(5));
        values.insert("not_declared".into(), serde_json::json!(true));
        host.set_settings("line-counter", values);
        run_echo(&host);
        let mut values = Map::new();
        values.insert("depth".into(), serde_json::json!(7));
        host.set_settings("line-counter", values);
        host.shutdown_all(QUIT_BUDGET);
        let seen = launcher.transcripts_after(1);
        let received = &seen[0].received;
        let initialize = received
            .iter()
            .find_map(|m| match m {
                crate::protocol::Incoming::Request(r) if r.method == methods::INITIALIZE => {
                    Some(r.params.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(initialize["settings"], serde_json::json!({"depth": 5}));
        let changed = received
            .iter()
            .find_map(|m| match m {
                crate::protocol::Incoming::Notification(n)
                    if n.method == methods::SETTINGS_CHANGED =>
                {
                    Some(n.params.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(changed["settings"], serde_json::json!({"depth": 7}));
    }

    #[test]
    fn a_missing_plugins_folder_finds_nothing() {
        let mut host = Host::new(&PluginSettings::default());
        let nowhere = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-plugins-folder");
        assert!(host.discover(&nowhere).is_empty());
        assert!(host.plugins().is_empty());
        let err = host.call("thumbs", "command/run", Value::Null).unwrap_err();
        assert_eq!(err.to_string(), "no plugin is called thumbs");
        assert_eq!(host.timeout(), Duration::from_secs(10));
    }

    #[test]
    fn a_plugin_starts_on_first_use_and_stops_politely() {
        let launcher = FakeLauncher::new(Mode::Echo);
        let ticker = Arc::new(ManualTicker::new());
        let host = host(Arc::clone(&launcher), &ticker, "");
        assert!(!host.is_running("line-counter"));
        let answer = host
            .call("line-counter", methods::COMMAND_RUN, Value::Null)
            .unwrap();
        assert_eq!(answer["done"], 1);
        assert!(host.is_running("line-counter"));
        host.call("line-counter", methods::COMMAND_RUN, Value::Null)
            .unwrap();
        assert_eq!(launcher.launches(), 1);
        let started = Instant::now();
        host.shutdown_all(QUIT_BUDGET);
        assert!(started.elapsed() < QUIT_BUDGET + Duration::from_secs(1));
        assert!(!host.is_running("line-counter"));
        assert!(host.take_events().is_empty());
    }

    #[test]
    fn plugins_off_and_disabled_plugins_say_so() {
        let ticker = Arc::new(ManualTicker::new());
        let off = host(FakeLauncher::new(Mode::Echo), &ticker, "").plugins_off();
        assert_eq!(
            off.call("line-counter", "command/run", Value::Null)
                .unwrap_err()
                .to_string(),
            "Plugins are off for this session."
        );
        let mut disabled = Host::new(&PluginSettings::default());
        let mut plugin = info("");
        plugin.enabled = false;
        disabled.add(plugin);
        assert_eq!(
            disabled
                .call("line-counter", "command/run", Value::Null)
                .unwrap_err()
                .to_string(),
            "Line counter is not enabled"
        );
    }

    #[test]
    fn a_plugin_that_never_starts_is_turned_off_after_two_tries() {
        let launcher = FakeLauncher::new(Mode::Silent);
        let ticker = Arc::new(ManualTicker::new());
        let host = host(launcher, &ticker, "");
        let first = host.call("line-counter", "command/run", Value::Null);
        assert!(first.is_err());
        assert_eq!(
            host.take_events(),
            [HostEvent::StartFailed {
                plugin: "line-counter".into(),
                message: Message::error("Line counter did not start in 1 second.")
            }]
        );
        let _ = host.call("line-counter", "command/run", Value::Null);
        assert_eq!(
            host.take_events(),
            [HostEvent::TurnedOff {
                plugin: "line-counter".into(),
                message: Message::warning("Line counter is off: it did not start twice.")
            }]
        );
        assert_eq!(
            host.call("line-counter", "command/run", Value::Null)
                .unwrap_err()
                .to_string(),
            "Line counter is off: it did not start twice."
        );
        host.turn_on("line-counter");
        assert!(host.off_reason("line-counter").is_none());
    }

    #[test]
    fn three_crashes_in_a_minute_turn_a_plugin_off() {
        let launcher = FakeLauncher::new(Mode::Echo);
        let ticker = Arc::new(ManualTicker::new());
        let host = host(Arc::clone(&launcher), &ticker, "");
        for round in 0..3 {
            host.call("line-counter", "command/run", Value::Null)
                .unwrap();
            // The plugin dies: kill it behind the host's back.
            let slot = host.slot(host.plugin("line-counter").unwrap());
            let live = slot.lock().unwrap().live.clone().unwrap();
            live.connection().kill_unexpectedly();
            for _ in 0..200 {
                if live.is_ended() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            ticker.advance(Duration::from_secs(10));
            host.tick();
            let events = host.take_events();
            if round < 2 {
                assert_eq!(
                    events,
                    [HostEvent::Crashed {
                        plugin: "line-counter".into(),
                        message: Message::warning(
                            "Line counter stopped unexpectedly. It starts again when you use it."
                        )
                    }]
                );
            } else {
                assert_eq!(
                    events,
                    [HostEvent::TurnedOff {
                        plugin: "line-counter".into(),
                        message: Message::warning("Line counter is off: it stopped 3 times.")
                    }]
                );
            }
        }
        assert_eq!(launcher.launches(), 3);
        assert!(
            host.call("line-counter", "command/run", Value::Null)
                .is_err()
        );
    }

    #[test]
    fn the_idle_stop_ends_a_plugin_nobody_uses() {
        let launcher = FakeLauncher::new(Mode::Echo);
        let ticker = Arc::new(ManualTicker::new());
        let host = host(launcher, &ticker, "idle_timeout = \"5m\"\n");
        host.call("line-counter", "command/run", Value::Null)
            .unwrap();
        ticker.advance(Duration::from_secs(299));
        host.tick();
        assert!(host.is_running("line-counter"));
        ticker.advance(Duration::from_secs(1));
        host.tick();
        assert_eq!(
            host.take_events(),
            [HostEvent::IdleStopped {
                plugin: "line-counter".into()
            }]
        );
        assert!(!host.is_running("line-counter"));
    }
}
