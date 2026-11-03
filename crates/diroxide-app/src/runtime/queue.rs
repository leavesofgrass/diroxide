//! The transfer queue on the runtime's side (R25, part 1, 2.2; lane A2,
//! commit 79): a confirmed plan goes to the engine's scheduler
//! ([`diroxide_ops::Queue`]) and runs when the queue grants it a start.
//!
//! - **A waiting job is data:** its plan is held here, and its thread is
//!   made only on [`Grant::Start`]. A local job starts at once, as alpha 1
//!   does; a remote one takes a slot on its host (two per host, four in
//!   all); a component download runs alone; a plugin's command is never
//!   queued.
//! - **A question gives its slot back** ([`Queue::asking`]): the answer is
//!   held until the queue grants [`Grant::Continue`], then delivered. An
//!   answer that cancels the job goes at once, so a cancel never waits for
//!   a slot. The connection question (R25, 2.4) is never counted: its job
//!   probes for every job on a lost host, and holding its answer could
//!   hold them all.
//! - **Every change is told:** [`QueueNote::Started`] when a held job
//!   starts, [`QueueNote::Waits`] when a job waits its turn, and a
//!   [`QueueNote::View`] with the order after any change, so the app's
//!   jobs line and Jobs list say what the queue holds.
//! - **Pause holds a waiting job** in its place (lane Q2):
//!   [`QueueNote::Held`] and [`QueueNote::Released`] say it, and Pause
//!   all jobs and Resume all jobs hold and let go every waiting one with
//!   the queue.
//! - **The queue is saved across restarts** (lane Q2, `saved_queue.rs`):
//!   after every change the waiting, held and running jobs, each with its
//!   request when it came through a plan request, and the interrupted
//!   jobs not yet retried or forgotten, go to `queue.json` off this
//!   thread. At start they come back as [`QueueNote::Interrupted`];
//!   nothing starts on its own.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant};

use diroxide_core::{ConflictAction, ErrorAction, JobId, JobKind, JobRequest, JobState, Message};
use diroxide_ops::{Cancelled, Grant, Plan, Queue, QueueLimits};

use super::saved_queue::{QueueFile, SavedJob, SavedState};
use crate::jobs::{InterruptedJob, QueueNote, job_object};
use crate::request::{AskId, Reply};

/// How many planned requests are kept for their run: a plan the owner
/// says No to is never run, so the oldest go first.
const PLANNED_KEPT: usize = 64;

/// What the saved queue keeps of a job the queue holds.
#[derive(Clone, Debug)]
struct Kept {
    kind: JobKind,
    /// "12 items to build".
    what: String,
    /// Its request, when it came through a plan request.
    request: Option<JobRequest>,
}

/// The queue, the plans and answers it holds, and its clock; and the
/// saved queue (lane Q2): what each job is, the interrupted jobs of the
/// last sessions, and the file.
#[derive(Debug)]
pub(super) struct Scheduler {
    queue: Queue,
    /// Confirmed plans that wait for their start.
    plans: HashMap<JobId, Plan>,
    /// Answers that wait for their job's slot.
    answers: HashMap<JobId, (AskId, Reply)>,
    /// The queue's clock starts with the runtime.
    epoch: Instant,
    /// The requests planned and not yet run, newest last.
    planned: VecDeque<JobRequest>,
    /// What each job the queue holds is, for the saved queue.
    kept: HashMap<JobId, Kept>,
    /// The interrupted jobs, each with its number for this session.
    interrupted: Vec<(u64, SavedJob)>,
    /// New jobs that retry an interrupted one, by its number: it leaves
    /// the list when the new job is confirmed.
    retrying: HashMap<JobId, u64>,
    /// The saved queue's file, once the state folder is known.
    file: Option<QueueFile>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            queue: Queue::new(QueueLimits::default()),
            plans: HashMap::new(),
            answers: HashMap::new(),
            epoch: Instant::now(),
            planned: VecDeque::new(),
            kept: HashMap::new(),
            interrupted: Vec::new(),
            retrying: HashMap::new(),
            file: None,
        }
    }
}

