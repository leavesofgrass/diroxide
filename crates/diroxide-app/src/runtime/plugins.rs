//! Plugins on the runtime's side (W4d, R22 commits 36 to 40): the scan,
//! the consent record, one host per enabled process plugin with its
//! supervisor, the app's [`HostServices`], and plugin commands run as jobs.
//!
//! - **The scan** reads the plugin folders, the scripts folder and the
//!   consent record on a worker and answers [`PluginEvent::Found`]; the
//!   runtime adopts the catalog as it passes ([`PluginRuntime::adopt`]):
//!   a host for each enabled process plugin, stopped when it is disabled
//!   or changed, and its providers in the registry.
//! - **Each host** has its own [`AppServices`], so every sentence and
//!   question carries its plugin; a pump forwards the host's events (a
//!   crash, a time-out, a lost provider) to the app every tenth of a
//!   second.
//! - **A plugin's call** that needs the app (a question, the panels, a
//!   job, a lister) goes out as [`PluginEvent::Call`] and waits on the
//!   router's thread for the app's [`Reply`]. While the owner thinks, the
//!   worker counts as idle, as a job's own questions do.
//! - **A run** checks the consent again first (a changed manifest or
//!   program asks again), then runs a script through the script runner or
//!   calls the plugin's `command/run`; Escape raises the cancel file and
//!   the request's cancel. Plugin-command jobs are not queued (R25's seed).
//! - **`--no-plugins`** starts nothing: the scan answers with the catalog
//!   marked off, and no host is made.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use diroxide_config::{Paths, PluginSettings};
use diroxide_core::{CommandId, Entry, JobId, JobRequest, Message};
use diroxide_ops::JobReport;
use diroxide_plugins::PluginError;
use diroxide_plugins::ask::{Answer, Question, TextDocument};
use diroxide_plugins::host::{Host, HostServices, Launcher, PluginInfo, QUIT_BUDGET};
use diroxide_plugins::listers::ListerView;
use diroxide_plugins::live::CallOptions;
use diroxide_plugins::log::{PluginLog, Source};
use diroxide_plugins::panels::{PanelChange, PanelChoice, PanelSnapshot};
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use diroxide_plugins::provider::PluginProvider;
use diroxide_plugins::router::CancelFlag;
use diroxide_plugins::script::consent::{ConsentState, ConsentStore};
use diroxide_plugins::script::context::StateFolders;
use diroxide_plugins::script::directives::Directive;
use diroxide_plugins::script::installed;
use diroxide_plugins::script::interpreter::Lookup;
use diroxide_plugins::script::runner::{self, Cancel, RunEnv, RunSink};
use diroxide_plugins::services::settings_for;
use diroxide_plugins::shapes::{PanelRef, WireContext, WireEntry};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::{CancelToken, Registry};
use serde_json::{Value, json};

use super::{Shared, Work, lock};
use crate::app::AppEvent;
use crate::plugins::catalog::{self, Catalog, Item, Roots, Runs};
use crate::plugins::{
    Call, CallId, Finished, Outcome, PluginEvent, PluginRequest, Reply, RunRequest,
};

/// How often a host's events are taken to the app.
const PUMP_EVERY: Duration = Duration::from_millis(100);

fn send(shared: &Shared, event: PluginEvent) {
    shared.send(AppEvent::Plugin(Box::new(event)));
}

/// A call waiting for the app's reply, and whether its worker was counted
/// idle while the owner thinks.
struct Waiting {
    reply_to: Sender<Reply>,
    lowered: bool,
}

/// The calls plugins made that wait for the app.
#[derive(Default)]
pub(super) struct CallTable {
    next: AtomicU64,
    waiting: Mutex<HashMap<CallId, Waiting>>,
}

/// Whether a call waits for the owner (a question, a document the plugin
/// waits on, a job's confirmation) or only for the app's state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    Owner,
    App,
}

/// The app's services for one plugin's host.
pub(super) struct AppServices {
    plugin: String,
    shared: Arc<Shared>,
    calls: Arc<CallTable>,
}

