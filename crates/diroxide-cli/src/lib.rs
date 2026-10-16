//! The command line behind the `diro` binary.
//!
//! - `diro [PATH] [OTHER]`: open the panels at PATH and OTHER.
//! - `diro serve --stdio`: the app core over JSON-RPC, the protocol the
//!   plugins speak, for tests, scripts and other programs.
//! - `diro search ROOT PATTERN`: search names, printing one place a line.
//! - `diro ls [PLACE]` and `diro ls -`: list a place, or the paths on
//!   standard input, one entry a line, the name first.
//! - `diro completions SHELL`: the completion script for bash, zsh, fish
//!   or PowerShell.
//! - `diro plugin list|new|enable|disable|run|log`: plugin tooling.
//! - `diro --version`, `diro --help`.
//!
//! Every argument must be valid Unicode: a name that is not is refused
//! with a message, never changed silently.
//!
//! Owners after Phase 0: W1d owns this file, `open.rs` and `serve.rs`;
//! W1e owns `cmd/` (W3b owns `cmd/plugin.rs`).

pub mod cmd;
pub mod open;
pub mod serve;

use std::ffi::OsString;
use std::io::{BufRead, Write};

use diroxide_core::{Explain, Message};
use thiserror::Error;

/// What was asked for on the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Open the panels.
    Open {
        /// Up to two places, as typed.
        places: Vec<String>,
        /// Keep the configuration beside the program.
        portable: bool,
        /// Start without plugins.
        no_plugins: bool,
    },
    /// Serve the app core over JSON-RPC.
    Serve,
    /// Search names.
    Search {
        /// Where to start.
        root: String,
        /// The name pattern.
        pattern: String,
    },
    /// Plugin tooling.
    Plugin(cmd::plugin::PluginCommand),
    /// List a place, or the paths on standard input.
    Ls(cmd::ls::LsArgs),
    /// Print a shell's completion script.
    Completions(cmd::completions::CompletionsArgs),
    /// Print the version.
    Version,
    /// Print the help.
    Help,
}

/// Why the command line could not be used.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CliError {
    /// An argument is not valid Unicode.
    #[error("the argument \"{0}\" is not valid Unicode")]
    NotUnicode(String),
    /// An option is not known.
    #[error("the option {0} is not known")]
    UnknownOption(String),
    /// A command needs an argument it did not get.
    #[error("{0} needs more arguments")]
    Missing(String),
    /// Too many arguments.
    #[error("there are too many arguments after {0}")]
    TooMany(String),
    /// Something went wrong while running, already in words.
    #[error("{0}")]
    Failed(String),
}

impl Explain for CliError {
    fn next_step(&self) -> String {
        match self {
            CliError::NotUnicode(_) => "Rename it, or open its folder and move to it.".to_owned(),
            CliError::Failed(_) => String::new(),
            _ => "Run diro --help for the usage.".to_owned(),
        }
    }
}

/// The version text, with the copyright line the plan puts here (section 1):
/// the one place besides NOTICE and the README that names the owner.
pub fn version_text() -> String {
    format!(
        "diro {}\nDirectory Oxide (diroxide), a two-panel file manager.\nCopyright (c) 2026 Jon Pielaet. License: Apache-2.0 OR MIT.",
        env!("CARGO_PKG_VERSION")
    )
}

/// The help text: one usage per line, meaning first.
pub fn help_text() -> String {
    [
        "diro: a two-panel file manager, keyboard first and accessible first.",
        "",
        "Usage:",
        "diro [PATH] [OTHER]       open the panels at PATH and OTHER",
        "diro serve --stdio        serve the app over JSON-RPC on standard input and output",
        "diro search ROOT PATTERN  print the places under ROOT whose names match PATTERN",
        "diro ls [PLACE]           list a folder, one entry a line, the name first",
        "diro ls -                 list the NUL-separated paths on standard input",
        "diro completions SHELL    print the completion script for bash, zsh, fish or powershell",
        "diro plugin list          list the plugins",
        "diro --version            print the version",
        "diro --help               print this help",
        "",
        "Options for opening:",
        "--portable                keep the settings beside the program",
        "--no-plugins              start without plugins",
        "",
        "Exit codes:",
        "0  done",
        "1  failed, or nothing found",
        "2  wrong use, such as an unknown option",
        "3  partly done: some items failed; the output says which",
        "",
        "diro ls --help gives the options of diro ls.",
    ]
    .join("\n")
}

