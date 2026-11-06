//! The trust scan's proof (answer 123; spike GS4 as a test): a repository
//! whose own configuration names a program for `core.fsmonitor` and for a
//! filter driver. Plain `git status` runs both; the plugin's title line,
//! its column and its commands run neither until the owner says yes, and
//! after a yes the title line's status still never runs the file monitor.

use serde_json::json;

use super::support::{FakeHost, Git, host, marker_script, slashed};
use diro_git::lines;

/// A repository whose configuration runs `NAME-fsmon.sh` as its file
/// monitor and, with `filter`, `NAME-filter.sh` as the clean filter of
/// every text file, with a changed file so status has a reason to run the
/// filter. Its two marker files appear when they run.
fn hostile(
    git: &Git,
    name: &str,
    filter: bool,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let repo = git.repo(name);
    let (fsmon, fsmon_ran) = marker_script(git, &format!("{name}-fsmon"));
    let (filt, filter_ran) = marker_script(git, &format!("{name}-filter"));
    git.run(
        &repo,
        &[
            "config",
            "core.fsmonitor",
            &format!("sh {}", slashed(&fsmon)),
        ],
    );
    if filter {
        git.run(
            &repo,
            &[
                "config",
                "filter.evil.clean",
                &format!("sh {}", slashed(&filt)),
            ],
        );
        std::fs::write(repo.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
    }
    // The same size and a new time: status cannot tell from the file's
    // stat whether it changed, so it reads it through the clean filter.
    std::fs::write(repo.join("hello.txt"), "one\ntwo\nthrex\n").unwrap();
    (repo, fsmon_ran, filter_ran)
}

/// Whether the plugin ran git for anything but its version and the scan.
fn ran_more_than_the_scan(host: &FakeHost) -> bool {
    !host
        .seen
        .tools
        .iter()
        .all(|args| args.iter().any(|a| a == "--version" || a == "config"))
}

#[test]
fn the_plugin_runs_nothing_of_an_untrusted_configuration_and_plain_status_does() {
    let (git, mut host) = host();
    let (repo, fsmon_ran, filter_ran) = hostile(&git, "hostile", true);

    // The title line asks once, starting on No, and names what it found;
    // the owner says no.
    host.answers.push_back(json!({"answer": false}));
    assert_eq!(host.describe(&repo).as_deref(), Some(lines::NOT_TRUSTED));
    assert_eq!(host.seen.asked.len(), 1);
    let question = &host.seen.asked[0];
    assert_eq!(question["title"], lines::TRUST_TITLE);
    assert_eq!(question["destructive"], true);
    assert_eq!(question["message"][0], lines::NOT_TRUSTED);
    let message = question["message"].to_string();
    assert!(message.contains("core.fsmonitor runs sh"), "{message}");
    assert!(message.contains("filter.evil.clean runs sh"), "{message}");
    // The next listing says the same without a question.
    assert_eq!(host.describe(&repo).as_deref(), Some(lines::NOT_TRUSTED));
    assert_eq!(host.seen.asked.len(), 1);
    // The column never asks; it says "not checked".
    assert_eq!(
        host.column(&repo, &["hello.txt", "notes.txt"]),
        [lines::NOT_CHECKED, lines::NOT_CHECKED]
    );
    // git ran only for its version and the scan: no status, no diff.
    assert!(!ran_more_than_the_scan(&host), "{:?}", host.seen.tools);
    // The no was kept: a new session says the same without a question.
    drop(host);
    let mut again = FakeHost::start(std::sync::Arc::clone(&git));
    assert_eq!(again.describe(&repo).as_deref(), Some(lines::NOT_TRUSTED));
    assert!(again.seen.asked.is_empty());
    assert!(!ran_more_than_the_scan(&again));
    drop(again);
    assert!(!fsmon_ran.exists(), "the file monitor ran");
    assert!(!filter_ran.exists(), "the filter ran");

    // The control: plain git status runs the configuration's programs.
    let out = git.output(&repo, &["status", "--porcelain"]);
    assert!(out.status.success());
    assert!(
        fsmon_ran.exists(),
        "plain status did not run the file monitor"
    );
    assert!(filter_ran.exists(), "plain status did not run the filter");
}

#[test]
fn a_command_line_override_keeps_the_file_monitor_from_running() {
    let (git, _host) = host();
    let (repo, fsmon_ran, _) = hostile(&git, "monitor", false);
    let out = git.output(
        &repo,
        &[
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain",
        ],
    );
    assert!(out.status.success());
    assert!(!fsmon_ran.exists(), "-c core.fsmonitor=false still ran it");
    // And the scan finds it without running it.
    let listing = git.output(
        &repo,
        &["config", "--list", "-z", "--show-scope", "--includes"],
    );
    let scan = diro_git::git::trust::scan(&listing.stdout);
    assert_eq!(scan.hits[0].key, "core.fsmonitor");
    assert!(!fsmon_ran.exists());
}

#[test]
fn a_yes_lets_status_run_without_the_monitor_and_a_changed_config_asks_again() {
    let (git, mut host) = host();
    let (repo, fsmon_ran, _) = hostile(&git, "trusted", false);
    host.answers.push_back(json!({"answer": true}));
    let title = host.describe(&repo).unwrap();
    assert_eq!(title, "main, no upstream, 1 changed here");
    let status = host
        .seen
        .tools
        .iter()
        .find(|a| a.iter().any(|w| w == "status"))
        .unwrap()
        .clone();
    assert_eq!(
        &status[..3],
        ["--no-optional-locks", "-c", "core.fsmonitor=false"]
    );
    assert!(
        !fsmon_ran.exists(),
        "the trusted status ran the file monitor"
    );
    assert_eq!(host.column(&repo, &["hello.txt"]), ["Modified"]);
    // The yes is kept by the configuration's digest: a new session takes
    // it, and a changed configuration asks again, starting on No.
    drop(host);
    let mut again = FakeHost::start(std::sync::Arc::clone(&git));
    assert_eq!(again.describe(&repo).as_deref(), Some(title.as_str()));
    assert!(again.seen.asked.is_empty());
    git.run(&repo, &["config", "alias.st", "!sh -c true"]);
    drop(again);
    let mut changed = FakeHost::start(std::sync::Arc::clone(&git));
    assert_eq!(changed.describe(&repo).as_deref(), Some(lines::NOT_TRUSTED));
    assert_eq!(changed.seen.asked.len(), 1);
    assert!(!fsmon_ran.exists());
}

#[test]
fn a_clean_configuration_asks_nothing() {
    let (git, mut host) = host();
    let repo = git.repo("clean");
    assert_eq!(
        host.describe(&repo).as_deref(),
        Some("main, no upstream, no changes")
    );
    assert!(host.seen.asked.is_empty());
}

#[test]
fn a_command_asks_when_the_title_line_was_not_answered() {
    let (git, mut host) = host();
    let (repo, fsmon_ran, filter_ran) = hostile(&git, "unanswered", true);
    // Escape at the title line's question keeps nothing.
    assert_eq!(host.describe(&repo).as_deref(), Some(lines::NOT_TRUSTED));
    // A command the owner starts asks once more, and a no stops it.
    host.answers.push_back(json!({"answer": false}));
    host.command("status", &repo, &[]).unwrap();
    assert_eq!(host.seen.asked.len(), 2);
    assert!(
        host.seen
            .said
            .iter()
            .any(|(t, i)| t == lines::NOT_TRUSTED && i == "warning"),
        "{}",
        host.seen.all_said()
    );
    assert!(host.seen.listers.is_empty());
    assert!(!ran_more_than_the_scan(&host), "{:?}", host.seen.tools);
    assert!(!fsmon_ran.exists() && !filter_ran.exists());
}
