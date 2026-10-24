//! The runtime: carries out the effects and requests that are I/O
//! (listing, planning and running jobs, watching, opening files, running
//! programs, writing the state folder) and turns their results into
//! [`AppEvent`]s. Frontends handle the screen and speech effects
//! themselves and hand the rest here.
//!
//! The work runs on worker threads, never on the input thread: one per
//! listing, one per plan, one per running job. Results come back on a
//! channel; [`Runtime::poll`] takes what is ready without waiting, and a
//! waker set with [`Runtime::set_waker`] tells the frontend that something
//! arrived. A listing's answer carries its request id, and the app drops
//! an answer to a request it no longer waits for.
//!
//! A job's questions (a conflict, an item's error, a password) block its
//! worker until the app answers through [`Request::Answer`]; a cancel
//! while a question is open answers it with Cancel. The engine's stall
//! watchdog reports through the same sink, so a stalled job is said as a
//! change of state.
//!
//! A frontend's loop is: `let screen = runtime.carry_out(&mut app,
//! app.handle(event));` then draw and say `screen`, then feed
//! `runtime.poll()` back to `app.handle`. [`Runtime::run_until_idle`] does
//! the same and waits for the workers, for tests and one-shot commands.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_archive::{Allowances, ArchiveError, ArchiveProvider, JobSecrets};
use diroxide_config::{ArchiveSettings, Paths};
use diroxide_core::{
    ConflictAction, ConflictAnswer, Effect, ErrorAction, JobId, JobState, Message, Place,
    RequestId, Secret, SecretRequest, Side,
};
use diroxide_ops::{
    Conflict, ConnectionAnswer, ConnectionQuestion, Engine, EngineOptions, ItemError, JobControl,
    JobNotice, JobProgress, JobSink, Journal, Ledger, Plan,
};
use diroxide_vfs::{
    CancelToken, OpContext, ProgressSink, ProgressUpdate, Registry, SecretSource, WatchHandle,
    limits,
};

mod archive_jobs;
// Optional components (W4d).
mod components;
// Plugins and script commands (W4d).
mod plugins;
// Hooks through the plugin hosts (lane A2, R25 commit 81).
mod hooks;
// The transfer queue (lane A2, R25 commit 79).
mod queue;

use crate::app::{App, AppEvent};
use crate::request::{AskId, Question, Reply, Request, Spawn, SpawnKind};

/// Called when an event is ready, from whatever thread made it: the
/// frontend wakes its loop (W1d-2 wires it to the terminal's event wait).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Opens a file with the system's program for it, never through a shell
/// (the `opener` crate in the real runtime; a recorder in tests).
pub trait Opener: Send + Sync {
    /// Opens the file; the error is for the log, never shown (spike four
    /// found the crates' error text unreadable).
    fn open(&self, path: &Path) -> Result<(), String>;

    /// Opens a web or mail address with the system's browser or mail
    /// program, with no shell. The runtime has checked it first: only
    /// `https://`, `http://` and `mailto:` addresses, whole and plain,
    /// arrive here. An opener for files only refuses.
    fn open_address(&self, address: &str) -> Result<(), String> {
        let _ = address;
        Err("this opener opens files only".to_owned())
    }
}

/// The system opener, through the `opener` crate: ShellExecute on
/// Windows, `xdg-open` or `open` elsewhere, with no shell string.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, path: &Path) -> Result<(), String> {
        opener::open(path).map_err(|e| e.to_string())
    }

    fn open_address(&self, address: &str) -> Result<(), String> {
        opener::open(address).map_err(|e| e.to_string())
    }
}

/// A program to start: its path or name, its arguments, its working
/// folder and its extra environment. Built from a [`Spawn`]; never a shell
/// string made from names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The program: a name looked up on the path, or a full path.
    pub program: OsString,
    /// Its arguments, each one argument.
    pub args: Vec<OsString>,
    /// The working folder.
    pub folder: PathBuf,
    /// Extra environment variables.
    pub env: Vec<(String, OsString)>,
}

/// Starts a program with the terminal handed over and waits for it.
pub trait Launcher: Send + Sync {
    /// Runs the program to its end; the exit code when there is one.
    fn run(&self, program: &Program) -> Result<Option<i32>, String>;

    /// The user's shell and the arguments that make it run one command
    /// line: `$SHELL -c`, else on Windows PowerShell 7, then 5.1, then
    /// cmd, each found on the path rather than assumed (spike four found
    /// no PowerShell 7 on the owner's machine).
    fn shell(&self) -> (OsString, Vec<OsString>);

    /// The editor for F4 when the `editor` setting is empty: `$VISUAL`,
    /// `$EDITOR`, then the platform's. The runtime checks the setting
    /// first (the owner's answer 74).
    fn editor(&self) -> OsString;
}

/// Runs programs on this machine with the standard library.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLauncher;

fn on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}

impl Launcher for SystemLauncher {
    fn run(&self, program: &Program) -> Result<Option<i32>, String> {
        let mut command = std::process::Command::new(&program.program);
        command.args(&program.args).current_dir(&program.folder);
        for (name, value) in &program.env {
            command.env(name, value);
        }
        command
            .status()
            .map(|status| status.code())
            .map_err(|e| e.to_string())
    }

    fn shell(&self) -> (OsString, Vec<OsString>) {
        if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
            return (shell, vec![OsString::from("-c")]);
        }
        if cfg!(windows) {
            let command = || vec![OsString::from("-NoLogo"), OsString::from("-Command")];
            if let Some(pwsh) = on_path("pwsh.exe") {
                return (pwsh.into_os_string(), command());
            }
            if let Some(powershell) = on_path("powershell.exe") {
                return (powershell.into_os_string(), command());
            }
            let cmd = std::env::var_os("ComSpec").unwrap_or_else(|| OsString::from("cmd.exe"));
            return (cmd, vec![OsString::from("/C")]);
        }
        (OsString::from("/bin/sh"), vec![OsString::from("-c")])
    }

    fn editor(&self) -> OsString {
        for name in ["VISUAL", "EDITOR"] {
            if let Some(editor) = std::env::var_os(name).filter(|e| !e.is_empty()) {
                return editor;
            }
        }
        if cfg!(windows) {
            OsString::from("notepad.exe")
        } else if cfg!(target_os = "macos") {
            OsString::from("nano")
        } else {
            OsString::from("vi")
        }
    }
}

/// What the frontend does around a program that takes the terminal: leave
/// the alternate screen and raw mode before, come back after (W1d-2 wires
/// it; without one the program simply runs).
pub type TerminalHook = Box<dyn FnMut(Handover) + Send>;

/// The two moments of a handover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handover {
    /// Give the terminal to the program: restore the screen as it was.
    Suspend,
    /// Take it back: raw mode, the alternate screen, a full redraw.
    Resume,
}

/// The questions workers wait on: the job that asks, and where the answer
/// goes.
type Asks = HashMap<AskId, (Option<JobId>, Sender<Reply>)>;

/// What the workers share with the runtime.
struct Shared {
    sender: Mutex<Sender<AppEvent>>,
    /// Work in flight that will end in an event: listings, plans, opens,
    /// and jobs not waiting for an answer.
    busy: AtomicUsize,
    waker: Mutex<Option<Waker>>,
    /// The questions workers wait on, with the job that asks.
    asks: Mutex<Asks>,
    next_ask: AtomicU64,
}

impl Shared {
    fn send(&self, event: AppEvent) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let _ = sender.send(event);
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    fn begin(&self) {
        self.busy.fetch_add(1, Ordering::SeqCst);
    }

    /// One worker fewer; never below none, so a plugin's question asked
    /// outside any job cannot wrap the count (W4d).
    fn end(&self) {
        let _ = self
            .busy
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// Counts a worker idle while its plugin waits for the owner, when one
    /// is counted at all; returns whether it did.
    fn lower(&self) -> bool {
        self.busy
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }

    /// Asks a question and waits for the answer, counting the worker as
    /// idle meanwhile; a cancel while waiting answers `on_cancel`.
    fn ask(&self, question: Question, cancel: &CancelToken, on_cancel: Reply) -> Reply {
        let ask = AskId::new(self.next_ask.fetch_add(1, Ordering::SeqCst) + 1);
        let (reply_to, replies) = mpsc::channel();
        self.asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ask, (question.job(), reply_to));
        self.send(AppEvent::Asks { ask, question });
        self.end();
        loop {
            match replies.recv_timeout(Duration::from_millis(50)) {
                // The answerer counted this worker busy again.
                Ok(reply) => return reply,
                Err(RecvTimeoutError::Timeout) if !cancel.is_cancelled() => {}
                Err(_) => {
                    let still_asked = self
                        .asks
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&ask)
                        .is_some();
                    if still_asked {
                        self.begin();
                        return on_cancel;
                    }
                    // An answer is on its way, already counted busy.
                    return replies.recv().unwrap_or(on_cancel);
                }
            }
        }
    }

    /// Answers a waiting question once: the worker is counted busy again
    /// before it wakes, so the runtime never looks idle in between.
    fn answer(&self, ask: AskId, reply: Reply) {
        let waiting = self
            .asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&ask);
        if let Some((_, reply_to)) = waiting {
            self.begin();
            if reply_to.send(reply).is_err() {
                self.end();
            }
        }
    }
}

/// Forwards a running job's progress and questions to the app.
struct ForwardSink {
    job: JobId,
    shared: Arc<Shared>,
    engine: Engine,
    cancel: CancelToken,
    last: Mutex<(Option<JobState>, Option<Instant>)>,
}

impl ForwardSink {
    /// Changes of state always go; other updates at most ten a second, the
    /// app thins them further by its own setting.
    fn worth_sending(&self, progress: &JobProgress) -> bool {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let changed = last.0 != Some(progress.state);
        let due = last
            .1
            .is_none_or(|at| now.duration_since(at) >= Duration::from_millis(100));
        if changed || due {
            *last = (Some(progress.state), Some(now));
            true
        } else {
            false
        }
    }
}

impl JobSink for ForwardSink {
    fn progress(&self, progress: &JobProgress) {
        if self.worth_sending(progress) {
            self.shared.send(AppEvent::JobProgress {
                job: self.job,
                progress: progress.clone(),
            });
        }
    }

    fn conflict(&self, conflict: &Conflict) -> ConflictAnswer {
        let question = Question::Conflict {
            job: self.job,
            conflict: Box::new(conflict.clone()),
            replace_undoable: self.engine.replace_can_be_undone(conflict),
        };
        let cancel = Reply::Conflict(ConflictAnswer::once(ConflictAction::Cancel));
        match self.shared.ask(question, &self.cancel, cancel) {
            Reply::Conflict(answer) => answer,
            _ => ConflictAnswer::once(ConflictAction::Cancel),
        }
    }

    fn error(&self, error: &ItemError) -> ErrorAction {
        let question = Question::Error {
            job: self.job,
            error: Box::new(error.clone()),
        };
        match self
            .shared
            .ask(question, &self.cancel, Reply::Error(ErrorAction::Cancel))
        {
            Reply::Error(action) => action,
            _ => ErrorAction::Cancel,
        }
    }

