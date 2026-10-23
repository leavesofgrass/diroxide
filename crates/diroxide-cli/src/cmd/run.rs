//! `diro run COMMAND [NAME=VALUE...]`: one command, headless, in the
//! command text form the banks, `command/run` and `diro serve` share
//! (plan section 5.8; R22 commit 28). W4b owns this file and its lines in
//! `lib.rs`, `cmd/mod.rs` and the completions.
//!
//! - Each word after the command's id is one argument, `name=value`, as
//!   the shell split it; a value with spaces needs only the shell's
//!   quotes (`"dest=D:\My Backup"`). Place arguments (`dest`, `to`,
//!   `place` and the like) may be relative to the current folder; they
//!   become full places before anything runs.
//! - The app starts with the active panel in the current folder and the
//!   other at the command's `dest` (the orthodox rule: the destination is
//!   the other panel), or at `--other PLACE`, or also in the current
//!   folder; it runs the command as the keys would, and prints every
//!   sentence it says, one a line.
//! - A question it asks is answered No, unless `--yes` answers yes; an
//!   answer is said as it is given, and a No, or a dialog still open at
//!   the end, makes the exit code 1.
//!
//! Exit codes: 0 done; 1 failed, or a question was left open; 2 wrong use.

use std::io::Write;

use diroxide_app::AppEvent;
use diroxide_core::{Command, CommandId, Effect, MessageKind, Place};

use crate::CliError;
use crate::open::{OpenOptions, build, current_place, paths};

/// `diro run` as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunArgs {
    /// The command's id, then each `name=value`.
    pub words: Vec<String>,
    /// `--yes`: answer yes to the questions it asks.
    pub yes: bool,
    /// `--portable`: the settings beside the program.
    pub portable: bool,
    /// `--other PLACE`: the other panel's place.
    pub other: Option<String>,
    /// `--help`.
    pub help: bool,
}

/// The help for `diro run`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro run: one command, headless, in the command text form.",
        "",
        "diro run COMMAND [NAME=VALUE...]   run it in the current folder; say what it says",
        "diro run file.copy dest=D:\\backup  for example: copy the cursor's file to D:\\backup",
        "",
        "Options:",
        "--yes          answer yes to the questions it asks; No otherwise",
        "--other PLACE  the other panel's place; the current folder by default",
        "--portable     use the settings beside the program",
        "",
        "Exit codes: 0 done; 1 failed, or a question was left open; 2 wrong use.",
    ]
    .join("\n")
}

/// Reads the arguments after `diro run`.
pub fn parse(args: &[String]) -> Result<RunArgs, CliError> {
    let mut parsed = RunArgs {
        words: Vec::new(),
        yes: false,
        portable: false,
        other: None,
        help: false,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" if parsed.words.is_empty() => parsed.help = true,
            "--yes" | "-y" => parsed.yes = true,
            "--portable" => parsed.portable = true,
            "--other" => match rest.next() {
                Some(value) if !value.is_empty() => parsed.other = Some(value.clone()),
                _ => return Err(CliError::Missing("--other".to_owned())),
            },
            option if option.starts_with("--") && parsed.words.is_empty() => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            word => parsed.words.push(word.to_owned()),
        }
    }
    if parsed.words.is_empty() && !parsed.help {
        return Err(CliError::Missing("run".to_owned()));
    }
    Ok(parsed)
}

/// Text quoted for the command text form when it needs it: spaces or a
/// quotation mark, which is written twice inside.
fn quoted(value: &str) -> String {
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// The command for the words: the id, then each `name=value`, with place
/// arguments made full against `here`.
pub fn command_of(words: &[String], here: &Place) -> Result<Command, String> {
    let Some((id, arguments)) = words.split_first() else {
        return Err("No command: the text is empty".to_owned());
    };
    let mut text = id.clone();
    for argument in arguments {
        let Some((name, value)) = argument.split_once('=') else {
            return Command::parse_text(&format!("{id} {argument}")).map_err(|e| e.to_string());
        };
        let value = if Command::PLACE_ARGUMENTS.contains(&name) {
            Place::resolve(value, here)
                .map(|p| p.to_string())
                .unwrap_or_else(|_| value.to_owned())
        } else {
            value.to_owned()
        };
        text.push(' ');
        text.push_str(name);
        text.push('=');
        text.push_str(&quoted(&value));
    }
    Command::parse_text(&text).map_err(|e| e.to_string())
}

/// Runs `diro run` with the platform's folders in the current folder.
pub fn run(args: &RunArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_error)?;
        return Ok(0);
    }
    let here = current_place()?;
    run_in(args, paths(args.portable).as_ref(), &here, out)
}

