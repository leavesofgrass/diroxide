//! A file's changes in diroxide's own compare screens (R29 2.4; R18 section
//! 2; the host-pieces lane's `ui/show_diff`): git supplies the old text
//! (`git cat-file blob REV:PATH`, which runs no filter and no text
//! converter), the working tree the new, and the host reads them in R18's
//! order, the verdict first, so a git diff sounds like every other diff.
//!
//! Which two: the index against the working tree when the file has
//! unstaged changes, HEAD against the index when it has only staged ones,
//! nothing against the working tree for an untracked file, ours against
//! theirs in a conflict, and the commit's parent against the commit on a
//! row of a commit's file list. "Compare with the last commit" is HEAD
//! against the working tree. Each side's title stays short ("HEAD",
//! "index", "working tree", HP-10).
//!
//! A binary file is said in words with its two sizes, since a diff side
//! carries text only (HP-8); a side over [`MAX_SIDE`] is too large to
//! compare here.

use serde_json::{Value, json};

use super::tree::read_status;
use super::{Ctx, RowData, Target, short_name};
use crate::error::GitError;
use crate::git::status::Kind;
use crate::git::{self, Run};
use crate::lines;
use crate::repo::Repo;

/// The most bytes one side may hold: two sides and their escaping stay
/// inside the host's 8 MiB line.
pub const MAX_SIDE: u64 = 2 * 1024 * 1024;

/// How far a NUL is looked for to call a file binary, as git does.
const BINARY_PROBE: usize = 8000;

/// A side's title: the working tree.
pub const WORKING_TREE: &str = "working tree";
/// The index.
pub const INDEX: &str = "index";
/// The last commit.
pub const HEAD: &str = "HEAD";
/// Ours, in a conflict.
pub const OURS: &str = "ours";
/// Theirs, in a conflict.
pub const THEIRS: &str = "theirs";
/// An untracked file's old side.
pub const NOT_TRACKED: &str = "not tracked";

/// A folder was picked.
pub const NOT_A_FILE: &str = "Not a file: pick a file to compare";

/// One side: its title and its bytes (empty when it is not there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// Its title.
    pub title: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// Whether bytes look binary: a NUL in the first 8,000.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_PROBE).any(|b| *b == 0)
}

/// A blob's bytes, or nothing when it is not there (a new file's old
/// side, a deleted file's new side).
fn blob(ctx: &Ctx<'_>, repo: &Repo, spec: &str) -> Result<Vec<u8>, GitError> {
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["cat-file", "blob", spec]),
    )?;
    if out.truncated {
        return Err(GitError::refused(
            lines::too_large(spec, git::MAX_OUTPUT),
            None,
        ));
    }
    Ok(if out.ok() { out.stdout } else { Vec::new() })
}

/// The working tree's bytes of a path, or nothing when it is gone.
fn working(repo: &Repo, path: &str) -> Result<Vec<u8>, GitError> {
    let file = repo.root.join(path);
    match std::fs::metadata(&file) {
        Ok(meta) if meta.is_dir() => Err(GitError::refused(NOT_A_FILE, None)),
        Ok(meta) if meta.len() > MAX_SIDE => Err(GitError::refused(
            lines::too_large(short_name(path), meta.len()),
            None,
        )),
        Ok(_) => std::fs::read(&file).map_err(|e| GitError::Bad(e.to_string())),
        Err(_) => Ok(Vec::new()),
    }
}

fn side(title: &str, bytes: Vec<u8>) -> Side {
    Side {
        title: title.to_owned(),
        bytes,
    }
}

/// Shows two sides, or says why not: binary in words, too large.
pub fn show(ctx: &Ctx<'_>, title: &str, old: Side, new: Side) -> Result<Value, GitError> {
    let name = short_name(title);
    let largest = old.bytes.len().max(new.bytes.len()) as u64;
    if largest > MAX_SIDE {
        return Err(GitError::refused(lines::too_large(name, largest), None));
    }
    if old.bytes == new.bytes {
        return Ok(ctx.done(None, &lines::no_changes(name), 0));
    }
    if is_binary(&old.bytes) || is_binary(&new.bytes) {
        let sentence = lines::binary(name, old.bytes.len() as u64, new.bytes.len() as u64);
        return Ok(ctx.done(None, &sentence, 1));
    }
    ctx.host.show_diff(
        title,
        (&old.title, &String::from_utf8_lossy(&old.bytes)),
        (&new.title, &String::from_utf8_lossy(&new.bytes)),
    )?;
    Ok(json!({"done": 1, "skipped": 0, "failed": []}))
}