/// What the runtime does after a change in the queue: start these plans,
/// deliver these answers, tell the app these notes.
#[derive(Debug, Default)]
pub(super) struct Granted {
    /// Plans to run now, each on its own worker.
    pub start: Vec<Plan>,
    /// Answers to deliver now.
    pub answers: Vec<(AskId, Reply)>,
    /// What to tell the app, in order.
    pub notes: Vec<QueueNote>,
}

impl Granted {
    /// Adds what another change granted, after this one's.
    fn extend(&mut self, other: Granted) {
        self.start.extend(other.start);
        self.answers.extend(other.answers);
        self.notes.extend(other.notes);
    }
}

/// Whether an answer ends its job, so it is delivered at once.
fn cancels(reply: &Reply) -> bool {
    match reply {
        Reply::Conflict(answer) => answer.action == ConflictAction::Cancel,
        Reply::Error(action) => *action == ErrorAction::Cancel,
        Reply::Secret(None) | Reply::Confirm(false) => true,
        _ => false,
    }
}

impl Scheduler {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    /// Whether the queue holds a job.
    pub(super) fn holds(&self, job: JobId) -> bool {
        self.queue.state(job).is_some()
    }

    /// The queue's state of a job, when it holds it.
    pub(super) fn state(&self, job: JobId) -> Option<JobState> {
        self.queue.state(job)
    }

    /// A confirmed plan: it starts now, or waits its turn. A plan that
    /// retries an interrupted job takes that job off the list (lane Q2).
    pub(super) fn submit(&mut self, plan: Plan) -> Granted {
        let job = plan.job;
        let kind = plan.kind;
        let class = diroxide_ops::JobClass::of_plan(&plan);
        let request = self.take_planned(job);
        let what = request
            .as_ref()
            .map(|r| job_object(&r.sources, r.destination.as_ref()))
            .unwrap_or_default();
        self.kept.insert(
            job,
            Kept {
                kind,
                what,
                request,
            },
        );
        let mut forgotten = None;
        if let Some(key) = self.retrying.remove(&job) {
            self.interrupted.retain(|(k, _)| *k != key);
            forgotten = Some(QueueNote::Forgotten(key));
        }
        self.plans.insert(job, plan);
        let now = self.now();
        let grants = self.queue.submit(job, kind, class, now);
        let starts = grants.contains(&Grant::Start(job));
        let mut granted = self.grant(grants);
        if !starts {
            granted.notes.insert(0, QueueNote::Waits(job));
        }
        granted.notes.extend(forgotten);
        granted.notes.push(self.changed());
        granted
    }

    /// A job's thread ended: its slots go to the next in line.
    pub(super) fn finished(&mut self, job: JobId) -> Granted {
        self.answers.remove(&job);
        let was_held = self.holds(job);
        let grants = self.queue.finished(job);
        self.kept.remove(&job);
        let mut granted = self.grant(grants);
        if was_held {
            granted.notes.push(self.changed());
        }
        granted
    }

    // ----- the saved queue (lane Q2) -----

    /// Opens the saved queue in the state folder: what it held comes back
    /// as interrupted jobs, told to the app once, and a file that does
    /// not read is said. Returns the notes and the problem to tell.
    pub(super) fn open_file(
        &mut self,
        folder: &Path,
        report: Box<dyn Fn(Message) + Send>,
    ) -> (Option<QueueNote>, Option<Message>) {
        let (file, loaded) = QueueFile::open(folder, report);
        self.file = file;
        let start = self.interrupted.len() as u64;
        self.interrupted = loaded
            .jobs
            .into_iter()
            .enumerate()
            .map(|(at, job)| (start + at as u64 + 1, job))
            .collect();
        // Written back at once with every job interrupted, so a second
        // crash does not count them as new again.
        self.save();
        let note = (!self.interrupted.is_empty()).then(|| QueueNote::Interrupted {
            jobs: self.interrupted_jobs(),
            new: loaded.new,
            by_quit: loaded.by_quit,
        });
        (note, loaded.problem)
    }

