//! `diro rename`: the batch rename engine from a shell (alpha 5, lane CX;
//! `research/bulk-rename.md`, "The shell form").
//!
//! - **What to rename:** `PATH...` as typed; `--in FOLDER`, with
//!   `--filter EXPRESSION` in the filter language, for the shells that
//!   expand no wildcards (PowerShell and cmd.exe); `--stdin` for paths on
//!   standard input, one a line or NUL-separated with `--null`; or
//!   `--from-pairs` for an old path and a new name on each line, a tab
//!   between them.
//! - **How:** `--find TEXT` or `--regex PATTERN` with `--replace TEXT`
//!   (`--all`, `--ignore-case`), `--template TEXT` with `--start`,
//!   `--step` and `--width` for `{counter}`, `--case FORM`, each on the
//!   part `--apply-to` names; or `--preset NAME` from
//!   `rename-presets.toml`, or `--recipe FILE` in the same shape.
//! - **The preview is the plan:** the heading, the count line, then one
//!   row a name, problems first, in the engine's own words.
//!   `--dry-run` stops there and ends "Nothing was renamed."; without it
//!   the question starts on No, or `--yes` renames without asking.
//!
//! Nothing is ever overwritten, deleted or moved to another folder: a
//! name that is taken is a problem row and the batch does not run. One
//! job renames the batch, with the engine's order for swaps and rings.
//!
//! Exit codes: 0 renamed, or a dry run with no problems; 1 problems, a
//! path that could not be read, a No, or nothing could run; 2 wrong use
//! or a rule that cannot be read; 3 partly renamed.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use diroxide_core::filter::Lookup;
use diroxide_core::names::display_name;
use diroxide_core::sort::{SortSpec, sort_entries};
use diroxide_core::{Entry, Explain, Filter, JobId, Message, Place};
use diroxide_ops::JobControl;
use diroxide_ops::rename_rules::{
    CaseForm, Counter, FindMode, MAX_COUNTER_WIDTH, Part, Presets, Recipe, RenamePlan,
    RenameSource, Rule, plan_pairs, plan_rename, read_context, run_rename,
};
use diroxide_search::{PathLimits, PathSeparator, read_path_list};
use diroxide_vfs::OpContext;

use crate::CliError;
use crate::cmd::archive::{
    Relay, Session, Unanswered, failure_lines, line, lines, report_code, with_terminal,
};

/// The last line of every run that renamed nothing (20 cells).
pub const NOTHING_RENAMED: &str = "Nothing was renamed.";

/// The last line when nobody was there to say yes (37 cells).
pub const ADD_YES: &str = "Nothing renamed. Add --yes to rename.";

/// The presets file in the config folder.
pub const PRESETS_FILE: &str = "rename-presets.toml";

/// Where the names to rename come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Names {
    /// The paths as typed.
    #[default]
    Paths,
    /// The entries of one folder (`--in`).
    InFolder(String),
    /// Paths on standard input (`--stdin`).
    Stdin,
    /// Old paths and new names on standard input (`--from-pairs`).
    Pairs,
}

/// `diro rename`'s arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameArgs {
    /// Where the names come from.
    pub names: Names,
    /// The paths, as typed, for [`Names::Paths`].
    pub paths: Vec<String>,
    /// `--filter EXPRESSION`, with `--in`.
    pub filter: Option<String>,
    /// `--hidden`: with `--in`, hidden entries too.
    pub hidden: bool,
    /// `--null`: standard input is NUL-separated.
    pub null: bool,
    /// `--find TEXT`.
    pub find: Option<String>,
    /// `--regex PATTERN`.
    pub regex: Option<String>,
    /// `--replace TEXT`.
    pub replace: Option<String>,
    /// `--all`: every match.
    pub all: bool,
    /// `--ignore-case`, when finding.
    pub ignore_case: bool,
    /// `--case FORM`.
    pub case: Option<CaseForm>,
    /// `--template TEXT`.
    pub template: Option<String>,
    /// `--start N`, `--step N`, `--width N` for `{counter}`.
    pub counter: Option<Counter>,
    /// `--apply-to stem|ext|name`.
    pub part: Part,
    /// `--preset NAME`.
    pub preset: Option<String>,
    /// `--recipe FILE`.
    pub recipe: Option<String>,
    /// `--portable`: the presets beside the program.
    pub portable: bool,
    /// `--dry-run`: the preview and nothing else.
    pub dry_run: bool,
    /// `--yes`: rename without asking.
    pub yes: bool,
    /// Print the help for `diro rename` and nothing else.
    pub help: bool,
}

