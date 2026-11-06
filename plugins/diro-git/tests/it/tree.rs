//! The working tree's commands through the fake host and real git: the
//! status list, stage and unstage from the panel and from the list, stage
//! all, Add to .gitignore, and Discard, which keeps what it drops.

use super::support::host;

#[test]
fn the_status_list_says_each_change_state_first() {
    let (git, mut host) = host();
    let repo = git.repo("list");
    // Odd names live only inside git, never on disk: committed, then
    // absent from the working tree, so the list says them as deleted.
    git.index_only(&repo, "two  spaces.txt", "x\n");
    git.index_only(&repo, "caf\u{e9}.txt", "y\n");
    git.run(&repo, &["commit", "-q", "-m", "Odd names"]);
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    git.run(&repo, &["mv", "hello.txt", "moved.txt"]);
    std::fs::write(repo.join("notes.txt"), "n\n").unwrap();
    host.command("status", &repo, &[]).unwrap();
    let opened = host.seen.listers.last().unwrap().clone();
    assert_eq!(opened["lister"], "status");
    assert_eq!(opened["title"], "4 changes, main, no upstream");
    let rows = host.rows("status");
    assert_eq!(
        rows,
        [
            "Deleted: caf\u{e9}.txt",
            "Renamed, modified: moved.txt, from hello.txt",
            "Deleted: two  spaces.txt",
            "Untracked: notes.txt",
        ]
    );
    assert_eq!(opened["rows"][1]["extra"]["state"], "Renamed, modified");
    assert_eq!(opened["rows"][1]["kind"], "item");
    // A name with two spaces survives the trip through a row: its id
    // stands for it, not its words.
    host.on_rows("stage", "status", &repo, &[2]).unwrap();
    assert_eq!(host.seen.last_said(), "Staged two  spaces.txt");
    let staged = git.run(&repo, &["diff", "--cached", "--name-only"]);
    assert!(staged.contains("two  spaces.txt"), "{staged}");
    // Staged from the list, the list comes again with the new states.
    assert_eq!(host.rows("status")[2], "Deleted, staged: two  spaces.txt");
}

#[test]
fn a_name_with_glob_characters_is_that_name_and_nothing_else() {
    let (git, mut host) = host();
    let repo = git.repo("glob");
    // "a[1].txt" lives in git only; "a1.txt", which the glob would match,
    // is a plain file on disk.
    git.index_only(&repo, "a[1].txt", "glob\n");
    git.run(&repo, &["commit", "-q", "-m", "Glob"]);
    std::fs::write(repo.join("a1.txt"), "not this one\n").unwrap();
    host.command("status", &repo, &[]).unwrap();
    let rows = host.rows("status");
    let row = rows.iter().position(|r| r == "Deleted: a[1].txt").unwrap();
    host.on_rows("stage", "status", &repo, &[row]).unwrap();
    let staged = git.run(&repo, &["diff", "--cached", "--name-only"]);
    assert!(staged.contains("a[1].txt"), "{staged}");
    assert!(!staged.contains("a1.txt"), "{staged}");
}

#[test]
fn stage_and_unstage_from_the_panel() {
    let (git, mut host) = host();
    let repo = git.repo("stage");
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    std::fs::write(repo.join("new.txt"), "n\n").unwrap();
    host.command("stage", &repo, &["hello.txt", "new.txt"])
        .unwrap();
    assert_eq!(host.seen.last_said(), "Staged 2 files");
    assert_eq!(
        host.column(&repo, &["hello.txt", "new.txt"]),
        ["Staged", "Added"]
    );
    host.command("unstage", &repo, &["hello.txt"]).unwrap();
    assert_eq!(host.seen.last_said(), "Unstaged hello.txt");
    assert_eq!(host.column(&repo, &["hello.txt"]), ["Modified"]);
    // The file kept its change.
    assert_eq!(
        std::fs::read_to_string(repo.join("hello.txt")).unwrap(),
        "changed\n"
    );
}

#[test]
fn unstage_before_the_first_commit_and_stage_all_here() {
    let (git, mut host) = host();
    let repo = git.path().join("fresh");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    git.run(&repo, &["init", "-q", "--initial-branch=main"]);
    std::fs::write(repo.join("sub").join("a.txt"), "a\n").unwrap();
    std::fs::write(repo.join("sub").join("b.txt"), "b\n").unwrap();
    std::fs::write(repo.join("top.txt"), "t\n").unwrap();
    host.command("stage_all", &repo.join("sub"), &[]).unwrap();
    assert_eq!(host.seen.last_said(), "Staged all changes here: 2 files");
    assert_eq!(
        host.column(&repo, &["top.txt", "sub"]),
        ["Untracked", "2 changed inside"]
    );
    host.command("unstage", &repo.join("sub"), &["a.txt"])
        .unwrap();
    assert_eq!(host.seen.last_said(), "Unstaged sub/a.txt");
    assert_eq!(
        host.column(&repo.join("sub"), &["a.txt", "b.txt"]),
        ["Untracked", "Added"]
    );
    assert_eq!(
        host.describe(&repo).as_deref(),
        Some("main, no commits yet, 3 changed")
    );
}

#[test]
fn add_to_gitignore_writes_anchored_patterns_through_the_host() {
    let (git, mut host) = host();
    let repo = git.repo("ignore");
    std::fs::write(repo.join(".gitignore"), "*.tmp").unwrap();
    std::fs::write(repo.join("debug.log"), "x\n").unwrap();
    std::fs::create_dir_all(repo.join("build")).unwrap();
    std::fs::write(repo.join("build").join("x.o"), "x\n").unwrap();
    host.command("ignore", &repo, &["debug.log", "build/"])
        .unwrap();
    assert_eq!(host.seen.last_said(), "Added 2 patterns to .gitignore");
    let (place, bytes) = host.seen.writes.last().unwrap().clone();
    assert!(place.ends_with(".gitignore"));
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "*.tmp\n/debug.log\n/build/\n"
    );
    assert_eq!(
        host.column(&repo, &["debug.log", "build"]),
        ["Ignored", "Ignored"]
    );
    // Asked again, nothing is added twice.
    host.command("ignore", &repo, &["debug.log"]).unwrap();
    assert_eq!(
        host.seen.last_said(),
        "Already in .gitignore: nothing added"
    );
    // The owner's no to the host's question keeps the file.
    std::fs::write(repo.join("other.log"), "x\n").unwrap();
    host.write_answer = false;
    host.command("ignore", &repo, &["other.log"]).unwrap();
    assert_eq!(host.seen.last_said(), "Kept .gitignore as it was");
}
