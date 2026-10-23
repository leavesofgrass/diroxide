//! The connection gate's tests (R25 commit 68), on the fake provider's
//! offline and drop modes and a fake clock: twelve jobs on a lost host
//! ask one question; the first success lets every job go on; cancel while
//! waiting; a provider whose loss is final; a host that answers is not
//! waited for; a frontend without the question asks per item as before.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::format::within_cells;
use diroxide_core::{ConflictAnswer, ErrorAction, JobId, JobKind, JobRequest, JobState, Place};
use diroxide_vfs::{Registry, VfsError};

use crate::engine::{Conflict, Engine, ItemError, JobControl, JobReport, JobSink};
use crate::fake::{CallFault, FakeCall, FakeProvider, LinkDrop};
use crate::gate::{ConnectionAnswer, ConnectionQuestion, GateEvent};
use crate::pace::{FakePacer, Pacer};
use crate::plan::Plan;
use crate::progress::JobProgress;
use crate::queue::HostKey;
use crate::testing::{Script, p};

fn host_root() -> Place {
    Place::parse("sftp://build/").unwrap()
}

fn host() -> HostKey {
    HostKey::new("sftp", "build", None)
}

fn remote(path: &str) -> Place {
    Place::parse(&format!("sftp://build{path}")).unwrap()
}

/// A fake that serves local places and `sftp://build`, with `count` files
/// of `size` bytes on the host, an empty local `/b`, and an engine on a
/// fake clock.
fn world(count: usize, size: usize) -> (FakeProvider, Engine, Arc<FakePacer>) {
    let fake = FakeProvider::new();
    for n in 0..count {
        let data: Vec<u8> = (0..size).map(|i| ((i + n) % 251) as u8).collect();
        fake.add_file(&remote(&format!("/srv/f{n:02}.bin")), &data);
    }
    fake.add_folder(&p("/b"));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    registry.register("sftp", Arc::new(fake.clone()));
    let pacer = Arc::new(FakePacer::new());
    let engine = Engine::new(Arc::new(registry)).with_pacer(pacer.clone());
    (fake, engine, pacer)
}

fn plan(engine: &Engine, job: u64, n: usize) -> Plan {
    let request = JobRequest::new(
        JobId::new(job),
        JobKind::Copy,
        vec![remote(&format!("/srv/f{n:02}.bin"))],
    )
    .to(p("/b"));
    engine.plan(&request).unwrap()
}

/// Run when the question is answered: the test's world changes then.
type OnAnswer = Box<dyn FnMut(ConnectionAnswer) + Send>;

/// What the question's sink shares across the jobs: the questions asked,
/// the answers to give, and when it may answer.
struct Asked {
    engine: Engine,
    pacer: Arc<FakePacer>,
    questions: Mutex<Vec<(Duration, ConnectionQuestion)>>,
    answers: Mutex<VecDeque<ConnectionAnswer>>,
    /// The sink answers once this many jobs wait and the test lets it.
    wait_for: usize,
    release: AtomicBool,
    on_answer: Mutex<Option<OnAnswer>>,
}

impl Asked {
    fn new(engine: &Engine, pacer: &Arc<FakePacer>, wait_for: usize) -> Arc<Self> {
        Arc::new(Self {
            engine: engine.clone(),
            pacer: Arc::clone(pacer),
            questions: Mutex::new(Vec::new()),
            answers: Mutex::new(VecDeque::new()),
            wait_for,
            release: AtomicBool::new(true),
            on_answer: Mutex::new(None),
        })
    }

    fn answer(&self, answers: &[ConnectionAnswer]) {
        self.answers.lock().unwrap().extend(answers.iter().copied());
    }

    fn asked(&self) -> usize {
        self.questions.lock().unwrap().len()
    }
}

/// One job's sink: its states and lines, its own control (a cancelled job
/// stops waiting for the answer), the shared question.
struct HostSink {
    control: JobControl,
    asked: Arc<Asked>,
    states: Mutex<Vec<JobState>>,
    said: Mutex<Vec<String>>,
    at_waiting: Mutex<Option<Duration>>,
}

