//! A task as the engine checks it (R28 6.3; R16 3.2 and section 4): the
//! keys of `tasks/NAME.toml` read into a [`TaskShape`], then checked into
//! a [`Task`], every problem a sentence that names the task first ("Task
//! scans: home has a wildcard."). A task with any problem never runs.
//!
//! The config crate reads the files (`diroxide_config::task_files`); its
//! `TaskConfig` reaches this module through [`Task::from_serialized`], so
//! this crate depends on nothing but core and vfs.
//!
//! What the checks hold to:
//!
//! - **Folders are literal and absolute** and never a refused folder
//!   ([`super::refuse`]). `home` holds `in`, `out`, `processed` and
//!   `failed`; a move or copy task names `to`; `after` may name a folder.
//!   Nothing goes into `in` but the files that arrive.
//! - **Unattended means no question and no replacement:** `conflicts`
//!   takes keep-both, skip or newer; `ask` and `overwrite` are refused
//!   with their reason, and so is any `after` that would remove a file.
//! - **The words are closed lists**, each refusal naming the words a key
//!   takes.

use std::time::Duration;

use diroxide_core::ConflictPolicy;
use diroxide_core::Place;
use diroxide_core::pattern::Pattern;
use serde::{Deserialize, Serialize};

use super::refuse::{Surroundings, literal_folder, refused, same};
use crate::convert::recipe::{is_plain_id, parse_timeout};

/// The most files of one task that run at once (R16 2.5: light work only).
pub const MOST_PARALLEL: u32 = 4;

/// The most retries a task may ask for: no retry loop is unbounded.
pub const MOST_RETRIES: u32 = 5;

/// The shortest quiet period a task may ask for.
pub const SHORTEST_SETTLE: Duration = Duration::from_millis(500);

/// The longest quiet period a task may ask for.
pub const LONGEST_SETTLE: Duration = Duration::from_secs(3600);

/// The four folders' names inside a task's home.
pub const FOLDER_NAMES: [&str; 4] = ["in", "out", "processed", "failed"];

/// `[trigger]` as written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct TriggerShape {
    /// `folder`.
    pub kind: String,
    /// The folder holding the four folders.
    pub home: String,
    /// Name patterns.
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    /// A filter expression.
    pub filter: Option<String>,
    /// The quiet period: `2s`, `500ms`, `1m`.
    pub settle: String,
    /// `ask`, `all` or `none`.
    pub catch_up: String,
}

impl Default for TriggerShape {
    fn default() -> Self {
        Self {
            kind: "folder".to_owned(),
            home: String::new(),
            matches: Vec::new(),
            filter: None,
            settle: "2s".to_owned(),
            catch_up: "ask".to_owned(),
        }
    }
}

/// `[run]` as written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct RunShape {
    /// `move`, `leave` or a folder.
    pub after: String,
    /// `keep-both`, `skip` or `newer`.
    pub conflicts: String,
    /// 1 to [`MOST_PARALLEL`].
    pub parallel: u32,
    /// A time limit for one file: `10m`.
    pub timeout: Option<String>,
    /// 0 to [`MOST_RETRIES`].
    pub retries: u32,
    /// `skip` or `fetch`.
    pub online_only: String,
}

impl Default for RunShape {
    fn default() -> Self {
        Self {
            after: "move".to_owned(),
            conflicts: "keep-both".to_owned(),
            parallel: 1,
            timeout: None,
            retries: 3,
            online_only: "skip".to_owned(),
        }
    }
}

/// `[say]` as written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct SayShape {
    /// `summary`, `each`, `failures` or `off`.
    pub announce: String,
}

impl Default for SayShape {
    fn default() -> Self {
        Self {
            announce: "summary".to_owned(),
        }
    }
}

/// A task file's keys as written, every word a string so a wrong one is
/// said in words. The config crate's `TaskConfig` has the same keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct TaskShape {
    /// The task's name, plain.
    pub name: String,
    /// What it does, in one sentence.
    pub about: String,
    /// `convert`, `move`, `copy`, `script`, `plugin` or `tell`.
    pub kind: String,
    /// A convert task's recipe id.
    pub recipe: Option<String>,
    /// A script task's command id.
    pub script: Option<String>,
    /// A plugin task's command id.
    pub plugin: Option<String>,
    /// A move or copy task's folder.
    pub to: Option<String>,
    /// When it runs.
    pub trigger: TriggerShape,
    /// How each file is handled.
    pub run: RunShape,
    /// What it says.
    pub say: SayShape,
}

