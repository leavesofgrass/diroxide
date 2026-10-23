//! The plugin's end of the askpass road: one local endpoint per
//! connection, which the helper ssh starts reaches (R24 section 5.3).
//!
//! - **On Unix** a socket file, mode 0600, in a new folder of mode 0700
//!   under the plugin's runtime folder: only the owner's account can
//!   reach it. The folder and the socket are removed when the connection
//!   ends.
//! - **On Windows** a socket on 127.0.0.1 only, never another interface,
//!   so nothing outside the computer reaches it and no firewall question
//!   is raised. Any account on the computer could connect to it, so the
//!   proof is what keeps them out: without the token, a peer learns one
//!   random nonce and nothing else, and after [`MAX_REFUSALS`] bad
//!   proofs the endpoint closes. A named pipe that only the owner's
//!   account can open needs a security descriptor, an `unsafe` call this
//!   crate does not make; it is the contract change request of this lane
//!   (a user-only pipe in `diroxide-vfs`'s platform module).
//!
//! Every connection to the endpoint is one prompt: the helper's hello,
//! the plugin's challenge, the helper's proven prompt, the answer. The
//! answer is made by the plugin's handler, which asks the owner; the
//! endpoint itself never logs a prompt's answer.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use diroxide_core::Secret;

use super::proof::{LEN, Token, random};
use super::wire::{
    ANSWER, ASK, CHALLENGE, HELLO, STATUS_CONFIRM, STATUS_REFUSE, STATUS_TEXT, read_frame,
    write_frame,
};
use crate::ssh::AskpassEnv;

/// How many bad proofs close the endpoint.
pub const MAX_REFUSALS: usize = 3;

/// How long the helper has for each step of its proof.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// One prompt ssh gave the helper, after its proof was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// The prompt's bytes, exactly as ssh gave them.
    pub raw: Vec<u8>,
    /// `SSH_ASKPASS_PROMPT`, when ssh set it (`confirm`, `none`).
    pub kind: Option<String>,
}

impl Prompt {
    /// The prompt as text (not cleaned: for the classifier).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.raw).into_owned()
    }
}

/// The plugin's answer to one prompt.
#[derive(Debug)]
pub enum Reply {
    /// Text for ssh: a password, a passphrase, or "yes" to a new host key.
    Answer(Secret),
    /// A confirmation with no text (`SSH_ASKPASS_PROMPT=confirm`).
    Confirm,
    /// No answer: ssh stops asking.
    Refuse,
}

/// What answers prompts: the plugin's dialogs, or a test's script.
pub type Handler = Arc<dyn Fn(&Prompt) -> Reply + Send + Sync>;

/// A connection between the helper and the plugin.
#[derive(Debug)]
pub enum Conn {
    /// Over 127.0.0.1.
    Tcp(TcpStream),
    /// Over a socket file.
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Conn {
    /// Sets how long a read may wait.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Conn::Tcp(stream) => stream.set_read_timeout(timeout),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.set_read_timeout(timeout),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Tcp(stream) => stream.read(buf),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Tcp(stream) => stream.write(buf),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Tcp(stream) => stream.flush(),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.flush(),
        }
    }
}

enum Bound {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixListener),
}

impl Bound {
    fn accept(&self) -> io::Result<Conn> {
        let conn = match self {
            Bound::Tcp(listener) => {
                let (stream, _) = listener.accept()?;
                stream.set_nonblocking(false)?;
                Conn::Tcp(stream)
            }
            #[cfg(unix)]
            Bound::Unix(listener) => {
                let (stream, _) = listener.accept()?;
                stream.set_nonblocking(false)?;
                Conn::Unix(stream)
            }
        };
        conn.set_read_timeout(Some(STEP_TIMEOUT))?;
        Ok(conn)
    }
}

