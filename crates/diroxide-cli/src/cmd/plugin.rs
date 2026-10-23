//! `diro plugin`: plugins and script commands from a shell (plan section
//! 5.8; R22 sections 4 and 7; answers 90 to 92). W4b owns this file and
//! its lines in `lib.rs`, `cmd/mod.rs` and the completions.
//!
//! - `diro plugin list`: one line a plugin or script, its name and state
//!   first: "Total size of selection: not enabled, script, id total-size".
//! - `diro plugin new NAME --language bash|python [--kind command|lister|
//!   provider] [--dir FOLDER]`: a working skeleton copied from a shipped
//!   example, never over anything, then three lines: what it wrote, how
//!   to check it, how to try it once.
//! - `diro plugin enable NAME [--yes]`: says what it runs, what diroxide
//!   checks and what it only states, then asks; the answer starts on No.
//!   `disable NAME` forgets the yes.
//! - `diro plugin run NAME [COMMAND] [-- PATH...]`: runs once, the paths
//!   as the selection in the current folder. A plugin that is not enabled
//!   runs once after one question; one that changed since it was enabled
//!   asks again. `--explain` says what a script or a manifest-only
//!   command would run, in words, and runs nothing.
//! - `diro plugin restart NAME`: starts a plugin's program, checks that it
//!   answers, and stops it.
//! - `diro plugin log NAME`: the last lines of its log.
//! - `diro plugin check [NAME|PATH]`: every problem, its field and its
//!   fix; "Checked 1 plugin: no problems."
//! - `diro plugin install PATH`: copies a plugin folder or a script into
//!   the plugins folder; installing never enables.
//!
//! `--portable` uses the folders beside the program. Exit codes: 0 done,
//! 1 failed or declined, 2 wrong use, 3 partly done. Every line a plugin
//! causes carries its name last (answer 90).

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use diroxide_components::pin::sha256_hex;
use diroxide_config::{Paths, PluginSettings};
use diroxide_core::{CommandId, Entry, Place};
use diroxide_plugins::host::{PluginInfo, QUIT_BUDGET};
use diroxide_plugins::manifest::{Manifest, Severity};
use diroxide_plugins::script::command::ScriptCommand;
use diroxide_plugins::script::consent::{
    ConsentState, ConsentStore, Fingerprint, Hasher, Kind, Subject, disabled_line, enabled_line,
    not_enabled_line, plugin_fingerprint, script_fingerprint, trust_problem,
};
use diroxide_plugins::script::context::{Picked, RunContext, StateFolders};
use diroxide_plugins::script::directives::Directive;
use diroxide_plugins::script::installed::{self, Installed, scripts_dir};
use diroxide_plugins::script::interpreter::Lookup;
use diroxide_plugins::script::runner::{self, Cancel, RunEnv, RunSink};
use diroxide_plugins::script::skeleton::{self, Kind as SkeletonKind, Language, SkeletonError};
use diroxide_plugins::services::settings_for;
use diroxide_plugins::shapes::{PanelRef, WireContext, WireEntry};
use diroxide_plugins::{Host, HostEvent, PluginError, lines};
use diroxide_tools::ToolRegistry;
use serde_json::{Map, Value, json};

use crate::CliError;
use crate::cmd::archive::{Asker, TerminalAsker};

/// The method `restart` calls to see that a plugin answers. No plugin
/// has it, so a working plugin answers "method not found" (-32601), as
/// the protocol asks of every plugin.
pub const PROBE_METHOD: &str = "diroxide/probe";

/// What `diro plugin` was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginAction {
    /// Print the help.
    Help,
    /// List the plugins and scripts.
    List,
    /// Write a skeleton.
    New(String),
    /// Enable one.
    Enable(String),
    /// Disable one.
    Disable(String),
    /// Run one once.
    Run {
        /// The plugin or script.
        name: String,
        /// The command, for a plugin with more than one.
        command: Option<String>,
        /// The selection, as typed.
        paths: Vec<String>,
    },
    /// Start a plugin, see that it answers, stop it.
    Restart(String),
    /// Print the end of a plugin's log.
    Log(String),
    /// Check one, a path, or every one.
    Check(Option<String>),
    /// Install a plugin folder or a script.
    Install(String),
}

/// `diro plugin` as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginArgs {
    /// The subcommand.
    pub action: PluginAction,
    /// `--yes`: the answer to the question, given ahead.
    pub yes: bool,
    /// `--portable`: the folders beside the program.
    pub portable: bool,
    /// `--explain`: say what would run, run nothing.
    pub explain: bool,
    /// `--language`: bash or python, for `new`.
    pub language: Option<String>,
    /// `--kind`: command, lister or provider, for `new`.
    pub kind: Option<String>,
    /// `--dir`: where `new` writes.
    pub dir: Option<String>,
}

