//! Fetch, pull, push and the lease push (plan section 5.16; R29 2.4): git
//! talks to its remotes under the owner's own keys and credential manager,
//! with `GIT_TERMINAL_PROMPT=0`, so a remote that needs a password git
//! cannot ask for says "Sign-in needed: git cannot ask here" and the next
//! step; this plugin never asks for, sees or keeps a password.
//!
//! Pull is fast-forward only. Push is never forced; the lease push
//! (`--force-with-lease --force-if-includes`) is its own command, behind
//! two questions that name the branch and start on No. Progress comes
//! from git's own progress lines, in words, at the host's interval.

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use super::tree::read_status;
use super::{Ctx, git_refused, needs_sign_in};
use crate::error::GitError;
use crate::git::progress::Said;
use crate::git::{self, LONG_TIMEOUT, Output, ProgressSink, Run};
use crate::host::Progress;
use crate::lines;
use crate::repo::Repo;

/// A sink that says git's progress lines as words under `verb`.
pub fn progress_sink(progress: &Progress, verb: &'static str) -> ProgressSink {
    let progress = progress.clone();
    let said = Arc::new(Mutex::new(Said::default()));
    Arc::new(move |line: &str| {
        let sentence = said
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next(verb, line);
        if let Some(sentence) = sentence {
            progress.report(&sentence);
        }
    })
}

fn talk(ctx: &Ctx<'_>, repo: &Repo, verb: &'static str, args: &[&str]) -> Result<Output, GitError> {
    ctx.progress.begin(verb);
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, args)
            .with_timeout(LONG_TIMEOUT)
            .with_progress(progress_sink(&ctx.progress, verb)),
    )?;
    ctx.plugin.statuses.drop_repository(&repo.root);
    if !out.ok() && needs_sign_in(&out.stderr) {
        return Err(GitError::refused(lines::SIGN_IN, Some(lines::SIGN_IN_HINT)));
    }
    Ok(out)
}

/// The repository's remotes, by name.
fn remotes(ctx: &Ctx<'_>, repo: &Repo) -> Result<Vec<String>, GitError> {
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["remote"]),
    )?;
    Ok(out
        .text()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Git: fetch.
pub fn fetch(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let names = remotes(ctx, &repo)?;
    let Some(first) = names.first().cloned() else {
        return Err(GitError::refused(lines::NO_REMOTE, None));
    };
    let out = talk(ctx, &repo, "Fetching", &["fetch", "--progress"])?;
    if !out.ok() {
        return Err(git_refused("Could not fetch", &out.reason()));
    }
    let status = read_status(ctx.host, &repo, &[])?;
    let branch = &status.branch;
    let sentence = match (&branch.head, &branch.upstream, branch.behind) {
        (Some(head), Some(upstream), Some(behind)) if behind > 0 => {
            lines::fetched_behind(head, behind, upstream)
        }
        (_, Some(upstream), _) => lines::fetched(upstream.split('/').next().unwrap_or(&first)),
        _ => lines::fetched(&first),
    };
    Ok(ctx.done(Some(&repo), &sentence, 1))
}

/// Whether git's words say a pull could not fast-forward.
pub fn not_fast_forward(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("not possible to fast-forward") || lower.contains("diverging branches")
}

/// How many commits are in `to` and not in `from`.
fn count_between(ctx: &Ctx<'_>, repo: &Repo, from: &str, to: &str) -> u32 {
    let range = format!("{from}..{to}");
    git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["rev-list", "--count", range.as_str()]),
    )
    .ok()
    .filter(Output::ok)
    .and_then(|o| o.text().trim().parse().ok())
    .unwrap_or(0)
}

/// Git: pull, fast-forward only.
pub fn pull(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let before = read_status(ctx.host, &repo, &[])?;
    let Some(head) = before.branch.head.clone() else {
        return Err(GitError::refused(super::branch::DETACHED, None));
    };
    if before.branch.upstream.is_none() {
        return Err(GitError::refused(lines::NO_UPSTREAM, None));
    }
    let old = before.branch.oid.clone();
    let out = talk(ctx, &repo, "Pulling", &["pull", "--ff-only", "--progress"])?;
    if !out.ok() {
        if not_fast_forward(&out.stderr) {
            return Err(GitError::refused(
                lines::NOT_FAST_FORWARD,
                Some(lines::MERGE_IN_TERMINAL),
            ));
        }
        return Err(git_refused("Could not pull", &out.reason()));
    }
    let pulled = match &old {
        Some(old) => count_between(ctx, &repo, old, "HEAD"),
        None => 0,
    };
    let sentence = if pulled == 0 {
        lines::UP_TO_DATE.to_owned()
    } else {
        lines::pulled(pulled, &head)
    };
    Ok(ctx.done(Some(&repo), &sentence, pulled as usize))
}

