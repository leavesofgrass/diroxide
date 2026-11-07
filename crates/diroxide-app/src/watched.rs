//! Watched folders in the app and the command line (alpha 4, lane AW, R30
//! commits 69 and 70; R28 6.3): the tasks as this computer has them,
//! checked by HL's engine, and the parts of a task the engine cannot
//! reach, given through its [`TaskActions`] trait: the recipe book, the
//! type registry, the filter language, a tool's version, and script
//! commands run unattended through the script host, whose questions
//! answer "no user present". A plugin's command is not run by a task in
//! this alpha: the task pauses and says so.
//!
//! The messages between the app's Watched folders list and its runtime
//! are here too: [`WatchedAsk`] and [`WatchedNote`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use diroxide_config::Paths;
use diroxide_core::filter::{Filter, FilterOptions, Lookup, Unknown};
use diroxide_core::{Clock, Entry, Message, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_ops::Engine;
use diroxide_ops::convert::{CartaRunner, Recipe, RecipeBook, Typing};
use diroxide_ops::hot::{
    ActionResult, ActionRun, CatchUpChoice, DryRun, HotNote, Resolved, RunnerOptions, Surroundings,
    Task, TaskActions, TaskRunner, TaskStore, check_tasks,
};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::CancelToken;

use crate::conversions::{RegistryTyping, recipe_book};

/// The tasks of this computer that may run, and every problem met while
/// reading and checking them, each a sentence naming its task first.
pub fn load_tasks(paths: &Paths) -> (Vec<(Task, PathBuf)>, Vec<String>) {
    let loaded = diroxide_config::task_files::load_tasks(paths);
    let mut problems: Vec<String> = loaded
        .problems
        .iter()
        .map(|problem| problem.to_string())
        .collect();
    let written: Vec<(String, diroxide_config::TaskConfig)> = loaded
        .value
        .iter()
        .map(|file| (file.file.clone(), file.task.clone()))
        .collect();
    let around = Surroundings::of_this_computer(&paths.config_dir, &paths.state_dir);
    let checked = check_tasks(&written, &around);
    problems.extend(checked.problems.iter().map(|p| p.line()));
    let tasks = checked
        .tasks
        .into_iter()
        .map(|task| {
            let file = diroxide_config::task_files::task_path(paths, &task.name);
            (task, file)
        })
        .collect();
    (tasks, problems)
}

/// A task's store, its state, ledger, log and lock, under the state
/// folder's `hot`.
pub fn store_of(paths: &Paths, task: &str) -> Result<TaskStore, String> {
    TaskStore::new(&paths.hot_dir(), task).map_err(|e| e.to_string())
}

/// How many files a task did and failed today, by the clock's day, from
/// its log: "3 done today, 1 failed" in the list's row.
pub fn today_tally(paths: &Paths, task: &str, clock: &dyn Clock) -> (usize, usize) {
    let Ok(store) = store_of(paths, task) else {
        return (0, 0);
    };
    let now = diroxide_ops::hot::log_time(clock.now(), clock.offset());
    let today = now.get(..10).unwrap_or_default();
    let (mut done, mut failed) = (0, 0);
    for line in store.log().newest(100_000) {
        let Some(when) = line.rsplit("  ").next() else {
            continue;
        };
        if !when.starts_with(today) {
            break;
        }
        if line.starts_with("done  ") {
            done += 1;
        } else if line.starts_with("failed  ") {
            failed += 1;
        }
    }
    (done, failed)
}

/// A runner for a task: its jobs on `engine` (the tools registry set),
/// the app's actions, the app's clock, carta in `carta` (a `diro` child
/// in the program), the spool in the state folder, and the summaries at
/// most once a `pace`.
pub fn runner_for(
    task: Task,
    engine: Engine,
    actions: Arc<dyn TaskActions>,
    paths: &Paths,
    clock: Arc<dyn Clock>,
    carta: CartaRunner,
    pace: Duration,
) -> Result<TaskRunner, String> {
    let store = store_of(paths, &task.name)?;
    let options = RunnerOptions {
        pace,
        ..RunnerOptions::default()
    };
    Ok(TaskRunner::new(task, engine, actions, store)
        .map_err(|e| e.to_string())?
        .with_clock(clock)
        .with_carta(carta)
        .with_spool(paths.spool_dir())
        .with_options(options))
}

/// A file's groups for the filter language's `group:`, from the type
/// registry, its own groups among them.
struct TaskLookup<'a> {
    types: &'a TypeRegistry,
    place: &'a Place,
}

impl Lookup for TaskLookup<'_> {
    fn path(&self, _entry: &Entry) -> Option<String> {
        Some(self.place.to_string())
    }

    fn group(&self, entry: &Entry) -> Result<Option<std::borrow::Cow<'_, str>>, Unknown> {
        let clock = diroxide_core::SystemClock::utc();
        self.types.groups_of(entry, &clock).map(|groups| {
            groups
                .first()
                .map(|g| std::borrow::Cow::Owned(g.id.clone()))
        })
    }

    fn in_group(&self, entry: &Entry, group: &str) -> Result<bool, Unknown> {
        let clock = diroxide_core::SystemClock::utc();
        self.types
            .groups_of(entry, &clock)
            .map(|groups| groups.iter().any(|g| g.id.eq_ignore_ascii_case(group)))
    }
}