    /// The one question for a lost host (R25, 2.4), through the app; a
    /// cancel while it waits answers Keep waiting, and the gate then sees
    /// the job's own cancel.
    fn connection(&self, question: &ConnectionQuestion) -> ConnectionAnswer {
        let asked = Question::Connection {
            job: self.job,
            question: question.clone(),
        };
        let keep = Reply::Connection(ConnectionAnswer::KeepWaiting);
        match self.shared.ask(asked, &self.cancel, keep) {
            Reply::Connection(answer) => answer,
            _ => ConnectionAnswer::KeepWaiting,
        }
    }

    /// A line the job says once: a file resumed, or one that could not be.
    fn notice(&self, notice: &JobNotice) {
        self.shared.send(AppEvent::JobNotice {
            job: self.job,
            lines: notice.lines(),
        });
    }
}

/// Asks the app for a secret on behalf of a listing or a job (ADR-0009).
struct ForwardSecrets {
    job: Option<JobId>,
    shared: Arc<Shared>,
}

impl SecretSource for ForwardSecrets {
    fn ask(&self, request: &SecretRequest, cancel: &CancelToken) -> Option<Secret> {
        let question = Question::Secret {
            job: self.job,
            request: request.clone(),
        };
        match self.shared.ask(question, cancel, Reply::Secret(None)) {
            Reply::Secret(secret) => secret,
            _ => None,
        }
    }
}

/// Forwards a listing's count, so a slow folder says "Reading folder,
/// 12,000 so far".
struct ListingProgress {
    request: RequestId,
    shared: Arc<Shared>,
}

impl ProgressSink for ListingProgress {
    fn report(&self, update: ProgressUpdate) {
        self.shared.send(AppEvent::ListProgress {
            request: self.request,
            so_far: update.items_done,
        });
    }
}

/// A job that runs, or waits to.
struct RunningJob {
    control: JobControl,
}

