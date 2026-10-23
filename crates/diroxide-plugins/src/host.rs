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

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use diroxide_config::PluginSettings;
use diroxide_core::{Message, Place, Side};
use diroxide_tools::process::ProcessSpec;
use serde_json::Value;

use crate::error::PluginError;
use crate::lines;
use crate::live::{LivePlugin, Notice, RequestKind};
use crate::log::{PluginLog, Source};
use crate::manifest::Manifest;
use crate::protocol::{ErrorKind, Notification, PROTOCOL, Params, Request, RpcError, methods};
use crate::router::{CancelFlag, Connection, Event, Inbound, Outcome};
use crate::session::{InitializeParams, SessionError, handshake, unconfirmed};
use crate::supervisor::{
    Action, CrashPolicy, CrashVerdict, Flood, FloodGuard, SystemTicker, Ticker,
};
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

/// A panel as a plugin sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelSnapshot {
    /// Which panel.
    pub side: Side,
    /// Its place.
    pub place: Place,
    /// The entry under the cursor.
    pub cursor: Option<Place>,
    /// The selected entries.
    pub selection: Vec<Place>,
}

/// What the host offers plugins, implemented by the app over its own state,
/// dialogs and job engine, so a plugin's operations get the same safety,
/// trash, progress and undo as built-in ones.
pub trait HostServices: Send + Sync {
    /// `ui/announce`: says a message.
    fn announce(&self, message: Message);

    /// `ui/ask`: asks a yes-or-no question through an accessible dialog.
    /// `None` when the user cancels.
    fn ask_yes_no(&self, question: &str) -> Option<bool>;

    /// `ui/ask`: asks for text, with a label. `None` when cancelled.
    fn ask_text(&self, label: &str, initial: &str) -> Option<String>;

    /// `panel/get`: a panel's state; `None` for the active one.
    fn panel(&self, side: Option<Side>) -> PanelSnapshot;
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
        /// The method waiting.
        method: String,
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

/// The handler of one connection's inbound traffic: progress and log
/// lines feed the supervisor; floods stop the plugin.
struct PluginInbound {
    name: String,
    live: OnceLock<Weak<LivePlugin>>,
    log: PluginLog,
    flood: Mutex<FloodGuard>,
    flooded: Mutex<bool>,
    ticker: Arc<dyn Ticker>,
}

impl PluginInbound {
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
}

impl Inbound for PluginInbound {
    fn request(&self, request: &Request, _cancel: &CancelFlag) -> Outcome {
        Err(RpcError::new(
            ErrorKind::MethodNotFound,
            format!("the host does not serve {} yet", request.method),
        ))
    }

    fn notification(&self, notification: &Notification) {
        match notification.method.as_str() {
            methods::PROGRESS => {
                let token = notification.params.get("token").and_then(Value::as_str);
                let waiting = notification
                    .params
                    .get("value")
                    .and_then(|v| v.get("state"))
                    .and_then(Value::as_str)
                    .map(|state| state == "waiting_for_user");
                if let (Some(token), Some(live)) = (token, self.live()) {
                    live.progress(token, waiting);
                }
            }
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
            Event::Stray(_) => self.count(Flood::Stray),
            Event::UnknownAnswer(_) | Event::Ended => {}
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
    events: Mutex<Vec<HostEvent>>,
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
            events: Mutex::new(Vec::new()),
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
        let inbound = Arc::new(PluginInbound {
            name: name.clone(),
            live: OnceLock::new(),
            log: log.clone(),
            flood: Mutex::new(FloodGuard::default()),
            flooded: Mutex::new(false),
            ticker: Arc::clone(&self.ticker),
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
        let live = self.live(info)?;
        let name = info.manifest.name.clone();
        live.call(method, params, kind)
            .map_err(|error| match ErrorKind::from_code(error.code) {
                Some(ErrorKind::TimedOut) => PluginError::Timeout(name),
                Some(ErrorKind::PluginUnavailable) => PluginError::Stopped(name),
                _ => PluginError::Remote {
                    plugin: name,
                    code: error.code,
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
                    Notice::Slow(request) => self.event(HostEvent::Waiting {
                        plugin: id.clone(),
                        method: live
                            .connection()
                            .router()
                            .method_of(&request)
                            .unwrap_or_default(),
                    }),
                    Notice::TimedOut(_, after) => self.event(HostEvent::TimedOut {
                        plugin: id.clone(),
                        message: Message::error(lines::timed_out(&name, after)),
                    }),
                }
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
    use crate::testing::{FakePlugin, Mode};
    use crate::transport::MemoryTransport;

    /// Starts in-process fake plugins in a mode.
    struct FakeLauncher {
        mode: Mutex<Mode>,
        launches: Mutex<usize>,
    }

    impl FakeLauncher {
        fn new(mode: Mode) -> Arc<Self> {
            Arc::new(Self {
                mode: Mutex::new(mode),
                launches: Mutex::new(0),
            })
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(
            &self,
            info: &PluginInfo,
            _log: &PluginLog,
        ) -> Result<Arc<dyn PluginTransport>, PluginError> {
            *self.launches.lock().unwrap() += 1;
            let mode = *self.mode.lock().unwrap();
            let (host, peer) = MemoryTransport::pair(&info.manifest.name)?;
            thread::spawn(move || FakePlugin::new(mode).serve(peer));
            Ok(Arc::new(host))
        }
    }

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
        assert_eq!(*launcher.launches.lock().unwrap(), 1);
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
        assert_eq!(*launcher.launches.lock().unwrap(), 3);
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
