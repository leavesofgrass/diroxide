//! The plugin's calls to the host: an image's bytes through the host's
//! ranged `fs/read` ([`HostFile`]), its size and time through `fs/stat`,
//! a line through `ui/announce`, and 7-Zip or `qemu-img` through
//! `tools/run` and `tools/find`.
//!
//! - **The host's ranged read is the only way to an image's bytes** but
//!   the spool: the host checks the place against the owner's settings
//!   and reads it through its own providers, so an image on a local
//!   drive, inside a zip or on SFTP reads the same way, a block at a
//!   time.
//! - **A cancel reaches the wait.** The request being served sets its
//!   cancel flag for its thread ([`Serving`]); a host read made for it
//!   stops waiting when the flag is set.

use std::cell::RefCell;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::Timestamp;
use diroxide_plugins::base64;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use diroxide_plugins::shapes::WireEntry;
use serde_json::{Value, json};

use crate::error::{DiskError, HostIo};
use crate::image::source::Backing;
use crate::rpc::{Cancel, Peer};

/// The most one inline `fs/read` answers: the host's 1 MiB.
pub const MAX_INLINE_READ: usize = 1024 * 1024;

/// How long a call to the host may take before the plugin gives up.
pub const HOST_WAIT: Duration = Duration::from_secs(60);

thread_local! {
    static SERVING: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}

/// While alive, the thread serves a request whose cancel flag host reads
/// made for it watch.
pub struct Serving {
    before: Option<Cancel>,
}

impl Serving {
    /// The thread now serves the request with this flag.
    pub fn start(cancel: &Cancel) -> Self {
        let before = SERVING.with(|s| s.replace(Some(cancel.clone())));
        Self { before }
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        let before = self.before.take();
        SERVING.with(|s| *s.borrow_mut() = before);
    }
}

/// The cancel flag of the request this thread serves, if any.
pub fn serving() -> Option<Cancel> {
    SERVING.with(|s| s.borrow().clone())
}

/// Calls the host for the request this thread serves.
pub fn call(peer: &Peer, method: &str, params: Value) -> Result<Value, DiskError> {
    let Value::Object(params) = params else {
        return Err(DiskError::Io("a call had no parameters".to_owned()));
    };
    let cancel = serving();
    peer.call(method, params, cancel.as_ref(), HOST_WAIT)
        .map_err(DiskError::from)
}

/// What `fs/stat` says of an image file: its size and time, to open it
/// and to notice when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// Its size in bytes.
    pub size: u64,
    /// When it last changed, when the host says.
    pub modified: Option<Timestamp>,
}

/// The size and time of the image at `place`, through the host.
pub fn stat(peer: &Peer, place: &str) -> Result<Stamp, DiskError> {
    let answer = call(peer, methods::FS_STAT, json!({"place": place}))?;
    let entry = answer.get("entry").cloned().unwrap_or(answer);
    let wire: WireEntry = serde_json::from_value(entry)
        .map_err(|_| DiskError::Io("the host's answer about the image was not read".into()))?;
    let entry = wire.to_entry();
    if entry.is_folder_like() && entry.kind != diroxide_core::EntryKind::Archive {
        return Err(DiskError::NotAFolder);
    }
    let size = entry
        .size
        .ok_or_else(|| DiskError::Io("the host did not say the image's size".into()))?;
    Ok(Stamp {
        size,
        modified: entry.modified,
    })
}

/// Says a line through the host, at an importance: `status`, `info`,
/// `warning` or `error`.
pub fn announce(peer: &Peer, text: &str, importance: &str) {
    let mut params = Params::new();
    params.insert("text".into(), json!(text));
    params.insert("importance".into(), json!(importance));
    peer.notify(methods::UI_ANNOUNCE, params);
}

/// An image file read through the host's ranged `fs/read`.
pub struct HostFile {
    peer: Arc<Peer>,
    place: String,
    len: u64,
}

