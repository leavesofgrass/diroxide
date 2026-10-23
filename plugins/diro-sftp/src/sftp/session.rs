//! An SFTP session (R24 section 2.5; answer 88): the handshake, the
//! extension table, the server's limits, the home folder, a cache of open
//! handles, and transfers with many requests in flight.
//!
//! - **The handshake** sends INIT and waits, with a limit, for VERSION as
//!   the first packet. A first packet whose length is absurd is text a
//!   shell startup file printed into the stream ([`SftpError::ShellChatter`],
//!   with its first bytes cleaned); a first packet that is not VERSION is
//!   not SFTP.
//! - **Extensions** are used only when the server names them
//!   ([`Extensions::has`]); `limits@openssh.com` sets the packet bound and
//!   the chunk size, never above the owner's setting.
//! - **Transfers** keep up to `requests` (64) requests of `chunk` (32 KiB)
//!   in flight: 2 MiB a transfer by default. Reads are delivered in order;
//!   a short read asks again for the rest before going on; every write's
//!   answer is checked, because a later write can succeed after an
//!   earlier one failed.
//! - **Handles** for reading are cached, at most [`MAX_HANDLES`] (fewer
//!   when the server's limit is lower), each closed after
//!   [`HANDLE_IDLE`] unused or when the cache is full.

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read, Write};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::codec::{
    Attrs, DEFAULT_MAX_PACKET, Limits, Request, Response, StatVfs, VERSION, frame, status, strings,
};
use super::mux::{Mux, SftpError, Ticket};
use crate::lines;
use crate::rpc::Cancel;
use crate::settings::Settings;

/// The most handles the cache keeps open.
pub const MAX_HANDLES: usize = 16;

/// How long a cached handle may stay unused before it is closed.
pub const HANDLE_IDLE: Duration = Duration::from_secs(10);

/// The most a server's first bytes are shown when they are not SFTP.
pub const CHATTER_SHOWN: usize = 200;

/// How a session is run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// Requests in flight a transfer (answer 88: 64).
    pub requests: usize,
    /// Bytes a request (answer 88: 32 KiB), before the server's limits.
    pub chunk: usize,
    /// How long the server has for its VERSION.
    pub version_timeout: Duration,
    /// How long one request may wait.
    pub call_timeout: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            requests: 64,
            chunk: 32 * 1024,
            version_timeout: Duration::from_secs(30),
            call_timeout: Duration::from_secs(30),
        }
    }
}

impl From<&Settings> for SessionConfig {
    fn from(settings: &Settings) -> Self {
        Self {
            requests: settings.requests as usize,
            chunk: settings.chunk_kib as usize * 1024,
            ..Self::default()
        }
    }
}

/// The server's extensions, as its VERSION named them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extensions(Vec<(Vec<u8>, Vec<u8>)>);

impl Extensions {
    /// Whether the server names an extension.
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n == name.as_bytes())
    }

    /// The version or data the server gave an extension.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.0
            .iter()
            .find(|(n, _)| n == name.as_bytes())
            .map(|(_, v)| v.as_slice())
    }

    /// Every name, as text, for the log.
    pub fn names(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|(n, _)| String::from_utf8_lossy(n).into_owned())
            .collect()
    }
}

/// Open handles kept for reading, by path, closed when unused.
#[derive(Debug, Default)]
pub struct HandleCache {
    open: HashMap<Vec<u8>, (Vec<u8>, Instant)>,
    limit: usize,
}

impl HandleCache {
    /// A cache of at most `limit` handles.
    pub fn new(limit: usize) -> Self {
        Self {
            open: HashMap::new(),
            limit: limit.max(1),
        }
    }

    /// How many handles are open.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    /// Whether none is open.
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// The handle open for a path, marked used now.
    pub fn get(&mut self, path: &[u8], now: Instant) -> Option<Vec<u8>> {
        let (handle, used) = self.open.get_mut(path)?;
        *used = now;
        Some(handle.clone())
    }