impl HostSink {
    fn new(control: &JobControl, asked: &Arc<Asked>) -> Self {
        Self {
            control: control.clone(),
            asked: Arc::clone(asked),
            states: Mutex::new(Vec::new()),
            said: Mutex::new(Vec::new()),
            at_waiting: Mutex::new(None),
        }
    }

    fn waited_once(&self) -> bool {
        let states = self.states.lock().unwrap();
        let mut changes: Vec<JobState> = Vec::new();
        for state in states.iter() {
            if changes.last() != Some(state) {
                changes.push(*state);
            }
        }
        changes
            .iter()
            .filter(|state| **state == JobState::WaitingForConnection)
            .count()
            == 1
    }
}

impl JobSink for HostSink {
    fn progress(&self, progress: &JobProgress) {
        if progress.state == JobState::WaitingForConnection {
            let mut at = self.at_waiting.lock().unwrap();
            at.get_or_insert(self.asked.pacer.now());
        }
        self.states.lock().unwrap().push(progress.state);
        self.said.lock().unwrap().push(progress.describe());
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }

    fn connection(&self, question: &ConnectionQuestion) -> ConnectionAnswer {
        let start = Instant::now();
        loop {
            if self.control.cancel.is_cancelled() {
                return ConnectionAnswer::KeepWaiting;
            }
            let waiting = self.asked.engine.gates().waiting(&host()).len();
            if self.asked.release.load(Ordering::SeqCst) && waiting >= self.asked.wait_for {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "the question was never let through"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let mut question = question.clone();
        question.jobs = self.asked.engine.gates().waiting(&host());
        self.asked
            .questions
            .lock()
            .unwrap()
            .push((self.asked.pacer.now(), question));
        let answer = self
            .asked
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ConnectionAnswer::CancelJobs);
        if let Some(hook) = self.asked.on_answer.lock().unwrap().as_mut() {
            hook(answer);
        }
        answer
    }
}

/// Runs each plan on its own thread with its own control and sink.
fn run_all(engine: &Engine, jobs: &[(Plan, JobControl, HostSink)]) -> Vec<JobReport> {
    thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(plan, control, sink)| scope.spawn(move || engine.run(plan, control, sink)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    })
}

fn events(engine: &Engine) -> Arc<Mutex<Vec<GateEvent>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sent = Arc::clone(&events);
    engine
        .gates()
        .on_event(move |event| sent.lock().unwrap().push(event.clone()));
    events
}

#[test]
fn twelve_jobs_on_a_lost_host_ask_one_question() {
    let (fake, engine, pacer) = world(12, 3000);
    let events = events(&engine);
    let asked = Asked::new(&engine, &pacer, 12);
    asked.answer(&[ConnectionAnswer::CancelJobs]);
    let jobs: Vec<(Plan, JobControl, HostSink)> = (0..12)
        .map(|n| {
            let control = JobControl::new();
            let sink = HostSink::new(&control, &asked);
            (plan(&engine, n as u64 + 1, n), control, sink)
        })
        .collect();
    fake.take_offline(&host_root());
    let reports = run_all(&engine, &jobs);
    assert_eq!(asked.asked(), 1, "one question for the host");
    let (_, question) = asked.questions.lock().unwrap()[0].clone();
    assert_eq!(question.jobs.len(), 12);
    assert_eq!(
        question.lines(),
        ["build is not answering.", "12 jobs wait for it."]
    );
    for (report, (_, _, sink)) in reports.iter().zip(&jobs) {
        assert!(report.cancelled, "{}", report.summary());
        assert!(report.done.is_empty());
        assert!(sink.waited_once(), "{:?}", sink.states.lock().unwrap());
        assert!(
            sink.said
                .lock()
                .unwrap()
                .iter()
                .any(|line| line == "Copying waits for build")
        );
    }
    let events = events.lock().unwrap();
    let cancelled = events
        .iter()
        .find_map(|event| match event {
            GateEvent::Cancelled { jobs, .. } => Some(jobs.len()),
            _ => None,
        })
        .expect("the cancel is said");
    assert_eq!(cancelled, 12, "cancel names every job");
    let lines: Vec<String> = events.iter().flat_map(GateEvent::lines).collect();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("Copy cancelled: f"))
    );
    assert!(
        lines
            .iter()
            .all(|line| within_cells(line, "Copy cancelled"))
    );
    assert!(fake.unfinished().is_empty(), "no half file left");
}

