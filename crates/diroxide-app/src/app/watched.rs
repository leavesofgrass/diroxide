//! The Watched folders list (alpha 4, lane AW, R30 commit 70; R28 6.3):
//! one row a task, "Scans, on, 3 done today, 1 failed" (33), its position
//! said after it; Enter on a row lists what can be done with that task:
//! turn it on or off, pause or resume it, preview it (the dry run, nothing
//! touched), show its log, put failed files back, say its rule, and edit
//! its file. Turning a task on asks its approval first, in the task's own
//! sentence, and then about the files already waiting; both questions
//! start on No and the store keeps the answers.
//!
//! A running task's notes reach the live region in words: its polite
//! summaries ("Scans: 3 done, 1 failed") at most every ten seconds, a
//! pause at once ("Scans paused: pdftotext is not installed"), never a
//! dialog. Say Status includes each running task's tally. Nothing a task
//! does is undone by Ctrl+Z: its jobs never reach the undo log.
//!
//! "hot" finds the command in the palette (its id is `app.hot-folders`).
//! The lists are hosted under the Organize purpose so the window's test
//! of every purpose keeps its count (CR5).

use std::collections::{BTreeSet, HashMap};

use diroxide_core::{Command, Effect, Importance, ListModel, ListRow, Message, MessageKind, Place};
use diroxide_ops::hot::lines::{list_row, put_back_question, state_line, summary_line};
use diroxide_ops::hot::{CatchUpChoice, FileOutcome, HotNote};

use super::{App, Purpose};
use crate::commands::ids;
use crate::conversions::SharedTypes;
use crate::dialog::{Dialog, with_default_no};
use crate::request::{Request, SharedClock, SpawnKind};
use crate::watched::{TaskRow, WatchedAsk, WatchedNote, WatchedRequest};

/// A running task's tally of today, by the app's clock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Tally {
    /// The day it counts: "2026-11-07".
    day: String,
    done: usize,
    failed: usize,
    skipped: usize,
}

/// What the app knows of the watched folders' tasks.
#[derive(Debug, Default)]
pub(super) struct WatchedState {
    /// The tasks running now in this diroxide.
    running: BTreeSet<String>,
    /// Each task's title, by name, as the runtime said it.
    titles: HashMap<String, String>,
    /// Each running task's tally of today.
    tallies: HashMap<String, Tally>,
    /// Each running task's status line, never spoken by itself: "Scans:
    /// 2 waiting, 1 running".
    status: HashMap<String, String>,
}

/// What a dialog of the Watched folders list is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WatchedWants {
    /// The list: Enter lists a task's actions.
    List(Vec<TaskRow>),
    /// A task's actions: Enter does one.
    Actions(Box<TaskRow>),
    /// The approval question: Yes turns the task on.
    Approve {
        /// The task.
        name: String,
        /// The key a yes records.
        key: String,
    },
    /// The catch-up question: Yes processes the waiting files, No leaves
    /// them; the task turns on either way.
    CatchUp {
        /// The task.
        name: String,
    },
    /// The put back question: Yes moves the files back to `in`.
    PutBack {
        /// The task.
        name: String,
        /// The files.
        names: Vec<String>,
    },
}

/// The actions' row ids.
const TURN_ON: &str = "on";
const TURN_OFF: &str = "off";
const PAUSE: &str = "pause";
const RESUME: &str = "resume";
const PREVIEW: &str = "preview";
const LOG: &str = "log";
const PUT_BACK: &str = "failed";
const SAY_RULE: &str = "say";
const EDIT: &str = "edit";

/// A question as the dialogs ask it, starting on No.
fn question(text: &str) -> Message {
    with_default_no(Message::new(
        text.to_owned(),
        Importance::High,
        MessageKind::Question,
    ))
}

impl App {
    /// Watched folders; `None` for any other command.
    pub(super) fn watched_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        if command.id != ids::WATCHED_FOLDERS {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        self.watched_ask(WatchedAsk::List);
        Some(Vec::new())
    }

