//! The task format's checks, in Windows and Unix paths on every system.

use std::time::Duration;

use diroxide_core::format::within_cells;
use diroxide_core::place::PathStyle;
use diroxide_core::{ConflictPolicy, Place};

use super::refuse::Surroundings;
use super::task::{
    After, Announce, CatchUpChoice, OnlineOnly, Task, TaskAction, TaskProblem, parse_settle, title,
};

/// R28 6.3's example, word for word.
const SCANS: &str = r#"
# tasks/scans.toml
name = "scans"                                  # plain: letters, digits, - and _
about = "Makes a text file from each scanned PDF."
kind = "convert"                                # convert, move, copy, script, plugin, tell
recipe = "pdf-to-text"

[trigger]
kind = "folder"
home = 'D:\scans'                               # in, out, processed and failed live here
match = ["*.pdf"]
settle = "2s"
catch-up = "ask"                                # ask, all or none

[run]
after = "move"                                  # move, leave, or a named folder; never a removal
conflicts = "keep-both"                         # keep-both, skip or newer; ask and overwrite are refused
parallel = 1
timeout = "10m"
retries = 3
online-only = "skip"                            # skip or fetch

[say]
announce = "summary"                            # summary, each, failures or off
"#;

fn windows() -> Surroundings {
    Surroundings::none(PathStyle::Windows)
}

fn win(text: &str) -> Place {
    Place::parse_with(text, PathStyle::Windows).unwrap()
}

/// The lines a task's problems say.
fn problems(text: &str) -> Vec<String> {
    match Task::from_toml(text, "tasks/scans.toml", &windows()) {
        Ok(task) => panic!("expected problems, got {task:?}"),
        Err(problems) => problems.iter().map(TaskProblem::line).collect(),
    }
}

#[test]
fn r28s_example_reads_whole() {
    let task = Task::from_toml(SCANS, "tasks/scans.toml", &windows()).unwrap();
    assert_eq!(task.name, "scans");
    assert_eq!(task.title(), "Scans");
    assert_eq!(task.about, "Makes a text file from each scanned PDF.");
    assert_eq!(
        task.action,
        TaskAction::Convert {
            recipe: "pdf-to-text".into()
        }
    );
    assert_eq!(task.folders.home, win("D:\\scans"));
    assert_eq!(task.folders.input, win("D:\\scans\\in"));
    assert_eq!(task.folders.out, win("D:\\scans\\out"));
    assert_eq!(task.folders.processed, win("D:\\scans\\processed"));
    assert_eq!(task.folders.failed, win("D:\\scans\\failed"));
    assert_eq!(task.settle, Duration::from_secs(2));
    assert_eq!(task.catch_up, CatchUpChoice::Ask);
    assert_eq!(task.after, After::Move);
    assert_eq!(task.conflicts, ConflictPolicy::KeepBoth);
    assert_eq!(task.parallel, 1);
    assert_eq!(task.timeout, Some(Duration::from_secs(600)));
    assert_eq!(task.retries, 3);
    assert_eq!(task.online_only, OnlineOnly::Skip);
    assert_eq!(task.announce, Announce::Summary);
    assert!(task.matches_name("scan-004.pdf"));
    assert!(task.matches_name("SCAN-004.PDF"));
    assert!(!task.matches_name("scan-004.txt"));
    let names: Vec<&str> = task.needs_folders().iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["in", "out", "processed", "failed"]);
    // The normalized form reads back as the same task.
    let again = Task::from_shape(&task.to_shape(), &windows()).unwrap();
    assert_eq!(again, task);
}

#[test]
fn the_defaults_fill_a_short_task() {
    let task = Task::from_toml(
        "name = \"inbox\"\nkind = \"tell\"\n[trigger]\nhome = 'D:\\inbox'\n",
        "tasks/inbox.toml",
        &windows(),
    )
    .unwrap();
    assert_eq!(task.action, TaskAction::Tell);
    assert!(task.matches.is_empty());
    assert!(task.matches_name("anything.bin"));
    assert_eq!(task.settle, Duration::from_secs(2));
    assert_eq!(task.timeout, None);
    let names: Vec<&str> = task.needs_folders().iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["in"]);
    assert!(task.writes_into().is_empty());
}

