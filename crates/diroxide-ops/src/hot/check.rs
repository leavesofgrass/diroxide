//! The tasks checked together (R16 2.4 and 3.1): two tasks never share an
//! `in` folder, and no chain of tasks, one's results or originals landing
//! in another's `in`, comes back to where it started. A task on such a
//! chain never runs: "Task scans: it loops into itself."

use serde::Serialize;

use super::refuse::{Surroundings, same};
use super::task::{Task, TaskProblem};

/// The most tasks that may be on at once (R16 2.8): a starting value.
pub const MOST_TASKS: usize = 50;

/// Tasks checked one by one and together: the ones that may run, and
/// every problem, each naming its task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// The tasks with no problem, in the order given.
    pub tasks: Vec<Task>,
    /// What is wrong with the others, in the order found.
    pub problems: Vec<TaskProblem>,
}

/// Checks each task given as written (the config crate's `TaskConfig`,
/// with its file's name for a task whose keys cannot be read), then the
/// set: a task that shares its `in` with an earlier one, or loops, is
/// left out with its reason.
pub fn check_tasks<T: Serialize>(written: &[(String, T)], around: &Surroundings) -> Checked {
    let mut checked = Checked::default();
    for (label, value) in written {
        match Task::from_serialized(value, label, around) {
            Ok(task) => {
                if checked.tasks.iter().any(|t| t.name == task.name) {
                    checked.problems.push(TaskProblem::new(
                        &task.name,
                        "its name is taken by another task",
                    ));
                } else {
                    checked.tasks.push(task);
                }
            }
            Err(problems) => checked.problems.extend(problems),
        }
    }
    let problems = check_set(&checked.tasks);
    checked
        .tasks
        .retain(|task| !problems.iter().any(|p| p.task == task.name));
    checked.problems.extend(problems);
    checked
}

/// The problems of a set of checked tasks: a shared `in` (said for the
/// later task) and every task on a loop.
pub fn check_set(tasks: &[Task]) -> Vec<TaskProblem> {
    let mut problems = Vec::new();
    for (later, task) in tasks.iter().enumerate() {
        if let Some(earlier) = tasks[..later]
            .iter()
            .find(|e| same(&e.folders.input, &task.folders.input))
        {
            problems.push(TaskProblem::new(
                &task.name,
                format!("its in folder is used by task {}", earlier.name),
            ));
        }
    }
    // An edge from a task to each task whose `in` it writes into.
    let feeds = |from: &Task, to: &Task| {
        from.writes_into()
            .iter()
            .any(|folder| same(folder, &to.folders.input))
    };
    for (start, task) in tasks.iter().enumerate() {
        let mut seen = vec![false; tasks.len()];
        let mut stack = vec![start];
        let mut loops = false;
        while let Some(at) = stack.pop() {
            for (next, other) in tasks.iter().enumerate() {
                if !feeds(&tasks[at], other) {
                    continue;
                }
                if next == start {
                    loops = true;
                    break;
                }
                if !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
            if loops {
                break;
            }
        }
        if loops {
            problems.push(TaskProblem::new(&task.name, "it loops into itself"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::task::TaskShape;
    use diroxide_core::format::within_cells;
    use diroxide_core::place::PathStyle;

    fn shape(name: &str, home: &str) -> TaskShape {
        let mut shape = TaskShape {
            name: name.to_owned(),
            recipe: Some("pdf-to-text".to_owned()),
            ..TaskShape::default()
        };
        shape.trigger.home = home.to_owned();
        shape
    }

    fn written(shapes: Vec<TaskShape>) -> Vec<(String, TaskShape)> {
        shapes
            .into_iter()
            .map(|s| (format!("tasks/{}.toml", s.name), s))
            .collect()
    }

    #[test]
    fn a_shared_queue_and_a_chain_back_to_itself_are_refused() {
        let around = Surroundings::none(PathStyle::Windows);
        let mut photos = shape("photos", "D:\\photos");
        photos.trigger.home = "D:\\photos".to_owned();
        let twin = shape("twin", "D:\\photos");
        // a's results land in b's queue, b's in a's.
        let mut a = shape("a", "D:\\a");
        a.kind = "move".to_owned();
        a.recipe = None;
        a.to = Some("D:\\b\\in".to_owned());
        let mut b = shape("b", "D:\\b");
        b.kind = "copy".to_owned();
        b.recipe = None;
        b.to = Some("D:\\a\\in".to_owned());
        // c feeds a, but nothing feeds c back.
        let mut c = shape("c", "D:\\c");
        c.kind = "move".to_owned();
        c.recipe = None;
        c.to = Some("D:\\a\\in".to_owned());
        let checked = check_tasks(&written(vec![photos, twin, a, b, c]), &around);
        let names: Vec<&str> = checked.tasks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["photos", "c"]);
        let lines: Vec<String> = checked.problems.iter().map(TaskProblem::line).collect();
        assert_eq!(
            lines,
            [
                "Task twin: its in folder is used by task photos.",
                "Task a: it loops into itself.",
                "Task b: it loops into itself.",
            ]
        );
        assert!(within_cells(&lines[0], "its in folder is used"));
        assert!(
            lines[1..].iter().all(|l| l.chars().count() <= 40),
            "{lines:?}"
        );
    }

    #[test]
    fn a_tasks_own_failed_folder_feeding_another_counts_as_a_loop() {
        let around = Surroundings::none(PathStyle::Unix);
        let one = shape("one", "/srv/one");
        // two watches one's failed folder and puts its originals back
        // into one's queue: a loop through failed.
        let mut two = shape("two", "/srv/two");
        two.kind = "move".to_owned();
        two.recipe = None;
        two.to = Some("/srv/one/in".to_owned());
        let mut checked = check_tasks(&written(vec![one, two]), &around);
        assert!(checked.problems.is_empty(), "{:?}", checked.problems);
        // Now point two's queue at one's failed folder.
        let mut tasks = std::mem::take(&mut checked.tasks);
        let failed = tasks[0].folders.failed.clone();
        tasks[1].folders.input = failed;
        let problems = check_set(&tasks);
        let lines: Vec<String> = problems.iter().map(TaskProblem::line).collect();
        assert_eq!(
            lines,
            [
                "Task one: it loops into itself.",
                "Task two: it loops into itself."
            ]
        );
    }

    #[test]
    fn a_name_used_twice_keeps_the_first() {
        let around = Surroundings::none(PathStyle::Unix);
        let first = shape("scans", "/srv/scans");
        let second = shape("scans", "/srv/other");
        let checked = check_tasks(&written(vec![first, second]), &around);
        assert_eq!(checked.tasks.len(), 1);
        assert_eq!(
            checked.problems[0].line(),
            "Task scans: its name is taken by another task."
        );
    }
}