    /// Keeps a new handle; the answer is the handles to close to stay
    /// within the limit, the least lately used first.
    pub fn insert(&mut self, path: &[u8], handle: Vec<u8>, now: Instant) -> Vec<Vec<u8>> {
        let mut close = Vec::new();
        if let Some((old, _)) = self.open.insert(path.to_vec(), (handle, now)) {
            close.push(old);
        }
        while self.open.len() > self.limit {
            let oldest = self
                .open
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(p, _)| p.clone());
            match oldest.and_then(|p| self.open.remove(&p)) {
                Some((handle, _)) => close.push(handle),
                None => break,
            }
        }
        close
    }

    /// Forgets a path's handle (it was closed, or the file changed).
    pub fn remove(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        self.open.remove(path).map(|(h, _)| h)
    }

    /// The handles unused for longer than `idle`, taken out to be closed.
    pub fn sweep(&mut self, now: Instant, idle: Duration) -> Vec<Vec<u8>> {
        let stale: Vec<Vec<u8>> = self
            .open
            .iter()
            .filter(|(_, (_, used))| now.saturating_duration_since(*used) >= idle)
            .map(|(p, _)| p.clone())
            .collect();
        stale
            .into_iter()
            .filter_map(|p| self.open.remove(&p).map(|(h, _)| h))
            .collect()
    }

    /// Every handle, taken out to be closed.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        self.open.drain().map(|(_, (h, _))| h).collect()
    }
}

/// An SFTP session over ssh's pipes, or any pair of streams.
pub struct Session {
    mux: Mux,
    extensions: Extensions,
    limits: Option<Limits>,
    chunk: usize,
    requests: usize,
    timeout: Duration,
    home: Vec<u8>,
    handles: Mutex<HandleCache>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("extensions", &self.extensions.names())
            .field("chunk", &self.chunk)
            .field("requests", &self.requests)
            .finish_non_exhaustive()
    }
}

/// Reads the first packet with the codec's bound; the reader comes back
/// with whatever it read past that packet.
type First = (Box<dyn Read + Send>, Vec<u8>, Result<Response, SftpError>);

fn read_first(mut input: Box<dyn Read + Send>) -> First {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match frame(&buffer, DEFAULT_MAX_PACKET) {
            Ok(Some((body, used))) => {
                let result = Response::decode(body).map_err(|e| SftpError::Protocol(e.to_string()));
                let rest = buffer[used..].to_vec();
                return (input, rest, result);
            }
            Ok(None) => {}
            Err(_) => {
                // A length no server sends: text, most likely a shell's.
                // What came is shown; nothing more is waited for, since a
                // server that printed text may print nothing else.
                buffer.truncate(CHATTER_SHOWN);
                let text = lines::cleaned(&String::from_utf8_lossy(&buffer), CHATTER_SHOWN);
                return (input, Vec::new(), Err(SftpError::ShellChatter(text)));
            }
        }
        match input.read(&mut chunk) {
            Ok(0) => {
                return (
                    input,
                    Vec::new(),
                    Err(SftpError::Lost(
                        "the server ended before it spoke SFTP".to_owned(),
                    )),
                );
            }
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(e) => return (input, Vec::new(), Err(SftpError::Lost(e.to_string()))),
        }
    }
}

fn status_of(response: &Response) -> Option<(u32, &[u8])> {
    match response {
        Response::Status { code, message, .. } => Some((*code, message.as_slice())),
        _ => None,
    }
}

fn refused(response: Response) -> SftpError {
    match response {
        Response::Status { code, message, .. } => SftpError::Status { code, message },
        other => SftpError::Protocol(format!("an unexpected {} answer", other.name())),
    }
}

/// What a pipelined read is waiting for.
struct Pending {
    offset: u64,
    len: usize,
    ticket: Ticket,
}

