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

use diroxide_config::Paths;
use diroxide_core::{
    ConflictAction, ConflictAnswer, Effect, ErrorAction, JobId, JobState, Message, Place,
    RequestId, Secret, SecretRequest, Side,
};
use diroxide_ops::{
    Conflict, Engine, EngineOptions, ItemError, JobControl, JobProgress, JobSink, Journal, Plan,
};
use diroxide_vfs::{
    CancelToken, OpContext, ProgressSink, ProgressUpdate, Registry, SecretSource, WatchHandle,
    limits,
};

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
}

/// The system opener, through the `opener` crate: ShellExecute on
/// Windows, `xdg-open` or `open` elsewhere, with no shell string.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, path: &Path) -> Result<(), String> {
        opener::open(path).map_err(|e| e.to_string())
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

    fn end(&self) {
        self.busy.fetch_sub(1, Ordering::SeqCst);
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
        Self {
            engine: Engine::new(Arc::clone(&registry)),
            registry,
            shared: Arc::new(Shared {
                sender: Mutex::new(sender),
                busy: AtomicUsize::new(0),
                waker: Mutex::new(None),
                asks: Mutex::new(HashMap::new()),
                next_ask: AtomicU64::new(0),
            }),
            receiver,
            queue: VecDeque::new(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            watches: [None, None],
            paths: None,
            opener: Arc::new(SystemOpener),
            launcher: Arc::new(SystemLauncher),
            hook: None,
            editor: None,
        }
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
        self.engine = self
            .engine
            .with_journal(Journal::new(paths.state_dir.clone()));
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
                        Ok(plan) => AppEvent::JobFinished(engine.run(&plan, &control, &sink)),
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
            Effect::RunPlugin(call) => {
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "run",
                    &call.plugin,
                    "plugins come with alpha.4",
                    "",
                )));
                true
            }
            Effect::OpenExternally(place) => {
                self.open(place.clone());
                true
            }
            Effect::SetClipboard(_) => {
                self.queue.push_back(AppEvent::Problem(Message::could_not(
                    "copy",
                    "to the system clipboard",
                    "alpha.1 keeps its own file clipboard",
                    "",
                )));
                true
            }
            _ => false,
        }
    }

    fn list(&self, request: RequestId, place: Place) {
        let registry = Arc::clone(&self.registry);
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
                .with_secrets(secrets);
            let result = registry.resolve(&place).and_then(|provider| {
                let entries = provider.list(&place, &ctx)?;
                shared.send(AppEvent::PlaceFacts {
                    place: place.clone(),
                    capabilities: provider.capabilities(&place),
                });
                Ok(entries)
            });
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

    fn cancel(&mut self, job: JobId) {
        if let Some(running) = lock(&self.jobs).get(&job) {
            running.control.cancel.cancel();
        }
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
            Request::Answer { ask, reply } => self.shared.answer(ask, reply),
            Request::Watch { side, place } => self.watch(side, place),
            Request::Places => self.worker(|shared| {
                shared.send(AppEvent::Places(diroxide_vfs::local_places()));
            }),
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
            Request::Settle(job) => {
                if let Some(paths) = &self.paths {
                    let _ = Journal::new(paths.state_dir.clone()).settle(job);
                }
            }
            Request::Spawn(spawn) => self.spawn(&spawn),
        }
    }

    fn run(&mut self, plan: Plan) {
        let engine = self.engine.clone();
        let control = JobControl::new();
        let jobs = Arc::clone(&self.jobs);
        lock(&jobs).insert(
            plan.job,
            RunningJob {
                control: control.clone(),
            },
        );
        self.worker(move |shared| {
            let sink = forward_sink(plan.job, shared, &engine, &control);
            let report = engine.run(&plan, &control, &sink);
            lock(&jobs).remove(&plan.job);
            shared.send(AppEvent::JobFinished(report));
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
            SpawnKind::Editor(file) => file.name().unwrap_or("The editor").to_owned(),
        };
        let editing = match &spawn.kind {
            SpawnKind::Editor(file) => Some(file),
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

    /// Takes the results that are ready, without waiting.
    pub fn poll(&mut self) -> Vec<AppEvent> {
        while let Ok(event) = self.receiver.try_recv() {
            self.queue.push_back(event);
        }
        self.queue.drain(..).collect()
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
        screen
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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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
    options
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
            AppEvent::JobFinished(report) => Some(report),
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
        assert!(!runtime.is_busy(), "a job waiting for an answer is idle");
        runtime.request(Request::Answer {
            ask,
            reply: Reply::Conflict(ConflictAnswer::once(ConflictAction::Replace)),
        });
        let report = finished(&mut runtime);
        assert_eq!(report.done.len(), 1, "{report:?}");
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
