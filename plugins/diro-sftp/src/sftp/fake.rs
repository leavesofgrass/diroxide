//! The fake SFTP server of the test plan (R24 section 2.5; R25 commit 55):
//! a file system in memory served over any pair of byte streams with the
//! plugin's own codec, so faults can be made on purpose in CI without a
//! container: a stall, short replies, replies out of order, a reused
//! request id, a stream that drops after so many bytes, and a server with
//! no `statvfs@openssh.com`.
//!
//! It serves what the provider needs: INIT and VERSION, REALPATH, folders
//! (OPENDIR, READDIR in batches, MKDIR, RMDIR), files (OPEN with its
//! flags, READ, WRITE, CLOSE), STAT, LSTAT, FSTAT, SETSTAT, REMOVE,
//! RENAME, and the OpenSSH extensions `posix-rename`, `statvfs`,
//! `limits` and `fsync`. Paths are bytes, as on a real server; a name
//! that is not UTF-8 is served as it is. The fake ssh's `serve` step
//! runs it on its standard input and output.
//!
//! It is a test double: public so the fake ssh and the tests of other
//! lanes use the same one, never used by the plugin itself.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use super::codec::{
    Attrs, DEFAULT_MAX_PACKET, Limits, NameEntry, Request, Response, StatVfs, VERSION, frame, open,
    read_strings, status,
};

/// How many names one READDIR answers.
pub const BATCH: usize = 100;

/// The permission bits of a folder and a file (with their type bits).
const DIR_MODE: u32 = 0o040_755;
const FILE_MODE: u32 = 0o100_644;
const LINK_MODE: u32 = 0o120_777;

/// One entry of the fake file system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A folder.
    Dir,
    /// A file and its bytes.
    File(Vec<u8>),
    /// A symbolic link and its target.
    Link(Vec<u8>),
}

/// A file system in memory, by absolute path in bytes.
#[derive(Clone, Debug)]
pub struct FakeFs {
    nodes: BTreeMap<Vec<u8>, Node>,
    modes: HashMap<Vec<u8>, u32>,
    times: HashMap<Vec<u8>, (u32, u32)>,
    /// The home folder REALPATH gives for ".".
    pub home: Vec<u8>,
    /// Every request handled, by name, in order, for the tests.
    pub calls: Vec<&'static str>,
}

impl Default for FakeFs {
    fn default() -> Self {
        let mut fs = Self {
            nodes: BTreeMap::new(),
            modes: HashMap::new(),
            times: HashMap::new(),
            home: b"/home/ada".to_vec(),
            calls: Vec::new(),
        };
        fs.nodes.insert(b"/".to_vec(), Node::Dir);
        fs.add_dir(b"/home/ada");
        fs
    }
}

fn parent(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|b| *b == b'/') {
        Some(0) | None => b"/",
        Some(at) => &path[..at],
    }
}

fn name(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|b| *b == b'/') {
        Some(at) => &path[at + 1..],
        None => path,
    }
}