impl Session {
    /// Starts a session: INIT, VERSION, the server's limits, the home
    /// folder.
    pub fn start(
        input: impl Read + Send + 'static,
        mut output: impl Write + Send + 'static,
        config: &SessionConfig,
    ) -> Result<Self, SftpError> {
        let init = Request::Init {
            version: VERSION,
            extensions: Vec::new(),
        };
        output
            .write_all(&init.encode())
            .and_then(|()| output.flush())
            .map_err(|e| SftpError::Lost(e.to_string()))?;
        let (tx, rx) = mpsc::channel();
        let input: Box<dyn Read + Send> = Box::new(input);
        thread::Builder::new()
            .name("sftp-version".to_owned())
            .spawn(move || {
                let _ = tx.send(read_first(input));
            })
            .map_err(|e| SftpError::Lost(e.to_string()))?;
        let (input, rest, first) = rx
            .recv_timeout(config.version_timeout)
            .map_err(|_| SftpError::TimedOut)?;
        let extensions = match first? {
            Response::Version {
                version: VERSION,
                extensions,
            } => Extensions(extensions),
            Response::Version { version, .. } => {
                return Err(SftpError::Protocol(format!(
                    "the server speaks version {version}"
                )));
            }
            other => {
                return Err(SftpError::Protocol(format!(
                    "the first answer was {}",
                    other.name()
                )));
            }
        };
        let mux = Mux::start(Cursor::new(rest).chain(input), output, DEFAULT_MAX_PACKET);
        let mut session = Self {
            mux,
            extensions,
            limits: None,
            chunk: config.chunk.max(1024),
            requests: config.requests.max(1),
            timeout: config.call_timeout,
            home: Vec::new(),
            handles: Mutex::new(HandleCache::new(MAX_HANDLES)),
        };
        if session.extensions.has("limits@openssh.com") {
            let answer = session.extended("limits@openssh.com", Vec::new(), None)?;
            if let Response::ExtendedReply { data, .. } = answer
                && let Ok(limits) = Limits::decode(&data)
            {
                session.apply_limits(limits);
            }
        }
        session.home = session.realpath(b".", None)?;
        Ok(session)
    }

    fn apply_limits(&mut self, limits: Limits) {
        let positive = |n: u64| (n > 0).then(|| usize::try_from(n).unwrap_or(usize::MAX));
        for bound in [positive(limits.max_read), positive(limits.max_write)]
            .into_iter()
            .flatten()
        {
            self.chunk = self.chunk.min(bound);
        }
        if let Some(handles) = positive(limits.max_open_handles) {
            let limit = handles.saturating_sub(4).clamp(1, MAX_HANDLES);
            *self.handles.lock().unwrap_or_else(PoisonError::into_inner) = HandleCache::new(limit);
        }
        self.limits = Some(limits);
    }

    /// The server's extensions.
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// The server's limits, when it gave them.
    pub fn limits(&self) -> Option<Limits> {
        self.limits
    }

    /// The bytes one request carries.
    pub fn chunk(&self) -> usize {
        self.chunk
    }

    /// The requests a transfer keeps in flight.
    pub fn requests(&self) -> usize {
        self.requests
    }

    /// The home folder the server gave for ".".
    pub fn home(&self) -> &[u8] {
        &self.home
    }

    /// The multiplexer, for its counts.
    pub fn mux(&self) -> &Mux {
        &self.mux
    }

    /// One request and its answer.
    pub fn call(
        &self,
        make: impl FnOnce(u32) -> Request,
        cancel: Option<&Cancel>,
    ) -> Result<Response, SftpError> {
        self.mux.call(make, cancel, self.timeout)
    }

    fn expect_ok(&self, response: Response) -> Result<(), SftpError> {
        match status_of(&response) {
            Some((status::OK, _)) => Ok(()),
            _ => Err(refused(response)),
        }
    }

    /// An extension's request; refused when the server does not name it.
    pub fn extended(
        &self,
        name: &str,
        data: Vec<u8>,
        cancel: Option<&Cancel>,
    ) -> Result<Response, SftpError> {
        if !self.extensions.has(name) {
            return Err(SftpError::Status {
                code: status::OP_UNSUPPORTED,
                message: Vec::new(),
            });
        }
        let name = name.as_bytes().to_vec();
        self.call(|id| Request::Extended { id, name, data }, cancel)
    }