/// Carries out I/O effects and requests, and queues their results.
pub struct Runtime {
    registry: Arc<Registry>,
    /// The registry without the plugins' providers, which each new
    /// catalog adds again (W4d).
    base: Arc<Registry>,
    /// Plugins and script commands (W4d).
    plugins: plugins::PluginRuntime,
    /// Optional components (W4d).
    components: components::ComponentRuntime,
    /// The `components_source` setting, from the app's settings.
    components_source: String,
    engine: Engine,
    shared: Arc<Shared>,
    receiver: Receiver<AppEvent>,
    queue: VecDeque<AppEvent>,
    jobs: Arc<Mutex<HashMap<JobId, RunningJob>>>,
    watches: [Option<WatchHandle>; 2],
    paths: Option<Paths>,
    opener: Arc<dyn Opener>,
    launcher: Arc<dyn Launcher>,
    hook: Option<TerminalHook>,
    /// The `editor` setting, when it names a program.
    editor: Option<OsString>,
    /// The archive provider in the registry, for the pack, unpack and test
    /// runners and for forgetting a locked archive.
    archives: Option<Arc<ArchiveProvider>>,
    /// The `[archives]` settings the provider was made from, when this
    /// runtime made it: a changed table makes it again.
    archive_settings: Option<ArchiveSettings>,
    /// The passwords of archive jobs between their plan and their run.
    archive_secrets: archive_jobs::JobSecretsMap,
    /// The arrival watch of Watch this folder (W3t): dropping it stops it.
    arrivals: Option<diroxide_ops::tracker::WatchRun>,
    /// The running search and its control (W3a).
    search: Option<(u64, diroxide_search::SearchControl)>,
    /// The transfer queue: when a confirmed plan starts (lane A2).
    scheduler: queue::Scheduler,
    /// The ledger of partial files, kept across engines made again: in
    /// the state folder once the paths are known (R25, 2.5).
    ledger: Arc<Ledger>,
    /// The `[jobs]` settings last applied (lane A2).
    job_settings: Option<diroxide_config::JobSettings>,
    /// The hosts each plugin said it lost, for a loss that turns final.
    lost: HashMap<String, Vec<diroxide_ops::HostKey>>,
    /// The engine's hook runner, through the plugin hosts (lane A2).
    hooks: Arc<hooks::AppHooks>,
    /// The active panel's place, marks and focus after the last batch,
    /// for the navigation hooks.
    nav_seen: Option<(Place, usize, Option<String>)>,
    /// The listings on their way, each with its cancel, so a listing the
    /// app no longer waits for stops (lane A2, R25 commit 77).
    listings: Arc<Mutex<HashMap<RequestId, CancelToken>>>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("registry", &self.registry)
            .field("busy", &self.shared.busy.load(Ordering::SeqCst))
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl Runtime {
    /// A runtime over a registry, with the engine's default options, the
    /// system opener and launcher, and no state folder (nothing is
    /// written to disk but what jobs do).
    pub fn new(registry: Arc<Registry>) -> Self {
        let (sender, receiver) = mpsc::channel();
        // A registry that cannot open archives gets the archive provider,
        // at the `[archives]` defaults until the app's settings arrive
        // (see `carry_out`).
        let (registry, archives, archive_settings) = if crate::archives::serves_archives(&registry)
        {
            (registry, None, None)
        } else {
            let settings = ArchiveSettings::default();
            let (registry, provider) =
                crate::archives::install(&registry, crate::archives::provider(&settings, None));
            (registry, Some(provider), Some(settings))
        };
        let shared = Arc::new(Shared {
            sender: Mutex::new(sender),
            busy: AtomicUsize::new(0),
            waker: Mutex::new(None),
            asks: Mutex::new(HashMap::new()),
            next_ask: AtomicU64::new(0),
        });
        let ledger = Arc::new(Ledger::in_memory());
        let hooks = Arc::new(hooks::AppHooks::default());
        let runner: Arc<dyn diroxide_ops::HookRunner> = hooks.clone();
        let engine = Engine::new(Arc::clone(&registry))
            .with_ledger(Arc::clone(&ledger))
            .with_hooks(runner);
        // A host's gate speaks through the app: back, keep waiting, the
        // jobs cancelled or stopped (R25, 2.4). The gates outlive every
        // engine made again, and so does this listener.
        let speaker = Arc::clone(&shared);
        engine
            .gates()
            .on_event(move |event| speaker.send(AppEvent::Gate(event.clone())));
        Self {
            engine,
            base: Arc::clone(&registry),
            plugins: plugins::PluginRuntime::default(),
            components: components::ComponentRuntime::default(),
            components_source: String::new(),
            registry,
            shared,
            receiver,
            queue: VecDeque::new(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            watches: [None, None],
            paths: None,
            opener: Arc::new(SystemOpener),
            launcher: Arc::new(SystemLauncher),
            hook: None,
            editor: None,
            archives,
            archive_settings,
            archive_secrets: Arc::new(Mutex::new(HashMap::new())),
            arrivals: None,
            search: None,
            scheduler: queue::Scheduler::default(),
            ledger,
            job_settings: None,
            lost: HashMap::new(),
            hooks,
            nav_seen: None,
            listings: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The same runtime with an archive provider the caller installed in
    /// the registry itself (tests, a frontend with its own limits): the
    /// runners use it, and the settings no longer remake it.
    pub fn with_archive_provider(mut self, provider: Arc<ArchiveProvider>) -> Self {
        self.archives = Some(provider);
        self.archive_settings = None;
        self
    }

    /// The archive provider, when the registry has one this runtime knows.
    pub fn archives(&self) -> Option<&Arc<ArchiveProvider>> {
        self.archives.as_ref()
    }

    /// Makes the archive provider again from the `[archives]` table when
    /// it changed, the spool folder from the state folder; a provider the
    /// caller installed is left as it is. The cached indexes go with the
    /// old provider.
    pub fn apply_archive_settings(&mut self, settings: &ArchiveSettings) {
        let Some(current) = &self.archive_settings else {
            return;
        };
        if current == settings {
            return;
        }
        let provider = crate::archives::provider(settings, self.paths.as_ref());
        let (registry, provider) = crate::archives::install(&self.base, provider);
        self.base = registry;
        self.archives = Some(provider);
        self.archive_settings = Some(settings.clone());
        self.rebuild_registry();
    }

    /// Makes the registry again from the base and the enabled plugins'
    /// providers, with an engine over it (W4d).
    fn rebuild_registry(&mut self) {
        let registry = Arc::new(self.plugins.registry_with_providers(&self.base));
        self.engine = self.engine_over(Arc::clone(&registry));
        self.registry = registry;
    }

    /// An engine over a registry that keeps what every engine shares: the
    /// options, the journal, the connection gates and their waiting jobs,
    /// the ledger of partial files and the global speed limit (lane A2).
    fn engine_over(&self, registry: Arc<Registry>) -> Engine {
        let mut engine = Engine::new(registry)
            .with_options(self.engine.options().clone())
            .with_gates(self.engine.gates().clone())
            .with_ledger(Arc::clone(&self.ledger))
            .with_speed_limit(self.engine.speed_limiter())
            .with_hooks(self.hooks.clone());
        if let Some(journal) = self.engine.journal() {
            engine = engine.with_journal(journal.clone());
        }
        engine
    }

    /// The `[jobs]` settings, applied when they change: the queue's
    /// limits, the global speed limit and its scope, and the retries
    /// (R25 commit 80).
    pub fn apply_job_settings(&mut self, settings: &diroxide_config::JobSettings) {
        if self.job_settings.as_ref() == Some(settings) {
            return;
        }
        self.job_settings = Some(*settings);
        let limits = diroxide_ops::QueueLimits {
            per_host: usize::try_from(settings.per_host).unwrap_or(2).max(1),
            remote: usize::try_from(settings.remote).unwrap_or(4).max(1),
            ..diroxide_ops::QueueLimits::default()
        };
        let granted = self.scheduler.set_limits(limits);
        self.carry_granted(granted);
        let limiter = self.engine.speed_limit();
        let limit = diroxide_ops::SpeedLimit::bytes_per_second(
            settings.speed_limit_kb.saturating_mul(1000),
        );
        if limiter.limit() != limit {
            limiter.set(limit);
        }
        limiter.set_scope(match settings.speed_limit_scope {
            diroxide_config::SpeedLimitScope::Remote => diroxide_ops::LimitScope::RemoteJobs,
            diroxide_config::SpeedLimitScope::All => diroxide_ops::LimitScope::AllJobs,
        });
        let mut options = self.engine.options().clone();
        retries_from(&mut options, settings);
        if &options != self.engine.options() {
            self.engine = self.engine.clone().with_options(options);
        }
    }

    /// The same runtime with plugins off for the session (`--no-plugins`):
    /// nothing is started, and the Plugins menu says so (W4d).
    pub fn without_plugins(mut self) -> Self {
        self.plugins.turn_off();
        self
    }

    /// The same runtime starting plugins through another launcher: tests
    /// start the in-process fakes (W4d).
    pub fn with_plugin_launcher(
        mut self,
        launcher: Arc<dyn diroxide_plugins::host::Launcher>,
    ) -> Self {
        self.plugins.set_launcher(launcher);
        self
    }

    /// The same runtime fetching components from the owner's source
    /// alone, never a public address: tests, which never reach the
    /// network (W4d).
    pub fn with_components_offline(mut self) -> Self {
        self.components.set_offline();
        self
    }

    /// The same runtime fetching components through another fetcher
    /// (tests record what is fetched) (W4d).
    pub fn with_component_fetcher(
        mut self,
        fetcher: Arc<dyn diroxide_components::Fetcher>,
    ) -> Self {
        self.components.set_fetcher(fetcher);
        self
    }

    /// Stops every running plugin politely, within two seconds; a
    /// frontend calls it at quit (dropping the runtime kills what is
    /// left without waiting).
    pub fn stop_plugins(&mut self) {
        self.plugins.shutdown();
    }

    /// Sets the editor from the `editor` setting: a program's name on the
    /// path or its full path; empty leaves the choice to the launcher
    /// (`$VISUAL`, `$EDITOR`, then the platform's). A changed setting
    /// reaches here through [`Request::SaveSetting`] too.
    pub fn set_editor(&mut self, editor: &str) {
        let editor = editor.trim();
        self.editor = (!editor.is_empty()).then(|| OsString::from(editor));
    }

    /// A runtime with only the local provider.
    pub fn with_local() -> Self {
        Self::new(Arc::new(Registry::with_local()))
    }

    /// The same runtime with the config and state folders: the undo
    /// journal is written in the state folder, and the session, the
    /// prompts' histories and changed settings are saved there.
    pub fn with_paths(mut self, paths: Paths) -> Self {
        // The ledger of partial files lives in the state folder, so the
        // next start finds what a crash left; one that cannot be read is
        // said and kept, and this session's stays in memory.
        match Ledger::open(&paths.state_dir) {
            Ok(ledger) => {
                self.ledger = Arc::new(ledger);
                self.engine = self.engine.with_ledger(Arc::clone(&self.ledger));
            }
            Err(error) => self.queue.push_back(AppEvent::Problem(Message::could_not(
                "read",
                "the list of partial files",
                &error.to_string(),
                "",
            ))),
        }
        self.engine = self
            .engine
            .with_journal(Journal::new(paths.state_dir.clone()));
        if let (Some(provider), Some(settings)) = (&self.archives, &self.archive_settings) {
            provider.set_spool_dir(Some(settings.spool_dir(&paths)));
        }
        self.paths = Some(paths);
        self
    }

    /// The same runtime with the engine's options (built from the
    /// settings by [`engine_options`]).
    pub fn with_engine_options(mut self, options: EngineOptions) -> Self {
        self.engine = self.engine.with_options(options);
        self
    }

    /// The same runtime with another opener (a recorder in tests).
    pub fn with_opener(mut self, opener: Arc<dyn Opener>) -> Self {
        self.opener = opener;
        self
    }

    /// The same runtime with another launcher (a recorder in tests).
    pub fn with_launcher(mut self, launcher: Arc<dyn Launcher>) -> Self {
        self.launcher = launcher;
        self
    }

    /// Sets the waker the workers call when an event is ready. App-side
    /// hook for W1d-2: wake the terminal's event wait.
    pub fn set_waker(&mut self, waker: Waker) {
        *self
            .shared
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// Sets what the frontend does around a program that takes the
    /// terminal. App-side hook for W1d-2: leave and re-enter the screen.
    pub fn set_terminal_hook(&mut self, hook: TerminalHook) {
        self.hook = Some(hook);
    }

    /// The registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Whether workers are still busy (waiting questions do not count).
    pub fn is_busy(&self) -> bool {
        self.shared.busy.load(Ordering::SeqCst) > 0
    }

    fn worker(&self, work: impl FnOnce(&Arc<Shared>) + Send + 'static) {
        let shared = Arc::clone(&self.shared);
        shared.begin();
        thread::spawn(move || {
            work(&shared);
            shared.end();
        });
    }

    /// Carries out an effect if it is I/O; returns whether it was. Screen
    /// and speech effects return `false` and are left to the frontend.
    /// [`Effect::AppRequests`] is the runtime's too, but the requests are
    /// the app's, so only [`Runtime::carry_out`] takes them; here it is
    /// consumed and the requests wait for that call.
    pub fn execute(&mut self, effect: &Effect) -> bool {
        match effect {
            Effect::AppRequests => true,
            Effect::List { request, place } => {
                self.list(*request, place.clone());
                true
            }
            Effect::StartJob(request) => {
                let engine = self.engine.clone();
                let request = request.clone();
                let jobs = Arc::clone(&self.jobs);
                let control = JobControl::new();
                lock(&jobs).insert(
                    request.id,
                    RunningJob {
                        control: control.clone(),
                    },
                );
                self.worker(move |shared| {
                    let sink = forward_sink(request.id, shared, &engine, &control);
                    let event = match engine.plan_with(&request, &control, &sink) {
                        Ok(plan) => {
                            AppEvent::JobFinished(Box::new(engine.run(&plan, &control, &sink)))
                        }
                        Err(error) => {
                            AppEvent::Problem(Message::from_error(request.kind.verb(), "", &error))
                        }
                    };
                    lock(&jobs).remove(&request.id);
                    shared.send(event);
                });
                true
            }
            Effect::CancelJob(job) => {
                self.cancel(*job);
                true
            }
            // A plugin's command runs through the app, which checks its
            // conditions and consent and makes it a job (W4d).
            Effect::RunPlugin(call) => {
                self.queue
                    .push_back(AppEvent::Command(call.command.clone()));
                true
            }
            Effect::OpenExternally(place) => {
                self.open(place.clone());
                true
            }
            Effect::OpenAddress(address) => {
                self.open_address(address.clone());
                true
            }
            Effect::SetClipboard(_) => {
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "copy",
                    "to the system clipboard",
                    "alpha 1 keeps its own file clipboard",
                    "",
                )));
                true
            }
            _ => false,
        }
    }

    fn list(&self, request: RequestId, place: Place) {
        let registry = Arc::clone(&self.registry);
        if place.kind() == diroxide_core::PlaceKind::Archive
            && let Some(archives) = self.archives.clone()
        {
            self.worker(move |shared| list_archive(shared, &registry, &archives, request, place));
            return;
        }
        let cancel = CancelToken::new();
        lock(&self.listings).insert(request, cancel.clone());
        let listings = Arc::clone(&self.listings);
        self.worker(move |shared| {
            let progress = Arc::new(ListingProgress {
                request,
                shared: Arc::clone(shared),
            });
            let secrets = Arc::new(ForwardSecrets {
                job: None,
                shared: Arc::clone(shared),
            });
            let ctx = OpContext::new()
                .with_progress(progress)
                .with_secrets(secrets)
                .with_cancel(cancel.clone());
            let result = registry.resolve(&place).and_then(|provider| {
                let entries = match provider.as_streaming() {
                    Some(streaming) => list_streamed(shared, streaming, request, &place, &ctx)?,
                    None => provider.list(&place, &ctx)?,
                };
                shared.send(AppEvent::PlaceFacts {
                    place: place.clone(),
                    capabilities: provider.capabilities(&place),
                });
                Ok(entries)
            });
            lock(&listings).remove(&request);
            shared.send(AppEvent::Listed {
                request,
                place,
                result,
            });
        });
    }

    fn open(&self, place: Place) {
        let opener = Arc::clone(&self.opener);
        self.worker(move |shared| {
            let name = place
                .name()
                .map_or_else(|| place.to_string(), str::to_owned);
            let name = diroxide_core::names::display_name(&name).into_owned();
            if let Some(refusal) = limits::opener_refusal(&place) {
                shared.send(AppEvent::Problem(Message::error(format!(
                    "{refusal}: {name}."
                ))));
                return;
            }
            let Some(path) = place.local_path() else {
                shared.send(AppEvent::Problem(Message::could_not(
                    "open",
                    &name,
                    "only files on this computer open in other programs",
                    "",
                )));
                return;
            };
            if opener.open(Path::new(path)).is_err() {
                shared.send(AppEvent::Problem(Message::could_not(
                    "open",
                    &name,
                    "the system has no program for it, or the program failed",
                    "Try Edit, or Run a command",
                )));
            }
        });
    }

    /// Opens a web or mail address on a worker, after checking it again
    /// by the safe opener rule: anything but a whole, plain `https://`,
    /// `http://` or `mailto:` address is refused and said.
    fn open_address(&mut self, address: String) {
        let shown = diroxide_core::names::spell_out(&address).into_owned();
        let what = match crate::quickview::LinkTarget::of(&address) {
            crate::quickview::LinkTarget::Web(_) => "the browser",
            crate::quickview::LinkTarget::Mail(_) => "the mail program",
            crate::quickview::LinkTarget::Refused(_) => {
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "open",
                    &shown,
                    "only web and mail addresses open from here",
                    "",
                )));
                return;
            }
        };
        let opener = Arc::clone(&self.opener);
        self.worker(move |shared| {
            if opener.open_address(&address).is_err() {
                shared.send(AppEvent::Problem(Message::could_not(
                    "open",
                    &shown,
                    &format!("{what} did not start"),
                    "Copy the address and open it yourself",
                )));
            }
        });
    }

    fn cancel(&mut self, job: JobId) {
        // A job that waits its turn leaves the queue at once: nothing was
        // done, and its report says it was cancelled (R25, 2.2).
        let (waiting, granted) = self.scheduler.cancel(job);
        if let Some(plan) = waiting {
            lock(&self.jobs).remove(&job);
            let mut report = diroxide_ops::JobReport::new(job, plan.kind);
            report.cancelled = true;
            self.queue
                .push_back(AppEvent::JobFinished(Box::new(report)));
        }
        self.carry_granted(granted);
        if let Some(running) = lock(&self.jobs).get(&job) {
            running.control.cancel.cancel();
        }
        self.plugins.cancel(job);
        self.components.cancel(job);
        let asks: Vec<AskId> = lock(&self.shared.asks)
            .iter()
            .filter(|(_, (asker, _))| *asker == Some(job))
            .map(|(ask, _)| *ask)
            .collect();
        for ask in asks {
            self.shared.answer(ask, Reply::Error(ErrorAction::Cancel));
        }
    }

    /// Carries out one request from the app.
    pub fn request(&mut self, request: Request) {
        match request {
            Request::Components(request) => {
                let shared = Arc::clone(&self.shared);
                let worker = |work: Work| {
                    spawn_worker(&shared, work);
                };
                self.components.request(
                    *request,
                    self.paths.as_ref(),
                    &self.components_source,
                    &worker,
                );
            }
            Request::Plugins(request) => {
                let shared = Arc::clone(&self.shared);
                let worker = |work: Work| {
                    spawn_worker(&shared, work);
                };
                self.plugins
                    .request(*request, &self.shared, self.paths.as_ref(), &worker);
            }
            Request::Plan(job) => {
                let engine = self.engine.clone();
                let control = JobControl::new();
                self.worker(move |shared| {
                    let sink = forward_sink(job.id, shared, &engine, &control);
                    let result = engine.plan_with(&job, &control, &sink).map(Box::new);
                    shared.send(AppEvent::Planned {
                        job: job.id,
                        result,
                    });
                });
            }
            Request::PlanUndo { job, record } => {
                let engine = self.engine.clone();
                self.worker(move |shared| {
                    let result = engine.plan_undo(&record, job).map(Box::new);
                    shared.send(AppEvent::Planned { job, result });
                });
            }
            Request::Run(plan) => self.run(*plan),
            Request::Answer { ask, reply } => {
                // A job that asked gave its slot back; its answer waits for
                // the slot again (R25, 2.2).
                let job = lock(&self.shared.asks).get(&ask).and_then(|(job, _)| *job);
                let granted = self.scheduler.answer(job, ask, reply);
                self.carry_granted(granted);
            }
            Request::Queue(request) => self.queue_request(request),
            Request::StopListing(listing) => {
                if let Some(cancel) = lock(&self.listings).get(&listing) {
                    cancel.cancel();
                }
            }
            Request::Watch { side, place } => self.watch(side, place),
            Request::Places => {
                let sources = self.plugins.site_sources();
                self.worker(move |shared| {
                    shared.send(AppEvent::Places(diroxide_vfs::local_places()));
                    // The plugins' sites, asked without signing in.
                    let sites = plugins::sites_of(&sources);
                    shared.send(AppEvent::Sites(sites));
                });
            }
            Request::SaveSession(session) => {
                if let Some(paths) = &self.paths
                    && let Err(error) = diroxide_config::save_session(paths, &session)
                {
                    self.queue.push_back(AppEvent::Problem(Message::from_error(
                        "save",
                        "the session",
                        &error,
                    )));
                }
            }
            Request::SaveHistory { key, entries } => {
                if let Some(paths) = &self.paths
                    && let Err(error) = crate::state::save_history(paths, &key, &entries)
                {
                    self.queue.push_back(AppEvent::Problem(Message::could_not(
                        "save",
                        &format!("the {key} history"),
                        &error,
                        "",
                    )));
                }
            }
            Request::SaveSetting { key, value } => {
                if key == "editor" {
                    let editor = value
                        .as_ref()
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    self.set_editor(&editor);
                }
                if let Some(paths) = &self.paths {
                    let saved = match value {
                        Some(value) => diroxide_config::set_setting(paths, &key, value),
                        None => diroxide_config::reset_setting(paths, &key),
                    };
                    if let Err(error) = saved {
                        self.queue.push_back(AppEvent::Problem(Message::from_error(
                            "save", &key, &error,
                        )));
                    }
                }
            }
            Request::EditKeymap(edit) => {
                if let Some(paths) = &self.paths
                    && let Err(said) = crate::keymap::file::save(paths, &edit)
                {
                    self.queue
                        .push_back(AppEvent::Problem(Message::error(said)));
                }
            }
            Request::Settle(job) => {
                if let Some(paths) = &self.paths {
                    let _ = Journal::new(paths.state_dir.clone()).settle(job);
                }
            }
            Request::Spawn(spawn) => self.spawn(&spawn),
            Request::PlanUnpack { job, request } => {
                let Some(archives) = self.archives.clone() else {
                    self.queue.push_back(AppEvent::ArchivePlanned {
                        job,
                        result: Err(archive_jobs::no_archives("unpack")),
                    });
                    return;
                };
                let registry = Arc::clone(&self.registry);
                let map = Arc::clone(&self.archive_secrets);
                self.worker(move |shared| {
                    archive_jobs::plan_unpack_job(
                        shared, &registry, &archives, &map, job, &request,
                    );
                });
            }
            Request::PlanPack { job, request } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    archive_jobs::plan_pack_job(shared, &registry, job, &request);
                });
            }
            Request::RunArchive(plan) => {
                let Some(archives) = self.archives.clone() else {
                    self.queue
                        .push_back(AppEvent::Problem(archive_jobs::no_archives("run")));
                    return;
                };
                let engine = self.engine.clone();
                let map = Arc::clone(&self.archive_secrets);
                let job = plan.job();
                let control = self.register(job);
                let jobs = Arc::clone(&self.jobs);
                self.worker(move |shared| {
                    archive_jobs::run_archive_job(
                        shared, &engine, &archives, &map, &control, &plan,
                    );
                    lock(&jobs).remove(&job);
                });
            }
            Request::TestArchive { job, archive } => {
                let Some(archives) = self.archives.clone() else {
                    self.queue
                        .push_back(AppEvent::Problem(archive_jobs::no_archives("test")));
                    return;
                };
                let engine = self.engine.clone();
                let control = self.register(job);
                let jobs = Arc::clone(&self.jobs);
                self.worker(move |shared| {
                    archive_jobs::test_archive_job(
                        shared, &engine, &archives, &control, job, &archive,
                    );
                    lock(&jobs).remove(&job);
                });
            }
            Request::OpenViewer {
                request,
                place,
                size,
                cap,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let result = open_viewer_source(shared, &registry, &place, size, cap);
                    shared.send(AppEvent::ViewerSource {
                        request,
                        place,
                        result,
                    });
                });
            }
            Request::ArchiveEntries(place) => {
                let Some(archives) = self.archives.clone() else {
                    return;
                };
                self.worker(move |shared| {
                    let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
                        job: None,
                        shared: Arc::clone(shared),
                    });
                    let secrets = JobSecrets::new(source);
                    let opened = secrets.with_tries(&OpContext::new(), |ctx| {
                        archives.open_archive(&place, ctx, Allowances::default())
                    });
                    secrets.forget();
                    let line = match opened {
                        Ok(opened) => {
                            let summary = opened.index().summary();
                            Ok(format!(
                                "Entries: {}",
                                diroxide_core::format::count(
                                    summary.members + summary.folders,
                                    "item",
                                    "items"
                                )
                            ))
                        }
                        Err(error) => Err(error.message("count", "the entries")),
                    };
                    shared.send(AppEvent::ArchiveEntries { place, line });
                });
            }
            Request::DropArchiveJob(job) => {
                // Dropping the job's secrets wipes its password.
                lock(&self.archive_secrets).remove(&job);
            }
            Request::ForgetArchive(container) => {
                if let Some(archives) = &self.archives {
                    archives.forget(&container);
                }
            }
            Request::Sweep { folders } => {
                let registry = Arc::clone(&self.registry);
                let spool = match (&self.paths, &self.archive_settings) {
                    (Some(paths), Some(settings)) => Some(settings.spool_dir(paths)),
                    (Some(paths), None) => Some(paths.spool_dir()),
                    _ => None,
                };
                // Nothing is said: what is swept is the program's own.
                thread::spawn(move || {
                    crate::archives::sweep(&registry, spool.as_deref(), &folders);
                });
            }
            Request::SaveLabels(store) => {
                if let Some(paths) = &self.paths
                    && let Err(error) = crate::labels::save(paths, &store)
                {
                    self.queue.push_back(AppEvent::Problem(Message::could_not(
                        "save",
                        "the labels",
                        &error,
                        "",
                    )));
                }
            }
            Request::SaveBookmarks(bookmarks) => {
                if let Some(paths) = &self.paths
                    && let Err(error) = crate::bookmarks::save(paths, &bookmarks)
                {
                    self.queue.push_back(AppEvent::Problem(Message::could_not(
                        "save",
                        "the bookmarks",
                        &error,
                        "",
                    )));
                }
            }
            Request::SaveViews(views) => {
                if let Some(paths) = &self.paths
                    && let Err(error) = crate::views::save(paths, &views)
                {
                    self.queue.push_back(AppEvent::Problem(Message::could_not(
                        "save",
                        "the folder views",
                        &error,
                        "",
                    )));
                }
            }
            Request::CheckPlaces { request, places } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let ctx = OpContext::new();
                    let mut missing = Vec::new();
                    let mut unreadable = 0;
                    for place in places {
                        match registry
                            .resolve(&place)
                            .and_then(|provider| provider.stat(&place, &ctx))
                        {
                            Ok(_) => {}
                            Err(diroxide_vfs::VfsError::NotFound) => missing.push(place),
                            Err(_) => unreadable += 1,
                        }
                    }
                    shared.send(AppEvent::PlacesChecked {
                        request,
                        missing,
                        unreadable,
                    });
                });
            }
            Request::WatchArrivals { watch, place, pace } => {
                self.watch_arrivals(watch, place, pace);
            }
            Request::StopArrivals => self.arrivals = None,
            Request::Search {
                search,
                query,
                clock,
            } => self.start_search(search, *query, clock),
            Request::StatPlaces { search, places } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let ctx = OpContext::new();
                    let mut found = Vec::new();
                    let mut missing = Vec::new();
                    let mut unreadable = 0;
                    for place in places {
                        match registry
                            .resolve(&place)
                            .and_then(|provider| provider.stat(&place, &ctx))
                        {
                            Ok(entry) => found.push(diroxide_search::SearchHit::new(place, entry)),
                            Err(diroxide_vfs::VfsError::NotFound) => missing.push(place),
                            Err(_) => unreadable += 1,
                        }
                    }
                    shared.send(AppEvent::LabeledFound {
                        search,
                        found,
                        missing,
                        unreadable,
                    });
                });
            }
            Request::Compare {
                request,
                before,
                after,
                options,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let result = compare_two(shared, &registry, request, &before, &after, &options);
                    shared.send(AppEvent::Compared { request, result });
                });
            }
            Request::SaveSearch(saved) => {
                if let Some(paths) = &self.paths {
                    let folder = paths.state_dir.join(diroxide_search::SAVED_FOLDER);
                    if let Err(error) = saved.save(&folder) {
                        let line = format!("Could not save the search: {error}");
                        self.queue
                            .push_back(AppEvent::Problem(Message::error(line)));
                    }
                }
            }
            Request::ListSaved { request } => {
                let folder = self
                    .paths
                    .as_ref()
                    .map(|p| p.state_dir.join(diroxide_search::SAVED_FOLDER));
                self.worker(move |shared| {
                    let found = folder
                        .map(|folder| {
                            diroxide_search::Saved::list(&folder)
                                .into_iter()
                                .map(|(path, result)| (path, result.map_err(|e| e.to_string())))
                                .collect()
                        })
                        .unwrap_or_default();
                    shared.send(AppEvent::SavedListed { request, found });
                });
            }
            Request::DeleteSaved(path) => {
                // Only a file inside the saved searches' own folder.
                let inside = self.paths.as_ref().is_some_and(|p| {
                    path.parent() == Some(p.state_dir.join(diroxide_search::SAVED_FOLDER).as_path())
                        && path.extension().is_some_and(|x| x == "toml")
                });
                if inside && let Err(error) = std::fs::remove_file(&path) {
                    let line = format!("Could not delete the saved search: {error}");
                    self.queue
                        .push_back(AppEvent::Problem(Message::error(line)));
                }
            }
            Request::SpaceList {
                request,
                place,
                grouping,
                clock,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    use diroxide_search::{Grouping, SearchControl};
                    let control = SearchControl::new();
                    let leaf = crate::panel::place_leaf(&place);
                    let result = match grouping {
                        Grouping::Kind => {
                            diroxide_search::space_by_kind(&place, &registry, &control)
                        }
                        Grouping::Age => diroxide_search::space_by_age(
                            &place,
                            &registry,
                            clock.0.as_ref(),
                            &control,
                        ),
                        _ => diroxide_search::space_list(&place, &registry, &control),
                    }
                    .map_err(|error| Message::from_error("count", &leaf, &error));
                    shared.send(AppEvent::SpaceListed { request, result });
                });
            }
            Request::Largest {
                search,
                place,
                limit,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let control = diroxide_search::SearchControl::new();
                    let leaf = crate::panel::place_leaf(&place);
                    let result = diroxide_search::largest_below(&place, &registry, &control, limit)
                        .map_err(|error| Message::from_error("search", &leaf, &error));
                    shared.send(AppEvent::LargestFound { search, result });
                });
            }
            Request::DirSizes { request, places } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let control = diroxide_search::SearchControl::new();
                    let sizes = diroxide_search::size_every_folder(
                        &places,
                        &registry,
                        &control,
                        &mut |_, _| {},
                    )
                    .into_iter()
                    .map(|(place, result)| {
                        let leaf = crate::panel::place_leaf(&place);
                        let result =
                            result.map_err(|error| Message::from_error("size", &leaf, &error));
                        (place, result)
                    })
                    .collect();
                    shared.send(AppEvent::DirSized { request, sizes });
                });
            }
            Request::Newest {
                search,
                scope,
                before,
                clock,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let control = diroxide_search::SearchControl::new();
                    let leaf = scope
                        .places
                        .first()
                        .map(crate::panel::place_leaf)
                        .unwrap_or_default();
                    let result = diroxide_search::newest_files(
                        &scope,
                        &registry,
                        clock.0.as_ref(),
                        &control,
                        before,
                    )
                    .map_err(|error| Message::from_error("search", &leaf, &error));
                    shared.send(AppEvent::NewestFound { search, result });
                });
            }
            Request::StopSearch => {
                if let Some((_, control)) = &self.search {
                    control.cancel();
                }
            }
            Request::PauseSearch(pause) => {
                if let Some((_, control)) = &self.search {
                    if pause {
                        control.pause();
                    } else {
                        control.resume();
                    }
                }
            }
            Request::CheckStale { search, row, hit } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let freshness =
                        diroxide_search::stale_check(&hit, &registry, &OpContext::new());
                    shared.send(AppEvent::StaleChecked {
                        search,
                        row,
                        freshness,
                    });
                });
            }
            Request::FilterContents {
                request,
                side: _,
                place,
                filter,
                hidden,
                clock,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let result = filter_contents(&registry, place, *filter, hidden, &clock);
                    shared.send(AppEvent::FilterContents { request, result });
                });
            }
        }
    }

    /// Starts the arrival watch of Watch this folder on its own thread,
    /// replacing the one before (W3t). Its notes come back tagged with
    /// `watch`; it never counts as busy, so waiting for the workers never
    /// waits for it.
    fn watch_arrivals(&mut self, watch: u64, place: Place, pace: Duration) {
        use diroxide_ops::tracker::{WatchNote, WatchOptions, spawn_watch};
        self.arrivals = None;
        let failed = |line: String| AppEvent::Arrivals {
            watch,
            note: WatchNote::Failed { line },
        };
        let folder = crate::panel::place_leaf(&place);
        let provider = match self.registry.resolve(&place) {
            Ok(provider) => provider,
            Err(error) => {
                let line = Message::from_error("watch", &folder, &error).text;
                self.queue.push_back(failed(line));
                return;
            }
        };
        let shared = Arc::clone(&self.shared);
        let sink = Box::new(move |note| shared.send(AppEvent::Arrivals { watch, note }));
        let options = WatchOptions {
            quiet: None,
            pace: Some(pace),
        };
        let clock = Arc::new(diroxide_core::SystemClock::utc());
        match spawn_watch(provider, place, options, clock, sink) {
            Ok(run) => self.arrivals = Some(run),
            Err(error) => {
                let line = Message::from_error("watch", &folder, &error).text;
                self.queue.push_back(failed(line));
            }
        }
    }

    /// Starts a search on a worker, stopping the one before. Its hits go
    /// to the app in batches (at most a tenth of a second apart), its
    /// progress at most four times a second; an online-only file asks the
    /// app once when the scope says to read such files.
    fn start_search(
        &mut self,
        search: u64,
        query: diroxide_search::SearchQuery,
        clock: crate::request::SharedClock,
    ) {
        use diroxide_search::{OnlineOnly, SearchControl, run_search};
        if let Some((_, old)) = self.search.take() {
            old.cancel();
        }
        let mut control = SearchControl::new();
        if query.scope.online_only == OnlineOnly::Read {
            let asker = OnlineAsker {
                shared: Arc::clone(&self.shared),
                cancel: control.cancel_token().clone(),
                answer: Mutex::new(None),
            };
            control = control.with_guard(Arc::new(asker));
        }
        self.search = Some((search, control.clone()));
        let registry = Arc::clone(&self.registry);
        self.worker(move |shared| {
            let place = query
                .scope
                .places
                .first()
                .map(crate::panel::place_leaf)
                .unwrap_or_default();
            let mut sink = BatchSink {
                shared: Arc::clone(shared),
                search,
                batch: Vec::new(),
                sent: Instant::now(),
                progress_sent: None,
            };
            let result = run_search(&query, &registry, clock.0.as_ref(), &control, &mut sink);
            sink.flush();
            let result = result.map_err(|error| Message::from_error("search", &place, &error));
            shared.send(AppEvent::SearchDone { search, result });
        });
    }

    /// Notes a running job's control, so Cancel reaches it.
    fn register(&self, job: JobId) -> JobControl {
        let control = JobControl::new();
        lock(&self.jobs).insert(
            job,
            RunningJob {
                control: control.clone(),
            },
        );
        control
    }

    /// A confirmed plan goes to the queue, which starts it now or holds it
    /// until its turn; its control is noted at once, so Cancel, Pause and
    /// a speed limit reach a waiting job too.
    fn run(&mut self, plan: Plan) {
        if diroxide_ops::JobClass::of_plan(&plan) == diroxide_ops::JobClass::Unqueued {
            self.start_plan(plan);
            return;
        }
        self.register(plan.job);
        let granted = self.scheduler.submit(plan);
        self.carry_granted(granted);
    }

    /// Starts what the queue granted, delivers the answers it let go, and
    /// tells the app.
    fn carry_granted(&mut self, granted: queue::Granted) {
        for note in granted.notes {
            self.queue.push_back(AppEvent::Queue(Box::new(note)));
        }
        for (ask, reply) in granted.answers {
            self.shared.answer(ask, reply);
        }
        for plan in granted.start {
            self.start_plan(plan);
        }
    }

    /// One of the Jobs list's actions, or a queue setting.
    fn queue_request(&mut self, request: crate::jobs::QueueRequest) {
        use crate::jobs::QueueRequest as Q;
        let control = |runtime: &Self, job: JobId| {
            lock(&runtime.jobs)
                .get(&job)
                .map(|running| running.control.clone())
        };
        let granted = match request {
            Q::RunNow(job) => self.scheduler.run_now(job),
            Q::MoveUp(job) => self.scheduler.shift(job, queue::Shift::Up).1,
            Q::MoveDown(job) => self.scheduler.shift(job, queue::Shift::Down).1,
            Q::MoveToFront(job) => self.scheduler.shift(job, queue::Shift::Front).1,
            Q::MoveToEnd(job) => self.scheduler.shift(job, queue::Shift::End).1,
            Q::Pause(job) | Q::Resume(job) => {
                if let Some(control) = control(self, job) {
                    if matches!(request, Q::Pause(_)) {
                        control.pause();
                    } else {
                        control.resume();
                    }
                }
                queue::Granted::default()
            }
            Q::PauseQueue => self.scheduler.set_paused(true),
            Q::ResumeQueue => self.scheduler.set_paused(false),
            Q::PauseAll | Q::ResumeAll => {
                let pause = request == Q::PauseAll;
                for running in lock(&self.jobs).values() {
                    if pause {
                        running.control.pause();
                    } else {
                        running.control.resume();
                    }
                }
                self.scheduler.set_paused(pause)
            }
            Q::SpeedLimit {
                job: Some(job),
                limit,
            } => {
                if let Some(control) = control(self, job) {
                    control.set_speed_limit(limit);
                }
                queue::Granted::default()
            }
            Q::SpeedLimit { job: None, limit } => {
                self.engine.speed_limit().set(limit);
                queue::Granted::default()
            }
            Q::LimitScope(scope) => {
                self.engine.speed_limit().set_scope(scope);
                queue::Granted::default()
            }
            Q::Limits(limits) => self.scheduler.set_limits(limits),
            Q::ErrorPolicy { job, policy } => {
                self.scheduler.set_error_policy(job, policy);
                queue::Granted::default()
            }
        };
        self.carry_granted(granted);
    }

    /// Runs a plan on a worker now, with the control noted for it.
    fn start_plan(&mut self, plan: Plan) {
        let engine = self.engine.clone();
        let jobs = Arc::clone(&self.jobs);
        let control = lock(&jobs)
            .entry(plan.job)
            .or_insert_with(|| RunningJob {
                control: JobControl::new(),
            })
            .control
            .clone();
        self.worker(move |shared| {
            let sink = forward_sink(plan.job, shared, &engine, &control);
            let report = engine.run(&plan, &control, &sink);
            lock(&jobs).remove(&plan.job);
            shared.send(AppEvent::JobFinished(Box::new(report)));
        });
    }

    fn watch(&mut self, side: Side, place: Option<Place>) {
        self.watches[side.index()] = None;
        let Some(place) = place else {
            return;
        };
        let provider = match self.registry.resolve(&place) {
            Ok(provider) => provider,
            Err(_) => {
                self.queue.push_back(AppEvent::NotWatched { side, place });
                return;
            }
        };
        let shared = Arc::clone(&self.shared);
        let watched = place.clone();
        let sink = Box::new(move |event| {
            shared.send(AppEvent::Watched {
                side,
                place: watched.clone(),
                event,
            });
        });
        match provider.watch(&place, sink) {
            Ok(Some(handle)) => self.watches[side.index()] = Some(handle),
            Ok(None) | Err(_) => self.queue.push_back(AppEvent::NotWatched { side, place }),
        }
    }

    fn spawn(&mut self, spawn: &Spawn) {
        let what = match &spawn.kind {
            SpawnKind::Shell => "Shell".to_owned(),
            SpawnKind::CommandLine(_) => "Command".to_owned(),
            SpawnKind::Editor(file) | SpawnKind::EditorAt { file, .. } => {
                file.name().unwrap_or("The editor").to_owned()
            }
            SpawnKind::Program { name, .. } => name.clone(),
        };
        let editing = match &spawn.kind {
            SpawnKind::Editor(file) | SpawnKind::EditorAt { file, .. } => Some(file),
            _ => None,
        };
        if let Some(refusal) = limits::program_refusal(&spawn.folder, editing) {
            self.queue
                .push_back(AppEvent::Problem(Message::error(format!("{refusal}."))));
            return;
        }
        let program = match self.program_for(spawn) {
            Ok(program) => program,
            Err(problem) => {
                self.queue.push_back(AppEvent::Problem(problem));
                return;
            }
        };
        if let Some(hook) = self.hook.as_mut() {
            hook(Handover::Suspend);
        }
        let result = self.launcher.run(&program);
        if let Some(hook) = self.hook.as_mut() {
            hook(Handover::Resume);
        }
        self.queue.push_back(AppEvent::ProgramEnded {
            what,
            folder: spawn.folder.clone(),
            result,
        });
    }

    fn program_for(&self, spawn: &Spawn) -> Result<Program, Message> {
        let folder = spawn
            .folder
            .local_path()
            .map(PathBuf::from)
            .ok_or_else(|| {
                Message::could_not(
                    "start a program",
                    "here",
                    "programs start only in folders on this computer",
                    "Go to a local folder first",
                )
            })?;
        let level = std::env::var("DIRO_LEVEL")
            .ok()
            .and_then(|level| level.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        let mut env = vec![
            ("DIRO_LEVEL".to_owned(), OsString::from(level.to_string())),
            (
                "DIRO_ACTIVE".to_owned(),
                OsString::from(spawn.folder.to_string()),
            ),
            (
                "DIRO_OTHER".to_owned(),
                OsString::from(spawn.other.to_string()),
            ),
        ];
        if let Some(cursor) = &spawn.cursor {
            env.push(("DIRO_CURSOR".to_owned(), OsString::from(cursor.to_string())));
        }
        if let Some(list) = self.selection_file(&spawn.selection) {
            env.push(("DIRO_SELECTION_FILE".to_owned(), list.into_os_string()));
        }
        let (program, args) = match &spawn.kind {
            SpawnKind::Shell => (self.launcher.shell().0, Vec::new()),
            SpawnKind::CommandLine(line) => {
                let (shell, mut args) = self.launcher.shell();
                args.push(OsString::from(line));
                (shell, args)
            }
            SpawnKind::EditorAt { file, line } => {
                let path = file.local_path().ok_or_else(|| {
                    Message::could_not(
                        "edit",
                        file.name().unwrap_or("it"),
                        "only files on this computer can be edited",
                        "",
                    )
                })?;
                let editor = self
                    .editor
                    .clone()
                    .unwrap_or_else(|| self.launcher.editor());
                let mut args = Vec::new();
                if takes_line(&editor) {
                    args.push(OsString::from(format!("+{line}")));
                }
                args.push(OsString::from(path));
                (editor, args)
            }
            SpawnKind::Editor(file) => {
                let path = file.local_path().ok_or_else(|| {
                    Message::could_not(
                        "edit",
                        file.name().unwrap_or("it"),
                        "only files on this computer can be edited",
                        "",
                    )
                })?;
                let editor = self
                    .editor
                    .clone()
                    .unwrap_or_else(|| self.launcher.editor());
                (editor, vec![OsString::from(path)])
            }
            SpawnKind::Program { program, args, .. } => {
                (program.clone().into_os_string(), args.clone())
            }
        };
        Ok(Program {
            program,
            args,
            folder,
            env,
        })
    }

    /// Writes the selection as a NUL-separated list in the state folder,
    /// for `DIRO_SELECTION_FILE`; `None` without a state folder.
    fn selection_file(&self, selection: &[Place]) -> Option<PathBuf> {
        let paths = self.paths.as_ref()?;
        let folder = paths.state_dir.join("run");
        std::fs::create_dir_all(&folder).ok()?;
        let path = folder.join("selection.nul");
        let mut file = std::fs::File::create(&path).ok()?;
        for place in selection {
            file.write_all(place.to_string().as_bytes()).ok()?;
            file.write_all(&[0]).ok()?;
        }
        Some(path)
    }

    /// Takes the results that are ready, without waiting. A plugin
    /// catalog is adopted as it passes: the hosts of the enabled plugins
    /// and their providers (W4d).
    pub fn poll(&mut self) -> Vec<AppEvent> {
        while let Ok(event) = self.receiver.try_recv() {
            self.queue.push_back(event);
        }
        let events: Vec<AppEvent> = self.queue.drain(..).collect();
        for event in &events {
            match event {
                AppEvent::Plugin(plugin) => {
                    if let crate::plugins::PluginEvent::Found(catalog) = plugin.as_ref()
                        && self.plugins.adopt(
                            catalog,
                            &self.shared,
                            self.paths.as_ref(),
                            &self.base,
                        )
                    {
                        self.rebuild_registry();
                    }
                    if let crate::plugins::PluginEvent::Found(_) = plugin.as_ref() {
                        self.hooks.set(self.plugins.hooked());
                    }
                    if let crate::plugins::PluginEvent::Host(host) = plugin.as_ref() {
                        self.host_event(host);
                    }
                }
                // The queue hears what its jobs do as it passes: a job's
                // end frees its slots, a question gives them back.
                AppEvent::JobFinished(report) => {
                    let granted = self.scheduler.finished(report.job);
                    self.carry_granted(granted);
                }
                AppEvent::Asks { question, .. } => {
                    if let Some(job) = question.job()
                        && question.gives_slot_back()
                    {
                        let granted = self.scheduler.asking(job);
                        self.carry_granted(granted);
                    }
                }
                _ => {}
            }
        }
        events
    }

    /// What a plugin host's event means for the connection gates: a lost
    /// provider holds its host's jobs waiting (`provider/lost`), and a
    /// plugin turned off for good stops them with its reason, so a job
    /// stops only when the loss is final.
    fn host_event(&mut self, event: &diroxide_plugins::host::HostEvent) {
        use diroxide_plugins::host::HostEvent;
        match event {
            HostEvent::ProviderLost {
                plugin,
                authority,
                message,
                for_good,
            } => {
                let place = Place::parse(authority)
                    .or_else(|_| Place::parse(&format!("{authority}/")))
                    .ok();
                if let Some(host) = place.as_ref().and_then(diroxide_ops::HostKey::of) {
                    if *for_good {
                        // The plugin will not reconnect: the waiting jobs
                        // stop with its reason rather than wait.
                        self.engine.gates().lost_for_good(&host, &message.text);
                        if let Some(hosts) = self.lost.get_mut(plugin) {
                            hosts.retain(|h| h != &host);
                        }
                    } else {
                        self.engine.gates().lost(&host);
                        let hosts = self.lost.entry(plugin.clone()).or_default();
                        if !hosts.contains(&host) {
                            hosts.push(host);
                        }
                    }
                }
            }
            HostEvent::TurnedOff { plugin, message } => {
                for host in self.lost.remove(plugin).unwrap_or_default() {
                    self.engine.gates().lost_for_good(&host, &message.text);
                }
            }
            _ => {}
        }
    }

    /// Waits up to `timeout` for a result, then takes what is ready.
    pub fn wait(&mut self, timeout: Duration) -> Vec<AppEvent> {
        if self.queue.is_empty()
            && let Ok(event) = self.receiver.recv_timeout(timeout)
        {
            self.queue.push_back(event);
        }
        self.poll()
    }

    /// Takes what is ready, waiting while workers are busy; empty only
    /// when nothing is ready and no worker will send more on its own.
    fn settle(&mut self) -> Vec<AppEvent> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let events = self.poll();
            if !events.is_empty() {
                return events;
            }
            if !self.is_busy() || Instant::now() > deadline {
                return self.poll();
            }
            let waited = self.wait(Duration::from_millis(10));
            if !waited.is_empty() {
                return waited;
            }
        }
    }

    /// Carries out the effects that are I/O and the app's queued
    /// requests, and returns the screen and speech effects in order. A
    /// frontend calls this with every batch of effects the app returns.
    /// The requests are taken where [`Effect::AppRequests`] stands in the
    /// batch, and any still waiting at its end (from an app call that does
    /// not mark them) after it.
    pub fn carry_out(&mut self, app: &mut App, effects: Vec<Effect>) -> Vec<Effect> {
        self.apply_archive_settings(&app.settings().archives);
        self.apply_job_settings(&app.settings().jobs);
        self.plugins.set_settings(&app.settings().plugins);
        if self.components_source != app.settings().components_source {
            self.components_source = app.settings().components_source.clone();
        }
        let mut screen = Vec::new();
        for effect in effects {
            if effect == Effect::AppRequests {
                for request in app.take_requests() {
                    self.request(request);
                }
            } else if !self.execute(&effect) {
                screen.push(effect);
            }
        }
        for request in app.take_requests() {
            self.request(request);
        }
        self.observe_navigation(app);
        screen
    }

    /// Notes where the active panel is, its marks and its focus, and tells
    /// the navigation hooks what changed, debounced (R25 commit 81): a new
    /// folder is `enter_folder`, other changes `select`. Nothing is noted
    /// while no plugin hooks them.
    fn observe_navigation(&mut self, app: &App) {
        if !self.hooks.wants_nav() {
            self.nav_seen = None;
            return;
        }
        let panel = app.active_panel();
        if panel.is_loading() {
            return;
        }
        let place = panel.place().clone();
        let count = panel.list().selected_count();
        let cursor = panel.list().focused().map(|row| row.id.clone());
        let now = (place.clone(), count, cursor.clone());
        let event = match &self.nav_seen {
            Some(seen) if *seen == now => None,
            Some((seen, ..)) if *seen == place => Some(hooks::NavEvent::Select {
                place,
                count: count as u64,
                cursor,
            }),
            _ => Some(hooks::NavEvent::EnterFolder { place }),
        };
        self.nav_seen = Some(now);
        if let Some(event) = event {
            self.hooks.navigated(event);
        }
    }

    /// Carries out effects, feeds their results back to the app, and
    /// repeats until nothing is left and no worker is busy; returns the
    /// screen and speech effects in order. A job waiting for an answer is
    /// not busy, so this returns with its question open. For tests, `diro`
    /// one-shot commands and the server; an interactive frontend polls
    /// instead.
    pub fn run_until_idle(&mut self, app: &mut App, effects: Vec<Effect>) -> Vec<Effect> {
        let mut for_frontend = Vec::new();
        let mut pending = effects;
        loop {
            for_frontend.extend(self.carry_out(app, pending));
            let events = self.settle();
            if events.is_empty() {
                return for_frontend;
            }
            pending = events
                .into_iter()
                .flat_map(|event| app.handle(event))
                .collect();
        }
    }
}

