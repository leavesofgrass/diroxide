//! A plugin's file system as a provider (R3, section 7): the bridge from
//! the [`Provider`] trait of `diroxide-vfs`, which the panels and the job
//! engine use, to `provider/*` requests on the plugin's connection.
//!
//! - **Declared, then confirmed.** A method the `[[provider]]` table does
//!   not list is unsupported without calling the plugin (the host's own
//!   -32017); `list` and `stat` are always there.
//! - **Places travel whole,** in the provider's own form
//!   (`memfs://box/docs`). Names in answers are checked: an empty name, a
//!   dot name or one with a separator is left out and logged, so a plugin
//!   cannot steer a listing outside its folder.
//! - **Listings stream.** The plugin may answer in one piece or in
//!   partial batches (`$/progress` with `kind = "partial"`);
//!   [`PluginProvider::as_streaming`] hands each batch to the caller as it
//!   arrives, so a large folder shows rows before it is complete, and
//!   [`Provider::list`] collects them.
//! - **Reads are ranged and stateless**, a chunk at a time, so a reader can
//!   seek (the archive reader on a plugin's place reads a zip's directory
//!   at its end without the rest).
//! - **Writes are sessions:** `write_begin`, chunks, then `write_end` with
//!   `commit` true; a writer dropped without its commit (a cancel, a
//!   failure) ends the session with `commit` false, and a plugin that
//!   stops mid-write is never told to commit, so nothing is left behind.
//!   A `resume` request carries the offset the host believes is there
//!   ([`PluginProvider::write_resuming`]); the plugin answers the offset
//!   it goes on from, 0 when it starts over, and the session's chunks
//!   start there.
//! - **Sign-in:** `provider/connect` once per authority before its first
//!   call; an `auth_required` answer to any call sends the host back to
//!   connect once, then the call is tried again.
//! - **Cancellation** is the operation's token: the request is cancelled
//!   with `$/cancel_request` and still answered.
//! - **Volumes and places:** `provider/volumes` lists an authority's
//!   storages, or with a place the one that holds it, and fills
//!   [`Provider::space`] and [`Provider::volume`]; a number the plugin
//!   does not give is not known, never zero. `provider/places` lists the
//!   places the plugin offers (its sites) as rows with a state word,
//!   without signing in. Both are cleaned and checked like listings.
//! - **An untrusted other side** (a changed host key) is refused with the
//!   plugin's hint kept: the next step in words, said and never run.

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use diroxide_core::{Attributes, Entry, Place};
use diroxide_vfs::provider::{LinkKind, LinkRequest};
use diroxide_vfs::{
    AttributeChange, Capabilities, CopyHint, FileWriter, OpContext, Provider, ReadSeek, SpaceInfo,
    StreamingList, TrashCheck, VfsError, VolumeInfo, VolumeKind, WatchHandle, WatchSink, WriteMode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::base64;
use crate::error::PluginError;
use crate::host::Host;
use crate::lines;
use crate::live::CallOptions;
use crate::manifest::ProviderSpec;
use crate::places::plugin_place;
use crate::protocol::ErrorKind;
use crate::services::clean_text;
use crate::shapes::{WireEntry, WirePlaceRow, WireVolume, format_time};

/// The size of one read or write chunk: 1 MiB, base64 on the wire.
pub const CHUNK: usize = 1024 * 1024;

/// `provider/write_begin`'s parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteBegin {
    /// The file.
    pub place: String,
    /// `fail` when the name is taken, or `overwrite`.
    pub conflict: String,
    /// How the bytes travel: `inline`.
    pub transfer: String,
    /// The size, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_hint: Option<u64>,
    /// A request to go on from where an earlier write stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeRequest>,
}

/// What an earlier, interrupted write left, for a resume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRequest {
    /// How many bytes the host believes are there.
    pub offset: u64,
    /// The source's fingerprint, so a changed source starts over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// `provider/write_begin`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteBegun {
    /// The session's id.
    pub write_id: String,
    /// Where the next chunk goes: the resumed offset, or 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

/// One storage of a plugin's provider, read from `provider/volumes` and
/// checked: the name cleaned, the root parsed like any place a plugin
/// names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeRow {
    /// Its name, cleaned, meaning first.
    pub name: String,
    /// Its root.
    pub place: Place,
    /// The bytes the user may still write; `None` is not known.
    pub free: Option<u64>,
    /// Its size; `None` is not known.
    pub total: Option<u64>,
    /// Whether it can be unplugged.
    pub removable: bool,
}

impl VolumeRow {
    /// The row in words, meaning first: "Home on build.example, 12.0 GB
    /// free of 50.0 GB", or "..., free space not known".
    pub fn words(&self) -> String {
        lines::volume_row(&self.name, self.free, self.total)
    }

    /// The free space and size, for the copy plan's room check.
    pub fn space(&self) -> SpaceInfo {
        SpaceInfo {
            free_to_user: self.free,
            total: self.total,
            max_file_size: None,
        }
    }
}

/// One place a plugin offers, read from `provider/places` and checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceRow {
    /// Its spoken name, cleaned.
    pub name: String,
    /// Where Enter goes.
    pub place: Place,
    /// Its state in words, cleaned: "saved", "connected", "lost".
    pub state: String,
    /// More words, read last.
    pub detail: Option<String>,
}

impl PlaceRow {
    /// The row in words, the state right after the name: "build, saved,
    /// alice@build.example".
    pub fn words(&self) -> String {
        lines::place_row(&self.name, &self.state, self.detail.as_deref())
    }
}

/// The places a plugin offers, with how many rows were left out because
/// their place or name could not be used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceRows {
    /// The rows, in the plugin's order.
    pub rows: Vec<PlaceRow>,
    /// How many were left out.
    pub skipped: usize,
}

/// A plugin's provider for one scheme.
pub struct PluginProvider {
    host: Weak<Host>,
    plugin: String,
    spec: ProviderSpec,
    connected: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for PluginProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginProvider")
            .field("plugin", &self.plugin)
            .field("scheme", &self.spec.scheme)
            .finish()
    }
}