/// The help for `diro rename`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro rename: rename many names by a rule, the preview first.",
        "",
        "Usage:",
        "diro rename PATH... RULE      rename these entries",
        "diro rename --in FOLDER RULE  rename the entries of FOLDER; --filter EXPRESSION picks",
        "diro rename --stdin RULE      rename the paths on standard input, one a line",
        "diro rename --from-pairs      old path, a tab, new name, one pair a line",
        "",
        "Rules, applied in this order:",
        "--find TEXT --replace TEXT    replace the first TEXT; --all for every one",
        "--regex PATTERN --replace TEXT  a pattern, $1 for a group",
        "--ignore-case                 find without regard to case",
        "--template TEXT               make the name anew: {stem}, {counter}, {modified}",
        "--start N --step N --width N  the counter of --template; 1, 1 and 1 by default",
        "--case FORM                   lower, upper, title or sentence",
        "--apply-to PART               stem (the default), ext or name",
        "--preset NAME                 a preset from rename-presets.toml",
        "--recipe FILE                 a recipe file in the presets' shape",
        "",
        "Options:",
        "--dry-run, -n                 print the preview and rename nothing",
        "--yes, -y                     rename without asking",
        "--null, -0                    standard input is NUL-separated",
        "--hidden                      with --in, hidden entries too",
        "--portable                    the presets beside the program",
        "",
        "Exit codes:",
        "0  renamed, or a dry run with no problems",
        "1  problems, a path not read, a No, or nothing could run; nothing renamed",
        "2  wrong use, or a rule that cannot be read",
        "3  partly renamed: the output says which were not",
    ]
    .join("\n")
}

