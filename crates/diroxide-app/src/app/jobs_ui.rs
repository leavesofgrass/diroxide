//! The Jobs list (R25, part 1, 2.2; lane A2, commit 79): the queue is the
//! jobs list. Three groups, each under a heading that says its count
//! ("Running, 2 jobs", "Waiting, 1 job", "Finished, 5 jobs"), every row the
//! state first ("Copying 40 percent: 12 items to build", "Waiting 2 of 5:
//! copy 3 items to build", "Done: copy 12 items to build").
//!
//! - **Enter** opens the row's actions: Pause or Resume, Run now, Move to
//!   the front, Move up, Move down, Move to the end, Set a speed limit,
//!   the error policy of a waiting job, Cancel; on a finished row Show what
//!   failed and Retry the failed items; then the queue as a whole: Pause
//!   or Resume the queue, Pause all jobs, Resume all jobs. Every one of
//!   those is in the palette too, so no new key chord is needed (R25 N6).
//! - **Delete** cancels a waiting job without a question, and asks first
//!   for a running one, starting on No.
//! - The list follows the jobs while it is open: a row keeps its focus by
//!   the job's id from group to group.

use std::collections::HashMap;

use diroxide_core::job::ErrorPolicy;
use diroxide_core::{
    Command, ConflictPolicy, Effect, JobId, JobKind, JobRequest, JobState, Key, Layer, ListModel,
    ListRow, Message, Redraw,
};
use diroxide_ops::SpeedLimit;

use super::{App, Purpose, Then};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::{FinishedJob, JobFacts, JobGroup, QueueNote, QueueRequest, short_verb};
use crate::keymap::Lookup;
use crate::request::Request;

/// What the Jobs list keeps between its dialogs.
#[derive(Debug, Default)]
pub(super) struct JobsState {
    /// What each planned job is on, from its request, until it runs.
    facts: HashMap<JobId, JobFacts>,
    /// Each queued job's error policy, as the request or the owner set it.
    policies: HashMap<JobId, ErrorPolicy>,
    /// A job that moved in line: its new place is said when the queue's
    /// next view comes.
    moved: Option<JobId>,
}

/// What a dialog of the Jobs list is for.
#[derive(Clone, Debug)]
pub(super) enum Wants {
    /// The list itself.
    List,
    /// A job's actions.
    Actions {
        /// The job.
        job: JobId,
    },
    /// "Cancel copy 12 items to build? No is the default. y or n".
    ConfirmCancel {
        /// The job.
        job: JobId,
    },
    /// The speed limit typed for a job, or for every remote job.
    SpeedLimit {
        /// The job, or `None` for the global limit.
        job: Option<JobId>,
    },
    /// A waiting job's error policy.
    ErrorPolicy {
        /// The job.
        job: JobId,
    },
}

/// The kinds a retry can run again from their own sources.
fn retries(kind: JobKind) -> bool {
    matches!(
        kind,
        JobKind::Copy | JobKind::Move | JobKind::Trash | JobKind::DeletePermanently
    )
}

/// The row id of a job, so the focus follows it from group to group.
fn row_id(job: JobId) -> String {
    format!("job:{}", job.get())
}

/// The job a row id names.
fn job_of(row: &str) -> Option<JobId> {
    row.strip_prefix("job:")
        .and_then(|n| n.parse().ok())
        .map(JobId::new)
}

impl App {
    /// Notes what a job is on as it is planned, for its rows and lines,
    /// and its error policy.
    pub(super) fn note_job_facts(&mut self, request: &JobRequest) {
        self.jobs_ui.facts.insert(
            request.id,
            JobFacts::of(&request.sources, request.destination.as_ref()),
        );
        self.jobs_ui.policies.insert(request.id, request.errors);
    }

    /// Takes what a job is on, as it starts.
    pub(super) fn take_job_facts(&mut self, job: JobId) -> JobFacts {
        self.jobs_ui.facts.remove(&job).unwrap_or_default()
    }

