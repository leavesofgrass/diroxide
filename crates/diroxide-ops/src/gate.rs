//! The connection gate (R25, 2.4): a lost host is one fact, not one
//! dialog per queued job. Every job that touches a host shares that
//! host's gate.
//!
//! 1. A job's call fails with a lost network on a remote place. The job
//!    asks the gate: a host the gate does not yet hold lost is probed at
//!    once (a stat of its root, through its provider), so a passing
//!    failure costs nothing; a probe that fails marks the host lost.
//! 2. Every job on a lost host waits as
//!    [`diroxide_core::JobState::WaitingForConnection`] and says so once
//!    ("Copying waits for build"). Its tries run on the shared gate, not
//!    one timer each: one waiting job, the prober, probes at the backoff
//!    times (5, 15 and 45 seconds by default, [`EngineOptions::retry_waits`]).
//! 3. The first success opens the gate: every job goes on, resuming its
//!    file where it can, and [`GateEvent::Back`] says "build is back. Copy
//!    goes on." once.
//! 4. When the budget ends, one question per host goes through the
//!    prober's sink ([`JobSink::connection`]): "build is not answering.",
//!    "3 jobs wait for it.", with Keep waiting first, then Try again now
//!    and Cancel these jobs. Keep waiting starts a new budget and says
//!    "Waiting for build."; Cancel names every job it stops.
//! 5. A provider that says its loss is final ([`Gates::lost_for_good`])
//!    stops the waiting jobs with its reason.
//!
//! A job cancelled while it waits leaves at once; if it was the prober,
//! another waiting job takes the probes over. Nothing here touches the
//! network by itself: the probe is the provider's own stat, and the tests
//! drive it with the fake provider's offline and drop switches and a fake
//! clock.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use diroxide_core::format::count;
use diroxide_core::{JobId, JobKind, Place};
use diroxide_vfs::{CancelToken, ErrorClass, OpContext, Registry};

use crate::engine::JobSink;
use crate::options::EngineOptions;
use crate::pace::{self, Pacer};
use crate::queue::HostKey;

/// A job that waits for a host, as the question and the events name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiter {
    /// The job.
    pub job: JobId,
    /// What kind of job, for its verb.
    pub kind: JobKind,
    /// The item it was working on, when it had one: "report.iso".
    pub item: Option<String>,
}

impl Waiter {
    /// The line that names the job when Cancel these jobs stops it, the
    /// verb first: "Copy cancelled: report.iso on build".
    pub fn cancelled_line(&self, host: &str) -> String {
        let verb = crate::engine::summary_word(self.kind);
        match &self.item {
            Some(item) => format!("{verb} cancelled: {item} on {host}"),
            None => format!("{verb} cancelled on {host}"),
        }
    }
}

/// The one question for a host whose budget ran out (R25, 2.4, step 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionQuestion {
    /// The host's name: "build".
    pub host: String,
    /// The jobs that wait for it, in the order they came.
    pub jobs: Vec<Waiter>,
}

/// The answer to a [`ConnectionQuestion`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConnectionAnswer {
    /// Keep waiting: a new budget of tries, said "Waiting for build.".
    /// The first choice, and where the focus starts.
    KeepWaiting,
    /// Try the connection again now, then keep waiting if it is still
    /// lost.
    TryNow,
    /// Cancel every job that waits for this host, each named.
    CancelJobs,
    /// No one can answer here (a frontend without the question): the jobs
    /// stop waiting together and each asks about its item, as a lost
    /// network did before the gate. The default of [`JobSink::connection`].
    AskEachJob,
}

