//! Diffs, the log, a commit's files and blame through the fake host and
//! real git: which two texts each compare gets, in short titles, binary
//! files in words, and rows with the date and subject first.

use serde_json::json;

use super::support::host;

#[test]
fn a_diff_hands_the_compare_screens_the_right_two_texts() {
    let (git, mut host) = host();
    let repo = git.repo("diff");
    // Unstaged: the index against the working tree.
    std::fs::write(repo.join("hello.txt"), "one\n2\nthree\n").unwrap();
    host.command("diff", &repo, &["hello.txt"]).unwrap();
    let diff = host.seen.diffs.last().unwrap().clone();
    assert_eq!(diff["title"], "hello.txt");
    assert_eq!(
        diff["old"],
        json!({"title": "index", "text": "one\ntwo\nthree\n"})
    );
    assert_eq!(
        diff["new"],
        json!({"title": "working tree", "text": "one\n2\nthree\n"})
    );
    // Staged only: HEAD against the index.
    git.run(&repo, &["add", "hello.txt"]);
    host.command("diff", &repo, &["hello.txt"]).unwrap();
    let diff = host.seen.diffs.last().unwrap().clone();
    assert_eq!(diff["old"]["title"], "HEAD");
    assert_eq!(diff["old"]["text"], "one\ntwo\nthree\n");
    assert_eq!(diff["new"]["title"], "index");
    assert_eq!(diff["new"]["text"], "one\n2\nthree\n");
    // Compare with the last commit: HEAD against the working tree.
    std::fs::write(repo.join("hello.txt"), "one\n2\n3\n").unwrap();
    host.command("diff_head", &repo, &["hello.txt"]).unwrap();
    let diff = host.seen.diffs.last().unwrap().clone();
    assert_eq!(diff["old"]["title"], "HEAD");
    assert_eq!(diff["new"]["text"], "one\n2\n3\n");
    // Untracked: nothing against the working tree.
    std::fs::write(repo.join("new.txt"), "n\n").unwrap();
    host.command("diff", &repo, &["new.txt"]).unwrap();
    let diff = host.seen.diffs.last().unwrap().clone();
    assert_eq!(diff["old"], json!({"title": "not tracked", "text": ""}));
    // No change says so; a binary file is said in words with its sizes.
    git.run(&repo, &["add", "."]);
    git.run(&repo, &["commit", "-q", "-m", "Second"]);
    let diffs = host.seen.diffs.len();
    host.command("diff", &repo, &["hello.txt"]).unwrap();
    assert_eq!(host.seen.last_said(), "No changes in hello.txt");
    std::fs::write(repo.join("logo.png"), b"PNG\0\x01\x02").unwrap();
    git.run(&repo, &["add", "logo.png"]);
    git.run(&repo, &["commit", "-q", "-m", "Logo"]);
    std::fs::write(repo.join("logo.png"), b"PNG\0\x01\x02\x03\x04").unwrap();
    host.command("diff", &repo, &["logo.png"]).unwrap();
    assert_eq!(
        host.seen.last_said(),
        "Binary file changed: logo.png, 6 bytes to 8 bytes"
    );
    assert_eq!(host.seen.diffs.len(), diffs);
    // A folder is not compared.
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    host.command("diff", &repo, &["sub/"]).unwrap();
    assert_eq!(host.seen.last_said(), "Not a file: pick a file to compare");
}

#[test]
fn the_log_lists_date_and_subject_first_and_a_commit_s_files() {
    let (git, mut host) = host();
    let repo = git.repo("log");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src").join("lib.rs"), "fn a() {}\n").unwrap();
    git.run(&repo, &["add", "."]);
    git.run(&repo, &["commit", "-q", "-m", "Add the library"]);
    git.run(&repo, &["mv", "hello.txt", "greeting.txt"]);
    std::fs::write(repo.join("src").join("lib.rs"), "fn b() {}\n").unwrap();
    git.run(&repo, &["add", "."]);
    git.run(&repo, &["commit", "-q", "-m", "Rename and change"]);
    host.command("log", &repo, &[]).unwrap();
    let opened = host.seen.listers.last().unwrap().clone();
    assert_eq!(opened["lister"], "log");
    assert_eq!(opened["title"], "Log of log: 3 commits");
    let rows = host.rows("log");
    let date = git.run(&repo, &["log", "-1", "--format=%ad", "--date=short"]);
    let short = git.run(&repo, &["log", "-1", "--format=%h"]);
    assert_eq!(
        rows[0],
        format!("{}, Rename and change, {}", date.trim(), short.trim())
    );
    assert!(rows[2].contains(", First, "));
    // The log of one file.
    host.command("file_log", &repo.join("src"), &["lib.rs"])
        .unwrap();
    assert_eq!(
        host.seen.listers.last().unwrap()["title"],
        "Log of lib.rs: 2 commits"
    );
    // Enter on a commit lists its files; Enter on a file compares it with
    // the commit before.
    host.command("log", &repo, &[]).unwrap();
    host.on_rows("commit_files", "log", &repo, &[0]).unwrap();
    let opened = host.seen.listers.last().unwrap().clone();
    assert_eq!(opened["lister"], "commit");
    assert_eq!(opened["title"], format!("Commit {}: 2 files", short.trim()));
    let files = host.rows("commit");
    assert_eq!(
        files,
        [
            "Renamed: greeting.txt, from hello.txt",
            "Modified: src/lib.rs"
        ]
    );
    host.on_rows("diff", "commit", &repo, &[1]).unwrap();
    let diff = host.seen.diffs.last().unwrap().clone();
    assert_eq!(diff["old"]["title"], format!("before {}", short.trim()));
    assert_eq!(diff["old"]["text"], "fn a() {}\n");
    assert_eq!(diff["new"]["title"], short.trim());
    assert_eq!(diff["new"]["text"], "fn b() {}\n");
    // The first commit's files are everything it added.
    host.on_rows("commit_files", "log", &repo, &[2]).unwrap();
    assert_eq!(host.rows("commit"), ["Added: hello.txt"]);
}

#[test]
fn blame_says_who_changed_the_line_and_when() {
    let (git, mut host) = host();
    let repo = git.repo("blame");
    host.answers.push_back(json!({"text": "2"}));
    host.command("blame", &repo, &["hello.txt"]).unwrap();
    let asked = host.seen.asked.last().unwrap().clone();
    assert_eq!(asked["kind"], "text");
    assert_eq!(asked["message"][0], "Line number: hello.txt");
    let said = host.seen.last_said().to_owned();
    assert!(said.starts_with("Line 2: Ada Example, "), "{said}");
    assert!(said.contains(": First, "), "{said}");
    // The weekday is computed from the commit's own date.
    let date = git.run(
        &repo,
        &["log", "-1", "--format=%ad", "--date=format:%A, %B %d, %Y"],
    );
    let date = date.trim().replace(" 0", " ");
    assert!(said.contains(&date), "{said} / {date}");
    // A line not committed yet says so.
    std::fs::write(repo.join("hello.txt"), "one\nnew two\nthree\n").unwrap();
    host.answers.push_back(json!({"text": "2"}));
    host.command("blame", &repo, &["hello.txt"]).unwrap();
    assert_eq!(host.seen.last_said(), "Line 2: not committed yet");
}
