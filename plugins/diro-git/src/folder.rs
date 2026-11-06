//! What the plugin says about a folder on its own (R29 2.3): the title
//! line's words (`place/describe`) and the Git column (`column/format`),
//! and the first step every later one shares: git found, and new enough.
//!
//! The repository is found by its `.git`, which runs nothing, and git's
//! version is read once a session. Until the status reader and the trust
//! scan come, a folder in a repository says only that git is missing or
//! too old, and otherwise nothing: the title keeps its own words and the
//! column's cells stay "pending".

use std::path::PathBuf;
use std::sync::PoisonError;

use diroxide_core::Place;
use diroxide_plugins::protocol::Params;
use serde_json::{Value, json};

use crate::error::GitError;
use crate::git;
use crate::git::version::Version;
use crate::host::Host;
use crate::lines;
use crate::plugin::Plugin;

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

/// The words for a repository's folder when git cannot serve it, or
/// `None` when it can.
fn missing_words(plugin: &Plugin, host: Host<'_>) -> Result<Option<String>, GitError> {
    match git_ready(plugin, host) {
        Ok(_) => Ok(None),
        Err(GitError::NotFound) => Ok(Some(lines::GIT_NOT_FOUND.to_owned())),
        Err(GitError::TooOld(v)) => Ok(Some(lines::too_old(v))),
        Err(other) => Err(other),
    }
}

/// `place/describe`: git missing or too old says so; otherwise no words
/// yet.
pub fn describe(plugin: &Plugin, host: Host<'_>, params: &Params) -> Result<Value, GitError> {
    let Some(folder) = params
        .get("place")
        .and_then(Value::as_str)
        .and_then(local_path)
    else {
        return Ok(json!({}));
    };
    if plugin.find_repo(&folder).is_none() {
        return Ok(json!({}));
    }
    Ok(match missing_words(plugin, host)? {
        Some(words) => json!({"text": words}),
        None => json!({}),
    })
}

/// `column/format`: git missing or too old in every cell; otherwise no
/// values yet.
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
    let in_repo = params
        .get("folder")
        .and_then(Value::as_str)
        .and_then(local_path)
        .is_some_and(|folder| plugin.find_repo(&folder).is_some());
    if !in_repo {
        return Ok(json!({"values": {}}));
    }
    let Some(words) = missing_words(plugin, host)? else {
        return Ok(json!({"values": {}}));
    };
    let values: serde_json::Map<String, Value> =
        places.into_iter().map(|p| (p, json!(words))).collect();
    Ok(json!({"values": values}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_or_remote_place_is_no_local_folder() {
        assert!(local_path("sftp://host/home").is_none());
        let here = std::env::current_dir().unwrap();
        assert_eq!(local_path(&here.display().to_string()), Some(here));
    }
}
