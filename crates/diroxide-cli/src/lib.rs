//! The command line behind the `diro` binary.
//!
//! - `diro [PATH] [OTHER]`: open the panels at PATH and OTHER.
//! - `diro serve --stdio`: the app core over JSON-RPC, the protocol the
//!   plugins speak, for tests, scripts and other programs.
//! - `diro search PLACE [EXPRESSION]`: search by name, kind, size, date and
//!   contents in the filter language, one row a hit, the name first.
//! - `diro ls [PLACE]` and `diro ls -`: list a place, or the paths on
//!   standard input, one entry a line, the name first.
//! - `diro completions SHELL`: the completion script for bash, zsh, fish
//!   or PowerShell.
//! - `diro keys check [FILE]` and `diro keys print`: the keymap, checked or
//!   printed.
//! - `diro init SHELL`: the shell function that moves the shell where diro
//!   left it, through `--cwd-file`.
//! - `diro archive list|pack|unpack|test`: archives from a shell.
//! - `diro diff BEFORE AFTER`: compare two files, the verdict first, then
//!   one line a change; exit 0 the same, 1 different, 2 trouble.
//! - `diro split FILE` and `diro join PART`: split a file into numbered
//!   parts with a `.crc` sidecar, and put them back together.
//! - `diro watch [PATH]`: say each file that settles in a folder, one line
//!   an arrival, until Ctrl+C; `--print0` for NUL-separated paths.
//! - `diro plugin list|new|enable|disable|run|log`: plugin tooling.
//! - `diro --version`, `diro --help`.
//!
//! Every argument must be valid Unicode: a name that is not is refused
//! with a message, never changed silently.
//!
//! Owners after Phase 0: W1d owns this file, `open.rs` and `serve.rs`, and
//! `cmd/keys.rs` and `cmd/init.rs`; W1e owns the rest of `cmd/` (W3b owns
//! `cmd/plugin.rs`; W2g owns `cmd/archive.rs`, `cmd/split.rs` and
//! `cmd/join.rs`, and their lines here; W3d owns `cmd/diff.rs` and its
//! lines here).

pub mod cmd;
pub mod open;
pub mod serve;

use std::ffi::OsString;
use std::io::{BufRead, Write};

use diroxide_core::{Explain, Message};
use thiserror::Error;

pub use open::OpenOptions;

/// What was asked for on the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Open the panels.
    Open(OpenOptions),
    /// Serve the app core over JSON-RPC.
    Serve,
    /// Search by name, kind, size, date and contents.
    Search(Box<cmd::search::SearchArgs>),
    /// Plugin tooling.
    Plugin(cmd::plugin::PluginCommand),
    /// List a place, or the paths on standard input.
    Ls(cmd::ls::LsArgs),
    /// Print a shell's completion script.
    Completions(cmd::completions::CompletionsArgs),
    /// Check or print the keys.
    Keys(cmd::keys::KeysArgs),
    /// Print a shell's wrapper for `--cwd-file`.
    Init(cmd::init::InitArgs),
    /// List, pack, unpack or test an archive.
    Archive(cmd::archive::ArchiveArgs),
    /// Split a file into numbered parts.
    Split(cmd::split::SplitArgs),
    /// Join a split file's parts.
    Join(cmd::join::JoinArgs),
    /// Compare two files.
    Diff(cmd::diff::DiffArgs),
    /// Say each file that settles in a folder (W3t).
    Watch(cmd::watch::WatchArgs),
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
        "diro search PLACE [EXPRESSION]  find by name, kind, size, date and contents",
        "diro ls [PLACE]           list a folder, one entry a line, the name first",
        "diro ls -                 list the NUL-separated paths on standard input",
        "diro completions SHELL    print the completion script for bash, zsh, fish or powershell",
        "diro keys check [FILE]    say each problem in keymap.toml or FILE",
        "diro keys print           print the current keys as keymap.toml text",
        "diro keys test            say what each key sends and runs; Escape ends it",
        "diro init SHELL           print the shell function that moves the shell where diro left it",
        "diro archive list PLACE   list an archive's members, one a line, the path first",
        "diro archive pack ARCHIVE ITEM...  pack items into a new archive",
        "diro archive unpack ARCHIVE  unpack into the current folder, or --into FOLDER",
        "diro archive test ARCHIVE  read every member of an archive and check it",
        "diro split FILE --preset NAME  split into numbered parts; or --size N, --parts N",
        "diro join PART            join a split file's parts back into one file",
        "diro diff BEFORE AFTER    compare two files: the verdict first, then each change",
        "diro watch [PATH]         say each file that settles in a folder; Ctrl+C stops",
        "diro plugin list          list the plugins",
        "diro --version            print the version",
        "diro --help               print this help",
        "",
        "Options for opening:",
        "--portable                keep the settings beside the program",
        "--no-plugins              start without plugins",
        "--tui                     open in the terminal, even with a display",
        "--gui                     open the window (it comes with alpha.6)",
        "--inline                  draw below the prompt, not on the whole screen",
        "--cwd-file FILE           at quit, write the active folder to FILE for the shell",
        "--theme NAME              use this theme for this session",
        "",
        "Exit codes:",
        "0  done",
        "1  failed, or nothing found",
        "2  wrong use, such as an unknown option",
        "3  partly done: some items failed; the output says which",
        "",
        "diro ls --help gives the options of diro ls; diro archive --help those of diro archive.",
        "diro diff --help gives the options of diro diff.",
        "diro ls --help gives the options of diro ls; diro search --help and diro archive --help theirs.",
    ]
    .join("\n")
}

