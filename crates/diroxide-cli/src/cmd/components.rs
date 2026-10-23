//! `diro components`: the optional components from a shell (plan section
//! 5.14, R22 section 6.4). W4c owns this file and its lines in `lib.rs`,
//! `cmd/mod.rs` and the completions.
//!
//! - `diro components list [--json]`: one line a component, the title and
//!   the state first, then the size, the license, what it is and its id.
//! - `diro components download ID [--yes]`: says what it is, its size,
//!   its license and where it comes from, asks (the answer starts on No),
//!   then downloads it; progress on standard error in plain sentences,
//!   never a spinner.
//! - `diro components verify [ID]`: hashes each installed file.
//! - `diro components remove ID [--yes]`: prints each path it will
//!   remove, asks, then removes those and nothing else.
//! - `diro components install ID PATH`: from a zip file or a folder,
//!   checked against the same pins, for a computer without a network.
//!
//! `--source FOLDER` uses that source for one run instead of the
//! `components_source` setting; `--portable` uses the settings and the
//! components beside the program. Exit codes: 0 done, 1 failed with
//! nothing changed, 2 wrong use, 3 partly done (members left out, with
//! the reasons printed).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_components::download::project_releases;
use diroxide_components::manifest::{self, Manifest};
use diroxide_components::messages::{self, ProgressVoice};
use diroxide_components::unpack::NOT_ITS_FILE;
use diroxide_components::{
    Component, Consent, Downloader, Fetcher, FreeSpace, Kind, Origin, Outcome, Source, Sources,
    StandardFetcher, Status, host_target, install_from,
};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::OpContext;

use crate::CliError;
use crate::cmd::archive::{Asker, TerminalAsker};

/// What `diro components` was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComponentsAction {
    /// List every component.
    List,
    /// Ask, then download one.
    Download(String),
    /// Check the installed files of one, or of every installed one.
    Verify(Option<String>),
    /// List one's files, ask, then remove them.
    Remove(String),
    /// Install one from a zip file or a folder.
    Install(String, String),
    /// Print the help.
    Help,
}

/// `diro components` as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentsArgs {
    /// The subcommand.
    pub action: ComponentsAction,
    /// `--yes`: the answer to the question, given ahead.
    pub yes: bool,
    /// `--json`: one JSON object a line, for `list`.
    pub json: bool,
    /// `--portable`: the settings and components beside the program.
    pub portable: bool,
    /// `--source FOLDER`: this source for one run.
    pub source: Option<String>,
}

/// The help for `diro components`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro components: optional components, asked for before anything is fetched.",
        "",
        "diro components list [--json]        each component: title, state, size, license, id",
        "diro components download ID [--yes]  say its size and license, ask, then download it",
        "diro components verify [ID]          check each installed file by size and SHA-256",
        "diro components remove ID [--yes]    list its files, ask, then remove them",
        "diro components install ID PATH      install from a zip file or a folder",
        "",
        "Options:",
        "--source FOLDER  use this folder as the components source for this run",
        "--portable       use the settings and components beside the program",
        "--yes            answer yes to the question; it is still printed",
        "",
        "Exit codes: 0 done; 1 failed, nothing changed; 2 wrong use; 3 partly done.",
    ]
    .join("\n")
}

/// Reads the arguments after `diro components`.
pub fn parse(args: &[String]) -> Result<ComponentsArgs, CliError> {
    let mut words = Vec::new();
    let mut parsed = ComponentsArgs {
        action: ComponentsAction::Help,
        yes: false,
        json: false,
        portable: false,
        source: None,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(parsed),
            "--yes" | "-y" => parsed.yes = true,
            "--json" => parsed.json = true,
            "--portable" => parsed.portable = true,
            "--source" => match rest.next() {
                Some(value) if !value.is_empty() => parsed.source = Some(value.clone()),
                _ => return Err(CliError::Missing("--source".to_owned())),
            },
            option if option.starts_with("--source=") => {
                parsed.source = Some(option["--source=".len()..].to_owned());
            }
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            word => words.push(word.to_owned()),
        }
    }
    let mut words = words.into_iter();
    let first = words.next();
    let one = |words: &mut std::vec::IntoIter<String>, name: &str| {
        words
            .next()
            .ok_or_else(|| CliError::Missing(format!("components {name}")))
    };
    parsed.action = match first.as_deref() {
        None => ComponentsAction::Help,
        Some("list") => ComponentsAction::List,
        Some("download") => ComponentsAction::Download(one(&mut words, "download")?),
        Some("verify") => ComponentsAction::Verify(words.next()),
        Some("remove") => ComponentsAction::Remove(one(&mut words, "remove")?),
        Some("install") => {
            let id = one(&mut words, "install")?;
            ComponentsAction::Install(id, one(&mut words, "install")?)
        }
        Some(other) => {
            return Err(CliError::Failed(format!(
                "diro components has no {other}; use list, download, verify, remove or install"
            )));
        }
    };
    if let Some(extra) = words.next() {
        return Err(CliError::TooMany(format!("components {extra}")));
    }
    if parsed.json && parsed.action != ComponentsAction::List {
        return Err(CliError::Failed(
            "--json goes with diro components list".to_owned(),
        ));
    }
    Ok(parsed)
}

