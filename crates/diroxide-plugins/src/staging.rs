//! Host services, part four, continued: staged outputs (`fs/stage` and
//! `fs/commit`; alpha 4, lane AE, E's request 1), so a plugin that makes
//! a file writes it where diroxide's own jobs write theirs and the owner
//! gets the engine's undo and trash for it.
//!
//! - **`fs/stage`** `{"place": OUTPUT}` hands out a staging path for an
//!   output the plugin is about to create: a `.diro-part-` name (eight
//!   hexadecimal digits and the output's extension) in the output's own
//!   folder, the name the engine's jobs stage under, which the arrival
//!   tracker and a hot folder ignore. The folder must be on this computer
//!   and the output's name free and portable. The answer is `{"stage":
//!   ID, "path": PATH}`; the plugin writes the whole output to `PATH`
//!   itself. At most [`MAX_STAGES`] stages are open at once.
//! - **`fs/commit`** `{"stage": ID}` moves a staged output into place as a
//!   job: renamed in its folder, never over a file (a name taken meanwhile
//!   is numbered as copies are, "report.pdf (2).age"), and recorded as
//!   made, so Undo puts it in the trash. The answer is `{"committed":
//!   true, "place": PLACE, "name": NAME}`; the app hears the job through
//!   [`crate::host::HostServices::committed`].
//! - A stage never committed is removed when the plugin's session ends:
//!   only the staging file the host named, and only when it is a file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_core::{Entry, EntryKind, JobId, JobKind, JobOrigin, Place};
use diroxide_ops::JobReport;
use diroxide_ops::convert::staging::{staging_name, token};
use diroxide_ops::undo::{LeftAs, UndoRecord};
use diroxide_vfs::names::numbered_name;
use diroxide_vfs::{OpContext, VfsError};
use serde_json::{Value, json};

use crate::files::{Files, portable_refusal, vfs_to_rpc};
use crate::places::plugin_place;
use crate::protocol::{ErrorKind, Params, RpcError};

/// The most stages one plugin may hold open at once.
pub const MAX_STAGES: usize = 64;

/// How many numbered names a commit tries when its name is taken.
const NUMBERED_TRIES: u32 = 999;

/// One staged output.
#[derive(Debug)]
struct Stage {
    /// The staging file as a place, in the output's folder.
    place: Place,
    /// The same file as a path, which the plugin writes.
    path: PathBuf,
    /// Where it goes when it is committed.
    target: Place,
}

/// The open stages of one plugin's session.
#[derive(Debug, Default)]
struct Table {
    open: HashMap<String, Stage>,
}

impl Drop for Table {
    fn drop(&mut self) {
        // A stage never committed: its staging file, only when it is one.
        for stage in self.open.values() {
            if std::fs::symlink_metadata(&stage.path).is_ok_and(|m| m.is_file()) {
                let _ = std::fs::remove_file(&stage.path);
            }
        }
    }
}

/// The stages a plugin's session holds, shared by its calls.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stages(Arc<Mutex<Table>>);

impl Stages {
    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

/// The staging file's extension: the output's own, when it is a plain
/// one, so a tool reads the format from it; else `part`.
fn staging_extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=10).contains(&ext.len())
                && ext.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            ext.to_ascii_lowercase()
        }
        _ => "part".to_owned(),
    }
}