/// Reads the arguments after `rename`.
pub fn parse(args: &[String]) -> Result<RenameArgs, CliError> {
    let mut parsed = RenameArgs::default();
    let mut only_paths = false;
    let mut folder: Option<String> = None;
    let mut stdin = false;
    let mut pairs = false;
    let mut start: Option<i64> = None;
    let mut step: Option<i64> = None;
    let mut width: Option<u8> = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_paths {
            parsed.paths.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        // A value may be empty only for --replace, which removes the text.
        let mut value = |flag: &str, empty_ok: bool| -> Result<String, CliError> {
            match inline.clone().or_else(|| rest.next().cloned()) {
                Some(value) if empty_ok || !value.is_empty() => Ok(value),
                _ => Err(CliError::Missing(flag.to_owned())),
            }
        };
        match name {
            "--" => only_paths = true,
            "--help" | "-h" => parsed.help = true,
            "--in" => folder = Some(value("--in", false)?),
            "--filter" => parsed.filter = Some(value("--filter", false)?),
            "--hidden" => parsed.hidden = true,
            "--stdin" => stdin = true,
            "--from-pairs" => pairs = true,
            "--null" | "-0" => parsed.null = true,
            "--find" => parsed.find = Some(value("--find", false)?),
            "--regex" => parsed.regex = Some(value("--regex", false)?),
            "--replace" => parsed.replace = Some(value("--replace", true)?),
            "--all" => parsed.all = true,
            "--ignore-case" | "-i" => parsed.ignore_case = true,
            "--case" => {
                let text = value("--case", false)?;
                parsed.case = Some(case_named(&text).ok_or_else(|| {
                    CliError::Failed(format!(
                        "there is no case {text}; use lower, upper, title or sentence"
                    ))
                })?);
            }
            "--template" => parsed.template = Some(value("--template", false)?),
            "--start" => start = Some(number("--start", &value("--start", false)?)?),
            "--step" => step = Some(number("--step", &value("--step", false)?)?),
            "--width" => {
                let text = value("--width", false)?;
                width = Some(match text.parse::<u8>() {
                    Ok(n) if (1..=MAX_COUNTER_WIDTH).contains(&n) => n,
                    _ => {
                        return Err(CliError::Failed(format!(
                            "the width {text} is not a number from 1 to {MAX_COUNTER_WIDTH}"
                        )));
                    }
                });
            }
            "--apply-to" => {
                let text = value("--apply-to", false)?;
                parsed.part = match text.to_lowercase().as_str() {
                    "stem" => Part::Stem,
                    "ext" | "extension" => Part::Extension,
                    "name" => Part::Name,
                    _ => {
                        return Err(CliError::Failed(format!(
                            "there is no part {text}; use stem, ext or name"
                        )));
                    }
                };
            }
            "--preset" => parsed.preset = Some(value("--preset", false)?),
            "--recipe" => parsed.recipe = Some(value("--recipe", false)?),
            "--portable" => parsed.portable = true,
            "--dry-run" | "-n" => parsed.dry_run = true,
            "--yes" | "-y" => parsed.yes = true,
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            path => parsed.paths.push(path.to_owned()),
        }
    }
    if parsed.help {
        return Ok(parsed);
    }
    let sources = [!parsed.paths.is_empty(), folder.is_some(), stdin, pairs];
    match sources.iter().filter(|given| **given).count() {
        0 => return Err(CliError::Missing("rename".to_owned())),
        1 => {}
        _ => {
            return Err(wrong(
                "name the entries one way: paths, --in, --stdin or --from-pairs",
            ));
        }
    }
    parsed.names = match (folder, stdin, pairs) {
        (Some(folder), _, _) => Names::InFolder(folder),
        (_, true, _) => Names::Stdin,
        (_, _, true) => Names::Pairs,
        _ => Names::Paths,
    };
    let in_folder = matches!(parsed.names, Names::InFolder(_));
    if (parsed.filter.is_some() || parsed.hidden) && !in_folder {
        return Err(wrong("--filter and --hidden go with --in FOLDER"));
    }
    if let Some(text) = &parsed.filter
        && let Err(error) = Filter::parse(text)
    {
        return Err(wrong(&format!(
            "the filter cannot be read: {error}. {}",
            error.next_step()
        )));
    }
    if parsed.null && !matches!(parsed.names, Names::Stdin | Names::Pairs) {
        return Err(wrong("--null goes with --stdin or --from-pairs"));
    }
    if parsed.find.is_some() && parsed.regex.is_some() {
        return Err(wrong("use --find or --regex, not both"));
    }
    let finds = parsed.find.is_some() || parsed.regex.is_some();
    if finds && parsed.replace.is_none() {
        return Err(CliError::Missing("--replace".to_owned()));
    }
    if parsed.replace.is_some() && !finds {
        return Err(wrong("--replace goes with --find or --regex"));
    }
    if (parsed.all || parsed.ignore_case) && !finds {
        return Err(wrong("--all and --ignore-case go with --find or --regex"));
    }
    let counted = start.is_some() || step.is_some() || width.is_some();
    if counted && parsed.template.is_none() {
        return Err(wrong("--start, --step and --width go with --template"));
    }
    if counted {
        let base = Counter::default();
        parsed.counter = Some(Counter {
            start: start.unwrap_or(base.start),
            step: step.unwrap_or(base.step),
            width: width.unwrap_or(base.width),
            per_folder: false,
        });
    }
    let inline = finds || parsed.case.is_some() || parsed.template.is_some();
    let saved = [parsed.preset.is_some(), parsed.recipe.is_some()];
    if saved.iter().all(|given| *given) {
        return Err(wrong("use --preset or --recipe, not both"));
    }
    let any_saved = saved.iter().any(|given| *given);
    if parsed.names == Names::Pairs {
        if inline || any_saved {
            return Err(wrong(
                "--from-pairs takes the new names from its input, not rules",
            ));
        }
    } else if inline && any_saved {
        return Err(wrong(
            "a preset or a recipe holds its own rules; leave out the rule options",
        ));
    } else if !inline && !any_saved {
        return Err(wrong(
            "say how to rename: --find, --regex, --template, --case, --preset or --recipe",
        ));
    }
    Ok(parsed)
}

