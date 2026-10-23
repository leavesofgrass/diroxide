//! A plugin's log as a list (R22 commit 40): newest line first, with the
//! last error before everything else, so the first row says what went
//! wrong. The log is the host's file, `<plugins state>/<id>/<id>.log`, one
//! line each "TIME SOURCE: TEXT"; the old file (`.log.1`) is read after
//! it.

use std::fs;
use std::path::{Path, PathBuf};

use diroxide_core::{ListModel, ListRow};

/// How many lines the list shows at most.
pub const MAX_LINES: usize = 500;

/// The log file of an item, in the plugins' state folder.
pub fn log_path(plugins_state: &Path, id: &str) -> PathBuf {
    plugins_state.join(id).join(format!("{id}.log"))
}

/// Whether a log line tells of a failure: the program's error output, an
/// error the plugin logged, or the host's failure words.
pub fn is_error(line: &str) -> bool {
    let Some((_, rest)) = line.split_once(' ') else {
        return false;
    };
    let Some((source, text)) = rest.split_once(": ") else {
        return false;
    };
    match source {
        "stderr" | "error" => true,
        "host" => {
            text.starts_with("Could not")
                || text.contains("stopped unexpectedly")
                || text.contains("timed out")
                || text.contains("did not start")
        }
        _ => false,
    }
}

/// Reads a log and its old file, newest line first, at most
/// [`MAX_LINES`].
pub fn read_newest_first(path: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let old = {
        let mut name = path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    };
    for file in [path, old.as_path()] {
        if let Ok(bytes) = fs::read(file) {
            let text = String::from_utf8_lossy(&bytes);
            lines.extend(text.lines().rev().map(str::to_owned));
        }
        if lines.len() >= MAX_LINES {
            break;
        }
    }
    lines.truncate(MAX_LINES);
    lines
}

/// The text of a line without its time: "stderr: cannot read a.txt".
fn without_time(line: &str) -> &str {
    line.split_once(' ').map_or(line, |(_, rest)| rest)
}

/// The log list of a plugin, titled with its name: the last error first
/// ("Last error: stderr: cannot read a.txt"), then every line newest
/// first, each without its time and with its time after it.
pub fn log_list(name: &str, newest_first: &[String]) -> ListModel {
    let mut rows = Vec::new();
    if let Some(error) = newest_first.iter().find(|l| is_error(l)) {
        rows.push(ListRow::new(
            "last-error",
            format!("Last error: {}", without_time(error)),
        ));
    }
    for (i, line) in newest_first.iter().enumerate() {
        let time = line.split_once(' ').map_or("", |(t, _)| t);
        let row = ListRow::new(i.to_string(), without_time(line).to_owned());
        rows.push(if time.is_empty() {
            row
        } else {
            row.with_details([time.to_owned()])
        });
    }
    let mut list = ListModel::new(format!("Log of {name}")).with_rows(rows);
    list.set_empty_text("The log is empty");
    list.set_help("Up and Down read a line, newest first, Escape closes.");
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_error_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_path(dir.path(), "line-counter");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "2026-10-23T10:00:00 host: started\n2026-10-23T10:00:01 stderr: cannot read a.txt\n2026-10-23T10:00:02 info: counted 3 files\n",
        )
        .unwrap();
        let lines = read_newest_first(&path);
        assert_eq!(lines.len(), 3);
        let list = log_list("Line counter", &lines);
        assert_eq!(list.title(), "Log of Line counter");
        assert_eq!(
            list.announce_focus(),
            "Last error: stderr: cannot read a.txt, 1 of 4"
        );
        assert_eq!(list.rows()[1].label, "info: counted 3 files");
        assert!(read_newest_first(&dir.path().join("none.log")).is_empty());
        assert!(!is_error("2026 host: started"));
        assert!(is_error(
            "2026 host: Could not start Line counter: no program."
        ));
    }
}