/// A path made absolute against the home folder and its "." and ".."
/// parts resolved; trailing slashes dropped.
fn normal(home: &[u8], path: &[u8]) -> Vec<u8> {
    let joined: Vec<u8> = if path.first() == Some(&b'/') {
        path.to_vec()
    } else {
        let mut j = home.to_vec();
        j.push(b'/');
        j.extend_from_slice(path);
        j
    };
    let mut parts: Vec<&[u8]> = Vec::new();
    for part in joined.split(|b| *b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut out = Vec::new();
    for part in parts {
        out.push(b'/');
        out.extend_from_slice(part);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

impl FakeFs {
    /// Adds a folder and its parents.
    pub fn add_dir(&mut self, path: &[u8]) {
        let mut at = Vec::new();
        for part in path.split(|b| *b == b'/').filter(|p| !p.is_empty()) {
            at.push(b'/');
            at.extend_from_slice(part);
            self.nodes.entry(at.clone()).or_insert(Node::Dir);
        }
    }

    /// Adds a file, with its folder.
    pub fn add_file(&mut self, path: &[u8], bytes: &[u8]) {
        self.add_dir(parent(path));
        self.nodes.insert(path.to_vec(), Node::File(bytes.to_vec()));
    }

    /// Adds a symbolic link.
    pub fn add_link(&mut self, path: &[u8], target: &[u8]) {
        self.add_dir(parent(path));
        self.nodes
            .insert(path.to_vec(), Node::Link(target.to_vec()));
    }

    /// A file's bytes, when it is a file.
    pub fn file(&self, path: &[u8]) -> Option<&[u8]> {
        match self.nodes.get(path) {
            Some(Node::File(bytes)) => Some(bytes),
            _ => None,
        }
    }

    /// The entry at a path.
    pub fn node(&self, path: &[u8]) -> Option<&Node> {
        self.nodes.get(path)
    }

    /// Every path it holds, in order.
    pub fn paths(&self) -> Vec<Vec<u8>> {
        self.nodes.keys().cloned().collect()
    }

    fn children(&self, path: &[u8]) -> Vec<Vec<u8>> {
        self.nodes
            .keys()
            .filter(|k| k.as_slice() != path && parent(k) == path)
            .cloned()
            .collect()
    }

    fn attrs(&self, path: &[u8]) -> Option<Attrs> {
        let node = self.nodes.get(path)?;
        let (size, mode) = match node {
            Node::Dir => (4096, DIR_MODE),
            Node::File(bytes) => (bytes.len() as u64, FILE_MODE),
            Node::Link(target) => (target.len() as u64, LINK_MODE),
        };
        let mode = self.modes.get(path).copied().unwrap_or(mode);
        Some(Attrs {
            size: Some(size),
            uid_gid: Some((1000, 1000)),
            permissions: Some(mode),
            times: Some(
                self.times
                    .get(path)
                    .copied()
                    .unwrap_or((1_700_000_000, 1_700_000_000)),
            ),
            extended: Vec::new(),
        })
    }

    /// Follows links, at most eight deep.
    fn follow(&self, path: &[u8]) -> Vec<u8> {
        let mut at = path.to_vec();
        for _ in 0..8 {
            match self.nodes.get(&at) {
                Some(Node::Link(target)) => at = normal(parent(&at), target),
                _ => break,
            }
        }
        at
    }
}

/// The faults a test asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    /// Stop answering after this many requests (input is still read).
    pub stall_after: Option<usize>,
    /// A READ answers at most this many bytes, whatever was asked.
    pub short_reads: Option<usize>,
    /// Answers are held back and sent in reverse order, a few at a time.
    pub out_of_order: bool,
    /// Every answer to a READ is sent twice, the second with the id again.
    pub reuse_ids: bool,
    /// The stream ends after this many bytes of answers, mid-packet.
    pub drop_after_bytes: Option<usize>,
    /// The server does not offer `statvfs@openssh.com`.
    pub no_statvfs: bool,
    /// The server does not offer `posix-rename@openssh.com`.
    pub no_posix_rename: bool,
    /// The server offers `copy-data` (OpenSSH 9.0 and later).
    pub copy_data: bool,
    /// A folder's first READDIR also names `.`, `..` twice over, `a/b`,
    /// `/etc/passwd`, a name with a NUL and an empty name, as a hostile
    /// server might.
    pub hostile_names: bool,
    /// A WRITE keeps at most this many of its bytes and still answers OK,
    /// as a server that lies does.
    pub short_writes: Option<usize>,
    /// The WRITE of this number, counted from 1, answers a failure: "No
    /// space left on device".
    pub fail_write_at: Option<usize>,
    /// Closing a file opened for writing answers a failure: "Disk quota
    /// exceeded"; the bytes stay.
    pub fail_close: bool,
}

/// The names a hostile server adds to a listing.
pub const HOSTILE_NAMES: [&[u8]; 8] = [
    b".",
    b"..",
    b".",
    b"..",
    b"a/b",
    b"/etc/passwd",
    b"nul\0name",
    b"",
];

/// The extensions the fake offers in its VERSION answer.
pub fn extensions(faults: &Faults) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = vec![
        (b"limits@openssh.com".to_vec(), b"1".to_vec()),
        (b"fsync@openssh.com".to_vec(), b"1".to_vec()),
    ];
    if !faults.no_posix_rename {
        out.insert(0, (b"posix-rename@openssh.com".to_vec(), b"1".to_vec()));
    }
    if !faults.no_statvfs {
        out.push((b"statvfs@openssh.com".to_vec(), b"2".to_vec()));
    }
    if faults.copy_data {
        out.push((b"copy-data".to_vec(), b"1".to_vec()));
    }
    out
}

/// The limits the fake reports.
pub const LIMITS: Limits = Limits {
    max_packet: 256 * 1024,
    max_read: 255 * 1024,
    max_write: 255 * 1024,
    max_open_handles: 64,
};

enum Open {
    File { path: Vec<u8>, write: bool },
    Dir { names: Vec<Vec<u8>>, at: usize },
}

struct Server {
    fs: Arc<Mutex<FakeFs>>,
    faults: Faults,
    handles: HashMap<Vec<u8>, Open>,
    next_handle: u64,
    writes: usize,
}

fn status(id: u32, code: u32, words: &str) -> Response {
    Response::Status {
        id,
        code,
        message: words.as_bytes().to_vec(),
        language: Vec::new(),
    }
}

impl Server {
    fn handle(&mut self, open: Open) -> Vec<u8> {
        self.next_handle += 1;
        let handle = format!("h{}", self.next_handle).into_bytes();
        self.handles.insert(handle.clone(), open);
        handle
    }

    fn answer(&mut self, request: Request) -> Response {
        let fs = Arc::clone(&self.fs);
        let mut fs = fs.lock().unwrap_or_else(PoisonError::into_inner);
        fs.calls.push(request.name());
        let home = fs.home.clone();
        let hostile = self.faults.hostile_names;
        let path = |p: &[u8]| normal(&home, p);
        match request {
            Request::Init { .. } => Response::Version {
                version: VERSION,
                extensions: extensions(&self.faults),
            },
            Request::Realpath { id, path: p } => Response::Name {
                id,
                entries: vec![NameEntry {
                    filename: path(&p),
                    longname: Vec::new(),
                    attrs: Attrs::default(),
                }],
            },
            Request::Stat { id, path: p } => {
                let at = fs.follow(&path(&p));
                match fs.attrs(&at) {
                    Some(attrs) => Response::Attrs { id, attrs },
                    None => status(id, status::NO_SUCH_FILE, "No such file"),
                }
            }
            Request::Lstat { id, path: p } => match fs.attrs(&path(&p)) {
                Some(attrs) => Response::Attrs { id, attrs },
                None => status(id, status::NO_SUCH_FILE, "No such file"),
            },
            Request::Fstat { id, handle } => match self.handles.get(&handle) {
                Some(Open::File { path: p, .. }) => match fs.attrs(p) {
                    Some(attrs) => Response::Attrs { id, attrs },
                    None => status(id, status::FAILURE, "Gone"),
                },
                _ => status(id, status::FAILURE, "Bad handle"),
            },
            Request::Opendir { id, path: p } => {
                let at = fs.follow(&path(&p));
                match fs.node(&at) {
                    Some(Node::Dir) => {
                        let names = fs.children(&at);
                        drop(fs);
                        let handle = self.handle(Open::Dir { names, at: 0 });
                        Response::Handle { id, handle }
                    }
                    Some(_) => status(id, status::FAILURE, "Not a directory"),
                    None => status(id, status::NO_SUCH_FILE, "No such file"),
                }
            }
            Request::Readdir { id, handle } => match self.handles.get_mut(&handle) {
                Some(Open::Dir { names, at }) => {
                    if *at >= names.len() {
                        return status(id, status::EOF, "End of file");
                    }
                    let end = (*at + BATCH).min(names.len());
                    let mut entries: Vec<NameEntry> = names[*at..end]
                        .iter()
                        .map(|full| NameEntry {
                            filename: name(full).to_vec(),
                            longname: b"-rw-r--r-- not to be trusted".to_vec(),
                            attrs: fs.attrs(full).unwrap_or_default(),
                        })
                        .collect();
                    if hostile && *at == 0 {
                        entries.extend(HOSTILE_NAMES.iter().map(|n| NameEntry {
                            filename: n.to_vec(),
                            longname: Vec::new(),
                            attrs: Attrs {
                                permissions: Some(FILE_MODE),
                                ..Attrs::default()
                            },
                        }));
                    }
                    *at = end;
                    Response::Name { id, entries }
                }
                _ => status(id, status::FAILURE, "Bad handle"),
            },
            Request::Open {
                id,
                path: p,
                pflags,
                ..
            } => {
                let at = path(&p);
                let exists = fs.node(&at).is_some();
                if matches!(fs.node(&at), Some(Node::Dir)) {
                    return status(id, status::FAILURE, "Is a directory");
                }
                if pflags & open::CREAT != 0 {
                    if exists && pflags & open::EXCL != 0 {
                        return status(id, status::FAILURE, "File exists");
                    }
                    if !matches!(fs.node(parent(&at)), Some(Node::Dir)) {
                        return status(id, status::NO_SUCH_FILE, "No such file");
                    }
                    if !exists || pflags & open::TRUNC != 0 {
                        fs.nodes.insert(at.clone(), Node::File(Vec::new()));
                    }
                } else if !exists {
                    return status(id, status::NO_SUCH_FILE, "No such file");
                }
                drop(fs);
                let handle = self.handle(Open::File {
                    path: at,
                    write: pflags & open::WRITE != 0,
                });
                Response::Handle { id, handle }
            }
            Request::Read {
                id,
                handle,
                offset,
                len,
            } => match self.handles.get(&handle) {
                Some(Open::File { path: p, .. }) => {
                    let Some(bytes) = fs.file(p) else {
                        return status(id, status::FAILURE, "Gone");
                    };
                    let start = usize::try_from(offset).unwrap_or(usize::MAX);
                    if start >= bytes.len() {
                        return status(id, status::EOF, "End of file");
                    }
                    let mut take = len as usize;
                    if let Some(short) = self.faults.short_reads {
                        take = take.min(short);
                    }
                    let end = start.saturating_add(take).min(bytes.len());
                    Response::Data {
                        id,
                        data: bytes[start..end].to_vec(),
                    }
                }
                _ => status(id, status::FAILURE, "Bad handle"),
            },
            Request::Write {
                id,
                handle,
                offset,
                data,
            } => match self.handles.get(&handle) {
                Some(Open::File {
                    path: p,
                    write: true,
                }) => {
                    let p = p.clone();
                    self.writes += 1;
                    if self.faults.fail_write_at == Some(self.writes) {
                        return status(id, status::FAILURE, "No space left on device");
                    }
                    let Some(Node::File(bytes)) = fs.nodes.get_mut(&p) else {
                        return status(id, status::FAILURE, "Gone");
                    };
                    let data = match self.faults.short_writes {
                        Some(keep) => &data[..keep.min(data.len())],
                        None => &data[..],
                    };
                    let start = usize::try_from(offset).unwrap_or(usize::MAX);
                    if start > bytes.len() {
                        bytes.resize(start, 0);
                    }
                    let end = start + data.len();
                    if end > bytes.len() {
                        bytes.resize(end, 0);
                    }
                    bytes[start..end].copy_from_slice(data);
                    status(id, status::OK, "Success")
                }
                Some(_) => status(id, status::PERMISSION_DENIED, "Not open for writing"),
                None => status(id, status::FAILURE, "Bad handle"),
            },
            Request::Close { id, handle } => match self.handles.remove(&handle) {
                Some(Open::File { write: true, .. }) if self.faults.fail_close => {
                    status(id, status::FAILURE, "Disk quota exceeded")
                }
                Some(_) => status(id, status::OK, "Success"),
                None => status(id, status::FAILURE, "Bad handle"),
            },
            Request::Setstat { id, path: p, attrs } => {
                let at = path(&p);
                if fs.node(&at).is_none() {
                    return status(id, status::NO_SUCH_FILE, "No such file");
                }
                if let Some(mode) = attrs.permissions {
                    let kind = fs.attrs(&at).and_then(|a| a.permissions).unwrap_or(0) & 0o170_000;
                    fs.modes.insert(at.clone(), kind | (mode & 0o7777));
                }
                if let Some(times) = attrs.times {
                    fs.times.insert(at, times);
                }
                status(id, status::OK, "Success")
            }
            Request::Fsetstat { id, handle, attrs } => {
                // A size cuts or grows the open file, as ftruncate does.
                if let (Some(size), Some(Open::File { path: p, .. })) =
                    (attrs.size, self.handles.get(&handle))
                    && let Some(Node::File(bytes)) = fs.nodes.get_mut(p)
                {
                    bytes.resize(usize::try_from(size).unwrap_or(usize::MAX), 0);
                }
                status(id, status::OK, "Success")
            }
            Request::Remove { id, path: p } => {
                let at = path(&p);
                match fs.node(&at) {
                    Some(Node::File(_) | Node::Link(_)) => {
                        fs.nodes.remove(&at);
                        status(id, status::OK, "Success")
                    }
                    Some(Node::Dir) => status(id, status::FAILURE, "Is a directory"),
                    None => status(id, status::NO_SUCH_FILE, "No such file"),
                }
            }
            Request::Mkdir { id, path: p, .. } => {
                let at = path(&p);
                if fs.node(&at).is_some() {
                    // OpenSSH answers a plain failure here.
                    return status(id, status::FAILURE, "Failure");
                }
                if !matches!(fs.node(parent(&at)), Some(Node::Dir)) {
                    return status(id, status::NO_SUCH_FILE, "No such file");
                }
                fs.nodes.insert(at, Node::Dir);
                status(id, status::OK, "Success")
            }
            Request::Rmdir { id, path: p } => {
                let at = path(&p);
                match fs.node(&at) {
                    Some(Node::Dir) if fs.children(&at).is_empty() => {
                        fs.nodes.remove(&at);
                        status(id, status::OK, "Success")
                    }
                    Some(Node::Dir) => status(id, status::FAILURE, "Failure"),
                    Some(_) => status(id, status::FAILURE, "Not a directory"),
                    None => status(id, status::NO_SUCH_FILE, "No such file"),
                }
            }
            Request::Rename { id, from, to } => {
                let (from, to) = (path(&from), path(&to));
                if fs.node(&to).is_some() {
                    return status(id, status::FAILURE, "Failure");
                }
                rename(&mut fs, &from, &to, id)
            }
            Request::Readlink { id, path: p } => match fs.node(&path(&p)) {
                Some(Node::Link(target)) => Response::Name {
                    id,
                    entries: vec![NameEntry {
                        filename: target.clone(),
                        longname: Vec::new(),
                        attrs: Attrs::default(),
                    }],
                },
                _ => status(id, status::FAILURE, "Not a link"),
            },
            Request::Symlink { id, first, second } => {
                // OpenSSH's order: target, then link path.
                let link = path(&second);
                if fs.node(&link).is_some() {
                    return status(id, status::FAILURE, "Failure");
                }
                fs.add_link(&link, &first);
                status(id, status::OK, "Success")
            }
            Request::Extended {
                id,
                name: ext,
                data,
            } => self.extended(&mut fs, id, &ext, &data, &home),
        }
    }

    fn extended(&self, fs: &mut FakeFs, id: u32, ext: &[u8], data: &[u8], home: &[u8]) -> Response {
        match ext {
            b"posix-rename@openssh.com" if !self.faults.no_posix_rename => {
                match read_strings(data, 2) {
                    Ok(parts) => {
                        let (from, to) = (normal(home, &parts[0]), normal(home, &parts[1]));
                        if matches!(fs.node(&to), Some(Node::Dir)) {
                            return status(id, status::FAILURE, "Failure");
                        }
                        fs.nodes.remove(&to);
                        rename(fs, &from, &to, id)
                    }
                    Err(_) => status(id, status::BAD_MESSAGE, "Bad message"),
                }
            }
            b"statvfs@openssh.com" if !self.faults.no_statvfs => {
                let used: u64 = fs
                    .nodes
                    .values()
                    .map(|n| match n {
                        Node::File(b) => b.len() as u64,
                        _ => 0,
                    })
                    .sum();
                let blocks = 1_000_000u64;
                let free = blocks.saturating_sub(used.div_ceil(4096));
                Response::ExtendedReply {
                    id,
                    data: StatVfs {
                        bsize: 4096,
                        frsize: 4096,
                        blocks,
                        bfree: free,
                        bavail: free.saturating_sub(1000),
                        files: 100_000,
                        ffree: 90_000,
                        favail: 90_000,
                        fsid: 7,
                        flag: 0,
                        namemax: 255,
                    }
                    .encode(),
                }
            }
            b"limits@openssh.com" => Response::ExtendedReply {
                id,
                data: LIMITS.encode(),
            },
            b"fsync@openssh.com" => status(id, status::OK, "Success"),
            _ => status(id, status::OP_UNSUPPORTED, "Unsupported"),
        }
    }
}

fn rename(fs: &mut FakeFs, from: &[u8], to: &[u8], id: u32) -> Response {
    if fs.node(from).is_none() {
        return status(id, status::NO_SUCH_FILE, "No such file");
    }
    let moved: Vec<Vec<u8>> = fs
        .nodes
        .keys()
        .filter(|k| {
            k.as_slice() == from || (k.starts_with(from) && k.get(from.len()) == Some(&b'/'))
        })
        .cloned()
        .collect();
    for key in moved {
        if let Some(node) = fs.nodes.remove(&key) {
            let mut new = to.to_vec();
            new.extend_from_slice(&key[from.len()..]);
            fs.nodes.insert(new, node);
        }
    }
    status(id, status::OK, "Success")
}

/// Writes answers, cutting the stream after a byte budget when asked.
struct Out<W: Write> {
    out: W,
    budget: Option<usize>,
}

impl<W: Write> Out<W> {
    fn send(&mut self, response: &Response) -> io::Result<()> {
        let bytes = response.encode();
        match &mut self.budget {
            Some(left) if *left < bytes.len() => {
                let cut = *left;
                self.out.write_all(&bytes[..cut])?;
                self.out.flush()?;
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "dropped on purpose",
                ))
            }
            Some(left) => {
                *left -= bytes.len();
                self.out.write_all(&bytes)?;
                self.out.flush()
            }
            None => {
                self.out.write_all(&bytes)?;
                self.out.flush()
            }
        }
    }
}

