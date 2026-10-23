//! The transfer queue's scheduler (R25, part 1, 2.2): when a planned and
//! confirmed job starts. The queue is the jobs list, not a second thing:
//! this is a pure component the runtime asks, holding job ids and the
//! slots they need, never a thread. A waiting job is data; it gets its
//! thread when the queue grants it a start.
//!
//! - **Classes and limits** ([`JobClass`], [`QueueLimits`]): a local job
//!   starts at once, as alpha 1 does; a remote job takes a slot on each
//!   host it touches (two running jobs per host, matching the two ssh
//!   sessions of answer 87) and one of four remote slots in all; a
//!   component download runs one at a time; a plugin's command is not
//!   queued.
//! - **Both or neither.** A job that needs two hosts (a copy from one
//!   server to another) takes a slot on both at once or waits; the hosts
//!   are kept in sorted order, and the queue is one owner's data, so two
//!   such jobs can never wait on each other.
//! - **Order.** First in, first out within each host and within the
//!   remote total: a waiting job holds back later jobs that need the same
//!   host, so none is starved. Move up, move down, to the front, to the
//!   end. Run now starts a waiting job past the limits.
//! - **A question gives the slot back** ([`Queue::asking`]): one forgotten
//!   question cannot hold the queue. The answer waits for a slot again
//!   ([`Queue::answered`]), ahead of jobs that never started, and the
//!   runtime delivers it when the queue grants [`Grant::Continue`].
//! - **Cancel from any state** ([`Queue::cancel`]): a waiting job leaves at
//!   once; a running one is cancelled by its control and leaves when its
//!   thread ends ([`Queue::finished`]).
//!
//! Every method returns the grants it caused, in order; the runtime starts
//! a worker for each [`Grant::Start`] and delivers the held answer for each
//! [`Grant::Continue`]. The clock is the engine's [`crate::Pacer`] time,
//! passed in, so the property test runs on a fake clock.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use diroxide_core::{JobId, JobKind, JobRequest, JobState, Place};

use crate::plan::Plan;

/// A host as the queue and the connection gate know it: the scheme, the
/// host and the port. The user is left out: two accounts on one host share
/// its limits and its connection gate.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostKey {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl HostKey {
    /// A host key from its parts; the host is compared without case.
    pub fn new(scheme: &str, host: &str, port: Option<u16>) -> Self {
        Self {
            scheme: scheme.to_ascii_lowercase(),
            host: host.to_lowercase(),
            port,
        }
    }

    /// The host of a remote place, or of the archive a place is inside
    /// when that archive is on a remote place; `None` for a local place.
    pub fn of(place: &Place) -> Option<Self> {
        if let Some((container, _)) = place.archive() {
            return Self::of(container);
        }
        place
            .remote()
            .map(|remote| Self::new(remote.scheme(), remote.host(), remote.port()))
    }

    /// The host's name, for lines: "build".
    pub fn name(&self) -> &str {
        &self.host
    }

    /// The scheme: `sftp`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The port, when one was given.
    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

impl fmt::Display for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.host)
    }
}

/// The class of a job, from the places it touches and its kind: what
/// limits it waits for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JobClass {
    /// Only places on this computer: no limit, it starts at once.
    Local,
    /// A remote place on each of these hosts, sorted, each once.
    Remote(Vec<HostKey>),
    /// A download of an optional component: one at a time.
    Component,
    /// Not queued: a plugin's command, which reports its own progress.
    Unqueued,
}

impl JobClass {
    /// The class of a job of this kind over these places.
    pub fn of<'a>(kind: JobKind, places: impl IntoIterator<Item = &'a Place>) -> Self {
        match kind {
            JobKind::PluginCommand => return JobClass::Unqueued,
            JobKind::Download => return JobClass::Component,
            _ => {}
        }
        let hosts: BTreeSet<HostKey> = places.into_iter().filter_map(HostKey::of).collect();
        if hosts.is_empty() {
            JobClass::Local
        } else {
            JobClass::Remote(hosts.into_iter().collect())
        }
    }

    /// The class of a request: its sources, its renames and its
    /// destination.
    pub fn of_request(request: &JobRequest) -> Self {
        let places = request
            .sources
            .iter()
            .chain(request.renames.iter().map(|(place, _)| place))
            .chain(request.destination.iter());
        Self::of(request.kind, places)
    }

    /// The class of a plan: every step's subject and result, and the
    /// destination.
    pub fn of_plan(plan: &Plan) -> Self {
        let mut places: Vec<&Place> = Vec::with_capacity(plan.steps.len() * 2 + 1);
        for step in &plan.steps {
            places.push(step.subject());
            if let Some(result) = step.result() {
                places.push(result);
            }
        }
        places.extend(plan.facts.destination.iter());
        Self::of(plan.kind, places)
    }

    /// The hosts this class takes a slot on.
    pub fn hosts(&self) -> &[HostKey] {
        match self {
            JobClass::Remote(hosts) => hosts,
            _ => &[],
        }
    }
}

