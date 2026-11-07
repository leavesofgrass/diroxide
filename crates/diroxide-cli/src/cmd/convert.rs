//! `diro convert FILE... --to FORMAT`: conversions with recipes, carta's
//! writers first, one line per result, the name first, the summary last;
//! exit codes 0 all done, 1 nothing could be done, 2 wrong use, 3 partly
//! done (alpha.7, R28 5.3; R30 commit 63, lane AW).
//!
//! - `--to FORMAT` takes an extension or a format's words ("docx",
//!   "Word") and picks, for each file, the first recipe that takes it and
//!   makes that: carta's first, then the recipe file's order. `--recipe
//!   ID` forces one. A folder is converted one level, or its whole tree
//!   with `--recursive`, which mirrors the tree into the target.
//! - The outputs go to `--out FOLDER`, or beside each file. A taken name
//!   gets a number ("report (2).pdf"); `--conflicts skip` or `newer`
//!   instead. Nothing is ever replaced.
//! - Standard output gets one line a result ("Made: notes.org", "Failed:
//!   a.heic, ImageMagick made no file", "Skipped: b.png, output exists"),
//!   then the summary ("Report: 1 failed, 2 converted"). Progress words
//!   go to standard error at most every ten seconds, never a spinner.
//!   `--print0` writes only the made files' paths, NUL-separated, for
//!   `xargs -0`; `--json` one JSON object a line.
//! - `--dry-run` prints each plan and writes nothing. `--report FILE`
//!   writes the report as Markdown, names only, never a full path.
//! - `--list [FILE]` prints the recipes, one a line, the id first, then
//!   what it does, the tool, and "ready" or what it needs.
//!
//! A carta recipe runs in a `diro` child the engine can stop.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use diroxide_app::conversions::{RegistryTyping, recipe_book, report_shelf};
use diroxide_config::Paths;
use diroxide_core::{Clock, ConflictAnswer, ConflictPolicy, EntryKind, ErrorAction, JobId, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_ops::convert::{
    Availability, CartaRunner, ConvertReport, ConvertRequest, Recipe, RecipeBook, Typing,
};
use diroxide_ops::{Conflict, Engine, ItemError, JobControl, JobProgress, JobSink};
use diroxide_vfs::{CancelToken, OpContext, Registry, ToolRunner};

use crate::CliError;

/// How often progress words are written to standard error.
const PROGRESS_EVERY: Duration = Duration::from_secs(10);

/// `diro convert`'s arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertArgs {
    /// The files and folders, as typed.
    pub files: Vec<String>,
    /// The format to make: an extension or a format's words.
    pub to: Option<String>,
    /// The recipe to use, by id, instead of the one `--to` picks.
    pub recipe: Option<String>,
    /// The folder the outputs go to; beside each file by default.
    pub out: Option<String>,
    /// What to do when an output's name is taken: keep both by default.
    pub conflicts: ConflictPolicy,
    /// Convert folders with everything below them.
    pub recursive: bool,
    /// Say each plan and write nothing.
    pub dry_run: bool,
    /// Write the report as Markdown to this file.
    pub report: Option<String>,
    /// One JSON object a line.
    pub json: bool,
    /// The made files' paths only, NUL-separated.
    pub print0: bool,
    /// List the recipes.
    pub list: bool,
    /// Print the help.
    pub help: bool,
}

impl Default for ConvertArgs {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            to: None,
            recipe: None,
            out: None,
            conflicts: ConflictPolicy::KeepBoth,
            recursive: false,
            dry_run: false,
            report: None,
            json: false,
            print0: false,
            list: false,
            help: false,
        }
    }
}

