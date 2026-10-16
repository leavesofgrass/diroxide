//! `diro completions SHELL`: print the shell's own completion script for
//! `diro`, so its subcommands, options and places complete at a prompt
//! (plan section 5.19). Bash, zsh, fish and PowerShell.
//!
//! The scripts come from clap's completion generator over [`command`], a
//! description of diro's command line that exists only for completion:
//! the parser in `lib.rs` stays the one that runs. A test keeps the two
//! in step: every subcommand `diro --help` names is in [`command`].

use std::io::Write;

use clap::{Arg, ArgAction, Command, ValueHint};
use clap_complete::{Shell as ClapShell, generate};

use crate::CliError;

/// A shell diro writes completions for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    /// Bash.
    Bash,
    /// zsh.
    Zsh,
    /// fish.
    Fish,
    /// PowerShell, 5.1 and 7.
    PowerShell,
}

impl Shell {
    /// Every shell, in the order the help names them.
    pub const ALL: [Shell; 4] = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell];

    /// The name typed after `diro completions`.
    pub const fn name(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::PowerShell => "powershell",
        }
    }

    /// The shell for a typed name, ignoring case; `pwsh` is PowerShell.
    pub fn from_name(name: &str) -> Option<Shell> {
        match name.to_ascii_lowercase().as_str() {
            "bash" => Some(Shell::Bash),
            "zsh" => Some(Shell::Zsh),
            "fish" => Some(Shell::Fish),
            "powershell" | "pwsh" => Some(Shell::PowerShell),
            _ => None,
        }
    }

    const fn generator(self) -> ClapShell {
        match self {
            Shell::Bash => ClapShell::Bash,
            Shell::Zsh => ClapShell::Zsh,
            Shell::Fish => ClapShell::Fish,
            Shell::PowerShell => ClapShell::PowerShell,
        }
    }
}

/// What `diro completions` was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionsArgs {
    /// Print the script for this shell.
    Script(Shell),
    /// Print the help for `diro completions`.
    Help,
}

/// The help for `diro completions`, one line each, meaning first.
pub fn help_text() -> String {
    [
        "diro completions: print the shell's completion script for diro.",
        "",
        "Usage:",
        "diro completions SHELL   SHELL is bash, zsh, fish or powershell",
        "",
        "Load it once in a session, or save it where the shell looks:",
        "bash:        diro completions bash > ~/.local/share/bash-completion/completions/diro",
        "zsh:         diro completions zsh > ~/.zfunc/_diro (with ~/.zfunc in fpath)",
        "fish:        diro completions fish > ~/.config/fish/completions/diro.fish",
        "PowerShell:  diro completions powershell | Out-String | Invoke-Expression",
        "",
        "Exit codes:",
        "0  printed",
        "2  wrong use: no shell, or a shell diro does not know",
    ]
    .join("\n")
}

/// Reads the arguments after `completions`.
pub fn parse(args: &[String]) -> Result<CompletionsArgs, CliError> {
    match args {
        [] => Err(CliError::Missing("completions".to_owned())),
        [flag] if flag == "--help" || flag == "-h" => Ok(CompletionsArgs::Help),
        [name] => Shell::from_name(name)
            .map(CompletionsArgs::Script)
            .ok_or_else(|| {
                CliError::Failed(format!(
                    "there are no completions for {name}; use bash, zsh, fish or powershell"
                ))
            }),
        [_, ..] => Err(CliError::TooMany("completions".to_owned())),
    }
}

/// Prints the script or the help.
pub fn run(args: CompletionsArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let failed =
        |e: std::io::Error| CliError::Failed(format!("the script could not be written: {e}"));
    match args {
        CompletionsArgs::Help => writeln!(out, "{}", help_text()).map_err(failed)?,
        CompletionsArgs::Script(shell) => {
            let mut script = Vec::new();
            generate(shell.generator(), &mut command(), "diro", &mut script);
            out.write_all(&script).map_err(failed)?;
        }
    }
    Ok(0)
}

fn flag(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .action(ArgAction::SetTrue)
        .help(help)
}

fn named(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name).required(true).help(help)
}