/// A usage mistake in words; `main_with` exits 2 for it.
fn wrong(sentence: &str) -> CliError {
    CliError::Failed(sentence.to_owned())
}

fn number(flag: &str, text: &str) -> Result<i64, CliError> {
    text.parse::<i64>()
        .map_err(|_| wrong(&format!("{flag} needs a whole number, not {text}")))
}

/// A case form by its name.
pub fn case_named(text: &str) -> Option<CaseForm> {
    match text.to_lowercase().as_str() {
        "lower" | "lowercase" => Some(CaseForm::Lower),
        "upper" | "uppercase" => Some(CaseForm::Upper),
        "title" => Some(CaseForm::Title),
        "sentence" => Some(CaseForm::Sentence),
        _ => None,
    }
}

/// The recipe of the inline rule options, in the order the help gives:
/// the replacement, the template, the case.
pub fn inline_recipe(args: &RenameArgs) -> Recipe {
    let mut rules = Vec::new();
    let found = match (&args.find, &args.regex) {
        (Some(find), _) => Some((find, FindMode::Text)),
        (_, Some(pattern)) => Some((pattern, FindMode::Pattern)),
        _ => None,
    };
    if let Some((find, mode)) = found {
        rules.push(Rule::Replace {
            find: find.clone(),
            with: args.replace.clone().unwrap_or_default(),
            mode,
            all: args.all,
            ignore_case: args.ignore_case,
            part: args.part,
        });
    }
    if let Some(template) = &args.template {
        rules.push(Rule::template(template.clone()).on(args.part));
    }
    if let Some(form) = args.case {
        rules.push(Rule::case(form).on(args.part));
    }
    let recipe = Recipe::of(rules);
    match args.counter {
        Some(counter) => recipe.counting(counter),
        None => recipe,
    }
}

/// Runs `diro rename` on this computer's files and the terminal.
pub fn run(args: &RenameArgs, input: &mut dyn Read, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    with_terminal(|session| run_with(args, input, out, session))
}

/// Runs `diro rename` in a session: the providers, the clock, the current
/// folder and who answers.
pub fn run_with(
    args: &RenameArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    session: &Session<'_>,
) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    let read = match gather(args, input, session) {
        Ok(read) => read,
        Err(Stop(lines_said, code)) => {
            lines(out, &lines_said)?;
            return Ok(code);
        }
    };
    lines(out, &read.refused)?;
    let engine = session.engine();
    let control = JobControl::new();
    let sources: Vec<RenameSource> = match &read.batch {
        Batch::Rules(sources) => sources.clone(),
        Batch::Pairs(pairs) => pairs.iter().map(|(source, _)| source.clone()).collect(),
    };
    let context = read_context(&engine, &sources, &control).with_offset(session.clock.offset());
    let plan = match &read.batch {
        Batch::Rules(sources) => {
            let recipe = match recipe_for(args, session) {
                Ok(recipe) => recipe,
                Err(Stop(lines_said, code)) => {
                    lines(out, &lines_said)?;
                    return Ok(code);
                }
            };
            match plan_rename(sources, &recipe, &context) {
                Ok(plan) => plan,
                Err(error) => {
                    lines(out, &[error.line(), NOTHING_RENAMED.to_owned()])?;
                    return Ok(2);
                }
            }
        }
        Batch::Pairs(pairs) => plan_pairs(pairs, &context),
    };
    lines(out, &preview_lines(&plan))?;
    let unread = !read.refused.is_empty();
    if args.dry_run {
        line(out, NOTHING_RENAMED)?;
        return Ok(if plan.problems() > 0 || unread { 1 } else { 0 });
    }
    if unread {
        line(out, NOTHING_RENAMED)?;
        return Ok(1);
    }
    if let Some(refusal) = plan.refusal_line() {
        line(out, &refusal)?;
        if plan.problems() > 0 {
            line(out, NOTHING_RENAMED)?;
            return Ok(1);
        }
        return Ok(0);
    }
    if !args.yes {
        let question = format!("{} y or n", plan.question());
        if !session.confirm(&question, out)? {
            let last = if session.asker.present() {
                NOTHING_RENAMED
            } else {
                ADD_YES
            };
            line(out, last)?;
            return Ok(1);
        }
    }
    let relay = Relay::new();
    let mut unanswered = Unanswered::default();
    let env = diroxide_ops::rename_rules::RenameEnv {
        engine: &engine,
        job: JobId::new(1),
        control: &control,
        sink: relay.as_ref(),
        lookup: None,
        offset: session.clock.offset(),
    };
    let result = session.drive(&relay, &mut unanswered, || run_rename(&plan, &env));
    unanswered.write(out)?;
    match result {
        Err(refused) => {
            lines(out, &[refused.line(), NOTHING_RENAMED.to_owned()])?;
            Ok(1)
        }
        Ok(report) => {
            let wanted = plan.renaming();
            let done = report.done.len().min(wanted);
            let summary = if done == wanted && report.failed.is_empty() {
                renamed_line(done)
            } else {
                format!(
                    "Renamed {done} of {wanted}. {} not renamed.",
                    wanted.saturating_sub(done)
                )
            };
            line(out, &summary)?;
            let skipped: Vec<String> = report
                .skipped
                .iter()
                .map(|(place, why)| format!("Not renamed: {}, {why}", place_name(place)))
                .collect();
            lines(out, &skipped)?;
            lines(out, &failure_lines(&report))?;
            Ok(report_code(&report))
        }
    }
}

