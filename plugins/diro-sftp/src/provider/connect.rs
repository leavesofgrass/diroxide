//! Connecting (R25 commit 57; S-A's notes, "What S-B should do first"):
//! the OpenSSH client found and asked for the host's settings, an askpass
//! endpoint kept for the connection's life, ssh started, the SFTP session
//! begun on its pipes; then the capabilities this server offers, the
//! watcher that notices a lost link, and the quiet close.
//!
//! - **Capabilities per connection:** server-side copy only when the
//!   server names `copy-data`, an atomic rename only with
//!   `posix-rename@openssh.com`, free space only with
//!   `statvfs@openssh.com`, and resume always.
//! - **A lost link** (ssh ended, the stream closed, a packet that could
//!   not be read) sends `provider/lost` once, with the reason in words,
//!   and the connection is gone: the next call answers "Not connected"
//!   and the host connects again.
//! - **Failures** are read in order: a changed host key first (refused,
//!   with the line to forget the old key, never run), then an ssh below
//!   8.4 refused a password it could not ask, then the rest.
//! - **Nothing secret is kept** but what answer 87 needs: ssh's own
//!   password for this account, in memory, wiped when the connection
//!   goes, so a second session never asks twice.

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::Secret;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use serde_json::{Value, json};

use super::write::Writes;
use super::{Call, authority};
use crate::askpass::classify::{PromptKind, classify};
use crate::askpass::dialogs::{ANSWER_LIMIT, Asker, PASSWORD_TRIES, host_key_refusal};
use crate::askpass::{Handler, Listener, Prompt, Reply};
use crate::lines;
use crate::plugin::State;
use crate::rpc::{Cancel, Peer};
use crate::sftp::errors::{self, Op};
use crate::sftp::mux::SftpError;
use crate::sftp::session::{Session, SessionConfig};
use crate::ssh::launch::own_program;
use crate::ssh::locate::probe;
use crate::ssh::options::{Remote, SessionOptions};
use crate::ssh::stderr::{self, Cause, refusal};
use crate::ssh::{Found, Launch, SearchPlaces, SshProcess, Target, locate, resolve};

/// How often the watcher looks at a connection's links.
pub const WATCH_EVERY: Duration = Duration::from_millis(100);

/// How often a connect that is still waiting tells the host so.
pub const CONNECTING_EVERY: Duration = Duration::from_secs(5);

/// How long ssh has to end after its session failed, so its last words
/// can be read.
pub const LAST_WORDS: Duration = Duration::from_secs(3);

/// ssh's own password for the connection's account, as the owner typed
/// it.
struct Remembered {
    user: String,
    host: String,
    secret: Secret,
}

/// What a connection remembers of the owner's answers: whether a secret
/// was typed (a session that needed one is never reopened unasked), and
/// ssh's own password for the connection's account (answer 87). Wiped
/// when it is dropped.
#[derive(Default)]
pub struct Memory {
    typed: AtomicBool,
    password: Mutex<Option<Remembered>>,
}

impl fmt::Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory")
            .field("typed", &self.typed())
            .field("password", &self.has_password())
            .finish()
    }
}

impl Memory {
    /// A memory with nothing in it.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Whether the owner typed a secret for this connection: a password,
    /// a passphrase or an answer to the server's question.
    pub fn typed(&self) -> bool {
        self.typed.load(Ordering::SeqCst)
    }

    /// Notes that a secret was typed.
    pub fn mark_typed(&self) {
        self.typed.store(true, Ordering::SeqCst);
    }

    /// Whether ssh's own password is remembered.
    pub fn has_password(&self) -> bool {
        self.password
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    fn remembered(&self, user: &str, host: &str) -> Option<Secret> {
        self.password
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|r| r.user == user && r.host == host)
            .map(|r| r.secret.clone())
    }

    fn remember(&self, user: &str, host: &str, secret: Secret) {
        *self.password.lock().unwrap_or_else(PoisonError::into_inner) = Some(Remembered {
            user: user.to_owned(),
            host: host.to_owned(),
            secret,
        });
    }