/// Compares two files for the app, the progress sent at most four times a
/// second; a cancel says "Not compared".
fn compare_two(
    shared: &Arc<Shared>,
    registry: &Registry,
    request: RequestId,
    before: &Place,
    after: &Place,
    options: &diroxide_diff::FileOptions,
) -> Result<Box<diroxide_diff::FileComparison>, Message> {
    use diroxide_diff::{DiffError, FileSide, Verdict, compare_files, verdict_line};
    let name = |place: &Place| crate::panel::place_leaf(place);
    let before_provider = registry
        .resolve(before)
        .map_err(|e| Message::from_error("compare", &name(before), &e))?;
    let after_provider = registry
        .resolve(after)
        .map_err(|e| Message::from_error("compare", &name(after), &e))?;
    let sink = Arc::new(CompareSink {
        shared: Arc::clone(shared),
        request,
        sent: Mutex::new(None),
    });
    let ctx = OpContext::new().with_progress(sink);
    let result = compare_files(
        FileSide {
            provider: before_provider.as_ref(),
            place: before,
        },
        FileSide {
            provider: after_provider.as_ref(),
            place: after,
        },
        options,
        &ctx,
    );
    match result {
        Ok(result) => Ok(Box::new(result)),
        Err(DiffError::Cancelled) => Err(Message::info(verdict_line(
            Verdict::NotCompared,
            &name(after),
            &name(before),
        ))),
        Err(error) => Err(Message::from_error("compare", &name(after), &error)),
    }
}