impl HostFile {
    /// The image at `place`, whose size the host said.
    pub fn new(peer: Arc<Peer>, place: String, len: u64) -> Self {
        Self { peer, place, len }
    }

    fn range(&self, offset: u64, length: usize) -> Result<Vec<u8>, DiskError> {
        let answer = call(
            &self.peer,
            methods::FS_READ,
            json!({"place": self.place, "offset": offset, "length": length, "transfer": "inline"}),
        )?;
        answer
            .get("data")
            .and_then(Value::as_str)
            .and_then(base64::decode)
            .ok_or_else(|| DiskError::Io("the host's bytes were not base64".into()))
    }
}

impl Backing for HostFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let wanted = buffer.len().min(room);
        let mut done = 0;
        while done < wanted {
            let length = (wanted - done).min(MAX_INLINE_READ);
            let bytes = self
                .range(offset + done as u64, length)
                .map_err(HostIo::into_io)?;
            if bytes.is_empty() {
                break;
            }
            let n = bytes.len().min(wanted - done);
            buffer[done..done + n].copy_from_slice(&bytes[..n]);
            done += n;
        }
        Ok(done)
    }
}

/// What `tools/find` says of a tool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Where it is.
    pub path: Option<String>,
    /// Its version line, when the host knows how to ask.
    pub version: Option<String>,
}

/// Whether a tool the manifest names is found, through the host.
pub fn find(peer: &Peer, tool: &str) -> Result<Option<Found>, DiskError> {
    let answer = call(peer, methods::TOOLS_FIND, json!({"tool": tool}))?;
    if !answer
        .get("found")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let text = |key: &str| answer.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(Some(Found {
        path: text("path"),
        version: text("version"),
    }))
}

/// What a tool run gave back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ran {
    /// Its exit code; `None` when it was stopped.
    pub code: Option<i32>,
    /// Its standard output, raw.
    pub stdout: Vec<u8>,
    /// Its standard error, as text.
    pub stderr: String,
    /// Whether the output passed its cap and the tool was stopped.
    pub truncated: bool,
}

/// A tool run: its arguments, never a shell line.
#[derive(Clone, Debug, Default)]
pub struct Run<'a> {
    /// The tool's name, as the manifest lists it.
    pub tool: &'a str,
    /// The arguments.
    pub args: Vec<String>,
    /// Text for its standard input.
    pub stdin: Option<String>,
    /// How long it may take.
    pub timeout: Option<Duration>,
    /// The most output it may make.
    pub max_output: Option<u64>,
}

/// Runs a tool through the host's `tools/run`, its output in base64 so
/// bytes come back as they are.
pub fn run(peer: &Peer, run: &Run<'_>) -> Result<Ran, DiskError> {
    let mut params = json!({
        "tool": run.tool,
        "args": run.args,
        "encoding": "base64",
        "env": {"LC_ALL": "C.UTF-8", "LANG": "C.UTF-8"},
    });
    if let Some(stdin) = &run.stdin {
        params["stdin"] = json!(stdin);
    }
    if let Some(timeout) = run.timeout {
        params["timeout_ms"] = json!(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX));
    }
    if let Some(max) = run.max_output {
        params["max_output_bytes"] = json!(max);
    }
    let answer = call(peer, methods::TOOLS_RUN, params)?;
    let bytes = |key: &str| {
        answer
            .get(key)
            .and_then(Value::as_str)
            .and_then(base64::decode)
            .unwrap_or_default()
    };
    Ok(Ran {
        code: answer
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok()),
        stdout: bytes("stdout"),
        stderr: String::from_utf8_lossy(&bytes("stderr")).into_owned(),
        truncated: answer
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Whether the host's answer says it does not offer a service.
pub fn not_offered(error: &DiskError) -> bool {
    matches!(error, DiskError::Host(RpcError { code, .. })
        if [ErrorKind::MethodNotFound.code(), ErrorKind::DeniedByHost.code(), ErrorKind::NotFound.code()]
            .contains(code))
}
