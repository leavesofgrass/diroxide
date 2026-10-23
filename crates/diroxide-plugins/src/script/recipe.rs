//! How a script command runs: the keys a script's header and a
//! manifest-only command share (R3 section 5.1, completed by R22 section
//! 2.6). [`Recipe::read`] reads them from a table with every problem named
//! by its field, so `diro plugin check` says the same words for a header
//! line and a `[[command]]` table.
//!
//! **`unattended` is defined here, once.** A command that declares
//! `unattended = "ok"` promises that it never asks (no `prompt`, no
//! `confirm`, no terminal mode, no reading of standard input), that it
//! ends within its `timeout` (so it must carry one), that it honors
//! `DIRO_UNATTENDED=1`, and that under `DIRO_DRY_RUN=1` it changes
//! nothing. Hot-folder rules and `diro task run --unattended` run only
//! such commands, and the consent text adds "It can run without asking."
//! [`Recipe::unattended_problems`] is the check.

use std::collections::BTreeMap;

use toml::{Table, Value};

use crate::manifest::{Limit, Problem};
use crate::script::read::Reader;

/// The most rows a lister keeps by default: "First 5,000 shown, narrow
/// the search."
pub const DEFAULT_MAX_ROWS: u32 = 5000;

/// The most rows a lister may ask for.
pub const MAX_ROWS_LIMIT: u32 = 100_000;

/// The keys of the run family, read by [`Recipe::read`].
pub const RECIPE_KEYS: &[&str] = &[
    "mode",
    "per",
    "stdin",
    "cwd",
    "timeout",
    "confirm",
    "prompt",
    "unattended",
    "dry_run",
    "cursor_fallback",
    "exit_words",
    "list",
    "max_rows",
    "selection_file",
    "interpreter",
    "stdout_directives",
];

/// Where a command's output goes (R3's modes, with R22's `lister` and
/// `launch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RunMode {
    /// Live in the output pane, one line a row (the default).
    #[default]
    OutputPane,
    /// Kept in the command's log, shown only on failure.
    Run,
    /// Shown in the viewer when the command ends.
    OutputViewer,
    /// The first output line becomes each entry's custom column; one run
    /// per entry.
    OutputColumn,
    /// The screen is handed over, then restored.
    Terminal,
    /// As `Terminal`, then waits for Enter.
    TerminalWait,
    /// The output lines are places, opened as a lister of real files.
    Lister,
    /// Started detached; the host answers at once.
    Launch,
}

impl RunMode {
    /// Every mode, in the order the guide lists them.
    pub const ALL: [RunMode; 8] = [
        RunMode::OutputPane,
        RunMode::Run,
        RunMode::OutputViewer,
        RunMode::OutputColumn,
        RunMode::Terminal,
        RunMode::TerminalWait,
        RunMode::Lister,
        RunMode::Launch,
    ];

    /// The mode's word in a header or a manifest.
    pub const fn word(self) -> &'static str {
        match self {
            RunMode::OutputPane => "output-pane",
            RunMode::Run => "run",
            RunMode::OutputViewer => "output-viewer",
            RunMode::OutputColumn => "output-column",
            RunMode::Terminal => "terminal",
            RunMode::TerminalWait => "terminal-wait",
            RunMode::Lister => "lister",
            RunMode::Launch => "launch",
        }
    }

    /// The mode for a word.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.word() == word)
    }

    /// Whether the command shares the terminal with the owner.
    pub const fn is_terminal(self) -> bool {
        matches!(self, RunMode::Terminal | RunMode::TerminalWait)
    }
}

/// How often a command runs for a selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Per {
    /// Once for the whole selection (the default).
    #[default]
    Selection,
    /// Once per entry, with `DIRO_CURSOR` set to it.
    Entry,
}

/// What a command gets on standard input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StdinForm {
    /// The command context as JSON, then the end of input (the default).
    #[default]
    Json,
    /// Nothing: the null device.
    None,
}

/// The folder a command starts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CwdChoice {
    /// The active panel's folder (the default).
    #[default]
    Active,
    /// The other panel's folder.
    Other,
    /// The script's own folder.
    Script,
}