impl AppServices {
    fn call(&self, request: Call, wait: Wait) -> Option<Reply> {
        let id = CallId::new(self.calls.next.fetch_add(1, Ordering::SeqCst) + 1);
        let (reply_to, replies) = mpsc::channel();
        lock(&self.calls.waiting).insert(
            id,
            Waiting {
                reply_to,
                lowered: false,
            },
        );
        send(
            &self.shared,
            PluginEvent::Call {
                call: id,
                plugin: self.plugin.clone(),
                request,
            },
        );
        // Counted idle only after the question is on its way, so a test's
        // wait never ends before it arrives; never when it was answered
        // already.
        if wait == Wait::Owner
            && let Some(waiting) = lock(&self.calls.waiting).get_mut(&id)
        {
            waiting.lowered = self.shared.lower();
        }
        let reply = replies.recv().ok();
        if reply.is_none()
            && let Some(waiting) = lock(&self.calls.waiting).remove(&id)
            && waiting.lowered
        {
            self.shared.begin();
        }
        reply
    }
}

impl HostServices for AppServices {
    fn announce(&self, message: Message) {
        send(
            &self.shared,
            PluginEvent::Said {
                plugin: self.plugin.clone(),
                message,
            },
        );
    }

    fn progress(&self, message: Message) {
        send(
            &self.shared,
            PluginEvent::Progress {
                plugin: self.plugin.clone(),
                message,
            },
        );
    }

    fn ask(&self, question: &Question) -> Answer {
        match self.call(Call::Ask(Box::new(question.clone())), Wait::Owner) {
            Some(Reply::Answer(answer)) => answer,
            _ => Answer::Cancelled,
        }
    }

    fn show_text(&self, document: &TextDocument) -> Result<(), RpcError> {
        let wait = if document.wait {
            Wait::Owner
        } else {
            Wait::App
        };
        match self.call(Call::ShowText(Box::new(document.clone())), wait) {
            Some(Reply::Done(Err(error))) => Err(error),
            _ => Ok(()),
        }
    }

    fn panel(&self, which: PanelChoice) -> Option<PanelSnapshot> {
        match self.call(Call::Panel(which), Wait::App) {
            Some(Reply::Panel(panel)) => panel.map(|p| *p),
            _ => None,
        }
    }

    fn set_panel(&self, change: &PanelChange) -> Result<(), RpcError> {
        match self.call(Call::SetPanel(Box::new(change.clone())), Wait::App) {
            Some(Reply::Done(result)) => result.map(|_| ()),
            _ => Err(RpcError::new(
                ErrorKind::Failed,
                "the panels did not answer",
            )),
        }
    }

    fn run_job(&self, request: &JobRequest, cancel: &CancelFlag) -> Result<JobReport, RpcError> {
        if cancel.is_cancelled() {
            return Err(RpcError::new(ErrorKind::RequestCancelled, "cancelled"));
        }
        match self.call(Call::RunJob(Box::new(request.clone())), Wait::Owner) {
            Some(Reply::Job(result)) => result.map(|report| *report),
            _ => Err(RpcError::new(ErrorKind::Failed, "the job did not run")),
        }
    }

    fn open_lister(&self, lister: &ListerView) -> Result<(), RpcError> {
        match self.call(Call::OpenLister(Box::new(lister.clone())), Wait::App) {
            Some(Reply::Done(result)) => result.map(|_| ()),
            _ => Err(RpcError::new(
                ErrorKind::Failed,
                "the panels did not answer",
            )),
        }
    }

    fn execute(&self, command: &CommandId, args: &Params) -> Result<Value, RpcError> {
        let call = Call::Execute {
            command: command.clone(),
            args: args.clone(),
        };
        match self.call(call, Wait::App) {
            Some(Reply::Done(result)) => result,
            _ => Err(RpcError::new(ErrorKind::Failed, "the app did not answer")),
        }
    }
}

/// One enabled process plugin's host.
struct PluginHost {
    host: Arc<Host>,
    /// What the host was made from: a change makes it again.
    item: Item,
}

/// The cancels of the plugin commands that run.
#[derive(Clone, Default)]
struct RunCancel {
    script: Cancel,
    request: CancelToken,
}

/// Everything the runtime keeps about plugins.
pub(crate) struct PluginRuntime {
    off: bool,
    launcher: Option<Arc<dyn Launcher>>,
    settings: PluginSettings,
    hosts: HashMap<String, PluginHost>,
    catalog: Arc<Mutex<Catalog>>,
    calls: Arc<CallTable>,
    runs: Arc<Mutex<HashMap<JobId, RunCancel>>>,
    lookup: Option<Lookup>,
    /// The schemes of the providers in the registry now.
    schemes: Vec<String>,
}