/// Sends a compare's progress to the app, thinned.
struct CompareSink {
    shared: Arc<Shared>,
    request: RequestId,
    sent: Mutex<Option<Instant>>,
}

impl ProgressSink for CompareSink {
    fn report(&self, update: ProgressUpdate) {
        let mut sent = lock(&self.sent);
        if sent.is_some_and(|at| at.elapsed() < Duration::from_millis(250)) {
            return;
        }
        *sent = Some(Instant::now());
        self.shared.send(AppEvent::CompareProgress {
            request: self.request,
            done: update.bytes_done,
            total: update.bytes_total,
        });
    }
}

/// A search's sink: hits in batches, progress thinned, both sent to the
/// app tagged with the search's number.
struct BatchSink {
    shared: Arc<Shared>,
    search: u64,
    batch: Vec<diroxide_search::SearchHit>,
    sent: Instant,
    progress_sent: Option<Instant>,
}

impl BatchSink {
    const BATCH: usize = 256;
    const EVERY: Duration = Duration::from_millis(100);
    const PROGRESS_EVERY: Duration = Duration::from_millis(250);

    fn flush(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let hits = std::mem::take(&mut self.batch);
        self.sent = Instant::now();
        self.shared.send(AppEvent::SearchFound {
            search: self.search,
            hits,
        });
    }
}