impl Default for TaskShape {
    fn default() -> Self {
        Self {
            name: String::new(),
            about: String::new(),
            kind: "convert".to_owned(),
            recipe: None,
            script: None,
            plugin: None,
            to: None,
            trigger: TriggerShape::default(),
            run: RunShape::default(),
            say: SayShape::default(),
        }
    }
}

/// What a task does with each file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TaskAction {
    /// Runs this conversion recipe; the result goes to `out`.
    Convert {
        /// The recipe's id.
        recipe: String,
    },
    /// Moves the file to the task's `to` folder.
    Move,
    /// Copies the file to the task's `to` folder.
    Copy,
    /// Runs a script command that declares `unattended = "ok"`.
    Script {
        /// The command's id.
        id: String,
    },
    /// Runs a plugin's command.
    Plugin {
        /// The command's id.
        id: String,
    },
    /// Only logs and says the arrival; changes nothing.
    Tell,
}

impl TaskAction {
    /// The kind's word in the file: `convert`.
    pub fn word(&self) -> &'static str {
        match self {
            TaskAction::Convert { .. } => "convert",
            TaskAction::Move => "move",
            TaskAction::Copy => "copy",
            TaskAction::Script { .. } => "script",
            TaskAction::Plugin { .. } => "plugin",
            TaskAction::Tell => "tell",
        }
    }

    /// Whether the action makes something in `out`.
    pub fn makes_output(&self) -> bool {
        matches!(
            self,
            TaskAction::Convert { .. } | TaskAction::Script { .. } | TaskAction::Plugin { .. }
        )
    }

    /// Whether the original is left where it is whatever `after` says:
    /// a move takes it, a tell never touches it.
    pub fn keeps_original_in_place(&self) -> bool {
        matches!(self, TaskAction::Tell)
    }
}

/// Where the original goes after a run.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum After {
    /// To `processed`, or to `failed` when the run fails.
    Move,
    /// Stays in `in`; the ledger remembers it, so it is handled once.
    Leave,
    /// To this folder, or to `failed` when the run fails.
    Folder(Place),
}

/// What the first turn-on does with the files already waiting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CatchUpChoice {
    /// Ask once, starting on No above 20 files.
    #[default]
    Ask,
    /// Process them.
    All,
    /// Leave them; only new arrivals are processed.
    None,
}

impl CatchUpChoice {
    /// The word in the file and the state: `ask`, `all`, `none`.
    pub fn word(self) -> &'static str {
        match self {
            CatchUpChoice::Ask => "ask",
            CatchUpChoice::All => "all",
            CatchUpChoice::None => "none",
        }
    }

    /// Reads the word.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "ask" => Some(CatchUpChoice::Ask),
            "all" => Some(CatchUpChoice::All),
            "none" => Some(CatchUpChoice::None),
            _ => None,
        }
    }
}

/// What to do with a file that is online only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OnlineOnly {
    /// Leave it, said in the log.
    #[default]
    Skip,
    /// Let the system fetch it first.
    Fetch,
}

/// What the task says out loud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Announce {
    /// One polite line for everything since the last.
    #[default]
    Summary,
    /// A line for each file, folded into the summary when they come fast.
    Each,
    /// Only failures.
    Failures,
    /// Nothing; the log keeps it all.
    Off,
}

/// A task's folders.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TaskFolders {
    /// The folder that holds the four.
    pub home: Place,
    /// `in`: the watched queue.
    pub input: Place,
    /// `out`: results.
    pub out: Place,
    /// `processed`: originals after success.
    pub processed: Place,
    /// `failed`: originals that could not be handled.
    pub failed: Place,
    /// A move or copy task's destination.
    pub to: Option<Place>,
}