/// The help for `diro convert`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro convert: convert files with recipes, one line a result.",
        "",
        "Usage:",
        "diro convert FILE... --to FORMAT  convert to an extension or format",
        "diro convert FILE... --recipe ID  convert with this recipe",
        "diro convert --list [FILE]        list the recipes, or those for FILE",
        "",
        "Options:",
        "--out FOLDER          put the outputs here; beside each file by default",
        "--conflicts WHICH     a taken name: keep-both (the default), skip or newer",
        "--recursive           convert folders with everything below them",
        "--dry-run             say the plan and write nothing",
        "--report FILE         write the report as Markdown, names only",
        "--json                one JSON object a line",
        "--print0              write the made files' paths only, NUL-separated",
        "",
        "Nothing is ever replaced: a taken name gets a number.",
        "",
        "Exit codes:",
        "0  all done",
        "1  nothing could be done: a missing tool, no recipe",
        "2  wrong use: a bad option",
        "3  partly done: some files failed; the lines say which",
    ]
    .join("\n")
}

/// Reads the arguments after `convert`.
pub fn parse(args: &[String]) -> Result<ConvertArgs, CliError> {
    let mut parsed = ConvertArgs::default();
    let mut only_files = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_files {
            parsed.files.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |flag: &str| -> Result<String, CliError> {
            match inline.clone().or_else(|| rest.next().cloned()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(CliError::Missing(flag.to_owned())),
            }
        };
        match name {
            "--" => only_files = true,
            "--help" | "-h" => parsed.help = true,
            "--to" => parsed.to = Some(value("--to")?),
            "--recipe" => parsed.recipe = Some(value("--recipe")?),
            "--out" => parsed.out = Some(value("--out")?),
            "--report" => parsed.report = Some(value("--report")?),
            "--conflicts" => {
                let which = value("--conflicts")?;
                parsed.conflicts = match which.as_str() {
                    "keep-both" => ConflictPolicy::KeepBoth,
                    "skip" => ConflictPolicy::Skip,
                    "newer" => ConflictPolicy::Newer,
                    other => {
                        return Err(CliError::UnknownOption(format!("--conflicts {other}")));
                    }
                };
            }
            "--recursive" | "-r" => parsed.recursive = true,
            "--dry-run" => parsed.dry_run = true,
            "--json" => parsed.json = true,
            "--print0" | "--null" | "-0" => parsed.print0 = true,
            "--list" => parsed.list = true,
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            file => parsed.files.push(file.to_owned()),
        }
    }
    if parsed.help {
        return Ok(parsed);
    }
    if parsed.list {
        if parsed.files.len() > 1 {
            return Err(CliError::TooMany("convert --list".to_owned()));
        }
        return Ok(parsed);
    }
    if parsed.files.is_empty() {
        return Err(CliError::Missing("convert".to_owned()));
    }
    if parsed.to.is_none() && parsed.recipe.is_none() {
        return Err(CliError::Missing("convert --to".to_owned()));
    }
    if parsed.json && parsed.print0 {
        return Err(CliError::TooMany("convert --json".to_owned()));
    }
    Ok(parsed)
}

/// What a conversion runs with: the current folder, the config and state
/// folders, the helper programs, the file types, the clock, where carta
/// runs, and whether progress is written.
pub struct ConvertEnv {
    /// The current folder, for relative paths.
    pub here: Place,
    /// The config and state folders: the owner's recipes, the spool and
    /// the kept reports.
    pub paths: Option<Paths>,
    /// The helper programs.
    pub tools: Arc<dyn ToolRunner>,
    /// The file types.
    pub types: Arc<TypeRegistry>,
    /// The clock, for the report's time.
    pub clock: Arc<dyn Clock>,
    /// Where carta runs: a `diro` child in the program, in process in
    /// tests.
    pub carta: CartaRunner,
    /// Whether progress words go to standard error.
    pub progress: bool,
}

/// Runs `diro convert` on this computer: the real folders, the tools
/// registry, the file types with the owner's, the local clock, and carta
/// in a `diro` child.
pub fn run(args: &ConvertArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(|e| CliError::Failed(e.to_string()))?;
        return Ok(0);
    }
    let paths = crate::open::paths(false);
    let config = paths
        .as_ref()
        .map(diroxide_config::load)
        .unwrap_or_default();
    let types = TypeRegistry::builtin()
        .with_config(&config.filetypes)
        .with_retro(config.settings.filetypes.retro);
    let carta = std::env::current_exe()
        .map(CartaRunner::child)
        .unwrap_or_default();
    let env = ConvertEnv {
        here: crate::open::current_place()?,
        paths,
        tools: Arc::new(super::archive::tools_here()),
        types: Arc::new(types),
        clock: Arc::new(diroxide_app::startup::local_clock()),
        carta,
        progress: std::io::IsTerminal::is_terminal(&std::io::stderr()),
    };
    run_with(args, out, &env)
}