impl Default for PluginRuntime {
    fn default() -> Self {
        Self {
            off: false,
            launcher: None,
            settings: PluginSettings::default(),
            hosts: HashMap::new(),
            catalog: Arc::new(Mutex::new(Catalog::default())),
            calls: Arc::new(CallTable::default()),
            runs: Arc::new(Mutex::new(HashMap::new())),
            lookup: None,
            schemes: Vec::new(),
        }
    }
}

impl std::fmt::Debug for PluginRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRuntime")
            .field("off", &self.off)
            .field("hosts", &self.hosts.len())
            .finish_non_exhaustive()
    }
}

/// The plugins' state folder: logs, runs, consent and settings.
pub(crate) fn plugins_state(paths: &Paths) -> PathBuf {
    paths.state_dir.join("plugins")
}

fn roots(paths: &Paths) -> Roots {
    let examples = catalog::examples_dir(&paths.components_dir());
    let components = paths.components_dir();
    Roots {
        plugins: paths.plugins_dir(),
        examples: examples.is_dir().then_some(examples),
        components: components.is_dir().then_some(components),
        state: plugins_state(paths),
    }
}

fn tools_of(paths: Option<&Paths>) -> ToolRegistry {
    match paths {
        Some(paths) => ToolRegistry::builtin().with_overrides(diroxide_config::load(paths).tools),
        None => ToolRegistry::builtin(),
    }
}

fn run_env(item: &Item, paths: Option<&Paths>, lookup: &Lookup) -> RunEnv {
    let state = paths.map_or_else(std::env::temp_dir, plugins_state);
    let mut env = RunEnv::system(StateFolders {
        runs: state.join("runs"),
        data: state.join(&item.id).join("data"),
    });
    env.lookup = lookup.clone();
    env.registry = tools_of(paths);
    if let Some(manifest) = &item.manifest {
        let values = crate::plugins::settings::read_values(
            &crate::plugins::settings::settings_path(&state, &item.id),
        );
        env.settings = settings_for(manifest, &values);
    }
    env
}

impl PluginRuntime {
    /// Turns plugins off for the session: nothing starts.
    pub(crate) fn turn_off(&mut self) {
        self.off = true;
    }

    /// Starts plugins through another launcher (tests start fakes).
    pub(crate) fn set_launcher(&mut self, launcher: Arc<dyn Launcher>) {
        self.launcher = Some(launcher);
    }

    /// Takes the app's `[plugins]` settings.
    pub(crate) fn set_settings(&mut self, settings: &PluginSettings) {
        if self.settings != *settings {
            self.settings = settings.clone();
        }
    }

    fn lookup(&mut self) -> Lookup {
        self.lookup.get_or_insert_with(Lookup::system).clone()
    }

    /// Cancels a plugin command's run; whether one was running.
    pub(crate) fn cancel(&self, job: JobId) -> bool {
        match lock(&self.runs).get(&job) {
            Some(run) => {
                run.script.cancel();
                run.request.cancel();
                true
            }
            None => false,
        }
    }

    /// Stops every plugin within the quit budget.
    pub(crate) fn shutdown(&mut self) {
        for (_, host) in self.hosts.drain() {
            host.host.shutdown_all(QUIT_BUDGET);
        }
    }

