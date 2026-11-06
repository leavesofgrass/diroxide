//! What the plugin says about a folder on its own (R29 2.3 and 2.5; answer
//! 123): the title line's words (`place/describe`) and the Git column
//! (`column/format`), and the steps every command shares: git found and
//! new enough, the trust scan, and a folder's status.
//!
//! The order never changes: find the repository by its `.git`, which runs
//! nothing; read git's version once; scan the repository's own
//! configuration, which runs nothing; and only for a clean or trusted
//! configuration run `git status`, automatic, with `-c
//! core.fsmonitor=false` and `--no-optional-locks`. The title line asks the
//! trust question once a session per repository; the column never asks,
//! it says "not checked".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use diroxide_core::Place;
use diroxide_plugins::protocol::{ErrorKind, Params};
use serde_json::{Value, json};

use crate::cache::{COLUMN_FRESH, Kept, TITLE_FRESH};
use crate::error::GitError;
use crate::git::status::{self, Status};
use crate::git::trust::{self, SCAN_ARGS, Scan};
use crate::git::version::Version;
use crate::git::{self, Run};
use crate::host::Host;
use crate::lines;
use crate::plugin::Plugin;
use crate::repo::Repo;

/// A place's local path, or `None` for an archive or remote place.
pub fn local_path(place: &str) -> Option<PathBuf> {
    let place = Place::parse(place).ok()?;
    place.local_path().map(PathBuf::from)
}

/// git found and new enough: its version, read once a session.
pub fn git_ready(plugin: &Plugin, host: Host<'_>) -> Result<Version, GitError> {
    let known = *plugin
        .version
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let version = match known {
        Some(version) => version,
        None => {
            let version = git::version(host.peer, Some(host.cancel))?;
            *plugin
                .version
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(version);
            version
        }
    };
    if version.is_enough() {
        Ok(version)
    } else {
        Err(GitError::TooOld(version))
    }
}

/// The trust scan of a repository, kept until its configuration changes.
pub fn scan(plugin: &Plugin, host: Host<'_>, repo: &Repo) -> Result<Arc<Scan>, GitError> {
    let stamps = repo.config_stamps();
    if let Some(scan) = plugin.scans.get(&repo.root, stamps) {
        return Ok(scan);
    }
    let out = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &SCAN_ARGS).automatic(),
    )?;
    if !out.ok() {
        return Err(GitError::refused(out.reason(), None));
    }
    let scan = Arc::new(trust::scan(&out.stdout));
    plugin.scans.put(&repo.root, stamps, Arc::clone(&scan));
    Ok(scan)
}

/// Where a repository stands with the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Its configuration runs no program.
    Clean,
    /// It does, and the owner said yes to this configuration.
    Yes,
    /// It does, and the owner said no.
    No,
    /// It does, and the owner has not answered.
    Unasked,
}

/// Where a repository stands, from its scan and the owner's answers.
pub fn trust_of(plugin: &Plugin, repo: &Repo, scan: &Scan) -> Trust {
    if scan.is_clean() {
        return Trust::Clean;
    }
    match plugin.trust.answer(&repo.root, &scan.digest) {
        Some(true) => Trust::Yes,
        Some(false) => Trust::No,
        None => Trust::Unasked,
    }
}

/// Asks the trust question, starting on No, and keeps a yes or a no; an
/// Escape keeps nothing. True for yes.
pub fn ask_trust(
    plugin: &Plugin,
    host: Host<'_>,
    repo: &Repo,
    scan: &Scan,
) -> Result<bool, GitError> {
    let hits: Vec<String> = scan.hits.iter().map(trust::Hit::words).collect();
    let lines = lines::trust_lines(&repo.root.display().to_string(), &hits);
    let answer = host.confirm(lines::TRUST_TITLE, &lines, true)?;
    if let Some(yes) = answer {
        // An answer that cannot be written still holds for the session.
        let _ = plugin.trust.record(&repo.root, &scan.digest, yes);
    }
    Ok(answer == Some(true))
}

/// For a command: the repository's configuration runs nothing, or the
/// owner trusts it; asks once when the owner has not answered.
pub fn ensure_trusted(plugin: &Plugin, host: Host<'_>, repo: &Repo) -> Result<(), GitError> {
    let scan = scan(plugin, host, repo)?;
    match trust_of(plugin, repo, &scan) {
        Trust::Clean | Trust::Yes => Ok(()),
        Trust::No => Err(GitError::NotTrusted),
        Trust::Unasked => {
            plugin.first_ask(&repo.root);
            if ask_trust(plugin, host, repo, &scan)? {
                Ok(())
            } else {
                Err(GitError::NotTrusted)
            }
        }
    }
}