/// What the app and the command line give a task's runner. See the
/// module documentation.
pub struct AppTaskActions {
    book: RecipeBook,
    typing: RegistryTyping,
    types: Arc<TypeRegistry>,
    versions: Option<ToolRegistry>,
    plugins_dir: Option<PathBuf>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for AppTaskActions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppTaskActions").finish_non_exhaustive()
    }
}

impl AppTaskActions {
    /// The actions of this computer: the recipe book with the owner's
    /// `conversions.toml`, the type registry, the tools registry for the
    /// versions a task's approval names (none in tests, so no version
    /// command runs), and the plugins folder for script commands.
    pub fn new(
        paths: Option<&Paths>,
        types: Arc<TypeRegistry>,
        versions: Option<ToolRegistry>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            book: recipe_book(paths),
            typing: RegistryTyping(Arc::clone(&types)),
            types,
            versions,
            plugins_dir: paths.map(Paths::plugins_dir),
            clock,
        }
    }

    /// The script command with this id, from the plugins folder's
    /// scripts.
    fn script(&self, id: &str) -> Result<diroxide_plugins::script::command::ScriptCommand, String> {
        let folder = self
            .plugins_dir
            .as_ref()
            .ok_or_else(|| format!("no script command is named {id}"))?;
        diroxide_plugins::script::installed::scan(folder, diroxide_plugins::manifest::Os::current())
            .iter()
            .filter_map(diroxide_plugins::script::installed::Installed::script_command)
            .find(|command| command.id == id)
            .ok_or_else(|| format!("no script command is named {id}"))
    }
}

/// The SHA-256 of a file in hexadecimal, for a script's approval key.
fn digest_of(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(diroxide_components::pin::sha256_hex(&bytes))
}

impl TaskActions for AppTaskActions {
    fn recipe(&self, id: &str) -> Option<Recipe> {
        self.book.get(id).cloned()
    }

    fn typing(&self) -> &dyn Typing {
        &self.typing
    }

    fn tool_version(&self, tool: &str) -> Option<String> {
        let registry = self.versions.as_ref()?;
        let spec = registry.spec(tool)?;
        if spec.version_args.is_empty() {
            return None;
        }
        let Ok(diroxide_tools::ToolStatus::Found(program)) = registry.find(tool) else {
            return None;
        };
        diroxide_tools::version::version_line(&program, spec.version_args)
            .and_then(|line| diroxide_tools::version::version_number(&line))
    }

    fn passes(&self, filter: &str, place: &Place, entry: &Entry) -> Result<bool, String> {
        let groups = self.types.group_names();
        let filter = Filter::parse_with_groups(filter, &FilterOptions::default(), &groups)
            .map_err(|e| e.to_string())?;
        let lookup = TaskLookup {
            types: &self.types,
            place,
        };
        Ok(filter
            .evaluator(self.clock.as_ref())
            .matches(entry, &lookup))
    }

    fn resolve_script(&self, id: &str) -> Result<Resolved, String> {
        let command = self.script(id)?;
        let file = match &command.program {
            diroxide_plugins::script::command::Program::Script(path) => path.clone(),
            _ => return Err(format!("{id} is not a script")),
        };
        Ok(Resolved::Script {
            id: id.to_owned(),
            digest: digest_of(&file)?,
        })
    }