/// A row of a commit's file list: the parent against the commit.
fn commit_file(ctx: &Ctx<'_>, repo: &Repo, row: &RowData, hash: &str) -> Result<Value, GitError> {
    let path = row.path.clone().unwrap_or_default();
    let short: String = hash.chars().take(7).collect();
    let before = row.from.clone().unwrap_or_else(|| path.clone());
    let old = blob(ctx, repo, &format!("{hash}^:{before}"))?;
    let new = blob(ctx, repo, &format!("{hash}:{path}"))?;
    show(
        ctx,
        &path,
        side(&lines::before(&short), old),
        side(&short, new),
    )
}

/// Git: show the changes, and Git: compare with the last commit (`head`).
pub fn diff(ctx: &Ctx<'_>, head: bool) -> Result<Value, GitError> {
    let (repo, paths, targets) = ctx.target_paths()?;
    if let Some(Target::Row(row)) = targets.first()
        && let Some(hash) = &row.hash
        && row.path.is_some()
    {
        return commit_file(ctx, &repo, row, hash);
    }
    if matches!(targets.first(), Some(Target::File { folder: true, .. })) {
        return Err(GitError::refused(NOT_A_FILE, None));
    }
    let Some(path) = paths.first().cloned() else {
        return Err(GitError::refused(NOT_A_FILE, None));
    };
    if head {
        let old = blob(ctx, &repo, &format!("HEAD:{path}"))?;
        let new = working(&repo, &path)?;
        return show(ctx, &path, side(HEAD, old), side(WORKING_TREE, new));
    }
    let status = read_status(ctx.host, &repo, std::slice::from_ref(&path))?;
    let Some(change) = status
        .changes
        .iter()
        .find(|c| c.bare_path() == path.trim_end_matches('/'))
    else {
        return Ok(ctx.done(None, &lines::no_changes(short_name(&path)), 0));
    };
    if path.ends_with('/') || change.path.ends_with('/') {
        return Err(GitError::refused(NOT_A_FILE, None));
    }
    let index = format!(":{path}");
    let (old, new) = match change.kind {
        Kind::Conflict { .. } => (
            side(OURS, blob(ctx, &repo, &format!(":2:{path}"))?),
            side(THEIRS, blob(ctx, &repo, &format!(":3:{path}"))?),
        ),
        Kind::Untracked | Kind::Ignored => (
            side(NOT_TRACKED, Vec::new()),
            side(WORKING_TREE, working(&repo, &path)?),
        ),
        _ if change.in_tree() => (
            side(INDEX, blob(ctx, &repo, &index)?),
            side(WORKING_TREE, working(&repo, &path)?),
        ),
        _ => {
            let from = change.from.clone().unwrap_or_else(|| path.clone());
            (
                side(HEAD, blob(ctx, &repo, &format!("HEAD:{from}"))?),
                side(INDEX, blob(ctx, &repo, &index)?),
            )
        }
    };
    show(ctx, &path, old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_is_a_nul_near_the_start_and_titles_stay_short() {
        assert!(is_binary(b"PNG\0\x01"));
        assert!(!is_binary(b"plain text\n"));
        let mut late = vec![b'a'; BINARY_PROBE];
        late.push(0);
        assert!(!is_binary(&late));
        for title in [WORKING_TREE, INDEX, HEAD, OURS, THEIRS, NOT_TRACKED] {
            assert!(title.chars().count() <= 16, "{title}");
        }
        assert!(lines::before("a1b2c3d").chars().count() <= 16);
        assert!(NOT_A_FILE.chars().count() <= 40);
    }
}