/// "Renamed 50 files." (17 cells), "Renamed 1 file."
pub fn renamed_line(done: usize) -> String {
    let files = if done == 1 { "file" } else { "files" };
    format!("Renamed {done} {files}.")
}

/// The preview: the heading, the count line, then every row, problems
/// first.
pub fn preview_lines(plan: &RenamePlan) -> Vec<String> {
    let mut said = vec![heading(plan), plan.count_line()];
    said.extend(plan.problems_first().into_iter().map(|row| row.line()));
    said
}

/// "Rename preview in D:\photos." or "Rename preview in 3 folders."
pub fn heading(plan: &RenamePlan) -> String {
    let folders: BTreeSet<String> = plan
        .sources()
        .iter()
        .filter_map(|source| source.place.parent())
        .map(|folder| folder.to_string())
        .collect();
    match folders.len() {
        1 => format!(
            "Rename preview in {}.",
            folders
                .iter()
                .next()
                .map(String::as_str)
                .unwrap_or_default()
        ),
        0 => "Rename preview.".to_owned(),
        n => format!("Rename preview in {n} folders."),
    }
}

fn place_name(place: &Place) -> String {
    display_name(place.name().unwrap_or_default()).into_owned()
}

/// What the run was given, read and checked.
struct Gathered {
    batch: Batch,
    /// The paths that could not be read, said first.
    refused: Vec<String>,
}

enum Batch {
    Rules(Vec<RenameSource>),
    Pairs(Vec<(RenameSource, String)>),
}

/// A stop before the preview: the lines to say and the exit code.
struct Stop(Vec<String>, i32);

/// "Could not rename X: CAUSE. NEXT STEP."
fn could_not(text: &str, error: &dyn Explain) -> String {
    Message::from_error("rename", &display_name(text), error).text
}