/// An endpoint for one connection.
pub struct Listener {
    endpoint: String,
    token: Token,
    stop: Arc<AtomicBool>,
    refused: Arc<AtomicUsize>,
    answered: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
    folder: Option<PathBuf>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("endpoint", &self.endpoint)
            .field("refused", &self.refused())
            .finish_non_exhaustive()
    }
}

fn bind(runtime: Option<&Path>) -> io::Result<(Bound, String, Option<PathBuf>)> {
    #[cfg(unix)]
    if let Some(runtime) = runtime {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let tag: String = random()
            .unwrap_or_default()
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect();
        let folder = runtime.join(format!("askpass-{tag}"));
        std::fs::DirBuilder::new().mode(0o700).create(&folder)?;
        let socket = folder.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("unix:{}", socket.display());
        return Ok((Bound::Unix(listener), endpoint, Some(folder)));
    }
    let _ = runtime;
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    listener.set_nonblocking(true)?;
    let endpoint = format!("tcp:{}", listener.local_addr()?);
    Ok((Bound::Tcp(listener), endpoint, None))
}

fn serve_one(stream: &mut Conn, token: &Token, handler: &Handler) -> Result<(), bool> {
    // Err(true) is a failed proof; Err(false) any other broken exchange.
    let (kind, parts) = read_frame(stream).map_err(|_| false)?;
    let helper_nonce = match (kind, parts.as_slice()) {
        (HELLO, [nonce]) if nonce.len() == LEN => nonce.clone(),
        _ => return Err(true),
    };
    let plugin_nonce = random().ok_or(false)?;
    let proof = token.plugin_proof(&helper_nonce, &plugin_nonce);
    write_frame(stream, CHALLENGE, &[&plugin_nonce, &proof]).map_err(|_| false)?;
    let (kind, parts) = read_frame(stream).map_err(|_| true)?;
    let (prompt_kind, raw, proof) = match (kind, parts.as_slice()) {
        (ASK, [prompt_kind, raw, proof]) => (prompt_kind, raw, proof),
        _ => return Err(true),
    };
    if !token.check_helper(&helper_nonce, &plugin_nonce, prompt_kind, raw, proof) {
        return Err(true);
    }
    let prompt = Prompt {
        raw: raw.clone(),
        kind: Some(String::from_utf8_lossy(prompt_kind).into_owned()).filter(|k| !k.is_empty()),
    };
    match handler(&prompt) {
        Reply::Answer(secret) => write_frame(
            stream,
            ANSWER,
            &[&[STATUS_TEXT], secret.expose().as_bytes()],
        ),
        Reply::Confirm => write_frame(stream, ANSWER, &[&[STATUS_CONFIRM], b""]),
        Reply::Refuse => write_frame(stream, ANSWER, &[&[STATUS_REFUSE], b""]),
    }
    .map_err(|_| false)
}