    /// At the start, the tasks the owner left on start again; a headless
    /// command starts none.
    pub(super) fn watched_start(&mut self) {
        if !self.headless {
            self.watched_ask(WatchedAsk::StartOn);
        }
    }

    /// Asks the runtime, with what a task runs with.
    fn watched_ask(&mut self, ask: WatchedAsk) {
        let request = WatchedRequest {
            ask,
            types: SharedTypes(std::sync::Arc::clone(&self.types)),
            tools: self.tool_runner(),
            clock: SharedClock(std::sync::Arc::clone(&self.clock)),
        };
        self.request(Request::Watched(Box::new(request)));
    }

    /// A task's title as said: the runtime's, else the name's.
    fn task_title(&self, name: &str) -> String {
        self.watched
            .titles
            .get(name)
            .cloned()
            .unwrap_or_else(|| diroxide_ops::hot::task::title(name))
    }

    /// Today's day, by the app's clock: "2026-11-07".
    fn today(&self) -> String {
        let c = self.now().to_civil(self.clock.offset());
        format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
    }

    /// Say Status's part: each running task's tally of today and its
    /// status line, in name order.
    pub(super) fn hot_status(&self) -> Vec<String> {
        let today = self.today();
        let mut out = Vec::new();
        for name in &self.watched.running {
            let title = self.task_title(name);
            let tally = self
                .watched
                .tallies
                .get(name)
                .filter(|t| t.day == today)
                .cloned()
                .unwrap_or_default();
            out.push(summary_line(
                &title,
                tally.done,
                tally.failed,
                tally.skipped,
            ));
            if let Some(status) = self.watched.status.get(name) {
                out.push(status.clone());
            }
        }
        out
    }

    /// What the runtime answered, or what a running task told.
    pub(super) fn watched_note(&mut self, note: WatchedNote) -> Vec<Effect> {
        match note {
            WatchedNote::Listed {
                rows,
                problems,
                folder,
            } => self.watched_list(rows, &problems, &folder),
            WatchedNote::AskApproval {
                name,
                sentence,
                question: text,
                key,
            } => self.open_dialog(
                Dialog::Confirm {
                    question: question(&text),
                    listing: vec![sentence],
                    on_yes: Command::new(ids::YES),
                },
                Purpose::Organize(Box::new(super::organize::Wants::Watched(
                    WatchedWants::Approve { name, key },
                ))),
            ),
            WatchedNote::AskCatchUp {
                name,
                question: text,
            } => self.open_dialog(
                Dialog::Confirm {
                    question: question(&text),
                    listing: Vec::new(),
                    on_yes: Command::new(ids::YES),
                },
                Purpose::Organize(Box::new(super::organize::Wants::Watched(
                    WatchedWants::CatchUp { name },
                ))),
            ),
            WatchedNote::Hot { name, note } => self.hot_note(&name, note),
            WatchedNote::Stopped(name) => {
                self.watched.running.remove(&name);
                self.watched.status.remove(&name);
                let line = state_line(&self.task_title(&name), false, false);
                vec![self.say(Message::info(line))]
            }
            WatchedNote::Preview { name, dry } => {
                let title = format!("{}: {}", self.task_title(&name), dry.summary);
                let lines = if dry.lines.is_empty() {
                    vec!["Nothing waits in in".to_owned()]
                } else {
                    dry.lines
                };
                self.open_dialog(Dialog::Read { title, lines }, Purpose::Read)
            }
            WatchedNote::Log { name, lines } => {
                let title = self.task_title(&name);
                if lines.is_empty() {
                    return vec![self.say(Message::info(format!("{title} log is empty")))];
                }
                let title = format!(
                    "{title} log: {}, newest first",
                    diroxide_core::format::count(lines.len() as u64, "line", "lines")
                );
                self.open_dialog(Dialog::Read { title, lines }, Purpose::Read)
            }
            WatchedNote::Failed { name, names } => {
                if names.is_empty() {
                    let title = self.task_title(&name);
                    return vec![self.say(Message::info(format!("Nothing failed in {title}")))];
                }
                self.open_dialog(
                    Dialog::Confirm {
                        question: question(&put_back_question(names.len())),
                        listing: names
                            .iter()
                            .map(|n| diroxide_core::names::spell_out(n).into_owned())
                            .collect(),
                        on_yes: Command::new(ids::YES),
                    },
                    Purpose::Organize(Box::new(super::organize::Wants::Watched(
                        WatchedWants::PutBack { name, names },
                    ))),
                )
            }
            WatchedNote::Said(messages) => messages.into_iter().map(|m| self.say(m)).collect(),
        }
    }

