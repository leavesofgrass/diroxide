//! Branches and stashes (plan section 5.16; R29 2.4): list, switch,
//! create and rename branches; list, push, pop, drop and show stashes.
//!
//! Switching never forces: when git refuses because local changes would
//! be lost, the plugin says which files and that they need a commit or a
//! stash first, and offers no `--force`. A drop lists the stash and starts
//! on No; a pop or a drop checks first that the stash on the row is still
//! the one at that number, since numbers move when stashes come and go.

use serde_json::Value;

use super::{Ctx, Row, RowData, Target, View, git_refused};
use crate::error::GitError;
use crate::git::refs::{self, BRANCH_FORMAT, STASH_FORMAT};
use crate::git::{self, Run};
use crate::host::Host;
use crate::lines;
use crate::repo::Repo;

/// The local branches as rows.
pub fn branch_rows(host: Host<'_>, repo: &Repo) -> Result<Vec<Row>, GitError> {
    let out = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &["for-each-ref", BRANCH_FORMAT, "refs/heads"]).automatic(),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not list the branches", &out.reason()));
    }
    Ok(refs::parse_branches(&out.text())
        .into_iter()
        .map(|b| Row {
            name: b.row(),
            columns: vec![
                ("standing", b.standing()),
                ("date", b.date.clone()),
                ("subject", b.subject.clone()),
            ],
            data: RowData {
                root: repo.root.clone(),
                branch: Some(b.name.clone()),
                word: if b.current {
                    CURRENT.to_owned()
                } else {
                    b.standing()
                },
                ..RowData::default()
            },
        })
        .collect())
}

/// The word a current branch's row data carries.
const CURRENT: &str = "current";

/// What the branches list says with no branch yet.
pub const NO_BRANCHES: &str = "No branches yet: make a first commit";

/// What a branch command off a branch row says.
pub const PICK_A_BRANCH: &str = "Pick a branch in the branches list";

/// Git: branches.
pub fn branches(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let rows = branch_rows(ctx.host, &repo)?;
    if rows.is_empty() {
        return Err(GitError::refused(NO_BRANCHES, None));
    }
    let current = rows
        .iter()
        .find(|r| r.data.word == CURRENT)
        .and_then(|r| r.data.branch.clone());
    let title = lines::branches_title(rows.len(), current.as_deref());
    ctx.open(
        "branches",
        &title,
        View::Branches {
            root: repo.root.clone(),
        },
        rows,
    )
}

fn refresh_branches(ctx: &Ctx<'_>, repo: &Repo) -> Result<(), GitError> {
    if ctx.lister() == Some("branches") {
        branches_again(ctx, repo)?;
    }
    Ok(())
}

fn branches_again(ctx: &Ctx<'_>, repo: &Repo) -> Result<Value, GitError> {
    let rows = branch_rows(ctx.host, repo)?;
    let current = rows
        .iter()
        .find(|r| r.data.word == CURRENT)
        .and_then(|r| r.data.branch.clone());
    let title = lines::branches_title(rows.len(), current.as_deref());
    ctx.open(
        "branches",
        &title,
        View::Branches {
            root: repo.root.clone(),
        },
        rows,
    )
}

/// The files git names when it refuses to switch because local changes
/// would be overwritten: the indented lines after its first line.
pub fn overwritten(stderr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in stderr.lines() {
        if line.contains("would be overwritten") {
            inside = true;
            continue;
        }
        if inside {
            if let Some(path) = line.strip_prefix('\t') {
                out.push(path.trim().to_owned());
            } else {
                inside = false;
            }
        }
    }
    out
}

/// Git: switch to this branch, from a row of the branches list.
pub fn switch(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, _, targets) = ctx.target_paths()?;
    let Some(Target::Row(RowData {
        branch: Some(name),
        word,
        ..
    })) = targets.first()
    else {
        return Err(GitError::refused(PICK_A_BRANCH, None));
    };
    if word == CURRENT {
        return Ok(ctx.done(None, &lines::switched(name), 0));
    }
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["switch", name.as_str()]),
    )?;
    if !out.ok() {
        let files = overwritten(&out.stderr);
        if !files.is_empty() {
            return Err(GitError::refused(
                lines::cannot_switch_changes(&files),
                Some(lines::COMMIT_OR_STASH),
            ));
        }
        return Err(git_refused("Could not switch", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::switched(name), 1);
    refresh_branches(ctx, &repo)?;
    Ok(done)
}

/// Whether a name may be a branch: checked here for the plain cases, then
/// by `git check-ref-format --branch`.
fn plain_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn checked_branch_name(ctx: &Ctx<'_>, repo: &Repo, name: &str) -> Result<(), GitError> {
    if !plain_branch_name(name) {
        return Err(GitError::refused(lines::BAD_BRANCH_NAME, None));
    }
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["check-ref-format", "--branch", name]),
    )?;
    if out.ok() {
        Ok(())
    } else {
        Err(GitError::refused(lines::BAD_BRANCH_NAME, None))
    }
}