    /// The job in words, its verb first: "copy 12 items to build".
    pub(super) fn job_name(&self, job: JobId) -> String {
        let (kind, what) = match (self.jobs.kind_of(job), self.jobs.what_of(job)) {
            (Some(kind), Some(what)) => (kind, what.to_owned()),
            _ => match self.jobs.finished_job(job) {
                Some(done) => (done.kind, done.what.clone()),
                None => (JobKind::Unknown, String::new()),
            },
        };
        if what.is_empty() {
            short_verb(kind).to_owned()
        } else {
            format!("{} {what}", short_verb(kind))
        }
    }

    /// The Jobs list's commands and the queue's, from a menu or the
    /// palette.
    pub(super) fn jobs_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let id = &command.id;
        let effects = if *id == ids::JOBS_LIST {
            self.open_jobs_list(None)
        } else if *id == ids::JOB_HISTORY {
            self.open_jobs_list(Some(JobGroup::Finished))
        } else if *id == ids::JOBS_PAUSE_QUEUE {
            self.queue_wide("queue-pause")
        } else if *id == ids::JOBS_RESUME_QUEUE {
            self.queue_wide("queue-resume")
        } else if *id == ids::JOBS_PAUSE_ALL {
            self.queue_wide("pause-all")
        } else if *id == ids::JOBS_RESUME_ALL {
            self.queue_wide("resume-all")
        } else if *id == ids::JOBS_SPEED_LIMIT {
            self.speed_limit_field(None)
        } else if *id == ids::JOBS_RETRY_FAILED {
            self.retry_failed(None)
        } else {
            return None;
        };
        Some(effects)
    }

    /// What the queue said: a job started or waits, its new order. The
    /// line is said, and an open Jobs list follows.
    pub(super) fn queue_note(&mut self, note: QueueNote) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw(Redraw::JobsLine)];
        if let Some(line) = self.jobs.queue_note(&note) {
            effects.push(self.say(Message::info(line)));
        }
        if matches!(note, QueueNote::View { .. })
            && let Some(job) = self.jobs_ui.moved.take()
            && let Some(row) = self.jobs.row_of(job)
        {
            effects.push(self.say(Message::info(row)));
        }
        effects.extend(self.refresh_jobs_list());
        effects
    }

    /// The rows of the Jobs list, each group under its heading; `None`
    /// when no job runs, waits or ran.
    fn jobs_list_model(&self) -> Option<ListModel> {
        let rows = self.jobs.rows();
        if rows.is_empty() {
            return None;
        }
        let mut list_rows = Vec::new();
        for group in [JobGroup::Running, JobGroup::Waiting, JobGroup::Finished] {
            let in_group: Vec<_> = rows.iter().filter(|r| r.group == group).collect();
            if in_group.is_empty() {
                continue;
            }
            let heading = match group {
                JobGroup::Running => "group:running",
                JobGroup::Waiting => "group:waiting",
                JobGroup::Finished => "group:finished",
            };
            list_rows.push(ListRow::new(heading, group.heading(in_group.len())));
            list_rows.extend(
                in_group
                    .into_iter()
                    .map(|row| ListRow::new(row_id(row.job), row.text.clone())),
            );
        }
        let mut list = ListModel::new("Jobs").with_rows(list_rows);
        let delete = self.keymap.key_text(&ids::TRASH);
        let help = if delete.is_empty() {
            "Up and Down choose, Enter opens a job's actions, Escape closes.".to_owned()
        } else {
            format!(
                "Up and Down choose, Enter opens a job's actions, {delete} cancels a job, Escape closes."
            )
        };
        list.set_help(help);
        Some(list)
    }

    /// Opens the Jobs list, the focus on the job in front, or on the first
    /// job of a group.
    pub(super) fn open_jobs_list(&mut self, focus: Option<JobGroup>) -> Vec<Effect> {
        let Some(mut list) = self.jobs_list_model() else {
            return vec![self.say(Message::info("No jobs have run yet."))];
        };
        let first_of = |group: JobGroup| {
            self.jobs
                .rows()
                .into_iter()
                .find(|row| row.group == group)
                .map(|row| row.job)
        };
        let target = match focus {
            Some(group) => first_of(group).or_else(|| self.jobs.front()),
            None => self.jobs.front().or_else(|| first_of(JobGroup::Finished)),
        };
        match target {
            Some(job) => {
                list.focus_id(&row_id(job));
            }
            None => {
                list.move_focus(diroxide_core::Move::To(1));
            }
        }
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::JOBS_LIST),
            },
            Purpose::Jobs(Box::new(Wants::List)),
        )
    }

    /// Makes every open Jobs list follow the jobs, keeping its focus on
    /// the same job.
    pub(super) fn refresh_jobs_list(&mut self) -> Vec<Effect> {
        let open: Vec<usize> = self
            .opens
            .iter()
            .enumerate()
            .filter(
                |(_, open)| matches!(&open.purpose, Purpose::Jobs(w) if matches!(**w, Wants::List)),
            )
            .map(|(at, _)| at)
            .collect();
        if open.is_empty() {
            return Vec::new();
        }
        let Some(fresh) = self.jobs_list_model() else {
            return Vec::new();
        };
        for at in open {
            if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(at) {
                list.set_rows(fresh.rows().to_vec());
            }
        }
        vec![Effect::Redraw(Redraw::All)]
    }

    /// A job's actions, as a list: what fits its state, then the queue's.
    fn job_actions(&mut self, job: JobId) -> Vec<Effect> {
        let waiting = self.jobs.waits(job);
        let running = !waiting && self.jobs.ids().contains(&job);
        let finished = self.jobs.finished_job(job).cloned();
        let mut rows = Vec::new();
        let title;
        if waiting {
            title = self.jobs.row_of(job).unwrap_or_default();
            rows.push(ListRow::new("run-now", "Run now"));
            rows.push(ListRow::new("front", "Move to the front"));
            rows.push(ListRow::new("up", "Move up"));
            rows.push(ListRow::new("down", "Move down"));
            rows.push(ListRow::new("end", "Move to the end"));
            rows.push(ListRow::new("speed", "Set a speed limit"));
            let policy = self.jobs_ui.policies.get(&job).copied().unwrap_or_default();
            rows.push(ListRow::new(
                "policy",
                format!("On errors: {}", policy.label().to_lowercase()),
            ));
            rows.push(ListRow::new("cancel", "Cancel this job"));
        } else if running {
            title = self.jobs.row_of(job).unwrap_or_default();
            if self.jobs.state_of(job) == Some(JobState::Paused) {
                rows.push(ListRow::new("resume", "Resume"));
            } else {
                rows.push(ListRow::new("pause", "Pause"));
            }
            rows.push(ListRow::new("speed", "Set a speed limit"));
            rows.push(ListRow::new("cancel", "Cancel this job"));
        } else if let Some(done) = &finished {
            title = done.row();
            rows.push(ListRow::new("failed", "Show what failed"));
            let mut retry = ListRow::new("retry", "Retry the failed items");
            retry.enabled = retries(done.kind) && !done.retry_sources.is_empty();
            rows.push(retry);
        } else {
            return vec![self.say(Message::info("That job is gone."))];
        }
        if self.jobs.queue_paused() {
            rows.push(ListRow::new("queue-resume", "Resume the queue"));
        } else {
            rows.push(ListRow::new("queue-pause", "Pause the queue"));
        }
        rows.push(ListRow::new("pause-all", "Pause all jobs"));
        rows.push(ListRow::new("resume-all", "Resume all jobs"));
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_help("Up and Down choose, Enter does it, Escape goes back to the jobs.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::JOBS_LIST),
            },
            Purpose::Jobs(Box::new(Wants::Actions { job })),
        )
    }

    /// Enter in one of the Jobs list's dialogs.
    pub(super) fn jobs_accept(
        &mut self,
        wants: Wants,
        row: Option<String>,
        text: String,
    ) -> Vec<Effect> {
        match wants {
            Wants::List => {
                let Some(row) = row else {
                    return Vec::new();
                };
                match job_of(&row) {
                    Some(job) => self.job_actions(job),
                    None => {
                        // A heading: the focus goes to its first job.
                        if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut() {
                            list.move_focus(diroxide_core::Move::Down);
                        }
                        self.list_focus_line()
                    }
                }
            }
            Wants::Actions { job } => {
                let Some(action) = row else {
                    return Vec::new();
                };
                self.job_action(job, &action)
            }
            Wants::SpeedLimit { job } => self.speed_limit_typed(job, &text),
            Wants::ErrorPolicy { job } => {
                let policy = match row.as_deref() {
                    Some("continue") => ErrorPolicy::ContinueAndList,
                    Some("stop") => ErrorPolicy::StopAtFirst,
                    _ => ErrorPolicy::Ask,
                };
                let mut effects = self.close_top();
                self.jobs_ui.policies.insert(job, policy);
                self.request(Request::Queue(QueueRequest::ErrorPolicy { job, policy }));
                let line = format!("On errors: {}", policy.label().to_lowercase());
                effects.push(self.say(Message::info(line)));
                effects
            }
            Wants::ConfirmCancel { .. } => Vec::new(),
        }
    }

    /// Says the focused row of the top list.
    fn list_focus_line(&mut self) -> Vec<Effect> {
        let line = match self.dialogs.last() {
            Some((_, dialog)) => dialog.focus_line(),
            None => return Vec::new(),
        };
        vec![
            Effect::Redraw(Redraw::All),
            Effect::Announce(Message::status(line)),
        ]
    }

    /// One of a job's actions.
    fn job_action(&mut self, job: JobId, action: &str) -> Vec<Effect> {
        let name = self.job_name(job);
        let queue = |app: &mut App, request: QueueRequest| app.request(Request::Queue(request));
        match action {
            "failed" => {
                let index = self.jobs.history().iter().rposition(|f| f.job == job);
                let mut effects = self.close_top();
                if let Some(index) = index {
                    effects.extend(self.job_items(index));
                }
                effects
            }
            "retry" => {
                let mut effects = self.close_all();
                effects.extend(self.retry_failed(Some(job)));
                effects
            }
            "speed" => {
                let mut effects = self.close_top();
                effects.extend(self.speed_limit_field(Some(job)));
                effects
            }
            "policy" => {
                let current = self.jobs_ui.policies.get(&job).copied().unwrap_or_default();
                let rows = ErrorPolicy::ALL
                    .iter()
                    .map(|policy| {
                        let id = match policy {
                            ErrorPolicy::Ask => "ask",
                            ErrorPolicy::ContinueAndList => "continue",
                            ErrorPolicy::StopAtFirst => "stop",
                        };
                        ListRow::new(id, policy.label())
                    })
                    .collect();
                let mut list = ListModel::new(format!("On errors: {name}")).with_rows(rows);
                if let Some(at) = ErrorPolicy::ALL.iter().position(|p| *p == current) {
                    list.move_focus(diroxide_core::Move::To(at));
                }
                list.set_help("Up and Down choose, Enter sets it, Escape keeps it.");
                let mut effects = self.close_top();
                effects.extend(self.open_dialog(
                    Dialog::Choose {
                        list,
                        on_enter: Command::new(ids::JOBS_LIST),
                    },
                    Purpose::Jobs(Box::new(Wants::ErrorPolicy { job })),
                ));
                effects
            }
            "cancel" => {
                let mut effects = self.close_top();
                effects.extend(self.cancel_job(job));
                effects
            }
            "pause" | "resume" => {
                let mut effects = self.close_top();
                let (request, word) = if action == "pause" {
                    (QueueRequest::Pause(job), "Pausing")
                } else {
                    (QueueRequest::Resume(job), "Resuming")
                };
                queue(self, request);
                effects.push(self.say(Message::info(format!("{word} {name}"))));
                effects
            }
            "run-now" => {
                let effects = self.close_top();
                queue(self, QueueRequest::RunNow(job));
                effects
            }
            "front" | "up" | "down" | "end" => {
                let effects = self.close_top();
                let request = match action {
                    "front" => QueueRequest::MoveToFront(job),
                    "up" => QueueRequest::MoveUp(job),
                    "down" => QueueRequest::MoveDown(job),
                    _ => QueueRequest::MoveToEnd(job),
                };
                self.jobs_ui.moved = Some(job);
                queue(self, request);
                effects
            }
            other => {
                let mut effects = self.close_top();
                effects.extend(self.queue_wide(other));
                effects
            }
        }
    }

    /// The queue as a whole: pause or resume the queue, pause or resume
    /// every job.
    fn queue_wide(&mut self, action: &str) -> Vec<Effect> {
        let (request, line) = match action {
            "queue-pause" => (
                QueueRequest::PauseQueue,
                "Queue paused: nothing new starts.",
            ),
            "queue-resume" => (QueueRequest::ResumeQueue, "Queue goes on."),
            "pause-all" => (QueueRequest::PauseAll, "All jobs paused."),
            "resume-all" => (QueueRequest::ResumeAll, "All jobs go on."),
            _ => return Vec::new(),
        };
        self.request(Request::Queue(request));
        vec![self.say(Message::info(line))]
    }

    /// Cancels a job: a waiting one at once, a running one after a
    /// question that starts on No.
    pub(super) fn cancel_job(&mut self, job: JobId) -> Vec<Effect> {
        if self.jobs.waits(job) {
            return vec![Effect::CancelJob(job)];
        }
        if !self.jobs.ids().contains(&job) {
            return vec![self.say(Message::info("That job has finished."))];
        }
        let name = self.job_name(job);
        let question = with_default_no(Message::question(&format!("Cancel {name}")));
        let listing = self.jobs.row_of(job).into_iter().collect();
        self.open_dialog(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Purpose::Jobs(Box::new(Wants::ConfirmCancel { job })),
        )
    }

    /// Yes in one of the Jobs list's confirmations; the dialog is closed.
    pub(super) fn jobs_yes(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::ConfirmCancel { job } => {
                let name = self.job_name(job);
                vec![
                    Effect::CancelJob(job),
                    self.say(Message::info(format!("Cancelling: {name}"))),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Escape in one of the Jobs list's dialogs; the dialog is closed.
    pub(super) fn jobs_escape(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::ConfirmCancel { .. } => vec![self.say(Message::info("Not cancelled."))],
            Wants::SpeedLimit { .. } => vec![self.say(Message::info("Speed limit unchanged."))],
            _ => Vec::new(),
        }
    }

    /// The Jobs list's own keys: the trash key cancels the focused job,
    /// a waiting one at once and a running one after a question.
    pub(super) fn jobs_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let Some(Purpose::Jobs(wants)) = self.opens.last().map(|o| &o.purpose) else {
            return None;
        };
        if !matches!(**wants, Wants::List) {
            return None;
        }
        let layers = [Layer::Panel, Layer::List, Layer::Global];
        let cancels = matches!(
            self.keymap.lookup(&layers, key),
            Lookup::Command(id) if id == ids::TRASH || id == ids::DELETE_PERMANENTLY
        );
        if !cancels {
            return None;
        }
        let row = match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => list.focused().map(|r| r.id.clone()),
            _ => None,
        };
        let Some(job) = row.as_deref().and_then(job_of) else {
            return Some(vec![self.say(Message::status("Choose a job first."))]);
        };
        if self.jobs.finished_job(job).is_some() && !self.jobs.ids().contains(&job) {
            return Some(vec![self.say(Message::info("That job has finished."))]);
        }
        Some(self.cancel_job(job))
    }

    /// The speed limit field, for a job or for every remote job.
    fn speed_limit_field(&mut self, job: Option<JobId>) -> Vec<Effect> {
        let label = match job {
            Some(_) => "Speed limit for this job, such as 500 KB",
            None => "Speed limit of remote jobs, such as 500 KB",
        };
        let field = EntryField::new(label, CompletionSource::None);
        self.open_dialog(
            Dialog::edit(field),
            Purpose::Jobs(Box::new(Wants::SpeedLimit { job })),
        )
    }

    /// A typed speed limit: a number and a unit, or off.
    fn speed_limit_typed(&mut self, job: Option<JobId>, text: &str) -> Vec<Effect> {
        let typed = text.trim();
        let limit = if typed.is_empty() || typed.eq_ignore_ascii_case("off") || typed == "0" {
            None
        } else {
            match SpeedLimit::parse(typed) {
                Some(limit) => Some(limit),
                None => {
                    return vec![
                        self.say(Message::error("Not a speed: type 500 KB, 2 MB or off.")),
                    ];
                }
            }
        };
        let mut effects = self.close_top();
        match job {
            Some(job) => self.request(Request::Queue(QueueRequest::SpeedLimit {
                job: Some(job),
                limit,
            })),
            None => effects.extend(self.set_global_speed_limit(limit)),
        }
        let line = limit.map_or_else(|| "Speed limit off".to_owned(), SpeedLimit::line);
        effects.push(self.say(Message::info(line)));
        effects
    }

    /// The global speed limit, for the runtime now.
    pub(super) fn set_global_speed_limit(&mut self, limit: Option<SpeedLimit>) -> Vec<Effect> {
        self.request(Request::Queue(QueueRequest::SpeedLimit {
            job: None,
            limit,
        }));
        Vec::new()
    }

    /// Retry the failed items: a new job over the job's own sources that
    /// hold what failed, skipping what is already there, planned and asked
    /// as any job is. With no job named, the newest job that had failures.
    pub(super) fn retry_failed(&mut self, job: Option<JobId>) -> Vec<Effect> {
        let done: Option<FinishedJob> = match job {
            Some(job) => self.jobs.finished_job(job).cloned(),
            None => self
                .jobs
                .history()
                .iter()
                .rev()
                .find(|f| !f.failed.is_empty())
                .cloned(),
        };
        let Some(done) = done else {
            return vec![self.say(Message::info("Nothing to retry: no job failed."))];
        };
        if !retries(done.kind) {
            let verb = short_verb(done.kind);
            return vec![self.say(Message::info(format!(
                "Retry is not offered for {verb} jobs."
            )))];
        }
        if done.retry_sources.is_empty() {
            return vec![self.say(Message::info("Nothing to retry: nothing failed."))];
        }
        let new = self.next_job_id();
        let mut request = JobRequest::new(new, done.kind, done.retry_sources.clone());
        if let Some(destination) = &done.destination {
            request = request.to(destination.clone());
        }
        request.conflicts = ConflictPolicy::Skip;
        let side = self.active;
        let note = Some("Retry: what is already there is skipped".to_owned());
        self.plan_job(side, request, note, Then::Report, true)
    }

    /// Say Status's words for the jobs that wait their turn.
    pub(super) fn waiting_status(&self) -> Vec<String> {
        self.jobs
            .rows()
            .into_iter()
            .filter(|row| row.group == JobGroup::Waiting)
            .map(|row| row.text)
            .collect()
    }

    /// The facts of a job whose plan is run now, for its rows.
    pub(super) fn facts_for_run(&mut self, job: JobId) -> JobFacts {
        self.take_job_facts(job)
    }

    /// Whether a job's start may be held back by the queue: a remote job,
    /// a component download, or any job while the queue is paused.
    pub(super) fn may_wait(&self, plan: &diroxide_ops::Plan) -> bool {
        self.jobs.queue_paused()
            || matches!(
                diroxide_ops::JobClass::of_plan(plan),
                diroxide_ops::JobClass::Remote(_) | diroxide_ops::JobClass::Component
            )
    }
}