/// How many jobs of each class run at once (R25 N1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueLimits {
    /// Running jobs per host: 2, the two ssh sessions of answer 87.
    pub per_host: usize,
    /// Running remote jobs in all: 4.
    pub remote: usize,
    /// Component downloads at once: 1.
    pub components: usize,
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            per_host: 2,
            remote: 4,
            components: 1,
        }
    }
}

/// What the runtime does for a job the queue lets go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grant {
    /// Start the job: give it a worker and run its plan.
    Start(JobId),
    /// The job answered a question and has its slot again: deliver the
    /// answer it waits with, so it goes on.
    Continue(JobId),
}

/// What [`Queue::cancel`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cancelled {
    /// The job had not started: it is gone from the queue, and nothing
    /// was done. Its report says it was cancelled before it began.
    BeforeStart,
    /// The job is running, asking, or waiting to go on after an answer:
    /// cancel its control; it leaves the queue when its thread ends.
    WhileRunning,
    /// The queue does not hold this job.
    Unknown,
}

/// Where a job is in the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Planned and confirmed, never started: waits its turn.
    Waiting,
    /// Running, holding its slots.
    Running,
    /// Asking a question: its slots are given back.
    Asking,
    /// Answered, waiting for its slots again to go on.
    Returning,
}

#[derive(Clone, Debug)]
struct Item {
    job: JobId,
    kind: JobKind,
    class: JobClass,
    slot: Slot,
    /// Started by Run now, past the limits.
    forced: bool,
    submitted: Duration,
}

/// The scheduler. See the module documentation.
#[derive(Clone, Debug, Default)]
pub struct Queue {
    limits: QueueLimits,
    items: Vec<Item>,
    paused: bool,
}

impl Queue {
    /// An empty queue with these limits.
    pub fn new(limits: QueueLimits) -> Self {
        Self {
            limits,
            items: Vec::new(),
            paused: false,
        }
    }

    /// The limits.
    pub fn limits(&self) -> QueueLimits {
        self.limits
    }

    /// Changes the limits (the queue settings); a higher limit starts the
    /// jobs it now lets in.
    pub fn set_limits(&mut self, limits: QueueLimits) -> Vec<Grant> {
        self.limits = limits;
        self.schedule()
    }

    /// Adds a planned, confirmed job. The grants say whether it, or
    /// nothing, starts now; when it does not, it waits its turn.
    pub fn submit(
        &mut self,
        job: JobId,
        kind: JobKind,
        class: JobClass,
        now: Duration,
    ) -> Vec<Grant> {
        if self.index(job).is_some() {
            return Vec::new();
        }
        self.items.push(Item {
            job,
            kind,
            class,
            slot: Slot::Waiting,
            forced: false,
            submitted: now,
        });
        self.schedule()
    }

    /// A job's thread ended (done, failed or cancelled): it leaves the
    /// queue and its slots go to the next in line.
    pub fn finished(&mut self, job: JobId) -> Vec<Grant> {
        if let Some(at) = self.index(job) {
            self.items.remove(at);
        }
        self.schedule()
    }

    /// Cancels a job from any state. A waiting job leaves at once; the
    /// runtime cancels a running one's control, and the job leaves when
    /// its thread ends.
    pub fn cancel(&mut self, job: JobId) -> Cancelled {
        match self.index(job) {
            None => Cancelled::Unknown,
            Some(at) if self.items[at].slot == Slot::Waiting => {
                self.items.remove(at);
                Cancelled::BeforeStart
            }
            Some(_) => Cancelled::WhileRunning,
        }
    }

    /// A running job asks a question: its slots go to the next in line
    /// until it is answered.
    pub fn asking(&mut self, job: JobId) -> Vec<Grant> {
        match self.index(job) {
            Some(at) if self.items[at].slot == Slot::Running => {
                self.items[at].slot = Slot::Asking;
                self.items[at].forced = false;
                self.schedule()
            }
            _ => Vec::new(),
        }
    }