    /// Adopts a catalog as it passes to the app: a host for each enabled
    /// process plugin (kept when unchanged), the others stopped. Returns
    /// whether the providers changed, so the registry is made again.
    pub(crate) fn adopt(
        &mut self,
        found: &Catalog,
        shared: &Arc<Shared>,
        paths: Option<&Paths>,
        base: &Arc<Registry>,
    ) -> bool {
        *lock(&self.catalog) = found.clone();
        let wanted: Vec<&Item> = found
            .items
            .iter()
            .filter(|i| !found.off && i.contributes() && i.is_process())
            .collect();
        let stale: Vec<String> = self
            .hosts
            .iter()
            .filter(|(id, host)| {
                !wanted
                    .iter()
                    .any(|i| &i.id == *id && i.fingerprint == host.item.fingerprint)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(old) = self.hosts.remove(&id) {
                let host = old.host;
                let _ = thread::Builder::new()
                    .name("plugin-stop".to_owned())
                    .spawn(move || host.shutdown_all(QUIT_BUDGET));
            }
        }
        for item in wanted {
            if self.hosts.contains_key(&item.id) {
                continue;
            }
            let Some(manifest) = item.manifest.clone() else {
                continue;
            };
            let services = Arc::new(AppServices {
                plugin: item.id.clone(),
                shared: Arc::clone(shared),
                calls: Arc::clone(&self.calls),
            });
            let mut host = Host::new(&self.settings)
                .with_registry(Arc::clone(base))
                .with_services(services);
            if let Some(paths) = paths {
                host = host.with_state_dir(plugins_state(paths));
            }
            if let Some(launcher) = &self.launcher {
                host = host.with_launcher(Arc::clone(launcher));
            }
            host.add(PluginInfo {
                manifest,
                folder: item.path.clone(),
                enabled: true,
            });
            let host = Arc::new(host);
            if let Some(paths) = paths {
                let values = crate::plugins::settings::read_values(
                    &crate::plugins::settings::settings_path(&plugins_state(paths), &item.id),
                );
                if !values.is_empty() {
                    host.set_settings(&item.id, values);
                }
            }
            let _ = Host::start_supervisor(&host);
            let _ = pump(&host, shared);
            self.hosts.insert(
                item.id.clone(),
                PluginHost {
                    host,
                    item: item.clone(),
                },
            );
        }
        let mut schemes: Vec<String> = self
            .hosts
            .values()
            .flat_map(|h| {
                h.item
                    .manifest
                    .iter()
                    .flat_map(|m| m.providers.iter().map(|p| p.scheme.clone()))
            })
            .collect();
        schemes.sort();
        let changed = schemes != self.schemes;
        self.schemes = schemes;
        changed
    }

    /// The enabled plugins' providers that offer places, for the
    /// go-to-place list's Sites group (lane A2, R25 commit 78).
    pub(super) fn site_sources(&self) -> Vec<SiteSource> {
        let mut sources: Vec<SiteSource> = Vec::new();
        for (id, host) in &self.hosts {
            let Some(manifest) = host.item.manifest.as_ref() else {
                continue;
            };
            for provider in &manifest.providers {
                if provider.methods.contains("places") {
                    sources.push((
                        Arc::clone(&host.host),
                        id.clone(),
                        provider.scheme.clone(),
                        manifest.name.clone(),
                    ));
                }
            }
        }
        sources.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));
        sources
    }

    /// The enabled plugins with hooks, for the engine's hook runner (lane
    /// A2, R25 commit 81).
    pub(crate) fn hooked(&self) -> Vec<super::hooks::Hooked> {
        let mut hooked: Vec<super::hooks::Hooked> = self
            .hosts
            .iter()
            .filter_map(|(id, host)| {
                let manifest = host.item.manifest.as_ref()?;
                (!manifest.hooks.is_empty()).then(|| super::hooks::Hooked {
                    id: id.clone(),
                    name: manifest.name.clone(),
                    host: Arc::clone(&host.host),
                    hooks: manifest.hooks.clone(),
                    limit: manifest.timeouts.hook.unwrap_or(Duration::from_secs(2)),
                })
            })
            .collect();
        hooked.sort_by(|a, b| a.id.cmp(&b.id));
        hooked
    }

    /// The registry with every enabled plugin's providers added to `base`.
    pub(crate) fn registry_with_providers(&self, base: &Registry) -> Registry {
        let mut registry = base.clone();
        for host in self.hosts.values() {
            for (scheme, provider) in PluginProvider::all(&host.host) {
                registry.register(&scheme, provider);
            }
        }
        registry
    }

    /// Answers a plugin's call.
    pub(crate) fn reply(&self, shared: &Shared, call: CallId, reply: Reply) {
        let waiting = lock(&self.calls.waiting).remove(&call);
        if let Some(waiting) = waiting {
            if waiting.lowered {
                shared.begin();
            }
            if waiting.reply_to.send(reply).is_err() && waiting.lowered {
                shared.end();
            }
        }
    }

    /// Carries out one request; the work that reads or runs goes to a
    /// worker through `worker`.
    pub(crate) fn request(
        &mut self,
        request: PluginRequest,
        shared: &Arc<Shared>,
        paths: Option<&Paths>,
        worker: &dyn Fn(Work),
    ) {
        match request {
            PluginRequest::Scan => self.scan(paths, worker),
            PluginRequest::Enable { id } => {
                let catalog = lock(&self.catalog).clone();
                let paths = paths.cloned();
                let lookup = self.lookup();
                worker(Box::new(move |shared| {
                    let note = enable(&catalog, &id, paths.as_ref(), &lookup);
                    send(shared, PluginEvent::Note(note));
                    send(
                        shared,
                        PluginEvent::Found(Box::new(scan_now(paths.as_ref(), &lookup, false))),
                    );
                }));
            }
            PluginRequest::Disable { id } => {
                let catalog = lock(&self.catalog).clone();
                let paths = paths.cloned();
                let lookup = self.lookup();
                worker(Box::new(move |shared| {
                    let note = disable(&catalog, &id, paths.as_ref());
                    send(shared, PluginEvent::Note(note));
                    send(
                        shared,
                        PluginEvent::Found(Box::new(scan_now(paths.as_ref(), &lookup, false))),
                    );
                }));
            }
            PluginRequest::TurnOn { id } => {
                if let Some(host) = self.hosts.get(&id) {
                    host.host.turn_on(&id);
                    let name = host.item.name.clone();
                    send(
                        shared,
                        PluginEvent::Note(Message::info(format!("On again: {name}."))),
                    );
                }
            }
            PluginRequest::Run(run) => self.run(*run, paths, worker),
            PluginRequest::Reply { call, reply } => self.reply(shared, call, *reply),
            PluginRequest::Setting { id, key, value } => {
                let Some(paths) = paths else {
                    send(
                        shared,
                        PluginEvent::Note(Message::error(
                            "Could not save the setting: there is no state folder.",
                        )),
                    );
                    return;
                };
                let path = crate::plugins::settings::settings_path(&plugins_state(paths), &id);
                let mut values = crate::plugins::settings::read_values(&path);
                match value {
                    Some(value) => {
                        values.insert(key, value);
                    }
                    None => {
                        values.remove(&key);
                    }
                }
                if let Err(e) = crate::plugins::settings::write_values(&path, &values) {
                    send(
                        shared,
                        PluginEvent::Note(Message::error(format!(
                            "Could not save the setting: {e}."
                        ))),
                    );
                    return;
                }
                if let Some(host) = self.hosts.get(&id) {
                    host.host.set_settings(&id, values);
                }
            }
            PluginRequest::Log { id } => {
                let path = paths.map(|p| crate::plugins::log::log_path(&plugins_state(p), &id));
                worker(Box::new(move |shared| {
                    let lines = path
                        .as_deref()
                        .map(crate::plugins::log::read_newest_first)
                        .unwrap_or_default();
                    send(shared, PluginEvent::Logged { id, lines });
                }));
            }
            PluginRequest::Complete {
                call,
                question,
                text,
                cursor,
            } => {
                worker(Box::new(move |shared| {
                    let Some(diroxide_plugins::ask::Completion::Plugin(completer)) =
                        &question.completion
                    else {
                        return;
                    };
                    // A query the next keystroke cancelled says nothing.
                    if let Ok(found) = completer.query(&text, cursor) {
                        let candidates = found.into_iter().map(|c| (c.text, c.kind)).collect();
                        send(
                            shared,
                            PluginEvent::Candidates {
                                call,
                                text,
                                candidates,
                            },
                        );
                    }
                }));
            }
            PluginRequest::ListerRows {
                plugin,
                lister,
                offset,
                refresh,
            } => {
                let Some(host) = self.hosts.get(&plugin).map(|h| Arc::clone(&h.host)) else {
                    return;
                };
                worker(Box::new(move |shared| {
                    match host.lister_rows(&plugin, &lister, offset, refresh) {
                        Ok(rows) => send(
                            shared,
                            PluginEvent::ListerRows {
                                plugin,
                                lister,
                                rows,
                                refresh,
                            },
                        ),
                        // A plugin without `lister/rows` brings its rows up
                        // to date with its own command (lane A3).
                        Err(PluginError::Remote { code, .. })
                            if refresh && code == ErrorKind::MethodNotFound.code() =>
                        {
                            send(shared, PluginEvent::ListerNoRows { plugin, lister });
                        }
                        Err(error) => send(
                            shared,
                            PluginEvent::Note(Message::error(
                                diroxide_plugins::lines::could_not_run(
                                    "lister rows",
                                    &error.to_string(),
                                ),
                            )),
                        ),
                    }
                }));
            }
            PluginRequest::ListerClosed { plugin, lister } => {
                if let Some(host) = self.hosts.get(&plugin) {
                    host.host.lister_closed(&plugin, &lister);
                }
            }
        }
    }

    fn scan(&mut self, paths: Option<&Paths>, worker: &dyn Fn(Work)) {
        let paths = paths.cloned();
        let off = self.off;
        let lookup = self.lookup();
        worker(Box::new(move |shared| {
            let catalog = scan_now(paths.as_ref(), &lookup, off);
            send(shared, PluginEvent::Found(Box::new(catalog)));
        }));
    }

    fn run(&mut self, run: RunRequest, paths: Option<&Paths>, worker: &dyn Fn(Work)) {
        let catalog = lock(&self.catalog).clone();
        let job = run.job;
        let cancel = RunCancel::default();
        lock(&self.runs).insert(job, cancel.clone());
        let runs = Arc::clone(&self.runs);
        let host = catalog
            .any_command(&run.command)
            .and_then(|(item, _)| self.hosts.get(&item.id))
            .map(|h| Arc::clone(&h.host));
        let paths = paths.cloned();
        let lookup = self.lookup();
        let off = self.off;
        worker(Box::new(move |shared| {
            let finished = run_command(
                &catalog,
                &run,
                host.as_deref(),
                paths.as_ref(),
                &lookup,
                &cancel,
                shared,
                off,
            );
            lock(&runs).remove(&job);
            send(shared, PluginEvent::Finished(Box::new(finished)));
        }));
    }
}