impl Files {
    /// `fs/stage`: a staging path for an output the plugin creates.
    pub(crate) fn stage(&self, params: &Params, ctx: &OpContext) -> Result<Value, RpcError> {
        let text = params
            .get("place")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("place is missing"))?;
        let target = plugin_place(text)?;
        let name = target
            .name()
            .ok_or_else(|| invalid("an output needs a name"))?
            .to_owned();
        if let Some(why) = portable_refusal(&name) {
            return Err(RpcError::new(ErrorKind::InvalidName, why));
        }
        self.not_reentrant(&target)?;
        let folder = target
            .parent()
            .ok_or_else(|| invalid("an output needs a folder"))?;
        let Some(local_folder) = folder.local_path().map(PathBuf::from) else {
            return Err(RpcError::new(
                ErrorKind::Unsupported,
                "only a folder on this computer takes a staged output",
            ));
        };
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host reaches no files"))?;
        let provider = registry.resolve(&folder).map_err(|e| vfs_to_rpc(&e))?;
        match provider.stat(&folder, ctx) {
            Ok(entry) if entry.kind == EntryKind::Folder => {}
            Ok(_) => {
                return Err(RpcError::new(
                    ErrorKind::NotADirectory,
                    "the output's folder is not a folder",
                ));
            }
            Err(e) => return Err(vfs_to_rpc(&e)),
        }
        match provider.stat(&target, ctx) {
            Ok(_) => {
                return Err(RpcError::new(
                    ErrorKind::AlreadyExists,
                    format!("{name} exists; stage another name"),
                ));
            }
            Err(VfsError::NotFound) => {}
            Err(e) => return Err(vfs_to_rpc(&e)),
        }
        let mut table = self.stages.table();
        if table.open.len() >= MAX_STAGES {
            return Err(RpcError::new(
                ErrorKind::Unavailable,
                format!("{MAX_STAGES} outputs are staged already; commit them first"),
            ));
        }
        let extension = staging_extension(&name);
        let free = (0..8).find_map(|_| {
            let staged = staging_name(token(), &extension);
            let place = folder.join(&staged).ok()?;
            matches!(provider.stat(&place, ctx), Err(VfsError::NotFound)).then(|| (staged, place))
        });
        let Some((staged, place)) = free else {
            return Err(RpcError::new(
                ErrorKind::Failed,
                "no staging name was free in the output's folder",
            ));
        };
        let id = staged
            .trim_start_matches(diroxide_vfs::names::UNFINISHED_PREFIX)
            .split('.')
            .next()
            .unwrap_or_default()
            .to_owned();
        let path = local_folder.join(&staged);
        let answer = json!({"stage": id, "path": path.display().to_string()});
        table.open.insert(
            id,
            Stage {
                place,
                path,
                target,
            },
        );
        Ok(answer)
    }

    /// `fs/commit`: the staged output moved into place, never over a
    /// file; the answer, and the job's report for the app's undo.
    pub(crate) fn commit(
        &self,
        params: &Params,
        ctx: &OpContext,
    ) -> Result<(Value, JobReport), RpcError> {
        let id = params
            .get("stage")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("stage is missing"))?;
        let stage =
            self.stages.table().open.remove(id).ok_or_else(|| {
                RpcError::new(ErrorKind::NotFound, format!("no stage is named {id}"))
            })?;
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host reaches no files"))?;
        // What was written must be one plain file: never a folder or a link.
        let kept = std::fs::symlink_metadata(&stage.path);
        match &kept {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    "the stage holds something that is not a file",
                ));
            }
            Err(_) => {
                return Err(RpcError::new(
                    ErrorKind::NotFound,
                    "nothing was written to the stage",
                ));
            }
        }
        let provider = registry
            .resolve(&stage.target)
            .map_err(|e| vfs_to_rpc(&e))?;
        let folder = stage
            .target
            .parent()
            .ok_or_else(|| invalid("an output needs a folder"))?;
        let name = stage.target.name().unwrap_or_default().to_owned();
        let discard = || {
            let _ = std::fs::remove_file(&stage.path);
        };
        let mut placed = None;
        for n in 1..=NUMBERED_TRIES {
            let candidate = if n == 1 {
                name.clone()
            } else {
                numbered_name(&name, n)
            };
            let target = folder
                .join(&candidate)
                .map_err(|e| vfs_to_rpc(&VfsError::from(e)))?;
            match provider.rename(&stage.place, &target, ctx) {
                Ok(()) => {
                    placed = Some((target, candidate));
                    break;
                }
                Err(VfsError::AlreadyExists) => {}
                Err(e) => {
                    discard();
                    return Err(vfs_to_rpc(&e));
                }
            }
        }
        let Some((target, made)) = placed else {
            discard();
            return Err(RpcError::new(
                ErrorKind::AlreadyExists,
                format!("no free name was left for {name}"),
            ));
        };
        let entry = provider
            .stat(&target, ctx)
            .unwrap_or_else(|_| Entry::new(made.clone(), EntryKind::File));
        let job = JobId::new(0);
        let mut report = JobReport::new(job, JobKind::PluginCommand);
        report.done.push(target.clone());
        report.undo = UndoRecord::for_job(job, JobKind::PluginCommand);
        report.undo.origin = JobOrigin::Plugin(self.plugin.clone());
        report.undo.created.push(target.clone());
        report.undo.left.push(LeftAs::of(&target, &entry));
        let answer = json!({"committed": true, "place": target.to_string(), "name": made});
        Ok((answer, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_staging_extension_is_the_outputs_when_plain() {
        assert_eq!(staging_extension("report.pdf.age"), "age");
        assert_eq!(staging_extension("Photo.JPG"), "jpg");
        assert_eq!(staging_extension("README"), "part");
        assert_eq!(staging_extension(".profile"), "part");
        assert_eq!(staging_extension("odd.ex-t"), "part");
        assert_eq!(staging_extension("long.abcdefghijk"), "part");
    }
}
