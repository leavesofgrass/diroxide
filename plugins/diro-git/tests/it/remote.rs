//! Remotes through the fake host and real git, with `file://` remotes of
//! the test's own and nothing on the network: fetch, a fast-forward pull
//! and one that stops, push and the remote's refusal, and the lease push
//! behind its two questions.

use serde_json::json;

use super::support::{host, slashed};
use diro_git::lines;

/// A second working copy of `bare`, to make "someone else's" commits.
fn other_copy(git: &super::support::Git, bare: &std::path::Path, name: &str) -> std::path::PathBuf {
    let address = format!("file://{}", slashed(bare));
    git.run(git.path(), &["clone", "-q", &address, name]);
    let copy = git.path().join(name);
    git.run(&copy, &["config", "core.autocrlf", "false"]);
    copy
}

#[test]
fn fetch_and_pull_fast_forward_only() {
    let (git, mut host) = host();
    let repo = git.repo("mine");
    let bare = git.remote_for(&repo, "pull-origin.git");
    let theirs = other_copy(&git, &bare, "theirs");
    std::fs::write(theirs.join("theirs.txt"), "t\n").unwrap();
    git.run(&theirs, &["add", "theirs.txt"]);
    git.run(&theirs, &["commit", "-q", "-m", "Theirs"]);
    git.run(&theirs, &["push", "-q"]);
    host.command("fetch", &repo, &[]).unwrap();
    assert_eq!(
        host.seen.last_said(),
        "Fetched: main is 1 behind origin/main"
    );
    assert_eq!(
        host.describe(&repo).as_deref(),
        Some("main, 1 behind, no changes here")
    );
    host.command("pull", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), "Pulled 1 commit into main");
    assert!(repo.join("theirs.txt").exists());
    host.command("pull", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::UP_TO_DATE);
    // Both sides moved: the pull stops and says why.
    std::fs::write(theirs.join("theirs.txt"), "t2\n").unwrap();
    git.run(&theirs, &["commit", "-q", "-am", "Theirs again"]);
    git.run(&theirs, &["push", "-q"]);
    std::fs::write(repo.join("mine.txt"), "m\n").unwrap();
    git.run(&repo, &["add", "mine.txt"]);
    git.run(&repo, &["commit", "-q", "-m", "Mine"]);
    host.command("pull", &repo, &[]).unwrap();
    assert!(
        host.seen
            .said
            .iter()
            .any(|(t, i)| t == lines::NOT_FAST_FORWARD && i == "warning"),
        "{}",
        host.seen.all_said()
    );
    assert_eq!(host.seen.last_said(), lines::MERGE_IN_TERMINAL);
    // Every run had the prompt turned off, and asked no credential.
    assert!(host.seen.ran("--ff-only"));
}

#[test]
fn push_asks_first_and_the_remote_s_refusal_is_said() {
    let (git, mut host) = host();
    let repo = git.repo("pusher");
    let bare = git.remote_for(&repo, "push-origin.git");
    host.command("push", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::NOTHING_TO_PUSH);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git.run(&repo, &["add", "a.txt"]);
    git.run(&repo, &["commit", "-q", "-m", "A"]);
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    git.run(&repo, &["add", "b.txt"]);
    git.run(&repo, &["commit", "-q", "-m", "B"]);
    // Escape pushes nothing.
    assert_eq!(
        host.command("push", &repo, &[]).unwrap_err()["code"],
        -32800
    );
    let question = host.seen.asked.last().unwrap().clone();
    assert_eq!(question["title"], "Push 2 commits to origin/main?");
    assert_eq!(question["destructive"], false);
    host.answers.push_back(json!({"answer": true}));
    host.command("push", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), "Pushed 2 commits to origin/main");
    let theirs_head = git.run(&bare, &["rev-parse", "main"]);
    let mine_head = git.run(&repo, &["rev-parse", "main"]);
    assert_eq!(theirs_head, mine_head);
    // Someone else pushed first: refused, never forced.
    let theirs = other_copy(&git, &bare, "pusher-theirs");
    std::fs::write(theirs.join("t.txt"), "t\n").unwrap();
    git.run(&theirs, &["add", "t.txt"]);
    git.run(&theirs, &["commit", "-q", "-m", "T"]);
    git.run(&theirs, &["push", "-q"]);
    std::fs::write(repo.join("c.txt"), "c\n").unwrap();
    git.run(&repo, &["add", "c.txt"]);
    git.run(&repo, &["commit", "-q", "-m", "C"]);
    host.answers.push_back(json!({"answer": true}));
    host.command("push", &repo, &[]).unwrap();
    assert!(
        host.seen
            .said
            .iter()
            .any(|(t, i)| t == lines::PUSH_REFUSED && i == "warning"),
        "{}",
        host.seen.all_said()
    );
    assert_eq!(host.seen.last_said(), lines::PULL_FIRST);
    assert!(
        !host
            .seen
            .tools
            .iter()
            .any(|a| a.iter().any(|w| w == "--force"))
    );
}

#[test]
fn the_lease_push_waits_behind_two_questions_that_name_the_branch() {
    let (git, mut host) = host();
    let repo = git.repo("lease");
    let bare = git.remote_for(&repo, "lease-origin.git");
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git.run(&repo, &["add", "a.txt"]);
    git.run(&repo, &["commit", "-q", "-m", "A"]);
    git.run(&repo, &["push", "-q"]);
    git.run(&repo, &["commit", "-q", "--amend", "-m", "A, reworded"]);
    // The first no stops it.
    host.answers.push_back(json!({"answer": false}));
    assert_eq!(
        host.command("push_lease", &repo, &[]).unwrap_err()["code"],
        -32800
    );
    assert_eq!(host.seen.asked.len(), 1);
    assert_eq!(
        host.seen.asked[0]["title"],
        "Replace history of origin/main?"
    );
    assert_eq!(host.seen.asked[0]["destructive"], true);
    // A yes, then a no at the second, stops it too.
    host.answers.push_back(json!({"answer": true}));
    host.answers.push_back(json!({"answer": false}));
    assert_eq!(
        host.command("push_lease", &repo, &[]).unwrap_err()["code"],
        -32800
    );
    assert_eq!(host.seen.asked[2]["title"], "Really replace origin/main?");
    assert_eq!(host.seen.asked[2]["destructive"], true);
    assert!(!host.seen.ran("--force-with-lease"));
    // Two yeses replace it, with the lease.
    host.answers.push_back(json!({"answer": true}));
    host.answers.push_back(json!({"answer": true}));
    host.command("push_lease", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), "Replaced the history of origin/main");
    assert!(host.seen.ran("--force-with-lease"));
    assert!(host.seen.ran("--force-if-includes"));
    assert_eq!(
        git.run(&bare, &["log", "-1", "--format=%s", "main"]).trim(),
        "A, reworded"
    );
}
