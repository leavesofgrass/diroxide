//! The `sftp` provider: the `provider/*` methods the host calls, served
//! over one connection per account and host (R24 "The plugin's shape";
//! R25 commits 57 to 64).
//!
//! - **Connections** are kept by their [`Target`] (host, user, port), so
//!   two accounts on one host never share a session. `provider/connect`
//!   opens one ([`connect`]); every other call finds it, or answers
//!   `auth_required` ("Not connected"), which the host meets by
//!   connecting.
//! - **A lost link** is noticed by the connection's watcher or by the
//!   call that met it; `provider/lost` goes to the host once, with the
//!   reason in words, and the connection is forgotten.
//! - **`provider/disconnect`** (the last lease ended) closes a connection
//!   quietly: it was not lost, so nothing is said.

pub mod connect;
pub mod copy;
pub mod fake;
pub mod list;
pub mod ops;
pub mod read;
pub mod volumes;
pub mod write;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use diroxide_core::Place;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use serde_json::Value;

pub use connect::{Connection, Link, Memory, OpenRequest, Opened, Opener, SshOpener};

use crate::lines;
use crate::plugin::{SCHEME, State};
use crate::rpc::{Cancel, Peer};
use crate::ssh::Target;

/// One request from the host, with what answering it needs.
#[derive(Clone, Copy, Debug)]
pub struct Call<'a> {
    /// The host, for questions, progress and notices.
    pub peer: &'a Arc<Peer>,
    /// The request's parameters.
    pub params: &'a Params,
    /// The request's cancel flag.
    pub cancel: &'a Cancel,
    /// The plugin's settings and folders.
    pub state: &'a State,
}

impl Call<'_> {
    /// A text parameter.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(Value::as_str)
    }

    /// A number parameter.
    pub fn number(&self, key: &str) -> Option<u64> {
        self.params.get(key).and_then(Value::as_u64)
    }

    /// A true-or-false parameter, false when absent.
    pub fn flag(&self, key: &str) -> bool {
        self.params
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// An `sftp` place named in a request: where to connect, and the path on
/// the server as bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
    /// The place as the host gave it.
    pub place: Place,
    /// The host, user and port.
    pub target: Target,
    /// The absolute path on the server.
    pub path: Vec<u8>,
}

impl At {
    /// The `sftp` place in a request's parameter `key`.
    pub fn from_params(params: &Params, key: &str) -> Result<Self, RpcError> {
        let text = params
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "a place is missing"))?;
        Self::parse(text)
    }

    /// An `sftp` place from its text.
    pub fn parse(text: &str) -> Result<Self, RpcError> {
        let place = Place::parse(text)
            .map_err(|e| RpcError::new(ErrorKind::InvalidParams, format!("the place: {e}")))?;
        let remote = place
            .remote()
            .filter(|r| r.scheme() == SCHEME)
            .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "not an sftp place"))?;
        let target = Target::from_place(&place)
            .map_err(|e| RpcError::new(ErrorKind::InvalidParams, e.to_string()))?;
        let path = remote.path().as_bytes().to_vec();
        Ok(Self {
            place,
            target,
            path,
        })
    }

    /// The last name of the path, as bytes; empty at the root.
    pub fn name(&self) -> &[u8] {
        match self.path.iter().rposition(|b| *b == b'/') {
            Some(at) => &self.path[at + 1..],
            None => &self.path,
        }
    }

    /// The folder that holds it; the root for the root.
    pub fn parent_path(&self) -> Vec<u8> {
        match self.path.iter().rposition(|b| *b == b'/') {
            Some(0) | None => b"/".to_vec(),
            Some(at) => self.path[..at].to_vec(),
        }
    }
}

/// The text of an authority as `provider/lost` names it:
/// `sftp://alice@build.example:2222`.
pub fn authority(target: &Target) -> String {
    format!("{SCHEME}://{target}")
}

/// The plugin's provider: its connections and how they are opened.
pub struct Provider {
    opener: Arc<dyn Opener>,
    connections: Mutex<HashMap<Target, Arc<Connection>>>,
    opening: Mutex<HashMap<Target, Arc<Mutex<()>>>>,
    sites: OnceLock<Arc<crate::sites::Sites>>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("opener", &self.opener)
            .field("connections", &self.lock().len())
            .finish()
    }
}