/// Writes one line, or fails in words.
fn line(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    writeln!(out, "{text}").map_err(|e| CliError::Failed(e.to_string()))
}

/// A place's name as the lines say it.
fn name_of(place: &Place) -> String {
    place.name().map_or_else(
        || place.to_string(),
        |n| diroxide_core::names::display_name(n).into_owned(),
    )
}

/// The extension of a name, lowercase, without its dot.
fn extension(name: &str) -> Option<String> {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => Some(ext.to_ascii_lowercase()),
        _ => None,
    }
}

/// The first bytes of a local file, for its type; empty when it cannot be
/// read.
fn head_of(place: &Place) -> Vec<u8> {
    let mut head = vec![0u8; diroxide_ops::convert::typing::HEAD_BYTES];
    let read = place
        .local_path()
        .and_then(|path| std::fs::File::open(path).ok())
        .and_then(|mut file| file.read(&mut head).ok())
        .unwrap_or(0);
    head.truncate(read);
    head
}

/// The recipe `--to` picks for one file.
fn pick<'a>(
    book: &'a RecipeBook,
    to: &str,
    place: &Place,
    typing: &dyn Typing,
) -> Option<&'a Recipe> {
    let name = place.name().unwrap_or_default();
    let kind = typing.kind_of(name, &head_of(place));
    book.pick_for(
        to,
        extension(name).as_deref(),
        kind.type_id.as_deref(),
        &kind.groups,
    )
}

/// `--list`: one line a recipe, the id first, then what it does, the
/// tool and whether it can run here; with a file, only the recipes that
/// take it.
fn list(
    book: &RecipeBook,
    file: Option<&Place>,
    env: &ConvertEnv,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let typing = RegistryTyping(Arc::clone(&env.types));
    let taken = file.map(|place| {
        let name = place.name().unwrap_or_default().to_owned();
        let kind = typing.kind_of(&name, &head_of(place));
        (extension(&name), kind)
    });
    for (recipe, state) in book.availability(env.tools.as_ref(), &CancelToken::new()) {
        if let Some((ext, kind)) = &taken
            && !recipe.takes(ext.as_deref(), kind.type_id.as_deref(), &kind.groups)
        {
            continue;
        }
        let tool = if recipe.tool == "carta" {
            "carta, built in".to_owned()
        } else {
            env.tools.name_of(&recipe.tool)
        };
        let state = match state {
            Availability::Ready => "ready".to_owned(),
            Availability::NeedsTool { name } => format!("needs {name}"),
            Availability::NeedsEncoder { encoder } => format!("needs {encoder}"),
        };
        line(
            out,
            &format!("{}: {}, {tool}, {state}", recipe.id, recipe.name),
        )?;
    }
    for problem in book.problems() {
        eprintln!("{problem}");
    }
    Ok(0)
}

/// Progress words on standard error at most every ten seconds, the
/// plan's own ("Converting 12 of 40 images"); a conflict is never asked
/// (the policy answers it) and an item's error never stops the rest.
struct StderrSink {
    on: bool,
    last: Mutex<Option<Instant>>,
    plan: Mutex<Option<diroxide_ops::convert::ConvertPlan>>,
}