/// Git: new branch: asks its name, then `git switch -c NAME`.
pub fn new_branch(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let Some(name) = ctx.host.ask_text(
        lines::NEW_BRANCH_TITLE,
        &[lines::BRANCH_NAME.to_owned()],
        "",
        None,
    )?
    else {
        return Err(GitError::Cancelled);
    };
    let name = name.trim().to_owned();
    checked_branch_name(ctx, &repo, &name)?;
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["switch", "-c", name.as_str()]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not make the branch", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::created_branch(&name), 1);
    refresh_branches(ctx, &repo)?;
    Ok(done)
}

/// The checked-out branch's name.
fn current_branch(ctx: &Ctx<'_>, repo: &Repo) -> Result<String, GitError> {
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["symbolic-ref", "--short", "-q", "HEAD"]),
    )?;
    let name = out.text().trim().to_owned();
    if out.ok() && !name.is_empty() {
        Ok(name)
    } else {
        Err(GitError::refused(DETACHED, None))
    }
}

/// What a branch command says on a detached head.
pub const DETACHED: &str = "No branch here: the head is detached";

/// Git: rename branch: the row's branch, or the current one.
pub fn rename(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, _, targets) = ctx.target_paths()?;
    let old = match targets.first() {
        Some(Target::Row(RowData {
            branch: Some(name), ..
        })) => name.clone(),
        _ => current_branch(ctx, &repo)?,
    };
    let Some(new) = ctx.host.ask_text(
        lines::RENAME_BRANCH_TITLE,
        &[format!("New name for {old}:")],
        &old,
        None,
    )?
    else {
        return Err(GitError::Cancelled);
    };
    let new = new.trim().to_owned();
    if new == old {
        return Err(GitError::Cancelled);
    }
    checked_branch_name(ctx, &repo, &new)?;
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["branch", "-m", old.as_str(), new.as_str()]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not rename the branch", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::renamed_branch(&old, &new), 1);
    refresh_branches(ctx, &repo)?;
    Ok(done)
}

/// The stashes as rows, the newest first.
pub fn stash_rows(host: Host<'_>, repo: &Repo) -> Result<Vec<Row>, GitError> {
    let out = git::run(
        host.peer,
        Some(host.cancel),
        &Run::new(&repo.root, &["stash", "list", "-z", STASH_FORMAT]).automatic(),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not list the stashes", &out.reason()));
    }
    Ok(refs::parse_stashes(&out.stdout)
        .into_iter()
        .map(|s| Row {
            name: s.row(),
            columns: vec![
                ("branch", s.branch.clone().unwrap_or_default()),
                ("date", s.date.clone()),
            ],
            data: RowData {
                root: repo.root.clone(),
                stash: Some((s.number, s.hash.clone())),
                word: s.message.clone(),
                ..RowData::default()
            },
        })
        .collect())
}

/// Git: stashes.
pub fn stashes(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let rows = stash_rows(ctx.host, &repo)?;
    if rows.is_empty() {
        return Err(GitError::refused(lines::NO_STASHES, None));
    }
    let title = lines::stashes_title(rows.len());
    ctx.open(
        "stashes",
        &title,
        View::Stashes {
            root: repo.root.clone(),
        },
        rows,
    )
}

fn refresh_stashes(ctx: &Ctx<'_>, repo: &Repo) -> Result<(), GitError> {
    if ctx.lister() != Some("stashes") {
        return Ok(());
    }
    let rows = stash_rows(ctx.host, repo)?;
    let title = lines::stashes_title(rows.len());
    ctx.open(
        "stashes",
        &title,
        View::Stashes {
            root: repo.root.clone(),
        },
        rows,
    )
    .map(drop)
}

/// Git: stash changes: asks a message, then `git stash push`.
pub fn stash_push(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let Some(message) = ctx.host.ask_text(
        lines::STASH_TITLE,
        &[lines::STASH_MESSAGE.to_owned()],
        "",
        None,
    )?
    else {
        return Err(GitError::Cancelled);
    };
    let mut args = vec!["stash".to_owned(), "push".to_owned()];
    let message = message.trim().to_owned();
    if !message.is_empty() {
        args.push("-m".to_owned());
        args.push(message);
    }
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &args),
    )?;
    if out.text().contains("No local changes to save") {
        return Err(GitError::refused(lines::NOTHING_TO_STASH, None));
    }
    if !out.ok() {
        return Err(git_refused("Could not stash", &out.reason()));
    }
    Ok(ctx.done(Some(&repo), lines::STASHED, 1))
}

/// The stash a command acts on: the row's, or the newest; checked to be
/// still the one at its number. Its repository, number and message.
fn chosen_stash(ctx: &Ctx<'_>, newest: bool) -> Result<(Repo, u32, String), GitError> {
    let (repo, _, targets) = ctx.target_paths()?;
    let (number, hash, message) = match targets.first() {
        Some(Target::Row(RowData {
            stash: Some((number, hash)),
            word,
            ..
        })) => (*number, hash.clone(), word.clone()),
        _ if newest => {
            let rows = stash_rows(ctx.host, &repo)?;
            match rows.first() {
                Some(Row {
                    data:
                        RowData {
                            stash: Some((number, hash)),
                            word,
                            ..
                        },
                    ..
                }) => (*number, hash.clone(), word.clone()),
                _ => return Err(GitError::refused(lines::NO_STASHES, None)),
            }
        }
        _ => return Err(GitError::refused(PICK_A_STASH, None)),
    };
    let selector = format!("stash@{{{number}}}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(
            &repo.root,
            &["rev-parse", "-q", "--verify", selector.as_str()],
        ),
    )?;
    if !out.ok() || out.text().trim() != hash {
        return Err(GitError::refused(lines::STASH_MOVED, None));
    }
    Ok((repo, number, message))
}