/// The provider error for a plugin's error.
pub fn plugin_to_vfs(error: &PluginError, scheme: &str, operation: &str) -> VfsError {
    let (code, message, hint) = match error {
        PluginError::Remote {
            code,
            message,
            hint,
            ..
        } => (*code, message.clone(), hint.clone()),
        PluginError::Timeout(_) => {
            return VfsError::Io {
                kind: io::ErrorKind::TimedOut,
                message: error.to_string(),
            };
        }
        other => {
            return VfsError::Io {
                kind: io::ErrorKind::Other,
                message: other.to_string(),
            };
        }
    };
    match ErrorKind::from_code(code) {
        Some(ErrorKind::NotFound) => VfsError::NotFound,
        Some(ErrorKind::AlreadyExists) => VfsError::AlreadyExists,
        Some(ErrorKind::PermissionDenied | ErrorKind::DeniedByHost) => VfsError::PermissionDenied,
        Some(ErrorKind::NotADirectory) => VfsError::NotAFolder,
        Some(ErrorKind::IsADirectory) => VfsError::Io {
            kind: io::ErrorKind::IsADirectory,
            message: "it is a folder".to_owned(),
        },
        Some(ErrorKind::NotEmpty) => VfsError::NotEmpty,
        Some(ErrorKind::InvalidName) => VfsError::NameInvalid,
        Some(ErrorKind::Unsupported | ErrorKind::MethodNotFound) => {
            VfsError::unsupported(scheme, operation)
        }
        Some(ErrorKind::Unavailable | ErrorKind::PluginUnavailable) => VfsError::NetworkLost,
        Some(ErrorKind::AuthRequired) => VfsError::PasswordRequired,
        Some(ErrorKind::NoSpace) => VfsError::StorageFull,
        Some(ErrorKind::TooLarge) => VfsError::FileTooLarge,
        Some(ErrorKind::Conflict) => VfsError::Changed,
        Some(ErrorKind::Untrusted) => VfsError::Unsafe {
            why: lines::untrusted_why(&message, hint.as_deref()),
        },
        Some(ErrorKind::RequestCancelled) => VfsError::Cancelled,
        Some(ErrorKind::TimedOut) => VfsError::Io {
            kind: io::ErrorKind::TimedOut,
            message,
        },
        _ => VfsError::Io {
            kind: io::ErrorKind::Other,
            message,
        },
    }
}

fn wire_name_is_safe(name: &str) -> bool {
    !(name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']))
}

impl PluginProvider {
    /// The provider a plugin's manifest declares for a scheme, or `None`
    /// when it declares none.
    pub fn new(host: &Arc<Host>, plugin: &str, scheme: &str) -> Option<Self> {
        let spec = host
            .plugin(plugin)?
            .manifest
            .providers
            .iter()
            .find(|p| p.scheme == scheme)?
            .clone();
        Some(Self {
            host: Arc::downgrade(host),
            plugin: plugin.to_owned(),
            spec,
            connected: Mutex::new(HashSet::new()),
        })
    }

    /// Every provider of every enabled plugin, by scheme, for the
    /// registry.
    pub fn all(host: &Arc<Host>) -> Vec<(String, Arc<dyn Provider>)> {
        let mut out = Vec::new();
        for info in host.plugins().iter().filter(|p| p.enabled) {
            for spec in &info.manifest.providers {
                if let Some(provider) = Self::new(host, &info.manifest.id, &spec.scheme) {
                    out.push((spec.scheme.clone(), Arc::new(provider) as Arc<dyn Provider>));
                }
            }
        }
        out
    }

    fn has(&self, method: &str) -> bool {
        method == "list" || method == "stat" || self.spec.methods.contains(method)
    }

    fn flag(&self, flag: &str) -> bool {
        self.spec.flags.contains(flag)
    }

    fn host(&self) -> Result<Arc<Host>, VfsError> {
        self.host.upgrade().ok_or(VfsError::NetworkLost)
    }

    /// Signs in to the place's authority once, when the provider connects.
    fn connect(&self, place: &Place, force: bool) -> Result<(), VfsError> {
        if !self.has("connect") {
            return Ok(());
        }
        let authority = authority_of(place);
        let mut connected = self
            .connected
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if connected.contains(&authority) && !force {
            return Ok(());
        }
        let host = self.host()?;
        host.call(
            &self.plugin,
            "provider/connect",
            json!({"place": format!("{authority}/"), "interactive": true}),
        )
        .map_err(|e| plugin_to_vfs(&e, &self.spec.scheme, "sign in"))?;
        connected.insert(authority);
        Ok(())
    }

    /// One provider call: refused when the manifest does not list it,
    /// after a sign-in when needed, cancelled with the operation, and
    /// tried once more after a refused sign-in.
    fn call(
        &self,
        method: &str,
        operation: &str,
        place: &Place,
        params: Value,
        ctx: &OpContext,
    ) -> Result<Value, VfsError> {
        if !self.has(method) {
            return Err(VfsError::unsupported(&self.spec.scheme, operation));
        }
        ctx.check()?;
        self.connect(place, false)?;
        let host = self.host()?;
        let full = format!("provider/{method}");
        let attempt = || {
            host.call_with(
                &self.plugin,
                &full,
                params.clone(),
                CallOptions {
                    cancel: Some(&ctx.cancel),
                    batches: None,
                },
            )
        };
        match attempt() {
            Err(PluginError::Remote { code, .. })
                if ErrorKind::from_code(code) == Some(ErrorKind::AuthRequired) =>
            {
                self.connect(place, true)?;
                attempt().map_err(|e| plugin_to_vfs(&e, &self.spec.scheme, operation))
            }
            other => other.map_err(|e| plugin_to_vfs(&e, &self.spec.scheme, operation)),
        }
    }

    fn entries_of(&self, place: &Place, values: &[Value]) -> Vec<Entry> {
        values
            .iter()
            .filter_map(|value| serde_json::from_value::<WireEntry>(value.clone()).ok())
            .filter(|wire| {
                let safe = wire_name_is_safe(&wire.name);
                if !safe && let Some(log) = self.host.upgrade().and_then(|h| h.log(&self.plugin)) {
                    log.write(
                        crate::log::Source::Host,
                        &format!("left out an entry of {place} with a name that is not plain"),
                    );
                }
                safe
            })
            .map(|wire| wire.to_entry())
            .collect()
    }

    fn entry_of(&self, place: &Place, value: Value) -> Result<Entry, VfsError> {
        let value = match value {
            Value::Object(mut map) if map.contains_key("entry") => {
                map.remove("entry").unwrap_or(Value::Null)
            }
            other => other,
        };
        let wire: WireEntry = serde_json::from_value(value).map_err(|_| VfsError::Io {
            kind: io::ErrorKind::InvalidData,
            message: format!("the plugin's entry for {place} could not be read"),
        })?;
        Ok(wire.to_entry())
    }

    fn read_chunk(
        &self,
        place: &Place,
        offset: u64,
        ctx: &OpContext,
    ) -> Result<(Vec<u8>, bool), VfsError> {
        let answer = self.call(
            "read",
            "read files",
            place,
            json!({"place": place.to_string(), "offset": offset, "length": CHUNK, "transfer": "inline"}),
            ctx,
        )?;
        let data = answer
            .get("data")
            .and_then(Value::as_str)
            .and_then(base64::decode)
            .ok_or_else(|| VfsError::Io {
                kind: io::ErrorKind::InvalidData,
                message: "the plugin's data is not base64".to_owned(),
            })?;
        let eof = answer.get("eof").and_then(Value::as_bool).unwrap_or(true) || data.is_empty();
        Ok((data, eof))
    }
}

/// A file of a plugin's place, read a chunk at a time.
struct PluginReader {
    provider: Arc<PluginProvider>,
    place: Place,
    ctx: OpContext,
    offset: u64,
    buffer: Vec<u8>,
    at: usize,
    eof: bool,
}

impl Read for PluginReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.buffer.len() {
            if self.eof {
                return Ok(0);
            }
            let (data, eof) = self
                .provider
                .read_chunk(&self.place, self.offset, &self.ctx)
                .map_err(|e| io::Error::other(e.to_string()))?;
            self.offset += data.len() as u64;
            self.buffer = data;
            self.at = 0;
            self.eof = eof;
            if self.buffer.is_empty() {
                return Ok(0);
            }
        }
        let n = out.len().min(self.buffer.len() - self.at);
        out[..n].copy_from_slice(&self.buffer[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl Seek for PluginReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let here = self.offset - (self.buffer.len() - self.at) as u64;
        let target = match to {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(d) => here
                .checked_add_signed(d)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "before the start"))?,
            SeekFrom::End(d) => {
                let size = self
                    .provider
                    .stat(&self.place, &self.ctx)
                    .map_err(|e| io::Error::other(e.to_string()))?
                    .size
                    .unwrap_or(0);
                size.checked_add_signed(d).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "before the start")
                })?
            }
        };
        self.offset = target;
        self.buffer.clear();
        self.at = 0;
        self.eof = false;
        Ok(target)
    }
}

