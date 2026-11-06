//! New repositories and the trust answer (plan section 5.16; R29 2.4 and
//! 2.5): clone into the active panel's folder as a job with its progress
//! in words, make the folder a repository with a `.gitignore` from a
//! template and a first commit, and change the trust answer.
//!
//! A clone that is cancelled may leave its half-made folder; the plugin
//! hands it to diroxide's delete, which lists it and asks before it goes
//! to the trash. git never asks a question here: an address that needs a
//! password says "Sign-in needed: git cannot ask here".

use serde_json::{Value, json};

use super::{Ctx, git_refused, needs_sign_in, place_of};
use crate::error::GitError;
use crate::folder::{self, Trust};
use crate::git::trust::Hit;
use crate::git::{self, LONG_TIMEOUT, Run};
use crate::lines;

/// The `.gitignore` templates the plugin ships, by their choice's words.
pub const TEMPLATES: &[(&str, &str)] = &[
    ("None", ""),
    ("Rust", "/target\n**/*.rs.bk\n"),
    (
        "Python",
        "__pycache__/\n*.py[cod]\n.venv/\nvenv/\n.mypy_cache/\n.pytest_cache/\ndist/\nbuild/\n*.egg-info/\n",
    ),
    ("Node", "node_modules/\nnpm-debug.log*\ndist/\n.env\n"),
    (
        "System files",
        ".DS_Store\nThumbs.db\ndesktop.ini\n*.swp\n*~\n",
    ),
];

/// Whether a folder name keeps the plain-name rule: letters, digits, `-`,
/// `_` and `.`, not starting with a dot, not a device name.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !matches!(
            name.to_ascii_uppercase().split('.').next().unwrap_or(""),
            "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "LPT1" | "LPT2"
        )
}

/// The folder a clone makes when none is named: the address's last name
/// without `.git`.
pub fn name_from_address(address: &str) -> String {
    let trimmed = address.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or(trimmed);
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

/// The clone form.
pub fn clone_form() -> Value {
    json!({
        "title": lines::CLONE_TITLE,
        "summary": "The repository is copied into a new folder in this panel.",
        "fields": [
            {"id": "address", "label": "Address", "kind": "text"},
            {"id": "name", "label": "Folder name, or empty for the address's", "kind": "text", "default": ""},
        ],
        "focus": "address",
        "default_button": "Clone",
    })
}

/// Git: clone into this panel.
pub fn clone(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let folder = ctx.folder()?;
    folder::git_ready(ctx.plugin, ctx.host)?;
    let Some(values) = ctx.host.form(clone_form())? else {
        return Err(GitError::Cancelled);
    };
    let text = |id: &str| {
        values
            .get(id)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let address = text("address");
    if address.is_empty() {
        return Err(GitError::Cancelled);
    }
    let name = match text("name") {
        named if named.is_empty() => name_from_address(&address),
        named => named,
    };
    if !plain_name(&name) {
        return Err(GitError::refused(lines::PLAIN_NAME, None));
    }
    let target = folder.join(&name);
    if target.exists() {
        return Err(GitError::refused(lines::folder_exists(&name), None));
    }
    ctx.progress.begin("Cloning");
    let run = Run::new(&folder, &["clone", "--progress", "--", &address, &name])
        .with_timeout(LONG_TIMEOUT)
        .with_progress(super::remote::progress_sink(&ctx.progress, "Cloning"));
    let result = git::run(ctx.host.peer, Some(ctx.host.cancel), &run);
    let left_behind = || target.exists();
    match result {
        Err(GitError::Cancelled) => {
            if left_behind() {
                // The host lists it and asks first; to the trash.
                let _ = ctx.host.trash(&[place_of(&target)]);
            }
            Err(GitError::Cancelled)
        }
        Err(other) => Err(other),
        Ok(out) if !out.ok() => {
            if left_behind() {
                let _ = ctx.host.trash(&[place_of(&target)]);
            }
            if needs_sign_in(&out.stderr) {
                return Err(GitError::refused(lines::SIGN_IN, Some(lines::SIGN_IN_HINT)));
            }
            Err(GitError::refused(lines::clone_stopped(&out.reason()), None))
        }
        Ok(_) => Ok(ctx.done(None, &lines::cloned(&name), 1)),
    }
}

/// The init form.
pub fn init_form() -> Value {
    let choices: Vec<&str> = TEMPLATES.iter().map(|(name, _)| *name).collect();
    json!({
        "title": lines::INIT_TITLE,
        "fields": [
            {"id": "branch", "label": "First branch", "kind": "text", "default": "main"},
            {"id": "template", "label": ".gitignore template", "kind": "choice", "choices": choices, "default": "None"},
            {"id": "commit", "label": "Commit the files here now", "kind": "checkbox", "default": true},
            {"id": "message", "label": "First commit's message", "kind": "text", "default": "First commit"},
        ],
        "focus": "branch",
        "default_button": "Make it",
    })
}

/// Git: make this folder a repository.
pub fn init(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let folder = ctx.folder()?;
    if folder.join(".git").exists() {
        return Err(GitError::refused(lines::ALREADY_A_REPOSITORY, None));
    }
    folder::git_ready(ctx.plugin, ctx.host)?;
    if let Some(outer) = ctx.plugin.find_repo(&folder) {
        let question = lines::inside_another(&outer.root.display().to_string());
        let lines = vec!["Make a separate repository here anyway?".to_owned()];
        if ctx.host.confirm(&question, &lines, true)? != Some(true) {
            return Err(GitError::Cancelled);
        }
    }
    let Some(values) = ctx.host.form(init_form())? else {
        return Err(GitError::Cancelled);
    };
    let text = |id: &str, default: &str| {
        values
            .get(id)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(default)
            .to_owned()
    };
    let branch = text("branch", "main");
    if branch.starts_with('-') || branch.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(GitError::refused(lines::BAD_BRANCH_NAME, None));
    }
    let initial = format!("--initial-branch={branch}");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&folder, &["init", "-q", initial.as_str()]),
    )?;
    if !out.ok() {
        return Err(git_refused("Could not make the repository", &out.reason()));
    }
    let template = text("template", "None");
    let ignore = folder.join(".gitignore");
    if let Some((_, body)) = TEMPLATES
        .iter()
        .find(|(name, body)| *name == template && !body.is_empty())
        && !ignore.exists()
    {
        ctx.host.write_file(&place_of(&ignore), body.as_bytes())?;
    }
    let commit = values
        .get("commit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !commit {
        return Ok(ctx.done(None, &lines::made_repository(&branch, None), 1));
    }
    let repo = ctx
        .plugin
        .find_repo(&folder)
        .ok_or(GitError::NotARepository)?;
    let add = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&folder, &["add", "--all"]).with_paths(&["."]),
    )?;
    if !add.ok() {
        return Err(git_refused("Could not stage the files", &add.reason()));
    }
    let staged = super::tree::read_status(ctx.host, &repo, &[])?
        .staged()
        .count();
    if staged == 0 {
        return Ok(ctx.done(None, &lines::made_repository(&branch, None), 1));
    }
    let message = text("message", "First commit");
    let out = git::run(
        ctx.host.peer,
        Some(ctx.host.cancel),
        &Run::new(&folder, &["commit", "-q", "-F", "-"])
            .with_stdin(message)
            .with_timeout(LONG_TIMEOUT),
    )?;
    if !out.ok() {
        ctx.host
            .announce(&lines::made_repository(&branch, None), "info");
        if super::commit::no_identity(&out.stderr) {
            return Err(GitError::refused(
                lines::NO_IDENTITY,
                Some(lines::SET_IDENTITY),
            ));
        }
        return Err(GitError::refused(
            lines::commit_stopped(&out.reason()),
            None,
        ));
    }
    Ok(ctx.done(
        Some(&repo),
        &lines::made_repository(&branch, Some(staged)),
        staged,
    ))
}

