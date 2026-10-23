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

use std::collections::HashMap;
use std::time::{Duration, Instant};

use diroxide_core::{ConflictAction, ErrorAction, JobId, JobState};
use diroxide_ops::{Cancelled, Grant, Plan, Queue, QueueLimits};

use crate::jobs::QueueNote;
use crate::request::{AskId, Reply};

/// The queue, the plans and answers it holds, and its clock.
#[derive(Debug)]
pub(super) struct Scheduler {
    queue: Queue,
    /// Confirmed plans that wait for their start.
    plans: HashMap<JobId, Plan>,
    /// Answers that wait for their job's slot.
    answers: HashMap<JobId, (AskId, Reply)>,
    /// The queue's clock starts with the runtime.
    epoch: Instant,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            queue: Queue::new(QueueLimits::default()),
            plans: HashMap::new(),
            answers: HashMap::new(),
            epoch: Instant::now(),
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

    /// A confirmed plan: it starts now, or waits its turn.
    pub(super) fn submit(&mut self, plan: Plan) -> Granted {
        let job = plan.job;
        let kind = plan.kind;
        let class = diroxide_ops::JobClass::of_plan(&plan);
        self.plans.insert(job, plan);
        let now = self.now();
        let grants = self.queue.submit(job, kind, class, now);
        let starts = grants.contains(&Grant::Start(job));
        let mut granted = self.grant(grants);
        if !starts {
            granted.notes.insert(0, QueueNote::Waits(job));
        }
        granted.notes.push(self.view());
        granted
    }

    /// A job's thread ended: its slots go to the next in line.
    pub(super) fn finished(&mut self, job: JobId) -> Granted {
        self.answers.remove(&job);
        let was_held = self.holds(job);
        let grants = self.queue.finished(job);
        let mut granted = self.grant(grants);
        if was_held {
            granted.notes.push(self.view());
        }
        granted
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
                let grants = self.queue.finished(job);
                let mut granted = self.grant(grants);
                granted.notes.push(self.view());
                (plan, granted)
            }
            Cancelled::WhileRunning | Cancelled::Unknown => (None, Granted::default()),
        }
    }

    /// Runs a waiting job now, past the limits.
    pub(super) fn run_now(&mut self, job: JobId) -> Granted {
        let grants = self.queue.run_now(job);
        let mut granted = self.grant(grants);
        granted.notes.push(self.view());
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
                notes: vec![self.view()],
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
        granted.notes.push(self.view());
        granted
    }

    /// New limits from the settings: a higher one starts what it lets in.
    pub(super) fn set_limits(&mut self, limits: QueueLimits) -> Granted {
        if limits == self.queue.limits() {
            return Granted::default();
        }
        let grants = self.queue.set_limits(limits);
        let mut granted = self.grant(grants);
        granted.notes.push(self.view());
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
                true
            }
            None => false,
        }
    }

    /// The queue's order and state for the app.
    fn view(&self) -> QueueNote {
        QueueNote::View {
            waiting: self.queue.waiting(),
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
        let fake = FakeProvider::new();
        fake.add_folder(&local("/b"));
        let mut registry = Registry::new();
        registry.set_local(Arc::new(fake.clone()));
        registry.register("sftp", Arc::new(fake.clone()));
        let engine = Engine::new(Arc::new(registry));
        (1..=count)
            .map(|n| {
                let source = remote(&format!("/srv/f{n}.bin"));
                fake.add_file(&source, b"data");
                let request =
                    JobRequest::new(JobId::new(n), JobKind::Copy, vec![source]).to(local("/b"));
                engine.plan(&request).unwrap()
            })
            .collect()
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
}