impl JobSink for StderrSink {
    fn progress(&self, progress: &JobProgress) {
        if !self.on {
            return;
        }
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < PROGRESS_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        if let Some(plan) = self
            .plan
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            eprintln!("{}", plan.progress_line(progress));
        }
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

/// The counts of a whole run, over every plan.
#[derive(Default)]
struct Tally {
    made: usize,
    failed: usize,
    skipped: usize,
    refused: usize,
    would: usize,
}

impl Tally {
    /// "Report: 1 failed, 2 converted, 1 skipped", what went wrong first.
    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        parts.push(format!("{} converted", self.made));
        if self.refused > 0 {
            parts.push(format!("{} refused", self.refused));
        }
        if self.skipped > 0 {
            parts.push(format!("{} skipped", self.skipped));
        }
        format!("Report: {}", parts.join(", "))
    }
}

/// One result, as a line or a JSON object.
fn result(
    out: &mut dyn Write,
    args: &ConvertArgs,
    verdict: &str,
    name: &str,
    detail: Option<&str>,
) -> Result<(), CliError> {
    if args.print0 {
        return Ok(());
    }
    if args.json {
        let mut object = serde_json::json!({ "result": verdict.to_lowercase(), "name": name });
        if let Some(detail) = detail {
            object["detail"] = serde_json::Value::String(detail.to_owned());
        }
        return line(out, &object.to_string());
    }
    match detail {
        Some(detail) => line(out, &format!("{verdict}: {name}, {detail}")),
        None => line(out, &format!("{verdict}: {name}")),
    }
}