    /// A job's question was answered: it waits for its slots again, ahead
    /// of every job that never started. The grants include
    /// [`Grant::Continue`] for it when a slot is free now.
    pub fn answered(&mut self, job: JobId) -> Vec<Grant> {
        match self.index(job) {
            Some(at) if self.items[at].slot == Slot::Asking => {
                self.items[at].slot = Slot::Returning;
                self.schedule()
            }
            _ => Vec::new(),
        }
    }

    /// Starts a waiting job now, past the limits (WinSCP's "execute
    /// now"); its slots still count against the others.
    pub fn run_now(&mut self, job: JobId) -> Vec<Grant> {
        match self.index(job) {
            Some(at) if matches!(self.items[at].slot, Slot::Waiting | Slot::Returning) => {
                let item = &mut self.items[at];
                let grant = if item.slot == Slot::Waiting {
                    Grant::Start(job)
                } else {
                    Grant::Continue(job)
                };
                item.slot = Slot::Running;
                item.forced = true;
                vec![grant]
            }
            _ => Vec::new(),
        }
    }

    /// Moves a waiting job one place earlier; `false` when it cannot move.
    pub fn move_up(&mut self, job: JobId) -> bool {
        self.shift(job, |queue, at| queue.waiting_before(at))
    }

    /// Moves a waiting job one place later; `false` when it cannot move.
    pub fn move_down(&mut self, job: JobId) -> bool {
        self.shift(job, |queue, at| queue.waiting_after(at))
    }

    /// Moves a waiting job before every other waiting job.
    pub fn move_to_front(&mut self, job: JobId) -> bool {
        self.shift(job, |queue, at| {
            queue.items[..at]
                .iter()
                .position(|item| item.slot == Slot::Waiting)
        })
    }

    /// Moves a waiting job after every other waiting job.
    pub fn move_to_end(&mut self, job: JobId) -> bool {
        self.shift(job, |queue, at| {
            queue
                .items
                .iter()
                .rposition(|item| item.slot == Slot::Waiting)
                .filter(|&last| last > at)
        })
    }

    /// Pauses the queue: nothing new starts until [`Queue::resume`]. Jobs
    /// already running go on, and an answered job still gets its slot.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Lets the queue start jobs again.
    pub fn resume(&mut self) -> Vec<Grant> {
        self.paused = false;
        self.schedule()
    }

    /// Whether the queue is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// A waiting job's place in line, from 1, and how many wait: "Waiting
    /// 2 of 5".
    pub fn position(&self, job: JobId) -> Option<(usize, usize)> {
        let waiting: Vec<JobId> = self.waiting();
        let at = waiting.iter().position(|&id| id == job)?;
        Some((at + 1, waiting.len()))
    }

    /// The state the jobs list shows for a job: waiting its turn (also
    /// after an answer, until its slot is free), running, or waiting for
    /// an answer; `None` when the queue does not hold it.
    pub fn state(&self, job: JobId) -> Option<JobState> {
        self.index(job).map(|at| match self.items[at].slot {
            Slot::Waiting | Slot::Returning => JobState::Queued,
            Slot::Running => JobState::Running,
            Slot::Asking => JobState::WaitingForAnswer,
        })
    }

    /// The jobs that wait their turn, in order: never started first, as
    /// listed; answered jobs that wait to go on are not counted here.
    pub fn waiting(&self) -> Vec<JobId> {
        self.items
            .iter()
            .filter(|item| item.slot == Slot::Waiting)
            .map(|item| item.job)
            .collect()
    }

    /// The jobs that hold their slots now, in order.
    pub fn running(&self) -> Vec<JobId> {
        self.items
            .iter()
            .filter(|item| item.slot == Slot::Running)
            .map(|item| item.job)
            .collect()
    }

    /// Every job the queue holds, in order, with its kind and state.
    pub fn jobs(&self) -> Vec<(JobId, JobKind, JobState)> {
        self.items
            .iter()
            .filter_map(|item| Some((item.job, item.kind, self.state(item.job)?)))
            .collect()
    }

    /// How long a job has been in the queue, by the clock its times came
    /// from.
    pub fn waited(&self, job: JobId, now: Duration) -> Option<Duration> {
        self.index(job)
            .map(|at| now.saturating_sub(self.items[at].submitted))
    }