/// A write session on a plugin's place.
struct PluginWriter {
    provider: Arc<PluginProvider>,
    place: Place,
    ctx: OpContext,
    id: String,
    sent: u64,
    buffer: Vec<u8>,
    ended: bool,
}

impl PluginWriter {
    fn send(&mut self, last: bool) -> Result<(), VfsError> {
        if self.buffer.is_empty() && !last {
            return Ok(());
        }
        let data = base64::encode(&self.buffer);
        self.provider.call(
            "write_chunk",
            "write files",
            &self.place,
            json!({"write_id": self.id, "offset": self.sent, "data": data, "final": last}),
            &self.ctx,
        )?;
        self.sent += self.buffer.len() as u64;
        self.buffer.clear();
        Ok(())
    }

    fn end(&mut self, commit: bool) -> Result<(), VfsError> {
        self.ended = true;
        let ctx = if commit {
            self.ctx.clone()
        } else {
            // An abandon goes out even after a cancel.
            OpContext::new()
        };
        self.provider
            .call(
                "write_end",
                "write files",
                &self.place,
                json!({"write_id": self.id, "commit": commit}),
                &ctx,
            )
            .map(|_| ())
    }
}

impl Write for PluginWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= CHUNK {
            self.send(false)
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl FileWriter for PluginWriter {
    fn commit(mut self: Box<Self>, _durable: bool) -> Result<(), VfsError> {
        if let Err(error) = self.send(true) {
            let _ = self.end(false);
            return Err(error);
        }
        self.end(true)
    }
}

impl Drop for PluginWriter {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.end(false);
        }
    }
}

impl StreamingList for PluginProvider {
    fn list_batches(
        &self,
        place: &Place,
        ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<Entry>) -> ControlFlow<()>,
    ) -> Result<(), VfsError> {
        ctx.check()?;
        self.connect(place, false)?;
        let host = self.host()?;
        let mut take = |values: Vec<Value>| batch(self.entries_of(place, &values));
        let mut taken = ControlFlow::Continue(());
        let mut forward = |values: Vec<Value>| {
            taken = take(values);
            taken
        };
        let answer = host
            .call_with(
                &self.plugin,
                "provider/list",
                json!({"place": place.to_string()}),
                CallOptions {
                    cancel: Some(&ctx.cancel),
                    batches: Some(&mut forward),
                },
            )
            .map_err(|e| plugin_to_vfs(&e, &self.spec.scheme, "list folders"))?;
        if let Some(entries) = answer.get("entries").and_then(Value::as_array)
            && !entries.is_empty()
            && taken.is_continue()
        {
            let _ = take(entries.clone());
        }
        Ok(())
    }
}

impl Provider for PluginProvider {
    fn scheme(&self) -> &str {
        &self.spec.scheme
    }