/// Runs `diro convert` with these surroundings. See the module
/// documentation.
pub fn run_with(
    args: &ConvertArgs,
    out: &mut dyn Write,
    env: &ConvertEnv,
) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    let book = recipe_book(env.paths.as_ref());
    let resolve = |text: &str| {
        Place::resolve(text, &env.here).map_err(|e| {
            CliError::Failed(
                diroxide_core::Message::from_error(
                    "convert",
                    &diroxide_core::names::display_name(text),
                    &e,
                )
                .text,
            )
        })
    };
    if args.list {
        let file = args.files.first().map(|f| resolve(f)).transpose()?;
        return list(&book, file.as_ref(), env, out);
    }
    let (registry, _archives) = super::archive::archive_registry();
    let engine = Engine::new(Arc::clone(&registry)).with_tools(Arc::clone(&env.tools));
    let typing = RegistryTyping(Arc::clone(&env.types));
    let forced = match &args.recipe {
        Some(id) => match book.get(id) {
            Some(recipe) => Some(recipe.clone()),
            None => {
                line(out, &format!("Could not convert: no recipe is named {id}."))?;
                return Ok(1);
            }
        },
        None => None,
    };
    let target = args.out.as_deref().map(resolve).transpose()?;
    let mut tally = Tally::default();
    let mut could_not = 0usize;
    // The sources by recipe and target folder, in the order first met.
    let mut groups: Vec<(Recipe, Place, Vec<Place>)> = Vec::new();
    for text in &args.files {
        let place = resolve(text)?;
        let name = name_of(&place);
        let entry = registry
            .resolve(&place)
            .and_then(|p| p.stat(&place, &OpContext::new()));
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                result(out, args, "Failed", &name, Some(&error.to_string()))?;
                tally.failed += 1;
                could_not += 1;
                continue;
            }
        };
        let folder = entry.kind == EntryKind::Folder;
        let recipe = match (&forced, &args.to) {
            (Some(recipe), _) => Some(recipe.clone()),
            (None, Some(to)) if folder => folder_recipe(&book, to, &place, &registry, &typing),
            (None, Some(to)) => pick(&book, to, &place, &typing).cloned(),
            (None, None) => None,
        };
        let Some(recipe) = recipe else {
            let to = args.to.clone().unwrap_or_default();
            result(
                out,
                args,
                "Failed",
                &name,
                Some(&format!("no recipe makes {to} from it")),
            )?;
            tally.failed += 1;
            could_not += 1;
            continue;
        };
        let destination = match &target {
            Some(folder) => folder.clone(),
            None => place.parent().unwrap_or_else(|| env.here.clone()),
        };
        match groups
            .iter_mut()
            .find(|(r, d, _)| r.id == recipe.id && *d == destination)
        {
            Some((_, _, sources)) => sources.push(place),
            None => groups.push((recipe, destination, vec![place])),
        }
    }
    let mut reports: Vec<ConvertReport> = Vec::new();
    let mut made_paths: Vec<String> = Vec::new();
    for (n, (recipe, destination, sources)) in groups.into_iter().enumerate() {
        let job = JobId::new(n as u64 + 1);
        let mut request = ConvertRequest::new(job, sources, destination, recipe)
            .conflicts(args.conflicts)
            .recursive(args.recursive)
            .carta(env.carta.clone());
        if let Some(paths) = &env.paths {
            request = request.spool(paths.spool_dir());
        }
        let control = JobControl::new();
        let plan = match engine.plan_convert(&request, &typing, &control) {
            Ok(plan) => plan,
            Err(error) => {
                line(out, &format!("Could not convert: {error}."))?;
                could_not += request.sources.len();
                continue;
            }
        };
        if args.dry_run {
            if !args.print0 {
                for row in plan.lines() {
                    line(out, &row)?;
                }
            }
            tally.would += plan.work.len();
            tally.skipped += plan.skipped.len();
            tally.refused += plan.refused.len();
            continue;
        }
        let sink = StderrSink {
            on: env.progress,
            last: Mutex::new(None),
            plan: Mutex::new(Some(plan.clone())),
        };
        let mut report = engine.run_convert(&plan, &control, &sink);
        report.stamp(env.clock.as_ref());
        for (place, cause) in &report.job.failed {
            result(out, args, "Failed", &name_of(place), Some(cause))?;
        }
        for (place, reason) in &report.refused {
            result(out, args, "Refused", &name_of(place), Some(reason))?;
        }
        for (place, reason) in &report.job.skipped {
            result(out, args, "Skipped", &name_of(place), Some(reason))?;
        }
        for (_, output) in &report.made {
            result(out, args, "Made", &name_of(output), None)?;
            if let Some(path) = output.local_path() {
                made_paths.push(path.to_owned());
            }
        }
        tally.made += report.made.len();
        tally.failed += report.job.failed.len();
        tally.refused += report.refused.len();
        tally.skipped += report.job.skipped.len();
        if let Some(paths) = &env.paths {
            // A shelf that cannot be written never fails the run.
            let _ = report_shelf(paths).keep(&report.kept(env.clock.as_ref()));
        }
        reports.push(report);
    }
    if args.print0 {
        for path in &made_paths {
            write!(out, "{path}\0").map_err(|e| CliError::Failed(e.to_string()))?;
        }
    }
    if let Some(file) = &args.report
        && !args.dry_run
    {
        let path = resolve(file)?;
        let text: Vec<String> = reports
            .iter()
            .map(|r| r.markdown(env.clock.as_ref()))
            .collect();
        let written = path
            .local_path()
            .map(|p| std::fs::write(p, text.join("\n")))
            .unwrap_or_else(|| Err(std::io::Error::other("it is not on this computer")));
        if let Err(error) = written {
            let line_text = format!("Could not write the report {}: {error}.", name_of(&path));
            if args.print0 {
                eprintln!("{line_text}");
            } else {
                line(out, &line_text)?;
            }
        }
    }
    let summary = if args.dry_run {
        let not = tally.skipped + tally.refused;
        if not == 0 {
            format!("Dry run: {} would convert", tally.would)
        } else {
            format!("Dry run: {} would convert, {not} skipped", tally.would)
        }
    } else {
        tally.summary()
    };
    if args.print0 {
        eprintln!("{summary}");
    } else if args.json {
        line(out, &serde_json::json!({ "summary": summary }).to_string())?;
    } else {
        line(out, &summary)?;
    }
    let trouble = could_not + tally.failed.saturating_sub(could_not) + tally.refused;
    Ok(if args.dry_run {
        if tally.would == 0 && trouble > 0 {
            1
        } else {
            0
        }
    } else if trouble == 0 {
        0
    } else if tally.made == 0 {
        1
    } else {
        3
    })
}