/// The help for `diro plugin`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro plugin: plugins and script commands from a shell.",
        "",
        "diro plugin list                 each plugin and script: name, state, kind, id",
        "diro plugin new NAME --language bash|python  write a working skeleton",
        "  --kind command|lister|provider   for Python; command by default",
        "  --dir FOLDER                     where to write; the plugins folder by default",
        "diro plugin enable NAME [--yes]  say what it may do, ask, then enable it",
        "diro plugin disable NAME         forget that it was enabled",
        "diro plugin run NAME [COMMAND] [-- PATH...]  run it once on PATH",
        "diro plugin run NAME --explain   say what it would run; run nothing",
        "diro plugin restart NAME         start it, check that it answers, stop it",
        "diro plugin log NAME             the last lines of its log",
        "diro plugin check [NAME|PATH]    every problem, its field and its fix",
        "diro plugin install PATH         copy a plugin folder or a script in; not enabled",
        "",
        "Options:",
        "--yes       answer yes to the question; it is still printed",
        "--portable  use the settings and plugins beside the program",
        "",
        "Exit codes: 0 done; 1 failed or declined; 2 wrong use; 3 partly done.",
    ]
    .join("\n")
}

/// Reads the arguments after `diro plugin`.
pub fn parse(args: &[String]) -> Result<PluginArgs, CliError> {
    let mut parsed = PluginArgs {
        action: PluginAction::Help,
        yes: false,
        portable: false,
        explain: false,
        language: None,
        kind: None,
        dir: None,
    };
    let mut words = Vec::new();
    let mut paths = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--" => {
                paths.extend(rest.by_ref().cloned());
                break;
            }
            "--help" | "-h" => return Ok(parsed),
            "--yes" | "-y" => parsed.yes = true,
            "--portable" => parsed.portable = true,
            "--explain" => parsed.explain = true,
            "--language" | "--kind" | "--dir" => {
                let value = rest
                    .next()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| CliError::Missing(arg.clone()))?;
                set_valued(&mut parsed, arg, value.clone());
            }
            option
                if ["--language=", "--kind=", "--dir="]
                    .iter()
                    .any(|p| option.starts_with(p)) =>
            {
                let (flag, value) = option.split_once('=').unwrap_or((option, ""));
                if value.is_empty() {
                    return Err(CliError::Missing(flag.to_owned()));
                }
                set_valued(&mut parsed, flag, value.to_owned());
            }
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            word => words.push(word.to_owned()),
        }
    }
    let mut words = words.into_iter();
    let first = words.next();
    let mut one = |what: &str| {
        words
            .next()
            .ok_or_else(|| CliError::Missing(format!("plugin {what}")))
    };
    parsed.action = match first.as_deref() {
        None => PluginAction::Help,
        Some("list") => PluginAction::List,
        Some("new") => PluginAction::New(one("new")?),
        Some("enable") => PluginAction::Enable(one("enable")?),
        Some("disable") => PluginAction::Disable(one("disable")?),
        Some("restart") => PluginAction::Restart(one("restart")?),
        Some("log") => PluginAction::Log(one("log")?),
        Some("install") => PluginAction::Install(one("install")?),
        Some("check") => PluginAction::Check(words.next()),
        Some("run") => {
            let name = one("run")?;
            let command = words.next();
            PluginAction::Run {
                name,
                command,
                paths: std::mem::take(&mut paths),
            }
        }
        Some(other) => {
            return Err(CliError::Failed(format!(
                "diro plugin has no {other}; use list, new, enable, disable, run, restart, log, check or install"
            )));
        }
    };
    if let Some(extra) = words.next() {
        return Err(CliError::TooMany(format!("plugin {extra}")));
    }
    if !paths.is_empty() {
        return Err(CliError::Failed(
            "paths after -- go with diro plugin run".to_owned(),
        ));
    }
    let valued = parsed.language.is_some() || parsed.kind.is_some() || parsed.dir.is_some();
    if valued && !matches!(parsed.action, PluginAction::New(_)) {
        return Err(CliError::Failed(
            "--language, --kind and --dir go with diro plugin new".to_owned(),
        ));
    }
    if parsed.explain && !matches!(parsed.action, PluginAction::Run { .. }) {
        return Err(CliError::Failed(
            "--explain goes with diro plugin run".to_owned(),
        ));
    }
    Ok(parsed)
}

/// Sets `--language`, `--kind` or `--dir`.
fn set_valued(parsed: &mut PluginArgs, flag: &str, value: String) {
    match flag {
        "--language" => parsed.language = Some(value),
        "--kind" => parsed.kind = Some(value),
        _ => parsed.dir = Some(value),
    }
}