/// How a list of places is written: one a line, or each ended by a NUL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ListForm {
    /// One place a line, LF-ended (the default).
    #[default]
    Lines,
    /// Each place ended by a NUL byte: right for any name.
    Nul,
}

impl ListForm {
    /// The form's word.
    pub const fn word(self) -> &'static str {
        match self {
            ListForm::Lines => "lines",
            ListForm::Nul => "nul",
        }
    }
}

/// How a script command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// Where the output goes.
    pub mode: RunMode,
    /// Once for the selection, or once per entry.
    pub per: Per,
    /// What standard input gets.
    pub stdin: StdinForm,
    /// The folder it starts in.
    pub cwd: CwdChoice,
    /// The longest it may run; none by default.
    pub timeout: Limit,
    /// A question the host asks first, the answer starting on No.
    pub confirm: Option<String>,
    /// One line the host asks first, passed as `DIRO_INPUT` and `{input}`;
    /// an empty answer cancels.
    pub prompt: Option<String>,
    /// The command never asks and ends within its timeout.
    pub unattended: bool,
    /// The command has a preview: under `DIRO_DRY_RUN=1` it prints what it
    /// would do and changes nothing.
    pub dry_run: bool,
    /// Whether the cursor's entry counts when nothing is marked.
    pub cursor_fallback: bool,
    /// Sentences for exit codes that mean something.
    pub exit_words: BTreeMap<i32, String>,
    /// For a lister: how the output's places are separated.
    pub list: ListForm,
    /// For a lister: the most rows kept.
    pub max_rows: u32,
    /// Which selection file a script reads; a line reader is refused a
    /// name with a line break.
    pub selection_file: ListForm,
    /// The interpreter, replacing the choice by extension.
    pub interpreter: Vec<String>,
    /// Whether `diro:` lines on standard output are directives too.
    pub stdout_directives: bool,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            mode: RunMode::default(),
            per: Per::default(),
            stdin: StdinForm::default(),
            cwd: CwdChoice::default(),
            timeout: None,
            confirm: None,
            prompt: None,
            unattended: false,
            dry_run: false,
            cursor_fallback: true,
            exit_words: BTreeMap::new(),
            list: ListForm::default(),
            max_rows: DEFAULT_MAX_ROWS,
            selection_file: ListForm::default(),
            interpreter: Vec::new(),
            stdout_directives: false,
        }
    }
}