impl ConnectionAnswer {
    /// The choices offered, in order, each with its label: Keep waiting
    /// first.
    pub const CHOICES: [(ConnectionAnswer, &'static str); 3] = [
        (ConnectionAnswer::KeepWaiting, "Keep waiting"),
        (ConnectionAnswer::TryNow, "Try again now"),
        (ConnectionAnswer::CancelJobs, "Cancel these jobs"),
    ];
}

impl ConnectionQuestion {
    /// The question's lines, the fact first: "build is not answering.",
    /// "3 jobs wait for it." (R25's 23 and 19 cells).
    pub fn lines(&self) -> Vec<String> {
        let jobs = self.jobs.len() as u64;
        let wait = if jobs == 1 { "waits" } else { "wait" };
        vec![
            format!("{} is not answering.", self.host),
            format!("{} {wait} for it.", count(jobs, "job", "jobs")),
        ]
    }
}

/// What a gate tells the app, once per host and moment: the app says each
/// line (lane A2 wires it through [`Gates::on_event`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateEvent {
    /// A probe failed: the host is lost, and its jobs start to wait. Each
    /// job says that it waits through its own state; nothing more is said
    /// here.
    Lost {
        /// The host's name.
        host: String,
    },
    /// The first success: every waiting job goes on.
    Back {
        /// The host's name.
        host: String,
        /// The jobs that go on.
        jobs: Vec<Waiter>,
    },
    /// The owner chose Keep waiting: a new budget starts.
    KeepWaiting {
        /// The host's name.
        host: String,
    },
    /// The owner chose Cancel these jobs: each is stopped and named.
    Cancelled {
        /// The host's name.
        host: String,
        /// The jobs stopped.
        jobs: Vec<Waiter>,
    },
    /// The provider said the loss is final: the waiting jobs stop.
    LostForGood {
        /// The host's name.
        host: String,
        /// Why, in words, from the provider.
        reason: String,
        /// The jobs stopped.
        jobs: Vec<Waiter>,
    },
}

impl GateEvent {
    /// The lines to say, meaning first, each inside 40 cells for a short
    /// host name: "build is back. Copy goes on.", "build is back. 3 jobs go
    /// on.", "Waiting for build.", one "Copy cancelled: report.iso on
    /// build" per job; nothing for [`GateEvent::Lost`], whose jobs say it.
    pub fn lines(&self) -> Vec<String> {
        match self {
            GateEvent::Lost { .. } => Vec::new(),
            GateEvent::Back { host, jobs } => vec![match jobs.as_slice() {
                [one] => format!(
                    "{host} is back. {} goes on.",
                    crate::engine::summary_word(one.kind)
                ),
                many => format!(
                    "{host} is back. {} go on.",
                    count(many.len() as u64, "job", "jobs")
                ),
            }],
            GateEvent::KeepWaiting { host } => vec![format!("Waiting for {host}.")],
            GateEvent::Cancelled { host, jobs } => {
                jobs.iter().map(|job| job.cancelled_line(host)).collect()
            }
            GateEvent::LostForGood { reason, jobs, .. } => jobs
                .iter()
                .map(|job| {
                    format!(
                        "{} stopped: {reason}",
                        crate::engine::summary_word(job.kind)
                    )
                })
                .collect(),
        }
    }
}

/// What waiting at a gate came to, for the job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The host answers: try the item again (resuming where it can).
    Back,
    /// The job was cancelled, by its own control or by Cancel these jobs.
    Cancelled,
    /// The provider said the loss is final, with its reason.
    LostForGood(String),
    /// No one can answer the question here: ask about the item.
    AskEachJob,
}

/// A listener for gate events.
type Listener = Arc<dyn Fn(&GateEvent) + Send + Sync>;

/// One host's gate.
#[derive(Debug, Default)]
struct GateState {
    /// The host is lost now.
    lost: bool,
    /// The provider said the loss is final, with this reason.
    gone: Option<String>,
    /// How many times the gate has opened: a waiter that saw it open goes
    /// on.
    opened: u64,
    /// How many times Cancel these jobs was answered: the waiters of that
    /// round leave.
    cancels: u64,
    /// How many times the question answered "no one can answer here".
    released: u64,
    /// The jobs that wait, in the order they came.
    waiting: Vec<Waiter>,
    /// The job whose thread probes and asks.
    prober: Option<JobId>,
}