/// What a run of `diro plugin` works with: the real folders and terminal
/// in `diro`, folders under a test folder and a scripted asker in tests.
pub struct Env<'a> {
    /// The config and state folders.
    pub paths: Paths,
    /// Who answers the questions.
    pub asker: &'a dyn Asker,
    /// Where a prompt's answer is read from.
    pub input: &'a mut dyn BufRead,
    /// Standard error: logs and refusals.
    pub err: &'a mut dyn Write,
    /// Where interpreters are found.
    pub lookup: Lookup,
    /// The tools, with the owner's `tools.toml`.
    pub registry: ToolRegistry,
    /// The current folder, the panels of a run.
    pub here: Place,
    /// The hash of the consent record.
    pub hash: Hasher,
}

impl Env<'_> {
    fn plugins_dir(&self) -> PathBuf {
        self.paths.plugins_dir()
    }

    /// The plugins' state folder: logs, runs, consent.
    fn state_dir(&self) -> PathBuf {
        self.paths.state_dir.join("plugins")
    }
}

fn write_error(e: io::Error) -> CliError {
    CliError::Failed(e.to_string())
}

/// Runs `diro plugin` with the real folders, tools and terminal.
pub fn run(
    args: &PluginArgs,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    if args.action == PluginAction::Help {
        writeln!(out, "{}", help_text()).map_err(write_error)?;
        return Ok(0);
    }
    let Some(paths) = crate::open::paths(args.portable) else {
        return Err(CliError::Failed(
            "Could not find the settings folder. Try --portable.".to_owned(),
        ));
    };
    let config = diroxide_config::load(&paths);
    let asterisks = config.settings.secret_echo != diroxide_config::SecretEcho::Silent;
    let asker = TerminalAsker::if_present(asterisks);
    let mut err = io::stderr();
    let env = Env {
        paths,
        asker: asker.as_ref(),
        input,
        err: &mut err,
        lookup: Lookup::system(),
        registry: ToolRegistry::builtin().with_overrides(config.tools.clone()),
        here: crate::open::current_place()?,
        hash: sha256_hex,
    };
    run_in(args, env, out)
}

/// Runs `diro plugin` in `env`, writing results to `out`.
pub fn run_in(args: &PluginArgs, mut env: Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    match &args.action {
        PluginAction::Help => {
            writeln!(out, "{}", help_text()).map_err(write_error)?;
            Ok(0)
        }
        PluginAction::List => list(&env, out),
        PluginAction::New(name) => new(name, args, &env, out),
        PluginAction::Enable(name) => enable(name, args.yes, &env, out),
        PluginAction::Disable(name) => disable(name, &env, out),
        PluginAction::Run {
            name,
            command,
            paths,
        } => run_one(name, command.as_deref(), paths, args, &mut env, out),
        PluginAction::Restart(name) => restart(name, args.yes, &mut env, out),
        PluginAction::Log(name) => log(name, &env, out),
        PluginAction::Check(target) => check(target.as_deref(), &env, out),
        PluginAction::Install(path) => install(path, &env, out),
    }
}

/// A plugin or script by name, or the sentence that none is called so.
fn found(name: &str, env: &Env<'_>) -> Result<Installed, CliError> {
    installed::find(&env.plugins_dir(), name, env.lookup.os).ok_or_else(|| {
        CliError::Failed(format!(
            "No plugin or script is called {name}. Run diro plugin list for their names."
        ))
    })
}

/// Its fingerprint now, and the question's subject.
fn subject_of(item: &Installed, env: &Env<'_>) -> Result<(Fingerprint, Subject, Kind), CliError> {
    let failed = |e: io::Error| CliError::Failed(format!("Could not read {}: {e}.", item.name()));
    match item {
        Installed::Plugin(plugin) => {
            let manifest = plugin.report.manifest.as_ref().ok_or_else(|| {
                CliError::Failed(format!(
                    "{} has errors. Run diro plugin check {} for them.",
                    item.name(),
                    item.id()
                ))
            })?;
            let fingerprint =
                plugin_fingerprint(manifest, &plugin.folder, env.hash).map_err(failed)?;
            Ok((
                fingerprint,
                Subject::plugin(manifest, env.lookup.os),
                Kind::Plugin,
            ))
        }
        Installed::Script(script) => {
            if let Some(why) = trust_problem(&script.path) {
                return Err(CliError::Failed(format!(
                    "Could not use {}: {why}. Make it writable by you alone.",
                    item.name()
                )));
            }
            let fingerprint = script_fingerprint(&script.path, env.hash).map_err(failed)?;
            let header = &script.header.header;
            let file = script
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (runs, policy) = match env.lookup.choose(&script.path, &header.recipe.interpreter) {
                Ok(interpreter) => (
                    interpreter.runs_line(&script.path),
                    interpreter.policy_override,
                ),
                Err(_) => (format!("It runs: {file}"), false),
            };
            let subject = Subject::script(&item.name(), runs, policy, header.recipe.unattended);
            Ok((fingerprint, subject, Kind::Script))
        }
    }
}