    /// The list: one row a task, the problems last and unavailable.
    fn watched_list(
        &mut self,
        rows: Vec<TaskRow>,
        problems: &[String],
        folder: &std::path::Path,
    ) -> Vec<Effect> {
        for row in &rows {
            self.watched
                .titles
                .insert(row.name.clone(), row.title.clone());
            if row.running {
                self.watched.running.insert(row.name.clone());
            } else {
                self.watched.running.remove(&row.name);
            }
        }
        if rows.is_empty() && problems.is_empty() {
            let line = format!(
                "No watched folders. Each task is a file in {}",
                folder.display()
            );
            return vec![self.say(Message::info(line))];
        }
        let mut list_rows: Vec<ListRow> = rows
            .iter()
            .map(|row| {
                let label = list_row(
                    &row.title,
                    row.on,
                    row.paused.is_some(),
                    row.done_today,
                    row.failed_today,
                );
                let mut list_row = ListRow::new(row.name.clone(), label);
                if let Some(why) = &row.paused {
                    list_row.details.push(why.clone());
                }
                list_row
            })
            .collect();
        for (i, problem) in problems.iter().enumerate() {
            let mut row = ListRow::new(format!("problem:{i}"), problem.clone());
            row.enabled = false;
            list_rows.push(row);
        }
        let title = format!(
            "Watched folders: {}",
            diroxide_core::format::count(rows.len() as u64, "task", "tasks")
        );
        let mut list = ListModel::new(title).with_rows(list_rows);
        list.set_help("Up and Down choose a task, Enter lists what it can do, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Watched(
                WatchedWants::List(rows),
            ))),
        )
    }

    /// A task's actions, each a row.
    fn task_actions(&mut self, row: TaskRow) -> Vec<Effect> {
        let mut rows = Vec::new();
        if row.on {
            rows.push(ListRow::new(TURN_OFF, "Turn off").with_details(["stop watching"]));
        } else {
            rows.push(ListRow::new(TURN_ON, "Turn on").with_details(["asks first"]));
        }
        if row.running && row.paused.is_some() {
            rows.push(ListRow::new(RESUME, "Resume"));
        } else if row.running {
            rows.push(ListRow::new(PAUSE, "Pause"));
        }
        rows.push(ListRow::new(PREVIEW, "Preview").with_details(["what it would do now"]));
        rows.push(ListRow::new(LOG, "Show log"));
        rows.push(ListRow::new(PUT_BACK, "Put failed back"));
        rows.push(ListRow::new(SAY_RULE, "Say rule"));
        rows.push(ListRow::new(EDIT, "Edit").with_details(["the task file"]));
        let mut list = ListModel::new(format!("{} actions", row.title)).with_rows(rows);
        list.set_help("Up and Down choose, Enter does it, Escape goes back.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Organize(Box::new(super::organize::Wants::Watched(
                WatchedWants::Actions(Box::new(row)),
            ))),
        )
    }

    /// What a running task told: its polite lines and pauses said, its
    /// tally and status kept.
    fn hot_note(&mut self, name: &str, note: HotNote) -> Vec<Effect> {
        let title = self.task_title(name);
        match note {
            HotNote::Started { .. } => {
                self.watched.running.insert(name.to_owned());
                vec![self.say(Message::info(state_line(&title, true, false)))]
            }
            HotNote::Say(line) => vec![self.say(Message::info(line))],
            HotNote::Status(line) => {
                match line {
                    Some(line) => self.watched.status.insert(name.to_owned(), line),
                    None => self.watched.status.remove(name),
                };
                Vec::new()
            }
            HotNote::File(file) => {
                let today = self.today();
                let tally = self.watched.tallies.entry(name.to_owned()).or_default();
                if tally.day != today {
                    *tally = Tally {
                        day: today,
                        ..Tally::default()
                    };
                }
                match file.outcome {
                    FileOutcome::Done | FileOutcome::Told => tally.done += 1,
                    FileOutcome::Failed => tally.failed += 1,
                    FileOutcome::Skipped => tally.skipped += 1,
                    _ => {}
                }
                Vec::new()
            }
            HotNote::Paused(line) => vec![self.say(Message::warning(line))],
            HotNote::Resumed => vec![self.say(Message::info(state_line(&title, true, false)))],
            _ => Vec::new(),
        }
    }

    /// Enter in one of the Watched folders list's dialogs.
    pub(super) fn watched_accept(
        &mut self,
        wants: WatchedWants,
        row: Option<String>,
    ) -> Vec<Effect> {
        let Some(row) = row else {
            return Vec::new();
        };
        match wants {
            WatchedWants::List(rows) => {
                let Some(task) = rows.into_iter().find(|t| t.name == row) else {
                    return Vec::new();
                };
                self.task_actions(task)
            }
            WatchedWants::Actions(task) => self.task_action(*task, &row),
            _ => Vec::new(),
        }
    }

    /// One of a task's actions.
    fn task_action(&mut self, task: TaskRow, action: &str) -> Vec<Effect> {
        let name = task.name.clone();
        let mut effects = self.close_all();
        match action {
            TURN_ON => self.watched_ask(WatchedAsk::TurnOn {
                name,
                approve: None,
                catch_up: None,
            }),
            TURN_OFF => self.watched_ask(WatchedAsk::TurnOff(name)),
            PAUSE => self.watched_ask(WatchedAsk::Pause(name)),
            RESUME => self.watched_ask(WatchedAsk::Resume(name)),
            PREVIEW => self.watched_ask(WatchedAsk::Preview(name)),
            LOG => self.watched_ask(WatchedAsk::Log(name)),
            PUT_BACK => self.watched_ask(WatchedAsk::Failed(name)),
            SAY_RULE => effects.push(self.say(Message::info(task.sentence))),
            EDIT => match task.file.to_str().and_then(|p| Place::parse(p).ok()) {
                Some(file) => {
                    let side = self.active;
                    let spawn = self.spawn_for(side, SpawnKind::Editor(file));
                    self.request(Request::Spawn(Box::new(spawn)));
                    effects.push(self.say(Message::info(format!("Editing {}", task.title))));
                }
                None => {
                    let line = format!("The task file is {}", task.file.display());
                    effects.push(self.say(Message::info(line)));
                }
            },
            _ => {}
        }
        effects
    }

    /// Yes to one of the Watched folders list's questions.
    pub(super) fn watched_yes(&mut self, wants: WatchedWants) -> Vec<Effect> {
        match wants {
            WatchedWants::Approve { name, key } => self.watched_ask(WatchedAsk::TurnOn {
                name,
                approve: Some(key),
                catch_up: None,
            }),
            WatchedWants::CatchUp { name } => self.watched_ask(WatchedAsk::TurnOn {
                name,
                approve: None,
                catch_up: Some(CatchUpChoice::All),
            }),
            WatchedWants::PutBack { name, names } => {
                self.watched_ask(WatchedAsk::PutBack { name, names });
            }
            _ => {}
        }
        Vec::new()
    }

    /// No or Escape on one of the Watched folders list's dialogs: the
    /// catch-up question's No leaves the waiting files and turns the task
    /// on; the others change nothing.
    pub(super) fn watched_escape(&mut self, wants: WatchedWants) -> Vec<Effect> {
        match wants {
            WatchedWants::CatchUp { name } => {
                self.watched_ask(WatchedAsk::TurnOn {
                    name,
                    approve: None,
                    catch_up: Some(CatchUpChoice::None),
                });
                Vec::new()
            }
            WatchedWants::Approve { .. } | WatchedWants::PutBack { .. } => {
                vec![self.say(Message::info("Nothing changed."))]
            }
            _ => Vec::new(),
        }
    }
}