#[derive(Debug, Default)]
struct HostGate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl HostGate {
    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Every host's gate, shared by every job of one engine (the engine's
/// clones share it). Lane A2 tells it what the plugins say: a host lost
/// ([`Gates::lost`], from `provider/lost`), back ([`Gates::back`]), or
/// lost for good ([`Gates::lost_for_good`]).
#[derive(Clone, Default)]
pub struct Gates {
    gates: Arc<Mutex<HashMap<HostKey, Arc<HostGate>>>>,
    listener: Arc<Mutex<Option<Listener>>>,
}

impl fmt::Debug for Gates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lost: Vec<String> = self
            .map()
            .iter()
            .filter(|(_, gate)| gate.lock().lost)
            .map(|(host, _)| host.to_string())
            .collect();
        f.debug_struct("Gates").field("lost", &lost).finish()
    }
}

/// What a job brings to a gate.
pub(crate) struct Visit<'a> {
    /// The host.
    pub host: &'a HostKey,
    /// The place to probe: the host's root, with the job's user.
    pub probe: &'a Place,
    /// The job, for the question and the events.
    pub waiter: Waiter,
    /// Where the probe's provider is found.
    pub registry: &'a Registry,
    /// The engine's clock: the waits between probes.
    pub pacer: &'a dyn Pacer,
    /// The waits of one budget.
    pub options: &'a EngineOptions,
    /// The job's own cancel.
    pub cancel: &'a CancelToken,
    /// The job's sink, which asks the question when this job probes.
    pub sink: &'a dyn JobSink,
}