/// Asks the last row, after printing the others; `--yes` answers ahead,
/// and with no terminal the answer is No.
fn agreed(
    rows: &[String],
    yes: bool,
    asker: &dyn Asker,
    out: &mut dyn Write,
) -> Result<bool, CliError> {
    let Some((question, before)) = rows.split_last() else {
        return Ok(false);
    };
    for row in before {
        writeln!(out, "{row}").map_err(write_error)?;
    }
    if yes {
        writeln!(out, "{question} yes, from --yes").map_err(write_error)?;
        return Ok(true);
    }
    if !asker.present() {
        writeln!(out, "{question}").map_err(write_error)?;
        writeln!(out, "Answered no: there is no terminal to ask.").map_err(write_error)?;
        return Ok(false);
    }
    out.flush().map_err(write_error)?;
    Ok(asker.confirm(question))
}

fn has_errors(item: &Installed, env: &Env<'_>) -> bool {
    installed::check(item, &env.lookup)
        .iter()
        .any(|p| p.severity == Severity::Error)
}

fn list(env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let all = installed::scan(&env.plugins_dir(), env.lookup.os);
    if all.is_empty() {
        writeln!(
            out,
            "No plugins or scripts in {}",
            env.plugins_dir().display()
        )
        .map_err(write_error)?;
        return Ok(0);
    }
    let store = ConsentStore::open(&env.state_dir());
    for item in &all {
        let state = if has_errors(item, env) {
            "has errors"
        } else {
            match subject_of(item, env).map(|(f, _, _)| store.state(item.id(), &f)) {
                Ok(ConsentState::Enabled) => "enabled",
                Ok(ConsentState::Changed) => "changed, asks again",
                Ok(ConsentState::NotEnabled) => "not enabled",
                Err(_) => "cannot be read",
            }
        };
        let name = item.name();
        let id = if name == item.id() {
            String::new()
        } else {
            format!(", id {}", item.id())
        };
        writeln!(out, "{name}: {state}, {}{id}", item.kind_words()).map_err(write_error)?;
    }
    Ok(0)
}

fn new(name: &str, args: &PluginArgs, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let language_word = args.language.as_deref().unwrap_or("python");
    let Some(language) = Language::parse(language_word) else {
        writeln!(
            out,
            "Could not make {name}: {language_word} is not a language. Use bash or python."
        )
        .map_err(write_error)?;
        return Ok(2);
    };
    let kind_word = args.kind.as_deref().unwrap_or("command");
    let Some(kind) = SkeletonKind::parse(kind_word) else {
        writeln!(
            out,
            "Could not make {name}: {kind_word} is not a kind. Use command, lister or provider."
        )
        .map_err(write_error)?;
        return Ok(2);
    };
    let skeleton = match skeleton::generate(name, language, kind) {
        Ok(skeleton) => skeleton,
        Err(error) => {
            writeln!(out, "{}", error.sentence()).map_err(write_error)?;
            return Ok(match error {
                SkeletonError::Exists(_) | SkeletonError::Io(_) => 1,
                _ => 2,
            });
        }
    };
    let (dir, in_plugins) = match &args.dir {
        Some(text) => {
            let place = Place::resolve(text, &env.here).map_err(|e| {
                CliError::Failed(diroxide_core::Message::from_error("use", text, &e).text)
            })?;
            let Some(local) = place.local_path() else {
                writeln!(out, "Could not make {name}: --dir must be a local folder.")
                    .map_err(write_error)?;
                return Ok(2);
            };
            (PathBuf::from(local), false)
        }
        None => (skeleton::default_dir(&env.plugins_dir(), language), true),
    };
    match skeleton::write(&skeleton, &dir) {
        Ok(written) => {
            for line in skeleton::written_lines(&skeleton, &written, &dir, in_plugins) {
                writeln!(out, "{line}").map_err(write_error)?;
            }
            Ok(0)
        }
        Err(error) => {
            writeln!(out, "{}", error.sentence()).map_err(write_error)?;
            Ok(1)
        }
    }
}

fn enable(name: &str, yes: bool, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let item = found(name, env)?;
    if has_errors(&item, env) {
        writeln!(out, "Not enabled: {} has errors.", item.name()).map_err(write_error)?;
        writeln!(out, "Next: diro plugin check {}", item.id()).map_err(write_error)?;
        return Ok(1);
    }
    let (fingerprint, subject, kind) = subject_of(&item, env)?;
    let mut store = ConsentStore::open(&env.state_dir());
    let rows = match store.state(item.id(), &fingerprint) {
        ConsentState::Enabled => {
            writeln!(out, "Already enabled: {}.", item.name()).map_err(write_error)?;
            return Ok(0);
        }
        ConsentState::Changed => subject.changed_question(),
        ConsentState::NotEnabled => subject.enable_question(),
    };
    if !agreed(&rows, yes, env.asker, out)? {
        writeln!(out, "{}", not_enabled_line()).map_err(write_error)?;
        return Ok(1);
    }
    store
        .enable(item.id(), kind, fingerprint)
        .map_err(|e| CliError::Failed(format!("Could not save the consent: {e}.")))?;
    writeln!(out, "{}", enabled_line(&item.name())).map_err(write_error)?;
    Ok(0)
}