/// What a run works with: the real folders and terminal in `diro`, folders
/// under a test folder and a scripted asker in tests.
pub struct Env<'a> {
    /// The components folder.
    pub components_dir: PathBuf,
    /// The `components_source` setting.
    pub source_setting: String,
    /// The public releases' base, `None` for a run that must not reach the
    /// network (tests).
    pub releases: Option<String>,
    /// How files are fetched.
    pub fetcher: Arc<dyn Fetcher>,
    /// The computer the files are for.
    pub target: String,
    /// The progress interval (the `progress_interval_seconds` setting).
    pub interval: Duration,
    /// Who answers the questions.
    pub asker: &'a dyn Asker,
    /// Standard error: progress and failures.
    pub err: &'a mut dyn Write,
    /// The free space, when not the volume's own (tests).
    pub space: Option<Arc<dyn FreeSpace>>,
}

/// Runs `diro components` with the real settings, tools and terminal.
pub fn run(args: &ComponentsArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.action == ComponentsAction::Help {
        writeln!(out, "{}", help_text()).map_err(write_error)?;
        return Ok(0);
    }
    let Some(paths) = crate::open::paths(args.portable) else {
        return Err(CliError::Failed(
            "Could not find the settings folder. Try --portable.".to_owned(),
        ));
    };
    let config = diroxide_config::load(&paths);
    let tools = ToolRegistry::builtin().with_overrides(config.tools.clone());
    let asterisks = config.settings.secret_echo != diroxide_config::SecretEcho::Silent;
    let asker = TerminalAsker::if_present(asterisks);
    let mut err = io::stderr();
    let env = Env {
        components_dir: paths.components_dir(),
        source_setting: config.settings.components_source.clone(),
        releases: project_releases(),
        fetcher: Arc::new(StandardFetcher::from_tools(&tools)),
        target: host_target().to_owned(),
        interval: Duration::from_secs(config.settings.progress_interval_seconds),
        asker: asker.as_ref(),
        err: &mut err,
        space: None,
    };
    run_in(args, env, out)
}

fn write_error(e: io::Error) -> CliError {
    CliError::Failed(e.to_string())
}

/// The list this run knows: the embedded one, and the source folder's,
/// whose refusals are said on standard error.
fn known(sources: &Sources, err: &mut dyn Write) -> Manifest {
    let mut list = Manifest::embedded();
    if let Some(path) = sources.list_path()
        && path.is_file()
    {
        let read = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= manifest::MAX_BYTES as u64)
            .and_then(|_| std::fs::read_to_string(&path).ok());
        match read.map(|text| manifest::parse(&text, Origin::OwnSource)) {
            Some(Ok(own)) => list.add_source(own),
            Some(Err(e)) => {
                let _ = writeln!(err, "The source's list was not read: {e}.");
            }
            None => {
                let _ = writeln!(err, "The source's list could not be read.");
            }
        }
    }
    for refusal in &list.refused {
        let _ = writeln!(err, "{}", refusal.line());
    }
    list
}

/// The kind in words, for a list row.
fn kind_words(kind: Kind) -> &'static str {
    match kind {
        Kind::Plugin => "a plugin",
        Kind::Helper => "a helper program",
        Kind::Data => "a data pack",
    }
}

