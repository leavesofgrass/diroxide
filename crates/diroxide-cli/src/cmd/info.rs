//! `diro info PATH...`: a file's details as plain lines, for scripts and
//! the Braille display (alpha.7, the owner's answer 121): the name and
//! its summary first ("photo.png: PNG, 4 by 3 pixels, 2.3 KB"), then each
//! detail label first, in the details list's order of facts. Several
//! paths are told apart by a blank line. The details are read as the
//! details list reads them, the tool-backed readers through the tools
//! registry; nothing is written.
//!
//! Exit codes: 0 every file was read, 1 a file could not be (said on its
//! own line, the others still told), 2 wrong use.

use std::io::Write;
use std::sync::Arc;

use diroxide_app::request::SharedTools;
use diroxide_core::Place;
use diroxide_vfs::Registry;

use crate::CliError;
use crate::open::current_place;

/// `diro info`'s arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfoArgs {
    /// The paths, as typed.
    pub args: Vec<String>,
    /// Print the help for `diro info` and nothing else.
    pub help: bool,
}

/// The help for `diro info`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro info: a file's details as plain lines, the name first.",
        "",
        "Usage:",
        "diro info PATH...          the name and summary, then each detail",
        "",
        "Exit codes:",
        "0  every file was read",
        "1  a file could not be read; its line says why",
        "2  wrong use",
    ]
    .join("\n")
}

/// Reads the arguments after `info`.
pub fn parse(args: &[String]) -> Result<InfoArgs, CliError> {
    let mut parsed = InfoArgs::default();
    let mut only_paths = false;
    for arg in args {
        match arg.as_str() {
            _ if only_paths => parsed.args.push(arg.clone()),
            "--" => only_paths = true,
            "--help" | "-h" => parsed.help = true,
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            path => parsed.args.push(path.to_owned()),
        }
    }
    if parsed.args.is_empty() && !parsed.help {
        return Err(CliError::Missing("info".to_owned()));
    }
    Ok(parsed)
}

/// Runs `diro info` with the tools registry's runner.
pub fn run(args: &InfoArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let tools = SharedTools(Arc::new(diroxide_tools::ToolRegistry::builtin()));
    run_with(args, &Registry::with_local(), &tools, out)
}

/// Runs `diro info` over a registry, the tool-backed readers through
/// `tools` (a test gives a runner that starts nothing).
pub fn run_with(
    args: &InfoArgs,
    registry: &Registry,
    tools: &SharedTools,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let write = |out: &mut dyn Write, line: &str| {
        writeln!(out, "{line}").map_err(|e| CliError::Failed(e.to_string()))
    };
    if args.help {
        write(out, &help_text())?;
        return Ok(0);
    }
    let here = current_place()?;
    let mut code = 0;
    for (i, typed) in args.args.iter().enumerate() {
        if i > 0 {
            write(out, "")?;
        }
        let place = match Place::resolve(typed, &here) {
            Ok(place) => place,
            Err(error) => {
                write(out, &format!("{typed}: cannot be read as a path: {error}"))?;
                code = 1;
                continue;
            }
        };
        match diroxide_app::metadata::info_lines(registry, &place, tools) {
            Ok(lines) => {
                for line in lines {
                    write(out, &line)?;
                }
            }
            Err(why) => {
                write(out, &format!("{typed}: could not be read: {why}"))?;
                code = 1;
            }
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_read() {
        let args = parse(&["a.txt".to_owned(), "--".to_owned(), "-b".to_owned()]).unwrap();
        assert_eq!(args.args, ["a.txt", "-b"]);
        assert!(matches!(parse(&[]), Err(CliError::Missing(_))));
        assert!(matches!(
            parse(&["--size".to_owned()]),
            Err(CliError::UnknownOption(_))
        ));
        assert!(parse(&["--help".to_owned()]).unwrap().help);
    }

    #[test]
    fn a_files_details_are_plain_lines_name_first() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, b"one\ntwo\nthree\n").unwrap();
        let args = InfoArgs {
            args: vec![
                file.display().to_string(),
                dir.path().join("gone.txt").display().to_string(),
            ],
            help: false,
        };
        let mut out = Vec::new();
        // A runner that finds no program, so no test starts one.
        let tools = SharedTools(Arc::new(diroxide_vfs::NoTools));
        let code = run_with(&args, &Registry::with_local(), &tools, &mut out).unwrap();
        assert_eq!(code, 1);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("notes.txt: "), "{text}");
        assert!(lines.contains(&"Name: notes.txt"), "{text}");
        assert!(lines.contains(&"Lines: 3"), "{text}");
        assert!(
            lines.iter().position(|l| l.starts_with("Name: "))
                < lines.iter().position(|l| l.starts_with("Path: ")),
            "{text}"
        );
        assert!(
            lines
                .last()
                .is_some_and(|l| l.starts_with(&format!("{}: could not be read", args.args[1]))),
            "{text}"
        );
    }
}