/// A checked task. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// Its name, plain: `scans`.
    pub name: String,
    /// What it does, in one sentence.
    pub about: String,
    /// What it does with each file.
    pub action: TaskAction,
    /// Its folders.
    pub folders: TaskFolders,
    /// The name patterns as written; empty takes every file.
    pub matches: Vec<String>,
    patterns: Vec<Pattern>,
    /// The filter expression a file must pass, read by the app.
    pub filter: Option<String>,
    /// The quiet period.
    pub settle: Duration,
    /// What the first turn-on does with waiting files.
    pub catch_up: CatchUpChoice,
    /// Where the original goes.
    pub after: After,
    /// What a taken output name gets: keep both, skip or newer.
    pub conflicts: ConflictPolicy,
    /// How many files run at once.
    pub parallel: u32,
    /// A time limit for one file, beside the recipe's own.
    pub timeout: Option<Duration>,
    /// How many times a file error that may pass is tried again.
    pub retries: u32,
    /// What to do with an online-only file.
    pub online_only: OnlineOnly,
    /// What it says.
    pub announce: Announce,
}

/// Something wrong with a task, said with its name first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskProblem {
    /// The task's name, or its file's name when it has none.
    pub task: String,
    /// What is wrong, the rest of the sentence: "home has a wildcard".
    pub words: String,
}

impl TaskProblem {
    /// A problem of a task.
    pub fn new(task: &str, words: impl Into<String>) -> Self {
        Self {
            task: task.to_owned(),
            words: words.into(),
        }
    }

    /// The sentence: "Task scans: home has a wildcard."
    pub fn line(&self) -> String {
        format!("Task {}: {}.", self.task, self.words)
    }
}

impl std::fmt::Display for TaskProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.line())
    }
}

/// Reads a quiet period: a whole number and `ms`, `s`, `m` or `h`
/// (`500ms`, `2s`, `1m`), from half a second to an hour.
pub fn parse_settle(text: &str) -> Result<Duration, &'static str> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().map_err(|_| "is not a time")?;
    let millis = match unit.trim() {
        "ms" => Some(number),
        "s" => number.checked_mul(1000),
        "m" => number.checked_mul(60_000),
        "h" => number.checked_mul(3_600_000),
        _ => return Err("is not a time"),
    }
    .ok_or("is longer than an hour")?;
    let settle = Duration::from_millis(millis);
    if settle < SHORTEST_SETTLE {
        return Err("is shorter than half a second");
    }
    if settle > LONGEST_SETTLE {
        return Err("is longer than an hour");
    }
    Ok(settle)
}

/// A duration as a task file writes it: `2s`, `500ms`, `10m`.
fn settle_text(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis.is_multiple_of(3_600_000) {
        format!("{}h", millis / 3_600_000)
    } else if millis.is_multiple_of(60_000) {
        format!("{}m", millis / 60_000)
    } else if millis.is_multiple_of(1000) {
        format!("{}s", millis / 1000)
    } else {
        format!("{millis}ms")
    }
}

/// Checks a word against a closed list, saying the list when it is not on
/// it: "announce cannot be loud; use summary, each, failures or off".
fn word<'a>(
    key: &str,
    value: &str,
    words: &[&'a str],
    problems: &mut Vec<String>,
) -> Option<&'a str> {
    if let Some(found) = words.iter().find(|w| **w == value) {
        return Some(found);
    }
    let list = match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    };
    problems.push(format!("{key} cannot be {value}; use {list}"));
    None
}