/// Reads requests on a thread of its own and hands them on.
fn read_requests(mut input: impl Read + Send + 'static) -> Receiver<Request> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            while let Ok(Some((body, used))) = frame(&buffer, DEFAULT_MAX_PACKET) {
                let request = Request::decode(body);
                buffer.drain(..used);
                match request {
                    Ok(request) => {
                        if tx.send(request).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            if frame(&buffer, DEFAULT_MAX_PACKET).is_err() {
                return;
            }
            match input.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            }
        }
    });
    rx
}

/// Serves one session until the input ends, the stream drops on purpose,
/// or the output fails.
pub fn serve(
    input: impl Read + Send + 'static,
    output: impl Write,
    fs: Arc<Mutex<FakeFs>>,
    faults: Faults,
) -> io::Result<()> {
    let requests = read_requests(input);
    let mut out = Out {
        out: output,
        budget: faults.drop_after_bytes,
    };
    let mut server = Server {
        fs,
        faults: faults.clone(),
        handles: HashMap::new(),
        next_handle: 0,
        writes: 0,
    };
    let mut answered = 0usize;
    let mut held: Vec<Response> = Vec::new();
    loop {
        let request = if faults.out_of_order {
            match requests.recv_timeout(Duration::from_millis(30)) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    for response in held.drain(..).rev() {
                        out.send(&response)?;
                    }
                    return Ok(());
                }
            }
        } else {
            match requests.recv() {
                Ok(request) => Some(request),
                Err(_) => return Ok(()),
            }
        };
        let Some(request) = request else {
            // Nothing more is coming for now: what is held goes out,
            // last first.
            for response in held.drain(..).rev() {
                out.send(&response)?;
            }
            continue;
        };
        let is_init = matches!(request, Request::Init { .. });
        let is_read = matches!(request, Request::Read { .. });
        let response = server.answer(request);
        if !is_init && faults.stall_after.is_some_and(|n| answered >= n) {
            continue;
        }
        if !is_init {
            answered += 1;
        }
        if faults.out_of_order && !is_init {
            held.push(response);
            if held.len() >= 4 {
                for response in held.drain(..).rev() {
                    out.send(&response)?;
                }
            }
            continue;
        }
        out.send(&response)?;
        if faults.reuse_ids && is_read {
            out.send(&response)?;
        }
    }
}