    /// Keeps a planned request until its plan is run, so the saved queue
    /// can retry it.
    pub(super) fn planned(&mut self, request: &JobRequest) {
        self.planned.retain(|r| r.id != request.id);
        self.planned.push_back(request.clone());
        while self.planned.len() > PLANNED_KEPT {
            self.planned.pop_front();
        }
    }

    /// The request a plan came from, taken for its run.
    pub(super) fn take_planned(&mut self, job: JobId) -> Option<JobRequest> {
        let at = self.planned.iter().position(|r| r.id == job)?;
        self.planned.remove(at)
    }

    /// Forgets an interrupted job: it leaves the list and the file.
    pub(super) fn forget(&mut self, key: u64) -> Granted {
        let before = self.interrupted.len();
        self.interrupted.retain(|(k, _)| *k != key);
        if self.interrupted.len() == before {
            return Granted::default();
        }
        self.save();
        Granted {
            notes: vec![QueueNote::Forgotten(key)],
            ..Granted::default()
        }
    }

    /// A new job retries an interrupted one: when it is confirmed, the
    /// interrupted one leaves the list.
    pub(super) fn retried(&mut self, key: u64, job: JobId) {
        self.retrying.insert(job, key);
    }

    /// Quit: the queue as it stands is written, marked as a quit, and
    /// nothing more is, so the jobs the quit stops are the ones the next
    /// start lists.
    pub(super) fn freeze_for_quit(&mut self) {
        let jobs = self.snapshot();
        if let Some(file) = &mut self.file {
            file.freeze(&jobs);
        }
    }

    /// Waits for the saved queue's writes, at most `limit`; whether they
    /// are on the disk (true with no file).
    #[cfg(test)]
    pub(super) fn flush(&self, limit: Duration) -> bool {
        self.file.as_ref().is_none_or(|file| file.flush(limit))
    }

    /// The interrupted jobs as the app lists them.
    fn interrupted_jobs(&self) -> Vec<InterruptedJob> {
        self.interrupted
            .iter()
            .map(|(key, job)| InterruptedJob {
                key: *key,
                kind: job.kind,
                what: job.what.clone(),
                started: job.started,
                request: job.request.clone(),
            })
            .collect()
    }

    /// Every job the saved queue keeps, in order: the interrupted ones
    /// first, then the queue's, each where it is.
    fn snapshot(&self) -> Vec<SavedJob> {
        let mut jobs: Vec<SavedJob> = self
            .interrupted
            .iter()
            .map(|(_, job)| job.clone())
            .collect();
        for (job, kind, _) in self.queue.jobs() {
            let waiting = self.plans.get(&job);
            let state = match waiting {
                Some(_) if self.queue.is_held(job) => SavedState::Held,
                Some(_) => SavedState::Waiting,
                None => SavedState::Running,
            };
            let kept = self.kept.get(&job);
            let mut request = kept.and_then(|k| k.request.clone());
            if let (Some(request), Some(plan)) = (request.as_mut(), waiting) {
                request.errors = plan.facts.error_policy;
            }
            jobs.push(SavedJob {
                job,
                kind: kept.map_or(kind, |k| k.kind),
                state,
                started: waiting.is_none(),
                what: kept.map(|k| k.what.clone()).unwrap_or_default(),
                request,
            });
        }
        jobs
    }

    /// Writes the saved queue as it stands, off this thread.
    fn save(&mut self) {
        if self.file.is_none() {
            return;
        }
        let jobs = self.snapshot();
        if let Some(file) = &mut self.file {
            file.save(&jobs);
        }
    }

    /// After a change: the saved queue written, and the queue's view.
    fn changed(&mut self) -> QueueNote {
        self.save();
        self.view()
    }

    /// A job asks a question: its slots go to the next in line.
    pub(super) fn asking(&mut self, job: JobId) -> Granted {
        let grants = self.queue.asking(job);
        self.grant(grants)
    }

