//! The commit form through the fake host and real git: the staged files in
//! words, the message on standard input, Amend and Sign-off, an empty
//! message, nothing staged, a missing author, and the repository's hooks.

use serde_json::json;

use super::support::{host, slashed};
use diro_git::lines;

#[test]
fn a_commit_asks_the_form_and_sends_the_message_on_standard_input() {
    let (git, mut host) = host();
    let repo = git.repo("commit");
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    std::fs::write(repo.join("notes.txt"), "n\n").unwrap();
    git.run(&repo, &["add", "hello.txt", "notes.txt"]);
    host.answers.push_back(json!({"values": {
        "message": "Fix the parser\n\nA body line; with $HOME and \"quotes\".",
        "amend": false, "signoff": true}}));
    host.command("commit", &repo, &[]).unwrap();
    let form = host.seen.forms.last().unwrap().clone();
    assert_eq!(form["title"], "Commit");
    assert_eq!(form["focus"], "message");
    assert_eq!(
        form["summary"],
        "2 files staged on main. Modified: hello.txt. Added: notes.txt."
    );
    assert_eq!(form["fields"][0]["kind"], "multiline");
    let said = host.seen.last_said().to_owned();
    assert!(said.starts_with("Committed 2 files on main: "), "{said}");
    assert!(said.chars().count() <= 40, "{said}");
    // The message went whole, never on a command line.
    let message = git.run(&repo, &["log", "-1", "--format=%B"]);
    assert!(
        message.starts_with("Fix the parser\n\nA body line; with $HOME and \"quotes\"."),
        "{message}"
    );
    assert!(
        message.contains("Signed-off-by: Ada Example <ada@example.invalid>"),
        "{message}"
    );
    let commit = host
        .seen
        .tools
        .iter()
        .find(|a| a.iter().any(|w| w == "commit"))
        .unwrap();
    assert!(commit.iter().all(|a| !a.contains("Fix the parser")));
    assert_eq!(host.column(&repo, &["hello.txt"]), ["Clean"]);

    // Amend keeps the files and replaces the message.
    std::fs::write(repo.join("hello.txt"), "again\n").unwrap();
    git.run(&repo, &["add", "hello.txt"]);
    host.answers.push_back(
        json!({"values": {"message": "Fix the parser, take two", "amend": true, "signoff": false}}),
    );
    host.command("commit", &repo, &[]).unwrap();
    assert!(
        host.seen
            .last_said()
            .starts_with("Amended the last commit on main: ")
    );
    assert_eq!(git.run(&repo, &["rev-list", "--count", "HEAD"]).trim(), "2");
}

#[test]
fn an_empty_message_nothing_staged_and_escape_commit_nothing() {
    let (git, mut host) = host();
    let repo = git.repo("empty");
    host.command("commit", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::NOTHING_STAGED);
    assert!(host.seen.forms.is_empty());
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    git.run(&repo, &["add", "hello.txt"]);
    host.answers
        .push_back(json!({"values": {"message": "  \n ", "amend": false, "signoff": false}}));
    host.command("commit", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::EMPTY_MESSAGE);
    // Escape answers cancelled.
    let cancelled = host.command("commit", &repo, &[]).unwrap_err();
    assert_eq!(cancelled["code"], -32800);
    assert_eq!(git.run(&repo, &["rev-list", "--count", "HEAD"]).trim(), "1");
}

#[test]
fn a_missing_author_is_said_with_the_next_step() {
    let (git, mut host) = host();
    let repo = git.repo("anonymous");
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    git.run(&repo, &["add", "hello.txt"]);
    // The repository asks git to use only configured names, and there is
    // none: git refuses rather than guessing.
    git.run(&repo, &["config", "user.useConfigOnly", "true"]);
    host.answers
        .push_back(json!({"values": {"message": "x", "amend": false, "signoff": false}}));
    // The fake host gives git the placeholder author by environment; this
    // test takes it away for the commit alone.
    host.fake = Some(Box::new({
        let git = std::sync::Arc::clone(&git);
        let repo = repo.clone();
        move |args: &[String]| {
            if !args.iter().any(|a| a == "commit") {
                return None;
            }
            let out = git
                .command(&repo)
                .args(args)
                .env_remove("GIT_AUTHOR_NAME")
                .env_remove("GIT_AUTHOR_EMAIL")
                .env_remove("GIT_COMMITTER_NAME")
                .env_remove("GIT_COMMITTER_EMAIL")
                .output()
                .unwrap();
            Some(json!({"exit_code": out.status.code(),
                "stdout": diroxide_plugins::base64::encode(&out.stdout),
                "stderr": diroxide_plugins::base64::encode(&out.stderr),
                "encoding": "base64", "truncated": false}))
        }
    }));
    host.command("commit", &repo, &[]).unwrap();
    let said = &host.seen.said;
    assert!(
        said.iter()
            .any(|(t, i)| t == lines::NO_IDENTITY && i == "warning"),
        "{}",
        host.seen.all_said()
    );
    assert_eq!(said.last().unwrap().0, lines::SET_IDENTITY);
}

#[test]
fn a_commit_runs_the_repository_s_hooks_and_says_so() {
    let (git, mut host) = host();
    let repo = git.repo("hooks");
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let ran = git.path().join("hook.ran");
    std::fs::write(
        hooks.join("pre-commit"),
        format!(
            "#!/bin/sh\necho ran > \"{}\"\necho the hook refuses >&2\nexit 1\n",
            slashed(&ran)
        ),
    )
    .unwrap();
    make_runnable(&hooks.join("pre-commit"));
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    git.run(&repo, &["add", "hello.txt"]);
    host.answers
        .push_back(json!({"values": {"message": "x", "amend": false, "signoff": false}}));
    host.command("commit", &repo, &[]).unwrap();
    assert!(ran.exists(), "the hook did not run");
    assert!(
        host.seen
            .said
            .iter()
            .any(|(t, _)| t == lines::RUNNING_HOOKS),
        "{}",
        host.seen.all_said()
    );
    assert!(
        host.seen
            .progress
            .iter()
            .any(|p| p["message"] == lines::RUNNING_HOOKS)
    );
    assert_eq!(host.seen.last_said(), "Commit stopped: the hook refuses");
    assert_eq!(
        host.seen.texts.last().unwrap()["title"],
        "What the commit hooks said"
    );
}

/// Marks a hook as runnable where the system has such a mark.
fn make_runnable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}