fn disable(name: &str, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let item = found(name, env)?;
    let mut store = ConsentStore::open(&env.state_dir());
    let was = store
        .disable(item.id())
        .map_err(|e| CliError::Failed(format!("Could not save the consent: {e}.")))?;
    if was {
        writeln!(out, "{}", disabled_line(&item.name())).map_err(write_error)?;
    } else {
        writeln!(out, "{} was not enabled.", item.name()).map_err(write_error)?;
    }
    Ok(0)
}

/// Whether a plugin or script may run now: enabled and unchanged, or the
/// owner says yes to running it once, or to enabling it again.
fn may_run(
    item: &Installed,
    yes: bool,
    env: &Env<'_>,
    out: &mut dyn Write,
) -> Result<bool, CliError> {
    if has_errors(item, env) {
        writeln!(out, "Could not run {}: it has errors.", item.name()).map_err(write_error)?;
        writeln!(out, "Next: diro plugin check {}", item.id()).map_err(write_error)?;
        return Ok(false);
    }
    let (fingerprint, subject, kind) = subject_of(item, env)?;
    let mut store = ConsentStore::open(&env.state_dir());
    match store.state(item.id(), &fingerprint) {
        ConsentState::Enabled => Ok(true),
        ConsentState::NotEnabled => {
            let agreed = agreed(&subject.run_once_question(), yes, env.asker, out)?;
            if !agreed {
                writeln!(out, "{}", not_enabled_line()).map_err(write_error)?;
            }
            Ok(agreed)
        }
        ConsentState::Changed => {
            let agreed = agreed(&subject.changed_question(), yes, env.asker, out)?;
            if agreed {
                store
                    .enable(item.id(), kind, fingerprint)
                    .map_err(|e| CliError::Failed(format!("Could not save the consent: {e}.")))?;
                writeln!(out, "{}", enabled_line(&item.name())).map_err(write_error)?;
            } else {
                writeln!(out, "{}", not_enabled_line()).map_err(write_error)?;
            }
            Ok(agreed)
        }
    }
}

/// The context of a run from a shell: both panels in the current folder,
/// the paths as the marked entries, the first as the cursor.
fn shell_context(paths: &[String], env: &Env<'_>) -> Result<RunContext, CliError> {
    let mut context = RunContext::new(env.here.clone(), env.here.clone());
    for text in paths {
        let place = Place::resolve(text, &env.here).map_err(|e| {
            CliError::Failed(diroxide_core::Message::from_error("use", text, &e).text)
        })?;
        context.marked.push(Picked::from_disk(place));
    }
    context.cursor = context.marked.first().cloned();
    Ok(context)
}

/// Says a run's live effects on standard output, the plugin's name last.
struct ShellSink<'a> {
    source: String,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

impl RunSink for ShellSink<'_> {
    fn act(&mut self, directive: &Directive) {
        if let Some(message) = directive.message(&self.source) {
            let _ = writeln!(self.out, "{}", message.text);
        }
    }

    fn output(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }

    fn log(&mut self, line: &str) {
        let _ = writeln!(self.err, "{line}");
    }

    fn note(&mut self, message: &diroxide_core::Message) {
        let _ = writeln!(self.out, "{}", message.text);
    }
}

/// A deferred directive in words, for a run with no panels.
fn deferred_words(directive: &Directive) -> Option<String> {
    let place = directive.place()?;
    Some(match directive {
        Directive::Cd(_) => format!("The panel would open {place}."),
        Directive::CdOther(_) => format!("The other panel would open {place}."),
        Directive::Select(_) => format!("The panel would mark {place}."),
        Directive::Unselect(_) => format!("The panel would unmark {place}."),
        _ => format!("The cursor would go to {place}."),
    })
}