/// Reads the names to rename.
fn gather(
    args: &RenameArgs,
    input: &mut dyn Read,
    session: &Session<'_>,
) -> Result<Gathered, Stop> {
    let mut refused = Vec::new();
    let batch = match &args.names {
        Names::Paths => Batch::Rules(stat_all(&args.paths, session, &mut refused)),
        Names::InFolder(folder) => Batch::Rules(in_folder(args, folder, session)?),
        Names::Stdin => {
            let separator = if args.null {
                PathSeparator::Nul
            } else {
                PathSeparator::Lines
            };
            let list = read_path_list(input, separator, PathLimits::default())
                .map_err(|error| Stop(vec![error.to_string(), NOTHING_RENAMED.to_owned()], 2))?;
            let mut texts = Vec::new();
            for item in &list.items {
                match &item.text {
                    Some(text) => texts.push(text.clone()),
                    None => refused.push(format!(
                        "Could not rename {}: the name is not valid Unicode. Rename it, or open its folder and move to it.",
                        item.what()
                    )),
                }
            }
            if list.stopped_at_limit {
                return Err(Stop(
                    vec![
                        format!("List is too long: over {} paths.", list.max_paths),
                        NOTHING_RENAMED.to_owned(),
                    ],
                    1,
                ));
            }
            Batch::Rules(stat_all(&texts, session, &mut refused))
        }
        Names::Pairs => Batch::Pairs(read_pairs(args, input, session, &mut refused)?),
    };
    let empty = match &batch {
        Batch::Rules(sources) => sources.is_empty(),
        Batch::Pairs(pairs) => pairs.is_empty(),
    };
    if empty {
        refused.push("Nothing to rename: no names were given.".to_owned());
        refused.push(NOTHING_RENAMED.to_owned());
        return Err(Stop(refused, 1));
    }
    Ok(Gathered { batch, refused })
}

/// Each path read through its provider; one that cannot be read is said
/// and left out.
fn stat_all(
    texts: &[String],
    session: &Session<'_>,
    refused: &mut Vec<String>,
) -> Vec<RenameSource> {
    let mut seen = std::collections::HashSet::new();
    let mut sources = Vec::new();
    for text in texts {
        match stat_one(text, session) {
            Ok(source) if seen.insert(source.place.clone()) => sources.push(source),
            Ok(_) => {}
            Err(sentence) => refused.push(sentence),
        }
    }
    sources
}

fn stat_one(text: &str, session: &Session<'_>) -> Result<RenameSource, String> {
    let place = Place::resolve(text, session.here).map_err(|e| could_not(text, &e))?;
    let entry = session
        .registry
        .resolve(&place)
        .and_then(|provider| provider.stat(&place, &OpContext::new()))
        .map_err(|e| could_not(text, &AtTheShell(&e)))?;
    Ok(RenameSource::new(place, entry))
}

/// A provider's error at the shell: a place that is not there asks for
/// the path to be checked.
struct AtTheShell<'a>(&'a diroxide_vfs::VfsError);

impl std::fmt::Display for AtTheShell<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Explain for AtTheShell<'_> {
    fn next_step(&self) -> String {
        match self.0 {
            diroxide_vfs::VfsError::NotFound => "Check the path.".to_owned(),
            other => other.next_step(),
        }
    }
}

/// A filter's lookup for a folder's direct entries.
struct Direct;

impl Lookup for Direct {
    fn depth(&self, _entry: &Entry) -> Option<u32> {
        Some(1)
    }
}

/// The entries of one folder in the panel's order, hidden ones left out
/// unless asked, the filter kept.
fn in_folder(
    args: &RenameArgs,
    text: &str,
    session: &Session<'_>,
) -> Result<Vec<RenameSource>, Stop> {
    let stop = |sentence: String| Stop(vec![sentence, NOTHING_RENAMED.to_owned()], 1);
    let folder = Place::resolve(text, session.here).map_err(|e| stop(could_not(text, &e)))?;
    let mut entries = session
        .registry
        .resolve(&folder)
        .and_then(|provider| provider.list(&folder, &OpContext::new()))
        .map_err(|e| stop(could_not(text, &AtTheShell(&e))))?;
    if !args.hidden {
        entries.retain(|entry| !entry.hidden);
    }
    if let Some(filter) = &args.filter {
        let filter = Filter::parse(filter).map_err(|e| {
            Stop(
                vec![format!("The filter cannot be read: {e}. {}", e.next_step())],
                2,
            )
        })?;
        let test = filter.evaluator(session.clock);
        entries.retain(|entry| test.matches(entry, &Direct));
    }
    sort_entries(&mut entries, &SortSpec::default());
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let place = folder.join(&entry.name).ok()?;
            Some(RenameSource::new(place, entry))
        })
        .collect())
}

