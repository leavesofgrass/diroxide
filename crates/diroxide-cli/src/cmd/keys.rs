//! `diro keys check [FILE]`, `print`, `preset`, `test` and `probe`: the
//! owner's keymap from the command line (answer 34), so a `keymap.toml`
//! can live in dotfiles and travel between machines.
//!
//! - `check` with a FILE says each problem in that keymap file in words,
//!   one a line, meaning first: unknown commands and keys, Insert, keys a
//!   terminal cannot send, keys a screen reader owns, a key given twice.
//!   With no FILE it checks every built-in preset first ("Preset Total
//!   Commander: OK."), then the config folder's `keymap.toml`.
//! - `print` writes the keys that apply, both columns, as `keymap.toml`
//!   text: the defaults, the chosen preset, then the owner's lines, every
//!   command with its keys or "none"; read back, it gives the same keys,
//!   and `check` finds it clean.
//! - `preset` lists the presets, the one in use first said so; `preset
//!   NAME` applies one: it sets `preset` in `config.toml` and clears
//!   nothing, so the owner's lines stay on top.
//! - `test` says, for each key pressed, what diroxide would run and what
//!   the terminal sent, until Escape or ten seconds without a key (the key
//!   probe of R19 spike one, for the owner's terminals).
//! - `probe` asks for each key of the key-delivery matrix in turn and
//!   prints what arrived for each in the matrix's words (`ctrl+v = kept`),
//!   the column the owner sends back for `terminal-delivery.toml`.
//!
//! Exit codes: 0 clean, printed or tested, 1 problems found, 2 wrong use, a
//! file that cannot be read, or `test` without a terminal (R14's table).

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use diroxide_app::keymap::{self, Platform, presets};
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
    /// Say what each key sends and runs.
    Test,
    /// Ask for each key of the delivery matrix and say what arrived.
    Probe,
    /// List the presets, or apply the one named.
    Preset(Option<String>),
    /// Print the help.
    Help,
}

/// The help for `diro keys`, one line each, meaning first.
pub fn help_text() -> String {
    [
        "diro keys: check or print the keys, as keymap.toml holds them.",
        "",
        "Usage:",
        "diro keys check [FILE]   say each problem in FILE, or check the presets and keymap.toml",
        "diro keys print          write the keys that apply, both columns",
        "diro keys preset [NAME]  list the presets, or apply NAME; your own keys stay on top",
        "diro keys test           say what each key sends and runs; Escape ends it",
        "diro keys probe          ask for each key in turn and print what arrived",
        "",
        "Exit codes:",
        "0  clean, printed, applied or tested",
        "1  problems found; each is a line",
        "2  wrong use, the file cannot be read, or no terminal for test or probe",
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
        [test] if test == "test" => Ok(KeysArgs::Test),
        [test, ..] if test == "test" => Err(CliError::TooMany("keys test".to_owned())),
        [probe] if probe == "probe" => Ok(KeysArgs::Probe),
        [preset] if preset == "preset" => Ok(KeysArgs::Preset(None)),
        [preset, name] if preset == "preset" => Ok(KeysArgs::Preset(Some(name.clone()))),
        [preset, _, ..] if preset == "preset" => Err(CliError::TooMany("keys preset".to_owned())),
        [probe, ..] if probe == "probe" => Err(CliError::TooMany("keys probe".to_owned())),
        [other, ..] => Err(CliError::UnknownOption(other.clone())),
    }
}

fn failed(e: std::io::Error) -> CliError {
    CliError::Failed(format!("the output could not be written: {e}"))
}

