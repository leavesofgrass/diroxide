//! The title line and the Git column through the fake host and real git
//! (R29 2.3): the words, "here" as this folder and below, the cache by the
//! index, `HEAD` and the folder, and git missing or too old.

use serde_json::json;

use super::support::host;
use diro_git::lines;

#[test]
fn the_title_says_branch_upstream_and_what_changed_here() {
    let (git, mut host) = host();
    let repo = git.repo("title");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::write(repo.join("sub").join("a.txt"), "a\n").unwrap();
    std::fs::write(repo.join("sub").join("b.txt"), "b\n").unwrap();
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    assert_eq!(
        host.describe(&repo).as_deref(),
        Some("main, no upstream, 2 changed here")
    );
    assert_eq!(
        host.describe(&repo.join("sub")).as_deref(),
        Some("main, no upstream, 1 changed here")
    );
    let _bare = git.remote_for(&repo, "title-origin.git");
    git.run(&repo, &["add", "hello.txt"]);
    git.run(&repo, &["commit", "-q", "-m", "Second"]);
    // The commit changed the index and HEAD, so the title is taken again.
    assert_eq!(
        host.describe(&repo).as_deref(),
        Some("main, 1 ahead, 1 changed here")
    );
    let title = host.describe(&repo).unwrap();
    assert!(title.chars().count() <= lines::TITLE_CELLS, "{title}");
    git.run(&repo, &["checkout", "-q", "--detach"]);
    let detached = host.describe(&repo).unwrap();
    assert!(detached.starts_with("Detached at "), "{detached}");
    // A folder outside any repository gets no words.
    let plain = git.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(host.describe(&plain), None);
}

