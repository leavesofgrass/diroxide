//! `diro keys check [FILE]` and `diro keys print`: the owner's keymap from
//! the command line (answer 34), so a `keymap.toml` can live in dotfiles
//! and travel between machines.
//!
//! - `check` reads a keymap file (the config folder's `keymap.toml` when
//!   no file is named) and says each problem in words, one a line, meaning
//!   first: unknown commands and keys, Insert, keys a terminal cannot
//!   send, keys a screen reader owns, a key given twice.
//! - `print` writes the current keys of both columns as `keymap.toml`
//!   text, every command with its keys or "none"; read back, it gives the
//!   same keys, and `check` finds it clean.
//!
//! Exit codes: 0 clean or printed, 1 problems found, 2 wrong use or a file
//! that cannot be read (R14's table).

use std::io::Write;
use std::path::PathBuf;

use diroxide_app::keymap::{self, Platform};
use diroxide_config::{KeymapConfig, Paths};
use diroxide_core::format::count;

use crate::CliError;

/// What `diro keys` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeysArgs {
    /// Check a keymap file; the config folder's when `None`.
    Check(Option<String>),
    /// Print the current keys.
    Print,
    /// Print the help.
    Help,
}

/// The help for `diro keys`, one line each, meaning first.
pub fn help_text() -> String {
    [
        "diro keys: check or print the keys, as keymap.toml holds them.",
        "",
        "Usage:",
        "diro keys check [FILE]   say each problem in FILE, or in keymap.toml",
        "diro keys print          write the current keys of both columns",
        "",
        "Exit codes:",
        "0  clean, or printed",
        "1  problems found; each is a line",
        "2  wrong use, or the file cannot be read",
    ]
    .join("\n")
}

/// Reads the arguments after `keys`.
pub fn parse(args: &[String]) -> Result<KeysArgs, CliError> {
    match args {
        [] => Err(CliError::Missing("keys".to_owned())),
        [flag] if flag == "--help" || flag == "-h" => Ok(KeysArgs::Help),
        [check] if check == "check" => Ok(KeysArgs::Check(None)),
        [check, file] if check == "check" => Ok(KeysArgs::Check(Some(file.clone()))),
        [check, _, ..] if check == "check" => Err(CliError::TooMany("keys check".to_owned())),
        [print] if print == "print" => Ok(KeysArgs::Print),
        [print, ..] if print == "print" => Err(CliError::TooMany("keys print".to_owned())),
        [other, ..] => Err(CliError::UnknownOption(other.clone())),
    }
}

fn failed(e: std::io::Error) -> CliError {
    CliError::Failed(format!("the output could not be written: {e}"))
}

/// Runs `diro keys` with the config folder of `paths` (none: the
/// defaults, and `check` needs a file).
pub fn run(args: &KeysArgs, paths: Option<&Paths>, out: &mut dyn Write) -> Result<i32, CliError> {
    let platform = Platform::current();
    match args {
        KeysArgs::Help => {
            writeln!(out, "{}", help_text()).map_err(failed)?;
            Ok(0)
        }
        KeysArgs::Print => {
            let config = paths.map(diroxide_config::load).unwrap_or_default();
            let (terminal, window) = keymap::keymaps(&config.keymap, platform);
            write!(out, "{}", keymap::print_keys(&terminal, &window)).map_err(failed)?;
            Ok(0)
        }
        KeysArgs::Check(file) => {
            let path = match (file, paths) {
                (Some(file), _) => PathBuf::from(file),
                (None, Some(paths)) => paths.config_file(Paths::KEYMAP),
                (None, None) => {
                    writeln!(
                        out,
                        "Could not check the keys: the config folder is not known. Name the file: diro keys check FILE."
                    )
                    .map_err(failed)?;
                    return Ok(2);
                }
            };
            let shown = path.display().to_string();
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    let cause = match error.kind() {
                        std::io::ErrorKind::NotFound => "it does not exist".to_owned(),
                        std::io::ErrorKind::InvalidData => "it is not UTF-8 text".to_owned(),
                        _ => error.to_string(),
                    };
                    writeln!(out, "Could not read {shown}: {cause}.").map_err(failed)?;
                    return Ok(2);
                }
            };
            let config = match KeymapConfig::from_toml(&text) {
                Ok(config) => config,
                Err(error) => {
                    writeln!(out, "Could not check {shown}: {error}.").map_err(failed)?;
                    return Ok(2);
                }
            };
            let problems = keymap::check_keymap(&config, platform);
            for problem in &problems {
                writeln!(out, "{}", problem.text).map_err(failed)?;
            }
            if problems.is_empty() {
                writeln!(out, "No problems in {shown}.").map_err(failed)?;
                Ok(0)
            } else {
                writeln!(
                    out,
                    "{} in {shown}.",
                    count(problems.len() as u64, "problem", "problems")
                )
                .map_err(failed)?;
                Ok(1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(&args(&["check"])), Ok(KeysArgs::Check(None)));
        assert_eq!(
            parse(&args(&["check", "k.toml"])),
            Ok(KeysArgs::Check(Some("k.toml".into())))
        );
        assert_eq!(parse(&args(&["print"])), Ok(KeysArgs::Print));
        assert_eq!(parse(&args(&["-h"])), Ok(KeysArgs::Help));
        assert_eq!(parse(&args(&[])), Err(CliError::Missing("keys".into())));
        assert_eq!(
            parse(&args(&["sing"])),
            Err(CliError::UnknownOption("sing".into()))
        );
    }

    fn run_text(args: &KeysArgs, paths: Option<&Paths>) -> (i32, String) {
        let mut out = Vec::new();
        let code = run(args, paths, &mut out).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn print_reads_back_clean_and_a_bad_file_is_said_line_by_line() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let (code, printed) = run_text(&KeysArgs::Print, Some(&paths));
        assert_eq!(code, 0);
        assert!(printed.contains("\"file.copy\" = \"f5\""), "{printed}");
        let file = dir.path().join("printed.toml");
        std::fs::write(&file, &printed).unwrap();
        let (code, said) = run_text(
            &KeysArgs::Check(Some(file.display().to_string())),
            Some(&paths),
        );
        assert_eq!(code, 0, "{said}");
        assert!(said.starts_with("No problems in "), "{said}");
        let bad = dir.path().join("bad.toml");
        std::fs::write(
            &bad,
            "[terminal.panel]\n\"file.copy\" = \"insert\"\n\"no.such\" = \"f5\"\n",
        )
        .unwrap();
        let (code, said) = run_text(
            &KeysArgs::Check(Some(bad.display().to_string())),
            Some(&paths),
        );
        assert_eq!(code, 1, "{said}");
        let lines: Vec<&str> = said.lines().collect();
        assert!(
            lines.last().unwrap().starts_with("2 problems in "),
            "{said}"
        );
        for line in &lines {
            assert!(!line.is_empty());
        }
        let (code, said) = run_text(
            &KeysArgs::Check(Some(dir.path().join("none.toml").display().to_string())),
            Some(&paths),
        );
        assert_eq!(code, 2);
        assert!(said.ends_with("it does not exist.\n"), "{said}");
        std::fs::write(&bad, "[terminal.panel\n").unwrap();
        let (code, _) = run_text(
            &KeysArgs::Check(Some(bad.display().to_string())),
            Some(&paths),
        );
        assert_eq!(code, 2);
    }
}