#[test]
fn ask_overwrite_and_removal_are_refused_with_their_reason() {
    let ask = SCANS.replace("conflicts = \"keep-both\"", "conflicts = \"ask\"");
    assert_eq!(
        problems(&ask),
        ["Task scans: ask is refused; a task never asks."]
    );
    let overwrite = SCANS.replace("conflicts = \"keep-both\"", "conflicts = \"overwrite\"");
    assert_eq!(
        problems(&overwrite),
        ["Task scans: overwrite is refused; a task never replaces a file."]
    );
    let delete = SCANS.replace("after = \"move\"", "after = \"delete\"");
    assert_eq!(
        problems(&delete),
        ["Task scans: delete is refused; a task never removes a file."]
    );
    let trash = SCANS.replace("after = \"move\"", "after = \"trash\"");
    assert_eq!(
        problems(&trash),
        ["Task scans: trash is refused; a task never removes a file."]
    );
    for line in problems(&ask)
        .iter()
        .chain(&problems(&overwrite))
        .chain(&problems(&delete))
    {
        let verdict = line.split(';').next().unwrap();
        assert!(within_cells(line, verdict), "{line}");
    }
}

#[test]
fn every_problem_is_said_not_only_the_first() {
    let bad = SCANS
        .replace("home = 'D:\\scans'", "home = '%USERPROFILE%\\scans'")
        .replace("settle = \"2s\"", "settle = \"soon\"")
        .replace("parallel = 1", "parallel = 9")
        .replace("retries = 3", "retries = 40")
        .replace("announce = \"summary\"", "announce = \"loud\"")
        .replace("catch-up = \"ask\"", "catch-up = \"some\"")
        .replace("online-only = \"skip\"", "online-only = \"download\"")
        .replace("timeout = \"10m\"", "timeout = \"forever\"");
    assert_eq!(
        problems(&bad),
        [
            "Task scans: home has a variable.",
            "Task scans: settle soon is not a time.",
            "Task scans: catch-up cannot be some; use ask, all or none.",
            "Task scans: parallel must be 1 to 4.",
            "Task scans: timeout forever is not a time limit.",
            "Task scans: retries must be 0 to 5.",
            "Task scans: online-only cannot be download; use skip or fetch.",
            "Task scans: announce cannot be loud; use summary, each, failures or off.",
        ]
    );
}

#[test]
fn names_kinds_and_their_keys_must_agree() {
    assert_eq!(
        problems(&SCANS.replace("name = \"scans\"", "name = \"my scans\"")),
        ["Task my scans: its name may hold only letters, digits, - and _."]
    );
    assert_eq!(
        problems(&SCANS.replace("name = \"scans\"", "name = \"\"")),
        ["Task unnamed: it has no name."]
    );
    assert_eq!(
        problems(&SCANS.replace("kind = \"convert\"", "kind = \"zip\"")),
        ["Task scans: kind cannot be zip; use convert, move, copy, script, plugin or tell."]
    );
    assert_eq!(
        problems(&SCANS.replace("recipe = \"pdf-to-text\"", "")),
        ["Task scans: a convert task needs a recipe."]
    );
    let moving = SCANS.replace("kind = \"convert\"", "kind = \"move\"");
    assert_eq!(
        problems(&moving),
        [
            "Task scans: recipe is not used by a move task.",
            "Task scans: a move task needs to."
        ]
    );
    let moving = moving
        .replace("recipe = \"pdf-to-text\"", "to = 'D:\\software'")
        .replace("after = \"move\"", "after = \"leave\"");
    assert_eq!(
        problems(&moving),
        ["Task scans: after is not used by a move task."]
    );
    let moving = moving.replace("after = \"leave\"", "after = \"move\"");
    let task = Task::from_toml(&moving, "tasks/scans.toml", &windows()).unwrap();
    assert_eq!(task.action, TaskAction::Move);
    assert_eq!(task.folders.to, Some(win("D:\\software")));
    let names: Vec<&str> = task.needs_folders().iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["in", "failed"]);
    let script = SCANS
        .replace("kind = \"convert\"", "kind = \"script\"")
        .replace("recipe = \"pdf-to-text\"", "script = \"script.ocr-pages\"");
    let task = Task::from_toml(&script, "tasks/scans.toml", &windows()).unwrap();
    assert_eq!(
        task.action,
        TaskAction::Script {
            id: "script.ocr-pages".into()
        }
    );
    let odd = script.replace("script.ocr-pages", "ocr pages; rm");
    assert_eq!(
        problems(&odd),
        ["Task scans: script may hold only letters, digits, dots, - and _."]
    );
}

#[test]
fn nothing_but_arrivals_goes_into_in() {
    let into_in = SCANS.replace("after = \"move\"", "after = 'D:\\scans\\in\\done'");
    assert_eq!(
        problems(&into_in),
        ["Task scans: after is in the in folder: it loops into itself."]
    );
    let copy_in = SCANS
        .replace("kind = \"convert\"", "kind = \"copy\"")
        .replace("recipe = \"pdf-to-text\"", "to = 'd:\\SCANS\\IN'");
    assert_eq!(
        problems(&copy_in),
        ["Task scans: to is in the in folder: it loops into itself."]
    );
    let home = SCANS.replace("after = \"move\"", "after = 'D:\\scans'");
    assert_eq!(
        problems(&home),
        ["Task scans: after is the home folder; name a folder of its own."]
    );
    let elsewhere = SCANS.replace("after = \"move\"", "after = 'E:\\archive\\scans'");
    let task = Task::from_toml(&elsewhere, "tasks/scans.toml", &windows()).unwrap();
    assert_eq!(task.after, After::Folder(win("E:\\archive\\scans")));
}