impl Task {
    /// Checks a task as written, in this computer's surroundings. Every
    /// problem is said, not only the first; a task with any never runs.
    pub fn from_shape(shape: &TaskShape, around: &Surroundings) -> Result<Self, Vec<TaskProblem>> {
        let name = shape.name.trim();
        let who = if name.is_empty() { "unnamed" } else { name };
        let mut problems: Vec<String> = Vec::new();
        if name.is_empty() {
            problems.push("it has no name".to_owned());
        } else if !is_plain_id(name) {
            problems.push("its name may hold only letters, digits, - and _".to_owned());
        }
        let style = around.style;
        let id_of = |key: &str, value: &Option<String>, problems: &mut Vec<String>| {
            let id = value.as_deref().map(str::trim).unwrap_or_default();
            let bare = key.trim_start_matches("a ");
            if id.is_empty() {
                problems.push(format!("no {bare}; a {} task needs one", shape.kind.trim()));
                None
            } else if !is_command_id(id) {
                problems.push(format!(
                    "{bare} may hold only letters, digits, dots, - and _"
                ));
                None
            } else {
                Some(id.to_owned())
            }
        };
        let kind = word(
            "kind",
            shape.kind.trim(),
            &["convert", "move", "copy", "script", "plugin", "tell"],
            &mut problems,
        );
        let action =
            match kind {
                Some("convert") => id_of("a recipe", &shape.recipe, &mut problems)
                    .map(|recipe| TaskAction::Convert { recipe }),
                Some("move") => Some(TaskAction::Move),
                Some("copy") => Some(TaskAction::Copy),
                Some("script") => id_of("a script", &shape.script, &mut problems)
                    .map(|id| TaskAction::Script { id }),
                Some("plugin") => id_of("a plugin", &shape.plugin, &mut problems)
                    .map(|id| TaskAction::Plugin { id }),
                Some(_) => Some(TaskAction::Tell),
                None => None,
            };
        let kind_word = kind.unwrap_or("this");
        for (key, value, owner) in [
            ("recipe", &shape.recipe, "convert"),
            ("script", &shape.script, "script"),
            ("plugin", &shape.plugin, "plugin"),
        ] {
            if value.is_some() && kind.is_some_and(|k| k != owner) {
                problems.push(format!("{key} is not used by a {kind_word} task"));
            }
        }
        let wants_to = matches!(kind, Some("move" | "copy"));
        let to = match (&shape.to, wants_to) {
            (Some(text), true) => match literal_folder("to", text, style) {
                Ok(place) => {
                    if let Some(words) = refused("to", &place, around) {
                        problems.push(words);
                    }
                    Some(place)
                }
                Err(words) => {
                    problems.push(words);
                    None
                }
            },
            (None, true) => {
                problems.push(format!("no to folder; a {kind_word} task needs one"));
                None
            }
            (Some(_), false) if kind.is_some() => {
                problems.push(format!("to is not used by a {kind_word} task"));
                None
            }
            _ => None,
        };

        // The trigger.
        word(
            "trigger kind",
            shape.trigger.kind.trim(),
            &["folder"],
            &mut problems,
        );
        let home = match literal_folder("home", &shape.trigger.home, style) {
            Ok(home) => {
                if let Some(words) = refused("home", &home, around) {
                    problems.push(words);
                }
                Some(home)
            }
            Err(words) => {
                problems.push(words);
                None
            }
        };
        let folders = home.and_then(|home| {
            let child = |name: &str| home.join(name).ok();
            Some(TaskFolders {
                input: child("in")?,
                out: child("out")?,
                processed: child("processed")?,
                failed: child("failed")?,
                to: to.clone(),
                home,
            })
        });
        let mut patterns = Vec::new();
        for text in &shape.trigger.matches {
            let text = text.trim();
            let regex = text.starts_with("re:") || text.starts_with("not re:");
            if !regex && text.contains(['/', '\\']) {
                problems.push(format!("match {text} is a path; a pattern names files"));
                continue;
            }
            match Pattern::parse(text) {
                Ok(pattern) => patterns.push(pattern),
                Err(error) => problems.push(format!("match {text} needs care: {error}")),
            }
        }
        let filter = shape
            .trigger
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_owned);
        let settle = match parse_settle(&shape.trigger.settle) {
            Ok(settle) => settle,
            Err(why) => {
                problems.push(format!("settle {} {why}", shape.trigger.settle.trim()));
                SHORTEST_SETTLE
            }
        };
        let catch_up = word(
            "catch-up",
            shape.trigger.catch_up.trim(),
            &["ask", "all", "none"],
            &mut problems,
        )
        .and_then(CatchUpChoice::from_word)
        .unwrap_or_default();

