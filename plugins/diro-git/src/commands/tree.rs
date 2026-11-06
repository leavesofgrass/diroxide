//! The working tree's commands (R29 2.4): the status list, stage, unstage,
//! stage all, Add to .gitignore, and Discard, which keeps what it drops.
//!
//! Paths always come after `--` with literal pathspecs, from the
//! repository's top. Discard is `git stash push --keep-index -m "diroxide
//! discard TIME" -- PATHS` on tracked files only (spike GS5): the files go
//! back to what is staged, staged work stays, untracked files are never
//! touched (that would be `git clean`, which does not exist here), and the
//! stash brings it all back.

use std::time::SystemTime;

use diroxide_core::{Timestamp, UtcOffset};
use serde_json::Value;

use super::{Ctx, Row, RowData, Target, View, git_refused, place_of};
use crate::error::GitError;
use crate::git::status::{self, Change, Status};
use crate::git::{self, Run};
use crate::host::Host;
use crate::lines;
use crate::repo::Repo;

/// The repository's changes, as the status list's rows.
pub fn status_rows(host: Host<'_>, repo: &Repo) -> Result<(Status, Vec<Row>), GitError> {
    let status = read_status(host, repo, &[])?;
    let rows = status
        .changes
        .iter()
        .filter(|c| !matches!(c.kind, status::Kind::Ignored))
        .map(|c| row_of(repo, c))
        .collect();
    Ok((status, rows))
}

fn row_of(repo: &Repo, change: &Change) -> Row {
    let word = change.word();
    Row {
        name: lines::change_row(&word, &change.path, change.from.as_deref()),
        columns: vec![("state", word.clone()), ("path", change.path.clone())],
        data: RowData {
            root: repo.root.clone(),
            path: Some(change.path.clone()),
            from: change.from.clone(),
            word,
            ..RowData::default()
        },
    }
}

/// The status of the whole repository, or of some paths, as a read: no
/// optional locks, no file monitor.
pub fn read_status(host: Host<'_>, repo: &Repo, paths: &[String]) -> Result<Status, GitError> {
    read_status_with(host, repo, paths, "--untracked-files=normal")
}

/// The same, with git's choice for untracked files: `normal` lists a new
/// folder as one entry, `all` lists every file in it.
pub fn read_status_with(
    host: Host<'_>,
    repo: &Repo,
    paths: &[String],
    untracked: &str,
) -> Result<Status, GitError> {
    let mut run = Run::new(
        &repo.root,
        &["status", "--porcelain=v2", "-z", "--branch", untracked],
    )
    .automatic();
    if !paths.is_empty() {
        run = run.with_paths(paths);
    }
    let out = git::run(host.peer, Some(host.cancel), &run)?;
    if !out.ok() {
        return Err(git_refused("Could not read the status", &out.reason()));
    }
    status::parse(&out.stdout).map_err(|e| GitError::Bad(e.to_string()))
}

/// Git: status: the repository's changed files as a list.
pub fn status(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let (status, rows) = status_rows(ctx.host, &repo)?;
    let title = lines::status_title(rows.len(), &status.branch);
    ctx.open(
        "status",
        &title,
        View::Status {
            root: repo.root.clone(),
        },
        rows,
    )
}

/// After a change made from the status list, the list again.
fn refresh(ctx: &Ctx<'_>, repo: &Repo) -> Result<(), GitError> {
    if ctx.lister() != Some("status") {
        return Ok(());
    }
    let (status, rows) = status_rows(ctx.host, repo)?;
    let title = lines::status_title(rows.len(), &status.branch);
    ctx.open(
        "status",
        &title,
        View::Status {
            root: repo.root.clone(),
        },
        rows,
    )
    .map(drop)
}

fn paths_or_nothing(paths: Vec<String>) -> Result<Vec<String>, GitError> {
    if paths.is_empty() {
        Err(GitError::refused(lines::NOTHING_CHANGED, None))
    } else {
        Ok(paths)
    }
}

/// Git: stage: `git add -- PATHS`.
pub fn stage(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, paths, _) = ctx.target_paths()?;
    let paths = paths_or_nothing(paths)?;
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["add"]).with_paths(&paths),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not stage", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::staged(&paths), paths.len());
    refresh(ctx, &repo)?;
    Ok(done)
}

/// Whether the branch has a commit yet.
pub fn has_head(host: Host<'_>, repo: &Repo) -> Result<bool, GitError> {
    let out = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &["rev-parse", "--verify", "-q", "HEAD"]),
    )?;
    Ok(out.ok())
}

/// Git: unstage: `git restore --staged -- PATHS`, or before the first
/// commit `git rm --cached`.
pub fn unstage(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, paths, _) = ctx.target_paths()?;
    let paths = paths_or_nothing(paths)?;
    let args: &[&str] = if has_head(ctx.host, &repo)? {
        &["restore", "--staged"]
    } else {
        &["rm", "--cached", "-r", "-q"]
    };
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, args).with_paths(&paths),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not unstage", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::unstaged(&paths), paths.len());
    refresh(ctx, &repo)?;
    Ok(done)
}