    /// How many jobs the queue holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the queue holds no job.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many slots are taken now on a host, by running jobs, forced or
    /// not.
    pub fn running_on(&self, host: &HostKey) -> usize {
        self.items
            .iter()
            .filter(|item| item.slot == Slot::Running && item.class.hosts().contains(host))
            .count()
    }

    // ----- inside -----

    fn index(&self, job: JobId) -> Option<usize> {
        self.items.iter().position(|item| item.job == job)
    }

    fn waiting_before(&self, at: usize) -> Option<usize> {
        self.items[..at]
            .iter()
            .rposition(|item| item.slot == Slot::Waiting)
    }

    fn waiting_after(&self, at: usize) -> Option<usize> {
        self.items[at + 1..]
            .iter()
            .position(|item| item.slot == Slot::Waiting)
            .map(|offset| at + 1 + offset)
    }

    /// Moves a waiting job to the index `to` finds, keeping every other
    /// job's order, and starts what the new order lets start.
    fn shift(&mut self, job: JobId, to: impl Fn(&Self, usize) -> Option<usize>) -> bool {
        let Some(at) = self.index(job) else {
            return false;
        };
        if self.items[at].slot != Slot::Waiting {
            return false;
        }
        let Some(target) = to(self, at) else {
            return false;
        };
        let item = self.items.remove(at);
        self.items.insert(target, item);
        true
    }

    /// What is in use now: per host, remote in all, components.
    fn usage(&self) -> (BTreeMap<HostKey, usize>, usize, usize) {
        let mut hosts: BTreeMap<HostKey, usize> = BTreeMap::new();
        let mut remote = 0;
        let mut components = 0;
        for item in self.items.iter().filter(|item| item.slot == Slot::Running) {
            match &item.class {
                JobClass::Remote(list) => {
                    remote += 1;
                    for host in list {
                        *hosts.entry(host.clone()).or_default() += 1;
                    }
                }
                JobClass::Component => components += 1,
                JobClass::Local | JobClass::Unqueued => {}
            }
        }
        (hosts, remote, components)
    }