impl diroxide_search::SearchSink for BatchSink {
    fn found(&mut self, hit: diroxide_search::SearchHit) -> std::ops::ControlFlow<()> {
        self.batch.push(hit);
        if self.batch.len() >= Self::BATCH || self.sent.elapsed() >= Self::EVERY {
            self.flush();
        }
        std::ops::ControlFlow::Continue(())
    }

    fn progress(&mut self, progress: &diroxide_search::SearchProgress) {
        if self.sent.elapsed() >= Self::EVERY {
            self.flush();
        }
        if self
            .progress_sent
            .is_some_and(|at| at.elapsed() < Self::PROGRESS_EVERY)
        {
            return;
        }
        self.progress_sent = Some(Instant::now());
        self.shared.send(AppEvent::SearchProgress {
            search: self.search,
            progress: progress.clone(),
        });
    }
}

/// Asks the app, once a search, whether online-only files may be read,
/// which downloads them; the answer holds for the rest of the search.
struct OnlineAsker {
    shared: Arc<Shared>,
    cancel: CancelToken,
    answer: Mutex<Option<bool>>,
}

impl diroxide_search::OnlineGuard for OnlineAsker {
    fn allow(&self, _place: &Place, entry: &diroxide_core::Entry) -> bool {
        let mut answer = lock(&self.answer);
        if let Some(yes) = *answer {
            return yes;
        }
        let question = Question::OnlineOnly {
            name: entry.name.clone(),
            size: entry.size,
        };
        let yes = matches!(
            self.shared
                .ask(question, &self.cancel, Reply::Confirm(false)),
            Reply::Confirm(true)
        );
        *answer = Some(yes);
        yes
    }
}