/// Git: stage all changes here: `git add --all -- .` in the folder.
pub fn stage_all(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let folder = ctx.folder()?;
    let relative = repo.relative(&folder).unwrap_or_default();
    let before = read_status_with(ctx.host, &repo, &[], "--untracked-files=all")?;
    let count = before
        .under(&relative)
        .filter(|c| c.in_tree() || matches!(c.kind, status::Kind::Untracked))
        .count();
    if count == 0 {
        return Err(GitError::refused(lines::NOTHING_CHANGED, None));
    }
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&folder, &["add", "--all"]).with_paths(&["."]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not stage", &out.reason()));
    }
    Ok(ctx.done(Some(&repo), &lines::staged_all(count), count))
}

/// A path as a `.gitignore` pattern anchored at the top: `/build.log`,
/// `/target/` for a folder, with git's special characters escaped.
pub fn pattern(path: &str, folder: bool) -> String {
    let mut out = String::from("/");
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\' | '!' | '#') {
            out.push('\\');
        }
        out.push(c);
    }
    // A trailing space is kept only when escaped.
    if out.ends_with(' ') {
        let kept = out.trim_end_matches(' ').len();
        let spaces = out.len() - kept;
        out.truncate(kept);
        for _ in 0..spaces {
            out.push_str("\\ ");
        }
    }
    if folder && !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// Git: add to .gitignore: the marked paths as patterns at the end of the
/// top's `.gitignore`, written through the host (which asks before it
/// replaces the file).
pub fn ignore(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, paths, targets) = ctx.target_paths()?;
    let paths = paths_or_nothing(paths)?;
    let folders: Vec<bool> = targets
        .iter()
        .map(|t| match t {
            Target::File { folder, .. } => *folder,
            Target::Row(row) => row.path.as_deref().is_some_and(|p| p.ends_with('/')),
        })
        .collect();
    let file = repo.root.join(".gitignore");
    let existing = std::fs::read(&file).unwrap_or_default();
    let text = String::from_utf8_lossy(&existing).into_owned();
    let present: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut added = Vec::new();
    for (n, path) in paths.iter().enumerate() {
        let folder = folders.get(n).copied().unwrap_or(false);
        let line = pattern(path.trim_end_matches('/'), folder);
        if !present.contains(&line.as_str()) && !added.contains(&line) {
            added.push(line);
        }
    }
    if added.is_empty() {
        return Err(GitError::refused(lines::ALREADY_IGNORED, None));
    }
    let mut new = text.clone();
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    for line in &added {
        new.push_str(line);
        new.push('\n');
    }
    if !ctx.host.write_file(&place_of(&file), new.as_bytes())? {
        return Err(GitError::refused(KEPT_GITIGNORE, None));
    }
    let done = ctx.done(Some(&repo), &lines::ignored(added.len()), added.len());
    refresh(ctx, &repo)?;
    Ok(done)
}

/// What Add to .gitignore says when the owner kept the file.
pub const KEPT_GITIGNORE: &str = "Kept .gitignore as it was";

/// The time a discard's stash is named with, from the clock, in UTC:
/// "2026-11-06 14:30 UTC".
pub fn stash_time(now: SystemTime) -> String {
    let c = Timestamp::from_system_time(now).to_civil(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        c.year, c.month, c.day, c.hour, c.minute
    )
}

/// Git: discard changes: the marked tracked files put back as staged,
/// after a question that lists them and starts on No; what they held goes
/// to a stash.
pub fn discard(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, paths, _) = ctx.target_paths()?;
    let paths = paths_or_nothing(paths)?;
    let status = read_status(ctx.host, &repo, &paths)?;
    let untracked = status
        .changes
        .iter()
        .filter(|c| matches!(c.kind, status::Kind::Untracked))
        .count();
    let changed: Vec<&Change> = status
        .changes
        .iter()
        .filter(|c| c.in_tree() && !c.not_unicode)
        .collect();
    if changed.is_empty() {
        return Err(GitError::refused(lines::NOTHING_CHANGED, None));
    }
    let rows: Vec<String> = changed
        .iter()
        .map(|c| lines::change_row(&c.word(), &c.path, c.from.as_deref()))
        .collect();
    let title = lines::discard_title(changed.len());
    let question = lines::discard_lines(&rows, untracked);
    // A no, like Escape, leaves everything as it was: the host says
    // "cancelled".
    if ctx.host.confirm(&title, &question, true)? != Some(true) {
        return Err(GitError::Cancelled);
    }
    let message = lines::discard_message(&stash_time(SystemTime::now()));
    let targets: Vec<String> = changed.iter().map(|c| c.path.clone()).collect();
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(
            &repo.root,
            &["stash", "push", "--keep-index", "-m", &message],
        )
        .with_paths(&targets),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not discard", &out.reason()));
    }
    let done = ctx.done(
        Some(&repo),
        &lines::discarded(changed.len(), 0),
        changed.len(),
    );
    refresh(ctx, &repo)?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn a_pattern_is_anchored_and_escaped() {
        assert_eq!(pattern("build.log", false), "/build.log");
        assert_eq!(pattern("target", true), "/target/");
        assert_eq!(pattern("src/gen", true), "/src/gen/");
        assert_eq!(pattern("a[1]*.txt", false), "/a\\[1\\]\\*.txt");
        assert_eq!(pattern("#notes", false), "/\\#notes");
        assert_eq!(pattern("trail ", false), "/trail\\ ");
    }

    #[test]
    fn the_stash_time_comes_from_the_clock() {
        let time = UNIX_EPOCH + Duration::from_secs(1_793_974_970);
        assert_eq!(stash_time(time), "2026-11-06 14:22 UTC");
    }
}