/// Git: trust this repository: asks again, whatever the answer was.
pub fn trust(ctx: &Ctx<'_>) -> Result<Value, GitError> {
    let repo = ctx
        .plugin
        .find_repo(&ctx.folder()?)
        .ok_or(GitError::NotARepository)?;
    folder::git_ready(ctx.plugin, ctx.host)?;
    let scan = folder::scan(ctx.plugin, ctx.host, &repo)?;
    if folder::trust_of(ctx.plugin, &repo, &scan) == Trust::Clean {
        return Ok(ctx.done(None, lines::NOTHING_TO_TRUST, 0));
    }
    let hits: Vec<String> = scan.hits.iter().map(Hit::words).collect();
    let question = lines::trust_lines(&repo.root.display().to_string(), &hits);
    let Some(yes) = ctx.host.confirm(lines::TRUST_TITLE, &question, true)? else {
        return Err(GitError::Cancelled);
    };
    ctx.plugin.first_ask(&repo.root);
    let _ = ctx.plugin.trust.record(&repo.root, &scan.digest, yes);
    let sentence = if yes {
        lines::TRUSTED
    } else {
        lines::NOT_TRUSTED_KEPT
    };
    Ok(ctx.done(Some(&repo), sentence, usize::from(yes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_s_folder_comes_from_its_address_and_keeps_plain_names() {
        assert_eq!(
            name_from_address("https://example.invalid/team/repo.git"),
            "repo"
        );
        assert_eq!(name_from_address("file:///D:/srv/origin.git/"), "origin");
        assert_eq!(
            name_from_address("git@example.invalid:team/tool.git"),
            "tool"
        );
        assert_eq!(name_from_address("D:\\srv\\plain"), "plain");
        assert!(plain_name("repo"));
        assert!(plain_name("my-repo_2.0"));
        assert!(!plain_name(".hidden"));
        assert!(!plain_name("two words"));
        assert!(!plain_name("caf\u{e9}"));
        assert!(!plain_name("con"));
        assert!(!plain_name("a:b"));
    }

    #[test]
    fn the_forms_put_the_first_answer_first() {
        let form = clone_form();
        assert_eq!(form["focus"], "address");
        assert_eq!(form["fields"][0]["label"], "Address");
        let form = init_form();
        assert_eq!(form["fields"][1]["choices"][0], "None");
        assert_eq!(form["fields"][1]["default"], "None");
        assert_eq!(form["fields"][2]["default"], true);
        for (name, _) in TEMPLATES {
            assert!(name.chars().count() <= 40);
        }
    }
}