/// Runs `diro components` in `env`, writing results to `out`.
pub fn run_in(args: &ComponentsArgs, env: Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let setting = args.source.clone().unwrap_or(env.source_setting.clone());
    let sources = Sources {
        source: Source::from_setting(&setting),
        releases: env.releases.clone(),
    };
    let list = known(&sources, env.err);
    let find = |id: &str| -> Result<&Component, CliError> {
        list.get(id).ok_or_else(|| {
            CliError::Failed(format!(
                "Could not find the component {id}. Run diro components list for the ids."
            ))
        })
    };
    let say = |out: &mut dyn Write, line: &str| writeln!(out, "{line}").map_err(write_error);
    match &args.action {
        ComponentsAction::Help => {
            say(out, &help_text())?;
            Ok(0)
        }
        ComponentsAction::List => list_all(&list, &env, args.json, out),
        ComponentsAction::Download(id) => {
            let c = find(id)?;
            download(c, args.yes, &sources, env, out)
        }
        ComponentsAction::Verify(id) => verify(&list, id.as_deref(), &env, out),
        ComponentsAction::Remove(id) => {
            let c = find(id)?;
            remove(c, args.yes, &env, out)
        }
        ComponentsAction::Install(id, path) => {
            let c = find(id)?;
            let from = std::path::absolute(Path::new(path)).map_err(write_error)?;
            match install_from(
                c,
                &from,
                &env.components_dir,
                &env.target,
                &OpContext::new(),
            ) {
                Ok(outcome) => finished(c, &outcome, out),
                Err(error) => {
                    for line in messages::failure(&error) {
                        let _ = writeln!(env.err, "{line}");
                    }
                    Ok(1)
                }
            }
        }
    }
}

fn list_all(
    list: &Manifest,
    env: &Env<'_>,
    json: bool,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    if list.components.is_empty() {
        writeln!(out, "No optional components are listed.").map_err(write_error)?;
        return Ok(0);
    }
    for c in &list.components {
        let status = c.status_in(&env.components_dir, &env.target);
        if json {
            let object = serde_json::json!({
                "id": c.id,
                "title": c.title,
                "kind": c.kind.word(),
                "version": c.version,
                "state": status.words(),
                "size": c.size_for(&env.target),
                "license": c.license,
                "license_note": c.license_note,
                "features": c.features,
                "origin": match c.origin {
                    Origin::Public => "public",
                    Origin::OwnSource => "own source",
                },
            });
            writeln!(out, "{object}").map_err(write_error)?;
        } else {
            writeln!(
                out,
                "{}, {}, id {}",
                messages::manage_row(c, &status, &env.target),
                kind_words(c.kind),
                c.id
            )
            .map_err(write_error)?;
        }
    }
    Ok(0)
}

/// The end of a download or an install: "Ready", then each member left
/// out; 3 when one was.
fn finished(c: &Component, outcome: &Outcome, out: &mut dyn Write) -> Result<i32, CliError> {
    writeln!(out, "{}", messages::ready(c)).map_err(write_error)?;
    let mut partly = false;
    for (name, reason) in &outcome.left_out {
        partly |= reason != NOT_ITS_FILE;
        writeln!(out, "{}", messages::left_out(name, reason)).map_err(write_error)?;
    }
    Ok(if partly { 3 } else { 0 })
}

/// Asks `question`, printing it with the answer when `yes` gave it ahead,
/// or when there is no terminal to ask (then the answer is No).
fn agreed(
    question: &str,
    yes: bool,
    asker: &dyn Asker,
    out: &mut dyn Write,
) -> Result<bool, CliError> {
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

fn download(
    c: &Component,
    yes: bool,
    sources: &Sources,
    env: Env<'_>,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    match c.status_in(&env.components_dir, &env.target) {
        Status::Installed => {
            writeln!(out, "{}", messages::already_installed(c)).map_err(write_error)?;
            return Ok(0);
        }
        Status::NoBuild => {
            for line in messages::no_build(c) {
                let _ = writeln!(env.err, "{line}");
            }
            return Ok(1);
        }
        _ => {}
    }
    let ask = messages::ask_first(c, None, &env.target);
    for line in &ask.lines {
        writeln!(out, "{line}").map_err(write_error)?;
    }
    if !agreed(&ask.question, yes, env.asker, out)? {
        writeln!(out, "{}", messages::NOT_DOWNLOADED).map_err(write_error)?;
        return Ok(1);
    }
    let mut downloader = Downloader::new(env.components_dir.clone(), sources.clone(), env.fetcher)
        .with_target(&env.target);
    if let Some(space) = env.space {
        downloader = downloader.with_space(space);
    }
    let mut voice = ProgressVoice::new(&c.id, env.interval);
    let started = Instant::now();
    let err = env.err;
    let result = downloader.download(c, &Consent::yes(c), &OpContext::new(), &mut |event| {
        if let Some(line) = voice.hear(event, started.elapsed()) {
            let _ = writeln!(err, "{line}");
        }
    });
    match result {
        Ok(outcome) => finished(c, &outcome, out),
        Err(error) => {
            for line in messages::failure(&error) {
                let _ = writeln!(err, "{line}");
            }
            Ok(1)
        }
    }
}