/// A scan now, on the caller's thread: the catalog of every plugin and
/// script, or an empty one marked off.
pub(crate) fn scan_now(paths: Option<&Paths>, lookup: &Lookup, off: bool) -> Catalog {
    let Some(paths) = paths else {
        return Catalog {
            off,
            ..Catalog::default()
        };
    };
    let roots = roots(paths);
    let mut env = RunEnv::system(StateFolders {
        runs: roots.state.join("runs"),
        data: roots.state.join("data"),
    });
    env.lookup = lookup.clone();
    env.registry = tools_of(Some(paths));
    let mut found = catalog::scan(&roots, diroxide_components::sha256_hex, lookup, &env);
    found.off = off;
    for item in &mut found.items {
        if item.manifest.is_some() {
            item.values = crate::plugins::settings::read_values(
                &crate::plugins::settings::settings_path(&roots.state, &item.id),
            );
        }
    }
    found
}

/// Records the owner's yes for an item, with what its files are now.
fn enable(catalog: &Catalog, id: &str, paths: Option<&Paths>, lookup: &Lookup) -> Message {
    let (Some(item), Some(paths)) = (catalog.item(id), paths) else {
        return Message::error(format!("Could not enable {id}: it was not found."));
    };
    let fingerprint = installed::read_path(&item.path, lookup.os).and_then(|installed| {
        catalog::fingerprint_and_subject(&installed, diroxide_components::sha256_hex, lookup).0
    });
    let Some(fingerprint) = fingerprint else {
        return Message::error(format!(
            "Could not enable {}: it cannot be read.",
            item.name
        ));
    };
    let mut store = ConsentStore::open(&plugins_state(paths));
    match store.enable(&item.id, item.consent_kind(), fingerprint) {
        Ok(()) => Message::done(diroxide_plugins::script::consent::enabled_line(&item.name)),
        Err(e) => Message::error(format!("Could not save the consent: {e}.")),
    }
}

