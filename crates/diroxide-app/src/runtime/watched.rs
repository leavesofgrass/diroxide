//! The watched folders' tasks in the runtime (alpha 4, lane AW, R30
//! commit 70): each task the owner turns on runs on its own thread over
//! HL's engine (`spawn_task`), its notes sent to the app as they come;
//! the list's asks (turn on or off, pause, resume, preview, the log, the
//! failed files, put back) are carried out here.
//!
//! A task's jobs run on an engine with no undo journal, so nothing a
//! watched folder did is ever undone by Ctrl+Z, now or after a restart
//! (HL's request); its jobs carry `JobOrigin::HotFolder` besides. carta
//! runs in a `diro` child when the frontend gives one
//! ([`Runtime::with_carta_child`](super::Runtime::with_carta_child)), in
//! process otherwise (tests).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use diroxide_core::Message;
use diroxide_ops::Engine;
use diroxide_ops::convert::CartaRunner;
use diroxide_ops::hot::{StartError, TaskRun, TaskRunner, sentence, spawn_task};
use diroxide_tools::ToolRegistry;

use super::Runtime;
use crate::app::AppEvent;
use crate::watched::{
    AppTaskActions, TaskRow, WatchedAsk, WatchedNote, WatchedRequest, load_tasks, runner_for,
    store_of, today_tally,
};

/// The summaries' pace: at most one line this often (the progress
/// interval's default).
const PACE: Duration = Duration::from_secs(10);

/// The tasks running in this runtime, and how they run.
#[derive(Debug, Default)]
pub(super) struct WatchedRuntime {
    /// The running tasks, by name.
    runs: HashMap<String, TaskRun>,
    /// Where a task's carta runs.
    pub(super) carta: CartaRunner,
    /// The tools registry for the versions an approval names; none in
    /// tests, so no version command runs.
    pub(super) versions: Option<ToolRegistry>,
}

/// What the runtime sends for a watched folder's note.
fn note(note: WatchedNote) -> AppEvent {
    AppEvent::HotFolder(Box::new(note))
}

impl Runtime {
    /// Carries out one of the Watched folders list's asks.
    pub(super) fn watched(&mut self, request: WatchedRequest) {
        let Some(paths) = self.paths.clone() else {
            if request.ask != WatchedAsk::StartOn {
                self.queue
                    .push_back(note(WatchedNote::Said(vec![Message::info(
                        "Watched folders need the config folder",
                    )])));
            }
            return;
        };
        // A run whose thread ended is not running.
        self.hot.runs.retain(|_, run| !run.has_ended());
        match request.ask.clone() {
            WatchedAsk::List => {
                let (tasks, problems) = load_tasks(&paths);
                let rows = tasks
                    .into_iter()
                    .map(|(task, file)| {
                        let state = store_of(&paths, &task.name)
                            .map(|store| store.state())
                            .unwrap_or_default();
                        let (done_today, failed_today) =
                            today_tally(&paths, &task.name, request.clock.0.as_ref());
                        TaskRow {
                            done_today,
                            failed_today,
                            title: task.title(),
                            sentence: sentence(&task),
                            on: state.on,
                            running: self.hot.runs.contains_key(&task.name),
                            paused: state.paused.clone(),
                            name: task.name,
                            file,
                        }
                    })
                    .collect();
                let folder = paths.config_dir.join(diroxide_config::Paths::TASKS);
                self.queue.push_back(note(WatchedNote::Listed {
                    rows,
                    problems,
                    folder,
                }));
            }
            WatchedAsk::StartOn => {
                let (tasks, _) = load_tasks(&paths);
                for (task, _) in tasks {
                    let on = store_of(&paths, &task.name).is_ok_and(|s| s.state().on);
                    if on && !self.hot.runs.contains_key(&task.name) {
                        self.start_task(&task.name, None, None, &request);
                    }
                }
            }
            WatchedAsk::TurnOn {
                name,
                approve,
                catch_up,
            } => self.start_task(&name, approve.as_deref(), catch_up, &request),
            WatchedAsk::TurnOff(name) => {
                if let Some(run) = self.hot.runs.remove(&name) {
                    run.stop();
                }
                if let Ok(store) = store_of(&paths, &name) {
                    let _ = store.set_on(false);
                }
                self.queue.push_back(note(WatchedNote::Stopped(name)));
            }
            WatchedAsk::Pause(name) => match self.hot.runs.get(&name) {
                Some(run) => run.pause("paused by you"),
                None => self.say_not_running(&name),
            },
            WatchedAsk::Resume(name) => match self.hot.runs.get(&name) {
                Some(run) => run.resume(),
                None => self.start_task(&name, None, None, &request),
            },
            WatchedAsk::Preview(name) => {
                let result = self.task_runner(&name, &request).and_then(|r| r.preview());
                self.queue.push_back(note(match result {
                    Ok(dry) => WatchedNote::Preview { name, dry },
                    Err(why) => WatchedNote::Said(vec![Message::error(format!(
                        "Could not preview {name}: {why}."
                    ))]),
                }));
            }
            WatchedAsk::Log(name) => {
                let lines = store_of(&paths, &name)
                    .map(|store| store.log().newest(200))
                    .unwrap_or_default();
                self.queue.push_back(note(WatchedNote::Log { name, lines }));
            }
            WatchedAsk::Failed(name) => {
                let names = self
                    .task_runner(&name, &request)
                    .map(|runner| failed_names(&runner))
                    .unwrap_or_default();
                self.queue
                    .push_back(note(WatchedNote::Failed { name, names }));
            }
            WatchedAsk::PutBack { name, names } => {
                // The running task stops while its files come back, so its
                // ledger and the put back's never disagree; it starts again.
                let was_running = self.hot.runs.remove(&name).is_some();
                let said = match self.task_runner(&name, &request) {
                    Ok(mut runner) => runner.put_back(&names),
                    Err(why) => vec![why],
                };
                self.queue.push_back(note(WatchedNote::Said(
                    said.into_iter().map(Message::info).collect(),
                )));
                if was_running {
                    self.start_task(&name, None, None, &request);
                }
            }
        }
    }