/// Reads the options and places of `diro [PATH] [OTHER]`.
fn parse_open(args: &[String]) -> Result<OpenOptions, CliError> {
    let mut options = OpenOptions::default();
    let mut only_places = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_places {
            options.places.push(arg.clone());
            continue;
        }
        let (name, inline_value) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |flag: &str| -> Result<String, CliError> {
            match inline_value.clone().or_else(|| rest.next().cloned()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(CliError::Missing(flag.to_owned())),
            }
        };
        match name {
            "--" => only_places = true,
            "--portable" => options.portable = true,
            "--no-plugins" => options.no_plugins = true,
            "--tui" => options.tui = true,
            "--gui" => options.gui = true,
            "--inline" => options.inline = true,
            "--cwd-file" => {
                let file = value("--cwd-file")?;
                if file.chars().any(char::is_control) {
                    return Err(CliError::Failed(
                        "the --cwd-file name has control characters; choose a plain name"
                            .to_owned(),
                    ));
                }
                options.cwd_file = Some(file);
            }
            "--theme" => options.theme = Some(value("--theme")?),
            option if option.starts_with("--") => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            place => options.places.push(place.to_owned()),
        }
    }
    if options.places.len() > 2 {
        return Err(CliError::TooMany(options.places[1].clone()));
    }
    Ok(options)
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
        return Ok(Invocation::Open(OpenOptions::default()));
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
        "search" => cmd::search::parse(&args[1..]).map(|a| Invocation::Search(Box::new(a))),
        "plugin" => cmd::plugin::parse(&args[1..]).map(Invocation::Plugin),
        "ls" => cmd::ls::parse(&args[1..]).map(Invocation::Ls),
        "completions" => cmd::completions::parse(&args[1..]).map(Invocation::Completions),
        "keys" => cmd::keys::parse(&args[1..]).map(Invocation::Keys),
        "init" => cmd::init::parse(&args[1..]).map(Invocation::Init),
        "archive" => cmd::archive::parse(&args[1..]).map(Invocation::Archive),
        "split" => cmd::split::parse(&args[1..]).map(Invocation::Split),
        "join" => cmd::join::parse(&args[1..]).map(Invocation::Join),
        "diff" => cmd::diff::parse(&args[1..]).map(Invocation::Diff),
        "watch" => cmd::watch::parse(&args[1..]).map(Invocation::Watch),
        _ => parse_open(&args).map(Invocation::Open),
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
        Invocation::Open(options) => open::run(&options, out),
        Invocation::Serve => serve::run(input, out),
        Invocation::Search(args) => cmd::search::run(&args, out),
        Invocation::Plugin(command) => cmd::plugin::run(&command, out),
        Invocation::Ls(args) => cmd::ls::run(&args, input, out),
        Invocation::Completions(args) => cmd::completions::run(args, out),
        Invocation::Keys(args) => cmd::keys::run(&args, open::paths(false).as_ref(), out),
        Invocation::Init(args) => cmd::init::run(args, out),
        Invocation::Archive(args) => cmd::archive::run(&args, input, out),
        Invocation::Split(args) => cmd::split::run(&args, out),
        Invocation::Join(args) => cmd::join::run(&args, out),
        Invocation::Diff(args) => cmd::diff::run(&args, out),
        Invocation::Watch(args) => cmd::watch::run(&args, out),
    }
}

/// The whole program: parse, run, and turn errors into the house style on
/// `err`. Returns the exit code: 0 for success, 1 for failure, 2 for a
/// usage mistake, 3 when a subcommand was partly done. A `Failed` error
/// that is already a sentence ("Could not list D:\nope: it does not
/// exist. Check the path.") is written as it is; a bare cause is written
/// as "Could not finish: CAUSE.".
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
        Err(CliError::Failed(sentence))
            if sentence.chars().next().is_some_and(char::is_uppercase) =>
        {
            let _ = writeln!(err, "{sentence}");
            1
        }
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

    fn open(places: &[&str]) -> OpenOptions {
        OpenOptions {
            places: places.iter().map(|p| (*p).to_owned()).collect(),
            ..OpenOptions::default()
        }
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(args(&["--version"])), Ok(Invocation::Version));
        assert_eq!(parse(args(&["serve", "--stdio"])), Ok(Invocation::Serve));
        assert_eq!(
            parse(args(&["--portable", "a", "b"])),
            Ok(Invocation::Open(OpenOptions {
                portable: true,
                ..open(&["a", "b"])
            }))
        );
        assert_eq!(
            parse(args(&["--", "--weird-name"])),
            Ok(Invocation::Open(open(&["--weird-name"])))
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
            parse(args(&["search"])),
            Err(CliError::Missing("search".into()))
        );
    }

    #[test]
    fn the_opening_options_are_read() {
        assert_eq!(
            parse(args(&[
                "--tui",
                "--cwd-file",
                "cwd.txt",
                "--theme=nord",
                "--inline",
                "--gui",
                "left",
            ])),
            Ok(Invocation::Open(OpenOptions {
                tui: true,
                gui: true,
                inline: true,
                cwd_file: Some("cwd.txt".into()),
                theme: Some("nord".into()),
                ..open(&["left"])
            }))
        );
        assert_eq!(
            parse(args(&["--cwd-file"])),
            Err(CliError::Missing("--cwd-file".into()))
        );
        assert!(matches!(
            parse(args(&["--cwd-file", "a\u{7}b"])),
            Err(CliError::Failed(_))
        ));
        assert!(matches!(
            parse(args(&["keys", "print"])),
            Ok(Invocation::Keys(cmd::keys::KeysArgs::Print))
        ));
        assert!(matches!(
            parse(args(&["init", "bash"])),
            Ok(Invocation::Init(_))
        ));
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