    fn capabilities(&self, _place: &Place) -> Capabilities {
        let mut caps = Capabilities::read_only();
        caps.writable = self.flag("writable") && self.has("write_begin");
        caps.trash = self.flag("trash") && self.has("trash");
        caps.links = self.flag("links");
        caps.case_sensitive = self.flag("case_sensitive");
        caps.watch = self.flag("watch") && self.has("watch");
        caps.random_read = self.has("read");
        caps.clone_copy = self.flag("server_side_copy") && self.has("copy");
        caps.replace_rename = self.flag("atomic_rename") && self.has("rename");
        caps.max_name_length = self.spec.max_name_length.map(|n| n as usize);
        caps
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        let mut all = Vec::new();
        self.list_batches(place, ctx, &mut |batch| {
            all.extend(batch);
            ControlFlow::Continue(())
        })?;
        Ok(all)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        let answer = self.call(
            "stat",
            "read entries",
            place,
            json!({"place": place.to_string(), "follow_links": false}),
            ctx,
        )?;
        self.entry_of(place, answer)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Ok(Box::new(self.reader(place, ctx)?))
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        Ok(Box::new(self.reader(place, ctx)?))
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.begin_write(place, mode, None, ctx)
            .map(|(writer, _)| writer)
    }

    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        let Some(row) = self.volume_of(place, ctx)? else {
            return Ok(VolumeInfo::unknown(VolumeInfo::top_of(place)));
        };
        let kind = if row.removable {
            VolumeKind::Removable
        } else {
            VolumeKind::Network
        };
        let mut info = VolumeInfo::unknown(row.place).with_kind(kind);
        info.label = Some(row.name);
        Ok(info)
    }

    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        Ok(self
            .volume_of(place, ctx)?
            .map(|row| row.space())
            .unwrap_or_default())
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.call(
            "create_dir",
            "make folders",
            place,
            json!({"place": place.to_string(), "parents": false}),
            ctx,
        )
        .map(|_| ())
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.call(
            "rename",
            "rename",
            from,
            json!({"from": from.to_string(), "to": to.to_string(), "overwrite": false}),
            ctx,
        )
        .map(|_| ())
    }

    fn rename_replacing(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        if !self.flag("atomic_rename") {
            return Err(VfsError::unsupported(
                &self.spec.scheme,
                "rename over another entry",
            ));
        }
        self.call(
            "rename",
            "rename over another entry",
            from,
            json!({"from": from.to_string(), "to": to.to_string(), "overwrite": true}),
            ctx,
        )
        .map(|_| ())
    }

    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        let kind = match request.kind {
            LinkKind::Symbolic => "symbolic",
            LinkKind::Hard => "hard",
            LinkKind::Junction => "junction",
        };
        self.call(
            "create_link",
            "make links",
            place,
            json!({"place": place.to_string(), "target": request.target, "kind": kind}),
            ctx,
        )
        .map(|_| ())
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.call(
            "remove",
            "remove entries",
            place,
            json!({"place": place.to_string(), "recursive": false}),
            ctx,
        )
        .map(|_| ())
    }

    fn trash(&self, place: &Place, ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        if !self.flag("trash") {
            return Err(VfsError::unsupported(
                &self.spec.scheme,
                "move to the trash",
            ));
        }
        self.call(
            "trash",
            "move to the trash",
            place,
            json!({"place": place.to_string()}),
            ctx,
        )?;
        Ok(TrashCheck::InTrash)
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        if !self.has("set_attributes") {
            return Err(VfsError::unsupported(
                &self.spec.scheme,
                "change attributes",
            ));
        }
        let before = self.stat(place, ctx)?;
        let mut params = serde_json::Map::new();
        params.insert("place".into(), json!(place.to_string()));
        let mut inverse = AttributeChange::default();
        if let Some(Attributes::Unix(mode)) = change.attributes {
            params.insert("mode".into(), json!(mode & 0o7777));
            inverse.attributes = before.attributes;
        }
        if let Some(modified) = change.modified {
            params.insert("modified".into(), json!(format_time(modified)));
            inverse.modified = before.modified;
        }
        if let Some(accessed) = change.accessed {
            params.insert("accessed".into(), json!(format_time(accessed)));
        }
        self.call(
            "set_attributes",
            "change attributes",
            place,
            Value::Object(params),
            ctx,
        )?;
        Ok(inverse)
    }

    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        if !(self.flag("server_side_copy") && self.has("copy_hint") && self.has("copy")) {
            return CopyHint::Stream;
        }
        match self.call(
            "copy_hint",
            "copy on the server",
            from,
            json!({"from": from.to_string(), "to": to.to_string(), "move": false}),
            &OpContext::new(),
        ) {
            Ok(answer) if answer.get("mode").and_then(Value::as_str) == Some("server_side") => {
                CopyHint::ServerSide
            }
            _ => CopyHint::Stream,
        }
    }

    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.call(
            "copy",
            "copy on the server",
            from,
            json!({"from": from.to_string(), "to": to.to_string(), "overwrite": false, "preserve": true}),
            ctx,
        )
        .map(|_| ())
    }

    fn watch(&self, place: &Place, sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        if !(self.flag("watch") && self.has("watch")) {
            return Ok(None);
        }
        let host = self.host()?;
        let answer = self.call(
            "watch",
            "watch folders",
            place,
            json!({"place": place.to_string(), "recursive": false}),
            &OpContext::new(),
        )?;
        let Some(id) = answer.get("watch_id").and_then(Value::as_str) else {
            return Ok(None);
        };
        host.add_watch(&self.plugin, id, place.clone(), sink);
        let weak = Arc::downgrade(&host);
        let plugin = self.plugin.clone();
        let id = id.to_owned();
        let has_unwatch = self.has("unwatch");
        Ok(Some(WatchHandle::new(move || {
            if let Some(host) = weak.upgrade() {
                host.remove_watch(&plugin, &id);
                if has_unwatch {
                    let _ = host.call(&plugin, "provider/unwatch", json!({"watch_id": id}));
                }
            }
        })))
    }

    fn as_streaming(&self) -> Option<&dyn StreamingList> {
        Some(self)
    }
}

impl PluginProvider {
    /// Starts a write session that goes on from where an earlier,
    /// interrupted one stopped: the host says how many bytes it believes
    /// are there and the source's fingerprint; the answer is the writer and
    /// the offset its first chunk goes to, which the plugin chose (0 when
    /// it starts over: no partial file, a changed fingerprint, or a partial
    /// shorter than asked). The caller sends the bytes from that offset.
    pub fn write_resuming(
        &self,
        place: &Place,
        mode: WriteMode,
        resume: ResumeRequest,
        ctx: &OpContext,
    ) -> Result<(Box<dyn FileWriter>, u64), VfsError> {
        self.begin_write(place, mode, Some(resume), ctx)
    }

    fn begin_write(
        &self,
        place: &Place,
        mode: WriteMode,
        resume: Option<ResumeRequest>,
        ctx: &OpContext,
    ) -> Result<(Box<dyn FileWriter>, u64), VfsError> {
        let asked = resume.as_ref().map_or(0, |r| r.offset);
        let begin = WriteBegin {
            place: place.to_string(),
            conflict: match mode {
                WriteMode::CreateNew => "fail".to_owned(),
                WriteMode::Overwrite => "overwrite".to_owned(),
            },
            transfer: "inline".to_owned(),
            size_hint: None,
            resume,
        };
        let answer = self.call(
            "write_begin",
            "write files",
            place,
            serde_json::to_value(&begin).unwrap_or(Value::Null),
            ctx,
        )?;
        let begun: WriteBegun = serde_json::from_value(answer).map_err(|_| VfsError::Io {
            kind: io::ErrorKind::InvalidData,
            message: "the plugin's write session could not be read".to_owned(),
        })?;
        let mut writer = PluginWriter {
            provider: Arc::new(self.clone_for_io()),
            place: place.clone(),
            ctx: ctx.clone(),
            id: begun.write_id,
            sent: 0,
            buffer: Vec::new(),
            ended: false,
        };
        // A plugin cannot go on from further than the host asked: the bytes
        // between would never be sent.
        let offset = begun.offset.unwrap_or(0);
        if offset > asked {
            let _ = writer.end(false);
            return Err(VfsError::Io {
                kind: io::ErrorKind::InvalidData,
                message: format!(
                    "the plugin would resume at byte {offset}, past the {asked} the host has"
                ),
            });
        }
        writer.sent = offset;
        Ok((Box::new(writer), offset))
    }