impl Listener {
    /// Opens an endpoint whose prompts `handler` answers; `runtime` is
    /// the plugin's own folder for the socket on Unix.
    pub fn open(runtime: Option<&Path>, handler: Handler) -> io::Result<Self> {
        let token = Token::new().ok_or_else(|| io::Error::other("no random numbers"))?;
        let (bound, endpoint, folder) = bind(runtime)?;
        let stop = Arc::new(AtomicBool::new(false));
        let refused = Arc::new(AtomicUsize::new(0));
        let answered = Arc::new(AtomicUsize::new(0));
        let thread = {
            let (stop, refused, answered, token) = (
                Arc::clone(&stop),
                Arc::clone(&refused),
                Arc::clone(&answered),
                token.clone(),
            );
            thread::Builder::new()
                .name("askpass-endpoint".to_owned())
                .spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        let mut stream = match bound.accept() {
                            Ok(stream) => stream,
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(20));
                                continue;
                            }
                            Err(_) => continue,
                        };
                        let (refused, answered, token, handler, stop) = (
                            Arc::clone(&refused),
                            Arc::clone(&answered),
                            token.clone(),
                            Arc::clone(&handler),
                            Arc::clone(&stop),
                        );
                        let _ = thread::Builder::new()
                            .name("askpass-prompt".to_owned())
                            .spawn(move || match serve_one(&mut stream, &token, &handler) {
                                Ok(()) => {
                                    answered.fetch_add(1, Ordering::SeqCst);
                                }
                                Err(true) => {
                                    if refused.fetch_add(1, Ordering::SeqCst) + 1 >= MAX_REFUSALS {
                                        stop.store(true, Ordering::SeqCst);
                                    }
                                }
                                Err(false) => {}
                            });
                    }
                })?
        };
        Ok(Self {
            endpoint,
            token,
            stop,
            refused,
            answered,
            thread: Some(thread),
            folder,
        })
    }

    /// The endpoint as the helper's environment names it:
    /// `tcp:127.0.0.1:PORT` or `unix:/path`.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The environment ssh gives the helper for this connection.
    pub fn askpass_env(&self, helper: PathBuf) -> AskpassEnv {
        AskpassEnv {
            helper,
            endpoint: self.endpoint.clone(),
            token: self.token.to_hex(),
        }
    }

    /// How many exchanges failed their proof.
    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::SeqCst)
    }

    /// How many prompts were answered (with an answer or a refusal).
    pub fn answered(&self) -> usize {
        self.answered.load(Ordering::SeqCst)
    }

    /// Whether it still accepts helpers.
    pub fn is_open(&self) -> bool {
        !self.stop.load(Ordering::SeqCst)
    }

    /// Stops accepting and removes the socket's folder on Unix.
    pub fn close(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(folder) = self.folder.take() {
            let _ = std::fs::remove_file(folder.join("s"));
            let _ = std::fs::remove_dir(&folder);
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.close();
    }
}

/// Connects to an endpoint the helper was given: only 127.0.0.1 or a
/// socket file, never another host.
pub fn connect(endpoint: &str) -> io::Result<Conn> {
    let refuse = || io::Error::new(io::ErrorKind::InvalidInput, "not a local endpoint");
    if let Some(address) = endpoint.strip_prefix("tcp:") {
        let address: SocketAddr = address.parse().map_err(|_| refuse())?;
        if !address.ip().is_loopback() {
            return Err(refuse());
        }
        let stream = TcpStream::connect_timeout(&address, STEP_TIMEOUT)?;
        stream.set_read_timeout(Some(STEP_TIMEOUT))?;
        return Ok(Conn::Tcp(stream));
    }
    #[cfg(unix)]
    if let Some(path) = endpoint.strip_prefix("unix:") {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        stream.set_read_timeout(Some(STEP_TIMEOUT))?;
        return Ok(Conn::Unix(stream));
    }
    Err(refuse())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_endpoints_are_reached() {
        for bad in [
            "tcp:10.0.0.7:22",
            "tcp:build.example:22",
            "tcp:0.0.0.0:22",
            "pipe:x",
            "",
        ] {
            assert_eq!(
                connect(bad).err().map(|e| e.kind()),
                Some(io::ErrorKind::InvalidInput),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_windows_endpoint_listens_on_loopback_only() {
        if cfg!(unix) {
            return;
        }
        let listener = Listener::open(None, Arc::new(|_: &Prompt| Reply::Refuse)).unwrap();
        let address: SocketAddr = listener
            .endpoint()
            .strip_prefix("tcp:")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(address.ip(), Ipv4Addr::LOCALHOST);
    }

    #[cfg(unix)]
    #[test]
    fn the_unix_endpoint_is_the_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let mut listener =
            Listener::open(Some(root.path()), Arc::new(|_: &Prompt| Reply::Refuse)).unwrap();
        let socket = PathBuf::from(listener.endpoint().strip_prefix("unix:").unwrap());
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&socket), 0o600);
        assert_eq!(mode(socket.parent().unwrap()), 0o700);
        listener.close();
        assert!(!socket.exists());
    }
}
