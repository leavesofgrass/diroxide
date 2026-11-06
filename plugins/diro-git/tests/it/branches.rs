//! Branches and stashes through the fake host and real git: the list, a
//! switch git refuses and says why, new and renamed branches, and stashes
//! pushed, shown, popped and dropped, a drop starting on No.

use serde_json::json;

use super::support::host;
use diro_git::lines;

#[test]
fn branches_list_switch_and_refuse_to_lose_changes() {
    let (git, mut host) = host();
    let repo = git.repo("branches");
    git.run(&repo, &["branch", "topic"]);
    host.command("branches", &repo, &[]).unwrap();
    let opened = host.seen.listers.last().unwrap().clone();
    assert_eq!(opened["title"], "Branches: 2, on main");
    assert_eq!(
        host.rows("branches"),
        ["main, current, no upstream", "topic, no upstream"]
    );
    // Switching from the list.
    host.on_rows("switch", "branches", &repo, &[1]).unwrap();
    assert_eq!(host.seen.said[0].0, "Switched to topic");
    assert_eq!(
        git.run(&repo, &["branch", "--show-current"]).trim(),
        "topic"
    );
    // A change on topic that main would overwrite: git refuses, and the
    // words say which file and what to do.
    std::fs::write(repo.join("hello.txt"), "topic's text\n").unwrap();
    git.run(&repo, &["commit", "-q", "-am", "On topic"]);
    std::fs::write(repo.join("hello.txt"), "local change\n").unwrap();
    host.command("branches", &repo, &[]).unwrap();
    let main = host
        .rows("branches")
        .iter()
        .position(|r| r.starts_with("main"))
        .unwrap();
    host.on_rows("switch", "branches", &repo, &[main]).unwrap();
    assert!(
        host.seen
            .said
            .iter()
            .any(|(t, i)| t == "Cannot switch, local changes: hello.txt" && i == "warning"),
        "{}",
        host.seen.all_said()
    );
    assert_eq!(host.seen.last_said(), lines::COMMIT_OR_STASH);
    assert_eq!(
        git.run(&repo, &["branch", "--show-current"]).trim(),
        "topic"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("hello.txt")).unwrap(),
        "local change\n"
    );
}

#[test]
fn new_and_renamed_branches_check_their_names() {
    let (git, mut host) = host();
    let repo = git.repo("names");
    host.answers.push_back(json!({"text": "feature/one"}));
    host.command("branch_new", &repo, &[]).unwrap();
    assert_eq!(
        host.seen.last_said(),
        "Switched to a new branch: feature/one"
    );
    assert_eq!(
        git.run(&repo, &["branch", "--show-current"]).trim(),
        "feature/one"
    );
    host.answers.push_back(json!({"text": "bad..name"}));
    host.command("branch_new", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::BAD_BRANCH_NAME);
    host.answers.push_back(json!({"text": "-x"}));
    host.command("branch_new", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::BAD_BRANCH_NAME);
    // Rename the current branch: the question starts with its name.
    host.answers.push_back(json!({"text": "feature/two"}));
    host.command("branch_rename", &repo, &[]).unwrap();
    assert_eq!(host.seen.asked.last().unwrap()["initial"], "feature/one");
    assert_eq!(
        host.seen.last_said(),
        "Renamed branch feature/one to feature/two"
    );
    assert_eq!(
        git.run(&repo, &["branch", "--show-current"]).trim(),
        "feature/two"
    );
}

#[test]
fn stashes_push_show_pop_and_drop_after_a_question() {
    let (git, mut host) = host();
    let repo = git.repo("stashes");
    host.answers.push_back(json!({"text": "half done"}));
    host.command("stash_push", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::NOTHING_TO_STASH);
    std::fs::write(repo.join("hello.txt"), "half\n").unwrap();
    host.answers.push_back(json!({"text": "half done"}));
    host.command("stash_push", &repo, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::STASHED);
    assert_eq!(
        std::fs::read_to_string(repo.join("hello.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
    std::fs::write(repo.join("hello.txt"), "second\n").unwrap();
    git.run(&repo, &["stash", "push", "-q", "-m", "second try"]);
    host.command("stashes", &repo, &[]).unwrap();
    let rows = host.rows("stashes");
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].starts_with("Stash 0: second try, on main, "),
        "{rows:?}"
    );
    assert!(
        rows[1].starts_with("Stash 1: half done, on main, "),
        "{rows:?}"
    );
    // Enter shows a stash's changes as a document.
    host.on_rows("stash_show", "stashes", &repo, &[1]).unwrap();
    let text = host.seen.texts.last().unwrap().clone();
    assert_eq!(text["title"], "Stash 1");
    assert_eq!(text["kind"], "diff");
    assert!(text["text"].as_str().unwrap().contains("+half"));
    // A drop asks first and starts on No; a no keeps it.
    host.answers.push_back(json!({"answer": false}));
    let cancelled = host
        .on_rows("stash_drop", "stashes", &repo, &[0])
        .unwrap_err();
    assert_eq!(cancelled["code"], -32800);
    let question = host.seen.asked.last().unwrap().clone();
    assert_eq!(question["title"], "Drop stash 0?");
    assert_eq!(question["destructive"], true);
    assert_eq!(question["message"][0], "Stash 0: second try");
    assert_eq!(git.run(&repo, &["stash", "list"]).lines().count(), 2);
    // The numbers moved since the list was shown: refused, never the
    // wrong stash.
    git.run(&repo, &["stash", "drop", "-q", "stash@{0}"]);
    host.on_rows("stash_pop", "stashes", &repo, &[1]).unwrap();
    assert_eq!(host.seen.last_said(), lines::STASH_MOVED);
    // Shown again, the pop brings it back and removes it.
    host.command("stashes", &repo, &[]).unwrap();
    host.on_rows("stash_pop", "stashes", &repo, &[0]).unwrap();
    assert_eq!(host.seen.last_said(), "Popped stash 0");
    assert_eq!(
        std::fs::read_to_string(repo.join("hello.txt")).unwrap(),
        "half\n"
    );
    assert_eq!(git.run(&repo, &["stash", "list"]).lines().count(), 0);
}