    /// "Scans is not running".
    fn say_not_running(&mut self, name: &str) {
        self.queue
            .push_back(note(WatchedNote::Said(vec![Message::info(format!(
                "{} is not running",
                diroxide_ops::hot::task::title(name)
            ))])));
    }

    /// A runner for a task, its jobs on an engine without an undo journal.
    fn task_runner(&self, name: &str, request: &WatchedRequest) -> Result<TaskRunner, String> {
        let paths = self
            .paths
            .as_ref()
            .ok_or_else(|| "there is no config folder".to_owned())?;
        let (tasks, problems) = load_tasks(paths);
        let Some((task, _)) = tasks.into_iter().find(|(t, _)| t.name == name) else {
            let prefix = format!("Task {name}:");
            return Err(problems
                .into_iter()
                .find(|p| p.starts_with(&prefix))
                .unwrap_or_else(|| format!("Could not run {name}: no task is named {name}.")));
        };
        let engine = Engine::new(Arc::clone(&self.registry))
            .with_options(self.engine.options().clone())
            .with_tools(Arc::clone(&request.tools.0));
        let actions = Arc::new(AppTaskActions::new(
            Some(paths),
            Arc::clone(&request.types.0),
            self.hot.versions.clone(),
            Arc::clone(&request.clock.0),
        ));
        runner_for(
            task,
            engine,
            actions,
            paths,
            Arc::clone(&request.clock.0),
            self.hot.carta.clone(),
            PACE,
        )
    }

    /// Turns a task on: the approval and the catch-up answer recorded when
    /// given, then the run on its own thread; or the question it needs, or
    /// why it cannot run.
    fn start_task(
        &mut self,
        name: &str,
        approve: Option<&str>,
        catch_up: Option<diroxide_ops::hot::CatchUpChoice>,
        request: &WatchedRequest,
    ) {
        let runner = match self.task_runner(name, request) {
            Ok(runner) => runner,
            Err(why) => {
                self.queue
                    .push_back(note(WatchedNote::Said(vec![Message::error(why)])));
                return;
            }
        };
        let store = runner.store().clone();
        let recorded = approve
            .map(|key| store.approve(key, request.clock.0.now()))
            .transpose()
            .and_then(|_| catch_up.map(|c| store.remember_catch_up(c)).transpose())
            .and_then(|_| store.set_on(true));
        if let Err(error) = recorded {
            self.queue
                .push_back(note(WatchedNote::Said(vec![Message::error(format!(
                    "Could not turn on {}: {error}.",
                    runner.title()
                ))])));
            return;
        }
        let task_sentence = sentence(runner.task());
        let shared = Arc::clone(&self.shared);
        let owner = name.to_owned();
        let sink = Box::new(move |hot| {
            shared.send(note(WatchedNote::Hot {
                name: owner.clone(),
                note: hot,
            }));
        });
        match spawn_task(runner, Arc::clone(&request.clock.0), sink) {
            Ok(run) => {
                self.hot.runs.insert(name.to_owned(), run);
            }
            Err(StartError::NotApproved { question, key, .. }) => {
                let _ = store.set_on(false);
                self.queue.push_back(note(WatchedNote::AskApproval {
                    name: name.to_owned(),
                    sentence: task_sentence,
                    question,
                    key,
                }));
            }
            Err(StartError::AskCatchUp { question, .. }) => {
                self.queue.push_back(note(WatchedNote::AskCatchUp {
                    name: name.to_owned(),
                    question,
                }));
            }
            Err(error) => {
                self.queue
                    .push_back(note(WatchedNote::Said(vec![Message::error(
                        error.to_string(),
                    )])));
            }
        }
    }
}

/// The names in a task's `failed` folder, in name order.
fn failed_names(runner: &TaskRunner) -> Vec<String> {
    let failed = &runner.task().folders.failed;
    let mut names: Vec<String> = runner
        .engine()
        .registry()
        .resolve(failed)
        .and_then(|p| p.list(failed, &diroxide_vfs::OpContext::new()))
        .map(|entries| {
            entries
                .into_iter()
                .filter(|e| e.kind == diroxide_core::EntryKind::File)
                .map(|e| e.name)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}