    /// Starts every job the limits let start, answered jobs first, then
    /// waiting ones, each group in order; a job that cannot start holds
    /// back later jobs that need the same host or the same total, so the
    /// order is first in, first out and nothing is starved.
    fn schedule(&mut self) -> Vec<Grant> {
        let mut grants = Vec::new();
        let (mut hosts, mut remote, mut components) = self.usage();
        for returning in [true, false] {
            if !returning && self.paused {
                break;
            }
            let mut held_hosts: BTreeSet<HostKey> = BTreeSet::new();
            let mut remote_held = false;
            let mut components_held = false;
            for at in 0..self.items.len() {
                let wanted = if returning {
                    Slot::Returning
                } else {
                    Slot::Waiting
                };
                if self.items[at].slot != wanted {
                    continue;
                }
                let limits = self.limits;
                let fits = match &self.items[at].class {
                    JobClass::Local | JobClass::Unqueued => true,
                    JobClass::Component => {
                        let fits = !components_held && components < limits.components;
                        components_held |= !fits;
                        fits
                    }
                    JobClass::Remote(list) => {
                        let fits = !remote_held
                            && remote < limits.remote
                            && list.iter().all(|host| {
                                !held_hosts.contains(host)
                                    && hosts.get(host).copied().unwrap_or(0) < limits.per_host
                            });
                        if !fits {
                            // Hold back the later jobs that would take what
                            // this one waits for: its hosts, and the total
                            // when the total is what stops it.
                            held_hosts.extend(list.iter().cloned());
                            remote_held |= remote >= limits.remote;
                        }
                        fits
                    }
                };
                if !fits {
                    continue;
                }
                let item = &mut self.items[at];
                match &item.class {
                    JobClass::Remote(list) => {
                        remote += 1;
                        for host in list {
                            *hosts.entry(host.clone()).or_default() += 1;
                        }
                    }
                    JobClass::Component => components += 1,
                    JobClass::Local | JobClass::Unqueued => {}
                }
                item.slot = Slot::Running;
                grants.push(if returning {
                    Grant::Continue(item.job)
                } else {
                    Grant::Start(item.job)
                });
            }
        }
        grants
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;

    fn remote(host: &str) -> JobClass {
        JobClass::Remote(vec![HostKey::new("sftp", host, None)])
    }

    fn id(n: u64) -> JobId {
        JobId::new(n)
    }

    fn started(grants: &[Grant]) -> Vec<u64> {
        grants
            .iter()
            .map(|grant| match grant {
                Grant::Start(job) | Grant::Continue(job) => job.get(),
            })
            .collect()
    }

    #[test]
    fn classes_come_from_the_places_and_the_kind() {
        let local = Place::parse_with("/home/a/x", PathStyle::Unix).unwrap();
        let build = Place::parse("sftp://ada@Build:2222/srv/x").unwrap();
        let store = Place::parse("sftp://store/data").unwrap();
        assert_eq!(JobClass::of(JobKind::Copy, [&local]), JobClass::Local);
        let both = JobClass::of(JobKind::Copy, [&store, &local, &build]);
        assert_eq!(
            both.hosts(),
            &[
                HostKey::new("sftp", "build", Some(2222)),
                HostKey::new("sftp", "store", None)
            ]
        );
        assert_eq!(both.hosts()[0].to_string(), "build");
        assert_eq!(
            JobClass::of(JobKind::Download, [&build]),
            JobClass::Component
        );
        assert_eq!(
            JobClass::of(JobKind::PluginCommand, [&build]),
            JobClass::Unqueued
        );
        let inside = build.join("a.zip").unwrap().into_archive();
        assert_eq!(HostKey::of(&inside), HostKey::of(&build));
        assert_eq!(HostKey::of(&local), None);
        let request = JobRequest::new(id(1), JobKind::Copy, vec![local]).to(build);
        assert_eq!(
            JobClass::of_request(&request),
            remote("build").with_port(2222)
        );
    }

    impl JobClass {
        fn with_port(self, port: u16) -> Self {
            match self {
                JobClass::Remote(hosts) => JobClass::Remote(
                    hosts
                        .into_iter()
                        .map(|h| HostKey::new(&h.scheme, &h.host, Some(port)))
                        .collect(),
                ),
                other => other,
            }
        }
    }

    #[test]
    fn local_jobs_start_at_once_and_remote_ones_wait_their_turn() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        assert_eq!(
            started(&queue.submit(id(1), JobKind::Copy, JobClass::Local, zero)),
            [1]
        );
        assert_eq!(
            started(&queue.submit(id(2), JobKind::Copy, remote("build"), zero)),
            [2]
        );
        assert_eq!(
            started(&queue.submit(id(3), JobKind::Copy, remote("build"), zero)),
            [3]
        );
        assert!(
            queue
                .submit(id(4), JobKind::Copy, remote("build"), zero)
                .is_empty()
        );
        assert_eq!(queue.state(id(4)), Some(JobState::Queued));
        assert_eq!(queue.position(id(4)), Some((1, 1)));
        // Another host still has room.
        assert_eq!(
            started(&queue.submit(id(5), JobKind::Move, remote("store"), zero)),
            [5]
        );
        assert_eq!(started(&queue.finished(id(2))), [4]);
        assert_eq!(queue.running_on(&HostKey::new("sftp", "build", None)), 2);
    }