#[test]
fn refused_folders_name_the_key_first() {
    let around = Surroundings {
        home: Some(win("C:\\Users\\Ada")),
        ..windows()
    };
    let root = SCANS.replace("home = 'D:\\scans'", "home = 'D:\\'");
    let user = SCANS.replace("home = 'D:\\scans'", "home = 'C:\\Users\\Ada'");
    let trash = SCANS
        .replace("kind = \"convert\"", "kind = \"move\"")
        .replace("recipe = \"pdf-to-text\"", "to = 'D:\\$Recycle.Bin'");
    let lines = |text: &str| -> Vec<String> {
        Task::from_toml(text, "tasks/scans.toml", &around)
            .unwrap_err()
            .iter()
            .map(TaskProblem::line)
            .collect()
    };
    assert_eq!(
        lines(&root),
        ["Task scans: home is too broad, a drive root."]
    );
    assert_eq!(
        lines(&user),
        ["Task scans: home is too broad, your home folder."]
    );
    // A trash written with its dollar sign is a variable first.
    assert_eq!(lines(&trash), ["Task scans: to has a variable."]);
    for line in lines(&root).iter().chain(&lines(&user)) {
        assert!(within_cells(line, "home is too broad"), "{line}");
    }
}

#[test]
fn unknown_keys_and_wrong_kinds_of_value_are_said() {
    let typo = SCANS.replace("match = ", "mtach = ");
    let lines = problems(&typo);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("Task tasks/scans.toml: unknown field `mtach`"),
        "{lines:?}"
    );
    let wrong = SCANS.replace("parallel = 1", "parallel = \"one\"");
    let lines = problems(&wrong);
    assert!(
        lines[0].starts_with("Task tasks/scans.toml: invalid type"),
        "{lines:?}"
    );
}

#[test]
fn match_patterns_name_files_not_paths() {
    let path = SCANS.replace("[\"*.pdf\"]", "[\"in/*.pdf\"]");
    assert_eq!(
        problems(&path),
        ["Task scans: match in/*.pdf is a path; a pattern names files."]
    );
    let bracket = SCANS.replace("[\"*.pdf\"]", "[\"[a-*.pdf\"]");
    let lines = problems(&bracket);
    assert!(
        lines[0].starts_with("Task scans: match [a-*.pdf needs care"),
        "{lines:?}"
    );
    let several = SCANS.replace("[\"*.pdf\"]", "[\"*.pdf\", \"re:^scan-\\\\d+\\\\.tif$\"]");
    let task = Task::from_toml(&several, "tasks/scans.toml", &windows()).unwrap();
    assert!(task.matches_name("a.pdf"));
    assert!(task.matches_name("scan-12.tif"));
    assert!(!task.matches_name("photo.tif"));
}

#[test]
fn unix_paths_follow_their_own_rules() {
    let around = Surroundings::none(PathStyle::Unix);
    let unix = SCANS.replace("home = 'D:\\scans'", "home = '/srv/scans'");
    let task = Task::from_toml(&unix, "tasks/scans.toml", &around).unwrap();
    assert_eq!(task.folders.input.to_string(), "/srv/scans/in");
    let tilde = SCANS.replace("home = 'D:\\scans'", "home = '~/scans'");
    let lines: Vec<String> = Task::from_toml(&tilde, "tasks/scans.toml", &around)
        .unwrap_err()
        .iter()
        .map(TaskProblem::line)
        .collect();
    assert_eq!(
        lines,
        ["Task scans: home has a ~, which is never expanded."]
    );
}

#[test]
fn settle_reads_from_half_a_second_to_an_hour() {
    assert_eq!(parse_settle("500ms"), Ok(Duration::from_millis(500)));
    assert_eq!(parse_settle("2s"), Ok(Duration::from_secs(2)));
    assert_eq!(parse_settle("1m"), Ok(Duration::from_secs(60)));
    assert_eq!(parse_settle("1h"), Ok(Duration::from_secs(3600)));
    assert_eq!(parse_settle("100ms"), Err("is shorter than half a second"));
    assert_eq!(parse_settle("2h"), Err("is longer than an hour"));
    assert_eq!(parse_settle("2 days"), Err("is not a time"));
    assert_eq!(title("photo-inbox"), "Photo inbox");
    assert_eq!(title("scans"), "Scans");
}