/// One end of an in-memory duplex stream.
#[derive(Debug)]
pub struct End {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    buffer: Vec<u8>,
}

/// Two ends of a stream in memory: what one writes the other reads; an
/// end dropped ends the other's reading.
pub fn duplex() -> (End, End) {
    let (a_tx, a_rx) = mpsc::channel();
    let (b_tx, b_rx) = mpsc::channel();
    (
        End {
            tx: a_tx,
            rx: b_rx,
            buffer: Vec::new(),
        },
        End {
            tx: b_tx,
            rx: a_rx,
            buffer: Vec::new(),
        },
    )
}

impl End {
    /// Splits the end into its reading and writing halves.
    pub fn split(self) -> (EndReader, EndWriter) {
        (
            EndReader {
                rx: self.rx,
                buffer: self.buffer,
            },
            EndWriter { tx: self.tx },
        )
    }
}

/// The reading half of an [`End`].
#[derive(Debug)]
pub struct EndReader {
    rx: Receiver<Vec<u8>>,
    buffer: Vec<u8>,
}

/// The writing half of an [`End`].
#[derive(Clone, Debug)]
pub struct EndWriter {
    tx: Sender<Vec<u8>>,
}

fn read_from(rx: &Receiver<Vec<u8>>, buffer: &mut Vec<u8>, out: &mut [u8]) -> io::Result<usize> {
    if buffer.is_empty() {
        match rx.recv() {
            Ok(bytes) => *buffer = bytes,
            Err(_) => return Ok(0),
        }
    }
    let n = out.len().min(buffer.len());
    out[..n].copy_from_slice(&buffer[..n]);
    buffer.drain(..n);
    Ok(n)
}