/// Forgets the owner's yes for an item.
fn disable(catalog: &Catalog, id: &str, paths: Option<&Paths>) -> Message {
    let Some(paths) = paths else {
        return Message::error(format!("Could not disable {id}: there is no state folder."));
    };
    let name = catalog
        .item(id)
        .map_or_else(|| id.to_owned(), |i| i.name.clone());
    let mut store = ConsentStore::open(&plugins_state(paths));
    match store.disable(id) {
        Ok(true) => Message::done(diroxide_plugins::script::consent::disabled_line(&name)),
        Ok(false) => Message::info(format!("{name} was not enabled.")),
        Err(e) => Message::error(format!("Could not save the consent: {e}.")),
    }
}

/// The item's consent state now, read again from its files: a change
/// since the scan asks again.
fn state_now(item: &Item, paths: Option<&Paths>, lookup: &Lookup) -> ConsentState {
    let Some(paths) = paths else {
        return ConsentState::NotEnabled;
    };
    let Some(installed) = installed::read_path(&item.path, lookup.os) else {
        return ConsentState::Changed;
    };
    let (fingerprint, _) =
        catalog::fingerprint_and_subject(&installed, diroxide_components::sha256_hex, lookup);
    let Some(fingerprint) = fingerprint else {
        return ConsentState::Changed;
    };
    ConsentStore::open(&plugins_state(paths)).state(&item.id, &fingerprint)
}