fn run_one(
    name: &str,
    command: Option<&str>,
    paths: &[String],
    args: &PluginArgs,
    env: &mut Env<'_>,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let item = found(name, env)?;
    let context = shell_context(paths, env)?;
    let runnable = match &item {
        Installed::Script(_) => item.script_command(),
        Installed::Plugin(plugin) => {
            let Some(manifest) = plugin.report.manifest.as_ref() else {
                return may_run(&item, args.yes, env, out).map(|_| 1);
            };
            let spec = match command {
                Some(id) => manifest.command(id).ok_or_else(|| {
                    CliError::Failed(format!(
                        "{} has no command {id}. Its commands: {}.",
                        manifest.name,
                        manifest
                            .commands
                            .iter()
                            .map(|c| c.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?,
                None => match manifest.commands.as_slice() {
                    [only] => only,
                    [] => {
                        return Err(CliError::Failed(format!(
                            "{} has no commands to run.",
                            manifest.name
                        )));
                    }
                    many => {
                        return Err(CliError::Failed(format!(
                            "Name one of {}'s commands: {}.",
                            manifest.name,
                            many.iter()
                                .map(|c| c.id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                },
            };
            let index = manifest
                .commands
                .iter()
                .position(|c| c.id == spec.id)
                .unwrap_or_default();
            match ScriptCommand::from_manifest(manifest, index, spec, &plugin.folder) {
                Some((runnable, _)) => Some(runnable),
                None if args.explain => {
                    writeln!(out, "--explain is for scripts and manifest-only commands.")
                        .map_err(write_error)?;
                    return Ok(2);
                }
                None => {
                    if !may_run(&item, args.yes, env, out)? {
                        return Ok(1);
                    }
                    return run_process_plugin(
                        manifest,
                        &plugin.folder,
                        &spec.id,
                        &context,
                        env,
                        out,
                    );
                }
            }
        }
    };
    let Some(runnable) = runnable else {
        return Ok(1);
    };
    let run_env = run_env_for(&item, env);
    if args.explain {
        return match runner::explain(&runnable, &context, &run_env) {
            Ok(lines) => {
                for line in lines {
                    writeln!(out, "{line}").map_err(write_error)?;
                }
                Ok(0)
            }
            Err(e) => {
                writeln!(out, "{}", e.sentence(&runnable.title)).map_err(write_error)?;
                Ok(1)
            }
        };
    }
    if !may_run(&item, args.yes, env, out)? {
        return Ok(1);
    }
    let mut context = context;
    if let Some(question) = &runnable.recipe.confirm
        && !agreed(&[format!("{question} y or n")], args.yes, env.asker, out)?
    {
        writeln!(out, "{}", lines::cancelled(&runnable.title)).map_err(write_error)?;
        return Ok(1);
    }
    if let Some(prompt) = &runnable.recipe.prompt {
        write!(out, "{prompt} ").map_err(write_error)?;
        out.flush().map_err(write_error)?;
        let mut answer = String::new();
        env.input.read_line(&mut answer).map_err(write_error)?;
        context.input = Some(answer.trim_end_matches(['\r', '\n']).to_owned());
    }
    let mut sink = ShellSink {
        source: runnable.source.clone(),
        out,
        err: env.err,
    };
    let report = match runner::run(&runnable, &context, &run_env, &mut sink, &Cancel::new()) {
        Ok(report) => report,
        Err(e) => {
            writeln!(sink.out, "{}", e.sentence(&runnable.title)).map_err(write_error)?;
            return Ok(1);
        }
    };
    for place in &report.places {
        writeln!(sink.out, "{place}").map_err(write_error)?;
    }
    if runnable.is_lister() {
        let query = context.input.clone().unwrap_or_default();
        let line = if report.places.is_empty() {
            diroxide_plugins::script::words::no_results(&query)
        } else {
            diroxide_plugins::script::words::results_title(
                report.places.len(),
                &query,
                &runnable.source,
            )
        };
        writeln!(sink.out, "{line}").map_err(write_error)?;
        if report.capped {
            writeln!(
                sink.out,
                "{}",
                diroxide_plugins::script::words::first_shown(runnable.recipe.max_rows)
            )
            .map_err(write_error)?;
        }
    }
    if !report.output.is_empty()
        && runnable.recipe.mode != diroxide_plugins::script::RunMode::OutputPane
    {
        for line in &report.output {
            writeln!(sink.out, "{line}").map_err(write_error)?;
        }
    }
    for directive in &report.deferred {
        if let Some(words) = deferred_words(directive) {
            writeln!(sink.out, "{words}").map_err(write_error)?;
        }
    }
    let quiet = report.ending.is_done() && runnable.is_script();
    if !quiet {
        for message in &report.said {
            writeln!(sink.out, "{}", message.text).map_err(write_error)?;
        }
    }
    if report.wait_for_enter {
        writeln!(
            sink.out,
            "{}",
            diroxide_plugins::script::words::press_enter()
        )
        .map_err(write_error)?;
        let mut line = String::new();
        let _ = env.input.read_line(&mut line);
    }
    Ok(report.ending.exit_code())
}

/// The run environment of a script or a manifest-only command: its own
/// data folder, the plugins' runs folder, its settings' defaults.
fn run_env_for(item: &Installed, env: &Env<'_>) -> RunEnv {
    let state = env.state_dir();
    let mut run_env = RunEnv::system(StateFolders {
        runs: state.join("runs"),
        data: state.join(item.id()).join("data"),
    });
    run_env.lookup = env.lookup.clone();
    run_env.registry = env.registry.clone();
    if let Some(manifest) = item.manifest() {
        run_env.settings = settings_for(manifest, &Map::new());
    }
    run_env
}

/// A host for one plugin, enabled for this run, its log and state in the
/// plugins' state folder.
fn host_for(manifest: &Manifest, folder: &Path, env: &Env<'_>) -> Host {
    let mut host = Host::new(&PluginSettings::default()).with_state_dir(env.state_dir());
    host.add(PluginInfo {
        manifest: manifest.clone(),
        folder: folder.to_path_buf(),
        enabled: true,
    });
    host
}

/// Says what the host has to say: the plugin's words, its failures.
fn say_events(host: &Host, out: &mut dyn Write) -> Result<(), CliError> {
    for event in host.take_events() {
        let message = match event {
            HostEvent::Said { message, .. }
            | HostEvent::StartFailed { message, .. }
            | HostEvent::TimedOut { message, .. }
            | HostEvent::Crashed { message, .. }
            | HostEvent::TurnedOff { message, .. }
            | HostEvent::Flooded { message, .. }
            | HostEvent::Waiting { message, .. } => message,
            HostEvent::IdleStopped { .. } => continue,
        };
        writeln!(out, "{}", message.text).map_err(write_error)?;
    }
    Ok(())
}

/// The cause of a plugin error, without the plugin's name in front.
fn cause_of(error: &PluginError) -> String {
    match error {
        PluginError::Remote { message, code, .. } => format!("{message} (code {code})"),
        PluginError::Start { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn run_process_plugin(
    manifest: &Manifest,
    folder: &Path,
    command: &str,
    context: &RunContext,
    env: &Env<'_>,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let host = host_for(manifest, folder, env);
    let title = manifest
        .command(command)
        .map_or_else(|| manifest.name.clone(), |c| c.name.clone());
    let id = CommandId::new(format!("{}.{command}", manifest.id))
        .map_err(|e| CliError::Failed(e.to_string()))?;
    let wire = |p: &Picked| {
        let entry = Entry::new(p.place.name().unwrap_or_default(), p.kind);
        WireEntry::from_entry(&entry, Some(&p.place))
    };
    let wire_context = WireContext::new(
        &id,
        PanelRef::for_place(&context.active),
        PanelRef::for_place(&context.other),
    )
    .with_selection(
        context.cursor.as_ref().map(wire),
        context.marked.iter().map(wire).collect(),
    );
    let params = json!({ "command": command, "context": wire_context });
    let result = host.call(
        &manifest.id,
        diroxide_plugins::protocol::methods::COMMAND_RUN,
        params,
    );
    say_events(&host, out)?;
    let code = match result {
        Ok(value) => {
            let count = |key: &str| match value.get(key) {
                Some(Value::Array(items)) => items.len(),
                Some(Value::Number(n)) => usize::try_from(n.as_u64().unwrap_or(0)).unwrap_or(0),
                _ => 0,
            };
            let (done, failed) = (count("done"), count("failed"));
            if failed > 0 {
                writeln!(
                    out,
                    "{}",
                    lines::could_not_run(&title, &format!("{failed} failed"))
                )
                .map_err(write_error)?;
                writeln!(out, "{}", lines::next_log(&manifest.id)).map_err(write_error)?;
                if done > 0 { 3 } else { 1 }
            } else {
                0
            }
        }
        Err(error) => {
            writeln!(out, "{}", lines::could_not_run(&title, &cause_of(&error)))
                .map_err(write_error)?;
            if let Some(log) = host.log(&manifest.id)
                && let Some(last) = log.tail().last()
            {
                writeln!(out, "{}", lines::last_error(last)).map_err(write_error)?;
            }
            writeln!(out, "{}", lines::next_log(&manifest.id)).map_err(write_error)?;
            1
        }
    };
    host.shutdown_all(QUIT_BUDGET);
    say_events(&host, out)?;
    Ok(code)
}

fn restart(name: &str, yes: bool, env: &mut Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let item = found(name, env)?;
    let Installed::Plugin(plugin) = &item else {
        writeln!(
            out,
            "Nothing to restart: {} starts fresh each run.",
            item.name()
        )
        .map_err(write_error)?;
        return Ok(0);
    };
    let Some(manifest) = plugin.report.manifest.as_ref().filter(|m| m.is_process()) else {
        writeln!(
            out,
            "Nothing to restart: {} runs no program of its own.",
            item.name()
        )
        .map_err(write_error)?;
        return Ok(0);
    };
    if !may_run(&item, yes, env, out)? {
        return Ok(1);
    }
    let host = host_for(manifest, &plugin.folder, env);
    let started = std::time::Instant::now();
    let answer = host.call(&manifest.id, PROBE_METHOD, json!({}));
    let took = started.elapsed();
    say_events(&host, out)?;
    host.shutdown_all(QUIT_BUDGET);
    say_events(&host, out)?;
    match answer {
        Ok(_) | Err(PluginError::Remote { code: -32601, .. }) => {
            writeln!(
                out,
                "Started and stopped: {}, it answered in {}.",
                manifest.name,
                lines::duration_words(Duration::from_millis(took.as_millis() as u64))
            )
            .map_err(write_error)?;
            Ok(0)
        }
        Err(error) => {
            writeln!(
                out,
                "{}",
                lines::could_not_start(&manifest.name, &cause_of(&error))
            )
            .map_err(write_error)?;
            writeln!(out, "{}", lines::next_log(&manifest.id)).map_err(write_error)?;
            Ok(1)
        }
    }
}

fn log(name: &str, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let item = found(name, env)?;
    let folder = env.state_dir().join(item.id());
    let current = folder.join(format!("{}.log", item.id()));
    let old = folder.join(format!("{}.log.old", item.id()));
    let mut text = String::new();
    for path in [&old, &current] {
        if let Ok(bytes) = fs::read(path) {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        writeln!(out, "No log yet for {}.", item.name()).map_err(write_error)?;
        return Ok(0);
    }
    let skip = lines.len().saturating_sub(40);
    for line in &lines[skip..] {
        writeln!(out, "{line}").map_err(write_error)?;
    }
    Ok(0)
}

fn check(target: Option<&str>, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let items: Vec<Installed> = match target {
        None => installed::scan(&env.plugins_dir(), env.lookup.os),
        Some(name) => {
            let as_path = Path::new(name);
            if (as_path.is_dir() && as_path.join(Manifest::FILE).is_file()) || as_path.is_file() {
                vec![
                    installed::read_path(as_path, env.lookup.os)
                        .ok_or_else(|| CliError::Failed(format!("Could not read {name}.")))?,
                ]
            } else {
                vec![found(name, env)?]
            }
        }
    };
    let mut all = Vec::new();
    for item in &items {
        let problems = installed::check(item, &env.lookup);
        let name = item.name();
        for problem in &problems {
            writeln!(out, "{problem} ({name})").map_err(write_error)?;
        }
        all.extend(problems);
    }
    writeln!(out, "{}", installed::summary(items.len(), &all)).map_err(write_error)?;
    Ok(if all.iter().any(|p| p.severity == Severity::Error) {
        1
    } else {
        0
    })
}

/// Copies a folder's files, plain names only, never through a link.
fn copy_folder(from: &Path, to: &Path) -> io::Result<usize> {
    fs::create_dir(to)?;
    let mut copied = 0;
    for item in fs::read_dir(from)? {
        let item = item?;
        let kind = item.file_type()?;
        let name = item.file_name();
        let plain = name.to_str().is_some_and(|n| {
            !n.starts_with('.')
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        });
        if !plain || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copied += copy_folder(&item.path(), &to.join(&name))?;
        } else if kind.is_file() {
            fs::copy(item.path(), to.join(&name))?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn install(path: &str, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let source = Path::new(path);
    if source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        writeln!(out, "Unpack the zip first, then install its folder.").map_err(write_error)?;
        return Ok(2);
    }
    let item = installed::read_path(source, env.lookup.os).ok_or_else(|| {
        CliError::Failed(format!(
            "Could not install {path}: it is not a plugin folder or a script."
        ))
    })?;
    let problems = installed::check(&item, &env.lookup);
    if problems.iter().any(|p| p.severity == Severity::Error) {
        for problem in &problems {
            writeln!(out, "{problem} ({})", item.name()).map_err(write_error)?;
        }
        writeln!(out, "Not installed: {} has errors.", item.name()).map_err(write_error)?;
        return Ok(1);
    }
    let target = match &item {
        Installed::Plugin(_) => env.plugins_dir().join(item.id()),
        Installed::Script(script) => {
            scripts_dir(&env.plugins_dir()).join(script.path.file_name().unwrap_or_default())
        }
    };
    if target.exists() {
        writeln!(
            out,
            "Could not install {}: it exists. Remove it first, or choose another name.",
            item.id()
        )
        .map_err(write_error)?;
        return Ok(1);
    }
    let failed = |e: io::Error| CliError::Failed(format!("Could not install {}: {e}.", item.id()));
    match &item {
        Installed::Plugin(plugin) => {
            fs::create_dir_all(env.plugins_dir()).map_err(failed)?;
            copy_folder(&plugin.folder, &target).map_err(failed)?;
        }
        Installed::Script(script) => {
            fs::create_dir_all(scripts_dir(&env.plugins_dir())).map_err(failed)?;
            fs::copy(&script.path, &target).map_err(failed)?;
        }
    }
    writeln!(out, "Installed: {}. It is not enabled.", item.name()).map_err(write_error)?;
    writeln!(out, "Next: diro plugin enable {}", item.id()).map_err(write_error)?;
    Ok(0)
}

#[cfg(test)]
mod tests;