/// Runs a panel's content filter over its folder alone, depth 1, every
/// hit kept: the names that matched, and what could not be decided.
fn filter_contents(
    registry: &Registry,
    place: Place,
    filter: diroxide_core::filter::Filter,
    hidden: bool,
    clock: &crate::request::SharedClock,
) -> Result<crate::app::FilterContents, Message> {
    use diroxide_search::{FnSink, RowShape, Scope, SearchControl, SearchQuery, run_search};
    let folder = crate::panel::place_leaf(&place);
    let query = SearchQuery {
        filter: Some(filter),
        scope: Scope::new(place)
            .with_hidden(hidden)
            .with_max_depth(Some(1))
            .with_row_cap(None),
        rows: RowShape::PerFile,
    };
    let mut matched = Vec::new();
    let summary = run_search(
        &query,
        registry,
        clock.0.as_ref(),
        &SearchControl::new(),
        &mut FnSink(|hit: diroxide_search::SearchHit| matched.push(hit.entry.name)),
    )
    .map_err(|error| Message::from_error("filter", &folder, &error))?;
    Ok(crate::app::FilterContents {
        matched,
        unknown: summary.unknown,
    })
}

/// Whether an editor takes `+LINE` before the file: vi and its family,
/// nano, Emacs, Micro, Kakoune, joe, ne and mcedit.
fn takes_line(editor: &std::ffi::OsStr) -> bool {
    let name = Path::new(editor)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        name.as_str(),
        "vi" | "vim"
            | "nvim"
            | "gvim"
            | "view"
            | "nano"
            | "pico"
            | "emacs"
            | "emacsclient"
            | "micro"
            | "kak"
            | "joe"
            | "jstar"
            | "ne"
            | "mcedit"
            | "testeditor"
    )
}

/// Reads a file for the viewer: a seekable read where the provider has one
/// (a local file, a stored member, a spooled one), else a stream kept up to
/// `cap`; a locked member asks its password through the app, three tries.
fn open_viewer_source(
    shared: &Arc<Shared>,
    registry: &Registry,
    place: &Place,
    size: Option<u64>,
    cap: u64,
) -> Result<crate::viewer::ViewerSource, Message> {
    let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
        job: None,
        shared: Arc::clone(shared),
    });
    let secrets = JobSecrets::new(source);
    let ctx = OpContext::new();
    let opened = secrets.with_tries(&ctx, |ctx| {
        let provider = registry.resolve(place)?;
        match provider.open_seekable(place, ctx) {
            Ok(reader) => crate::viewer::ViewerSource::seekable(reader)
                .map_err(|e| ArchiveError::Vfs(diroxide_vfs::VfsError::from(e))),
            Err(diroxide_vfs::VfsError::WrongPassword) => {
                Err(ArchiveError::Vfs(diroxide_vfs::VfsError::WrongPassword))
            }
            Err(_) => {
                let reader = provider.read(place, ctx)?;
                Ok(crate::viewer::ViewerSource::stream(reader, size, cap))
            }
        }
    });
    opened.map_err(|error| {
        let name = place
            .name()
            .map(diroxide_core::names::display_name)
            .unwrap_or_default()
            .into_owned();
        error.message("view", &name)
    })
}

/// Lists a place inside an archive on a worker: the archive opened with
/// its password asked through the app (three tries, "Wrong password"
/// first from the second) and each limit's question asked once per kind,
/// then the folder listed with the cap's line; the facts go to the app
/// first, then the listing.
fn list_archive(
    shared: &Arc<Shared>,
    registry: &Registry,
    archives: &ArchiveProvider,
    request: RequestId,
    place: Place,
) {
    let progress = Arc::new(ListingProgress {
        request,
        shared: Arc::clone(shared),
    });
    let source: Arc<dyn SecretSource> = Arc::new(ForwardSecrets {
        job: None,
        shared: Arc::clone(shared),
    });
    let secrets = JobSecrets::new(source);
    let ctx = OpContext::new().with_progress(progress);
    let Some((container, _)) = place.archive() else {
        return;
    };
    let container = container.clone();
    let mut allow = Allowances::default();
    let outcome = loop {
        let attempt = secrets.with_tries(&ctx, |ctx| {
            let opened = archives.open_archive(&container, ctx, allow)?;
            let listing = archives.list_folder(&place, ctx, allow)?;
            Ok((opened, listing))
        });
        match attempt {
            Err(ArchiveError::Ask(question)) => {
                let asked = Question::Limit {
                    job: None,
                    question: question.clone(),
                };
                match shared.ask(asked, &ctx.cancel, Reply::Confirm(false)) {
                    Reply::Confirm(true) => allow = allow.with(question.kind),
                    _ => break Err(question.as_error()),
                }
            }
            other => break other.map_err(ArchiveError::into_vfs),
        }
    };
    let locked = secrets.asked() > 0;
    secrets.forget();
    let result = match outcome {
        Ok((opened, listing)) => {
            if let Ok(provider) = registry.resolve(&place) {
                shared.send(AppEvent::PlaceFacts {
                    place: place.clone(),
                    capabilities: provider.capabilities(&place),
                });
            }
            let mut notes = Vec::new();
            if listing.capped() {
                notes.push(diroxide_archive::capped_line());
            }
            let facts = crate::archives::ArchiveFacts {
                place: place.clone(),
                container: container.clone(),
                title: archives.title(&place),
                opened: opened.opened_line(),
                notes,
                locked,
            };
            shared.send(AppEvent::ArchiveOpened {
                request,
                facts: Box::new(facts),
            });
            Ok(listing.entries)
        }
        Err(error) => Err(error),
    };
    shared.send(AppEvent::Listed {
        request,
        place,
        result,
    });
}

