//! The toy provider: a file system in memory, `memfs://box/...`, that the
//! fake plugin serves in its provider mode. It is the subject of the
//! provider contract list (R22, section 8.3): listings in one answer or in
//! partial batches, ranged reads, write sessions that commit or leave
//! nothing, folders, renames, removes, and a sign-in that fails first;
//! and, for the provider plugins of stage 2 (R25's commit 46), its
//! volumes with and without a place, the places it offers, an authority
//! it refuses as untrusted with a hint (`memfs://changed`), and writes
//! that resume from a partial file. Its state can outlive a plugin
//! process, so a test can kill the plugin in the middle of a write and see
//! that nothing was left.

use std::collections::{BTreeMap, HashMap};

use diroxide_core::{EntryKind, Place};
use serde_json::{Value, json};

use crate::base64;
use crate::protocol::{ErrorKind, Params, RpcError};
use crate::shapes::WireEntry;

/// The scheme the toy serves.
pub const SCHEME: &str = "memfs";

/// The methods it implements, without the `provider/` prefix.
pub const METHODS: &[&str] = &[
    "connect",
    "list",
    "stat",
    "read",
    "write_begin",
    "write_chunk",
    "write_end",
    "create_dir",
    "rename",
    "remove",
    "volumes",
    "places",
];

/// The authority the toy refuses as untrusted, as a changed host key is.
pub const UNTRUSTED: &str = "changed";

/// The toy's size in bytes; what its files hold is taken from it.
pub const SIZE: u64 = 1_000_000;

/// How many entries go in one partial batch.
pub const BATCH: usize = 1000;

/// One entry of the toy.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Folder,
    File(Vec<u8>),
}

#[derive(Clone, Debug)]
struct Session {
    path: String,
    bytes: Vec<u8>,
    overwrite: bool,
}

/// What an interrupted write left: its bytes and the fingerprint of the
/// source it came from.
#[derive(Clone, Debug)]
struct Partial {
    bytes: Vec<u8>,
    fingerprint: Option<String>,
}

/// A file system in memory.
#[derive(Debug)]
pub struct MemFs {
    nodes: BTreeMap<String, Node>,
    writes: HashMap<String, Session>,
    partials: HashMap<String, Partial>,
    next: u64,
    refuse_connects: usize,
    space_known: bool,
    /// Every method called, in order, for the tests.
    pub calls: Vec<String>,
}

impl Default for MemFs {
    fn default() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert("/".to_owned(), Node::Folder);
        Self {
            nodes,
            writes: HashMap::new(),
            partials: HashMap::new(),
            next: 1,
            refuse_connects: 0,
            space_known: true,
            calls: Vec::new(),
        }
    }
}

fn error(kind: ErrorKind, message: &str) -> RpcError {
    RpcError::new(kind, message)
}

fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) => "/".to_owned(),
        Some((parent, _)) => parent.to_owned(),
        None => "/".to_owned(),
    }
}

fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn bad_name(name: &str) -> bool {
    name.is_empty() || name.contains([':', '*', '?', '"', '<', '>', '|', '\\'])
}

impl MemFs {
    /// The toy with a sign-in that is refused `times` times first.
    pub fn refusing_connects(times: usize) -> Self {
        Self {
            refuse_connects: times,
            ..Self::default()
        }
    }

    /// The toy with volumes that do not know their free space.
    pub fn without_free_space(self) -> Self {
        Self {
            space_known: false,
            ..self
        }
    }

    /// Leaves a partial file at a path, as an interrupted write does, with
    /// the fingerprint of its source.
    pub fn add_partial(&mut self, path: &str, bytes: &[u8], fingerprint: Option<&str>) {
        self.partials.insert(
            path.to_owned(),
            Partial {
                bytes: bytes.to_vec(),
                fingerprint: fingerprint.map(str::to_owned),
            },
        );
    }

    /// The partial file at a path, if one is left.
    pub fn partial(&self, path: &str) -> Option<&[u8]> {
        self.partials.get(path).map(|p| p.bytes.as_slice())
    }