impl Recipe {
    /// Reads the run keys of `table` ([`RECIPE_KEYS`]; other keys are the
    /// caller's), naming each field with `label`. Returns the recipe, with
    /// defaults where a value was missing or wrong, and the problems.
    pub fn read(table: &Table, label: &dyn Fn(&str) -> String) -> (Self, Vec<Problem>) {
        let mut r = Reader::new(table, label);
        let mut recipe = Recipe::default();
        let modes: Vec<&str> = RunMode::ALL.iter().map(|m| m.word()).collect();
        if let Some(word) = r.word("mode", &modes) {
            recipe.mode = RunMode::parse(&word).unwrap_or_default();
        }
        if let Some(word) = r.word("per", &["selection", "entry"]) {
            recipe.per = if word == "entry" {
                Per::Entry
            } else {
                Per::Selection
            };
        }
        if recipe.mode == RunMode::OutputColumn {
            recipe.per = Per::Entry;
        }
        if let Some(word) = r.word("stdin", &["json", "none"]) {
            recipe.stdin = if word == "none" {
                StdinForm::None
            } else {
                StdinForm::Json
            };
        }
        if let Some(word) = r.word("cwd", &["active", "other", "script"]) {
            recipe.cwd = match word.as_str() {
                "other" => CwdChoice::Other,
                "script" => CwdChoice::Script,
                _ => CwdChoice::Active,
            };
        }
        if let Some(limit) = r.duration("timeout") {
            recipe.timeout = limit;
        }
        recipe.confirm = r.string("confirm");
        recipe.prompt = r.string("prompt");
        for (key, text) in [("confirm", &recipe.confirm), ("prompt", &recipe.prompt)] {
            if text.as_deref().is_some_and(|t| t.trim().is_empty()) {
                r.error(
                    key,
                    "is empty",
                    "Write the question in words, or remove the key.",
                );
            }
        }
        match r.get("unattended") {
            None => {}
            Some(Value::String(word)) if word == "ok" => recipe.unattended = true,
            Some(_) => r.error(
                "unattended",
                "is not \"ok\"",
                "Write unattended = \"ok\" when the command never asks and ends within its timeout.",
            ),
        }
        match r.get("dry_run") {
            None => {}
            Some(Value::String(word)) if word == "supported" => recipe.dry_run = true,
            Some(_) => r.error(
                "dry_run",
                "is not \"supported\"",
                "Write dry_run = \"supported\" when DIRO_DRY_RUN=1 makes it change nothing.",
            ),
        }
        if let Some(fallback) = r.boolean("cursor_fallback") {
            recipe.cursor_fallback = fallback;
        }
        if let Some(words) = r.table("exit_words") {
            recipe.exit_words = read_exit_words(words, &mut r);
        }
        if let Some(word) = r.word("list", &["lines", "nul"]) {
            recipe.list = if word == "nul" {
                ListForm::Nul
            } else {
                ListForm::Lines
            };
        }
        if let Some(n) = r.integer("max_rows", 1, i64::from(MAX_ROWS_LIMIT)) {
            recipe.max_rows = u32::try_from(n).unwrap_or(DEFAULT_MAX_ROWS);
        }
        if let Some(word) = r.word("selection_file", &["lines", "nul"]) {
            recipe.selection_file = if word == "nul" {
                ListForm::Nul
            } else {
                ListForm::Lines
            };
        }
        if let Some(interpreter) = r.strings("interpreter") {
            if interpreter.first().is_none_or(|p| p.trim().is_empty()) {
                r.error(
                    "interpreter",
                    "is empty",
                    "Name the program first, then its arguments, such as [\"python3\", \"-I\"].",
                );
            } else {
                recipe.interpreter = interpreter;
            }
        }
        if let Some(on) = r.boolean("stdout_directives") {
            recipe.stdout_directives = on;
        }
        let mut problems = r.problems;
        if recipe.unattended {
            problems.extend(recipe.unattended_problems(label));
        }
        (recipe, problems)
    }

    /// What breaks the `unattended` promise: a prompt, a question, a
    /// terminal mode, or no timeout. Empty when the command does not
    /// declare it.
    pub fn unattended_problems(&self, label: &dyn Fn(&str) -> String) -> Vec<Problem> {
        if !self.unattended {
            return Vec::new();
        }
        let mut problems = Vec::new();
        let mut error = |key: &str, message: &str, fix: &str| {
            problems.push(Problem {
                severity: crate::manifest::Severity::Error,
                field: label(key),
                message: message.to_owned(),
                fix: fix.to_owned(),
            });
        };
        if self.prompt.is_some() {
            error(
                "prompt",
                "asks a question, and the command is unattended",
                "Remove prompt, or remove unattended.",
            );
        }
        if self.confirm.is_some() {
            error(
                "confirm",
                "asks a question, and the command is unattended",
                "Remove confirm, or remove unattended.",
            );
        }
        if self.mode.is_terminal() {
            error(
                "mode",
                "uses the terminal, and the command is unattended",
                "Use output-pane or run, or remove unattended.",
            );
        }
        if self.timeout.is_none() {
            error(
                "timeout",
                "is not set, and an unattended command must end",
                "Add a timeout such as timeout = \"5m\".",
            );
        }
        problems
    }

    /// The questions the host asks before a run, in order: the `confirm`
    /// question, then the `prompt` line.
    pub fn questions(&self) -> Vec<&str> {
        self.confirm
            .iter()
            .chain(self.prompt.iter())
            .map(String::as_str)
            .collect()
    }
}