/// What a stash command off a stash row says.
pub const PICK_A_STASH: &str = "Pick a stash in the stashes list";

/// Brings back what Discard put in a stash: each file's text as it was,
/// from the stash, into the working tree alone, so what was staged stays
/// staged; then the stash goes. A plain `git stash pop` would meet the
/// staged text and stop at a conflict. A file changed since the discard
/// is never overwritten.
fn undo_discard(ctx: &Ctx<'_>, repo: &Repo, number: u32) -> Result<Value, GitError> {
    let selector = format!("stash@{{{number}}}");
    let base = format!("{selector}^1");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(
            &repo.root,
            &[
                "diff",
                "--name-only",
                "-z",
                base.as_str(),
                selector.as_str(),
            ],
        )
        .automatic(),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not read the stash", &out.reason()));
    }
    let paths: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    if paths.is_empty() {
        return Err(GitError::refused(lines::NOTHING_CHANGED, None));
    }
    let now = super::tree::read_status(ctx.host, repo, &paths)?;
    let changed: Vec<String> = now
        .changes
        .iter()
        .filter(|c| c.in_tree())
        .map(|c| c.path.clone())
        .collect();
    if !changed.is_empty() {
        return Err(GitError::refused(
            lines::changed_since_discard(&changed),
            None,
        ));
    }
    let source = format!("--source={selector}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["restore", source.as_str(), "--worktree"]).with_paths(&paths),
    )?;
    if !out.ok() {
        return Err(git_refused(
            "Could not bring the changes back",
            &out.reason(),
        ));
    }
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["stash", "drop", "-q", selector.as_str()]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not drop the stash", &out.reason()));
    }
    let done = ctx.done(
        Some(repo),
        &lines::brought_back(paths.len(), number),
        paths.len(),
    );
    refresh_stashes(ctx, repo)?;
    Ok(done)
}

/// Git: pop stash; a stash Discard made comes back file by file.
pub fn stash_pop(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, number, message) = chosen_stash(ctx, true)?;
    if message.starts_with(&lines::discard_message("")) {
        return undo_discard(ctx, &repo, number);
    }
    let selector = format!("stash@{{{number}}}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["stash", "pop", selector.as_str()]),
    )?;
    ctx.plugin.statuses.drop_repository(&repo.root);
    if !out.ok() {
        return Err(git_refused("Could not pop the stash", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::popped(number), 1);
    refresh_stashes(ctx, &repo)?;
    Ok(done)
}

/// Git: drop stash, after a question that starts on No.
pub fn stash_drop(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, number, _) = chosen_stash(ctx, false)?;
    let words = match ctx.targets()?.first() {
        Some(Target::Row(row)) => format!("Stash {number}: {}", row.word),
        _ => format!("Stash {number}"),
    };
    if ctx
        .host
        .confirm(&lines::drop_title(number), &lines::drop_lines(&words), true)?
        != Some(true)
    {
        return Err(GitError::Cancelled);
    }
    let selector = format!("stash@{{{number}}}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["stash", "drop", selector.as_str()]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not drop the stash", &out.reason()));
    }
    let done = ctx.done(Some(&repo), &lines::dropped(number), 1);
    refresh_stashes(ctx, &repo)?;
    Ok(done)
}

/// Git: show a stash: its changes as a document.
pub fn stash_show(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let (repo, number, _) = chosen_stash(ctx, false)?;
    let selector = format!("stash@{{{number}}}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(
            &repo.root,
            &["stash", "show", "-p", "--no-color", selector.as_str()],
        )
        .automatic(),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not show the stash", &out.reason()));
    }
    ctx.host
        .show_text(&format!("Stash {number}"), &out.text(), "diff")?;
    Ok(serde_json::json!({"done": 1, "skipped": 0, "failed": []}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_git_will_not_overwrite_are_read() {
        let stderr = "error: Your local changes to the following files would be overwritten by checkout:\n\thello.txt\n\tsub/c.txt\nPlease commit your changes or stash them before you switch branches.\nAborting\n";
        assert_eq!(overwritten(stderr), ["hello.txt", "sub/c.txt"]);
        assert!(overwritten("fatal: invalid reference: x").is_empty());
    }

    #[test]
    fn plain_branch_names_pass_the_first_check() {
        assert!(plain_branch_name("topic/one"));
        assert!(!plain_branch_name("-x"));
        assert!(!plain_branch_name("two words"));
        assert!(!plain_branch_name(""));
        for line in [NO_BRANCHES, PICK_A_BRANCH, DETACHED, PICK_A_STASH] {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }
}