    /// The answer to a question a job asked: delivered when its job holds
    /// a slot again, or at once when the queue does not hold the job or
    /// the answer ends it. `None` means deliver it now.
    pub(super) fn answer(&mut self, job: Option<JobId>, ask: AskId, reply: Reply) -> Granted {
        let Some(job) = job.filter(|job| self.state(*job) == Some(JobState::WaitingForAnswer))
        else {
            return Granted {
                answers: vec![(ask, reply)],
                ..Granted::default()
            };
        };
        if cancels(&reply) {
            return Granted {
                answers: vec![(ask, reply)],
                ..Granted::default()
            };
        }
        self.answers.insert(job, (ask, reply));
        let grants = self.queue.answered(job);
        self.grant(grants)
    }

    /// Cancels a job: a waiting one leaves at once and its plan is given
    /// back (it never ran); for a running one the caller cancels its
    /// control.
    pub(super) fn cancel(&mut self, job: JobId) -> (Option<Plan>, Granted) {
        self.answers.remove(&job);
        match self.queue.cancel(job) {
            Cancelled::BeforeStart => {
                let plan = self.plans.remove(&job);
                self.kept.remove(&job);
                let grants = self.queue.finished(job);
                let mut granted = self.grant(grants);
                granted.notes.push(self.changed());
                (plan, granted)
            }
            Cancelled::WhileRunning | Cancelled::Unknown => (None, Granted::default()),
        }
    }

    /// Runs a waiting job now, past the limits.
    pub(super) fn run_now(&mut self, job: JobId) -> Granted {
        let grants = self.queue.run_now(job);
        let mut granted = self.grant(grants);
        granted.notes.push(self.changed());
        granted
    }

    /// Moves a waiting job; whether it moved.
    pub(super) fn shift(&mut self, job: JobId, how: Shift) -> (bool, Granted) {
        let moved = match how {
            Shift::Up => self.queue.move_up(job),
            Shift::Down => self.queue.move_down(job),
            Shift::Front => self.queue.move_to_front(job),
            Shift::End => self.queue.move_to_end(job),
        };
        (
            moved,
            Granted {
                notes: vec![self.changed()],
                ..Granted::default()
            },
        )
    }

    /// Pauses the queue (nothing new starts) or lets it go on.
    pub(super) fn set_paused(&mut self, paused: bool) -> Granted {
        let grants = if paused {
            self.queue.pause();
            Vec::new()
        } else {
            self.queue.resume()
        };
        let mut granted = self.grant(grants);
        granted.notes.push(self.changed());
        granted
    }

    /// New limits from the settings: a higher one starts what it lets in.
    pub(super) fn set_limits(&mut self, limits: QueueLimits) -> Granted {
        if limits == self.queue.limits() {
            return Granted::default();
        }
        let grants = self.queue.set_limits(limits);
        let mut granted = self.grant(grants);
        granted.notes.push(self.changed());
        granted
    }

    /// Changes a waiting job's error policy: it has not started, so its
    /// plan is still here; whether it was.
    pub(super) fn set_error_policy(
        &mut self,
        job: JobId,
        policy: diroxide_core::job::ErrorPolicy,
    ) -> bool {
        match self.plans.get_mut(&job) {
            Some(plan) => {
                plan.facts.error_policy = policy;
                self.save();
                true
            }
            None => false,
        }
    }

    /// Holds a job that waits its turn in its place (lane Q2); `None`
    /// when it does not wait (it started, or the queue never held it), so
    /// the caller pauses its control instead.
    pub(super) fn hold(&mut self, job: JobId) -> Option<Granted> {
        let grants = self.queue.hold(job)?;
        let mut granted = Granted {
            notes: vec![QueueNote::Held(job)],
            ..Granted::default()
        };
        granted.extend(self.grant(grants));
        granted.notes.push(self.changed());
        Some(granted)
    }

    /// Lets a held job go in the place it kept; `None` when it is not
    /// held.
    pub(super) fn release(&mut self, job: JobId) -> Option<Granted> {
        let grants = self.queue.release(job)?;
        let mut granted = Granted {
            notes: vec![QueueNote::Released(job)],
            ..Granted::default()
        };
        granted.extend(self.grant(grants));
        granted.notes.push(self.changed());
        Some(granted)
    }

