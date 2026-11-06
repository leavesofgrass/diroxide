//! The title line and the Git column through the fake host and real git
//! (R29 2.3): the words, "here" as this folder and below, the cache by the
//! index, `HEAD` and the folder, and git missing or too old.

use serde_json::json;

use super::support::host;
use diro_git::lines;

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
}
