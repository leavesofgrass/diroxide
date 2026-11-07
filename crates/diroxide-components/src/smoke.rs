//! The version check after a helper is installed (alpha 4, R28 1.3).
//!
//! When a helper's files are in place, each program it holds is run once
//! with its version arguments from the tools registry's table (never a
//! file, two seconds at most, in its own folder; [`version_line`]) and
//! its line is said the way Settings and `diro doctor` say it: "pandoc:
//! 3.13, from components". A program that does not start, or says nothing
//! a version can be read from, is said plainly: "Installed pandoc, but it
//! did not start." Nothing is removed either way: the files checked out
//! against their pins, and the owner may know why it does not start (a
//! missing system library, a security tool) better than diroxide does.
//!
//! The check runs only for the computer diroxide runs on, and only for a
//! helper; a plugin or a data pack is never run here.

use std::ffi::OsStr;
use std::path::Path;

use diroxide_tools::{
    BUILTIN, FoundBy, ToolReport, ToolSpec, ToolState, file_names, version_line, version_number,
};

use crate::component::{Component, Kind};

/// The line for a program that did not start: "Installed pandoc, but it
/// did not start."
pub fn did_not_start(name: &str) -> String {
    format!("Installed {name}, but it did not start.")
}

/// The line for a program that started and said its version: "pandoc:
/// 3.13, from components", or "pandoc: found, from components" when no
/// number could be read from its line.
pub fn started(spec: &ToolSpec, line: &str) -> String {
    ToolReport {
        id: spec.id.to_owned(),
        name: spec.name.to_owned(),
        state: ToolState::Found {
            version: version_number(line),
            by: FoundBy::Component,
        },
    }
    .line()
}

/// Runs the version command of each program `component` holds, in its
/// folder under `components_dir`, for the computer `target` names, and
/// gives one line a tool, in the order of its `tools` list. A component
/// that is not a helper, or names no tool, gives none; so does a tool the
/// registry's table does not know or that has no version command.
pub fn check_installed(component: &Component, components_dir: &Path, target: &str) -> Vec<String> {
    if component.kind != Kind::Helper {
        return Vec::new();
    }
    let programs = component.programs_in(components_dir, target);
    component
        .tools
        .iter()
        .filter_map(|id| BUILTIN.iter().find(|spec| spec.id == id.as_str()))
        .filter(|spec| !spec.version_args.is_empty())
        .map(|spec| {
            let line = programs
                .iter()
                .find(|path| is_program_of(path, spec) && path.is_file())
                .and_then(|path| version_line(path, spec.version_args))
                .filter(|line| spec.must_say.is_none_or(|words| line.contains(words)));
            match line {
                Some(line) => started(spec, &line),
                None => did_not_start(spec.name),
            }
        })
        .collect()
}

/// Whether the file at `path` is one of `spec`'s programs by its name:
/// `pandoc.exe` or `pandoc` for pandoc on Windows, `pandoc` elsewhere.
fn is_program_of(path: &Path, spec: &ToolSpec) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    spec.programs.iter().any(|program| {
        file_names(program).iter().any(|candidate| {
            if cfg!(windows) {
                candidate.eq_ignore_ascii_case(name)
            } else {
                candidate == name
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pin::FilePin;
    use diroxide_core::format::within_cells;
    use std::path::PathBuf;

    fn spec(id: &str) -> &'static ToolSpec {
        BUILTIN.iter().find(|s| s.id == id).unwrap()
    }

    /// The two lines, at 40 cells with the tool first.
    #[test]
    fn the_lines_say_the_tool_first() {
        let ok = started(spec("pandoc"), "pandoc 3.13");
        assert_eq!(ok, "pandoc: 3.13, from components");
        assert!(within_cells(&ok, "from components"));
        let ff = started(
            spec("ffmpeg"),
            "ffmpeg version n9.0.2-39-g7adfbe40dc-20261031",
        );
        assert_eq!(ff, "FFmpeg: 9.0.2, from components");
        let bad = did_not_start("pandoc");
        assert_eq!(bad, "Installed pandoc, but it did not start.");
        assert!(within_cells(&bad, "did not start"));
        assert_eq!(bad.chars().count(), 39);
        let none = started(spec("pandoc"), "something else");
        assert_eq!(none, "pandoc: found, from components");
    }

    /// Nothing is run for a plugin or a data pack, and a helper whose
    /// program is not there did not start; no program runs in this test.
    #[test]
    fn only_a_helper_that_is_there_is_run() {
        let tmp = tempfile::tempdir().unwrap();
        let mut pin = FilePin::new("pandoc.zip", 1, "0".repeat(64));
        pin.exec = Some(
            if cfg!(windows) {
                "pandoc.exe"
            } else {
                "pandoc"
            }
            .into(),
        );
        let mut c = Component::new("pandoc", "pandoc", vec![pin]);
        c.tools = vec!["pandoc".into(), "no-such-tool".into()];
        assert!(
            check_installed(&c, tmp.path(), "any").is_empty(),
            "a data pack"
        );
        c.kind = Kind::Plugin;
        assert!(
            check_installed(&c, tmp.path(), "any").is_empty(),
            "a plugin"
        );
        c.kind = Kind::Helper;
        assert_eq!(
            check_installed(&c, tmp.path(), "any"),
            ["Installed pandoc, but it did not start."]
        );
        c.tools.clear();
        assert!(check_installed(&c, tmp.path(), "any").is_empty());
    }

    #[test]
    fn a_program_is_known_by_its_file_name() {
        let ffprobe = PathBuf::from("bin").join(if cfg!(windows) {
            "FFPROBE.EXE"
        } else {
            "ffprobe"
        });
        assert!(is_program_of(&ffprobe, spec("ffprobe")));
        assert!(!is_program_of(&ffprobe, spec("ffmpeg")));
        assert!(is_program_of(&PathBuf::from("7zz"), spec("7z")));
    }
}