/// The status of a folder and below, automatic, kept for `fresh`; a
/// status slower than its limit is taken again without untracked files.
pub fn folder_status(
    plugin: &Plugin,
    host: Host<'_>,
    repo: &Repo,
    folder: &Path,
    fresh: Duration,
) -> Result<Kept, GitError> {
    let relative = repo.relative(folder).unwrap_or_default();
    let lock = plugin.statuses.lock_for(&repo.root);
    let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let stamps = repo.status_stamps(folder);
    if let Some(kept) = plugin.statuses.get(&repo.root, &relative, stamps, fresh) {
        return Ok(kept);
    }
    let run = |untracked: &str| {
        git::run(
            host.peer,
            Some(host.cancel),
            &Run::new(
                folder,
                &["status", "--porcelain=v2", "-z", "--branch", untracked],
            )
            .automatic()
            .with_paths(&["."]),
        )
    };
    let (out, untracked_counted) = match run("--untracked-files=normal") {
        Err(GitError::Host(e)) if e.code == ErrorKind::TimedOut.code() => {
            (run("--untracked-files=no")?, false)
        }
        other => (other?, true),
    };
    if !out.ok() {
        return Err(GitError::refused(out.reason(), None));
    }
    let status = status::parse(&out.stdout).map_err(|e| GitError::Bad(e.to_string()))?;
    let kept = Kept {
        status: Arc::new(status),
        untracked_counted,
        taken: Instant::now(),
        stamps,
    };
    plugin.statuses.put(&repo.root, &relative, kept.clone());
    Ok(kept)
}

/// The title line's words for a folder's status.
pub fn title_of(repo: &Repo, folder_relative: &str, kept: &Kept) -> String {
    let status: &Status = &kept.status;
    let here = status.under(folder_relative).count();
    lines::title(&lines::TitleFacts {
        branch: status.branch.clone(),
        operation: repo.operation(),
        here,
        conflicts: status
            .under(folder_relative)
            .filter(|c| c.is_conflict())
            .count(),
        untracked_counted: kept.untracked_counted,
    })
}

/// `place/describe`: the title line's words, or none for a place outside
/// a repository; git missing or too old says so.
pub fn describe(plugin: &Plugin, host: Host<'_>, params: &Params) -> Result<Value, GitError> {
    let Some(folder) = params
        .get("place")
        .and_then(Value::as_str)
        .and_then(local_path)
    else {
        return Ok(json!({}));
    };
    let Some(repo) = plugin.find_repo(&folder) else {
        return Ok(json!({}));
    };
    match git_ready(plugin, host) {
        Ok(_) => {}
        Err(GitError::NotFound) => return Ok(json!({"text": lines::GIT_NOT_FOUND})),
        Err(GitError::TooOld(v)) => return Ok(json!({"text": lines::too_old(v)})),
        Err(other) => return Err(other),
    }
    let scan = scan(plugin, host, &repo)?;
    let trusted = match trust_of(plugin, &repo, &scan) {
        Trust::Clean | Trust::Yes => true,
        Trust::No => false,
        // Asked once a session; meanwhile, and after a no or an Escape,
        // the title says why there is no status.
        Trust::Unasked => plugin.first_ask(&repo.root) && ask_trust(plugin, host, &repo, &scan)?,
    };
    if !trusted {
        return Ok(json!({"text": lines::NOT_TRUSTED}));
    }
    let kept = folder_status(plugin, host, &repo, &folder, TITLE_FRESH)?;
    let relative = repo.relative(&folder).unwrap_or_default();
    Ok(json!({"text": title_of(&repo, &relative, &kept)}))
}

/// The Git column's cell for one place inside the folder.
fn cell(status: &Status, relative: &str, unlisted: &mut Vec<String>) -> Option<String> {
    if let Some(change) = status
        .changes
        .iter()
        .find(|c| c.bare_path() == relative && !matches!(c.kind, status::Kind::Ignored))
    {
        return Some(change.word());
    }
    let inside = status.under(relative).count();
    if inside > 0 {
        return Some(lines::changed_inside(inside));
    }
    unlisted.push(relative.to_owned());
    None
}