/// Forwards a script's live effects to the app.
struct ForwardRun<'a> {
    job: JobId,
    source: String,
    shared: &'a Arc<Shared>,
    log: PluginLog,
    plugin: String,
}

impl RunSink for ForwardRun<'_> {
    fn act(&mut self, directive: &Directive) {
        if *directive == Directive::Refresh {
            send(self.shared, PluginEvent::Refresh);
            return;
        }
        let Some(message) = directive.message(&self.source) else {
            return;
        };
        let event = if matches!(directive, Directive::Progress { .. }) {
            PluginEvent::Progress {
                plugin: self.plugin.clone(),
                message,
            }
        } else {
            PluginEvent::Said {
                plugin: self.plugin.clone(),
                message,
            }
        };
        send(self.shared, event);
    }

    fn output(&mut self, line: &str) {
        send(
            self.shared,
            PluginEvent::Output {
                job: self.job,
                line: line.to_owned(),
            },
        );
    }

    fn log(&mut self, line: &str) {
        self.log.write(Source::Host, line);
    }

    fn note(&mut self, message: &Message) {
        send(
            self.shared,
            PluginEvent::Said {
                plugin: self.plugin.clone(),
                message: message.clone(),
            },
        );
    }
}

fn wire_entry(entry: &diroxide_plugins::script::context::Picked) -> WireEntry {
    let row = Entry::new(entry.place.name().unwrap_or_default(), entry.kind);
    WireEntry::from_entry(&row, Some(&entry.place))
}

