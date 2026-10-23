//! Host services, part four: files (`fs/*`), through the providers and the
//! job engine (R3, section 3.6; R22, sections 2.3 N8 and 7.2).
//!
//! - **Reading** (`fs/list`, `fs/stat`, `fs/read`) goes through the
//!   provider that serves the place, plugin providers included. `fs/read`
//!   answers a range inline in base64 (1 MiB at most), or writes it into a
//!   new file of the plugin's spool (`transfer = "file"`).
//! - **Writing a file** (`fs/write`) replaces it whole through the
//!   provider's committing writer, from base64 (4 MiB at most) or from a
//!   spool file. A file that exists is replaced only after the owner says
//!   yes in the host's own question, which starts on No.
//! - **Everything else** (`fs/mkdir`, `fs/rename`, `fs/copy`, `fs/move`,
//!   `fs/delete`) becomes a [`JobRequest`] from the plugin, which the app
//!   runs like a built-in job: the plan, the listing, the confirmation that
//!   starts on No, the trash by default, the anchor refusals and the undo
//!   log. Nothing the plugin sends turns them off.
//! - **Names** a plugin makes follow the portable-name policy: control,
//!   private-use and direction characters, `: * ? " < > | \`, a trailing
//!   dot or space and device names are refused, so no plugin makes the
//!   kind of name that led to the September 26 incident through diroxide.
//! - **Re-entrancy:** a call on a place the same plugin serves is -32032,
//!   because a plugin waiting for the answer could never serve it.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use diroxide_core::{ConflictPolicy, EntryKind, JobId, JobKind, JobOrigin, JobRequest, Place};
use diroxide_ops::JobReport;
use diroxide_vfs::{NameRules, OpContext, Registry, VfsError, WriteMode, check_name_with};
use serde_json::{Value, json};

use crate::base64;
use crate::places::{plugin_place, plugin_places};
use crate::protocol::{ErrorKind, Params, RpcError};
use crate::shapes::WireEntry;
use crate::spool::Spool;

/// The most an inline `fs/read` answers, in bytes.
pub const MAX_INLINE_READ: u64 = 1024 * 1024;

/// The most an inline `fs/write` takes, in bytes.
pub const MAX_INLINE_WRITE: usize = 4 * 1024 * 1024;

/// The most entries `fs/list` answers.
pub const MAX_LIST_ENTRIES: usize = 100_000;

/// The rules a plugin's new names follow on every system.
const PORTABLE: NameRules = NameRules {
    windows: true,
    max_length: Some(255),
    length_in_utf16: true,
};

/// The refusal of a name a plugin would make, or `None` when it may.
pub fn portable_refusal(name: &str) -> Option<String> {
    check_name_with(name, &[], &PORTABLE)
        .into_iter()
        .find(|issue| {
            issue.refused(&PORTABLE) || *issue == diroxide_vfs::NameIssue::HiddenCharacters
        })
        .map(|issue| format!("the name {} {}", spelled(name), issue.describe()))
}

/// A name in a message, with hidden characters spelled out.
fn spelled(name: &str) -> String {
    diroxide_core::names::spell_out(name).into_owned()
}

fn check_portable(name: &str) -> Result<(), RpcError> {
    match portable_refusal(name) {
        Some(why) => Err(RpcError::new(ErrorKind::InvalidName, why)),
        None => Ok(()),
    }
}

/// The plugin-side error for a provider's error.
pub fn vfs_to_rpc(error: &VfsError) -> RpcError {
    let kind = match error {
        VfsError::Cancelled => ErrorKind::RequestCancelled,
        VfsError::NotFound => ErrorKind::NotFound,
        VfsError::PermissionDenied => ErrorKind::PermissionDenied,
        VfsError::AlreadyExists => ErrorKind::AlreadyExists,
        VfsError::NotEmpty => ErrorKind::NotEmpty,
        VfsError::NotAFolder => ErrorKind::NotADirectory,
        VfsError::Unsupported { .. } | VfsError::UnsupportedFeature { .. } => {
            ErrorKind::Unsupported
        }
        VfsError::NoProvider(_) => ErrorKind::NotFound,
        VfsError::NameInvalid | VfsError::NameNotUnicode => ErrorKind::InvalidName,
        VfsError::StorageFull | VfsError::QuotaExceeded => ErrorKind::NoSpace,
        VfsError::FileTooLarge | VfsError::LimitReached { .. } => ErrorKind::TooLarge,
        VfsError::NetworkLost | VfsError::InUse => ErrorKind::Unavailable,
        VfsError::PasswordRequired | VfsError::WrongPassword => ErrorKind::AuthRequired,
        VfsError::Changed => ErrorKind::Conflict,
        VfsError::ReadOnlyFilesystem | VfsError::ReadOnlyHere { .. } => ErrorKind::PermissionDenied,
        _ => ErrorKind::Failed,
    };
    RpcError::new(kind, error.to_string())
}

