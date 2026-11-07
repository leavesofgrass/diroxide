//! A task in words (R16 5.3): one sentence generator for the Watched
//! folders list, the approval question, the log's first line and `diro
//! watch --list`, so a task reads the same everywhere. Every key reads as
//! a clause, and a key left at its default is silent where its default
//! goes without saying.
//!
//! "Scans. Watches D:\scans\in. When a file named like *.pdf arrives and
//! stops changing for 2 seconds, it runs the recipe pdf-to-text on it and
//! puts the result in D:\scans\out. It then moves the original to
//! D:\scans\processed; if the recipe fails, to D:\scans\failed. A taken
//! name gets a number. Nothing is overwritten or removed. One file at a
//! time."
//!
//! This is also what the owner hears before approving a task: it replaces
//! the colored check marks other programs show.

use std::time::Duration;

use diroxide_core::ConflictPolicy;

use super::task::{After, Announce, OnlineOnly, Task, TaskAction};

/// A duration in words: "half a second", "1 second", "2 seconds", "1
/// minute 30 seconds", "10 minutes", "1 hour", "1 hour 5 minutes".
pub fn duration_words(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        return if millis == 500 {
            "half a second".to_owned()
        } else {
            format!("{millis} milliseconds")
        };
    }
    let seconds = u64::try_from(millis / 1000).unwrap_or(u64::MAX);
    let unit = |n: u64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    if seconds < 60 {
        return unit(seconds, "second");
    }
    if seconds < 3600 {
        let (m, s) = (seconds / 60, seconds % 60);
        return if s == 0 {
            unit(m, "minute")
        } else {
            format!("{} {}", unit(m, "minute"), unit(s, "second"))
        };
    }
    let (h, m) = (seconds / 3600, (seconds % 3600) / 60);
    if m == 0 {
        unit(h, "hour")
    } else {
        format!("{} {}", unit(h, "hour"), unit(m, "minute"))
    }
}

/// A list read aloud: "a", "a or b", "a, b or c".
fn or_list(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}

/// What the action is called when it fails: "the recipe", "the copy".
fn action_noun(action: &TaskAction) -> &'static str {
    match action {
        TaskAction::Convert { .. } => "the recipe",
        TaskAction::Move => "the move",
        TaskAction::Copy => "the copy",
        TaskAction::Script { .. } => "the script",
        TaskAction::Plugin { .. } => "the command",
        TaskAction::Tell => "it",
    }
}