    fn run_script(&self, run: &ActionRun, cancel: &CancelToken) -> ActionResult {
        let command = match self.script(&run.command) {
            Ok(command) => command,
            Err(why) => return ActionResult::Pause(why),
        };
        let Some(plugins_dir) = &self.plugins_dir else {
            return ActionResult::Pause("script commands need the config folder".to_owned());
        };
        let context = diroxide_plugins::script::context::RunContext::for_task(
            &run.task,
            run.input.clone(),
            run.out.clone(),
        );
        let state = plugins_dir.join("state");
        let env = diroxide_plugins::script::runner::RunEnv::system(
            diroxide_plugins::script::context::StateFolders {
                runs: state.join("runs"),
                data: state.join(&command.owner_id).join("data"),
            },
        );
        let stop = diroxide_plugins::script::runner::Cancel::new();
        let watcher = cancel.clone();
        let stopper = stop.clone();
        let _watch = std::thread::spawn(move || {
            while !stopper.is_cancelled() {
                if watcher.is_cancelled() {
                    stopper.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let mut sink = diroxide_plugins::script::runner::Recorder::default();
        let outcome =
            diroxide_plugins::script::runner::run(&command, &context, &env, &mut sink, &stop);
        let result = match outcome {
            Ok(report) => match report.ending {
                diroxide_plugins::script::words::Ending::Exited(0) => {
                    ActionResult::Made(Vec::new())
                }
                ending => ActionResult::Failed {
                    cause: format!("{ending:?}"),
                    retry: false,
                },
            },
            Err(error) => ActionResult::Failed {
                cause: error.sentence(&command.title),
                retry: false,
            },
        };
        stop.cancel();
        result
    }
}

/// One task as the Watched folders list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRow {
    /// The task's name, its file's stem.
    pub name: String,
    /// Its title, as said: "Scans".
    pub title: String,
    /// The Say rule sentence.
    pub sentence: String,
    /// Whether the owner turned it on.
    pub on: bool,
    /// Whether it runs now in this diroxide.
    pub running: bool,
    /// Why it is paused, when it is.
    pub paused: Option<String>,
    /// Its task file, for Edit.
    pub file: PathBuf,
}

/// What the Watched folders list asks of the runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchedAsk {
    /// The tasks, with their state.
    List,
    /// Start every task the owner turned on (at the app's start).
    StartOn,
    /// Turn a task on: with the approval key the owner said yes to, and
    /// the catch-up answer, when they were asked.
    TurnOn {
        /// The task.
        name: String,
        /// The approval key to record first.
        approve: Option<String>,
        /// The answer to the catch-up question.
        catch_up: Option<CatchUpChoice>,
    },
    /// Turn a task off.
    TurnOff(String),
    /// Pause a running task.
    Pause(String),
    /// Resume a paused task.
    Resume(String),
    /// What the task would do now, touching nothing.
    Preview(String),
    /// The task's log, newest first.
    Log(String),
    /// The files in the task's `failed` folder.
    Failed(String),
    /// Move these files from `failed` back to `in`.
    PutBack {
        /// The task.
        name: String,
        /// The files' names.
        names: Vec<String>,
    },
}

/// What the runtime answers the Watched folders list, and what a running
/// task tells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchedNote {
    /// The tasks, and the problems met reading them.
    Listed {
        /// The tasks.
        rows: Vec<TaskRow>,
        /// The problems, each naming its task.
        problems: Vec<String>,
        /// The tasks folder, for "no tasks" and Edit.
        folder: PathBuf,
    },
    /// The task needs the owner's approval first.
    AskApproval {
        /// The task.
        name: String,
        /// The Say rule sentence.
        sentence: String,
        /// The question, ending "y or n".
        question: String,
        /// The key a yes records.
        key: String,
    },
    /// The task asks about the files waiting in `in`.
    AskCatchUp {
        /// The task.
        name: String,
        /// "Process the 14 files in Scans? y or n".
        question: String,
    },
    /// What a running task told.
    Hot {
        /// The task.
        name: String,
        /// The note.
        note: HotNote,
    },
    /// The task stopped running here.
    Stopped(String),
    /// A dry run's lines and summary.
    Preview {
        /// The task.
        name: String,
        /// What it found.
        dry: DryRun,
    },
    /// The task's log, newest first.
    Log {
        /// The task.
        name: String,
        /// The lines.
        lines: Vec<String>,
    },
    /// The files in `failed`.
    Failed {
        /// The task.
        name: String,
        /// The files' names.
        names: Vec<String>,
    },
    /// Lines to say: a put back's, or a refusal's.
    Said(Vec<Message>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{EntryKind, FixedClock, Timestamp, UtcOffset};

    fn actions() -> AppTaskActions {
        AppTaskActions::new(
            None,
            Arc::new(TypeRegistry::builtin()),
            None,
            Arc::new(FixedClock::new(
                Timestamp::from_civil(2026, 11, 7, 9, 0, 0, UtcOffset::UTC).unwrap(),
                UtcOffset::UTC,
            )),
        )
    }

    /// The parts a task asks for: a recipe by id, the filter language with
    /// the type registry's groups, and no script without a config folder;
    /// no version without a registry, so nothing runs.
    #[test]
    fn a_task_gets_recipes_the_filter_and_says_what_it_lacks() {
        let actions = actions();
        assert!(actions.recipe("markdown-to-org").is_some());
        assert!(actions.recipe("no-such").is_none());
        let place =
            Place::parse_with("/in/photo.png", diroxide_core::place::PathStyle::Unix).unwrap();
        let entry = Entry::new("photo.png", EntryKind::File).with_size(200_000);
        assert_eq!(
            actions.passes("group:images and size over 100 KB", &place, &entry),
            Ok(true)
        );
        assert_eq!(actions.passes("group:music", &place, &entry), Ok(false));
        assert!(actions.passes("size over", &place, &entry).is_err());
        assert_eq!(
            actions.resolve_script("script.tidy"),
            Err("no script command is named script.tidy".to_owned())
        );
        assert_eq!(actions.tool_version("magick"), None);
        let kind = actions.typing().kind_of("photo.png", &[]);
        assert_eq!(kind.type_id.as_deref(), Some("png"));
    }
}