/// What `fs/*` reaches: the providers, the spool, and the plugin's own
/// schemes for the re-entrancy rule.
#[derive(Clone)]
pub(crate) struct Files {
    pub(crate) registry: Option<Arc<Registry>>,
    pub(crate) spool: Option<Arc<Spool>>,
    pub(crate) own_schemes: Vec<String>,
    pub(crate) plugin: String,
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn place_param(params: &Params, key: &str) -> Result<Place, RpcError> {
    let text = params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{key} is missing")))?;
    plugin_place(text)
}

fn places_param(params: &Params) -> Result<Vec<Place>, RpcError> {
    match params.get("items").or_else(|| params.get("from")) {
        Some(Value::Array(items)) if !items.is_empty() => plugin_places(items),
        Some(Value::String(one)) => Ok(vec![plugin_place(one)?]),
        _ => Err(invalid("items must be a list of places")),
    }
}

fn conflict_param(params: &Params) -> Result<ConflictPolicy, RpcError> {
    Ok(match params.get("conflict").and_then(Value::as_str) {
        None | Some("ask") => ConflictPolicy::Ask,
        Some("skip") => ConflictPolicy::Skip,
        Some("overwrite") => ConflictPolicy::Overwrite,
        Some("keep_both") => ConflictPolicy::KeepBoth,
        Some("rename") => ConflictPolicy::Rename,
        Some("newer") => ConflictPolicy::Newer,
        Some(other) => return Err(invalid(format!("conflict {other} is not known"))),
    })
}

/// The answer to a file job: counts and the failures, problems first in
/// the summary.
pub fn report_value(report: &JobReport) -> Value {
    let failed: Vec<Value> = report
        .failed
        .iter()
        .map(|(place, reason)| json!({"place": place.to_string(), "reason": reason}))
        .collect();
    json!({
        "done": report.done.len(),
        "skipped": report.skipped.len(),
        "failed": failed,
        "cancelled": report.cancelled,
        "summary": report.summary(),
    })
}

impl Files {
    fn registry(&self) -> Result<&Arc<Registry>, RpcError> {
        self.registry
            .as_ref()
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host reaches no files"))
    }

    fn spool(&self) -> Result<&Arc<Spool>, RpcError> {
        self.spool
            .as_ref()
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host has no spool"))
    }

    /// Refuses a place this plugin serves itself (-32032).
    pub(crate) fn not_reentrant(&self, place: &Place) -> Result<(), RpcError> {
        if let Some(remote) = place.remote()
            && self.own_schemes.iter().any(|s| s == remote.scheme())
        {
            return Err(RpcError::new(
                ErrorKind::Reentrant,
                format!(
                    "{} serves {} places itself; read them directly",
                    self.plugin,
                    remote.scheme()
                ),
            ));
        }
        Ok(())
    }

    fn provider(&self, place: &Place) -> Result<Arc<dyn diroxide_vfs::Provider>, RpcError> {
        self.not_reentrant(place)?;
        self.registry()?.resolve(place).map_err(|e| vfs_to_rpc(&e))
    }

    /// `fs/list`: the entries of a folder.
    pub(crate) fn list(&self, params: &Params, ctx: &OpContext) -> Result<Value, RpcError> {
        let place = place_param(params, "place")?;
        let entries = self
            .provider(&place)?
            .list(&place, ctx)
            .map_err(|e| vfs_to_rpc(&e))?;
        let truncated = entries.len() > MAX_LIST_ENTRIES;
        let entries: Vec<WireEntry> = entries
            .iter()
            .take(MAX_LIST_ENTRIES)
            .map(|entry| {
                let at = place.join(&entry.name).ok();
                WireEntry::from_entry(entry, at.as_ref())
            })
            .collect();
        Ok(json!({"entries": entries, "truncated": truncated}))
    }