/// diro's command line as clap describes it, for completion only. Keep in
/// step with `parse` in `lib.rs` and the subcommands' own parsers: a new
/// subcommand or option goes here too.
pub fn command() -> Command {
    let no_help = |command: Command| command.disable_help_flag(true);
    Command::new("diro")
        .about("A two-panel file manager, keyboard first and accessible first.")
        .version(env!("CARGO_PKG_VERSION"))
        .disable_help_subcommand(true)
        .arg(flag("portable", "Keep the settings beside the program"))
        .arg(flag("no-plugins", "Start without plugins"))
        .arg(flag("tui", "Open in the terminal, even with a display"))
        .arg(flag("gui", "Open the window (it comes with alpha.6)"))
        .arg(flag(
            "inline",
            "Draw below the prompt, not on the whole screen",
        ))
        .arg(
            Arg::new("cwd-file")
                .long("cwd-file")
                .value_name("FILE")
                .value_hint(ValueHint::FilePath)
                .help("At quit, write the active folder to FILE for the shell"),
        )
        .arg(
            Arg::new("theme")
                .long("theme")
                .value_name("NAME")
                .help("Use this theme for this session"),
        )
        .arg(
            Arg::new("places")
                .num_args(0..=2)
                .value_hint(ValueHint::AnyPath)
                .help("Open the panels at PATH and OTHER"),
        )
        .subcommand(
            Command::new("ls")
                .about("List a place, one entry a line, the name first")
                .arg(flag(
                    "long",
                    "Add the exact size, times, attributes and owner",
                ))
                .arg(flag("json", "Write one JSON object a line"))
                .arg(flag("hidden", "List hidden entries too"))
                .arg(flag("lines", "With -, read one path a line"))
                .arg(flag("null", "With -, read NUL-separated paths").short('0'))
                .arg(
                    Arg::new("place")
                        .value_hint(ValueHint::AnyPath)
                        .help("The place to list, or - for paths on standard input"),
                ),
        )
        .subcommand(
            Command::new("completions")
                .about("Print the shell's completion script for diro")
                .arg(
                    Arg::new("shell")
                        .required(true)
                        .value_parser(Shell::ALL.map(Shell::name))
                        .help("The shell"),
                ),
        )
        .subcommand(
            Command::new("keys")
                .about("Check or print the keys, as keymap.toml holds them")
                .disable_help_subcommand(true)
                .subcommand(
                    Command::new("check")
                        .about("Say each problem in keymap.toml or FILE")
                        .arg(
                            Arg::new("file")
                                .value_hint(ValueHint::FilePath)
                                .help("The keymap file"),
                        ),
                )
                .subcommand(
                    Command::new("print").about("Print the current keys as keymap.toml text"),
                )
                .subcommand(
                    Command::new("test").about("Say what each key sends and runs; Escape ends it"),
                ),
        )
        .subcommand(
            Command::new("init")
                .about("Print the shell function that moves the shell where diro left it")
                .arg(
                    Arg::new("shell")
                        .required(true)
                        .value_parser(super::init::InitShell::ALL.map(super::init::InitShell::name))
                        .help("The shell"),
                ),
        )
        .subcommand(no_help(
            Command::new("search")
                .about("Print the places under ROOT whose names match PATTERN")
                .arg(named("root", "Where to start").value_hint(ValueHint::DirPath))
                .arg(named("pattern", "Names to match, globs joined by commas")),
        ))
        .subcommand(no_help(
            Command::new("serve")
                .about("Serve the app over JSON-RPC on standard input and output")
                .arg(flag("stdio", "Use standard input and output")),
        ))
        .subcommand(no_help(
            Command::new("plugin")
                .about("Plugin tooling")
                .disable_help_subcommand(true)
                .subcommand(no_help(Command::new("list").about("List the plugins")))
                .subcommand(no_help(
                    Command::new("new")
                        .about("Write a working skeleton of a new plugin")
                        .arg(named("name", "Its name"))
                        .arg(
                            Arg::new("language")
                                .long("language")
                                .value_parser(["python", "bash"])
                                .help("Its language"),
                        ),
                ))
                .subcommand(no_help(
                    Command::new("enable")
                        .about("Enable a plugin")
                        .arg(named("name", "The plugin")),
                ))
                .subcommand(no_help(
                    Command::new("disable")
                        .about("Disable a plugin")
                        .arg(named("name", "The plugin")),
                ))
                .subcommand(no_help(
                    Command::new("run")
                        .about("Run a plugin's command")
                        .arg(named("name", "The command")),
                ))
                .subcommand(no_help(
                    Command::new("log")
                        .about("Print a plugin's log")
                        .arg(named("name", "The plugin")),
                )),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_description_is_valid() {
        command().debug_assert();
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(
            parse(&args(&["bash"])),
            Ok(CompletionsArgs::Script(Shell::Bash))
        );
        assert_eq!(
            parse(&args(&["PWSH"])),
            Ok(CompletionsArgs::Script(Shell::PowerShell))
        );
        assert_eq!(parse(&args(&["--help"])), Ok(CompletionsArgs::Help));
        assert_eq!(
            parse(&args(&[])),
            Err(CliError::Missing("completions".into()))
        );
        assert_eq!(
            parse(&args(&["tcsh"])),
            Err(CliError::Failed(
                "there are no completions for tcsh; use bash, zsh, fish or powershell".into()
            ))
        );
        assert_eq!(
            parse(&args(&["bash", "zsh"])),
            Err(CliError::TooMany("completions".into()))
        );
    }

    #[test]
    fn every_shell_gets_a_script_that_knows_the_subcommands() {
        for shell in Shell::ALL {
            let mut out = Vec::new();
            assert_eq!(run(CompletionsArgs::Script(shell), &mut out), Ok(0));
            let script = String::from_utf8(out).unwrap();
            // fish writes long options as `-l hidden`, the others as
            // `--hidden`.
            for word in ["diro", "ls", "completions", "hidden", "--language"] {
                let word = if shell == Shell::Fish {
                    word.trim_start_matches('-')
                } else {
                    word
                };
                assert!(script.contains(word), "{} lacks {word}", shell.name());
            }
            // clap's fish and PowerShell generators do not offer a
            // positional argument's values, so `diro completions` completes
            // its shell names in bash and zsh only.
            if matches!(shell, Shell::Bash | Shell::Zsh) {
                assert!(script.contains("powershell"), "{}", shell.name());
            }
        }
    }

    #[test]
    fn every_subcommand_in_the_help_completes() {
        let command = command();
        for line in crate::help_text().lines() {
            let mut words = line.split_whitespace();
            if words.next() != Some("diro") {
                continue;
            }
            let Some(sub) = words.next() else { continue };
            if sub.starts_with(['-', '[']) {
                continue;
            }
            assert!(
                command.find_subcommand(sub).is_some(),
                "diro --help names {sub}, which completions do not know"
            );
        }
    }

    #[test]
    fn the_help_says_the_shells_first() {
        let help = help_text();
        assert!(diroxide_core::format::within_cells(
            help.lines().next().unwrap(),
            "diro completions: print the shell's"
        ));
        assert!(help.contains("Exit codes:"));
    }
}