fn verify(
    list: &Manifest,
    id: Option<&str>,
    env: &Env<'_>,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let chosen: Vec<&Component> = match id {
        Some(id) => match list.get(id) {
            Some(c) => vec![c],
            None => {
                return Err(CliError::Failed(format!(
                    "Could not find the component {id}. Run diro components list for the ids."
                )));
            }
        },
        None => list
            .components
            .iter()
            .filter(|c| {
                !matches!(
                    c.status_in(&env.components_dir, &env.target),
                    Status::NotInstalled | Status::NoBuild
                )
            })
            .collect(),
    };
    if chosen.is_empty() {
        writeln!(out, "No component is installed.").map_err(write_error)?;
        return Ok(0);
    }
    let mut code = 0;
    for c in chosen {
        if matches!(
            c.status_in(&env.components_dir, &env.target),
            Status::NotInstalled | Status::NoBuild
        ) {
            writeln!(out, "Not installed: {}.", c.title).map_err(write_error)?;
            code = 1;
            continue;
        }
        let checked = c.verify_in(&env.components_dir, &env.target);
        let lines = messages::verified(c, &checked);
        if lines.len() > 1 {
            code = 1;
        }
        for line in lines {
            writeln!(out, "{line}").map_err(write_error)?;
        }
    }
    Ok(code)
}