/// Reads `exit_words`: whole-number keys from 1 to 255, each a sentence.
fn read_exit_words(table: &Table, r: &mut Reader<'_>) -> BTreeMap<i32, String> {
    let mut words = BTreeMap::new();
    for (code, sentence) in table {
        let parsed = code.parse::<i32>().ok().filter(|n| (1..=255).contains(n));
        match (parsed, sentence) {
            (Some(n), Value::String(text)) if !text.trim().is_empty() => {
                words.insert(n, text.trim().to_owned());
            }
            (None, _) => r.error(
                "exit_words",
                &format!("\"{code}\" is not an exit code"),
                "Use whole numbers from 1 to 255 as the keys.",
            ),
            _ => r.error(
                "exit_words",
                &format!("the words for {code} are not a sentence"),
                "Give each code a sentence in quotes.",
            ),
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Severity;

    fn read(text: &str) -> (Recipe, Vec<Problem>) {
        let table: Table = text.parse().unwrap();
        Recipe::read(&table, &|key: &str| key.to_owned())
    }

    #[test]
    fn defaults_hold_with_no_keys() {
        let (recipe, problems) = read("");
        assert_eq!(recipe, Recipe::default());
        assert!(problems.is_empty());
        assert!(recipe.cursor_fallback);
        assert_eq!(recipe.max_rows, 5000);
    }

    #[test]
    fn every_key_reads() {
        let (recipe, problems) = read(
            r#"
            mode = "lister"
            per = "entry"
            stdin = "none"
            cwd = "script"
            timeout = "60s"
            prompt = "Search the whole computer for:"
            dry_run = "supported"
            cursor_fallback = false
            exit_words = { 8 = "Everything is not running. Start it, then try again." }
            list = "nul"
            max_rows = 5000
            selection_file = "nul"
            interpreter = ["python3", "-I"]
            stdout_directives = true
            "#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(recipe.mode, RunMode::Lister);
        assert_eq!(recipe.per, Per::Entry);
        assert_eq!(recipe.stdin, StdinForm::None);
        assert_eq!(recipe.cwd, CwdChoice::Script);
        assert_eq!(recipe.timeout, Some(std::time::Duration::from_secs(60)));
        assert!(recipe.dry_run && !recipe.cursor_fallback && recipe.stdout_directives);
        assert_eq!(
            recipe.exit_words[&8],
            "Everything is not running. Start it, then try again."
        );
        assert_eq!(recipe.list, ListForm::Nul);
        assert_eq!(recipe.selection_file, ListForm::Nul);
        assert_eq!(recipe.interpreter, ["python3", "-I"]);
    }

    #[test]
    fn a_column_runs_per_entry() {
        assert_eq!(read("mode = \"output-column\"").0.per, Per::Entry);
    }

    #[test]
    fn wrong_values_name_the_field_and_the_fix() {
        let (_, problems) = read(
            "mode = \"loud\"\nunattended = true\nexit_words = { x = \"y\" }\nmax_rows = 0\ninterpreter = []",
        );
        let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
        assert!(
            text.iter().any(|t| t.starts_with(
                "Error, mode: \"loud\" is not a known value. Use one of: output-pane"
            )),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.starts_with("Error, unattended: is not \"ok\".")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t.contains("\"x\" is not an exit code")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.starts_with("Error, max_rows: 0 is out of range")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.starts_with("Error, interpreter: is empty")),
            "{text:?}"
        );
    }

    #[test]
    fn unattended_refuses_questions_terminals_and_no_timeout() {
        let (recipe, problems) = read(
            "unattended = \"ok\"\nprompt = \"Name?\"\nconfirm = \"Sure?\"\nmode = \"terminal\"",
        );
        assert!(recipe.unattended);
        let fields: Vec<&str> = problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(fields, ["prompt", "confirm", "mode", "timeout"]);
        assert!(problems.iter().all(|p| p.severity == Severity::Error));
        let (_, fine) = read("unattended = \"ok\"\ntimeout = \"5m\"");
        assert!(fine.is_empty(), "{fine:?}");
    }

    #[test]
    fn questions_come_in_order() {
        let (recipe, _) = read("confirm = \"Resize 3 images?\"\nprompt = \"New width:\"");
        assert_eq!(recipe.questions(), ["Resize 3 images?", "New width:"]);
    }
}