/// The `command/run` parameters of a run: the plugin-local command and
/// the context, with a lister's rows as the selection.
fn command_params(local: &str, run: &RunRequest) -> Value {
    let context = &run.context;
    let mut wire = WireContext::new(
        &run.command,
        PanelRef::for_place(&context.active),
        PanelRef::for_place(&context.other),
    );
    match &run.lister {
        Some((lister, rows)) => {
            let rows: Vec<WireEntry> = rows
                .iter()
                .map(|row| WireEntry::from_entry(row, None))
                .collect();
            wire = wire.with_selection(None, rows);
            json!({"command": local, "lister": lister, "context": wire})
        }
        None => {
            wire = wire.with_selection(
                context.cursor.as_ref().map(wire_entry),
                context.marked.iter().map(wire_entry).collect(),
            );
            json!({"command": local, "context": wire})
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_command(
    catalog: &Catalog,
    run: &RunRequest,
    host: Option<&Host>,
    paths: Option<&Paths>,
    lookup: &Lookup,
    cancel: &RunCancel,
    shared: &Arc<Shared>,
    off: bool,
) -> Finished {
    let finished = |outcome: Outcome, last_error: Option<String>| Finished {
        job: run.job,
        command: run.command.clone(),
        outcome,
        last_error,
        limit: Duration::ZERO,
    };
    if off || catalog.off {
        return finished(
            Outcome::Refused(diroxide_plugins::lines::plugins_off()),
            None,
        );
    }
    let Some((item, entry)) = catalog.any_command(&run.command) else {
        return finished(
            Outcome::Refused(format!(
                "No enabled plugin has the command {}.",
                run.command
            )),
            None,
        );
    };
    match state_now(item, paths, lookup) {
        ConsentState::Enabled => {}
        ConsentState::Changed => {
            let rows = installed::read_path(&item.path, lookup.os)
                .and_then(|i| {
                    catalog::fingerprint_and_subject(&i, diroxide_components::sha256_hex, lookup).1
                })
                .map(|s| s.changed_question())
                .unwrap_or_else(|| item.changed_rows.clone());
            return finished(Outcome::Changed(rows), None);
        }
        ConsentState::NotEnabled if run.once => {}
        ConsentState::NotEnabled => {
            return finished(
                Outcome::Refused(format!("{} is not enabled.", item.name)),
                None,
            );
        }
    }
    if cancel.script.is_cancelled() {
        return finished(Outcome::Cancelled, None);
    }
    match &entry.runs {
        Runs::Script(command) => {
            let env = run_env(item, paths, lookup);
            let log = match paths {
                Some(paths) => PluginLog::at(crate::plugins::log::log_path(
                    &plugins_state(paths),
                    &item.id,
                )),
                None => PluginLog::in_memory(),
            };
            let mut sink = ForwardRun {
                job: run.job,
                source: command.source.clone(),
                shared,
                log: log.clone(),
                plugin: item.id.clone(),
            };
            match runner::run(command, &run.context, &env, &mut sink, &cancel.script) {
                Ok(report) => {
                    for line in &report.stderr {
                        log.write(Source::Stderr, line);
                    }
                    let last = report.stderr.last().cloned();
                    finished(Outcome::Script(Box::new(report)), last)
                }
                Err(error) => {
                    let sentence = error.sentence(&command.title);
                    log.write(Source::Host, &sentence);
                    finished(Outcome::Refused(sentence), None)
                }
            }
        }
        Runs::Process => {
            let Some(host) = host else {
                return finished(
                    Outcome::Refused(format!("{} is not running: enable it first.", item.name)),
                    None,
                );
            };
            let params = command_params(&entry.local, run);
            let options = CallOptions {
                cancel: Some(&cancel.request),
                batches: None,
            };
            let result = match host.call_with(&item.id, methods::COMMAND_RUN, params, options) {
                // The host cancels a request that ran past its limit; a
                // plugin that answers that cancel still timed out.
                Err(PluginError::Remote { code, .. })
                    if code == ErrorKind::RequestCancelled.code()
                        && !cancel.request.is_cancelled() =>
                {
                    Err(PluginError::Timeout(item.name.clone()))
                }
                other => other,
            };
            let last = host
                .log(&item.id)
                .and_then(|log| log.tail().last().cloned());
            let last = if result.is_err() { last } else { None };
            let mut done = finished(Outcome::Process(result), last);
            done.limit = item
                .manifest
                .as_ref()
                .and_then(|m| m.timeouts.command)
                .unwrap_or(host.timeout());
            done
        }
    }
}

/// A provider that may offer places: its host, plugin, scheme and the
/// plugin's name.
pub(super) type SiteSource = (Arc<Host>, String, String, String);

/// The places the plugins offer (`provider/places`), as rows for the
/// go-to-place list; asking never signs in, and a plugin that fails or
/// offers none adds nothing.
pub(super) fn sites_of(sources: &[SiteSource]) -> Vec<crate::app::SiteRow> {
    let ctx = diroxide_vfs::OpContext::new();
    let mut rows = Vec::new();
    for (host, plugin, scheme, name) in sources {
        let Some(provider) = PluginProvider::new(host, plugin, scheme) else {
            continue;
        };
        let Ok(places) = provider.places(&ctx) else {
            continue;
        };
        rows.extend(places.rows.into_iter().map(|row| crate::app::SiteRow {
            name: row.name,
            place: row.place,
            state: row.state,
            detail: row.detail,
            plugin: name.clone(),
            plugin_id: plugin.clone(),
        }));
    }
    rows
}

/// The storages of the plugins' connected places (`provider/volumes`),
/// for the Volumes list: one row a storage; asking never signs in, and a
/// place not connected is not asked (lane A3, R25 commit 88).
pub(super) fn remote_volumes(sources: &[SiteSource]) -> Vec<diroxide_vfs::RemoteVolume> {
    let ctx = diroxide_vfs::OpContext::new();
    let mut volumes = Vec::new();
    for (host, plugin, scheme, _) in sources {
        let Some(provider) = PluginProvider::new(host, plugin, scheme) else {
            continue;
        };
        let Ok(places) = provider.places(&ctx) else {
            continue;
        };
        for place in places.rows.iter().filter(|row| row.state == "connected") {
            let Ok(rows) = provider.volumes(&place.place, &ctx) else {
                continue;
            };
            volumes.extend(rows.into_iter().map(|row| diroxide_vfs::RemoteVolume {
                name: row.name,
                scheme: scheme.clone(),
                place: row.place,
                free: row.free,
                total: row.total,
                removable: row.removable,
            }));
        }
    }
    volumes
}

/// Forwards a host's events to the app every tenth of a second while it
/// lives.
fn pump(host: &Arc<Host>, shared: &Arc<Shared>) -> std::io::Result<JoinHandle<()>> {
    let weak: Weak<Host> = Arc::downgrade(host);
    let shared = Arc::clone(shared);
    thread::Builder::new()
        .name("plugin-events".to_owned())
        .spawn(move || {
            while let Some(host) = weak.upgrade() {
                for event in host.take_events() {
                    send(&shared, PluginEvent::Host(event));
                }
                drop(host);
                thread::sleep(PUMP_EVERY);
            }
        })
}