/// A listing that comes in batches (R25 commit 77): each batch goes to
/// the app as it arrives, so the rows appear as the provider sends them;
/// the whole list ends the listing as any listing does. A stop asked by
/// the app keeps what came: the panel lists it, and says it stopped.
fn list_streamed(
    shared: &Arc<Shared>,
    streaming: &dyn diroxide_vfs::StreamingList,
    request: RequestId,
    place: &Place,
    ctx: &OpContext,
) -> Result<Vec<diroxide_core::Entry>, diroxide_vfs::VfsError> {
    let mut all = Vec::new();
    let outcome = streaming.list_batches(place, ctx, &mut |batch| {
        if ctx.cancel.is_cancelled() {
            return std::ops::ControlFlow::Break(());
        }
        all.extend(batch.iter().cloned());
        shared.send(AppEvent::ListBatch {
            request,
            place: place.clone(),
            entries: batch,
        });
        std::ops::ControlFlow::Continue(())
    });
    match outcome {
        Ok(()) => Ok(all),
        Err(diroxide_vfs::VfsError::Cancelled) if ctx.cancel.is_cancelled() => Ok(all),
        Err(error) => Err(error),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Work for a worker thread, with what the workers share.
type Work = Box<dyn FnOnce(&Arc<Shared>) + Send>;

/// Runs work on a worker thread, counted busy until it ends.
fn spawn_worker(shared: &Arc<Shared>, work: Work) {
    let shared = Arc::clone(shared);
    shared.begin();
    thread::spawn(move || {
        work(&shared);
        shared.end();
    });
}

fn forward_sink(
    job: JobId,
    shared: &Arc<Shared>,
    engine: &Engine,
    control: &JobControl,
) -> ForwardSink {
    ForwardSink {
        job,
        shared: Arc::clone(shared),
        engine: engine.clone(),
        cancel: control.cancel.clone(),
        last: Mutex::new((None, None)),
    }
}

/// The engine's options from the settings: replace keeps the old file in
/// the trash as the setting says, and the home, config and state folders
/// are protected from a recursive removal.
pub fn engine_options(
    settings: &diroxide_config::Settings,
    protected: Vec<Place>,
) -> EngineOptions {
    let mut options = EngineOptions::default();
    options.replace_keeps_old_in_trash = settings.replace_to_trash;
    options.protected = protected;
    retries_from(&mut options, &settings.jobs);
    options
}

/// The silent tries after a lost network, from the `[jobs]` settings: the
/// count and the first wait; each next wait is three times longer (R25,
/// 2.3).
fn retries_from(options: &mut EngineOptions, settings: &diroxide_config::JobSettings) {
    options.network_retries = u32::try_from(settings.network_retries).unwrap_or(3);
    options.network_retry_wait = Duration::from_secs(settings.network_retry_seconds.max(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{JobKind, JobRequest, Redraw};
    use diroxide_ops::{FakeProvider, FakeSwitches, Latency};

    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    fn fake_runtime(fake: &FakeProvider) -> Runtime {
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        Runtime::new(Arc::new(registry))
    }

    #[test]
    fn a_listing_runs_on_a_worker_and_comes_back_with_its_request() {
        let mut runtime = Runtime::with_local();
        let place = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(runtime.execute(&Effect::List {
            request: RequestId::new(7),
            place,
        }));
        assert!(!runtime.execute(&Effect::Redraw(Redraw::All)));
        let mut events = Vec::new();
        loop {
            let more = runtime.settle();
            if more.is_empty() {
                break;
            }
            events.extend(more);
        }
        assert!(events.iter().any(|e| matches!(
            e,
            AppEvent::Listed { request, result: Ok(entries), .. }
                if *request == RequestId::new(7) && !entries.is_empty()
        )));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AppEvent::PlaceFacts { .. }))
        );
        assert!(runtime.poll().is_empty());
    }

    #[test]
    fn a_slow_listing_does_not_hold_up_the_caller() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            latency: Latency {
                per_call: Duration::from_millis(400),
                ..Latency::default()
            },
            ..FakeSwitches::default()
        });
        fake.add_folder(&p("/slow"));
        let mut runtime = fake_runtime(&fake);
        let started = Instant::now();
        runtime.execute(&Effect::List {
            request: RequestId::new(1),
            place: p("/slow"),
        });
        assert!(started.elapsed() < Duration::from_millis(200));
        assert!(runtime.is_busy());
        assert!(runtime.poll().is_empty());
        // The place's facts may arrive in a batch of their own before the
        // listing under load, so wait for the listing itself rather than
        // for the first batch (the full-suite run on main flaked on this).
        next(&mut runtime, |event| {
            matches!(event, AppEvent::Listed { .. }).then_some(())
        });
    }

    /// Waits for the first event `pick` takes; fails when the runtime goes
    /// idle without one.
    fn next<T>(runtime: &mut Runtime, mut pick: impl FnMut(AppEvent) -> Option<T>) -> T {
        loop {
            let events = runtime.settle();
            assert!(!events.is_empty(), "the runtime went idle first");
            for event in events {
                if let Some(found) = pick(event) {
                    return found;
                }
            }
        }
    }

    fn planned(runtime: &mut Runtime) -> Plan {
        next(runtime, |event| match event {
            AppEvent::Planned { result, .. } => Some(*result.unwrap()),
            _ => None,
        })
    }

    fn conflict_tree() -> (FakeProvider, Runtime) {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/x.txt"), b"new");
        fake.add_file(&p("/b/x.txt"), b"old");
        let runtime = fake_runtime(&fake);
        (fake, runtime)
    }

    fn copy_request() -> JobRequest {
        JobRequest::new(JobId::new(3), JobKind::Copy, vec![p("/a/x.txt")]).to(p("/b"))
    }

    fn asked(runtime: &mut Runtime) -> AskId {
        next(runtime, |event| match event {
            AppEvent::Asks {
                ask,
                question: Question::Conflict { .. },
            } => Some(ask),
            AppEvent::JobFinished(report) => panic!("finished without asking: {report:?}"),
            AppEvent::Asks { question, .. } => panic!("asked another question: {question:?}"),
            _ => None,
        })
    }

    fn finished(runtime: &mut Runtime) -> diroxide_ops::JobReport {
        next(runtime, |event| match event {
            AppEvent::JobFinished(report) => Some(*report),
            _ => None,
        })
    }

    #[test]
    fn a_conflict_waits_for_its_answer_and_the_job_goes_on() {
        let (fake, mut runtime) = conflict_tree();
        runtime.request(Request::Plan(Box::new(copy_request())));
        let plan = planned(&mut runtime);
        runtime.request(Request::Run(Box::new(plan)));
        let ask = asked(&mut runtime);
        // The worker counts itself idle on its own thread just after it
        // sends the question, so under load the flag can come down a
        // moment after the question arrives (P3b, W3l's finding). Settle
        // instead of looking at once: settling waits while any worker is
        // busy and ends only when nothing more will come on its own, so a
        // waiting job counted busy would leave the flag up here. What
        // matters is the order and the counts: nothing more before the
        // answer, one question, one finish, the file replaced.
        let mut before_answer = Vec::new();
        loop {
            let more = runtime.settle();
            if more.is_empty() {
                break;
            }
            before_answer.extend(more);
        }
        assert!(!runtime.is_busy(), "a job waiting for an answer is idle");
        assert!(
            !before_answer
                .iter()
                .any(|e| matches!(e, AppEvent::Asks { .. } | AppEvent::JobFinished(_))),
            "nothing is asked or finished before the answer: {before_answer:?}"
        );
        assert_eq!(fake.contents(&p("/b/x.txt")).unwrap(), b"old");
        runtime.request(Request::Answer {
            ask,
            reply: Reply::Conflict(ConflictAnswer::once(ConflictAction::Replace)),
        });
        let mut after_answer = Vec::new();
        loop {
            let more = runtime.settle();
            if more.is_empty() {
                break;
            }
            after_answer.extend(more);
        }
        let finishes: Vec<&diroxide_ops::JobReport> = after_answer
            .iter()
            .filter_map(|e| match e {
                AppEvent::JobFinished(report) => Some(&**report),
                _ => None,
            })
            .collect();
        assert_eq!(finishes.len(), 1, "{after_answer:?}");
        assert_eq!(finishes[0].done.len(), 1, "{:?}", finishes[0]);
        assert!(
            !after_answer
                .iter()
                .any(|e| matches!(e, AppEvent::Asks { .. })),
            "one question only: {after_answer:?}"
        );
        assert!(!runtime.is_busy());
        assert_eq!(fake.contents(&p("/b/x.txt")).unwrap(), b"new");
    }

    #[test]
    fn a_cancel_while_a_question_is_open_answers_it_with_cancel() {
        let (fake, mut runtime) = conflict_tree();
        runtime.request(Request::Plan(Box::new(copy_request())));
        let plan = planned(&mut runtime);
        runtime.request(Request::Run(Box::new(plan)));
        let _ = asked(&mut runtime);
        runtime.execute(&Effect::CancelJob(JobId::new(3)));
        let report = finished(&mut runtime);
        assert!(report.cancelled || report.done.is_empty(), "{report:?}");
        assert_eq!(fake.contents(&p("/b/x.txt")).unwrap(), b"old");
    }

    #[derive(Default)]
    struct Recorder {
        programs: Mutex<Vec<Program>>,
        opened: Mutex<Vec<PathBuf>>,
        addresses: Mutex<Vec<String>>,
    }

    impl Launcher for Recorder {
        fn run(&self, program: &Program) -> Result<Option<i32>, String> {
            lock(&self.programs).push(program.clone());
            Ok(Some(0))
        }
        fn shell(&self) -> (OsString, Vec<OsString>) {
            (OsString::from("testshell"), vec![OsString::from("-c")])
        }
        fn editor(&self) -> OsString {
            OsString::from("testeditor")
        }
    }

    impl Opener for Recorder {
        fn open(&self, path: &Path) -> Result<(), String> {
            lock(&self.opened).push(path.to_owned());
            Ok(())
        }

        fn open_address(&self, address: &str) -> Result<(), String> {
            lock(&self.addresses).push(address.to_owned());
            Ok(())
        }
    }

    #[test]
    fn an_address_opens_only_when_it_is_a_web_or_mail_address() {
        let recorder = Arc::new(Recorder::default());
        let mut runtime = Runtime::with_local().with_opener(recorder.clone());
        for address in ["https://example.org/docs", "mailto:someone@example.org"] {
            runtime.execute(&Effect::OpenAddress(address.to_owned()));
            assert!(runtime.settle().is_empty());
        }
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "C:\\x.exe",
            "https://a b",
        ] {
            runtime.execute(&Effect::OpenAddress(refused.to_owned()));
            let events = runtime.settle();
            assert!(
                matches!(
                    events.as_slice(),
                    [AppEvent::Problem(m)] if m.text.contains("only web and mail addresses open from here")
                ),
                "{refused}: {events:?}"
            );
        }
        assert_eq!(
            *lock(&recorder.addresses),
            ["https://example.org/docs", "mailto:someone@example.org"]
        );
        assert!(lock(&recorder.opened).is_empty());
    }

    #[test]
    fn a_program_gets_its_context_in_the_environment_never_in_the_line() {
        let recorder = Arc::new(Recorder::default());
        let mut runtime = Runtime::with_local().with_launcher(recorder.clone());
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let odd = here.join("name with spaces; and $HOME").unwrap();
        runtime.request(Request::Spawn(Box::new(Spawn {
            kind: SpawnKind::CommandLine("echo hi".to_owned()),
            folder: here.clone(),
            other: here.clone(),
            cursor: Some(odd.clone()),
            selection: vec![odd],
        })));
        let events = runtime.poll();
        assert!(matches!(
            events.as_slice(),
            [AppEvent::ProgramEnded {
                result: Ok(Some(0)),
                ..
            }]
        ));
        let programs = lock(&recorder.programs);
        let program = &programs[0];
        assert_eq!(program.program, OsString::from("testshell"));
        assert_eq!(
            program.args,
            [OsString::from("-c"), OsString::from("echo hi")]
        );
        let names: Vec<&str> = program.env.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"DIRO_ACTIVE") && names.contains(&"DIRO_CURSOR"));
        assert!(names.contains(&"DIRO_LEVEL"));
    }

    #[test]
    fn the_editor_setting_comes_before_the_launchers_choice() {
        let recorder = Arc::new(Recorder::default());
        let mut runtime = Runtime::with_local().with_launcher(recorder.clone());
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let file = here.join("Cargo.toml").unwrap();
        let spawn = |runtime: &mut Runtime| {
            runtime.request(Request::Spawn(Box::new(Spawn {
                kind: SpawnKind::Editor(file.clone()),
                folder: here.clone(),
                other: here.clone(),
                cursor: None,
                selection: Vec::new(),
            })));
            runtime.poll();
        };
        spawn(&mut runtime);
        runtime.set_editor("  ");
        spawn(&mut runtime);
        runtime.set_editor("myeditor");
        spawn(&mut runtime);
        runtime.request(Request::SaveSetting {
            key: "editor".to_owned(),
            value: Some(toml::Value::String("othereditor".to_owned())),
        });
        spawn(&mut runtime);
        runtime.request(Request::SaveSetting {
            key: "editor".to_owned(),
            value: None,
        });
        spawn(&mut runtime);
        let used: Vec<OsString> = lock(&recorder.programs)
            .iter()
            .map(|p| p.program.clone())
            .collect();
        assert_eq!(
            used,
            [
                "testeditor",
                "testeditor",
                "myeditor",
                "othereditor",
                "testeditor"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn open_externally_goes_to_the_opener_and_a_deep_path_is_refused() {
        let recorder = Arc::new(Recorder::default());
        let mut runtime = Runtime::with_local().with_opener(recorder.clone());
        let file = Place::parse(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .join("Cargo.toml")
            .unwrap();
        runtime.execute(&Effect::OpenExternally(file));
        assert!(runtime.settle().is_empty());
        assert_eq!(lock(&recorder.opened).len(), 1);
        if cfg!(windows) {
            let deep = Place::parse(&format!("D:\\{}", "d".repeat(270))).unwrap();
            runtime.execute(&Effect::OpenExternally(deep));
            let events = runtime.settle();
            assert!(matches!(
                events.as_slice(),
                [AppEvent::Problem(m)] if m.text.starts_with("Cannot open a file this deep")
            ));
            assert_eq!(lock(&recorder.opened).len(), 1);
        }
    }

    #[test]
    fn a_place_that_cannot_be_watched_says_so() {
        let fake = FakeProvider::new();
        fake.add_folder(&p("/w"));
        let mut runtime = fake_runtime(&fake);
        runtime.request(Request::Watch {
            side: Side::Left,
            place: Some(p("/w")),
        });
        assert!(matches!(
            runtime.poll().as_slice(),
            [AppEvent::NotWatched {
                side: Side::Left,
                ..
            }]
        ));
    }
}