impl Read for End {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        read_from(&self.rx, &mut self.buffer, out)
    }
}

impl Write for End {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.tx
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the other end is gone"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for EndReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        read_from(&self.rx, &mut self.buffer, out)
    }
}

impl Write for EndWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.tx
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the other end is gone"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Starts a fake server on a thread, serving `fs` with `faults`, and gives
/// the client's end.
pub fn spawn(fs: Arc<Mutex<FakeFs>>, faults: Faults) -> (EndReader, EndWriter) {
    let (client, server) = duplex();
    let (server_in, server_out) = server.split();
    thread::spawn(move || {
        let _ = serve(server_in, server_out, fs, faults);
    });
    client.split()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_made_absolute_and_plain() {
        assert_eq!(normal(b"/home/ada", b"."), b"/home/ada");
        assert_eq!(normal(b"/home/ada", b"docs/../a.txt"), b"/home/ada/a.txt");
        assert_eq!(normal(b"/home/ada", b"/srv//x/"), b"/srv/x");
        assert_eq!(normal(b"/home/ada", b"/.."), b"/");
        assert_eq!(parent(b"/a/b"), b"/a");
        assert_eq!(parent(b"/a"), b"/");
        assert_eq!(name(b"/a/b.txt"), b"b.txt");
    }
}