#[test]
fn the_column_says_each_row_s_state_in_words() {
    let (git, mut host) = host();
    let repo = git.repo("column");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src").join("lib.rs"), "fn x() {}\n").unwrap();
    git.run(&repo, &["add", "src/lib.rs"]);
    git.run(&repo, &["commit", "-q", "-m", "Source"]);
    std::fs::write(repo.join("notes.txt"), "n\n").unwrap();
    std::fs::write(repo.join("staged.txt"), "s\n").unwrap();
    git.run(&repo, &["add", "staged.txt"]);
    std::fs::write(repo.join("staged.txt"), "s2\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "*.log\n/build/\n").unwrap();
    std::fs::write(repo.join("debug.log"), "x\n").unwrap();
    std::fs::create_dir_all(repo.join("build")).unwrap();
    std::fs::write(repo.join("build").join("out.bin"), "x\n").unwrap();
    std::fs::write(repo.join("src").join("lib.rs"), "fn y() {}\n").unwrap();
    std::fs::write(repo.join("src").join("new.rs"), "\n").unwrap();
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    let cells = host.column(
        &repo,
        &[
            "hello.txt",
            "notes.txt",
            "staged.txt",
            "debug.log",
            "build",
            "src",
            ".gitignore",
        ],
    );
    assert_eq!(
        cells,
        [
            "Modified",
            "Untracked",
            "Added, modified",
            "Ignored",
            "Ignored",
            "2 changed inside",
            "Untracked",
        ]
    );
    git.run(&repo, &["add", "."]);
    git.run(&repo, &["commit", "-q", "-m", "All"]);
    assert_eq!(
        host.column(&repo, &["hello.txt", "src"]),
        ["Clean", "Clean"]
    );
    for cell in &cells {
        assert!(cell.chars().count() <= 40);
    }
}

#[test]
fn one_status_serves_the_title_and_the_column_until_a_stamp_changes() {
    let (git, mut host) = host();
    let repo = git.repo("cache");
    std::fs::write(repo.join("hello.txt"), "changed\n").unwrap();
    host.describe(&repo).unwrap();
    let statuses = |host: &super::support::FakeHost| {
        host.seen
            .tools
            .iter()
            .filter(|a| a.iter().any(|w| w == "status"))
            .count()
    };
    assert_eq!(statuses(&host), 1);
    // The column's page right after the listing takes the same status.
    host.column(&repo, &["hello.txt"]);
    assert_eq!(statuses(&host), 1);
    // A change to the index (git add) drops it.
    git.run(&repo, &["add", "hello.txt"]);
    assert_eq!(host.column(&repo, &["hello.txt"]), ["Staged"]);
    assert_eq!(statuses(&host), 2);
    // A command of the plugin's own drops the repository's statuses.
    assert!(!host.plugin.statuses.is_empty());
    host.command("unstage", &repo, &["hello.txt"]).unwrap();
    assert!(host.plugin.statuses.is_empty());
}

#[test]
fn git_too_old_or_missing_is_said_in_words() {
    let (git, mut host) = host();
    let repo = git.repo("old");
    host.fake = Some(Box::new(|args: &[String]| {
        (args.last().map(String::as_str) == Some("--version")).then(|| {
            json!({"exit_code": 0, "stdout": diroxide_plugins::base64::encode(b"git version 2.20.1\n"),
                "stderr": "", "encoding": "base64", "truncated": false})
        })
    }));
    assert_eq!(
        host.describe(&repo).as_deref(),
        Some("git is too old: 2.20, needs 2.30")
    );
    assert_eq!(
        host.column(&repo, &["hello.txt"]),
        ["git is too old: 2.20, needs 2.30"]
    );
    host.command("status", &repo, &[]).unwrap();
    assert!(
        host.seen
            .said
            .iter()
            .any(|(t, i)| t == "Git is too old: 2.20, needs 2.30" && i == "warning"),
        "{}",
        host.seen.all_said()
    );
    assert!(!host.seen.ran("status"));

    // A host that finds no git answers -32010, not found.
    let (git, mut host) = super::support::host();
    let repo = git.repo("missing");
    host.fake = Some(Box::new(|_: &[String]| {
        Some(
            json!({"error": {"code": -32010, "message": "git was not found",
            "data": {"kind": "not_found"}}}),
        )
    }));
    assert_eq!(host.describe(&repo).as_deref(), Some(lines::GIT_NOT_FOUND));
    assert_eq!(host.column(&repo, &["hello.txt"]), [lines::GIT_NOT_FOUND]);
    host.command("status", &repo, &[]).unwrap();
    assert_eq!(
        host.seen.said,
        [
            ("Git not found".to_owned(), "warning".to_owned()),
            (lines::GET_GIT.to_owned(), "info".to_owned())
        ]
    );
}

#[test]
fn a_repository_git_refuses_is_said_in_words_never_as_a_stopped_plugin() {
    let (git, mut host) = host();
    let repo = git.repo("refused");
    // git refuses a folder whose owner it does not trust (another user's,
    // or on a drive with no owners) until it is named in safe.directory.
    host.fake = Some(Box::new(|args: &[String]| {
        args.iter().any(|a| a == "status").then(|| {
            json!({"exit_code": 128, "stdout": "",
                "stderr": diroxide_plugins::base64::encode(
                    b"fatal: detected dubious ownership in repository at 'D:/x'\n"),
                "encoding": "base64", "truncated": false})
        })
    }));
    assert_eq!(host.describe(&repo).as_deref(), Some(lines::DUBIOUS));
    assert_eq!(
        host.column(&repo, &["hello.txt", "notes.txt"]),
        [lines::DUBIOUS, lines::DUBIOUS]
    );
    host.command("status", &repo, &[]).unwrap();
    assert_eq!(
        host.seen.said,
        [
            (
                "Git refuses it: dubious ownership".to_owned(),
                "warning".to_owned()
            ),
            (lines::SAFE_DIRECTORY_HINT.to_owned(), "info".to_owned())
        ]
    );
    // A folder inside .git has no working tree: no words, and the column
    // says it was not checked.
    host.fake = None;
    let inside = repo.join(".git").join("refs");
    assert_eq!(host.describe(&inside), None);
    assert_eq!(host.column(&inside, &["heads"]), [lines::NOT_CHECKED]);
}
