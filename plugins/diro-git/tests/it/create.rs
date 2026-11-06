//! Clone and init through the fake host and real git, with `file://`
//! addresses of the test's own and nothing on the network: a clone with
//! its progress in words, a failed one leaving no folder, and a new
//! repository with a template and a first commit.

use serde_json::json;

use super::support::{host, slashed};
use diro_git::lines;

#[test]
fn a_clone_says_its_progress_in_words_and_a_failed_one_leaves_no_folder() {
    let (git, mut host) = host();
    let source = git.repo("source");
    for n in 0..20 {
        std::fs::write(
            source.join(format!("f{n}.txt")),
            format!("{n}\n").repeat(200),
        )
        .unwrap();
    }
    git.run(&source, &["add", "."]);
    git.run(&source, &["commit", "-q", "-m", "Files"]);
    let panel = git.path().join("panel");
    std::fs::create_dir_all(&panel).unwrap();
    let address = format!("file://{}", slashed(&source));
    host.answers
        .push_back(json!({"values": {"address": address, "name": ""}}));
    host.command("clone", &panel, &[]).unwrap();
    assert_eq!(host.seen.last_said(), "Cloned into source");
    assert!(panel.join("source").join("f19.txt").exists());
    let form = host.seen.forms.last().unwrap().clone();
    assert_eq!(form["focus"], "address");
    // Progress came from git's own lines, in words, between begin and end.
    let reports: Vec<String> = host
        .seen
        .progress
        .iter()
        .filter(|p| p["kind"] == "report")
        .map(|p| p["message"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        reports
            .iter()
            .any(|r| r.starts_with("Cloning: ") && r.ends_with(" percent")),
        "{reports:?}"
    );
    assert!(
        reports.iter().all(|r| r.chars().count() <= 40),
        "{reports:?}"
    );
    assert_eq!(host.seen.progress.first().unwrap()["kind"], "begin");
    assert_eq!(host.seen.progress.last().unwrap()["kind"], "end");
    // A folder that is there already is never cloned into.
    host.answers
        .push_back(json!({"values": {"address": address, "name": "source"}}));
    host.command("clone", &panel, &[]).unwrap();
    assert_eq!(
        host.seen.last_said(),
        "A folder named source is here already"
    );
    // A name outside the plain-name rule is refused.
    host.answers
        .push_back(json!({"values": {"address": address, "name": "two words"}}));
    host.command("clone", &panel, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::PLAIN_NAME);
    // An address with nothing there: git's reason, and no folder left.
    let missing = format!("file://{}", slashed(&git.path().join("nothing-here.git")));
    host.answers
        .push_back(json!({"values": {"address": missing, "name": "gone"}}));
    host.command("clone", &panel, &[]).unwrap();
    assert!(
        host.seen.last_said().starts_with("Clone stopped: "),
        "{}",
        host.seen.all_said()
    );
    assert!(!panel.join("gone").exists());
    assert!(host.seen.deletes.is_empty());
}

#[test]
fn a_clone_that_stops_partway_hands_its_folder_to_the_trash() {
    let (git, mut host) = host();
    let panel = git.path().join("panel");
    std::fs::create_dir_all(&panel).unwrap();
    // git stops partway and leaves its half-made folder, as a cancelled
    // or broken clone can.
    let left = panel.join("half");
    host.fake = Some(Box::new(move |args: &[String]| {
        if !args.iter().any(|a| a == "clone") {
            return None;
        }
        std::fs::create_dir_all(left.join(".git")).unwrap();
        Some(json!({"exit_code": 128, "stdout": "",
            "stderr": diroxide_plugins::base64::encode(b"fatal: early EOF\n"),
            "encoding": "base64", "truncated": false}))
    }));
    host.answers
        .push_back(json!({"values": {"address": "file:///nowhere/half.git", "name": ""}}));
    host.command("clone", &panel, &[]).unwrap();
    assert_eq!(host.seen.last_said(), "Clone stopped: early EOF");
    // diroxide's delete gets the folder; it lists it and asks first.
    assert_eq!(
        host.seen.deletes[0]["items"][0],
        panel.join("half").display().to_string()
    );
}

#[test]
fn init_makes_a_repository_with_a_template_and_a_first_commit() {
    let (git, mut host) = host();
    let folder = git.path().join("project");
    std::fs::create_dir_all(folder.join("src")).unwrap();
    std::fs::write(folder.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(folder.join("README.md"), "# Project\n").unwrap();
    host.answers
        .push_back(json!({"values": {"branch": "main", "template": "Rust",
        "commit": true, "message": "First commit"}}));
    host.command("init", &folder, &[]).unwrap();
    assert_eq!(
        host.seen.last_said(),
        "Made a repository on main: 3 files committed"
    );
    assert_eq!(
        std::fs::read_to_string(folder.join(".gitignore")).unwrap(),
        "/target\n**/*.rs.bk\n"
    );
    assert_eq!(
        git.run(&folder, &["log", "-1", "--format=%s"]).trim(),
        "First commit"
    );
    assert_eq!(
        git.run(&folder, &["log", "-1", "--format=%an <%ae>"])
            .trim(),
        "Ada Example <ada@example.invalid>"
    );
    host.command("init", &folder, &[]).unwrap();
    assert_eq!(host.seen.last_said(), lines::ALREADY_A_REPOSITORY);
    // Inside another repository, it asks first, starting on No.
    let inner = folder.join("src");
    let cancelled = host.command("init", &inner, &[]).unwrap_err();
    assert_eq!(cancelled["code"], -32800);
    let question = host.seen.asked.last().unwrap().clone();
    assert!(
        question["title"]
            .as_str()
            .unwrap()
            .starts_with("This folder is inside the repository at ")
    );
    assert_eq!(question["destructive"], true);
}