    #[test]
    fn four_remote_jobs_in_all_and_one_component_at_a_time() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        for (n, host) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            queue.submit(id(n as u64 + 1), JobKind::Copy, remote(host), zero);
        }
        assert_eq!(queue.running().len(), 4);
        assert_eq!(queue.waiting(), [id(5)]);
        assert_eq!(
            started(&queue.submit(id(6), JobKind::Download, JobClass::Component, zero)),
            [6]
        );
        assert!(
            queue
                .submit(id(7), JobKind::Download, JobClass::Component, zero)
                .is_empty()
        );
        assert_eq!(started(&queue.finished(id(6))), [7]);
        assert_eq!(started(&queue.finished(id(1))), [5]);
    }

    #[test]
    fn a_job_on_two_hosts_takes_both_or_neither() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        let a = HostKey::new("sftp", "a", None);
        let b = HostKey::new("sftp", "b", None);
        queue.submit(id(1), JobKind::Copy, remote("a"), zero);
        queue.submit(id(2), JobKind::Copy, remote("a"), zero);
        let both = JobClass::Remote(vec![a.clone(), b.clone()]);
        assert!(queue.submit(id(3), JobKind::Copy, both, zero).is_empty());
        assert_eq!(
            queue.running_on(&b),
            0,
            "nothing taken on b while a is full"
        );
        // A later job on b alone waits behind the job that needs b too.
        assert!(
            queue
                .submit(id(4), JobKind::Copy, remote("b"), zero)
                .is_empty()
        );
        assert_eq!(started(&queue.finished(id(1))), [3, 4]);
        assert_eq!(queue.running_on(&a), 2);
        assert_eq!(queue.running_on(&b), 2);
    }

    #[test]
    fn a_question_gives_the_slot_back_and_the_answer_waits_for_one() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        queue.submit(id(1), JobKind::Copy, remote("a"), zero);
        queue.submit(id(2), JobKind::Copy, remote("a"), zero);
        queue.submit(id(3), JobKind::Copy, remote("a"), zero);
        assert_eq!(started(&queue.asking(id(1))), [3]);
        assert_eq!(queue.state(id(1)), Some(JobState::WaitingForAnswer));
        assert!(queue.answered(id(1)).is_empty(), "both slots are taken");
        assert_eq!(queue.state(id(1)), Some(JobState::Queued));
        assert!(
            queue
                .submit(id(4), JobKind::Copy, remote("a"), zero)
                .is_empty()
        );
        // The answered job goes on before the one that never started.
        assert_eq!(queue.finished(id(2)), [Grant::Continue(id(1))]);
        assert_eq!(queue.finished(id(3)), [Grant::Start(id(4))]);
    }

    #[test]
    fn cancel_works_from_every_state() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        queue.submit(id(1), JobKind::Copy, remote("a"), zero);
        queue.submit(id(2), JobKind::Copy, remote("a"), zero);
        queue.submit(id(3), JobKind::Copy, remote("a"), zero);
        assert_eq!(queue.cancel(id(3)), Cancelled::BeforeStart);
        assert_eq!(queue.state(id(3)), None);
        assert_eq!(queue.cancel(id(1)), Cancelled::WhileRunning);
        queue.asking(id(2));
        assert_eq!(queue.cancel(id(2)), Cancelled::WhileRunning);
        assert_eq!(queue.cancel(id(9)), Cancelled::Unknown);
        queue.finished(id(1));
        queue.finished(id(2));
        assert!(queue.is_empty());
    }

    #[test]
    fn run_now_passes_the_limit_and_order_can_change() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        for n in 1..=5 {
            queue.submit(id(n), JobKind::Copy, remote("a"), zero);
        }
        assert_eq!(queue.waiting(), [id(3), id(4), id(5)]);
        assert!(queue.move_to_front(id(5)));
        assert_eq!(queue.waiting(), [id(5), id(3), id(4)]);
        assert!(queue.move_down(id(5)));
        assert_eq!(queue.waiting(), [id(3), id(5), id(4)]);
        assert!(queue.move_up(id(4)));
        assert_eq!(queue.waiting(), [id(3), id(4), id(5)]);
        assert!(queue.move_to_end(id(3)));
        assert_eq!(queue.waiting(), [id(4), id(5), id(3)]);
        assert!(!queue.move_to_end(id(3)), "already last");
        assert!(!queue.move_up(id(1)), "a running job does not move");
        assert_eq!(queue.position(id(5)), Some((2, 3)));
        assert_eq!(queue.run_now(id(3)), [Grant::Start(id(3))]);
        assert_eq!(queue.running_on(&HostKey::new("sftp", "a", None)), 3);
        // The forced job's slot counts: two finishes start nothing new
        // until the host is under its limit again.
        assert!(queue.finished(id(1)).is_empty());
        assert_eq!(started(&queue.finished(id(2))), [4]);
    }

    #[test]
    fn a_paused_queue_starts_nothing_new() {
        let mut queue = Queue::new(QueueLimits::default());
        let zero = Duration::ZERO;
        queue.pause();
        assert!(
            queue
                .submit(id(1), JobKind::Copy, remote("a"), zero)
                .is_empty()
        );
        assert!(queue.is_paused());
        assert_eq!(started(&queue.resume()), [1]);
        let raised = QueueLimits {
            per_host: 3,
            ..QueueLimits::default()
        };
        queue.submit(id(2), JobKind::Copy, remote("a"), zero);
        assert!(
            queue
                .submit(id(3), JobKind::Copy, remote("a"), zero)
                .is_empty()
        );
        assert_eq!(started(&queue.set_limits(raised)), [3]);
        assert_eq!(
            queue.waited(id(3), Duration::from_secs(9)),
            Some(Duration::from_secs(9))
        );
    }

    /// The property test of R25 commit 66, on a fake clock: random
    /// submissions, cancels, questions, answers, run-now, moves, pauses and
    /// finishes. After every step the limits hold (a job started by Run
    /// now counts, and may pass them), every job is where the grants put
    /// it, and at the end the queue drains: no job lost or starved.
    mod props {
        use std::collections::{HashMap, HashSet};

        use proptest::prelude::*;

        use super::*;

        #[derive(Clone, Debug)]
        enum Op {
            Submit(u8),
            Finish(u8),
            Cancel(u8),
            Ask(u8),
            Answer(u8),
            RunNow(u8),
            Move(u8, u8),
            Pause,
            Resume,
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                4 => any::<u8>().prop_map(Op::Submit),
                3 => any::<u8>().prop_map(Op::Finish),
                1 => any::<u8>().prop_map(Op::Cancel),
                1 => any::<u8>().prop_map(Op::Ask),
                1 => any::<u8>().prop_map(Op::Answer),
                1 => any::<u8>().prop_map(Op::RunNow),
                1 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Op::Move(a, b)),
                1 => Just(Op::Pause),
                1 => Just(Op::Resume),
            ]
        }

        fn host(name: &str) -> HostKey {
            HostKey::new("sftp", name, None)
        }

        fn class(choice: u8) -> JobClass {
            match choice % 7 {
                0 => JobClass::Local,
                1 => JobClass::Component,
                2 => remote("a"),
                3 => remote("b"),
                4 => remote("c"),
                5 => JobClass::Remote(vec![host("a"), host("b")]),
                _ => JobClass::Remote(vec![host("b"), host("c")]),
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Seen {
            Waiting,
            Running,
            Asking,
            Returning,
        }

        struct Model {
            seen: HashMap<JobId, (Seen, JobClass)>,
            forced: HashSet<JobId>,
        }

        impl Model {
            fn pick(&self, n: u8, state: Seen) -> Option<JobId> {
                let mut ids: Vec<JobId> = self
                    .seen
                    .iter()
                    .filter(|(_, (seen, _))| *seen == state)
                    .map(|(id, _)| *id)
                    .collect();
                ids.sort();
                (!ids.is_empty()).then(|| ids[usize::from(n) % ids.len()])
            }

            fn apply(&mut self, grants: &[Grant]) {
                for grant in grants {
                    match grant {
                        Grant::Start(job) => {
                            let entry = self.seen.get_mut(job).expect("a known job");
                            assert_eq!(entry.0, Seen::Waiting, "{job:?} started twice");
                            entry.0 = Seen::Running;
                        }
                        Grant::Continue(job) => {
                            let entry = self.seen.get_mut(job).expect("a known job");
                            assert_eq!(entry.0, Seen::Returning, "{job:?} went on unasked");
                            entry.0 = Seen::Running;
                        }
                    }
                }
            }

            fn check(&self, queue: &Queue) {
                let limits = queue.limits();
                let mut per_host: HashMap<HostKey, (usize, usize)> = HashMap::new();
                let (mut remote, mut remote_forced) = (0, 0);
                let (mut components, mut components_forced) = (0, 0);
                for (job, (seen, class)) in &self.seen {
                    let expected = match seen {
                        Seen::Waiting | Seen::Returning => JobState::Queued,
                        Seen::Running => JobState::Running,
                        Seen::Asking => JobState::WaitingForAnswer,
                    };
                    assert_eq!(queue.state(*job), Some(expected), "{job:?} lost");
                    if *seen != Seen::Running {
                        continue;
                    }
                    let forced = usize::from(self.forced.contains(job));
                    match class {
                        JobClass::Remote(hosts) => {
                            remote += 1;
                            remote_forced += forced;
                            for host in hosts {
                                let entry = per_host.entry(host.clone()).or_default();
                                entry.0 += 1;
                                entry.1 += forced;
                            }
                        }
                        JobClass::Component => {
                            components += 1;
                            components_forced += forced;
                        }
                        JobClass::Local | JobClass::Unqueued => {}
                    }
                }
                assert_eq!(queue.len(), self.seen.len(), "the queue holds a stray job");
                for (host, (running, forced)) in per_host {
                    assert!(running <= limits.per_host + forced, "{host}: {running}");
                    assert_eq!(queue.running_on(&host), running);
                }
                assert!(remote <= limits.remote + remote_forced, "remote {remote}");
                assert!(
                    components <= limits.components + components_forced,
                    "components {components}"
                );
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn limits_hold_no_job_is_lost_and_the_queue_drains(
                ops in proptest::collection::vec(op(), 1..160)
            ) {
                let mut queue = Queue::new(QueueLimits::default());
                let mut model = Model { seen: HashMap::new(), forced: HashSet::new() };
                let mut clock = Duration::ZERO;
                let mut next = 1;
                for op in ops {
                    clock += Duration::from_secs(1);
                    match op {
                        Op::Submit(choice) => {
                            let job = id(next);
                            next += 1;
                            let class = class(choice);
                            model.seen.insert(job, (Seen::Waiting, class.clone()));
                            let grants = queue.submit(job, JobKind::Copy, class, clock);
                            model.apply(&grants);
                        }
                        Op::Finish(n) => {
                            if let Some(job) = model.pick(n, Seen::Running) {
                                model.seen.remove(&job);
                                model.forced.remove(&job);
                                let grants = queue.finished(job);
                                model.apply(&grants);
                            }
                        }
                        Op::Cancel(n) => {
                            let any: Vec<JobId> = model.seen.keys().copied().collect();
                            if let Some(&job) = any.iter().min_by_key(|id| (id.get() + u64::from(n)) % 7) {
                                let waiting = model.seen[&job].0 == Seen::Waiting;
                                let answer = queue.cancel(job);
                                if waiting {
                                    prop_assert_eq!(answer, Cancelled::BeforeStart);
                                    model.seen.remove(&job);
                                } else {
                                    prop_assert_eq!(answer, Cancelled::WhileRunning);
                                }
                            }
                        }
                        Op::Ask(n) => {
                            if let Some(job) = model.pick(n, Seen::Running) {
                                model.seen.get_mut(&job).unwrap().0 = Seen::Asking;
                                model.forced.remove(&job);
                                let grants = queue.asking(job);
                                model.apply(&grants);
                            }
                        }
                        Op::Answer(n) => {
                            if let Some(job) = model.pick(n, Seen::Asking) {
                                model.seen.get_mut(&job).unwrap().0 = Seen::Returning;
                                let grants = queue.answered(job);
                                model.apply(&grants);
                            }
                        }
                        Op::RunNow(n) => {
                            if let Some(job) = model.pick(n, Seen::Waiting) {
                                let grants = queue.run_now(job);
                                prop_assert_eq!(&grants, &vec![Grant::Start(job)]);
                                model.apply(&grants);
                                model.forced.insert(job);
                            }
                        }
                        Op::Move(n, how) => {
                            if let Some(job) = model.pick(n, Seen::Waiting) {
                                let before = queue.waiting();
                                match how % 4 {
                                    0 => queue.move_up(job),
                                    1 => queue.move_down(job),
                                    2 => queue.move_to_front(job),
                                    _ => queue.move_to_end(job),
                                };
                                let mut after = queue.waiting();
                                let mut sorted = before.clone();
                                sorted.sort();
                                after.sort();
                                prop_assert_eq!(sorted, after, "a move lost a job");
                            }
                        }
                        Op::Pause => queue.pause(),
                        Op::Resume => {
                            let grants = queue.resume();
                            model.apply(&grants);
                        }
                    }
                    model.check(&queue);
                }
                // Drain: answer every question, finish every running job,
                // until the queue is empty. Nothing may be left waiting.
                let grants = queue.resume();
                model.apply(&grants);
                for _ in 0..1000 {
                    if queue.is_empty() {
                        break;
                    }
                    let asking: Vec<JobId> = model.seen.iter()
                        .filter(|(_, (seen, _))| *seen == Seen::Asking)
                        .map(|(id, _)| *id).collect();
                    for job in asking {
                        model.seen.get_mut(&job).unwrap().0 = Seen::Returning;
                        let grants = queue.answered(job);
                        model.apply(&grants);
                    }
                    let running: Vec<JobId> = model.seen.iter()
                        .filter(|(_, (seen, _))| *seen == Seen::Running)
                        .map(|(id, _)| *id).collect();
                    prop_assert!(!running.is_empty() || queue.is_empty(), "the queue is stuck");
                    for job in running {
                        model.seen.remove(&job);
                        model.forced.remove(&job);
                        let grants = queue.finished(job);
                        model.apply(&grants);
                        model.check(&queue);
                    }
                }
                prop_assert!(queue.is_empty(), "jobs were starved: {:?}", queue.jobs());
            }
        }
    }
}
