//! Tasks: one TOML file each in the config folder's `tasks` folder
//! ([`Paths::TASKS`]), `tasks/NAME.toml` (plan question 52; R28 6.3). A
//! hot folder is a task whose trigger is a folder, written in this format
//! from the start, so synchronization, rename and delete tasks of later
//! alphas add their own keys to the same shape. The state of a task (on
//! or off, paused, approved, its ledger, log and lock) is never in its
//! file: it lives under `hot/` in the state folder.
//!
//! The words a task file takes are closed lists: an unknown word is a
//! parse error that names the file and the line. Unattended runs never
//! ask and never overwrite, so `conflicts` has no `ask` or `overwrite`.
//!
//! The contract step of alpha 4 (P4) fixes the keys; the hot-folder lane
//! adds the loader, its validation in words and the sentence generator.

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::paths::Paths;

/// What a task does with each file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskKind {
    /// Runs a conversion recipe ([`TaskConfig::recipe`]).
    #[default]
    Convert,
    /// Moves the file to [`TaskConfig::to`].
    Move,
    /// Copies the file to [`TaskConfig::to`].
    Copy,
    /// Runs a script command that declares `unattended = true`
    /// ([`TaskConfig::script`]).
    Script,
    /// Runs a plugin's command ([`TaskConfig::plugin`]).
    Plugin,
    /// Only logs and announces the arrival.
    Tell,
}

/// What starts a task.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerKind {
    /// A file that arrives in a folder and stops changing.
    #[default]
    Folder,
}

/// What the first turn-on does with the files already waiting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatchUp {
    /// Asks once, starting on No above 20 files.
    #[default]
    Ask,
    /// Processes every waiting file.
    All,
    /// Leaves them; only new arrivals are processed.
    None,
}

/// What an unattended run does when the output's name is taken: never
/// ask, never overwrite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskConflicts {
    /// Keeps both, the new one numbered: "report (2).pdf" (answer 55).
    #[default]
    KeepBoth,
    /// Leaves the file, said in the log.
    Skip,
    /// Makes the output only when the input is newer.
    Newer,
}

/// What to do with a file that is online only (a cloud placeholder).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnlineOnly {
    /// Leaves it, said in the log.
    #[default]
    Skip,
    /// Lets the system fetch it first.
    Fetch,
}

/// What the task says out loud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Announce {
    /// One polite line for everything since the last, at most every
    /// progress interval: "Scans: 3 done, 1 failed".
    #[default]
    Summary,
    /// A line for each file.
    Each,
    /// Only failures.
    Failures,
    /// Nothing; the log and the Watched folders list still say it.
    Off,
}

/// `[trigger]`: when the task runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TriggerConfig {
    /// What starts it.
    pub kind: TriggerKind,
    /// The folder that holds `in`, `out`, `processed` and `failed`, a
    /// literal absolute path: no wildcards, no variables, no `~`, no bare
    /// drive letter.
    pub home: String,
    /// Name patterns a file must match, after it settles and its type is
    /// checked.
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    /// A filter expression a file must pass instead of, or beside, the
    /// patterns: `group images and size over 100 KB`.
    pub filter: Option<String>,
    /// How long a file must stop changing, as written: `2s`.
    pub settle: String,
    /// What the first turn-on does with waiting files.
    pub catch_up: CatchUp,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            kind: TriggerKind::Folder,
            home: String::new(),
            matches: Vec::new(),
            filter: None,
            settle: "2s".to_owned(),
            catch_up: CatchUp::Ask,
        }
    }
}

/// `[run]`: how each file is handled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RunConfig {
    /// Where the original goes after a run: `move` (to `processed`, or to
    /// `failed` when the run fails), `leave`, or a named folder; never a
    /// removal.
    pub after: String,
    /// What a taken output name gets.
    pub conflicts: TaskConflicts,
    /// How many files run at once.
    pub parallel: u32,
    /// How long one file may take, as written: `10m`.
    pub timeout: Option<String>,
    /// How many times a file error the error's class allows retrying is
    /// tried again.
    pub retries: u32,
    /// What to do with an online-only file.
    pub online_only: OnlineOnly,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            after: "move".to_owned(),
            conflicts: TaskConflicts::KeepBoth,
            parallel: 1,
            timeout: None,
            retries: 3,
            online_only: OnlineOnly::Skip,
        }
    }
}

/// `[say]`: what the task says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SayConfig {
    /// How often, and about what.
    pub announce: Announce,
}

/// One task file, `tasks/NAME.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TaskConfig {
    /// Its name, plain (letters, digits, `-` and `_`), the file's stem:
    /// `scans`.
    pub name: String,
    /// What it does, in one sentence, meaning first.
    pub about: String,
    /// What it does with each file.
    pub kind: TaskKind,
    /// A conversion recipe's id, for [`TaskKind::Convert`].
    pub recipe: Option<String>,
    /// A script command's id, for [`TaskKind::Script`].
    pub script: Option<String>,
    /// A plugin command's id, for [`TaskKind::Plugin`].
    pub plugin: Option<String>,
    /// The folder a move or copy puts the file in.
    pub to: Option<String>,
    /// When it runs.
    pub trigger: TriggerConfig,
    /// How each file is handled.
    pub run: RunConfig,
    /// What it says.
    pub say: SayConfig,
}

impl TaskConfig {
    /// Reads a task file's text; `file` names it in a parse error
    /// (`tasks/scans.toml`).
    pub fn from_toml(text: &str, file: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::parse(file, e))
    }

    /// The file a task of this name lives in, under the config folder:
    /// `tasks/scans.toml`.
    pub fn file_name(name: &str) -> String {
        format!("{}/{name}.toml", Paths::TASKS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R28 6.3's example reads, its defaults fill the rest, and `ask`
    /// is not a word an unattended run takes.
    #[test]
    fn the_scans_task_reads_and_an_unattended_ask_is_refused() {
        let text = r#"
name = "scans"
about = "Makes a text file from each scanned PDF."
kind = "convert"
recipe = "pdf-to-text"

[trigger]
kind = "folder"
home = 'D:\scans'
match = ["*.pdf"]
settle = "2s"
catch-up = "ask"

[run]
after = "move"
conflicts = "keep-both"
parallel = 1
timeout = "10m"
retries = 3
online-only = "skip"

[say]
announce = "summary"
"#;
        let task = TaskConfig::from_toml(text, &TaskConfig::file_name("scans")).unwrap();
        assert_eq!(task.kind, TaskKind::Convert);
        assert_eq!(task.trigger.home, "D:\\scans");
        assert_eq!(task.trigger.matches, ["*.pdf"]);
        assert_eq!(task.run.conflicts, TaskConflicts::KeepBoth);
        assert_eq!(task.say.announce, Announce::Summary);
        let bare =
            TaskConfig::from_toml("name = \"tell\"\nkind = \"tell\"\n", "tasks/tell.toml").unwrap();
        assert_eq!(bare.trigger.settle, "2s");
        assert_eq!(bare.run.after, "move");
        assert_eq!(bare.run.retries, 3);
        let asked = text.replace("conflicts = \"keep-both\"", "conflicts = \"ask\"");
        let error = TaskConfig::from_toml(&asked, "tasks/scans.toml").unwrap_err();
        assert!(error.to_string().contains("tasks/scans.toml"), "{error}");
        assert_eq!(TaskConfig::file_name("scans"), "tasks/scans.toml");
    }
}