    /// `fs/stat`: one entry.
    pub(crate) fn stat(&self, params: &Params, ctx: &OpContext) -> Result<Value, RpcError> {
        let place = place_param(params, "place")?;
        let entry = self
            .provider(&place)?
            .stat(&place, ctx)
            .map_err(|e| vfs_to_rpc(&e))?;
        Ok(json!(WireEntry::from_entry(&entry, Some(&place))))
    }

    /// `fs/read`: a range, inline or into the spool.
    pub(crate) fn read(&self, params: &Params, ctx: &OpContext) -> Result<Value, RpcError> {
        let place = place_param(params, "place")?;
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let to_file = match params.get("transfer").and_then(Value::as_str) {
            None | Some("inline") => false,
            Some("file") => true,
            Some(other) => return Err(invalid(format!("transfer {other} is not inline or file"))),
        };
        let wanted = params
            .get("length")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_INLINE_READ);
        if !to_file && wanted > MAX_INLINE_READ {
            return Err(RpcError::new(
                ErrorKind::TooLarge,
                "an inline read is 1 MiB at most; use transfer file",
            ));
        }
        let provider = self.provider(&place)?;
        let size = provider.stat(&place, ctx).map_err(|e| vfs_to_rpc(&e))?.size;
        let mut reader: Box<dyn Read + Send> = match provider.open_seekable(&place, ctx) {
            Ok(mut seekable) => {
                seekable
                    .seek(SeekFrom::Start(offset))
                    .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
                Box::new(seekable)
            }
            Err(_) => {
                let mut stream = provider.read(&place, ctx).map_err(|e| vfs_to_rpc(&e))?;
                std::io::copy(&mut (&mut stream).take(offset), &mut std::io::sink())
                    .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
                stream
            }
        };
        let mut limited = (&mut reader).take(wanted);
        let length;
        let mut answer = if to_file {
            let spool_path = params
                .get("spool_path")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("a file transfer needs a spool_path"))?;
            let mut file = self
                .spool()?
                .create_to_write(spool_path, wanted.min(size.unwrap_or(wanted)))
                .map_err(|r| r.to_rpc())?;
            length = std::io::copy(&mut limited, &mut file)
                .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
            json!({})
        } else {
            let mut bytes = Vec::new();
            limited
                .read_to_end(&mut bytes)
                .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
            length = bytes.len() as u64;
            json!({"data": base64::encode(&bytes)})
        };
        let eof = size.map_or(length < wanted, |size| {
            offset.saturating_add(length) >= size
        });
        if let Some(fields) = answer.as_object_mut() {
            fields.insert("length".into(), json!(length));
            fields.insert("eof".into(), json!(eof));
        }
        Ok(answer)
    }

    /// The bytes of an `fs/write`: inline base64 or a spool file.
    fn write_source(&self, params: &Params) -> Result<Box<dyn Read + Send>, RpcError> {
        match params.get("transfer").and_then(Value::as_str) {
            Some("file") => {
                let spool_path = params
                    .get("spool_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("a file transfer needs a spool_path"))?;
                let file = self
                    .spool()?
                    .open_to_read(spool_path)
                    .map_err(|r| r.to_rpc())?;
                Ok(Box::new(file))
            }
            None | Some("inline") => {
                let text = params
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("data is missing"))?;
                if text.len() > MAX_INLINE_WRITE.div_ceil(3) * 4 {
                    return Err(RpcError::new(
                        ErrorKind::TooLarge,
                        "an inline write is 4 MiB at most; use transfer file",
                    ));
                }
                let bytes = base64::decode(text).ok_or_else(|| invalid("data is not base64"))?;
                Ok(Box::new(std::io::Cursor::new(bytes)))
            }
            Some(other) => Err(invalid(format!("transfer {other} is not inline or file"))),
        }
    }

    /// `fs/write`'s target and whether it exists, before the owner is
    /// asked: the place, its name checked.
    pub(crate) fn write_target(
        &self,
        params: &Params,
        ctx: &OpContext,
    ) -> Result<(Place, bool), RpcError> {
        let place = place_param(params, "place")?;
        let name = place
            .name()
            .ok_or_else(|| invalid("a file needs a name"))?
            .to_owned();
        check_portable(&name)?;
        let exists = match self.provider(&place)?.stat(&place, ctx) {
            Ok(entry) if entry.kind == EntryKind::Folder => {
                return Err(RpcError::new(ErrorKind::IsADirectory, "it is a folder"));
            }
            Ok(_) => true,
            Err(VfsError::NotFound) => false,
            Err(e) => return Err(vfs_to_rpc(&e)),
        };
        Ok((place, exists))
    }

    /// `fs/write` once allowed: the whole file, committed or nothing.
    pub(crate) fn write(
        &self,
        params: &Params,
        place: &Place,
        replace: bool,
        ctx: &OpContext,
    ) -> Result<Value, RpcError> {
        let mut source = self.write_source(params)?;
        let provider = self.provider(place)?;
        let mode = if replace {
            WriteMode::Overwrite
        } else {
            WriteMode::CreateNew
        };
        let mut writer = provider
            .write(place, mode, ctx)
            .map_err(|e| vfs_to_rpc(&e))?;
        let written = std::io::copy(&mut source, &mut writer)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        writer
            .flush()
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        writer.commit(true).map_err(|e| vfs_to_rpc(&e))?;
        Ok(json!({"written": true, "length": written}))
    }

    /// The job a file request becomes: what it does, from the plugin, in
    /// the words the engine reads. The id is the app's to give.
    pub(crate) fn job(&self, method: &str, params: &Params) -> Result<Vec<JobRequest>, RpcError> {
        let origin = JobOrigin::Plugin(self.plugin.clone());
        let request = |kind: JobKind, sources: Vec<Place>| {
            let mut request = JobRequest::new(JobId::new(0), kind, sources);
            request.origin = origin.clone();
            request
        };
        let checked = |places: &[Place]| -> Result<(), RpcError> {
            for place in places {
                self.not_reentrant(place)?;
            }
            Ok(())
        };
        Ok(match method {
            crate::protocol::methods::FS_MKDIR => {
                let places = match params.get("place") {
                    Some(_) => vec![place_param(params, "place")?],
                    None => places_param(params)?,
                };
                checked(&places)?;
                let parents = params
                    .get("parents")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut jobs = Vec::new();
                for place in places {
                    let (base, name) = self.make_folder_parts(&place, parents)?;
                    let mut job = request(JobKind::MakeFolder, Vec::new());
                    job.destination = Some(base);
                    job.new_name = Some(name);
                    jobs.push(job);
                }
                jobs
            }
            crate::protocol::methods::FS_RENAME => {
                let pairs: Vec<(Place, Place)> = match params.get("items") {
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|item| {
                            let item = item
                                .as_object()
                                .ok_or_else(|| invalid("each rename is {from, to}"))?;
                            Ok((place_param(item, "from")?, place_param(item, "to")?))
                        })
                        .collect::<Result<_, RpcError>>()?,
                    _ => vec![(place_param(params, "from")?, place_param(params, "to")?)],
                };
                let mut renames = Vec::new();
                for (from, to) in pairs {
                    checked(&[from.clone(), to.clone()])?;
                    if from.parent() != to.parent() {
                        return Err(invalid(
                            "a rename stays in its folder; use fs/move to move it",
                        ));
                    }
                    let name = to
                        .name()
                        .ok_or_else(|| invalid("the new name is empty"))?
                        .to_owned();
                    check_portable(&name)?;
                    renames.push((from, name));
                }
                let sources = renames.iter().map(|(p, _)| p.clone()).collect();
                let mut job = request(JobKind::Rename, sources);
                job.renames = renames;
                vec![job]
            }
            crate::protocol::methods::FS_COPY | crate::protocol::methods::FS_MOVE => {
                let sources = places_param(params)?;
                let to = place_param(params, "to")?;
                checked(&sources)?;
                checked(std::slice::from_ref(&to))?;
                for source in &sources {
                    if let Some(name) = source.name() {
                        check_portable(name)?;
                    }
                }
                let kind = if method == crate::protocol::methods::FS_COPY {
                    JobKind::Copy
                } else {
                    JobKind::Move
                };
                let mut job = request(kind, sources);
                job.destination = Some(to);
                job.conflicts = conflict_param(params)?;
                vec![job]
            }
            crate::protocol::methods::FS_DELETE => {
                let sources = places_param(params)?;
                checked(&sources)?;
                let permanent = params
                    .get("permanent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let kind = if permanent {
                    JobKind::DeletePermanently
                } else {
                    JobKind::Trash
                };
                vec![request(kind, sources)]
            }
            other => {
                return Err(RpcError::new(
                    ErrorKind::MethodNotFound,
                    format!("{other} is not a file job"),
                ));
            }
        })
    }

    /// Where a new folder is made, and the name (with the missing parents
    /// before it, separated by `/`, when `parents`).
    fn make_folder_parts(&self, place: &Place, parents: bool) -> Result<(Place, String), RpcError> {
        let mut names = Vec::new();
        let mut base = place.clone();
        loop {
            let name = base
                .name()
                .ok_or_else(|| invalid("a folder needs a name"))?
                .to_owned();
            check_portable(&name)?;
            names.push(name);
            base = base
                .parent()
                .ok_or_else(|| invalid("a folder needs a parent"))?;
            if !parents || self.exists(&base) {
                break;
            }
        }
        names.reverse();
        Ok((base, names.join("/")))
    }

    fn exists(&self, place: &Place) -> bool {
        self.provider(place)
            .ok()
            .is_some_and(|p| p.stat(place, &OpContext::new()).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::methods;

    fn params(value: Value) -> Params {
        value.as_object().cloned().unwrap()
    }

    fn files(root: &std::path::Path) -> (Files, Place) {
        let files = Files {
            registry: Some(Arc::new(Registry::with_local())),
            spool: Some(Arc::new(Spool::create(root.join("spool")).unwrap())),
            own_schemes: vec!["memfs".to_owned()],
            plugin: "Line counter".to_owned(),
        };
        let base = Place::parse(&root.join("work").display().to_string()).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        (files, base)
    }

    #[test]
    fn the_portable_name_policy() {
        for bad in [
            "a:b",
            "what?",
            "pipe|",
            "back\\slash",
            "trailing.",
            "trailing ",
            "CON",
            "nul.txt",
            "bidi\u{202E}gpj.exe",
            "\u{E000}private",
            "",
            "..",
        ] {
            assert!(portable_refusal(bad).is_some(), "{bad:?}");
        }
        for good in ["report.pdf", "a b c", "-leading-dash", "caf\u{e9}"] {
            assert!(portable_refusal(good).is_none(), "{good:?}");
        }
        assert_eq!(
            portable_refusal("trailing.").unwrap(),
            "the name trailing. ends in a dot or space"
        );
    }

    #[test]
    fn reads_go_through_the_provider_inline_or_into_the_spool() {
        let root = tempfile::tempdir().unwrap();
        let (files, base) = files(root.path());
        std::fs::write(root.path().join("work").join("a.txt"), b"0123456789").unwrap();
        let ctx = OpContext::new();
        let a = base.join("a.txt").unwrap().to_string();
        let listed = files
            .list(&params(json!({"place": base.to_string()})), &ctx)
            .unwrap();
        assert_eq!(listed["entries"][0]["name"], "a.txt");
        let stat = files.stat(&params(json!({"place": a})), &ctx).unwrap();
        assert_eq!(stat["size"], 10);
        let middle = files
            .read(&params(json!({"place": a, "offset": 2, "length": 3})), &ctx)
            .unwrap();
        assert_eq!(
            base64::decode(middle["data"].as_str().unwrap()).unwrap(),
            b"234"
        );
        assert_eq!(middle["eof"], false);
        let past = files
            .read(
                &params(json!({"place": a, "offset": 8, "length": 100})),
                &ctx,
            )
            .unwrap();
        assert_eq!(past["length"], 2);
        assert_eq!(past["eof"], true);
        let spooled = files
            .read(
                &params(json!({"place": a, "transfer": "file", "spool_path": "a.part"})),
                &ctx,
            )
            .unwrap();
        assert_eq!(spooled["length"], 10);
        assert_eq!(
            std::fs::read(root.path().join("spool").join("a.part")).unwrap(),
            b"0123456789"
        );
        let escaped = files
            .read(
                &params(json!({"place": a, "transfer": "file", "spool_path": "../a.part"})),
                &ctx,
            )
            .unwrap_err();
        assert_eq!(escaped.code, -32030);
        let too_big = files
            .read(&params(json!({"place": a, "length": 2_000_000})), &ctx)
            .unwrap_err();
        assert_eq!(too_big.code, -32021);
    }

    #[test]
    fn writes_commit_whole_and_never_make_a_bad_name() {
        let root = tempfile::tempdir().unwrap();
        let (files, base) = files(root.path());
        let ctx = OpContext::new();
        let new = base.join("new.txt").unwrap().to_string();
        let p = params(json!({"place": new, "data": base64::encode(b"hello")}));
        let (place, exists) = files.write_target(&p, &ctx).unwrap();
        assert!(!exists);
        files.write(&p, &place, false, &ctx).unwrap();
        assert_eq!(
            std::fs::read(root.path().join("work").join("new.txt")).unwrap(),
            b"hello"
        );
        let (_, exists) = files.write_target(&p, &ctx).unwrap();
        assert!(exists);
        let bad = base.join("bad").unwrap().to_string() + ".";
        let refused = files
            .write_target(&params(json!({"place": bad, "data": ""})), &ctx)
            .unwrap_err();
        assert_eq!(refused.code, -32016);
        std::fs::write(root.path().join("spool").join("in.part"), b"from spool").unwrap();
        let from_spool = params(json!({"place": new, "transfer": "file", "spool_path": "in.part"}));
        files.write(&from_spool, &place, true, &ctx).unwrap();
        assert_eq!(
            std::fs::read(root.path().join("work").join("new.txt")).unwrap(),
            b"from spool"
        );
    }

    #[test]
    fn file_requests_become_the_engines_jobs() {
        let root = tempfile::tempdir().unwrap();
        let (files, base) = files(root.path());
        let a = base.join("a.txt").unwrap();
        let out = base.join("out").unwrap();
        let copy = files
            .job(
                methods::FS_COPY,
                &params(json!({"items": [a.to_string()], "to": out.to_string(), "conflict": "keep_both"})),
            )
            .unwrap();
        assert_eq!(copy[0].kind, JobKind::Copy);
        assert_eq!(copy[0].sources, std::slice::from_ref(&a));
        assert_eq!(copy[0].destination, Some(out.clone()));
        assert_eq!(copy[0].conflicts, ConflictPolicy::KeepBoth);
        assert_eq!(copy[0].origin, JobOrigin::Plugin("Line counter".into()));
        let delete = files
            .job(
                methods::FS_DELETE,
                &params(json!({"items": [a.to_string()]})),
            )
            .unwrap();
        assert_eq!(delete[0].kind, JobKind::Trash);
        let gone = files
            .job(
                methods::FS_DELETE,
                &params(json!({"items": [a.to_string()], "permanent": true})),
            )
            .unwrap();
        assert_eq!(gone[0].kind, JobKind::DeletePermanently);
        let rename = files
            .job(
                methods::FS_RENAME,
                &params(
                    json!({"from": a.to_string(), "to": base.join("b.txt").unwrap().to_string()}),
                ),
            )
            .unwrap();
        assert_eq!(rename[0].renames, [(a.clone(), "b.txt".to_owned())]);
        let away = files
            .job(
                methods::FS_RENAME,
                &params(
                    json!({"from": a.to_string(), "to": out.join("b.txt").unwrap().to_string()}),
                ),
            )
            .unwrap_err();
        assert_eq!(away.code, -32602);
        let deep = base.join("x").unwrap().join("y").unwrap();
        let mkdir = files
            .job(
                methods::FS_MKDIR,
                &params(json!({"place": deep.to_string(), "parents": true})),
            )
            .unwrap();
        assert_eq!(mkdir[0].destination, Some(base.clone()));
        assert_eq!(mkdir[0].new_name.as_deref(), Some("x/y"));
        let bad = files
            .job(
                methods::FS_MKDIR,
                &params(json!({"place": base.join("CON").unwrap().to_string()})),
            )
            .unwrap_err();
        assert_eq!(bad.code, -32016);
    }

    #[test]
    fn a_place_the_plugin_serves_is_reentrant() {
        let root = tempfile::tempdir().unwrap();
        let (files, base) = files(root.path());
        let own = "memfs://box/a.txt";
        let refused = files
            .stat(&params(json!({"place": own})), &OpContext::new())
            .unwrap_err();
        assert_eq!(refused.code, -32032);
        let copy = files
            .job(
                methods::FS_COPY,
                &params(json!({"items": [own], "to": base.to_string()})),
            )
            .unwrap_err();
        assert_eq!(copy.code, -32032);
    }
}