fn remove(c: &Component, yes: bool, env: &Env<'_>, out: &mut dyn Write) -> Result<i32, CliError> {
    let removal = c.removal_in(&env.components_dir);
    if removal.is_empty() {
        writeln!(out, "Nothing to remove: {} is not installed.", c.title).map_err(write_error)?;
        return Ok(1);
    }
    for line in removal.lines() {
        writeln!(out, "{line}").map_err(write_error)?;
    }
    writeln!(out, "{}", removal.summary()).map_err(write_error)?;
    if !agreed(&messages::remove_question(c), yes, env.asker, out)? {
        writeln!(out, "Nothing was removed.").map_err(write_error)?;
        return Ok(1);
    }
    match removal.run() {
        Ok(_) => {
            writeln!(out, "Removed: {}.", c.title).map_err(write_error)?;
            Ok(0)
        }
        Err(error) => Err(CliError::Failed(format!(
            "Could not remove {}: {error}.",
            c.title
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_components::fake::FakeFetcher;
    use diroxide_components::sha256_hex;
    use diroxide_core::Secret;
    use std::fs;
    use std::sync::Mutex;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Answers each question with the next scripted answer; records them.
    struct Script {
        answers: Mutex<Vec<bool>>,
        asked: Mutex<Vec<String>>,
    }

    impl Script {
        fn new(answers: &[bool]) -> Self {
            Self {
                answers: Mutex::new(answers.iter().rev().copied().collect()),
                asked: Mutex::new(Vec::new()),
            }
        }
    }

    impl Asker for Script {
        fn present(&self) -> bool {
            true
        }
        fn confirm(&self, question: &str) -> bool {
            self.asked.lock().unwrap().push(question.to_owned());
            self.answers.lock().unwrap().pop().unwrap_or(false)
        }
        fn secret(&self, _prompt: &str) -> Option<Secret> {
            None
        }
        fn tell(&self, _line: &str) {}
    }

    /// A source folder with its own list: a data file and an unpacked
    /// pack, each in `<id>-<version>/`.
    fn source(dir: &Path) -> PathBuf {
        let folder = dir.join("source");
        fs::create_dir_all(folder.join("words-1.0")).unwrap();
        fs::write(folder.join("words-1.0").join("words.txt"), b"some words").unwrap();
        let list = format!(
            r#"format = 1

[[component]]
id = "words"
title = "Word list"
kind = "data"
version = "1.0"
license = "CC0-1.0"

[[component.file]]
name = "words.txt"
size = 10
sha256 = "{}"
"#,
            sha256_hex(b"some words")
        );
        fs::write(folder.join("components.toml"), list).unwrap();
        folder
    }

    struct Run {
        code: i32,
        out: String,
        err: String,
    }

    fn run_with(dir: &Path, typed: &[&str], setting: &str, asker: &dyn Asker) -> Run {
        let parsed = parse(&args(typed)).unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let env = Env {
            components_dir: dir.join("state").join("components"),
            source_setting: setting.to_owned(),
            releases: None,
            fetcher: Arc::new(StandardFetcher::local_only()),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            interval: Duration::ZERO,
            asker,
            err: &mut err,
            space: None,
        };
        let code = run_in(&parsed, env, &mut out).unwrap();
        Run {
            code,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    #[test]
    fn arguments_are_read() {
        let p = |list: &[&str]| parse(&args(list));
        assert_eq!(p(&["list"]).unwrap().action, ComponentsAction::List);
        assert!(p(&["list", "--json"]).unwrap().json);
        let d = p(&["download", "diro-examples", "--yes", "--source", "D:/src"]).unwrap();
        assert_eq!(d.action, ComponentsAction::Download("diro-examples".into()));
        assert!(d.yes);
        assert_eq!(d.source.as_deref(), Some("D:/src"));
        assert_eq!(
            p(&["verify"]).unwrap().action,
            ComponentsAction::Verify(None)
        );
        assert_eq!(
            p(&["install", "x", "a.zip", "--portable"]).unwrap(),
            ComponentsArgs {
                action: ComponentsAction::Install("x".into(), "a.zip".into()),
                yes: false,
                json: false,
                portable: true,
                source: None,
            }
        );
        assert_eq!(p(&[]).unwrap().action, ComponentsAction::Help);
        assert_eq!(
            p(&["download"]),
            Err(CliError::Missing("components download".into()))
        );
        assert_eq!(
            p(&["install", "x"]),
            Err(CliError::Missing("components install".into()))
        );
        assert_eq!(
            p(&["list", "--bogus"]),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert_eq!(
            p(&["remove", "a", "b"]),
            Err(CliError::TooMany("components b".into()))
        );
        assert!(matches!(p(&["fetch"]), Err(CliError::Failed(_))));
        assert!(matches!(p(&["verify", "--json"]), Err(CliError::Failed(_))));
    }

    #[test]
    fn download_yes_from_a_source_folder_then_verify_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = source(tmp.path());
        let setting = folder.display().to_string();
        let nobody = Script::new(&[]);
        let listed = run_with(tmp.path(), &["list"], &setting, &nobody);
        assert_eq!(
            listed.out,
            "Word list: not installed, 10 bytes, license CC0-1.0, a data pack, id words\n"
        );
        let got = run_with(
            tmp.path(),
            &["download", "words", "--yes"],
            &setting,
            &nobody,
        );
        assert_eq!(got.code, 0, "{}", got.err);
        assert_eq!(
            got.out,
            "Word list 1.0, words.\nSize 10 bytes, license CC0-1.0.\nFrom your own components source.\nDownload it now? y or n yes, from --yes\nReady: words 1.0.\n"
        );
        assert!(
            got.err.starts_with("Downloading words, 10 bytes.\n"),
            "{}",
            got.err
        );
        assert!(got.err.contains("Checking the files.\n"));
        assert!(
            !got.out.contains("Downloading"),
            "progress goes to standard error only"
        );
        let again = run_with(tmp.path(), &["download", "words"], &setting, &nobody);
        assert_eq!(again.out, "Already installed: words 1.0.\n");
        let checked = run_with(tmp.path(), &["verify"], &setting, &nobody);
        assert_eq!(
            (checked.code, checked.out.as_str()),
            (0, "Files check out: Word list.\n")
        );
        let file = tmp
            .path()
            .join("state")
            .join("components")
            .join("words")
            .join("words.txt");
        fs::write(&file, b"SOME WORDS").unwrap();
        let bad = run_with(tmp.path(), &["verify", "words"], &setting, &nobody);
        assert_eq!(bad.code, 1);
        assert_eq!(
            bad.out,
            "1 file does not check out: Word list.\nDoes not match: words.txt\n"
        );
        let removed = run_with(tmp.path(), &["remove", "words", "--yes"], &setting, &nobody);
        assert_eq!(removed.code, 0);
        assert_eq!(
            removed.out,
            format!(
                "{}\nRemoves 1 file.\nRemove Word list? y or n yes, from --yes\nRemoved: Word list.\n",
                file.display()
            )
        );
        assert!(!file.exists());
    }

    #[test]
    fn without_a_yes_nothing_is_fetched() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = source(tmp.path());
        let setting = folder.display().to_string();
        let no_terminal = crate::cmd::archive::NoAsker;
        let got = run_with(tmp.path(), &["download", "words"], &setting, &no_terminal);
        assert_eq!(got.code, 1);
        assert!(got.out.ends_with(
            "Download it now? y or n\nAnswered no: there is no terminal to ask.\nNot downloaded.\n"
        ));
        assert!(!tmp.path().join("state").exists(), "nothing was written");
        let script = Script::new(&[false]);
        let said_no = run_with(tmp.path(), &["download", "words"], &setting, &script);
        assert_eq!(
            (said_no.code, said_no.out.ends_with("Not downloaded.\n")),
            (1, true)
        );
        assert_eq!(*script.asked.lock().unwrap(), ["Download it now? y or n"]);
        let script = Script::new(&[true]);
        assert_eq!(
            run_with(tmp.path(), &["download", "words"], &setting, &script).code,
            0
        );
    }

    #[test]
    fn the_source_option_beats_the_setting_for_one_run() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = source(tmp.path());
        let nobody = Script::new(&[]);
        let without = run_with(tmp.path(), &["list"], "", &nobody);
        assert!(!without.out.contains("Word list"));
        let flag = format!("--source={}", folder.display());
        let with = run_with(tmp.path(), &["list", &flag], "", &nobody);
        assert!(with.out.starts_with("Word list: not installed"));
        let json = run_with(tmp.path(), &["list", "--json", &flag], "", &nobody);
        let object: serde_json::Value =
            serde_json::from_str(json.out.lines().next().unwrap()).unwrap();
        assert_eq!(object["id"], "words");
        assert_eq!(object["state"], "not installed");
        assert_eq!(object["size"], 10);
        assert_eq!(object["origin"], "own source");
    }

    #[test]
    fn a_missing_source_file_fails_with_nothing_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = source(tmp.path());
        fs::remove_file(folder.join("words-1.0").join("words.txt")).unwrap();
        let nobody = Script::new(&[]);
        let got = run_with(
            tmp.path(),
            &["download", "words", "--yes"],
            &folder.display().to_string(),
            &nobody,
        );
        assert_eq!(got.code, 1);
        assert!(
            got.err.contains("Not downloaded: the file is not there.\n"),
            "{}",
            got.err
        );
        assert!(
            !tmp.path()
                .join("state")
                .join("components")
                .join("words")
                .exists()
        );
    }

    #[test]
    fn install_from_a_folder_and_unknown_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = source(tmp.path());
        let setting = folder.display().to_string();
        let nobody = Script::new(&[]);
        let stick = tmp.path().join("stick");
        fs::create_dir_all(&stick).unwrap();
        fs::write(stick.join("words.txt"), b"some words").unwrap();
        let stick_arg = stick.display().to_string();
        let got = run_with(
            tmp.path(),
            &["install", "words", &stick_arg],
            &setting,
            &nobody,
        );
        assert_eq!((got.code, got.out.as_str()), (0, "Ready: words 1.0.\n"));
        let parsed = parse(&args(&["download", "nope"])).unwrap();
        let mut err = Vec::new();
        let env = Env {
            components_dir: tmp.path().join("c"),
            source_setting: setting,
            releases: None,
            fetcher: Arc::new(FakeFetcher::new()),
            target: "x".into(),
            interval: Duration::ZERO,
            asker: &nobody,
            err: &mut err,
            space: None,
        };
        let error = run_in(&parsed, env, &mut Vec::new()).unwrap_err();
        assert_eq!(
            error,
            CliError::Failed(
                "Could not find the component nope. Run diro components list for the ids.".into()
            )
        );
    }

    #[test]
    fn a_component_with_no_build_here_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = source(tmp.path());
        let list = fs::read_to_string(folder.join("components.toml")).unwrap();
        fs::write(
            folder.join("components.toml"),
            list.replace("size = 10", "size = 10\ntarget = \"aarch64-apple-darwin\""),
        )
        .unwrap();
        let nobody = Script::new(&[]);
        let got = run_with(
            tmp.path(),
            &["download", "words", "--yes"],
            &folder.display().to_string(),
            &nobody,
        );
        assert_eq!(got.code, 1);
        assert!(
            got.err.starts_with("No build for this computer.\n"),
            "{}",
            got.err
        );
    }

    #[test]
    fn every_help_line_puts_the_command_first() {
        for line in help_text()
            .lines()
            .filter(|l| l.starts_with("diro components "))
        {
            assert!(
                diroxide_core::format::within_cells(line, "diro components"),
                "{line}"
            );
        }
    }
}