    /// A path made absolute by the server.
    pub fn realpath(&self, path: &[u8], cancel: Option<&Cancel>) -> Result<Vec<u8>, SftpError> {
        let path = path.to_vec();
        match self.call(|id| Request::Realpath { id, path }, cancel)? {
            Response::Name { mut entries, .. } if !entries.is_empty() => {
                Ok(entries.swap_remove(0).filename)
            }
            other => Err(refused(other)),
        }
    }

    /// A path's attributes, links followed.
    pub fn stat(&self, path: &[u8], cancel: Option<&Cancel>) -> Result<Attrs, SftpError> {
        let path = path.to_vec();
        match self.call(|id| Request::Stat { id, path }, cancel)? {
            Response::Attrs { attrs, .. } => Ok(attrs),
            other => Err(refused(other)),
        }
    }

    /// Opens a file with [`super::codec::open`] flags.
    pub fn open(
        &self,
        path: &[u8],
        pflags: u32,
        attrs: Attrs,
        cancel: Option<&Cancel>,
    ) -> Result<Vec<u8>, SftpError> {
        let path = path.to_vec();
        match self.call(
            |id| Request::Open {
                id,
                path,
                pflags,
                attrs,
            },
            cancel,
        )? {
            Response::Handle { handle, .. } => Ok(handle),
            other => Err(refused(other)),
        }
    }

    /// Closes a handle; its status is checked (a quota error can come
    /// here).
    pub fn close(&self, handle: &[u8]) -> Result<(), SftpError> {
        let handle = handle.to_vec();
        let answer = self.call(|id| Request::Close { id, handle }, None)?;
        self.expect_ok(answer)
    }

    /// The free space at a path, when the server offers `statvfs`; `None`
    /// is "not known", never zero.
    pub fn statvfs(
        &self,
        path: &[u8],
        cancel: Option<&Cancel>,
    ) -> Result<Option<StatVfs>, SftpError> {
        if !self.extensions.has("statvfs@openssh.com") {
            return Ok(None);
        }
        match self.extended("statvfs@openssh.com", strings(&[path]), cancel)? {
            Response::ExtendedReply { data, .. } => Ok(StatVfs::decode(&data).ok()),
            other => Err(refused(other)),
        }
    }