/// `diro keys check` with no file: every preset, one line each when it is
/// clean ("Preset Total Commander: OK.") or its problems after it, then
/// the config folder's `keymap.toml` when there is one.
fn check_all(paths: Option<&Paths>, out: &mut dyn Write) -> Result<i32, CliError> {
    let platform = Platform::current();
    let mut code = 0;
    for preset in presets::presets() {
        let problems = presets::check(preset);
        if problems.is_empty() {
            writeln!(out, "Preset {}: OK.", preset.name).map_err(failed)?;
        } else {
            code = 1;
            writeln!(
                out,
                "Preset {}: {}.",
                preset.name,
                count(problems.len() as u64, "problem", "problems")
            )
            .map_err(failed)?;
            for problem in &problems {
                writeln!(out, "{problem}").map_err(failed)?;
            }
        }
    }
    let Some(paths) = paths else {
        writeln!(
            out,
            "No config folder: only the presets were checked. Name a file: diro keys check FILE."
        )
        .map_err(failed)?;
        return Ok(code);
    };
    let config = diroxide_config::load(paths);
    if let Some(said) = presets::unknown_preset(&config.settings.preset) {
        code = 1;
        writeln!(out, "{said}").map_err(failed)?;
    }
    let path = paths.config_file(Paths::KEYMAP);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            writeln!(
                out,
                "No keymap.toml in the config folder: the preset's keys apply as they are."
            )
            .map_err(failed)?;
            return Ok(code);
        }
        Err(error) => {
            writeln!(out, "Could not read {}: {error}.", path.display()).map_err(failed)?;
            return Ok(2);
        }
    };
    let shown = path.display().to_string();
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
    } else {
        code = 1;
        writeln!(
            out,
            "{} in {shown}.",
            count(problems.len() as u64, "problem", "problems")
        )
        .map_err(failed)?;
    }
    Ok(code)
}