/// The recipe `--to` picks for a folder: the one that takes the most of
/// its files, one level down.
fn folder_recipe(
    book: &RecipeBook,
    to: &str,
    folder: &Place,
    registry: &Registry,
    typing: &dyn Typing,
) -> Option<Recipe> {
    let entries = registry
        .resolve(folder)
        .and_then(|p| p.list(folder, &OpContext::new()))
        .ok()?;
    let mut counts: Vec<(String, usize)> = Vec::new();
    for entry in entries.iter().filter(|e| e.kind == EntryKind::File) {
        let Ok(place) = folder.join(&entry.name) else {
            continue;
        };
        if let Some(recipe) = pick(book, to, &place, typing) {
            match counts.iter_mut().find(|(id, _)| *id == recipe.id) {
                Some((_, n)) => *n += 1,
                None => counts.push((recipe.id.clone(), 1)),
            }
        }
    }
    let best = counts
        .iter()
        .max_by_key(|(_, n)| *n)
        .map(|(id, _)| id.clone())?;
    book.get(&best).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{FixedClock, Timestamp, UtcOffset};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn env(here: &std::path::Path, paths: Option<Paths>) -> ConvertEnv {
        ConvertEnv {
            here: Place::parse(here.to_str().unwrap()).unwrap(),
            paths,
            tools: Arc::new(diroxide_vfs::NoTools),
            types: Arc::new(TypeRegistry::builtin()),
            clock: Arc::new(FixedClock::new(
                Timestamp::from_civil(2026, 11, 7, 9, 0, 0, UtcOffset::UTC).unwrap(),
                UtcOffset::UTC,
            )),
            carta: CartaRunner::InProcess,
            progress: false,
        }
    }

    fn run_in(env: &ConvertEnv, list: &[&str]) -> (i32, String) {
        let parsed = parse(&args(list)).unwrap();
        let mut out = Vec::new();
        let code = run_with(&parsed, &mut out, env).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("convert-")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("notes.md"), "# Notes\n\nOne line.\n").unwrap();
        std::fs::write(dir.path().join("more.md"), "# More\n\nTwo lines.\n").unwrap();
        std::fs::create_dir_all(dir.path().join("out")).unwrap();
        dir
    }

    #[test]
    fn the_options_are_read_and_wrong_use_is_said() {
        let parsed = parse(&args(&[
            "a.md",
            "--to",
            "org",
            "--out=out",
            "--conflicts",
            "skip",
            "--recursive",
            "--dry-run",
            "--report",
            "r.md",
        ]))
        .unwrap();
        assert_eq!(parsed.files, ["a.md"]);
        assert_eq!(parsed.to.as_deref(), Some("org"));
        assert_eq!(parsed.out.as_deref(), Some("out"));
        assert_eq!(parsed.conflicts, ConflictPolicy::Skip);
        assert!(parsed.recursive && parsed.dry_run);
        assert_eq!(
            parse(&args(&["a.md"])),
            Err(CliError::Missing("convert --to".into()))
        );
        assert_eq!(
            parse(&args(&["--to", "org"])),
            Err(CliError::Missing("convert".into()))
        );
        assert_eq!(
            parse(&args(&["a.md", "--to", "org", "--conflicts", "overwrite"])),
            Err(CliError::UnknownOption("--conflicts overwrite".into()))
        );
        assert_eq!(
            parse(&args(&["a.md", "--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert!(parse(&args(&["--list"])).unwrap().list);
        for row in help_text().lines() {
            assert!(row.chars().count() <= 80, "{row}");
        }
    }

    #[test]
    fn two_files_convert_with_one_line_each_and_the_summary_last() {
        let dir = folder();
        let env = env(dir.path(), None);
        let (code, text) = run_in(
            &env,
            &["notes.md", "more.md", "--to", "org", "--out", "out"],
        );
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            ["Made: notes.org", "Made: more.org", "Report: 2 converted"]
        );
        for row in &lines {
            assert!(row.chars().count() <= 40, "{row}");
        }
        assert!(dir.path().join("out").join("notes.org").exists());
        // Again: the names are taken, so each gets a number; nothing is
        // replaced.
        let (code, text) = run_in(&env, &["notes.md", "--to", "org", "--out", "out"]);
        assert_eq!(code, 0, "{text}");
        assert!(text.starts_with("Made: notes (2).org\n"), "{text}");
        // Skip leaves the taken name.
        let (code, text) = run_in(
            &env,
            &[
                "notes.md",
                "--to",
                "org",
                "--out",
                "out",
                "--conflicts",
                "skip",
            ],
        );
        assert_eq!(code, 0, "{text}");
        assert!(
            text.starts_with("Skipped: notes.md, output exists"),
            "{text}"
        );
    }

    #[test]
    fn a_dry_run_writes_nothing_and_print0_writes_paths_only() {
        let dir = folder();
        let env = env(dir.path(), None);
        let (code, text) = run_in(&env, &["notes.md", "--to", "org", "--dry-run"]);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("notes.md to notes.org\n"), "{text}");
        assert!(text.ends_with("Dry run: 1 would convert\n"), "{text}");
        assert!(!dir.path().join("notes.org").exists());
        let (code, text) = run_in(&env, &["notes.md", "--to", "org", "--print0"]);
        assert_eq!(code, 0);
        let made = dir.path().join("notes.org");
        assert_eq!(text, format!("{}\0", made.to_str().unwrap()));
    }

    #[test]
    fn nothing_to_do_exits_one_and_part_done_exits_three() {
        let dir = folder();
        std::fs::write(dir.path().join("blob.qqq"), "x").unwrap();
        let env = env(dir.path(), None);
        let (code, text) = run_in(&env, &["blob.qqq", "--to", "org"]);
        assert_eq!(code, 1, "{text}");
        assert_eq!(
            text,
            "Failed: blob.qqq, no recipe makes org from it\nReport: 1 failed, 0 converted\n"
        );
        let (code, text) = run_in(&env, &["blob.qqq", "notes.md", "--to", "org"]);
        assert_eq!(code, 3, "{text}");
        let (code, text) = run_in(
            &env,
            &["notes.md", "--to", "docx", "--recipe", "markdown-to-word"],
        );
        assert_eq!(code, 1, "{text}");
        assert_eq!(
            text.lines().next(),
            Some("Could not convert: pandoc is not installed.")
        );
        let (code, text) = run_in(&env, &["notes.md", "--recipe", "no-such"]);
        assert_eq!(code, 1);
        assert_eq!(text, "Could not convert: no recipe is named no-such.\n");
    }

    #[test]
    fn the_list_names_each_recipe_id_first_and_json_is_one_object_a_line() {
        let dir = folder();
        let env = env(dir.path(), None);
        let (code, text) = run_in(&env, &["--list", "notes.md"]);
        assert_eq!(code, 0);
        assert!(
            text.lines()
                .any(|l| l == "markdown-to-org: Markdown to Org, carta, built in, ready"),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|l| l.starts_with("markdown-to-word: Markdown to Word, ")
                    && l.ends_with("needs pandoc")),
            "{text}"
        );
        assert!(
            text.lines()
                .all(|l| l.contains("markdown") || l.contains("Markdown")),
            "{text}"
        );
        let (code, text) = run_in(&env, &["notes.md", "--to", "org", "--json"]);
        assert_eq!(code, 0);
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["result"], "made");
        assert_eq!(lines[0]["name"], "notes.org");
        assert_eq!(lines[1]["summary"], "Report: 1 converted");
    }

    #[test]
    fn a_report_file_holds_names_only_and_the_shelf_keeps_it() {
        let dir = folder();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let env = env(dir.path(), Some(paths.clone()));
        let (code, _) = run_in(&env, &["notes.md", "--to", "org", "--report", "report.md"]);
        assert_eq!(code, 0);
        let text = std::fs::read_to_string(dir.path().join("report.md")).unwrap();
        assert!(text.contains("notes.org"), "{text}");
        let full = dir.path().to_str().unwrap();
        assert!(!text.contains(full), "{text}");
        assert_eq!(report_shelf(&paths).list().len(), 1);
    }
}