    /// The storages of a place's authority (`provider/volumes` with the
    /// authority), for the volumes list; empty when the provider declares
    /// none. A row whose root is not a place of this provider is left
    /// out.
    pub fn volumes(&self, place: &Place, ctx: &OpContext) -> Result<Vec<VolumeRow>, VfsError> {
        if !self.has("volumes") {
            return Ok(Vec::new());
        }
        let authority = authority_of(place);
        let answer = self.call(
            "volumes",
            "list volumes",
            place,
            json!({"authority": authority}),
            ctx,
        )?;
        Ok(self.volume_rows(&answer))
    }

    /// The one volume that holds a place (`provider/volumes` with the
    /// place); `None` when the provider declares no volumes or names none.
    pub fn volume_of(&self, place: &Place, ctx: &OpContext) -> Result<Option<VolumeRow>, VfsError> {
        if !self.has("volumes") {
            return Ok(None);
        }
        let answer = self.call(
            "volumes",
            "check free space",
            place,
            json!({"place": place.to_string()}),
            ctx,
        )?;
        Ok(self.volume_rows(&answer).into_iter().next())
    }

    fn is_ours(&self, place: &Place) -> bool {
        place
            .remote()
            .is_some_and(|r| r.scheme() == self.spec.scheme)
    }

    fn volume_rows(&self, answer: &Value) -> Vec<VolumeRow> {
        let values = match (answer.get("volumes"), answer.get("volume")) {
            (Some(Value::Array(values)), _) => values.clone(),
            (_, Some(one @ Value::Object(_))) => vec![one.clone()],
            _ => Vec::new(),
        };
        values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WireVolume>(value).ok())
            .filter_map(|wire| {
                let place = plugin_place(&wire.place).ok()?;
                let name = clean_text(&wire.name, lines::MAX_LINE_CELLS);
                (self.is_ours(&place) && !name.is_empty()).then_some(VolumeRow {
                    name,
                    place,
                    free: wire.free,
                    total: wire.total,
                    removable: wire.removable,
                })
            })
            .collect()
    }

    /// The places the plugin offers (`provider/places`), such as its saved
    /// sites and the hosts of a configuration file, for the go-to-place
    /// list. Listing them never signs in. A row whose place is not one of
    /// this provider's, or whose name or state is empty once cleaned, is
    /// left out and counted.
    pub fn places(&self, ctx: &OpContext) -> Result<PlaceRows, VfsError> {
        if !self.has("places") {
            return Ok(PlaceRows::default());
        }
        ctx.check()?;
        let host = self.host()?;
        let answer = host
            .call_with(
                &self.plugin,
                "provider/places",
                json!({}),
                CallOptions {
                    cancel: Some(&ctx.cancel),
                    batches: None,
                },
            )
            .map_err(|e| plugin_to_vfs(&e, &self.spec.scheme, "list places"))?;
        let values = answer
            .get("places")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = PlaceRows::default();
        for value in values {
            match self.place_row(value) {
                Some(row) => out.rows.push(row),
                None => out.skipped += 1,
            }
        }
        Ok(out)
    }

    fn place_row(&self, value: Value) -> Option<PlaceRow> {
        let wire = serde_json::from_value::<WirePlaceRow>(value).ok()?;
        let place = plugin_place(&wire.place).ok()?;
        let name = clean_text(&wire.name, lines::MAX_LINE_CELLS);
        let state = clean_text(&wire.state, lines::MAX_LINE_CELLS);
        let detail = wire
            .detail
            .map(|d| clean_text(&d, lines::MAX_LINE_CELLS * 2))
            .filter(|d| !d.is_empty());
        (self.is_ours(&place) && !name.is_empty() && !state.is_empty()).then_some(PlaceRow {
            name,
            place,
            state,
            detail,
        })
    }

    fn clone_for_io(&self) -> Self {
        Self {
            host: self.host.clone(),
            plugin: self.plugin.clone(),
            spec: self.spec.clone(),
            connected: Mutex::new(
                self.connected
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
            ),
        }
    }

    fn reader(&self, place: &Place, ctx: &OpContext) -> Result<PluginReader, VfsError> {
        if !self.has("read") {
            return Err(VfsError::unsupported(&self.spec.scheme, "read files"));
        }
        Ok(PluginReader {
            provider: Arc::new(self.clone_for_io()),
            place: place.clone(),
            ctx: ctx.clone(),
            offset: 0,
            buffer: Vec::new(),
            at: 0,
            eof: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use diroxide_config::PluginSettings;
    use diroxide_core::{EntryKind, JobId, JobKind, JobRequest};
    use diroxide_ops::Engine;
    use diroxide_vfs::Registry;

    use super::*;
    use crate::host::Launcher;
    use crate::supervisor::{ManualTicker, Ticker};
    use crate::testing::memfs::MemFs;
    use crate::testing::{FakeLauncher, Mode, SkippingSink, plugin_info};

    const MANIFEST: &str = "\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\nwritable = true\nmethods = [\"connect\", \"list\", \"stat\", \"read\", \"write_begin\", \"write_chunk\", \"write_end\", \"create_dir\", \"rename\", \"remove\"]\n";

    struct Toy {
        host: Arc<Host>,
        provider: Arc<PluginProvider>,
        memfs: Arc<Mutex<MemFs>>,
    }

    fn toy(memfs: MemFs, manifest: &str) -> Toy {
        let memfs = Arc::new(Mutex::new(memfs));
        let launcher = FakeLauncher::new(Mode::Echo);
        launcher.with_memfs(Arc::clone(&memfs));
        let ticker = Arc::new(ManualTicker::new());
        let mut host = Host::new(&PluginSettings::default())
            .with_launcher(launcher as Arc<dyn Launcher>)
            .with_ticker(ticker as Arc<dyn Ticker>);
        host.add(plugin_info(manifest));
        let host = Arc::new(host);
        let provider = Arc::new(PluginProvider::new(&host, "line-counter", "memfs").unwrap());
        Toy {
            host,
            provider,
            memfs,
        }
    }

    fn p(path: &str) -> Place {
        Place::parse(&format!("memfs://box{path}")).unwrap()
    }

    fn ctx() -> OpContext {
        OpContext::new()
    }

    fn read_all(provider: &PluginProvider, place: &Place) -> Vec<u8> {
        let mut bytes = Vec::new();
        provider
            .read(place, &ctx())
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    #[test]
    fn listings_come_whole_or_in_batches() {
        let mut fs = MemFs::default();
        fs.add_folder("/empty");
        for i in 0..10 {
            fs.add_file(&format!("/ten/f{i}.txt"), b"x");
        }
        for i in 0..5000 {
            fs.add_file(&format!("/many/f{i:04}.txt"), b"");
        }
        let toy = toy(fs, MANIFEST);
        assert!(toy.provider.list(&p("/empty"), &ctx()).unwrap().is_empty());
        assert_eq!(toy.provider.list(&p("/ten"), &ctx()).unwrap().len(), 10);
        let mut batches = Vec::new();
        toy.provider
            .as_streaming()
            .unwrap()
            .list_batches(&p("/many"), &ctx(), &mut |batch| {
                batches.push(batch.len());
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(batches, [1000; 5]);
        assert_eq!(toy.provider.list(&p("/many"), &ctx()).unwrap().len(), 5000);
        // Stopping after the first batch hands over no more.
        let mut seen = 0;
        toy.provider
            .list_batches(&p("/many"), &ctx(), &mut |batch| {
                seen += batch.len();
                ControlFlow::Break(())
            })
            .unwrap();
        assert_eq!(seen, 1000);
        assert_eq!(
            toy.provider.list(&p("/ten/f1.txt"), &ctx()).unwrap_err(),
            VfsError::NotAFolder
        );
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    #[test]
    fn stat_and_ranged_reads() {
        let mut fs = MemFs::default();
        fs.add_file("/a/data.bin", b"0123456789");
        fs.add_file("/a/empty.txt", b"");
        let toy = toy(fs, MANIFEST);
        let file = toy.provider.stat(&p("/a/data.bin"), &ctx()).unwrap();
        assert_eq!((file.kind, file.size), (EntryKind::File, Some(10)));
        assert_eq!(
            toy.provider.stat(&p("/a"), &ctx()).unwrap().kind,
            EntryKind::Folder
        );
        assert_eq!(
            toy.provider.stat(&p("/a/missing"), &ctx()).unwrap_err(),
            VfsError::NotFound
        );
        assert_eq!(read_all(&toy.provider, &p("/a/data.bin")), b"0123456789");
        assert!(read_all(&toy.provider, &p("/a/empty.txt")).is_empty());
        let mut seekable = toy
            .provider
            .open_seekable(&p("/a/data.bin"), &ctx())
            .unwrap();
        seekable.seek(SeekFrom::Start(4)).unwrap();
        let mut middle = [0u8; 3];
        seekable.read_exact(&mut middle).unwrap();
        assert_eq!(&middle, b"456");
        seekable.seek(SeekFrom::End(-2)).unwrap();
        let mut end = Vec::new();
        seekable.read_to_end(&mut end).unwrap();
        assert_eq!(end, b"89");
        seekable.seek(SeekFrom::Start(50)).unwrap();
        let mut past = Vec::new();
        seekable.read_to_end(&mut past).unwrap();
        assert!(past.is_empty());
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    #[test]
    fn writes_commit_or_leave_nothing() {
        let mut fs = MemFs::default();
        fs.add_file("/w/old.txt", b"old");
        let toy = toy(fs, MANIFEST);
        let big: Vec<u8> = (0..(CHUNK + 1000)).map(|i| (i % 251) as u8).collect();
        let mut writer = toy
            .provider
            .write(&p("/w/new.bin"), WriteMode::CreateNew, &ctx())
            .unwrap();
        writer.write_all(&big).unwrap();
        writer.commit(true).unwrap();
        assert_eq!(read_all(&toy.provider, &p("/w/new.bin")), big);
        let mut aborted = toy
            .provider
            .write(&p("/w/aborted.txt"), WriteMode::CreateNew, &ctx())
            .unwrap();
        aborted.write_all(b"half").unwrap();
        drop(aborted);
        assert_eq!(
            toy.provider.stat(&p("/w/aborted.txt"), &ctx()).unwrap_err(),
            VfsError::NotFound
        );
        assert_eq!(toy.memfs.lock().unwrap().open_writes(), 0);
        assert_eq!(
            toy.provider
                .write(&p("/w/old.txt"), WriteMode::CreateNew, &ctx())
                .err()
                .unwrap(),
            VfsError::AlreadyExists
        );
        let mut over = toy
            .provider
            .write(&p("/w/old.txt"), WriteMode::Overwrite, &ctx())
            .unwrap();
        over.write_all(b"new").unwrap();
        over.commit(true).unwrap();
        assert_eq!(read_all(&toy.provider, &p("/w/old.txt")), b"new");
        assert_eq!(
            toy.provider
                .write(&p("/w/bad:name"), WriteMode::CreateNew, &ctx())
                .err()
                .unwrap(),
            VfsError::NameInvalid
        );
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    #[test]
    fn folders_renames_and_removes() {
        let mut fs = MemFs::default();
        fs.add_file("/r/a.txt", b"a");
        fs.add_file("/r/b.txt", b"b");
        fs.add_file("/r/full/x.txt", b"x");
        let toy = toy(fs, MANIFEST);
        toy.provider.create_dir(&p("/r/new"), &ctx()).unwrap();
        assert_eq!(
            toy.provider
                .create_dir(&p("/r/none/deeper"), &ctx())
                .unwrap_err(),
            VfsError::NotFound
        );
        toy.provider
            .rename(&p("/r/a.txt"), &p("/r/c.txt"), &ctx())
            .unwrap();
        assert_eq!(
            toy.provider
                .rename(&p("/r/c.txt"), &p("/r/b.txt"), &ctx())
                .unwrap_err(),
            VfsError::AlreadyExists
        );
        toy.provider.remove(&p("/r/c.txt"), &ctx()).unwrap();
        toy.provider.remove(&p("/r/new"), &ctx()).unwrap();
        assert_eq!(
            toy.provider.remove(&p("/r/full"), &ctx()).unwrap_err(),
            VfsError::NotEmpty
        );
        // Not in the manifest: refused by the host, the plugin never asked.
        let before = toy.memfs.lock().unwrap().calls.len();
        assert!(matches!(
            toy.provider.trash(&p("/r/b.txt"), &ctx()).unwrap_err(),
            VfsError::Unsupported { .. }
        ));
        assert!(matches!(
            toy.provider
                .set_attributes(&p("/r/b.txt"), &AttributeChange::default(), &ctx())
                .unwrap_err(),
            VfsError::Unsupported { .. }
        ));
        assert_eq!(toy.memfs.lock().unwrap().calls.len(), before);
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    #[test]
    fn a_refused_sign_in_is_tried_again_and_a_cancel_stops_before_the_call() {
        let mut fs = MemFs::refusing_connects(1);
        fs.add_file("/s/a.txt", b"a");
        let toy = toy(fs, MANIFEST);
        assert_eq!(
            toy.provider.stat(&p("/s/a.txt"), &ctx()).unwrap_err(),
            VfsError::PasswordRequired
        );
        assert_eq!(
            toy.provider.stat(&p("/s/a.txt"), &ctx()).unwrap().name,
            "a.txt"
        );
        let cancelled = OpContext::new();
        cancelled.cancel.cancel();
        assert_eq!(
            toy.provider.stat(&p("/s/a.txt"), &cancelled).unwrap_err(),
            VfsError::Cancelled
        );
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    /// The test plan's kill mid-write: the plugin is never told to commit,
    /// so the file never appears, even in a store that outlives it.
    #[test]
    fn a_plugin_killed_mid_write_leaves_nothing() {
        let mut fs = MemFs::default();
        fs.add_folder("/k");
        let toy = toy(fs, MANIFEST);
        let mut writer = toy
            .provider
            .write(&p("/k/file.bin"), WriteMode::CreateNew, &ctx())
            .unwrap();
        writer.write_all(&vec![7u8; CHUNK + 10]).unwrap();
        toy.host.kill("line-counter");
        assert!(writer.commit(true).is_err());
        assert!(toy.memfs.lock().unwrap().file("/k/file.bin").is_none());
        assert_eq!(
            toy.provider.stat(&p("/k/file.bin"), &ctx()).unwrap_err(),
            VfsError::NotFound
        );
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    /// The check of R22's commit 13: a folder copied to the toy and back
    /// through the job engine comes back byte for byte.
    #[test]
    fn a_folder_copies_to_the_toy_and_back_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("inner")).unwrap();
        std::fs::write(source.join("one.txt"), b"first file").unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(source.join("inner").join("big.bin"), &big).unwrap();
        std::fs::write(source.join("inner").join("empty"), b"").unwrap();
        let mut fs = MemFs::default();
        fs.add_folder("/dest");
        let toy = toy(fs, MANIFEST);
        let mut registry = Registry::with_local();
        registry.register("memfs", Arc::clone(&toy.provider) as Arc<dyn Provider>);
        let engine = Engine::new(Arc::new(registry));
        let local = |path: &std::path::Path| Place::parse(&path.display().to_string()).unwrap();
        let run = |request: JobRequest| {
            let plan = engine.plan(&request).unwrap();
            let report = engine.run(&plan, &diroxide_ops::JobControl::new(), &SkippingSink);
            assert!(report.failed.is_empty(), "{:?}", report.failed);
            report
        };
        run(JobRequest::new(JobId::new(1), JobKind::Copy, vec![local(&source)]).to(p("/dest")));
        assert_eq!(
            toy.memfs.lock().unwrap().file("/dest/source/inner/big.bin"),
            Some(big.as_slice())
        );
        let back = root.path().join("back");
        std::fs::create_dir_all(&back).unwrap();
        run(
            JobRequest::new(JobId::new(2), JobKind::Copy, vec![p("/dest/source")]).to(local(&back)),
        );
        for file in ["one.txt", "inner/big.bin", "inner/empty"] {
            assert_eq!(
                std::fs::read(back.join("source").join(file)).unwrap(),
                std::fs::read(source.join(file)).unwrap(),
                "{file}"
            );
        }
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    #[test]
    fn the_resume_shape_reads_its_fixture() {
        let fixture = include_str!("../../../fixtures/plugins/shapes/write-begin-resume.json");
        let fixture: Value = serde_json::from_str(fixture).unwrap();
        let begin: WriteBegin = serde_json::from_value(fixture["params"].clone()).unwrap();
        assert_eq!(begin.resume.as_ref().unwrap().offset, 1_048_576);
        let begun: WriteBegun = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(begun.offset, Some(1_048_576));
        let plain: WriteBegun = serde_json::from_value(json!({"write_id": "w2"})).unwrap();
        assert_eq!(plain.offset, None);
    }

    #[test]
    fn plugin_errors_become_provider_errors() {
        let remote = |kind: ErrorKind| PluginError::Remote {
            plugin: "X".into(),
            code: kind.code(),
            message: "words".into(),
            hint: None,
        };
        for (kind, expected) in [
            (ErrorKind::NotFound, VfsError::NotFound),
            (ErrorKind::AlreadyExists, VfsError::AlreadyExists),
            (ErrorKind::NotEmpty, VfsError::NotEmpty),
            (ErrorKind::InvalidName, VfsError::NameInvalid),
            (ErrorKind::AuthRequired, VfsError::PasswordRequired),
            (ErrorKind::NoSpace, VfsError::StorageFull),
            (ErrorKind::Conflict, VfsError::Changed),
            (ErrorKind::RequestCancelled, VfsError::Cancelled),
        ] {
            assert_eq!(plugin_to_vfs(&remote(kind), "memfs", "read"), expected);
        }
        assert!(matches!(
            plugin_to_vfs(&remote(ErrorKind::Unsupported), "memfs", "read files"),
            VfsError::Unsupported { .. }
        ));
        let untrusted = PluginError::Remote {
            plugin: "SFTP places".into(),
            code: ErrorKind::Untrusted.code(),
            message: "Host key changed. Not connected.".into(),
            hint: Some("To forget the old key, run:\nssh-keygen -R build.example".into()),
        };
        assert_eq!(
            plugin_to_vfs(&untrusted, "sftp", "sign in"),
            VfsError::Unsafe {
                why: "Host key changed. Not connected. To forget the old key, run: ssh-keygen -R build.example".into()
            }
        );
        assert_eq!(
            diroxide_core::Explain::next_step(&untrusted),
            "To forget the old key, run:\nssh-keygen -R build.example"
        );
    }

    const MANIFEST_ALL: &str = "\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\nwritable = true\nmethods = [\"connect\", \"list\", \"stat\", \"read\", \"write_begin\", \"write_chunk\", \"write_end\", \"create_dir\", \"rename\", \"remove\", \"volumes\", \"places\"]\n";

    /// R25's commit 46: an untrusted authority is refused with its hint,
    /// and the plugin is never asked anything past the refused sign-in.
    #[test]
    fn an_untrusted_authority_is_refused_with_its_hint() {
        let toy = toy(MemFs::default(), MANIFEST_ALL);
        let changed = Place::parse("memfs://changed/x.txt").unwrap();
        let error = toy.provider.stat(&changed, &ctx()).unwrap_err();
        assert_eq!(
            error,
            VfsError::Unsafe {
                why: "Host key changed. Not connected. To forget the old key, run: ssh-keygen -R changed".into()
            }
        );
        assert_eq!(toy.memfs.lock().unwrap().calls, ["connect"]);
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    /// R25's commit 46: volumes with and without a place; a number the
    /// plugin does not give is not known, never zero; a provider that
    /// declares no volumes is never asked.
    #[test]
    fn volumes_with_and_without_a_place() {
        let mut fs = MemFs::default();
        fs.add_file("/v/data.bin", &[1u8; 1000]);
        let toy = toy(fs, MANIFEST_ALL);
        let rows = toy.provider.volumes(&p("/v"), &ctx()).unwrap();
        let words: Vec<String> = rows.iter().map(VolumeRow::words).collect();
        assert_eq!(
            words,
            [
                "Memory box, 999 KB free of 1.0 MB",
                "Spare shelf, 999 KB free of 1.0 MB"
            ]
        );
        assert_eq!(rows[0].place.to_string(), "memfs://box/");
        let space = toy.provider.space(&p("/v/data.bin"), &ctx()).unwrap();
        assert_eq!(
            (space.free_to_user, space.total),
            (Some(999_000), Some(1_000_000))
        );
        let volume = toy.provider.volume(&p("/v/data.bin"), &ctx()).unwrap();
        assert_eq!(volume.kind, VolumeKind::Network);
        assert_eq!(volume.label.as_deref(), Some("Memory box"));
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);

        let unknown = self::toy(MemFs::default().without_free_space(), MANIFEST_ALL);
        let space = unknown.provider.space(&p("/"), &ctx()).unwrap();
        assert_eq!(space, SpaceInfo::default());
        assert_eq!(
            unknown.provider.volumes(&p("/"), &ctx()).unwrap()[0].words(),
            "Memory box, free space not known"
        );
        unknown.host.shutdown_all(crate::host::QUIT_BUDGET);

        let undeclared = self::toy(MemFs::default(), MANIFEST);
        assert!(
            undeclared
                .provider
                .volumes(&p("/"), &ctx())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            undeclared.provider.space(&p("/"), &ctx()).unwrap(),
            SpaceInfo::default()
        );
        assert!(
            !undeclared
                .memfs
                .lock()
                .unwrap()
                .calls
                .contains(&"volumes".to_owned())
        );
        undeclared.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    /// R25's commit 46: the places a plugin offers, state first after the
    /// name, without a sign-in; rows that are not this provider's places,
    /// or have nothing to say, are left out and counted.
    #[test]
    fn places_are_rows_with_a_state_word() {
        let toy = toy(MemFs::default(), MANIFEST_ALL);
        let places = toy.provider.places(&ctx()).unwrap();
        let words: Vec<String> = places.rows.iter().map(PlaceRow::words).collect();
        assert_eq!(words, ["box, connected, Memory box", "spare, saved"]);
        assert_eq!(places.rows[1].place.to_string(), "memfs://spare/");
        assert_eq!(places.skipped, 3);
        assert_eq!(toy.memfs.lock().unwrap().calls, ["places"]);
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
        let undeclared = self::toy(MemFs::default(), MANIFEST);
        assert_eq!(
            undeclared.provider.places(&ctx()).unwrap(),
            PlaceRows::default()
        );
        undeclared.host.shutdown_all(crate::host::QUIT_BUDGET);
    }

    /// R25's commit 46: `write_begin` carries `resume` and the answer's
    /// offset is where the chunks go on; a changed source starts over.
    #[test]
    fn a_write_resumes_from_the_offset_the_plugin_answers() {
        let mut fs = MemFs::default();
        fs.add_folder("/r");
        fs.add_partial("/r/big.bin", b"0123456789", Some("src-1"));
        fs.add_partial("/r/other.bin", b"abcdef", Some("src-1"));
        let toy = toy(fs, MANIFEST_ALL);
        let (mut writer, offset) = toy
            .provider
            .write_resuming(
                &p("/r/big.bin"),
                WriteMode::CreateNew,
                ResumeRequest {
                    offset: 6,
                    fingerprint: Some("src-1".into()),
                },
                &ctx(),
            )
            .unwrap();
        assert_eq!(offset, 6);
        writer.write_all(b"6789AB").unwrap();
        writer.commit(true).unwrap();
        assert_eq!(read_all(&toy.provider, &p("/r/big.bin")), b"0123456789AB");
        assert!(toy.memfs.lock().unwrap().partial("/r/big.bin").is_none());
        // Another source: the plugin starts over at 0.
        let (mut writer, offset) = toy
            .provider
            .write_resuming(
                &p("/r/other.bin"),
                WriteMode::CreateNew,
                ResumeRequest {
                    offset: 3,
                    fingerprint: Some("src-2".into()),
                },
                &ctx(),
            )
            .unwrap();
        assert_eq!(offset, 0);
        writer.write_all(b"fresh").unwrap();
        writer.commit(true).unwrap();
        assert_eq!(read_all(&toy.provider, &p("/r/other.bin")), b"fresh");
        toy.host.shutdown_all(crate::host::QUIT_BUDGET);
    }
}

/// The authority of a remote place, `scheme://user@host:port` with the user
/// and the port only when the place has them, so two accounts or two ports
/// on one host are two sign-ins (S-B's request).
fn authority_of(place: &Place) -> String {
    place
        .remote()
        .map(|r| {
            let mut s = format!("{}://", r.scheme());
            if let Some(user) = r.user() {
                s.push_str(user);
                s.push('@');
            }
            s.push_str(r.host());
            if let Some(port) = r.port() {
                s.push_str(&format!(":{port}"));
            }
            s
        })
        .unwrap_or_default()
}
