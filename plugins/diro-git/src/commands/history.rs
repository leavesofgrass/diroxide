//! History in words (plan section 5.16; R29 2.4): the log of a folder or a
//! file as a list whose rows say the date and subject first and the hash
//! last; Enter on a commit lists the files it changed, and Enter on one of
//! those compares it with the commit before; and who last changed a line.
//!
//! The viewer gives a plugin's command no cursor line yet, so "who changed
//! a line" asks for the line number (a contract request).

use serde_json::Value;

use super::{Ctx, Row, RowData, Target, View, git_refused, short_name};
use crate::error::GitError;
use crate::git::blame;
use crate::git::log::{self, LOG_FORMAT, MAX_COMMITS};
use crate::git::{self, Run};
use crate::host::Host;
use crate::lines;
use crate::repo::Repo;

/// The commits that changed a path from the top ("" for all), as rows.
pub fn log_rows(host: Host<'_>, repo: &Repo, path: &str) -> Result<Vec<Row>, GitError> {
    let max = format!("--max-count={MAX_COMMITS}");
    let spec = if path.is_empty() { "." } else { path };
    let out = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &["log", "-z", "--date=short", LOG_FORMAT, &max])
            .automatic()
            .with_paths(&[spec]),
    )?;
    if !out.ok() {
        // A branch with no commit yet has no log.
        if out.stderr.contains("does not have any commits yet") {
            return Ok(Vec::new());
        }
        return Err(git_refused("Could not read the log", &out.reason()));
    }
    Ok(log::parse_log(&out.stdout)
        .into_iter()
        .map(|c| Row {
            name: c.row(),
            columns: vec![
                ("date", c.date.clone()),
                ("subject", c.subject.clone()),
                ("author", c.author.clone()),
                ("hash", c.short.clone()),
            ],
            data: RowData {
                root: repo.root.clone(),
                hash: Some(c.hash.clone()),
                word: c.subject.clone(),
                ..RowData::default()
            },
        })
        .collect())
}

/// Git: log of this folder, and Git: log of this file (`file`).
pub fn log(ctx: &Ctx<'_>, file: bool) -> Result<Value, GitError> {
    let (repo, path) = if file {
        // A folder on the cursor logs that folder; that is fine too.
        let (repo, paths, _) = ctx.target_paths()?;
        let path = paths
            .first()
            .cloned()
            .ok_or_else(|| GitError::refused(super::diff::NOT_A_FILE, None))?;
        (repo, path.trim_end_matches('/').to_owned())
    } else {
        let repo = ctx.repo()?;
        let folder = ctx.folder()?;
        let path = repo.relative(&folder).unwrap_or_default();
        (repo, path)
    };
    let what = if path.is_empty() {
        repo.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the repository".to_owned())
    } else {
        short_name(&path).to_owned()
    };
    let rows = log_rows(ctx.host, &repo, &path)?;
    if rows.is_empty() {
        return Err(GitError::refused(lines::no_commits(&what), None));
    }
    let title = lines::log_title(&what, rows.len());
    ctx.open(
        "log",
        &title,
        View::Log {
            root: repo.root.clone(),
            path,
            what,
        },
        rows,
    )
}

/// What one commit changed, as rows; against its first parent, or
/// everything for the first commit.
pub fn commit_rows(host: Host<'_>, repo: &Repo, hash: &str) -> Result<Vec<Row>, GitError> {
    let parents = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &["rev-list", "--parents", "-n", "1", hash]).automatic(),
    )?;
    if !parents.ok() {
        return Err(git_refused("Could not read the commit", &parents.reason()));
    }
    let text = parents.text();
    let parent = text.split_whitespace().nth(1).map(str::to_owned);
    let args: Vec<String> = match &parent {
        Some(parent) => ["diff", "--name-status", "-z", "-M", parent, hash]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        None => [
            "diff-tree",
            "-r",
            "-z",
            "--name-status",
            "--no-commit-id",
            "--root",
            hash,
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect(),
    };
    let out = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &args).automatic(),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not read the commit", &out.reason()));
    }
    Ok(log::parse_name_status(&out.stdout)
        .into_iter()
        .map(|f| Row {
            name: f.row(),
            columns: vec![("state", f.word().to_owned()), ("path", f.path.clone())],
            data: RowData {
                root: repo.root.clone(),
                path: Some(f.path.clone()),
                from: f.from.clone(),
                hash: Some(hash.to_owned()),
                word: f.word().to_owned(),
                ..RowData::default()
            },
        })
        .collect())
}

/// Git: files of this commit, from a row of the log.
pub fn commit_files(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, _, targets) = ctx.target_paths()?;
    let Some(Target::Row(RowData {
        hash: Some(hash), ..
    })) = targets.first()
    else {
        return Err(GitError::refused(PICK_A_COMMIT, None));
    };
    let rows = commit_rows(ctx.host, &repo, hash)?;
    let short: String = hash.chars().take(7).collect();
    let title = lines::commit_title(&short, rows.len());
    ctx.open(
        "commit",
        &title,
        View::Commit {
            root: repo.root.clone(),
            hash: hash.clone(),
        },
        rows,
    )
}

/// What the commit-files command says off a log's row.
pub const PICK_A_COMMIT: &str = "Pick a commit in a log list";

/// Git: who changed a line: asks the line, then says its author, date and
/// commit.
pub fn blame(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, paths, targets) = ctx.target_paths()?;
    if matches!(targets.first(), Some(Target::File { folder: true, .. })) {
        return Err(GitError::refused(super::diff::NOT_A_FILE, None));
    }
    let Some(path) = paths.first().cloned() else {
        return Err(GitError::refused(super::diff::NOT_A_FILE, None));
    };
    let Some(answer) = ctx.host.ask_text(
        lines::BLAME_TITLE,
        &[format!("{} {}", lines::BLAME_LINE, short_name(&path))],
        "1",
        Some(("[1-9][0-9]{0,8}", LINE_NUMBER_WORDS)),
    )?
    else {
        return Err(GitError::Cancelled);
    };
    let line: u32 = answer
        .trim()
        .parse()
        .map_err(|_| GitError::refused(LINE_NUMBER_WORDS, None))?;
    let range = format!("{line},{line}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["blame", "--porcelain", "-L", &range])
            .automatic()
            .with_paths(&[&path]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not read who changed it", &out.reason()));
    }
    let blame = blame::parse(&out.text())
        .ok_or_else(|| GitError::Bad("git's blame could not be read".to_owned()))?;
    Ok(ctx.done(None, &blame.words(line), 1))
}

/// What the line question says when the answer is not a line number.
pub const LINE_NUMBER_WORDS: &str = "Type a line number, 1 or more";