    /// Pause all jobs: the queue paused, and every waiting job held.
    pub(super) fn hold_all(&mut self) -> Granted {
        self.queue.hold_all();
        self.set_paused(true)
    }

    /// Resume all jobs: every held job let go, and the queue goes on.
    pub(super) fn release_all(&mut self) -> Granted {
        let grants = self.queue.release_all();
        let mut granted = self.grant(grants);
        granted.extend(self.set_paused(false));
        granted
    }

    /// The queue's order and state for the app.
    fn view(&self) -> QueueNote {
        QueueNote::View {
            waiting: self.queue.waiting(),
            held: self.queue.held(),
            paused: self.queue.is_paused(),
        }
    }

    /// Turns grants into plans to start and answers to deliver.
    fn grant(&mut self, grants: Vec<Grant>) -> Granted {
        let mut granted = Granted::default();
        for grant in grants {
            match grant {
                Grant::Start(job) => {
                    if let Some(plan) = self.plans.remove(&job) {
                        granted.notes.push(QueueNote::Started(job));
                        granted.start.push(plan);
                    }
                }
                Grant::Continue(job) => {
                    if let Some(answer) = self.answers.remove(&job) {
                        granted.answers.push(answer);
                    }
                }
            }
        }
        granted
    }
}

/// Where a waiting job moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shift {
    /// One place earlier.
    Up,
    /// One place later.
    Down,
    /// Before every other waiting job.
    Front,
    /// After every other waiting job.
    End,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diroxide_core::{JobKind, JobRequest, Place};
    use diroxide_ops::{Engine, FakeProvider};
    use diroxide_vfs::Registry;

    use super::*;

    fn remote(path: &str) -> Place {
        Place::parse(&format!("sftp://build{path}")).unwrap()
    }

    fn local(path: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", path.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(path).unwrap()
        }
    }

    fn plans(count: u64) -> Vec<Plan> {
        requests_and_plans(1..=count)
            .into_iter()
            .map(|(_, plan)| plan)
            .collect()
    }

    /// Copies from `build` to a local folder, one per id, each with the
    /// request it was planned from.
    fn requests_and_plans(ids: impl IntoIterator<Item = u64>) -> Vec<(JobRequest, Plan)> {
        let fake = FakeProvider::new();
        fake.add_folder(&local("/b"));
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        registry.register("sftp", Arc::new(fake.clone()));
        let engine = Engine::new(Arc::new(registry));
        ids.into_iter()
            .map(|n| {
                let source = remote(&format!("/srv/f{n}.bin"));
                fake.add_file(&source, b"data");
                let request =
                    JobRequest::new(JobId::new(n), JobKind::Copy, vec![source]).to(local("/b"));
                let plan = engine.plan(&request).unwrap();
                (request, plan)
            })
            .collect()
    }

    /// The saved queue as the file holds it now: each job's id and
    /// state, or `None` when there is no file.
    fn on_disk(scheduler: &Scheduler, folder: &Path) -> Option<Vec<(u64, String)>> {
        assert!(
            scheduler.flush(Duration::from_secs(10)),
            "the writer is stuck"
        );
        let text =
            std::fs::read_to_string(folder.join(super::super::saved_queue::QUEUE_FILE)).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        Some(
            value["jobs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|job| {
                    (
                        job["job"].as_u64().unwrap(),
                        job["state"].as_str().unwrap().to_owned(),
                    )
                })
                .collect(),
        )
    }

    /// Lane Q2: the saved queue follows every change, is removed when the
    /// queue is empty, keeps what a quit stopped, and comes back at the
    /// next start as interrupted jobs that Forget and Retry take away.
    #[test]
    fn the_saved_queue_follows_every_change_and_comes_back_interrupted() {
        let dir = tempfile::Builder::new()
            .prefix("scheduler-")
            .tempdir()
            .unwrap();
        let folder = dir.path();
        let mut scheduler = Scheduler::default();
        let (note, problem) = scheduler.open_file(folder, Box::new(|_| {}));
        assert!(note.is_none() && problem.is_none());
        assert_eq!(on_disk(&scheduler, folder), None);
        for (request, plan) in requests_and_plans(1..=3) {
            scheduler.planned(&request);
            scheduler.submit(plan);
        }
        let state = |n: u64, s: &str| (n, s.to_owned());
        assert_eq!(
            on_disk(&scheduler, folder),
            Some(vec![
                state(1, "running"),
                state(2, "running"),
                state(3, "waiting")
            ])
        );
        scheduler.hold(JobId::new(3));
        scheduler.set_error_policy(JobId::new(3), diroxide_core::job::ErrorPolicy::StopAtFirst);
        scheduler.finished(JobId::new(1));
        scheduler.finished(JobId::new(2));
        assert_eq!(on_disk(&scheduler, folder), Some(vec![state(3, "held")]));
        let text =
            std::fs::read_to_string(folder.join(super::super::saved_queue::QUEUE_FILE)).unwrap();
        assert!(text.contains("\"what\": \"f3.bin to b\""), "{text}");
        assert!(text.contains("stop-at-first"), "the policy is kept: {text}");
        scheduler.cancel(JobId::new(3));
        assert_eq!(on_disk(&scheduler, folder), None, "empty: removed");

        // Quit with two jobs running and one waiting: all are kept, and
        // the stop that follows writes nothing.
        for (request, plan) in requests_and_plans(4..=6) {
            scheduler.planned(&request);
            scheduler.submit(plan);
        }
        scheduler.freeze_for_quit();
        scheduler.cancel(JobId::new(6));
        scheduler.finished(JobId::new(4));
        scheduler.finished(JobId::new(5));
        assert_eq!(
            on_disk(&scheduler, folder),
            Some(vec![
                state(4, "running"),
                state(5, "running"),
                state(6, "waiting")
            ])
        );
        drop(scheduler);

        // The next start: all interrupted, by a quit, nothing started.
        let mut scheduler = Scheduler::default();
        let (note, _) = scheduler.open_file(folder, Box::new(|_| {}));
        let Some(QueueNote::Interrupted { jobs, new, by_quit }) = note else {
            panic!("{note:?}");
        };
        assert_eq!((new, by_quit), (3, true));
        let rows: Vec<String> = jobs.iter().map(InterruptedJob::row).collect();
        assert_eq!(
            rows,
            [
                "Interrupted: copy f4.bin to b",
                "Interrupted: copy f5.bin to b",
                "Interrupted: copy f6.bin to b"
            ]
        );
        assert_eq!(
            jobs.iter().map(|j| (j.key, j.started)).collect::<Vec<_>>(),
            [(1, true), (2, true), (3, false)]
        );
        assert!(scheduler.queue.is_empty(), "nothing starts on its own");
        assert_eq!(
            on_disk(&scheduler, folder),
            Some(vec![
                state(4, "interrupted"),
                state(5, "interrupted"),
                state(6, "interrupted")
            ])
        );

        // Forget takes one away; Retry another once its new job is
        // confirmed, and the file follows the new job.
        assert_eq!(scheduler.forget(1).notes, [QueueNote::Forgotten(1)]);
        assert!(scheduler.forget(1).notes.is_empty(), "already forgotten");
        let (request, plan) = requests_and_plans([10]).remove(0);
        scheduler.retried(3, JobId::new(10));
        scheduler.planned(&request);
        let granted = scheduler.submit(plan);
        assert!(granted.notes.contains(&QueueNote::Forgotten(3)));
        assert_eq!(
            on_disk(&scheduler, folder),
            Some(vec![state(5, "interrupted"), state(10, "running")])
        );
        scheduler.finished(JobId::new(10));
        scheduler.forget(2);
        assert_eq!(on_disk(&scheduler, folder), None);
    }

    #[test]
    fn two_run_on_a_host_and_the_third_waits_its_turn() {
        let mut scheduler = Scheduler::default();
        let mut started = Vec::new();
        let mut notes = Vec::new();
        for plan in plans(3) {
            let granted = scheduler.submit(plan);
            started.extend(granted.start.iter().map(|p| p.job.get()));
            notes.extend(granted.notes);
        }
        assert_eq!(started, [1, 2]);
        assert!(notes.contains(&QueueNote::Waits(JobId::new(3))));
        assert!(notes.contains(&QueueNote::View {
            waiting: vec![JobId::new(3)],
            held: Vec::new(),
            paused: false
        }));
        // A question gives the slot back: job 3 starts; the answer waits.
        let granted = scheduler.asking(JobId::new(1));
        assert_eq!(granted.start.len(), 1);
        let granted = scheduler.answer(
            Some(JobId::new(1)),
            AskId::new(9),
            Reply::Error(ErrorAction::Retry),
        );
        assert!(granted.answers.is_empty(), "held until a slot is free");
        let granted = scheduler.finished(JobId::new(2));
        assert_eq!(granted.answers.len(), 1, "delivered with the slot");
        // An answer that cancels goes at once.
        scheduler.asking(JobId::new(3));
        let granted = scheduler.answer(
            Some(JobId::new(3)),
            AskId::new(10),
            Reply::Error(ErrorAction::Cancel),
        );
        assert_eq!(granted.answers.len(), 1);
    }

    #[test]
    fn a_waiting_job_cancels_before_it_starts_and_moves_in_line() {
        let mut scheduler = Scheduler::default();
        for plan in plans(4) {
            scheduler.submit(plan);
        }
        let (moved, granted) = scheduler.shift(JobId::new(4), Shift::Front);
        assert!(moved);
        assert_eq!(
            granted.notes,
            [QueueNote::View {
                waiting: vec![JobId::new(4), JobId::new(3)],
                held: Vec::new(),
                paused: false
            }]
        );
        let (plan, _) = scheduler.cancel(JobId::new(3));
        assert_eq!(plan.map(|p| p.job), Some(JobId::new(3)));
        let granted = scheduler.run_now(JobId::new(4));
        assert_eq!(granted.start.len(), 1, "run now passes the limit");
        scheduler.set_paused(true);
        assert!(scheduler.submit(plans(5).pop().unwrap()).start.is_empty());
        assert_eq!(
            scheduler.set_paused(false).start.len(),
            0,
            "the host is full"
        );
        assert_eq!(scheduler.queue.len(), 4);
    }

    #[test]
    fn pause_holds_a_waiting_job_in_place_and_resume_lets_it_go() {
        let mut scheduler = Scheduler::default();
        for plan in plans(4) {
            scheduler.submit(plan);
        }
        let granted = scheduler.hold(JobId::new(3)).expect("it waits");
        assert_eq!(granted.notes[0], QueueNote::Held(JobId::new(3)));
        assert_eq!(
            granted.notes.last(),
            Some(&QueueNote::View {
                waiting: vec![JobId::new(3), JobId::new(4)],
                held: vec![JobId::new(3)],
                paused: false
            })
        );
        assert!(scheduler.hold(JobId::new(1)).is_none(), "it runs");
        assert!(scheduler.release(JobId::new(4)).is_none(), "not held");
        // The held job is passed over: job 4 takes the free slot.
        let granted = scheduler.finished(JobId::new(1));
        assert_eq!(
            granted.start.iter().map(|p| p.job).collect::<Vec<_>>(),
            [JobId::new(4)]
        );
        // Resume: back in line, and it starts when a slot is free.
        let granted = scheduler.release(JobId::new(3)).expect("held");
        assert_eq!(granted.notes[0], QueueNote::Released(JobId::new(3)));
        assert!(granted.start.is_empty(), "the host is full");
        // Pause all holds it again with the queue; Resume all lets go.
        scheduler.hold_all();
        assert!(scheduler.queue.is_held(JobId::new(3)));
        assert!(scheduler.finished(JobId::new(2)).start.is_empty());
        let granted = scheduler.release_all();
        assert_eq!(granted.start.len(), 1);
        assert!(!scheduler.queue.is_paused());
    }
}