    /// Adds a folder and its parents.
    pub fn add_folder(&mut self, path: &str) {
        let mut at = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            at.push('/');
            at.push_str(part);
            self.nodes.insert(at.clone(), Node::Folder);
        }
    }

    /// Adds a file, with its folder.
    pub fn add_file(&mut self, path: &str, bytes: &[u8]) {
        self.add_folder(&parent_of(path));
        self.nodes
            .insert(path.to_owned(), Node::File(bytes.to_vec()));
    }

    /// A file's bytes, if it exists.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        match self.nodes.get(path) {
            Some(Node::File(bytes)) => Some(bytes),
            _ => None,
        }
    }

    /// Forgets the open write sessions, as a plugin's process does when it
    /// stops: the files stay, the sessions die with the process.
    pub fn forget_sessions(&mut self) {
        self.writes.clear();
    }

    /// How many write sessions are open.
    pub fn open_writes(&self) -> usize {
        self.writes.len()
    }

    fn path(params: &Params, key: &str) -> Result<String, RpcError> {
        let text = params
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| error(ErrorKind::InvalidParams, "a place is missing"))?;
        let place = Place::parse(text)
            .map_err(|_| error(ErrorKind::InvalidParams, "the place cannot be read"))?;
        let remote = place
            .remote()
            .filter(|r| r.scheme() == SCHEME)
            .ok_or_else(|| error(ErrorKind::InvalidParams, "not a memfs place"))?;
        let path = remote.path().trim_end_matches('/');
        Ok(if path.is_empty() {
            "/".to_owned()
        } else {
            path.to_owned()
        })
    }

    fn authority(params: &Params, key: &str) -> Option<String> {
        let text = params.get(key).and_then(Value::as_str)?;
        Place::parse(text)
            .ok()?
            .remote()
            .map(|r| r.host().to_owned())
    }

    fn used(&self) -> u64 {
        self.nodes
            .values()
            .map(|node| match node {
                Node::File(bytes) => bytes.len() as u64,
                Node::Folder => 0,
            })
            .sum()
    }

    fn volume(&self, name: &str, place: &str) -> Value {
        if self.space_known {
            json!({"name": name, "place": place, "free": SIZE.saturating_sub(self.used()),
                "total": SIZE})
        } else {
            json!({"name": name, "place": place})
        }
    }

    fn entry(&self, path: &str) -> Option<WireEntry> {
        let node = self.nodes.get(path)?;
        let (kind, size) = match node {
            Node::Folder => (EntryKind::Folder, None),
            Node::File(bytes) => (EntryKind::File, Some(bytes.len() as u64)),
        };
        Some(WireEntry {
            name: name_of(path).to_owned(),
            name_raw: None,
            place: Some(crate::shapes::WirePlace(format!("{SCHEME}://box{path}"))),
            kind,
            size,
            modified: None,
            mode: None,
            attributes: Vec::new(),
            owner: None,
            link_target: None,
            mime: None,
            revision: None,
            extra: BTreeMap::new(),
        })
    }

    fn children(&self, path: &str) -> Vec<String> {
        let prefix = if path == "/" {
            "/".to_owned()
        } else {
            format!("{path}/")
        };
        self.nodes
            .keys()
            .filter(|key| {
                key.as_str() != "/"
                    && key.starts_with(&prefix)
                    && !key[prefix.len()..].contains('/')
            })
            .cloned()
            .collect()
    }

    fn needs_folder(&self, path: &str) -> Result<(), RpcError> {
        match self.nodes.get(path) {
            Some(Node::Folder) => Ok(()),
            Some(Node::File(_)) => Err(error(ErrorKind::NotADirectory, "not a folder")),
            None => Err(error(ErrorKind::NotFound, "no such folder")),
        }
    }

    /// Serves one provider method; partial batches go to `partials` as
    /// `$/progress` values for the token.
    pub fn serve(
        &mut self,
        method: &str,
        params: &Params,
        partials: &mut Vec<Value>,
    ) -> Result<Value, RpcError> {
        self.calls.push(method.to_owned());
        match method {
            "connect" => {
                if Self::authority(params, "place").as_deref() == Some(UNTRUSTED) {
                    return Err(RpcError::with_hint(
                        ErrorKind::Untrusted,
                        "Host key changed. Not connected.",
                        "To forget the old key, run:\nssh-keygen -R changed",
                    ));
                }
                if self.refuse_connects > 0 {
                    self.refuse_connects -= 1;
                    return Err(RpcError::with_hint(
                        ErrorKind::AuthRequired,
                        "the password was refused",
                        "Type the password again.",
                    ));
                }
                Ok(json!({"display_name": "Memory box", "root": format!("{SCHEME}://box/")}))
            }
            "list" => {
                let path = Self::path(params, "place")?;
                self.needs_folder(&path)?;
                let entries: Vec<WireEntry> = self
                    .children(&path)
                    .iter()
                    .filter_map(|child| self.entry(child))
                    .collect();
                match params.get("partial_token") {
                    Some(_) if entries.len() > BATCH => {
                        for batch in entries.chunks(BATCH) {
                            partials.push(json!({"kind": "partial", "entries": batch}));
                        }
                        Ok(json!({"complete": true}))
                    }
                    _ => Ok(json!({"entries": entries})),
                }
            }
            "stat" => {
                let path = Self::path(params, "place")?;
                self.entry(&path)
                    .map(|e| json!(e))
                    .ok_or_else(|| error(ErrorKind::NotFound, "no such entry"))
            }
            "read" => {
                let path = Self::path(params, "place")?;
                let bytes = match self.nodes.get(&path) {
                    Some(Node::File(bytes)) => bytes,
                    Some(Node::Folder) => return Err(error(ErrorKind::IsADirectory, "a folder")),
                    None => return Err(error(ErrorKind::NotFound, "no such file")),
                };
                let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let length = params
                    .get("length")
                    .and_then(Value::as_u64)
                    .unwrap_or(1 << 20) as usize;
                let start = offset.min(bytes.len());
                let end = start.saturating_add(length).min(bytes.len());
                Ok(json!({"data": base64::encode(&bytes[start..end]), "eof": end >= bytes.len()}))
            }
            "write_begin" => {
                let path = Self::path(params, "place")?;
                if bad_name(name_of(&path)) {
                    return Err(error(
                        ErrorKind::InvalidName,
                        "the name has a reserved character",
                    ));
                }
                self.needs_folder(&parent_of(&path))?;
                let overwrite = params.get("conflict").and_then(Value::as_str) == Some("overwrite");
                match self.nodes.get(&path) {
                    Some(Node::Folder) => return Err(error(ErrorKind::IsADirectory, "a folder")),
                    Some(Node::File(_)) if !overwrite => {
                        return Err(error(ErrorKind::AlreadyExists, "the file exists"));
                    }
                    _ => {}
                }
                // A resume goes on from the partial file when it is from the
                // same source and holds at least what the host asked for;
                // otherwise it starts over at 0.
                let resume = params.get("resume");
                let (offset, bytes) = match (resume, self.partials.get(&path)) {
                    (Some(asked), Some(partial)) => {
                        let at = asked.get("offset").and_then(Value::as_u64).unwrap_or(0);
                        let same = asked.get("fingerprint").and_then(Value::as_str)
                            == partial.fingerprint.as_deref();
                        if same && at <= partial.bytes.len() as u64 {
                            (at, partial.bytes[..at as usize].to_vec())
                        } else {
                            (0, Vec::new())
                        }
                    }
                    _ => (0, Vec::new()),
                };
                let id = format!("w{}", self.next);
                self.next += 1;
                self.writes.insert(
                    id.clone(),
                    Session {
                        path,
                        bytes,
                        overwrite,
                    },
                );
                Ok(json!({"write_id": id, "offset": resume.map(|_| offset)}))
            }
            "write_chunk" => {
                let id = params.get("write_id").and_then(Value::as_str).unwrap_or("");
                let session = self
                    .writes
                    .get_mut(id)
                    .ok_or_else(|| error(ErrorKind::NotFound, "no such write"))?;
                let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
                if offset != session.bytes.len() as u64 {
                    return Err(error(ErrorKind::Conflict, "a chunk came out of order"));
                }
                let data = params
                    .get("data")
                    .and_then(Value::as_str)
                    .and_then(base64::decode)
                    .ok_or_else(|| error(ErrorKind::InvalidParams, "data is not base64"))?;
                session.bytes.extend_from_slice(&data);
                Ok(json!({"length": session.bytes.len()}))
            }
            "write_end" => {
                let id = params.get("write_id").and_then(Value::as_str).unwrap_or("");
                let session = self
                    .writes
                    .remove(id)
                    .ok_or_else(|| error(ErrorKind::NotFound, "no such write"))?;
                if params.get("commit").and_then(Value::as_bool) != Some(true) {
                    return Ok(Value::Null);
                }
                if !session.overwrite && self.nodes.contains_key(&session.path) {
                    return Err(error(ErrorKind::AlreadyExists, "the file exists"));
                }
                let path = session.path.clone();
                self.partials.remove(&path);
                self.nodes.insert(path.clone(), Node::File(session.bytes));
                Ok(json!(self.entry(&path)))
            }
            "create_dir" => {
                let path = Self::path(params, "place")?;
                if bad_name(name_of(&path)) {
                    return Err(error(
                        ErrorKind::InvalidName,
                        "the name has a reserved character",
                    ));
                }
                if self.nodes.contains_key(&path) {
                    return Err(error(ErrorKind::AlreadyExists, "it exists"));
                }
                if params.get("parents").and_then(Value::as_bool) == Some(true) {
                    self.add_folder(&path);
                } else {
                    self.needs_folder(&parent_of(&path))?;
                    self.nodes.insert(path, Node::Folder);
                }
                Ok(Value::Null)
            }
            "rename" => {
                let from = Self::path(params, "from")?;
                let to = Self::path(params, "to")?;
                if !self.nodes.contains_key(&from) {
                    return Err(error(ErrorKind::NotFound, "no such entry"));
                }
                if bad_name(name_of(&to)) {
                    return Err(error(
                        ErrorKind::InvalidName,
                        "the name has a reserved character",
                    ));
                }
                if self.nodes.contains_key(&to)
                    && params.get("overwrite").and_then(Value::as_bool) != Some(true)
                {
                    return Err(error(ErrorKind::AlreadyExists, "the name is taken"));
                }
                self.needs_folder(&parent_of(&to))?;
                let moved: Vec<String> = self
                    .nodes
                    .keys()
                    .filter(|k| **k == from || k.starts_with(&format!("{from}/")))
                    .cloned()
                    .collect();
                for key in moved {
                    if let Some(node) = self.nodes.remove(&key) {
                        let new = format!("{to}{}", &key[from.len()..]);
                        self.nodes.insert(new, node);
                    }
                }
                Ok(Value::Null)
            }
            "remove" => {
                let path = Self::path(params, "place")?;
                match self.nodes.get(&path) {
                    None => return Err(error(ErrorKind::NotFound, "no such entry")),
                    Some(Node::Folder) if !self.children(&path).is_empty() => {
                        return Err(error(ErrorKind::NotEmpty, "the folder is not empty"));
                    }
                    _ => {}
                }
                self.nodes.remove(&path);
                Ok(Value::Null)
            }
            "volumes" => {
                if params.contains_key("place") {
                    Self::path(params, "place")?;
                    Ok(json!({"volumes": [self.volume("Memory box", "memfs://box/")]}))
                } else {
                    Ok(json!({"volumes": [
                        self.volume("Memory box", "memfs://box/"),
                        self.volume("Spare shelf", "memfs://spare/"),
                        // Not a place of this provider: the host leaves it out.
                        {"name": "Elsewhere", "place": "/elsewhere"},
                    ]}))
                }
            }
            "places" => Ok(json!({"places": [
                {"name": "box", "place": "memfs://box/", "state": "connected",
                    "detail": "Memory box"},
                {"name": "spare", "place": "memfs://spare/", "state": "saved"},
                // Left out by the host, and counted: a place that is not
                // one of this provider's, a bare drive, an empty state.
                {"name": "other", "place": "/home/other", "state": "saved"},
                {"name": "drive", "place": "C:", "state": "saved"},
                {"name": "blank", "place": "memfs://blank/", "state": "\u{200B}"},
            ]})),
            other => Err(error(
                ErrorKind::MethodNotFound,
                &format!("no method provider/{other}"),
            )),
        }
    }
}
