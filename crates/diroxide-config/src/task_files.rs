//! Reading the task files, `tasks/NAME.toml` in the config folder (alpha.7,
//! R28 6.3; plan question 52): one file a task, each read on its own, so
//! one that cannot be used is said and the others load.
//!
//! What this reader says, each problem naming the file first:
//!
//! - a file whose name is not plain ("tasks/My Scans.toml: a task file's
//!   name may hold only letters, digits, - and _."), by its file name;
//!   every other problem names the task by its file's stem first;
//! - text that is not TOML, or a value of the wrong kind, with its line;
//! - a key no task has, with its line, because a misspelled `match` would
//!   otherwise take every file ("Task scans: mtach is not a task
//!   key, line 9.");
//! - `conflicts = "ask"` or `"overwrite"`, refused with the reason (a task
//!   never asks and never replaces a file);
//! - a `name` that is not the file's.
//!
//! A missing `name` takes the file's. The job engine then checks each task
//! in full (its folders, its words, loops between tasks:
//! `diroxide_ops::hot`), and the app says those problems too. Nothing
//! here writes a task file: the owner writes them.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::paths::Paths;
use crate::store::{ConfigProblem, Loaded, ReadText, line_of, read_text};
use crate::tasks::TaskConfig;

/// The keys a task file may hold at its top, and in each table.
pub const TASK_KEYS: [&str; 10] = [
    "name", "about", "kind", "recipe", "script", "plugin", "to", "trigger", "run", "say",
];
/// The keys of `[trigger]`.
pub const TRIGGER_KEYS: [&str; 6] = ["kind", "home", "match", "filter", "settle", "catch-up"];
/// The keys of `[run]`.
pub const RUN_KEYS: [&str; 6] = [
    "after",
    "conflicts",
    "parallel",
    "timeout",
    "retries",
    "online-only",
];
/// The keys of `[say]`.
pub const SAY_KEYS: [&str; 1] = ["announce"];

/// One task file, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFile {
    /// Its name as said in a problem: `tasks/scans.toml`.
    pub file: String,
    /// The task.
    pub task: TaskConfig,
}

/// The file a task of this name lives in.
pub fn task_path(paths: &Paths, name: &str) -> PathBuf {
    paths
        .config_dir
        .join(Paths::TASKS)
        .join(format!("{name}.toml"))
}