/// Which of these paths (from the top) git ignores: `git check-ignore`,
/// automatic, over standard input.
fn ignored(host: Host<'_>, repo: &Repo, paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return Vec::new();
    }
    let mut input = String::new();
    for path in paths {
        input.push_str(path);
        input.push('\0');
    }
    let run = Run::new(&repo.root, &["check-ignore", "-z", "--stdin"])
        .automatic()
        .with_stdin(input);
    match git::run(host.peer, Some(host.cancel), &run) {
        // 0: some are ignored; 1: none is.
        Ok(out) if out.code == Some(0) => out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect(),
        _ => Vec::new(),
    }
}

/// `column/format`: a page of the Git column, at most 500 places of one
/// folder, each cell a word ("Modified", "Clean", "3 changed inside").
pub fn column(plugin: &Plugin, host: Host<'_>, params: &Params) -> Result<Value, GitError> {
    let places: Vec<String> = params
        .get("places")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let folder = params
        .get("folder")
        .and_then(Value::as_str)
        .and_then(local_path);
    let same = |text: &str| -> Value {
        Value::Object(places.iter().map(|p| (p.clone(), json!(text))).collect())
    };
    let Some(folder) = folder else {
        return Ok(json!({"values": same(lines::NO_REPOSITORY)}));
    };
    let Some(repo) = plugin.find_repo(&folder) else {
        return Ok(json!({"values": same(lines::NO_REPOSITORY)}));
    };
    match git_ready(plugin, host) {
        Ok(_) => {}
        Err(GitError::NotFound) => return Ok(json!({"values": same(lines::GIT_NOT_FOUND)})),
        Err(GitError::TooOld(v)) => return Ok(json!({"values": same(&lines::too_old(v))})),
        Err(other) => return Err(other),
    }
    let scan = scan(plugin, host, &repo)?;
    if !matches!(trust_of(plugin, &repo, &scan), Trust::Clean | Trust::Yes) {
        return Ok(json!({"values": same(lines::NOT_CHECKED)}));
    }
    let kept = folder_status(plugin, host, &repo, &folder, COLUMN_FRESH)?;
    let mut values = BTreeMap::new();
    let mut unlisted = Vec::new();
    let mut waiting = Vec::new();
    for place in &places {
        let Some(relative) = local_path(place).and_then(|p| repo.relative(&p)) else {
            values.insert(place.clone(), lines::NO_REPOSITORY.to_owned());
            continue;
        };
        match cell(&kept.status, &relative, &mut unlisted) {
            Some(word) => {
                values.insert(place.clone(), word);
            }
            None => waiting.push((place.clone(), relative)),
        }
    }
    let ignored = ignored(host, &repo, &unlisted);
    for (place, relative) in waiting {
        let word = if ignored.iter().any(|i| i.trim_end_matches('/') == relative) {
            lines::IGNORED
        } else {
            lines::CLEAN
        };
        values.insert(place, word.to_owned());
    }
    Ok(json!({"values": values}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::status::parse;

    const OID: &str = "100644 100644 100644 5626abf0f72e58d7a153368ba57db4c673c0e171 5626abf0f72e58d7a153368ba57db4c673c0e171";

    #[test]
    fn a_cell_is_the_file_s_state_or_what_changed_inside_a_folder() {
        let mut out = Vec::new();
        for record in [
            format!("1 .M N... {OID} a.txt"),
            format!("1 MM N... {OID} sub/c.txt"),
            format!("1 A. N... {OID} sub/deep/d.txt"),
            "? new/".to_owned(),
            "! target/".to_owned(),
        ] {
            out.extend_from_slice(record.as_bytes());
            out.push(0);
        }
        let status = parse(&out).unwrap();
        let mut unlisted = Vec::new();
        assert_eq!(
            cell(&status, "a.txt", &mut unlisted).as_deref(),
            Some("Modified")
        );
        assert_eq!(
            cell(&status, "sub", &mut unlisted).as_deref(),
            Some("2 changed inside")
        );
        assert_eq!(
            cell(&status, "new", &mut unlisted).as_deref(),
            Some("Untracked")
        );
        assert_eq!(cell(&status, "b.txt", &mut unlisted), None);
        assert_eq!(cell(&status, "target", &mut unlisted), None);
        assert_eq!(unlisted, ["b.txt", "target"]);
    }

    #[test]
    fn an_archive_or_remote_place_is_no_local_folder() {
        assert!(local_path("sftp://host/home").is_none());
        let here = std::env::current_dir().unwrap();
        assert_eq!(local_path(&here.display().to_string()), Some(here));
    }
}