/// The task read as one sentence ("Say rule"). See the module
/// documentation for the example it is tested against.
pub fn sentence(task: &Task) -> String {
    let folders = &task.folders;
    let mut out = vec![
        format!("{}.", task.title()),
        format!("Watches {}.", folders.input),
    ];
    let named =
        (!task.matches.is_empty()).then(|| format!("named like {}", or_list(&task.matches)));
    let which = match (&task.filter, named) {
        (Some(filter), Some(named)) => {
            format!("a file that passes the filter {filter} and is {named}")
        }
        (Some(filter), None) => format!("a file that passes the filter {filter}"),
        (None, Some(named)) => format!("a file {named}"),
        (None, None) => "any file".to_owned(),
    };
    let does = match &task.action {
        TaskAction::Convert { recipe } => format!(
            "it runs the recipe {recipe} on it and puts the result in {}",
            folders.out
        ),
        TaskAction::Move => format!(
            "it moves it to {}",
            folders
                .to
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default()
        ),
        TaskAction::Copy => format!(
            "it copies it to {}",
            folders
                .to
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default()
        ),
        TaskAction::Script { id } => format!(
            "it runs the script command {id} on it, with {} as its other folder",
            folders.out
        ),
        TaskAction::Plugin { id } => format!(
            "it runs the plugin command {id} on it, with {} as its other folder",
            folders.out
        ),
        TaskAction::Tell => "it says so and changes nothing".to_owned(),
    };
    out.push(format!(
        "When {which} arrives and stops changing for {}, {does}.",
        duration_words(task.settle)
    ));
    let noun = action_noun(&task.action);
    match (&task.action, &task.after) {
        (TaskAction::Tell, _) => {}
        (TaskAction::Move, _) => {
            out.push(format!("If {noun} fails, it goes to {}.", folders.failed));
        }
        (_, After::Move) => out.push(format!(
            "It then moves the original to {}; if {noun} fails, to {}.",
            folders.processed, folders.failed
        )),
        (_, After::Folder(folder)) => out.push(format!(
            "It then moves the original to {folder}; if {noun} fails, to {}.",
            folders.failed
        )),
        (_, After::Leave) => out.push(format!(
            "It leaves the original where it is, even if {noun} fails; it is handled once."
        )),
    }
    if task.action != TaskAction::Tell {
        out.push(
            match task.conflicts {
                ConflictPolicy::Skip => "A taken name is skipped.",
                ConflictPolicy::Newer => {
                    "An output that is up to date is skipped; another taken name gets a number."
                }
                _ => "A taken name gets a number.",
            }
            .to_owned(),
        );
        out.push("Nothing is overwritten or removed.".to_owned());
    }
    if let Some(limit) = task.timeout {
        out.push(format!(
            "Each file may take up to {}.",
            duration_words(limit)
        ));
    }
    if task.online_only == OnlineOnly::Fetch {
        out.push("A file only online is fetched first.".to_owned());
    }
    match task.announce {
        Announce::Summary => {}
        Announce::Each => out.push("It says each file.".to_owned()),
        Announce::Failures => out.push("It says only failures.".to_owned()),
        Announce::Off => out.push("It says nothing; the log keeps it all.".to_owned()),
    }
    out.push(if task.parallel <= 1 {
        "One file at a time.".to_owned()
    } else {
        format!("Up to {} files at a time.", task.parallel)
    });
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::refuse::Surroundings;
    use crate::hot::task::TaskShape;
    use diroxide_core::place::PathStyle;

    fn scans() -> TaskShape {
        let text = r#"
name = "scans"
about = "Makes a text file from each scanned PDF."
kind = "convert"
recipe = "pdf-to-text"

[trigger]
kind = "folder"
home = 'D:\scans'
match = ["*.pdf"]
settle = "2s"
catch-up = "ask"

[run]
after = "move"
conflicts = "keep-both"
parallel = 1
retries = 3
online-only = "skip"

[say]
announce = "summary"
"#;
        toml::from_str(text).unwrap()
    }

    fn task(shape: &TaskShape) -> Task {
        Task::from_shape(shape, &Surroundings::none(PathStyle::Windows)).unwrap()
    }

    #[test]
    fn the_scans_task_reads_as_r16s_sentence() {
        assert_eq!(
            sentence(&task(&scans())),
            "Scans. Watches D:\\scans\\in. When a file named like *.pdf arrives and stops \
             changing for 2 seconds, it runs the recipe pdf-to-text on it and puts the result \
             in D:\\scans\\out. It then moves the original to D:\\scans\\processed; if the \
             recipe fails, to D:\\scans\\failed. A taken name gets a number. Nothing is \
             overwritten or removed. One file at a time."
        );
    }

    #[test]
    fn every_key_reads_as_a_clause() {
        let mut downloads = scans();
        downloads.name = "downloads-sort".to_owned();
        downloads.kind = "move".to_owned();
        downloads.recipe = None;
        downloads.to = Some("D:\\software".to_owned());
        downloads.trigger.home = "D:\\Downloads-hot".to_owned();
        downloads.trigger.matches = vec!["*.iso".into(), "*.zip".into(), "*.7z".into()];
        downloads.trigger.settle = "500ms".to_owned();
        downloads.say.announce = "failures".to_owned();
        assert_eq!(
            sentence(&task(&downloads)),
            "Downloads sort. Watches D:\\Downloads-hot\\in. When a file named like *.iso, *.zip \
             or *.7z arrives and stops changing for half a second, it moves it to D:\\software. \
             If the move fails, it goes to D:\\Downloads-hot\\failed. A taken name gets a \
             number. Nothing is overwritten or removed. It says only failures. One file at a \
             time."
        );
        let mut tell = scans();
        tell.kind = "tell".to_owned();
        tell.recipe = None;
        tell.trigger.matches.clear();
        tell.say.announce = "each".to_owned();
        assert_eq!(
            sentence(&task(&tell)),
            "Scans. Watches D:\\scans\\in. When any file arrives and stops changing for 2 \
             seconds, it says so and changes nothing. It says each file. One file at a time."
        );
        let mut leave = scans();
        leave.run.after = "leave".to_owned();
        leave.run.conflicts = "newer".to_owned();
        leave.run.timeout = Some("90s".to_owned());
        leave.run.parallel = 2;
        leave.run.online_only = "fetch".to_owned();
        leave.trigger.filter = Some("size over 100 KB".to_owned());
        leave.say.announce = "off".to_owned();
        assert_eq!(
            sentence(&task(&leave)),
            "Scans. Watches D:\\scans\\in. When a file that passes the filter size over 100 KB \
             and is named like *.pdf arrives and stops changing for 2 seconds, it runs the \
             recipe pdf-to-text on it and puts the result in D:\\scans\\out. It leaves the \
             original where it is, even if the recipe fails; it is handled once. An output that \
             is up to date is skipped; another taken name gets a number. Nothing is \
             overwritten or removed. Each file may take up to 1 minute 30 seconds. A file only \
             online is fetched first. It says nothing; the log keeps it all. Up to 2 files at a \
             time."
        );
    }

    #[test]
    fn durations_read_as_words() {
        for (millis, words) in [
            (500, "half a second"),
            (750, "750 milliseconds"),
            (1000, "1 second"),
            (2000, "2 seconds"),
            (60_000, "1 minute"),
            (90_000, "1 minute 30 seconds"),
            (600_000, "10 minutes"),
            (3_600_000, "1 hour"),
            (3_900_000, "1 hour 5 minutes"),
        ] {
            assert_eq!(duration_words(Duration::from_millis(millis)), words);
        }
    }
}