/// Whether git's words say the remote refused a push for new work there.
pub fn push_refused(stderr: &str) -> bool {
    stderr.contains("[rejected]")
        || stderr.contains("non-fast-forward")
        || stderr.contains("fetch first")
}

/// Git: push, never forced.
pub fn push(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let status = read_status(ctx.host, &repo, &[])?;
    let Some(head) = status.branch.head.clone() else {
        return Err(GitError::refused(super::branch::DETACHED, None));
    };
    if status.branch.oid.is_none() {
        return Err(GitError::refused(lines::NOTHING_TO_PUSH, None));
    }
    let (args, question, sentence, ahead): (Vec<String>, String, String, u32) =
        match &status.branch.upstream {
            Some(upstream) => {
                let ahead = status.branch.ahead.unwrap_or(0);
                if ahead == 0 {
                    return Err(GitError::refused(lines::NOTHING_TO_PUSH, None));
                }
                (
                    vec!["push".into(), "--progress".into()],
                    lines::push_question(ahead, upstream),
                    lines::pushed(ahead, upstream),
                    ahead,
                )
            }
            None => {
                let names = remotes(ctx, &repo)?;
                let remote = names
                    .iter()
                    .find(|n| n.as_str() == "origin")
                    .or_else(|| names.first())
                    .cloned()
                    .ok_or_else(|| GitError::refused(lines::NO_REMOTE, None))?;
                (
                    vec![
                        "push".into(),
                        "--progress".into(),
                        "--set-upstream".into(),
                        remote.clone(),
                        head.clone(),
                    ],
                    lines::push_new_question(&head, &remote),
                    lines::pushed_new(&head, &remote),
                    1,
                )
            }
        };
    if ctx.host.confirm(&question, &[], false)? != Some(true) {
        return Err(GitError::Cancelled);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = talk(ctx, &repo, "Pushing", &args)?;
    if !out.ok() {
        if push_refused(&out.stderr) {
            return Err(GitError::refused(
                lines::PUSH_REFUSED,
                Some(lines::PULL_FIRST),
            ));
        }
        return Err(git_refused("Could not push", &out.reason()));
    }
    Ok(ctx.done(Some(&repo), &sentence, ahead as usize))
}

/// Git: replace remote history: the lease push, behind two questions that
/// name the upstream and start on No.
pub fn push_lease(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let status = read_status(ctx.host, &repo, &[])?;
    let Some(head) = status.branch.head.clone() else {
        return Err(GitError::refused(super::branch::DETACHED, None));
    };
    let Some(upstream) = status.branch.upstream.clone() else {
        return Err(GitError::refused(lines::NO_UPSTREAM, None));
    };
    let first = ctx.host.confirm(
        &lines::replace_question(&upstream),
        &lines::replace_lines(&head, &upstream),
        true,
    )?;
    if first != Some(true) {
        return Err(GitError::Cancelled);
    }
    if ctx
        .host
        .confirm(&lines::really_replace(&upstream), &[], true)?
        != Some(true)
    {
        return Err(GitError::Cancelled);
    }
    let out = talk(
        ctx,
        &repo,
        "Pushing",
        &[
            "push",
            "--progress",
            "--force-with-lease",
            "--force-if-includes",
        ],
    )?;
    if !out.ok() {
        if out.stderr.contains("stale info") || push_refused(&out.stderr) {
            return Err(GitError::refused(
                lines::LEASE_REFUSED,
                Some(lines::FETCH_FIRST),
            ));
        }
        return Err(git_refused("Could not push", &out.reason()));
    }
    Ok(ctx.done(Some(&repo), &lines::replaced(&upstream), 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_s_refusals_are_told_apart() {
        assert!(not_fast_forward(
            "fatal: Not possible to fast-forward, aborting."
        ));
        assert!(not_fast_forward(
            "hint: Diverging branches can't be fast-forwarded"
        ));
        assert!(!not_fast_forward("Already up to date."));
        assert!(push_refused(
            " ! [rejected]        main -> main (fetch first)"
        ));
        assert!(!push_refused("To file:///x\n   a..b  main -> main"));
    }
}