impl Provider {
    /// A provider whose connections `opener` makes.
    pub fn new(opener: Arc<dyn Opener>) -> Self {
        Self {
            opener,
            connections: Mutex::new(HashMap::new()),
            opening: Mutex::new(HashMap::new()),
            sites: OnceLock::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Target, Arc<Connection>>> {
        self.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The live connection to a target, or `auth_required` ("Not
    /// connected"); a connection that was lost is forgotten here.
    pub fn connection(&self, target: &Target) -> Result<Arc<Connection>, RpcError> {
        let mut connections = self.lock();
        match connections.get(target) {
            Some(connection) if !connection.is_gone() => Ok(Arc::clone(connection)),
            Some(_) => {
                connections.remove(target);
                Err(not_connected(target))
            }
            None => Err(not_connected(target)),
        }
    }

    /// The targets connected now.
    pub fn connected(&self) -> Vec<Target> {
        self.lock()
            .iter()
            .filter(|(_, c)| !c.is_gone())
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// Whether a connection to a target `of` names is open (`Some(true)`),
    /// was lost and said (`Some(false)`), or there is none: the state word
    /// of a site's row (S-C, R25 commit 73).
    pub fn link_state(&self, of: impl Fn(&Target) -> bool) -> Option<bool> {
        let connections = self.lock();
        let mut lost = false;
        for (target, connection) in connections.iter() {
            if of(target) {
                if !connection.is_gone() {
                    return Some(true);
                }
                lost |= connection.lost_was_said();
            }
        }
        lost.then_some(false)
    }

    /// `provider/connect`: opens the connection to the place's target, or
    /// finds the one open, and answers its capabilities.
    pub fn connect(&self, call: &Call<'_>) -> Result<Value, RpcError> {
        let at = At::from_params(call.params, "place")?;
        let gate = {
            let mut opening = self.opening.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(opening.entry(at.target.clone()).or_default())
        };
        // One connect at a time per target: a second waits and finds the
        // first's connection.
        let _turn = gate.lock().unwrap_or_else(PoisonError::into_inner);
        if let Ok(connection) = self.connection(&at.target) {
            return Ok(connection.answer());
        }
        let interactive = call.params.get("interactive").and_then(Value::as_bool) != Some(false);
        // The saved site this place belongs to, when one does: its ssh
        // options, and its saved password offered once (S-C).
        let sites = self.sites.get();
        let site = sites.and_then(|s| s.for_target(&at.target));
        let mut state = call.state.clone();
        state.site = site.as_ref().map(crate::sites::Site::options);
        let call = Call {
            state: &state,
            ..*call
        };
        let memory = Memory::new();
        if let (Some(sites), Some(site)) = (sites, &site)
            && let Some(secret) = sites.saved_secret(site)
        {
            memory.offer(secret);
        }
        let connection = Connection::open_with(
            Arc::clone(&self.opener),
            &at.target,
            interactive,
            &call,
            memory,
        )?;
        let answer = connection.answer();
        self.lock()
            .insert(at.target.clone(), Arc::clone(&connection));
        if let (Some(sites), Some(site)) = (sites, &site) {
            sites.after_login(site, connection.memory(), call.peer, call.cancel);
        }
        Ok(answer)
    }

    /// The askpass helper program its connections use, for `doctor`.
    pub fn helper(&self) -> Option<std::path::PathBuf> {
        self.opener.helper()
    }

    /// Gives the provider the plugin's sites, so a connection to a saved
    /// site uses its options and its saved password.
    pub fn set_sites(&self, sites: Arc<crate::sites::Sites>) {
        let _ = self.sites.set(sites);
    }

    /// Closes a connection quietly (`provider/disconnect`, or the owner's
    /// own disconnect); `false` when none was open.
    pub fn disconnect(&self, target: &Target) -> bool {
        let connection = self.lock().remove(target);
        match connection {
            Some(connection) => {
                connection.close();
                true
            }
            None => false,
        }
    }

    /// "Cancel connection" (R24 section 1.7): kills the target's ssh at
    /// once and sends `provider/lost`; `false` when none was open. The
    /// command that offers it is the sites lane's (S-C).
    pub fn cancel_connection(&self, target: &Target) -> bool {
        let connection = self.lock().remove(target);
        match connection {
            Some(connection) => {
                connection.cancel();
                true
            }
            None => false,
        }
    }

    /// Closes every connection: the plugin is shutting down.
    pub fn close_all(&self) {
        let all: Vec<Arc<Connection>> = self.lock().drain().map(|(_, c)| c).collect();
        for connection in all {
            connection.close();
        }
    }

    /// Answers one `provider/*` request.
    pub fn request(&self, method: &str, call: &Call<'_>) -> Result<Value, RpcError> {
        call.cancel.check()?;
        match method {
            methods::PROVIDER_CONNECT => self.connect(call),
            methods::PROVIDER_LIST => {
                let at = At::from_params(call.params, "place")?;
                list::list(&self.connection(&at.target)?, &at, call)
            }
            methods::PROVIDER_STAT => {
                let at = At::from_params(call.params, "place")?;
                list::stat(&self.connection(&at.target)?, &at, call)
            }
            methods::PROVIDER_READ => {
                let at = At::from_params(call.params, "place")?;
                read::read(&self.connection(&at.target)?, &at, call)
            }
            methods::PROVIDER_WRITE_BEGIN => {
                let at = At::from_params(call.params, "place")?;
                write::begin(&self.connection(&at.target)?, &at, call)
            }
            methods::PROVIDER_WRITE_CHUNK => write::chunk(&self.writing(call)?, call),
            methods::PROVIDER_WRITE_END => write::end(&self.writing(call)?, call),
            methods::PROVIDER_TRASH => ops::trash(),
            methods::PROVIDER_CREATE_DIR
            | methods::PROVIDER_REMOVE
            | methods::PROVIDER_SET_ATTRIBUTES
            | CREATE_LINK => {
                let at = At::from_params(call.params, "place")?;
                let connection = self.connection(&at.target)?;
                match method {
                    methods::PROVIDER_CREATE_DIR => ops::create_dir(&connection, &at, call),
                    methods::PROVIDER_REMOVE => ops::remove(&connection, &at, call),
                    methods::PROVIDER_SET_ATTRIBUTES => ops::set_attributes(&connection, &at, call),
                    _ => ops::create_link(&connection, &at, call),
                }
            }
            methods::PROVIDER_RENAME => {
                let at = At::from_params(call.params, "from")?;
                ops::rename(&self.connection(&at.target)?, &at, call)
            }
            methods::PROVIDER_COPY_HINT => copy::copy_hint(self, call),
            methods::PROVIDER_COPY => {
                let at = At::from_params(call.params, "from")?;
                copy::copy(&self.connection(&at.target)?, &at, call)
            }
            methods::PROVIDER_VOLUMES => {
                let at = At::from_params(call.params, "place")
                    .or_else(|_| At::from_params(call.params, "authority"))?;
                volumes::volumes(&self.connection(&at.target)?, call)
            }
            other => Err(RpcError::new(
                ErrorKind::Unsupported,
                format!("{other} is not served by this plugin"),
            )),
        }
    }

    /// The connection that holds the write session a request names.
    fn writing(&self, call: &Call<'_>) -> Result<Arc<Connection>, RpcError> {
        let id = call.text("write_id").unwrap_or_default();
        self.lock()
            .values()
            .find(|c| !c.is_gone() && c.writes().has(id))
            .cloned()
            .ok_or_else(write::no_session)
    }

    /// Takes one notification: `provider/disconnect` closes the place's
    /// connection, on a thread of its own so the reader never waits.
    pub fn notification(self: &Arc<Self>, method: &str, params: &Params) {
        if method != methods::PROVIDER_DISCONNECT {
            return;
        }
        let at = At::from_params(params, "place").or_else(|_| At::from_params(params, "authority"));
        if let Ok(at) = at {
            let provider = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("sftp-disconnect".to_owned())
                .spawn(move || provider.disconnect(&at.target));
        }
    }
}

/// Makes a link: served for when the host's provider methods gain it
/// (a contract request of this lane); not in the manifest's list yet.
pub const CREATE_LINK: &str = "provider/create_link";

/// "Not connected to build.example.", with the next step.
pub fn not_connected(target: &Target) -> RpcError {
    RpcError::with_hint(
        ErrorKind::AuthRequired,
        lines::not_connected(target.host()),
        lines::OPEN_TO_CONNECT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_give_the_target_and_the_path_as_bytes() {
        let at = At::parse("sftp://ada@build.example:2222/srv/www/index.html").unwrap();
        assert_eq!(at.target.host(), "build.example");
        assert_eq!(at.target.user(), Some("ada"));
        assert_eq!(at.target.port(), Some(2222));
        assert_eq!(at.path, b"/srv/www/index.html");
        assert_eq!(at.name(), b"index.html");
        assert_eq!(at.parent_path(), b"/srv/www");
        assert_eq!(
            At::parse("sftp://build.example/a").unwrap().parent_path(),
            b"/"
        );
        assert_eq!(At::parse("sftp://build.example/").unwrap().name(), b"");
        assert_eq!(
            authority(&at.target),
            "sftp://ada@build.example:2222".to_owned()
        );
        for bad in [
            "memfs://box/x",
            "sftp://-oProxyCommand=x/",
            "D:/local",
            "sftp://a b/",
        ] {
            assert_eq!(
                At::parse(bad).unwrap_err().code,
                ErrorKind::InvalidParams.code(),
                "{bad}"
            );
        }
    }
}