    /// A handle for reading a path, from the cache or newly opened; the
    /// cache closes what it no longer keeps.
    pub fn read_handle(&self, path: &[u8], cancel: Option<&Cancel>) -> Result<Vec<u8>, SftpError> {
        let now = Instant::now();
        if let Some(handle) = self
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path, now)
        {
            return Ok(handle);
        }
        let handle = self.open(path, super::codec::open::READ, Attrs::default(), cancel)?;
        let close = self
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path, handle.clone(), now);
        for old in close {
            let _ = self.close(&old);
        }
        Ok(handle)
    }

    /// Closes the cached handles unused for [`HANDLE_IDLE`].
    pub fn sweep_handles(&self, now: Instant) {
        let stale = self
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sweep(now, HANDLE_IDLE);
        for handle in stale {
            let _ = self.close(&handle);
        }
    }

    /// How many handles the cache keeps open.
    pub fn cached_handles(&self) -> usize {
        self.handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn read_ticket(&self, handle: &[u8], offset: u64, len: usize) -> Result<Pending, SftpError> {
        let h = handle.to_vec();
        let asked = u32::try_from(len).unwrap_or(u32::MAX);
        let ticket = self.mux.send(|id| Request::Read {
            id,
            handle: h,
            offset,
            len: asked,
        })?;
        Ok(Pending {
            offset,
            len,
            ticket,
        })
    }

    /// Reads up to `len` bytes from `offset` with requests in flight, and
    /// hands them to `sink` in order; `sink` answering false stops the
    /// read. The answer is how many bytes were handed over; fewer than
    /// `len` means the file ended.
    pub fn read_pipelined(
        &self,
        handle: &[u8],
        offset: u64,
        len: u64,
        cancel: Option<&Cancel>,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<u64, SftpError> {
        let end = offset.saturating_add(len);
        let mut next = offset;
        let mut window: VecDeque<Pending> = VecDeque::new();
        let mut delivered = 0u64;
        loop {
            while window.len() < self.requests && next < end {
                let n = usize::try_from((end - next).min(self.chunk as u64)).unwrap_or(self.chunk);
                window.push_back(self.read_ticket(handle, next, n)?);
                next += n as u64;
            }
            let Some(front) = window.pop_front() else {
                return Ok(delivered);
            };
            match front.ticket.wait(cancel, self.timeout)? {
                Response::Data { data, .. } => {
                    if data.len() > front.len {
                        return Err(SftpError::Protocol(
                            "the server sent more than was asked".to_owned(),
                        ));
                    }
                    delivered += data.len() as u64;
                    if !sink(&data) {
                        return Ok(delivered);
                    }
                    if data.len() < front.len {
                        // A short read: the rest of this piece comes next.
                        let at = front.offset + data.len() as u64;
                        let rest = front.len - data.len();
                        window.push_front(self.read_ticket(handle, at, rest)?);
                    }
                }
                Response::Status {
                    code: status::EOF, ..
                } => return Ok(delivered),
                other => return Err(refused(other)),
            }
        }
    }

    /// Writes `data` at `offset` with requests in flight; every answer is
    /// checked.
    pub fn write_pipelined(
        &self,
        handle: &[u8],
        offset: u64,
        data: &[u8],
        cancel: Option<&Cancel>,
    ) -> Result<(), SftpError> {
        let mut window: VecDeque<Ticket> = VecDeque::new();
        let mut at = 0usize;
        let mut first_error = None;
        while at < data.len() || !window.is_empty() {
            while window.len() < self.requests && at < data.len() && first_error.is_none() {
                let end = (at + self.chunk).min(data.len());
                let (h, piece) = (handle.to_vec(), data[at..end].to_vec());
                let place = offset + at as u64;
                window.push_back(self.mux.send(|id| Request::Write {
                    id,
                    handle: h,
                    offset: place,
                    data: piece,
                })?);
                at = end;
            }
            let Some(ticket) = window.pop_front() else {
                break;
            };
            let answer = ticket.wait(cancel, self.timeout);
            match answer {
                Ok(response) if status_of(&response).is_some_and(|(c, _)| c == status::OK) => {}
                Ok(response) => {
                    first_error.get_or_insert(refused(response));
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
            if first_error.is_some() && at < data.len() {
                // Nothing more is sent; what is in flight is still read.
                at = data.len();
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Closes every cached handle.
    pub fn close_handles(&self) {
        let all = self
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain();
        for handle in all {
            let _ = self.close(&handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handle_cache_keeps_sixteen_and_closes_the_oldest() {
        let mut cache = HandleCache::new(MAX_HANDLES);
        let start = Instant::now();
        let mut closed = Vec::new();
        for i in 0..20u64 {
            let at = start + Duration::from_millis(i);
            closed.extend(cache.insert(
                format!("/f{i}").as_bytes(),
                format!("h{i}").into_bytes(),
                at,
            ));
        }
        assert_eq!(cache.len(), MAX_HANDLES);
        assert_eq!(
            closed,
            [
                b"h0".to_vec(),
                b"h1".to_vec(),
                b"h2".to_vec(),
                b"h3".to_vec()
            ]
        );
        // A use keeps a handle from being the oldest.
        let later = start + Duration::from_millis(100);
        assert_eq!(cache.get(b"/f4", later), Some(b"h4".to_vec()));
        let more = cache.insert(b"/f20", b"h20".to_vec(), later);
        assert_eq!(more, [b"h5".to_vec()]);
        // Ten idle seconds close the rest but the one just used.
        let idle = cache.sweep(later + HANDLE_IDLE - Duration::from_millis(1), HANDLE_IDLE);
        assert_eq!(idle.len(), MAX_HANDLES - 2);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn settings_make_the_window() {
        let settings = Settings::default();
        let config = SessionConfig::from(&settings);
        assert_eq!((config.requests, config.chunk), (64, 32 * 1024));
    }
}