        // The run.
        let after_text = shape.run.after.trim();
        let after = match after_text {
            "move" => After::Move,
            "leave" => After::Leave,
            "delete" | "remove" | "trash" | "erase" | "purge" => {
                problems.push(format!(
                    "{after_text} is refused; a task never removes a file"
                ));
                After::Leave
            }
            text => match literal_folder("after", text, style) {
                Ok(place) => {
                    if let Some(words) = refused("after", &place, around) {
                        problems.push(words);
                    }
                    After::Folder(place)
                }
                Err(words) => {
                    if !text.contains(['\\', '/']) {
                        problems.push(format!(
                            "after cannot be {text}; use move, leave or a folder"
                        ));
                    } else {
                        problems.push(words);
                    }
                    After::Leave
                }
            },
        };
        if after != After::Move && matches!(kind, Some("move" | "tell")) {
            problems.push(format!("after is not used by a {kind_word} task"));
        }
        let conflicts = match shape.run.conflicts.trim() {
            "ask" => {
                problems.push("ask is refused; a task never asks".to_owned());
                ConflictPolicy::KeepBoth
            }
            "overwrite" | "replace" => {
                problems.push(format!(
                    "{} is refused; a task never replaces a file",
                    shape.run.conflicts.trim()
                ));
                ConflictPolicy::KeepBoth
            }
            other => match word(
                "conflicts",
                other,
                &["keep-both", "skip", "newer"],
                &mut problems,
            ) {
                Some("skip") => ConflictPolicy::Skip,
                Some("newer") => ConflictPolicy::Newer,
                _ => ConflictPolicy::KeepBoth,
            },
        };
        if !(1..=MOST_PARALLEL).contains(&shape.run.parallel) {
            problems.push(format!("parallel must be 1 to {MOST_PARALLEL}"));
        }
        let timeout = match shape.run.timeout.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => match parse_timeout(text) {
                Ok(limit) => Some(limit),
                Err(why) => {
                    problems.push(format!("timeout {text} {why}"));
                    None
                }
            },
        };
        if shape.run.retries > MOST_RETRIES {
            problems.push(format!("retries must be 0 to {MOST_RETRIES}"));
        }
        let online_only = match word(
            "online-only",
            shape.run.online_only.trim(),
            &["skip", "fetch"],
            &mut problems,
        ) {
            Some("fetch") => OnlineOnly::Fetch,
            _ => OnlineOnly::Skip,
        };
        let announce = match word(
            "announce",
            shape.say.announce.trim(),
            &["summary", "each", "failures", "off"],
            &mut problems,
        ) {
            Some("each") => Announce::Each,
            Some("failures") => Announce::Failures,
            Some("off") => Announce::Off,
            _ => Announce::Summary,
        };

        // Nothing goes into `in` but what arrives.
        if let Some(folders) = &folders {
            if let Some(to) = &folders.to
                && to.is_within(&folders.input)
            {
                problems.push("to is in the in folder: it loops into itself".to_owned());
            }
            if let After::Folder(folder) = &after {
                if folder.is_within(&folders.input) {
                    problems.push("after is in the in folder: it loops into itself".to_owned());
                }
                if same(folder, &folders.home) {
                    problems.push("after is the home folder; name a folder of its own".to_owned());
                }
            }
            if let Some(to) = &folders.to
                && same(to, &folders.home)
            {
                problems.push("to is the home folder; name a folder of its own".to_owned());
            }
        }

        if !problems.is_empty() {
            return Err(problems
                .into_iter()
                .map(|words| TaskProblem::new(who, words))
                .collect());
        }
        let (Some(action), Some(folders)) = (action, folders) else {
            return Err(vec![TaskProblem::new(who, "it could not be read")]);
        };
        Ok(Self {
            name: name.to_owned(),
            about: shape.about.trim().to_owned(),
            action,
            folders,
            matches: shape
                .trigger
                .matches
                .iter()
                .map(|m| m.trim().to_owned())
                .collect(),
            patterns,
            filter,
            settle,
            catch_up,
            after,
            conflicts,
            parallel: shape.run.parallel,
            timeout,
            retries: shape.run.retries,
            online_only,
            announce,
        })
    }

    /// A task from any value with a task's keys, such as the config
    /// crate's `TaskConfig`: through TOML's value form into a
    /// [`TaskShape`], then [`Task::from_shape`]. `label` names it when its
    /// keys cannot be read (its file: `tasks/scans.toml`).
    pub fn from_serialized<T: Serialize>(
        value: &T,
        label: &str,
        around: &Surroundings,
    ) -> Result<Self, Vec<TaskProblem>> {
        let shape: TaskShape = toml::Value::try_from(value)
            .ok()
            .and_then(|value| value.try_into().ok())
            .ok_or_else(|| vec![TaskProblem::new(label, "its keys could not be read")])?;
        Self::from_shape(&shape, around)
    }

    /// A task from a task file's text; `label` names it when the text is
    /// not a task (`tasks/scans.toml`).
    pub fn from_toml(
        text: &str,
        label: &str,
        around: &Surroundings,
    ) -> Result<Self, Vec<TaskProblem>> {
        let shape: TaskShape = toml::from_str(text).map_err(|error| {
            vec![TaskProblem::new(
                label,
                error.message().trim().trim_end_matches('.').to_owned(),
            )]
        })?;
        Self::from_shape(&shape, around)
    }

    /// The task's name as said: `scans` is "Scans", `photo-inbox` is
    /// "Photo inbox".
    pub fn title(&self) -> String {
        title(&self.name)
    }

    /// Whether a file's name matches the task's patterns (every name when
    /// it has none). The filter, when there is one, is the app's to read.
    pub fn matches_name(&self, name: &str) -> bool {
        self.patterns.is_empty() || self.patterns.iter().any(|p| p.matches(name, false))
    }

    /// The folders a run writes into, the originals' and the results':
    /// what another task's `in` must never be, or the two loop.
    pub fn writes_into(&self) -> Vec<&Place> {
        let mut out = Vec::new();
        if self.action.makes_output() {
            out.push(&self.folders.out);
        }
        if let Some(to) = &self.folders.to {
            out.push(to);
        }
        if !self.action.keeps_original_in_place() && self.action != TaskAction::Move {
            match &self.after {
                After::Move => out.push(&self.folders.processed),
                After::Folder(folder) => out.push(folder),
                After::Leave => {}
            }
        }
        if !self.action.keeps_original_in_place() && self.after != After::Leave {
            out.push(&self.folders.failed);
        }
        out
    }

    /// The folders this task needs, with their names as said, in the
    /// order a new set is made: `in` first.
    pub fn needs_folders(&self) -> Vec<(&'static str, &Place)> {
        let mut out = vec![("in", &self.folders.input)];
        if self.action.makes_output() {
            out.push(("out", &self.folders.out));
        }
        if !self.action.keeps_original_in_place()
            && self.action != TaskAction::Move
            && self.after == After::Move
        {
            out.push(("processed", &self.folders.processed));
        }
        if !self.action.keeps_original_in_place() && self.after != After::Leave {
            out.push(("failed", &self.folders.failed));
        }
        out
    }

    /// The task as written, normalized: the same keys, every value in its
    /// one form. Its text is what an approval is keyed by
    /// ([`super::approval`]), so a comment or a blank line changes
    /// nothing, and every real change does.
    pub fn to_shape(&self) -> TaskShape {
        let (recipe, script, plugin) = match &self.action {
            TaskAction::Convert { recipe } => (Some(recipe.clone()), None, None),
            TaskAction::Script { id } => (None, Some(id.clone()), None),
            TaskAction::Plugin { id } => (None, None, Some(id.clone())),
            _ => (None, None, None),
        };
        TaskShape {
            name: self.name.clone(),
            about: self.about.clone(),
            kind: self.action.word().to_owned(),
            recipe,
            script,
            plugin,
            to: self.folders.to.as_ref().map(ToString::to_string),
            trigger: TriggerShape {
                kind: "folder".to_owned(),
                home: self.folders.home.to_string(),
                matches: self.matches.clone(),
                filter: self.filter.clone(),
                settle: settle_text(self.settle),
                catch_up: self.catch_up.word().to_owned(),
            },
            run: RunShape {
                after: match &self.after {
                    After::Move => "move".to_owned(),
                    After::Leave => "leave".to_owned(),
                    After::Folder(folder) => folder.to_string(),
                },
                conflicts: match self.conflicts {
                    ConflictPolicy::Skip => "skip",
                    ConflictPolicy::Newer => "newer",
                    _ => "keep-both",
                }
                .to_owned(),
                parallel: self.parallel,
                timeout: self.timeout.map(settle_text),
                retries: self.retries,
                online_only: match self.online_only {
                    OnlineOnly::Skip => "skip",
                    OnlineOnly::Fetch => "fetch",
                }
                .to_owned(),
            },
            say: SayShape {
                announce: match self.announce {
                    Announce::Summary => "summary",
                    Announce::Each => "each",
                    Announce::Failures => "failures",
                    Announce::Off => "off",
                }
                .to_owned(),
            },
        }
    }
}

/// A task's name as said: the first letter capital, `-` and `_` as
/// spaces. `scans` is "Scans", `photo-inbox` is "Photo inbox".
pub fn title(name: &str) -> String {
    let spaced = name.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Whether an id names a recipe or a command: letters, digits, `-`, `_`
/// and `.` (`script.total-size`), at most 128, starting with a letter or
/// digit.
pub fn is_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