    /// Forgets the password.
    pub fn forget(&self) {
        self.password
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// One session's askpass handler. ssh's own password prompt for
    /// `account` (the user and host of the connection's `ssh -G` answer,
    /// and nothing else: a jump host's prompt names another host) is
    /// answered from memory when the owner typed it before; everything
    /// else goes to `asker`, the owner. With no asker (a second session)
    /// nothing is asked: what memory cannot answer is refused.
    pub fn handler(
        self: &Arc<Self>,
        asker: Option<Arc<Asker>>,
        account: Option<(String, String)>,
    ) -> Handler {
        let memory = Arc::clone(self);
        Arc::new(move |prompt: &Prompt| {
            let kind = classify(prompt);
            let ours = |user: &str, host: &str| {
                account
                    .as_ref()
                    .is_some_and(|(u, h)| u == user && h == host)
            };
            if let PromptKind::HostPassword { user, host } = &kind
                && ours(user, host)
                && let Some(secret) = memory.remembered(user, host)
            {
                return Reply::Answer(secret);
            }
            let Some(asker) = &asker else {
                return Reply::Refuse;
            };
            let reply = asker.answer(prompt);
            if let Reply::Answer(secret) = &reply {
                match &kind {
                    PromptKind::HostPassword { user, host } => {
                        memory.mark_typed();
                        if ours(user, host) {
                            memory.remember(user, host, secret.clone());
                        }
                    }
                    PromptKind::KeyPassphrase { .. } | PromptKind::Server { .. } => {
                        memory.mark_typed();
                    }
                    // A yes to a new host key is no secret: ssh wrote the
                    // key, and the next start does not ask again.
                    _ => {}
                }
            }
            reply
        })
    }
}

/// What an opener needs to open one session.
#[derive(Clone, Copy, Debug)]
pub struct OpenRequest<'a> {
    /// Where to connect.
    pub target: &'a Target,
    /// Whether the owner is looking: false for a background reconnect,
    /// which never asks anything.
    pub interactive: bool,
    /// Whether this is a connection's second session (answer 87): it asks
    /// the owner nothing and answers only from memory.
    pub second: bool,
    /// The host, for questions and progress.
    pub peer: &'a Arc<Peer>,
    /// The connection's memory.
    pub memory: &'a Arc<Memory>,
    /// The request's cancel flag.
    pub cancel: &'a Cancel,
    /// The plugin's settings and folders.
    pub state: &'a State,
    /// The host's progress token for the connect, when there is one.
    pub progress: Option<&'a str>,
}

/// One session, opened.
pub struct Opened {
    /// The SFTP session.
    pub session: Session,
    /// The ssh under it; none for a session in memory (the tests').
    pub process: Option<SshProcess>,
    /// The askpass endpoint, kept for the session's life.
    pub listener: Option<Listener>,
    /// The ssh in use, in words: "OpenSSH 9.9".
    pub ssh: Option<String>,
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opened")
            .field("session", &self.session)
            .field("ssh", &self.ssh)
            .finish_non_exhaustive()
    }
}

/// What opens sessions: ssh ([`SshOpener`]), or a test's fake server.
pub trait Opener: Send + Sync + fmt::Debug {
    /// Opens one session.
    fn open(&self, request: &OpenRequest<'_>) -> Result<Opened, RpcError>;
}

/// The opener that starts the OpenSSH client.
#[derive(Debug)]
pub struct SshOpener {
    helper: Option<PathBuf>,
    places: SearchPlaces,
    found: Mutex<Option<(Option<PathBuf>, Found)>>,
}

impl Default for SshOpener {
    fn default() -> Self {
        Self::new()
    }
}

impl SshOpener {
    /// The opener of this computer: ssh found by the rules of answer 85,
    /// the helper this program itself.
    pub fn new() -> Self {
        Self {
            helper: own_program(),
            places: SearchPlaces::from_env(),
            found: Mutex::new(None),
        }
    }

    /// The same, with another program as the askpass helper (a test runs
    /// the plugin in its own process, whose program is not the plugin's).
    pub fn with_helper(helper: PathBuf) -> Self {
        Self {
            helper: Some(helper),
            ..Self::new()
        }
    }