/// Whether a file stem is plain: letters, digits, `-` and `_`.
fn is_plain(stem: &str) -> bool {
    !stem.is_empty()
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The line a key is first written on in `text`, counted from 1: the
/// first line whose text starts with the key and an equals sign.
fn line_of_key(text: &str, key: &str) -> Option<usize> {
    text.lines()
        .position(|line| {
            line.trim_start()
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
        .map(|at| at + 1)
}

/// Reads one task file's text. `stem` is the file's name without
/// `.toml`, which a missing `name` takes and every problem names first:
/// "Task scans: mtach is not a task key, line 9."
pub fn read_task(text: &str, stem: &str) -> Result<TaskConfig, String> {
    let table: toml::Table = toml::from_str(text).map_err(|error| {
        let line = error.span().map(|span| line_of(text, span.start));
        match line {
            Some(line) => format!("Task {stem} could not be read: line {line} is not valid TOML."),
            None => format!("Task {stem} could not be read: it is not valid TOML."),
        }
    })?;
    let at = |key: &str| {
        line_of_key(text, key).map_or_else(String::new, |line| format!(", line {line}"))
    };
    let mut unknown = table
        .keys()
        .filter(|key| !TASK_KEYS.contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    for (section, keys) in [
        ("trigger", &TRIGGER_KEYS[..]),
        ("run", &RUN_KEYS[..]),
        ("say", &SAY_KEYS[..]),
    ] {
        match table.get(section) {
            None => {}
            Some(toml::Value::Table(inner)) => unknown.extend(
                inner
                    .keys()
                    .filter(|key| !keys.contains(&key.as_str()))
                    .map(String::as_str),
            ),
            Some(_) => {
                return Err(format!(
                    "Task {stem}: {section} must be a [{section}] table."
                ));
            }
        }
    }
    if let Some(key) = unknown.first() {
        return Err(format!("Task {stem}: {key} is not a task key{}.", at(key)));
    }
    if let Some(conflicts) = table
        .get("run")
        .and_then(|run| run.get("conflicts"))
        .and_then(toml::Value::as_str)
    {
        let why = match conflicts {
            "ask" => Some("a task never asks"),
            "overwrite" | "replace" => Some("a task never replaces a file"),
            _ => None,
        };
        if let Some(why) = why {
            return Err(format!(
                "Task {stem}: {conflicts} is refused{}; {why}.",
                at("conflicts")
            ));
        }
    }
    let mut task: TaskConfig = toml::from_str(text).map_err(|error| {
        let line = error.span().map(|span| line_of(text, span.start));
        let cause = error.message().trim().trim_end_matches('.').to_owned();
        match line {
            Some(line) => format!("Task {stem}: line {line}: {cause}."),
            None => format!("Task {stem}: {cause}."),
        }
    })?;
    let name = task.name.trim().to_owned();
    if name.is_empty() {
        task.name = stem.to_owned();
    } else if name != stem {
        return Err(format!(
            "Task {stem}: its name is {name}; it must be {stem}."
        ));
    }
    Ok(task)
}

/// Reads every task file in the config folder's `tasks` folder, in name
/// order. A missing folder means no tasks; every file that cannot be used
/// is left out and said.
pub fn load_tasks(paths: &Paths) -> Loaded<Vec<TaskFile>> {
    let folder = paths.config_dir.join(Paths::TASKS);
    let mut problems = Vec::new();
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::clean(Vec::new()),
        Err(error) => {
            problems.push(ConfigProblem::new(
                Paths::TASKS,
                format!("The tasks folder could not be read: {error}."),
            ));
            return Loaded {
                value: Vec::new(),
                problems,
            };
        }
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("toml"))
        })
        .collect();
    names.sort();
    let mut tasks = Vec::new();
    for name in names {
        let file = format!("{}/{name}", Paths::TASKS);
        let stem = name
            .rsplit_once('.')
            .map_or(name.as_str(), |(stem, _)| stem);
        if !is_plain(stem) {
            problems.push(ConfigProblem::new(
                &file,
                format!("{file}: a task file's name may hold only letters, digits, - and _."),
            ));
            continue;
        }
        let text = match read_text(&folder.join(&name), &file) {
            ReadText::Text(text) => text,
            ReadText::Missing => continue,
            ReadText::Failed(_) => {
                problems.push(ConfigProblem::new(
                    &file,
                    format!("Task {stem} could not be read: it is not UTF-8 text."),
                ));
                continue;
            }
        };
        match read_task(&text, stem) {
            Ok(task) => tasks.push(TaskFile { file, task }),
            Err(line) => problems.push(ConfigProblem::new(&file, line)),
        }
    }
    Loaded {
        value: tasks,
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{TaskConflicts, TaskKind};

    const SCANS: &str = "name = \"scans\"\nrecipe = \"pdf-to-text\"\n\n[trigger]\nhome = '/srv/scans'\nmatch = [\"*.pdf\"]\n\n[run]\nconflicts = \"keep-both\"\n";

    #[test]
    fn a_task_file_reads_and_its_name_comes_from_the_file() {
        let task = read_task(SCANS, "scans").unwrap();
        assert_eq!(task.name, "scans");
        assert_eq!(task.kind, TaskKind::Convert);
        assert_eq!(task.run.conflicts, TaskConflicts::KeepBoth);
        let nameless = SCANS.replace("name = \"scans\"\n", "");
        assert_eq!(read_task(&nameless, "scans").unwrap().name, "scans");
        assert_eq!(
            read_task(SCANS, "photos").unwrap_err(),
            "Task photos: its name is scans; it must be photos."
        );
    }

    #[test]
    fn each_mistake_is_said_with_its_line() {
        let say = |text: &str| read_task(text, "scans").unwrap_err();
        assert_eq!(
            say(&SCANS.replace("match", "mtach")),
            "Task scans: mtach is not a task key, line 6."
        );
        assert_eq!(
            say(&format!("colour = \"red\"\n{SCANS}")),
            "Task scans: colour is not a task key, line 1."
        );
        assert_eq!(
            say(&SCANS.replace("keep-both", "ask")),
            "Task scans: ask is refused, line 9; a task never asks."
        );
        assert_eq!(
            say(&SCANS.replace("keep-both", "overwrite")),
            "Task scans: overwrite is refused, line 9; a task never replaces a file."
        );
        assert_eq!(
            say(&SCANS.replace("[run]", "[run")),
            "Task scans could not be read: line 8 is not valid TOML."
        );
        assert!(
            say(&SCANS.replace("\"pdf-to-text\"", "3"))
                .starts_with("Task scans: line 2: invalid type"),
            "{}",
            say(&SCANS.replace("\"pdf-to-text\"", "3"))
        );
        assert_eq!(say("run = 3\n"), "Task scans: run must be a [run] table.");
        // The meaning is in the first 40 cells.
        let line = say(&SCANS.replace("keep-both", "ask"));
        assert!(diroxide_core::format::within_cells(
            &line,
            "ask is refused, line 9"
        ));
    }

    /// A scratch folder under the build's own folder, never the system's
    /// temporary folder.
    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tasks-unit-tmp");
        fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("tasks-")
            .tempdir_in(&base)
            .unwrap()
    }

    #[test]
    fn the_folder_is_read_in_name_order_and_bad_files_are_left_out() {
        let dir = scratch();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        assert_eq!(load_tasks(&paths), Loaded::clean(Vec::new()));
        let folder = paths.config_dir.join(Paths::TASKS);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("scans.toml"), SCANS).unwrap();
        fs::write(
            folder.join("inbox.toml"),
            "kind = \"tell\"\n[trigger]\nhome = '/srv/inbox'\n",
        )
        .unwrap();
        fs::write(folder.join("broken.toml"), "kind = \n").unwrap();
        fs::write(folder.join("notes.txt"), "not a task").unwrap();
        fs::create_dir_all(folder.join("old.toml")).unwrap();
        let loaded = load_tasks(&paths);
        let files: Vec<&str> = loaded.value.iter().map(|t| t.file.as_str()).collect();
        assert_eq!(files, ["tasks/inbox.toml", "tasks/scans.toml"]);
        assert_eq!(loaded.value[0].task.name, "inbox");
        let said: Vec<&str> = loaded.problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            ["Task broken could not be read: line 1 is not valid TOML."]
        );
        assert_eq!(loaded.problems[0].file, "tasks/broken.toml");
        assert_eq!(
            task_path(&paths, "scans"),
            paths.config_dir.join("tasks").join("scans.toml")
        );
    }
}