/// Reads `old<TAB>new` lines, or with `--null` old and new names in turn.
fn read_pairs(
    args: &RenameArgs,
    input: &mut dyn Read,
    session: &Session<'_>,
    refused: &mut Vec<String>,
) -> Result<Vec<(RenameSource, String)>, Stop> {
    let mut bytes = Vec::new();
    input
        .take(PathLimits::default().max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Stop(vec![format!("standard input could not be read: {e}")], 1))?;
    if bytes.len() as u64 > PathLimits::default().max_bytes {
        return Err(Stop(
            vec![
                "List is too long: over 64 MiB.".to_owned(),
                NOTHING_RENAMED.to_owned(),
            ],
            1,
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        Stop(
            vec![
                "The pairs are not valid Unicode. Rename it, or open its folder and move to it."
                    .to_owned(),
                NOTHING_RENAMED.to_owned(),
            ],
            2,
        )
    })?;
    let mut raw: Vec<(String, String)> = Vec::new();
    if args.null {
        let items: Vec<&str> = text.split('\0').filter(|item| !item.is_empty()).collect();
        if items.len() % 2 == 1 {
            return Err(Stop(
                vec![
                    "The last old name has no new name after it.".to_owned(),
                    NOTHING_RENAMED.to_owned(),
                ],
                2,
            ));
        }
        for pair in items.chunks(2) {
            raw.push((pair[0].to_owned(), pair[1].to_owned()));
        }
    } else {
        if text.contains('\0') {
            return Err(Stop(
                vec![
                    "The pairs have NUL separators. Add --null.".to_owned(),
                    NOTHING_RENAMED.to_owned(),
                ],
                2,
            ));
        }
        for (number, line_text) in text.lines().enumerate() {
            let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
            if line_text.is_empty() {
                continue;
            }
            match line_text.split_once('\t') {
                Some((old, new)) if !new.contains('\t') => {
                    raw.push((old.to_owned(), new.to_owned()));
                }
                Some(_) => {
                    return Err(Stop(
                        vec![
                            format!("Line {} has more than one tab.", number + 1),
                            NOTHING_RENAMED.to_owned(),
                        ],
                        2,
                    ));
                }
                None => {
                    return Err(Stop(
                        vec![
                            format!("Line {} has no tab between the names.", number + 1),
                            NOTHING_RENAMED.to_owned(),
                        ],
                        2,
                    ));
                }
            }
        }
    }
    let mut pairs = Vec::new();
    for (old, new) in raw {
        match stat_one(&old, session) {
            Ok(source) => pairs.push((source, new)),
            Err(sentence) => refused.push(sentence),
        }
    }
    Ok(pairs)
}

/// The recipe: the inline rules, a preset by name, or a recipe file.
fn recipe_for(args: &RenameArgs, session: &Session<'_>) -> Result<Recipe, Stop> {
    let stop = |sentence: String, code: i32| Stop(vec![sentence, NOTHING_RENAMED.to_owned()], code);
    if let Some(file) = &args.recipe {
        let place = Place::resolve(file, session.here).map_err(|e| stop(could_not(file, &e), 1))?;
        let text = std::fs::read_to_string(place.to_string()).map_err(|e| {
            stop(
                format!("Could not read the recipe {}: {e}.", display_name(file)),
                1,
            )
        })?;
        return Recipe::from_toml(&text).map_err(|e| stop(e.to_string(), 2));
    }
    if let Some(name) = &args.preset {
        let file = crate::open::paths(args.portable)
            .map(|paths| paths.config_file(PRESETS_FILE))
            .ok_or_else(|| stop("The config folder could not be found.".to_owned(), 1))?;
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(stop(format!("Could not read {PRESETS_FILE}: {e}."), 1)),
        };
        let presets = Presets::from_toml(&text).map_err(|e| stop(e.to_string(), 2))?;
        return presets
            .get(name)
            .map(|preset| preset.recipe.clone())
            .ok_or_else(|| {
                stop(
                    format!("No rename preset is named {}.", display_name(name)),
                    1,
                )
            });
    }
    Ok(inline_recipe(args))
}

#[cfg(test)]
mod tests;