fn write_error(e: std::io::Error) -> CliError {
    CliError::Failed(e.to_string())
}

/// Runs `diro run` with these folders, the current folder `here`.
pub fn run_in(
    args: &RunArgs,
    folders: Option<&diroxide_config::Paths>,
    here: &Place,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_error)?;
        return Ok(0);
    }
    let command = match command_of(&args.words, here) {
        Ok(command) => command,
        Err(sentence) => {
            writeln!(out, "{sentence}").map_err(write_error)?;
            return Ok(2);
        }
    };
    // The orthodox rule: the destination is the other panel, so a
    // command's dest argument opens it there unless --other says else.
    let mut places = vec![here.to_string()];
    if let Some(other) = &args.other {
        places.push(other.clone());
    } else if let Some(dest) = command.place("dest") {
        places.push(dest.to_string());
    }
    let options = OpenOptions {
        places,
        portable: args.portable,
        ..OpenOptions::default()
    };
    let (mut app, mut runtime) = build(&options, folders, here)?;
    app.set_screen_reader(true);
    let start = app.start();
    let _ = runtime.run_until_idle(&mut app, start);
    let mut failed = false;
    let mut say = |effects: Vec<Effect>, out: &mut dyn Write| -> Result<(), CliError> {
        for effect in effects {
            if let Effect::Announce(message) = effect {
                failed |= message.kind == MessageKind::Error;
                writeln!(out, "{}", message.text).map_err(write_error)?;
            }
        }
        Ok(())
    };
    let effects = app.handle(AppEvent::Command(command));
    let shown = runtime.run_until_idle(&mut app, effects);
    say(shown, out)?;
    // Each question is answered: yes with --yes, else No. A dialog
    // ignores answers for its first moments (the dialog_settle_ms
    // setting, so a key typed before it opened cannot answer it), so a
    // yes is tried again a tenth of a second later, for three seconds.
    let mut answered = None;
    let mut declined = false;
    for _ in 0..30 {
        let Some((id, dialog)) = app.dialog() else {
            break;
        };
        if answered != Some(id) {
            writeln!(
                out,
                "{}: answered {}.",
                dialog.title(),
                if args.yes { "yes, from --yes" } else { "no" }
            )
            .map_err(write_error)?;
            answered = Some(id);
            declined |= !args.yes;
        }
        let answer = if args.yes { "dialog.yes" } else { "dialog.no" };
        let effects = app.handle(AppEvent::Command(Command::new(CommandId::from_static(
            answer,
        ))));
        let shown = runtime.run_until_idle(&mut app, effects);
        say(shown, out)?;
        if !args.yes {
            break;
        }
        if app.dialog().map(|(open, _)| open) == Some(id) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    if let Some((_, dialog)) = app.dialog() {
        writeln!(
            out,
            "Left open: {}. diro serve can answer it.",
            dialog.title()
        )
        .map_err(write_error)?;
        return Ok(1);
    }
    Ok(if failed || declined { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_app::commands::CATALOG;
    use diroxide_core::ArgValue;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn here() -> Place {
        Place::parse(if cfg!(windows) { "D:\\work" } else { "/work" }).unwrap()
    }

    #[test]
    fn options_and_words_are_read() {
        let parsed = parse(&args(&["file.copy", "dest=x y", "--yes", "--other", "/b"])).unwrap();
        assert_eq!(parsed.words, ["file.copy", "dest=x y"]);
        assert!(parsed.yes);
        assert_eq!(parsed.other.as_deref(), Some("/b"));
        assert_eq!(parse(&args(&[])), Err(CliError::Missing("run".into())));
        assert_eq!(
            parse(&args(&["--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert!(parse(&args(&["--help"])).unwrap().help);
    }

    #[test]
    fn a_command_and_its_named_arguments_read() {
        let command = command_of(
            &args(&[
                "file.copy",
                "dest=backup copy",
                "count=3",
                "keep=true",
                "note=say \"hi\"",
            ]),
            &here(),
        )
        .unwrap();
        assert_eq!(command.id.as_str(), "file.copy");
        let dest = command.place("dest").unwrap();
        assert!(dest.to_string().ends_with("backup copy"), "{dest}");
        assert!(dest.is_within(&here()));
        assert_eq!(command.number("count"), Some(3));
        assert_eq!(command.flag("keep"), Some(true));
        assert_eq!(command.text("note"), Some("say \"hi\""));
        assert_eq!(
            command_of(&args(&["File.Copy"]), &here()).unwrap_err(),
            "Not a command id: File.Copy"
        );
        assert_eq!(
            command_of(&args(&["file.copy", "dest"]), &here()).unwrap_err(),
            "No value for dest: write dest=value"
        );
    }

    /// Every command of the catalog, with arguments of each kind, goes to
    /// text and back unchanged.
    #[test]
    fn every_catalog_command_round_trips_through_its_text() {
        let place = here().join("My Backup").unwrap();
        for info in &CATALOG {
            let bare = Command::new(info.id.clone());
            let text = bare.to_string();
            assert_eq!(text, info.id.as_str());
            assert_eq!(Command::parse_text(&text).unwrap(), bare, "{text}");
            let full = Command::new(info.id.clone())
                .with_arg("dest", ArgValue::Place(place.clone()))
                .with_arg("count", ArgValue::Number(12))
                .with_arg("force", ArgValue::Flag(false))
                .with_arg("name", ArgValue::Text("a \"quoted\" name".into()))
                .with_arg("pattern", ArgValue::Text("42".into()));
            let text = full.to_string();
            assert_eq!(Command::parse_text(&text).unwrap(), full, "{text}");
            assert!(text.starts_with(info.id.as_str()));
            let words: Vec<String> = std::iter::once(info.id.as_str().to_owned())
                .chain(full.args.iter().map(|(name, value)| match value {
                    ArgValue::Place(p) => format!("{name}={p}"),
                    ArgValue::Number(n) => format!("{name}={n}"),
                    ArgValue::Flag(b) => format!("{name}={b}"),
                    ArgValue::Text(t) => format!("{name}={t}"),
                    ArgValue::Places(_) => String::new(),
                }))
                .collect();
            let from_words = command_of(&words, &here()).unwrap();
            assert_eq!(from_words.id, full.id);
            assert_eq!(from_words.place("dest"), full.place("dest"));
            assert_eq!(from_words.text("name"), full.text("name"));
        }
        assert!(CATALOG.len() > 50);
    }

    #[test]
    fn a_command_runs_headless_and_says_what_it_says() {
        let root = tempfile::tempdir().unwrap();
        let config =
            diroxide_config::Paths::new(root.path().join("config"), root.path().join("state"));
        let work = root.path().join("work");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::write(work.join("a.txt"), b"a").unwrap();
        let here = Place::parse(&work.to_string_lossy()).unwrap();
        let mut out = Vec::new();
        let parsed = parse(&args(&["panel.reload"])).unwrap();
        let code = run_in(&parsed, Some(&config), &here, &mut out).unwrap();
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&out));
        let mut out = Vec::new();
        let parsed = parse(&args(&["no.such-command"])).unwrap();
        let code = run_in(&parsed, Some(&config), &here, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(code == 0 || code == 1, "{text}");
        let mut out = Vec::new();
        let parsed = parse(&args(&["Bad.Id"])).unwrap();
        assert_eq!(run_in(&parsed, Some(&config), &here, &mut out).unwrap(), 2);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Not a command id: Bad.Id\n"
        );
    }

    #[test]
    fn file_copy_runs_headless_with_the_answer_given() {
        let root = tempfile::tempdir().unwrap();
        let config =
            diroxide_config::Paths::new(root.path().join("config"), root.path().join("state"));
        let work = root.path().join("work");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(work.join("a.txt"), b"a").unwrap();
        let here = Place::parse(&work.to_string_lossy()).unwrap();
        let target = format!("dest={}", dest.to_string_lossy());
        let mut out = Vec::new();
        let parsed = parse(&args(&["file.copy", &target, "--yes"])).unwrap();
        let code = run_in(&parsed, Some(&config), &here, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains(
                ": answered yes, from --yes.
"
            ),
            "{text}"
        );
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"a", "{text}");
        // Without --yes the question is answered no, and nothing is copied.
        std::fs::write(work.join("b.txt"), b"b").unwrap();
        let other = format!("dest={}", root.path().join("elsewhere").to_string_lossy());
        std::fs::create_dir_all(root.path().join("elsewhere")).unwrap();
        let mut out = Vec::new();
        let parsed = parse(&args(&["file.copy", &other])).unwrap();
        let code = run_in(&parsed, Some(&config), &here, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, 1, "{text}");
        assert!(
            text.contains(
                ": answered no.
"
            ),
            "{text}"
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("elsewhere"))
                .unwrap()
                .count(),
            0
        );
    }
}
