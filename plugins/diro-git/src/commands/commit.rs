//! The commit form (plan section 5.16; R29 2.4; the host-pieces lane's
//! `ui/form`): the message first, the staged files in words above it,
//! Amend off, Sign-off a checkbox. The message goes to git on standard
//! input (`git commit -F -`), never on a command line. A commit runs the
//! repository's hooks, as it does in a terminal, and the job says so.
//!
//! The form has no "required" mark (HP-2), so an empty message is checked
//! here: "Nothing to commit: the message is empty".

use serde_json::{Value, json};

use super::Ctx;
use super::tree::{has_head, read_status};
use crate::error::GitError;
use crate::git::status::{Change, Kind};
use crate::git::{self, LONG_TIMEOUT, Run};
use crate::lines;

/// What a staged change does, in words: "Modified", "Added".
pub fn staged_word(change: &Change) -> &'static str {
    match change.kind {
        Kind::Changed { x, .. } | Kind::Renamed { x, .. } => match x {
            'A' => "Added",
            'D' => "Deleted",
            'R' => "Renamed",
            'C' => "Copied",
            'T' => "Type changed",
            _ => "Modified",
        },
        _ => "Modified",
    }
}

/// The form the commit asks: its summary, the message first, Amend when
/// there is a commit to amend, Sign-off.
pub fn commit_form(rows: &[String], branch: &str, can_amend: bool) -> Value {
    let mut fields = vec![json!({"id": "message", "label": "Message", "kind": "multiline"})];
    if can_amend {
        fields.push(json!({"id": "amend", "label": "Amend the last commit", "kind": "checkbox", "default": false}));
    }
    fields
        .push(json!({"id": "signoff", "label": "Sign-off", "kind": "checkbox", "default": false}));
    json!({
        "title": lines::COMMIT_TITLE,
        "summary": lines::commit_summary(rows, branch),
        "fields": fields,
        "focus": "message",
        "default_button": "Commit",
    })
}

/// Whether git's words say it knows no author.
pub fn no_identity(stderr: &str) -> bool {
    stderr.contains("Author identity unknown")
        || stderr.contains("Please tell me who you are")
        || stderr.contains("unable to auto-detect email address")
        || stderr.contains("empty ident name")
        || stderr.contains("auto-detection is disabled")
}

/// Git: commit.
pub fn commit(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx.repo()?;
    let status = read_status(ctx.host, &repo, &[])?;
    let conflicts = status.conflicts();
    if conflicts > 0 {
        return Err(GitError::refused(lines::conflicts_first(conflicts), None));
    }
    let staged: Vec<&Change> = status.staged().collect();
    if staged.is_empty() {
        return Err(GitError::refused(lines::NOTHING_STAGED, None));
    }
    let rows: Vec<String> = staged
        .iter()
        .map(|c| lines::change_row(staged_word(c), &c.path, c.from.as_deref()))
        .collect();
    let branch = status
        .branch
        .head
        .clone()
        .unwrap_or_else(|| "a detached head".to_owned());
    let can_amend = has_head(ctx.host, &repo)?;
    let Some(values) = ctx.host.form(commit_form(&rows, &branch, can_amend))? else {
        return Err(GitError::Cancelled);
    };
    let message = values
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if message.trim().is_empty() {
        return Err(GitError::refused(lines::EMPTY_MESSAGE, None));
    }
    let flag = |id: &str| values.get(id).and_then(Value::as_bool).unwrap_or(false);
    let amend = can_amend && flag("amend");
    let mut args = vec!["commit", "-F", "-"];
    if amend {
        args.push("--amend");
    }
    if flag("signoff") {
        args.push("--signoff");
    }
    ctx.progress.begin("Committing");
    let hooks = repo.has_commit_hooks();
    if hooks {
        ctx.progress.report(lines::RUNNING_HOOKS);
        ctx.host.announce(lines::RUNNING_HOOKS, "status");
    }
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &args)
            .with_stdin(message)
            .with_timeout(LONG_TIMEOUT),
    )?;
    ctx.plugin.statuses.drop_repository(&repo.root);
    if !out.ok() {
        if no_identity(&out.stderr) {
            return Err(GitError::refused(
                lines::NO_IDENTITY,
                Some(lines::SET_IDENTITY),
            ));
        }
        if out.stderr.contains("nothing to commit") || out.text().contains("nothing to commit") {
            return Err(GitError::refused(lines::NOTHING_STAGED, None));
        }
        if hooks && !out.stderr.trim().is_empty() {
            // What the hook said, whole, for reading at leisure.
            ctx.host
                .show_text("What the commit hooks said", &out.stderr, "plain")?;
        }
        return Err(GitError::refused(
            lines::commit_stopped(&out.reason()),
            None,
        ));
    }
    let short = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&repo.root, &["rev-parse", "--short", "HEAD"]),
    )?
    .text()
    .trim()
    .to_owned();
    let sentence = if amend {
        lines::amended(&branch, &short)
    } else {
        lines::committed(staged.len(), &branch, &short)
    };
    Ok(ctx.done(Some(&repo), &sentence, staged.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_puts_the_message_first_and_amend_only_with_a_commit() {
        let form = commit_form(&["Modified: hello.txt".into()], "main", true);
        assert_eq!(form["fields"][0]["id"], "message");
        assert_eq!(form["fields"][0]["kind"], "multiline");
        assert_eq!(form["fields"][1]["id"], "amend");
        assert_eq!(form["fields"][1]["default"], false);
        assert_eq!(form["fields"][2]["label"], "Sign-off");
        assert_eq!(form["focus"], "message");
        assert_eq!(form["default_button"], "Commit");
        assert_eq!(form["title"], "Commit");
        assert_eq!(
            form["summary"],
            "1 file staged on main. Modified: hello.txt."
        );
        let first = commit_form(&["Added: a.txt".into()], "main", false);
        assert_eq!(first["fields"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn git_s_missing_author_is_told_apart() {
        assert!(no_identity(
            "Author identity unknown\n\n*** Please tell me who you are."
        ));
        assert!(no_identity(
            "fatal: unable to auto-detect email address (got 'x@y.(none)')"
        ));
        assert!(!no_identity("error: pathspec did not match"));
    }
}