    /// The ssh to use, found once for each value of the setting.
    pub fn ssh(&self, state: &State) -> Result<Found, RpcError> {
        let setting = state.settings.ssh_program.clone();
        let mut found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((was, ssh)) = found.as_ref()
            && *was == setting
        {
            return Ok(ssh.clone());
        }
        let ssh = locate(setting.as_deref(), &self.places, probe).map_err(|e| {
            let lines = e.lines();
            RpcError::with_hint(
                ErrorKind::Unavailable,
                lines.first().cloned().unwrap_or_default(),
                lines[1.min(lines.len())..].join("\n"),
            )
        })?;
        *found = Some((setting, ssh.clone()));
        Ok(ssh)
    }
}

/// Sends a progress report for the connect, so the host knows it is
/// still under way.
fn report(peer: &Peer, token: Option<&str>, message: &str) {
    if let Some(token) = token {
        let mut params = Params::new();
        params.insert("token".into(), json!(token));
        params.insert(
            "value".into(),
            json!({"kind": "report", "state": "running", "message": message}),
        );
        peer.notify(methods::PROGRESS, params);
    }
}

/// Starts the SFTP session on ssh's pipes, on a thread of its own, so a
/// cancel can stop it: ssh is killed and the start ends.
fn start_watched(
    process: &mut SshProcess,
    config: &SessionConfig,
    request: &OpenRequest<'_>,
) -> Result<Session, SftpError> {
    let (Some(stdin), Some(stdout)) = (process.take_stdin(), process.take_stdout()) else {
        return Err(SftpError::Lost("ssh gave no pipes".to_owned()));
    };
    let (tx, rx) = mpsc::channel();
    let config = config.clone();
    thread::Builder::new()
        .name("sftp-start".to_owned())
        .spawn(move || {
            let _ = tx.send(Session::start(stdout, stdin, &config));
        })
        .map_err(|e| SftpError::Lost(e.to_string()))?;
    let began = Instant::now();
    let mut said = began;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(SftpError::Lost("the start stopped".to_owned()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if request.cancel.is_cancelled() {
            process.kill();
            let _ = rx.recv_timeout(Duration::from_secs(2));
            return Err(SftpError::Cancelled);
        }
        if said.elapsed() >= CONNECTING_EVERY {
            said = Instant::now();
            report(
                request.peer,
                request.progress,
                &lines::connecting(request.target.host()),
            );
        }
    }
}

/// How long the server has for its VERSION: the connect time, and, when
/// ssh may ask the owner, the time the owner has for each question.
fn version_timeout(state: &State, may_ask: bool) -> Duration {
    let connect = Duration::from_secs(u64::from(state.settings.connect_timeout));
    let asking = if may_ask {
        ANSWER_LIMIT * PASSWORD_TRIES
    } else {
        Duration::ZERO
    };
    SessionConfig::default().version_timeout + connect + asking
}

impl SshOpener {
    /// Why the session did not start, read in order: a changed host key,
    /// an ssh too old to ask a password, then the rest.
    fn failure(
        &self,
        error: &SftpError,
        process: &mut SshProcess,
        request: &OpenRequest<'_>,
        found: &Found,
    ) -> RpcError {
        let host = request.target.host();
        match error {
            SftpError::Cancelled => return errors::map(Op::Connect, error, host),
            // A shell that printed text, or a far end that is not SFTP,
            // may keep ssh running: it is ended at once.
            SftpError::ShellChatter(_) | SftpError::Protocol(_) => {
                process.kill();
                return errors::map(Op::Connect, error, host);
            }
            _ => {}
        }
        if process.wait_timeout(LAST_WORDS).is_none() {
            process.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while !process.stderr_ended() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let stderr = process.stderr_text();
        if let Some(refusal) = host_key_refusal(&stderr, host, request.target.port()) {
            return refusal;
        }
        if request.interactive
            && !found.version.askpass_force()
            && stderr.contains("Permission denied")
        {
            let lines = lines::too_old_for_passwords(cfg!(windows));
            return RpcError::with_hint(
                ErrorKind::AuthRequired,
                lines[0].clone(),
                lines[1..].join("\n"),
            );
        }
        if matches!(error, SftpError::TimedOut) && stderr::classify(&stderr) == Cause::Unknown {
            return errors::map(Op::Connect, error, host);
        }
        refusal(&stderr, host, request.target.port())
    }
}

impl Opener for SshOpener {
    fn open(&self, request: &OpenRequest<'_>) -> Result<Opened, RpcError> {
        let host = request.target.host();
        let settings = &request.state.settings;
        let found = self.ssh(request.state)?;
        request.cancel.check()?;
        let resolved = resolve(
            &found.path,
            request.target,
            settings.ssh_config_file.as_ref(),
        )
        .map_err(|e| {
            RpcError::with_hint(
                ErrorKind::Unavailable,
                lines::cannot_connect(host),
                e.to_string(),
            )
        })?;
        request.cancel.check()?;
        // A second session asks nothing: it may use the helper only to
        // give ssh the password the owner already typed.
        let interactive = request.interactive && (!request.second || request.memory.has_password());
        let debug_log = settings
            .ssh_debug_log
            .then(|| {
                request
                    .state
                    .state_dir
                    .as_ref()
                    .map(|d| d.join("ssh-debug.log"))
            })
            .flatten();
        let options = SessionOptions {
            target: request.target,
            resolved: &resolved,
            version: &found.version,
            settings,
            interactive,
            windows: cfg!(windows),
            key_file: None,
            add_keys_to_agent: false,
            debug_log,
            remote: Remote::Subsystem,
        };
        let may_ask = options.may_ask();
        let listener = if may_ask {
            let account = resolved
                .user()
                .zip(resolved.hostname())
                .map(|(u, h)| (u.to_owned(), h.to_owned()));
            let tries = resolved
                .get("numberofpasswordprompts")
                .and_then(|n| n.parse().ok())
                .unwrap_or(PASSWORD_TRIES);
            let asker =
                (!request.second).then(|| Asker::new(Arc::clone(request.peer), host, tries));
            let handler = request.memory.handler(asker, account);
            Some(
                Listener::open(request.state.state_dir.as_deref(), handler).map_err(|e| {
                    RpcError::with_hint(
                        ErrorKind::Unavailable,
                        lines::cannot_connect(host),
                        format!("The askpass endpoint failed: {e}"),
                    )
                })?,
            )
        } else {
            None
        };
        let askpass = match (&listener, &self.helper) {
            (Some(listener), Some(helper)) => Some(listener.askpass_env(helper.clone())),
            _ => None,
        };
        let launch = Launch {
            program: found.path.clone(),
            args: options.arguments(),
            askpass,
        };
        let mut process = launch.spawn().map_err(|e| {
            RpcError::with_hint(
                ErrorKind::Unavailable,
                lines::cannot_connect(host),
                e.to_string(),
            )
        })?;
        let config = SessionConfig {
            version_timeout: version_timeout(request.state, may_ask),
            ..SessionConfig::from(settings)
        };
        match start_watched(&mut process, &config, request) {
            Ok(session) => Ok(Opened {
                session,
                process: Some(process),
                listener,
                ssh: Some(found.version.words()),
            }),
            Err(error) => {
                let refusal = self.failure(&error, &mut process, request, &found);
                process.kill();
                Err(refusal)
            }
        }
    }
}

/// One SFTP session of a connection, with the ssh under it.
pub struct Link {
    session: Session,
    process: Mutex<Option<SshProcess>>,
    listener: Mutex<Option<Listener>>,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl Link {
    fn new(opened: Opened) -> Arc<Self> {
        Arc::new(Self {
            session: opened.session,
            process: Mutex::new(opened.process),
            listener: Mutex::new(opened.listener),
        })
    }

    /// The SFTP session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Whether the link is gone: its stream ended, or ssh did.
    pub fn is_dead(&self) -> bool {
        if self.session.mux().is_lost() {
            return true;
        }
        self.process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .is_some_and(|p| p.try_wait().is_some())
    }

    /// ssh's last words, for the reason a link was lost.
    pub fn stderr(&self) -> String {
        self.process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(SshProcess::stderr_text)
            .unwrap_or_default()
    }

    /// Ends the link: ssh and everything it started.
    pub fn kill(&self) {
        if let Some(mut process) = self
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            process.kill();
        }
        if let Some(mut listener) = self
            .listener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            listener.close();
        }
    }
}

/// A connection to one account on one host.
pub struct Connection {
    target: Target,
    peer: Arc<Peer>,
    memory: Arc<Memory>,
    opener: Arc<dyn Opener>,
    state: State,
    links: Mutex<Vec<Arc<Link>>>,
    ssh: Option<String>,
    gone: AtomicBool,
    said: AtomicBool,
    stop: AtomicBool,
    second_tried: AtomicBool,
    closing: Cancel,
    writes: Writes,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("target", &self.target)
            .field("gone", &self.is_gone())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Opens a connection's first session and starts its watcher.
    pub fn open(
        opener: Arc<dyn Opener>,
        target: &Target,
        interactive: bool,
        call: &Call<'_>,
    ) -> Result<Arc<Self>, RpcError> {
        let memory = Memory::new();
        let opened = opener.open(&OpenRequest {
            target,
            interactive,
            second: false,
            peer: call.peer,
            memory: &memory,
            cancel: call.cancel,
            state: call.state,
            progress: call.text("progress_token"),
        })?;
        let ssh = opened.ssh.clone();
        let connection = Arc::new(Self {
            target: target.clone(),
            peer: Arc::clone(call.peer),
            memory,
            opener,
            state: call.state.clone(),
            links: Mutex::new(vec![Link::new(opened)]),
            ssh,
            gone: AtomicBool::new(false),
            said: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            second_tried: AtomicBool::new(false),
            closing: Cancel::new(),
            writes: Writes::default(),
        });
        Self::watch(&connection);
        Ok(connection)
    }

    /// The session for a transfer (answer 87): the second session when it
    /// is open, so a listing on the first never waits behind a copy. The
    /// first transfer starts the second session in the background and
    /// goes on on the first; later ones move over once it is up. A
    /// connection that needed a typed secret other than ssh's own
    /// password never opens a second one, since it would have to ask
    /// again.
    pub fn transfer_link(self: &Arc<Self>) -> Result<Arc<Link>, RpcError> {
        if self.is_gone() {
            return Err(self.lost_error());
        }
        let first = {
            let links = self.links();
            if let Some(second) = links.get(1) {
                return Ok(Arc::clone(second));
            }
            links.first().cloned().ok_or_else(|| self.lost_error())?
        };
        let may = self.state.settings.sessions > 1
            && (!self.memory.typed() || self.memory.has_password());
        if may && !self.second_tried.swap(true, Ordering::SeqCst) {
            let connection = Arc::clone(self);
            let _ = thread::Builder::new()
                .name("sftp-second".to_owned())
                .spawn(move || connection.open_second());
        }
        Ok(first)
    }

    /// Opens the second session; on failure the connection keeps one.
    fn open_second(&self) {
        let opened = self.opener.open(&OpenRequest {
            target: &self.target,
            interactive: true,
            second: true,
            peer: &self.peer,
            memory: &self.memory,
            cancel: &self.closing,
            state: &self.state,
            progress: None,
        });
        if let Ok(opened) = opened {
            let link = Link::new(opened);
            let mut links = self.links();
            if self.is_gone() || links.len() != 1 {
                drop(links);
                link.kill();
                return;
            }
            links.push(link);
        }
    }

    /// The open write sessions.
    pub fn writes(&self) -> &Writes {
        &self.writes
    }

    /// How many sessions are open: one, or two once a transfer opened the
    /// second.
    pub fn sessions(&self) -> usize {
        self.links().len()
    }

    /// Forgets a path's cached read handles on every session: it was
    /// written, renamed or removed.
    pub fn forget(&self, path: &[u8]) {
        for link in self.links().iter() {
            link.session().forget_handle(path);
        }
    }

    /// The target.
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The host, for sentences.
    pub fn host(&self) -> &str {
        self.target.host()
    }

    /// The connection's memory.
    pub fn memory(&self) -> &Arc<Memory> {
        &self.memory
    }

    /// Whether it was lost or closed.
    pub fn is_gone(&self) -> bool {
        self.gone.load(Ordering::SeqCst)
    }

    /// Whether `provider/lost` was sent for it.
    pub fn lost_was_said(&self) -> bool {
        self.said.load(Ordering::SeqCst)
    }

    fn links(&self) -> std::sync::MutexGuard<'_, Vec<Arc<Link>>> {
        self.links.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The first session, for listings and every other quick call.
    pub fn link(&self) -> Result<Arc<Link>, RpcError> {
        if self.is_gone() {
            return Err(self.lost_error());
        }
        self.links()
            .first()
            .cloned()
            .ok_or_else(|| self.lost_error())
    }

    fn lost_error(&self) -> RpcError {
        errors::map(
            Op::Connect,
            &SftpError::Lost(String::new()),
            self.target.host(),
        )
    }

    /// An SFTP failure as the host's error; a lost session also loses the
    /// connection, which the host then hears about once.
    pub fn fail(self: &Arc<Self>, op: Op, error: &SftpError) -> RpcError {
        if errors::is_lost(error) {
            let connection = Arc::clone(self);
            let _ = thread::Builder::new()
                .name("sftp-lost".to_owned())
                .spawn(move || connection.lost());
        }
        errors::map(op, error, self.target.host())
    }

    /// Whether the server copies on its own (`copy-data`).
    pub fn can_copy_on_server(&self) -> bool {
        self.link()
            .is_ok_and(|l| l.session().extensions().has("copy-data"))
    }

    /// An SFTP failure on `path` as the host's error: a plain FAILURE is
    /// read by the operation's check first (R24 section 2.6), then
    /// mapped as [`Connection::fail`] does.
    pub fn fail_at(
        self: &Arc<Self>,
        op: Op,
        error: &SftpError,
        session: &Session,
        path: &[u8],
    ) -> RpcError {
        errors::refine(op, error, session, path).unwrap_or_else(|| self.fail(op, error))
    }

    /// What the server offers, for `provider/connect`'s answer.
    pub fn capabilities(&self) -> Value {
        let Ok(link) = self.link() else {
            return json!({});
        };
        let ext = link.session().extensions();
        let openssh = ext.names().iter().any(|n| n.ends_with("@openssh.com"));
        json!({
            "server_side_copy": ext.has("copy-data"),
            "atomic_rename": ext.has("posix-rename@openssh.com"),
            "resume": true,
            "free_space": ext.has("statvfs@openssh.com"),
            "fsync": ext.has("fsync@openssh.com"),
            "links": openssh,
            "trash": false,
            "watch": false,
        })
    }

    /// `provider/connect`'s answer: the name, the home folder as a place,
    /// the capabilities and the ssh in use.
    pub fn answer(&self) -> Value {
        let home = self
            .link()
            .ok()
            .and_then(|l| String::from_utf8(l.session().home().to_vec()).ok())
            .filter(|h| h.starts_with('/'))
            .unwrap_or_else(|| "/".to_owned());
        let mut answer = json!({
            "display_name": self.target.to_string(),
            "root": format!("{}{home}", authority(&self.target)),
            "capabilities": self.capabilities(),
        });
        if let Some(ssh) = &self.ssh {
            answer["ssh"] = json!(ssh);
        }
        answer
    }

    /// Closes the connection quietly: it was not lost.
    pub fn close(&self) {
        self.closing.cancel();
        self.writes.close_all();
        self.stop.store(true, Ordering::SeqCst);
        self.gone.store(true, Ordering::SeqCst);
        let links: Vec<Arc<Link>> = self.links().drain(..).collect();
        for link in links {
            link.session().close_handles();
            link.kill();
        }
        self.memory.forget();
    }

    /// The reason a dead link gives, in words.
    fn reason(&self, stderr: &str) -> String {
        stderr::lost_reason(stderr, self.target.host())
    }

    /// The connection is lost: its links end, and the host is told once.
    pub fn lost(&self) {
        if self.gone.swap(true, Ordering::SeqCst) {
            return;
        }
        self.closing.cancel();
        let links: Vec<Arc<Link>> = self.links().drain(..).collect();
        let stderr = links
            .iter()
            .map(|l| l.stderr())
            .collect::<Vec<_>>()
            .join("\n");
        for link in &links {
            link.kill();
        }
        self.memory.forget();
        self.say_lost(&self.reason(&stderr));
    }

    fn say_lost(&self, reason: &str) {
        if self.said.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut params = Params::new();
        params.insert("authority".into(), json!(authority(&self.target)));
        params.insert("reason".into(), json!(reason));
        self.peer.notify(methods::PROVIDER_LOST, params);
    }

    /// Watches the links on a thread of its own: a dead one loses the
    /// connection; idle read handles are closed.
    fn watch(connection: &Arc<Self>) {
        let weak: Weak<Self> = Arc::downgrade(connection);
        let _ = thread::Builder::new()
            .name("sftp-watch".to_owned())
            .spawn(move || {
                let mut swept = Instant::now();
                loop {
                    thread::sleep(WATCH_EVERY);
                    let Some(connection) = weak.upgrade() else {
                        return;
                    };
                    if connection.stop.load(Ordering::SeqCst) || connection.is_gone() {
                        return;
                    }
                    let links: Vec<Arc<Link>> = connection.links().clone();
                    if links.iter().any(|l| l.is_dead()) {
                        connection.lost();
                        return;
                    }
                    if swept.elapsed() >= Duration::from_secs(1) {
                        swept = Instant::now();
                        for link in &links {
                            link.session().sweep_handles(swept);
                        }
                    }
                }
            });
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        for link in self.links().drain(..) {
            link.kill();
        }
    }
}