/// `diro keys preset [NAME]`: the list, or applying one. Applying sets
/// `preset` in `config.toml`, and `single_letters` when the preset's
/// letters run commands (or after such a preset), as the app does.
fn run_preset(
    name: Option<&str>,
    paths: Option<&Paths>,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let config = paths.map(diroxide_config::load).unwrap_or_default();
    let current = presets::chosen(&config.settings.preset);
    let Some(name) = name else {
        writeln!(
            out,
            "{} presets, the one in use first:",
            presets::presets().len()
        )
        .map_err(failed)?;
        writeln!(out, "{}, in use: {}", current.id, current.description).map_err(failed)?;
        for preset in presets::presets().iter().filter(|p| p.id != current.id) {
            writeln!(out, "{}: {}", preset.id, preset.description).map_err(failed)?;
        }
        writeln!(out, "Apply one with diro keys preset NAME.").map_err(failed)?;
        return Ok(0);
    };
    let Some(preset) = presets::preset(name) else {
        let ids: Vec<&str> = presets::presets().iter().map(|p| p.id.as_str()).collect();
        writeln!(
            out,
            "No preset is named {}. The presets: {}.",
            name.trim(),
            ids.join(", ")
        )
        .map_err(failed)?;
        return Ok(2);
    };
    let Some(paths) = paths else {
        writeln!(
            out,
            "Could not apply {}: the config folder is not known.",
            preset.name
        )
        .map_err(failed)?;
        return Ok(2);
    };
    let letters = match presets::save_choice(paths, preset, config.settings.single_letters) {
        Ok(letters) => letters,
        Err(said) => {
            writeln!(out, "{said}").map_err(failed)?;
            return Ok(2);
        }
    };
    let mine = config
        .keymap
        .overrides
        .iter()
        .filter(|o| diroxide_app::commands::info(&o.command).is_some())
        .count();
    let mut said = format!("{} applied.", preset.keys_name());
    if mine > 0 {
        said.push_str(&format!(
            " Your {} in keymap.toml still apply.",
            count(mine as u64, "line", "lines")
        ));
    }
    if let Some(letters) = letters {
        said.push_str(match letters {
            diroxide_config::SingleLetters::Commands => " Single letters now run commands.",
            _ => " Single letters jump to names again.",
        });
    }
    writeln!(out, "{said}").map_err(failed)?;
    Ok(0)
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
            let preset = presets::chosen(&config.settings.preset);
            let (terminal, window) = presets::keymaps(&config.keymap, preset, platform);
            write!(out, "{}", keymap::print_keys(&terminal, &window)).map_err(failed)?;
            Ok(0)
        }
        KeysArgs::Preset(name) => run_preset(name.as_deref(), paths, out),
        KeysArgs::Test => {
            if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
                writeln!(
                    out,
                    "Could not test the keys: this is not a terminal. Run diro keys test in one."
                )
                .map_err(failed)?;
                return Ok(2);
            }
            let config = paths.map(diroxide_config::load).unwrap_or_default();
            let preset = presets::chosen(&config.settings.preset);
            let (terminal, _) = presets::keymaps(&config.keymap, preset, platform);
            diroxide_tui::keytest::run(&terminal).map_err(|e| CliError::Failed(e.to_string()))?;
            Ok(0)
        }
        KeysArgs::Probe => {
            if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
                writeln!(
                    out,
                    "Could not probe the keys: this is not a terminal. Run diro keys probe in one."
                )
                .map_err(failed)?;
                return Ok(2);
            }
            let report = diroxide_tui::keytest::probe(platform)
                .map_err(|e| CliError::Failed(e.to_string()))?;
            writeln!(out, "{report}").map_err(failed)?;
            Ok(0)
        }
        KeysArgs::Check(file) => {
            let path = match (file, paths) {
                (Some(file), _) => PathBuf::from(file),
                (None, paths) => return check_all(paths, out),
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
        assert_eq!(parse(&args(&["test"])), Ok(KeysArgs::Test));
        assert_eq!(parse(&args(&["probe"])), Ok(KeysArgs::Probe));
        assert_eq!(parse(&args(&["preset"])), Ok(KeysArgs::Preset(None)));
        assert_eq!(
            parse(&args(&["preset", "worker"])),
            Ok(KeysArgs::Preset(Some("worker".into())))
        );
        assert_eq!(
            parse(&args(&["preset", "a", "b"])),
            Err(CliError::TooMany("keys preset".into()))
        );
        assert_eq!(
            parse(&args(&["test", "x"])),
            Err(CliError::TooMany("keys test".into()))
        );
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
    fn check_without_a_file_says_ok_for_each_preset_then_the_owners_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let (code, said) = run_text(&KeysArgs::Check(None), Some(&paths));
        assert_eq!(code, 0, "{said}");
        let lines: Vec<&str> = said.lines().collect();
        assert_eq!(lines[0], "Preset diroxide defaults: OK.");
        for preset in presets::presets() {
            assert!(
                said.contains(&format!("Preset {}: OK.\n", preset.name)),
                "{said}"
            );
        }
        assert_eq!(
            lines.last().unwrap(),
            &"No keymap.toml in the config folder: the preset's keys apply as they are."
        );
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(
            paths.config_file(Paths::KEYMAP),
            "[terminal.panel]\n\"file.copy\" = \"insert\"\n",
        )
        .unwrap();
        let (code, said) = run_text(&KeysArgs::Check(None), Some(&paths));
        assert_eq!(code, 1, "{said}");
        assert!(
            said.contains("Insert is the screen reader key, line 2.\n"),
            "{said}"
        );
        let (code, said) = run_text(&KeysArgs::Check(None), None);
        assert_eq!(code, 0);
        assert!(
            said.ends_with("Name a file: diro keys check FILE.\n"),
            "{said}"
        );
    }

    #[test]
    fn preset_lists_and_applies_and_print_shows_its_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let (code, said) = run_text(&KeysArgs::Preset(None), Some(&paths));
        assert_eq!(code, 0);
        let lines: Vec<&str> = said.lines().collect();
        assert!(
            lines[0].ends_with(" presets, the one in use first:"),
            "{said}"
        );
        assert!(lines[1].starts_with("diroxide, in use: "), "{said}");
        assert_eq!(
            lines.last().unwrap(),
            &"Apply one with diro keys preset NAME."
        );
        let (code, said) = run_text(&KeysArgs::Preset(Some("norton".into())), Some(&paths));
        assert_eq!(code, 2);
        assert!(
            said.starts_with("No preset is named norton. The presets: diroxide"),
            "{said}"
        );
        let (code, said) = run_text(&KeysArgs::Preset(Some("diroxide".into())), Some(&paths));
        assert_eq!(code, 0, "{said}");
        assert_eq!(said, "The diroxide default keys applied.\n");
        let settings = std::fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap();
        assert!(settings.contains("preset = \"diroxide\""), "{settings}");
        let (code, _) = run_text(&KeysArgs::Preset(Some("diroxide".into())), None);
        assert_eq!(code, 2, "no config folder");
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