#[test]
fn the_first_success_opens_the_gate_for_every_job() {
    let (fake, engine, pacer) = world(3, 5000);
    let events = events(&engine);
    let asked = Asked::new(&engine, &pacer, 3);
    asked.answer(&[ConnectionAnswer::KeepWaiting]);
    let back = fake.clone();
    *asked.on_answer.lock().unwrap() = Some(Box::new(move |_| back.bring_back(&host_root())));
    let jobs: Vec<(Plan, JobControl, HostSink)> = (0..3)
        .map(|n| {
            let control = JobControl::new();
            let sink = HostSink::new(&control, &asked);
            (plan(&engine, n as u64 + 1, n), control, sink)
        })
        .collect();
    fake.take_offline(&host_root());
    let reports = run_all(&engine, &jobs);
    assert_eq!(asked.asked(), 1);
    for (n, report) in reports.iter().enumerate() {
        assert_eq!(report.done.len(), 1, "{}", report.summary());
        let name = format!("/srv/f{n:02}.bin");
        assert_eq!(
            fake.contents(&p(&format!("/b/f{n:02}.bin"))),
            fake.contents(&remote(&name))
        );
    }
    let events = events.lock().unwrap();
    let lines: Vec<String> = events.iter().flat_map(GateEvent::lines).collect();
    assert!(
        lines.contains(&"Waiting for build.".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"build is back. 3 jobs go on.".to_owned()),
        "{lines:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, GateEvent::Back { .. }))
            .count(),
        1,
        "said once"
    );
    assert!(!engine.gates().is_lost(&host()));
}

/// The drop mode: a copy cut after N bytes waits, comes back, and its
/// result equals the source. Spike Q2's engine half is measured here on
/// the fake clock: the wait is said at the moment of the loss, and the
/// question comes after 5, 15 and 45 seconds, 65 in all.
#[test]
fn a_copy_cut_by_a_drop_waits_and_finishes_whole() {
    let (fake, engine, pacer) = world(1, 3_000_000);
    let asked = Asked::new(&engine, &pacer, 1);
    asked.answer(&[ConnectionAnswer::KeepWaiting]);
    let back = fake.clone();
    *asked.on_answer.lock().unwrap() = Some(Box::new(move |_| back.bring_back(&host_root())));
    let control = JobControl::new();
    let sink = HostSink::new(&control, &asked);
    let jobs = vec![(plan(&engine, 1, 0), control, sink)];
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), 1_500_000));
    let started = pacer.now();
    let reports = run_all(&engine, &jobs);
    assert_eq!(reports[0].done.len(), 1, "{}", reports[0].summary());
    assert_eq!(
        fake.contents(&p("/b/f00.bin")),
        fake.contents(&remote("/srv/f00.bin"))
    );
    let waiting = jobs[0].2.at_waiting.lock().unwrap().expect("it waited");
    assert_eq!(waiting, started, "the wait is said at once");
    let (asked_at, _) = asked.questions.lock().unwrap()[0].clone();
    assert_eq!(asked_at - waiting, Duration::from_secs(65));
    assert!(reports[0].retries >= 1);
}

#[test]
fn a_job_cancelled_while_it_waits_leaves_and_the_others_wait_on() {
    let (fake, engine, pacer) = world(2, 2000);
    let events = events(&engine);
    let asked = Asked::new(&engine, &pacer, 1);
    asked.release.store(false, Ordering::SeqCst);
    asked.answer(&[ConnectionAnswer::CancelJobs]);
    let jobs: Vec<(Plan, JobControl, HostSink)> = (0..2)
        .map(|n| {
            let control = JobControl::new();
            let sink = HostSink::new(&control, &asked);
            (plan(&engine, n as u64 + 1, n), control, sink)
        })
        .collect();
    fake.take_offline(&host_root());
    let reports = thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(plan, control, sink)| {
                let engine = &engine;
                scope.spawn(move || engine.run(plan, control, sink))
            })
            .collect();
        let start = Instant::now();
        while engine.gates().waiting(&host()).len() < 2 {
            assert!(start.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(5));
        }
        jobs[0].1.cancel.cancel();
        while engine.gates().waiting(&host()).len() > 1 {
            assert!(start.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            engine.gates().waiting(&host())[0].job,
            JobId::new(2),
            "the other job waits on"
        );
        asked.release.store(true, Ordering::SeqCst);
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(reports[0].cancelled && reports[1].cancelled);
    let events = events.lock().unwrap();
    let named: Vec<JobId> = events
        .iter()
        .find_map(|event| match event {
            GateEvent::Cancelled { jobs, .. } => Some(jobs.iter().map(|job| job.job).collect()),
            _ => None,
        })
        .unwrap();
    assert_eq!(named, [JobId::new(2)], "cancel names the jobs it stopped");
}