impl Gates {
    /// No gate yet: every host answers until a probe says otherwise.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends every gate event to `listener` (the app's announcements),
    /// replacing an earlier one. Called on the job's thread: it must not
    /// block.
    pub fn on_event(&self, listener: impl Fn(&GateEvent) + Send + Sync + 'static) {
        *self.listener.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(listener));
    }

    /// Marks a host lost before any job finds it so (a plugin's
    /// `provider/lost`): the next failure on it waits at once.
    pub fn lost(&self, host: &HostKey) {
        let gate = self.gate(host);
        let mut state = gate.lock();
        if !state.lost {
            state.lost = true;
            drop(state);
            self.tell(&GateEvent::Lost {
                host: host.to_string(),
            });
        }
    }

    /// Opens a host's gate (a plugin connected again): every waiting job
    /// goes on, and a final loss is forgotten.
    pub fn back(&self, host: &HostKey) {
        let gate = self.gate(host);
        let jobs = {
            let mut state = gate.lock();
            state.gone = None;
            if !state.lost {
                return;
            }
            Self::open(&mut state)
        };
        gate.changed.notify_all();
        self.tell(&GateEvent::Back {
            host: host.to_string(),
            jobs,
        });
    }

    /// The provider says the loss is final (a plugin that stopped and
    /// will not come back): the waiting jobs stop with this reason, and
    /// later ones stop at once, until [`Gates::back`].
    pub fn lost_for_good(&self, host: &HostKey, reason: &str) {
        let gate = self.gate(host);
        let jobs = {
            let mut state = gate.lock();
            state.lost = true;
            state.gone = Some(reason.to_owned());
            state.waiting.clone()
        };
        gate.changed.notify_all();
        self.tell(&GateEvent::LostForGood {
            host: host.to_string(),
            reason: reason.to_owned(),
            jobs,
        });
    }

    /// Whether a host is held lost now.
    pub fn is_lost(&self, host: &HostKey) -> bool {
        self.map().get(host).is_some_and(|gate| gate.lock().lost)
    }

    /// The jobs waiting for a host now, in the order they came.
    pub fn waiting(&self, host: &HostKey) -> Vec<Waiter> {
        self.map()
            .get(host)
            .map(|gate| gate.lock().waiting.clone())
            .unwrap_or_default()
    }

    // ----- for the run -----

    /// Whether a host answers: `true` when the gate does not hold it lost
    /// and a probe now succeeds; a failed probe marks it lost.
    pub(crate) fn answers(&self, visit: &Visit<'_>) -> bool {
        let gate = self.gate(visit.host);
        if gate.lock().lost {
            return false;
        }
        if probe(visit.registry, visit.probe, visit.cancel) {
            return true;
        }
        let mut state = gate.lock();
        if !state.lost {
            state.lost = true;
            drop(state);
            self.tell(&GateEvent::Lost {
                host: visit.host.to_string(),
            });
        }
        false
    }

    /// Waits at a lost host's gate until it opens, the job is cancelled,
    /// the loss is final, or the question says otherwise. One waiting job
    /// probes at the backoff times and asks the one question; the others
    /// sleep on the gate.
    pub(crate) fn wait(&self, visit: &Visit<'_>) -> Outcome {
        let gate = self.gate(visit.host);
        let job = visit.waiter.job;
        let mut state = gate.lock();
        if let Some(reason) = &state.gone {
            return Outcome::LostForGood(reason.clone());
        }
        if !state.lost {
            return Outcome::Back;
        }
        let (opened, cancels, released) = (state.opened, state.cancels, state.released);
        state.waiting.push(visit.waiter.clone());
        loop {
            let left = if state.opened != opened {
                Some(Outcome::Back)
            } else if let Some(reason) = &state.gone {
                Some(Outcome::LostForGood(reason.clone()))
            } else if state.cancels != cancels || visit.cancel.is_cancelled() {
                Some(Outcome::Cancelled)
            } else if state.released != released {
                Some(Outcome::AskEachJob)
            } else {
                None
            };
            if let Some(outcome) = left {
                state.waiting.retain(|waiter| waiter.job != job);
                if state.prober == Some(job) {
                    state.prober = None;
                    gate.changed.notify_all();
                }
                return outcome;
            }
            if state.prober.is_none() {
                state.prober = Some(job);
            }
            if state.prober == Some(job) {
                drop(state);
                self.probe_round(&gate, visit);
                state = gate.lock();
                continue;
            }
            state = gate
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// One budget of probes, then the question; returns when the gate
    /// opened, the round ended by the answer, or the prober was cancelled.
    fn probe_round(&self, gate: &HostGate, visit: &Visit<'_>) {
        let host = visit.host.to_string();
        for wait in visit.options.retry_waits() {
            if !pace::wait(visit.pacer, wait, visit.cancel) {
                return;
            }
            if self.try_open(gate, visit) {
                return;
            }
        }
        if visit.cancel.is_cancelled() {
            return;
        }
        let question = ConnectionQuestion {
            host: host.clone(),
            jobs: gate.lock().waiting.clone(),
        };
        let answer = visit.sink.connection(&question);
        if visit.cancel.is_cancelled() {
            return;
        }
        match answer {
            ConnectionAnswer::KeepWaiting => self.tell(&GateEvent::KeepWaiting { host }),
            ConnectionAnswer::TryNow => {
                if !self.try_open(gate, visit) {
                    self.tell(&GateEvent::KeepWaiting { host });
                }
            }
            ConnectionAnswer::CancelJobs => {
                let jobs = {
                    let mut state = gate.lock();
                    state.cancels += 1;
                    state.waiting.clone()
                };
                gate.changed.notify_all();
                self.tell(&GateEvent::Cancelled { host, jobs });
            }
            ConnectionAnswer::AskEachJob => {
                {
                    let mut state = gate.lock();
                    state.released += 1;
                    state.lost = false;
                }
                gate.changed.notify_all();
            }
        }
    }

    /// Probes once; on success opens the gate and says so.
    fn try_open(&self, gate: &HostGate, visit: &Visit<'_>) -> bool {
        if !probe(visit.registry, visit.probe, visit.cancel) || visit.cancel.is_cancelled() {
            return false;
        }
        let jobs = {
            let mut state = gate.lock();
            if state.gone.is_some() {
                return false;
            }
            Self::open(&mut state)
        };
        gate.changed.notify_all();
        self.tell(&GateEvent::Back {
            host: visit.host.to_string(),
            jobs,
        });
        true
    }

    /// Opens a gate: the host answers, every waiter goes on.
    fn open(state: &mut GateState) -> Vec<Waiter> {
        state.lost = false;
        state.opened += 1;
        state.waiting.clone()
    }

    fn map(&self) -> MutexGuard<'_, HashMap<HostKey, Arc<HostGate>>> {
        self.gates.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn gate(&self, host: &HostKey) -> Arc<HostGate> {
        Arc::clone(self.map().entry(host.clone()).or_default())
    }

    fn tell(&self, event: &GateEvent) {
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(listener) = listener {
            listener(event);
        }
    }
}

/// Whether a host answers a stat of its root: any answer but a lost
/// network is an answer (a refusal says the host is there).
fn probe(registry: &Registry, place: &Place, cancel: &CancelToken) -> bool {
    let ctx = OpContext::new().with_cancel(cancel.clone());
    match registry
        .resolve(place)
        .and_then(|provider| provider.stat(place, &ctx))
    {
        Ok(_) => true,
        Err(error) => !matches!(error.class(), ErrorClass::Network | ErrorClass::Cancelled),
    }
}

/// The root of the host a place is on, with its user and port, to probe:
/// `sftp://ada@build:2222/`. `None` for a local place.
pub(crate) fn host_root(place: &Place) -> Option<Place> {
    if let Some((container, _)) = place.archive() {
        return host_root(container);
    }
    let remote = place.remote()?;
    let user = remote
        .user()
        .map(|user| format!("{user}@"))
        .unwrap_or_default();
    let port = remote
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Place::parse(&format!(
        "{}://{user}{}{port}/",
        remote.scheme(),
        remote.host()
    ))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiter(job: u64, item: Option<&str>) -> Waiter {
        Waiter {
            job: JobId::new(job),
            kind: JobKind::Copy,
            item: item.map(str::to_owned),
        }
    }

    #[test]
    fn the_lines_say_the_fact_first_in_40_cells() {
        let question = ConnectionQuestion {
            host: "build".into(),
            jobs: vec![waiter(1, None), waiter(2, None), waiter(3, None)],
        };
        assert_eq!(
            question.lines(),
            ["build is not answering.", "3 jobs wait for it."]
        );
        let one = ConnectionQuestion {
            host: "build".into(),
            jobs: vec![waiter(1, None)],
        };
        assert_eq!(one.lines()[1], "1 job waits for it.");
        assert_eq!(ConnectionAnswer::CHOICES[0].1, "Keep waiting");
        assert_eq!(
            ConnectionAnswer::CHOICES[0].0,
            ConnectionAnswer::KeepWaiting
        );
        let back = GateEvent::Back {
            host: "build".into(),
            jobs: vec![waiter(1, None)],
        };
        assert_eq!(back.lines(), ["build is back. Copy goes on."]);
        let many = GateEvent::Back {
            host: "build".into(),
            jobs: vec![waiter(1, None), waiter(2, None), waiter(3, None)],
        };
        assert_eq!(many.lines(), ["build is back. 3 jobs go on."]);
        assert_eq!(
            GateEvent::KeepWaiting {
                host: "build".into()
            }
            .lines(),
            ["Waiting for build."]
        );
        let cancelled = GateEvent::Cancelled {
            host: "build".into(),
            jobs: vec![waiter(1, Some("report.iso")), waiter(2, None)],
        };
        assert_eq!(
            cancelled.lines(),
            [
                "Copy cancelled: report.iso on build",
                "Copy cancelled on build"
            ]
        );
        for line in question
            .lines()
            .into_iter()
            .chain(back.lines())
            .chain(many.lines())
            .chain(cancelled.lines())
        {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert!(
            GateEvent::Lost {
                host: "build".into()
            }
            .lines()
            .is_empty()
        );
    }

    #[test]
    fn the_root_keeps_the_user_and_the_port() {
        let place = Place::parse("sftp://ada@build:2222/srv/data/x.iso").unwrap();
        assert_eq!(
            host_root(&place).unwrap().to_string(),
            "sftp://ada@build:2222/"
        );
        let inside = place.into_archive();
        assert_eq!(
            host_root(&inside).unwrap().to_string(),
            "sftp://ada@build:2222/"
        );
        let local = Place::parse(if cfg!(windows) { "D:\\x" } else { "/x" }).unwrap();
        assert_eq!(host_root(&local), None);
    }
}