/// Reads the arguments after the program's name.
pub fn parse(args: Vec<OsString>) -> Result<Invocation, CliError> {
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|raw| CliError::NotUnicode(raw.to_string_lossy().into_owned()))
        })
        .collect::<Result<_, _>>()?;
    let Some(first) = args.first() else {
        return Ok(Invocation::Open {
            places: Vec::new(),
            portable: false,
            no_plugins: false,
        });
    };
    match first.as_str() {
        "--version" | "-V" => Ok(Invocation::Version),
        "--help" | "-h" => Ok(Invocation::Help),
        "serve" => match args.get(1).map(String::as_str) {
            Some("--stdio") if args.len() == 2 => Ok(Invocation::Serve),
            Some("--stdio") => Err(CliError::TooMany("serve --stdio".to_owned())),
            Some(other) => Err(CliError::UnknownOption(other.to_owned())),
            None => Err(CliError::Missing("serve".to_owned())),
        },
        "search" => match &args[1..] {
            [root, pattern] => Ok(Invocation::Search {
                root: root.clone(),
                pattern: pattern.clone(),
            }),
            [_, _, ..] => Err(CliError::TooMany("search".to_owned())),
            _ => Err(CliError::Missing("search".to_owned())),
        },
        "plugin" => cmd::plugin::parse(&args[1..]).map(Invocation::Plugin),
        "ls" => cmd::ls::parse(&args[1..]).map(Invocation::Ls),
        "completions" => cmd::completions::parse(&args[1..]).map(Invocation::Completions),
        _ => {
            let mut places = Vec::new();
            let mut portable = false;
            let mut no_plugins = false;
            let mut only_places = false;
            for arg in &args {
                match arg.as_str() {
                    "--" if !only_places => only_places = true,
                    "--portable" if !only_places => portable = true,
                    "--no-plugins" if !only_places => no_plugins = true,
                    option if option.starts_with("--") && !only_places => {
                        return Err(CliError::UnknownOption(option.to_owned()));
                    }
                    place => places.push(place.to_owned()),
                }
            }
            if places.len() > 2 {
                return Err(CliError::TooMany(places[1].clone()));
            }
            Ok(Invocation::Open {
                places,
                portable,
                no_plugins,
            })
        }
    }
}

/// Runs an invocation, writing to `out` and reading `input` (for `serve`).
/// Returns the exit code.
pub fn run(
    invocation: Invocation,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let write_error = |e: std::io::Error| CliError::Failed(e.to_string());
    match invocation {
        Invocation::Version => {
            writeln!(out, "{}", version_text()).map_err(write_error)?;
            Ok(0)
        }
        Invocation::Help => {
            writeln!(out, "{}", help_text()).map_err(write_error)?;
            Ok(0)
        }
        Invocation::Open { places, .. } => open::run(&places, out),
        Invocation::Serve => serve::run(input, out),
        Invocation::Search { root, pattern } => cmd::search::run(&root, &pattern, out),
        Invocation::Plugin(command) => cmd::plugin::run(&command, out),
        Invocation::Ls(args) => cmd::ls::run(&args, input, out),
        Invocation::Completions(args) => cmd::completions::run(args, out),
    }
}

/// The whole program: parse, run, and turn errors into the house style on
/// `err`. Returns the exit code: 0 for success, 1 for failure, 2 for a
/// usage mistake, 3 when a subcommand was partly done.
pub fn main_with(
    args: Vec<OsString>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let invocation = match parse(args) {
        Ok(invocation) => invocation,
        Err(error) => {
            let _ = writeln!(err, "{}", Message::from_error("start", "diro", &error));
            return 2;
        }
    };
    match run(invocation, input, out) {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(err, "{}", Message::from_error("finish", "", &error));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(args(&["--version"])), Ok(Invocation::Version));
        assert_eq!(parse(args(&["serve", "--stdio"])), Ok(Invocation::Serve));
        assert_eq!(
            parse(args(&["--portable", "a", "b"])),
            Ok(Invocation::Open {
                places: vec!["a".into(), "b".into()],
                portable: true,
                no_plugins: false
            })
        );
        assert_eq!(
            parse(args(&["--", "--weird-name"])),
            Ok(Invocation::Open {
                places: vec!["--weird-name".into()],
                portable: false,
                no_plugins: false
            })
        );
        assert_eq!(
            parse(args(&["--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert_eq!(
            parse(args(&["a", "b", "c"])),
            Err(CliError::TooMany("b".into()))
        );
        assert_eq!(
            parse(args(&["search", "x"])),
            Err(CliError::Missing("search".into()))
        );
    }

    #[test]
    fn usage_mistakes_exit_with_two_and_a_sentence() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = main_with(
            args(&["--bogus"]),
            &mut std::io::empty(),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 2);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "Could not start diro: the option --bogus is not known. Run diro --help for the usage.\n"
        );
    }

    #[test]
    fn version_says_the_name_and_license_first() {
        let mut out = Vec::new();
        assert_eq!(
            run(Invocation::Version, &mut std::io::empty(), &mut out),
            Ok(0)
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("diro 0.1.0-alpha.1\n"));
        assert!(text.contains("Apache-2.0 OR MIT"));
    }
}