/// The plugin killed: a provider that says its loss is final stops every
/// waiting job with its reason.
#[test]
fn a_final_loss_stops_the_waiting_jobs_with_its_reason() {
    let (fake, engine, pacer) = world(2, 2000);
    let events = events(&engine);
    let asked = Asked::new(&engine, &pacer, 1);
    asked.release.store(false, Ordering::SeqCst);
    asked.answer(&[ConnectionAnswer::KeepWaiting]);
    let jobs: Vec<(Plan, JobControl, HostSink)> = (0..2)
        .map(|n| {
            let control = JobControl::new();
            let sink = HostSink::new(&control, &asked);
            (plan(&engine, n as u64 + 1, n), control, sink)
        })
        .collect();
    fake.take_offline(&host_root());
    let reports = thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(plan, control, sink)| {
                let engine = &engine;
                scope.spawn(move || engine.run(plan, control, sink))
            })
            .collect();
        let start = Instant::now();
        while engine.gates().waiting(&host()).len() < 2 {
            assert!(start.elapsed() < Duration::from_secs(20));
            thread::sleep(Duration::from_millis(5));
        }
        engine
            .gates()
            .lost_for_good(&host(), "the SFTP plugin stopped");
        asked.release.store(true, Ordering::SeqCst);
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    for report in &reports {
        assert!(report.cancelled);
        assert_eq!(report.failed[0].1, "the SFTP plugin stopped");
    }
    let lines: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .flat_map(GateEvent::lines)
        .collect();
    assert!(lines.contains(&"Copy stopped: the SFTP plugin stopped".to_owned()));
    // Until the provider is back, a new job stops at once.
    engine.gates().back(&host());
    assert!(!engine.gates().is_lost(&host()));
}

/// A passing failure on a host that answers its probe is no loss: the
/// item's own silent retry, no waiting state, no question.
#[test]
fn a_host_that_answers_is_not_waited_for() {
    let (fake, engine, pacer) = world(1, 100);
    fake.switches_mut().call_faults.push(
        CallFault::new(
            FakeCall::Read,
            remote("/srv/f00.bin"),
            VfsError::NetworkLost,
        )
        .times(1),
    );
    let sink = Script::default();
    let report = engine.run(&plan(&engine, 1, 0), &JobControl::new(), &sink);
    assert_eq!(report.done.len(), 1);
    assert_eq!(report.retries, 1);
    assert!(
        !sink
            .states
            .lock()
            .unwrap()
            .contains(&JobState::WaitingForConnection)
    );
    assert_eq!(pacer.now(), Duration::from_secs(5));
    assert!(!engine.gates().is_lost(&host()));
}

/// A frontend without the question (the default answer) gets the alpha 1
/// behavior after the budget: each job asks about its item.
#[test]
fn without_the_question_each_job_asks_about_its_item() {
    let (fake, engine, _pacer) = world(2, 100);
    let plans = [plan(&engine, 1, 0), plan(&engine, 2, 1)];
    fake.take_offline(&host_root());
    let sinks = [Script::default(), Script::default()];
    let reports: Vec<JobReport> = thread::scope(|scope| {
        let handles: Vec<_> = plans
            .iter()
            .zip(&sinks)
            .map(|(plan, sink)| {
                let engine = &engine;
                scope.spawn(move || engine.run(plan, &JobControl::new(), sink))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (report, sink) in reports.iter().zip(&sinks) {
        assert_eq!(report.failed.len(), 1, "{}", report.summary());
        assert_eq!(report.failed[0].1, "the network connection was lost");
        assert_eq!(sink.failed.lock().unwrap().len(), 1, "asked about its item");
    }
}
