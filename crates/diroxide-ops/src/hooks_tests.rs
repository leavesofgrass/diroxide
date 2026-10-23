//! Hooks in the engine (R25 commit 71) with a fake runner: a veto stops
//! the job before its question; a silent hook goes on or stops as its
//! manifest says, with a line either way; an error is silence, never a
//! quiet veto; hooks never run on the caller's thread; the after event is
//! sent once with the report; a job cancelled while a hook is asked.

use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use diroxide_core::{JobId, JobKind, JobRequest};

use crate::engine::{Engine, JobControl};
use crate::error::OpsError;
use crate::fake::FakeProvider;
use crate::hooks::{
    HookCall, HookEvent, HookReport, HookResult, HookRunner, HookStatus, HookVerdict, OnSilence,
    WakingHook,
};
use crate::testing::{Script, engine, p};

#[derive(Clone, Debug)]
enum Behavior {
    Go,
    Veto(&'static str),
    Sleep(Duration),
    Break,
}

#[derive(Debug, Default)]
struct FakeRunner {
    hooks: Vec<(WakingHook, Behavior)>,
    calls: Mutex<Vec<HookCall>>,
    asked: Mutex<Vec<(String, ThreadId)>>,
    afters: Mutex<Vec<HookReport>>,
}

impl FakeRunner {
    fn with(hooks: Vec<(WakingHook, Behavior)>) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            ..Self::default()
        })
    }
}

impl HookRunner for FakeRunner {
    fn waking(&self, call: &HookCall) -> Vec<WakingHook> {
        self.calls.lock().unwrap().push(call.clone());
        self.hooks.iter().map(|(hook, _)| hook.clone()).collect()
    }

    fn before(&self, hook: &WakingHook, _call: &HookCall) -> HookVerdict {
        self.asked
            .lock()
            .unwrap()
            .push((hook.name.clone(), thread::current().id()));
        let behavior = self
            .hooks
            .iter()
            .find(|(known, _)| known.index == hook.index)
            .map(|(_, behavior)| behavior.clone())
            .unwrap_or(Behavior::Go);
        match behavior {
            Behavior::Go => HookVerdict::Continue,
            Behavior::Veto(line) => HookVerdict::Cancel(line.to_owned()),
            Behavior::Sleep(time) => {
                thread::sleep(time);
                HookVerdict::Continue
            }
            Behavior::Break => panic!("the hook broke"),
        }
    }

    fn after(&self, report: &HookReport) {
        self.afters.lock().unwrap().push(report.clone());
    }
}

fn hook(index: usize, name: &str, timeout_ms: u64, on_timeout: OnSilence) -> WakingHook {
    WakingHook {
        plugin: "guard".to_owned(),
        name: name.to_owned(),
        index,
        timeout: Duration::from_millis(timeout_ms),
        on_timeout,
    }
}

fn tree() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_file(&p("/w/keep.txt"), b"keep");
    fake.add_file(&p("/w/a.txt"), b"a");
    fake.add_folder(&p("/b"));
    fake
}

fn hooked(fake: &FakeProvider, runner: &Arc<FakeRunner>) -> Engine {
    engine(fake).with_hooks(Arc::clone(runner) as Arc<dyn HookRunner>)
}

fn trash(sources: &[&str]) -> JobRequest {
    JobRequest::new(
        JobId::new(5),
        JobKind::Trash,
        sources.iter().map(|s| p(s)).collect(),
    )
}

fn copy() -> JobRequest {
    JobRequest::new(JobId::new(6), JobKind::Copy, vec![p("/w/a.txt")]).to(p("/b"))
}

#[test]
fn a_veto_stops_the_job_before_its_question() {
    let fake = tree();
    let line = "Delete stopped: keep.txt is protected (Folder guard)";
    let runner = FakeRunner::with(vec![(
        hook(0, "Folder guard", 2000, OnSilence::Continue),
        Behavior::Veto(line),
    )]);
    let engine = hooked(&fake, &runner);
    let refused = engine.plan_with(
        &trash(&["/w/keep.txt"]),
        &JobControl::new(),
        &Script::default(),
    );
    assert_eq!(refused, Err(OpsError::HookStopped(line.to_owned())));
    assert_eq!(refused.unwrap_err().to_string(), line, "said as it is");
    assert!(fake.exists(&p("/w/keep.txt")), "nothing touched");
    let calls = runner.calls.lock().unwrap();
    assert_eq!(calls[0].event, HookEvent::Delete);
    assert_eq!(calls[0].event.before(), "before_delete");
    assert_eq!(calls[0].items[0].from, p("/w/keep.txt"));
    assert_eq!(calls[0].count, 1);
    assert!(calls[0].summary.starts_with("Move 1 item to the trash"));
}

#[test]
fn a_silent_hook_goes_on_or_stops_as_its_manifest_says() {
    let fake = tree();
    let slow = Behavior::Sleep(Duration::from_millis(600));
    let lenient = FakeRunner::with(vec![(
        hook(0, "Slow", 50, OnSilence::Continue),
        slow.clone(),
    )]);
    let start = Instant::now();
    let plan = hooked(&fake, &lenient)
        .plan_with(&copy(), &JobControl::new(), &Script::default())
        .unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "waited past the limit"
    );
    assert_eq!(plan.facts.hook_warnings, ["Copy goes on: no answer (Slow)"]);
    let strict = FakeRunner::with(vec![(hook(0, "Slow", 50, OnSilence::Cancel), slow)]);
    assert_eq!(
        hooked(&fake, &strict).plan_with(&copy(), &JobControl::new(), &Script::default()),
        Err(OpsError::HookStopped(
            "Copy stopped: no answer (Slow)".to_owned()
        ))
    );
}

#[test]
fn an_error_in_a_hook_is_silence_never_a_quiet_veto() {
    let fake = tree();
    let runner = FakeRunner::with(vec![
        (
            hook(0, "Broken", 2000, OnSilence::Continue),
            Behavior::Break,
        ),
        (hook(1, "Fine", 2000, OnSilence::Continue), Behavior::Go),
    ]);
    let plan = hooked(&fake, &runner)
        .plan_with(&copy(), &JobControl::new(), &Script::default())
        .unwrap();
    assert_eq!(
        plan.facts.hook_warnings,
        ["Copy goes on: no answer (Broken)"]
    );
    assert_eq!(
        runner.asked.lock().unwrap().len(),
        2,
        "the next hook is asked"
    );
}

#[test]
fn hooks_never_run_on_the_callers_thread() {
    let fake = tree();
    let runner = FakeRunner::with(vec![(
        hook(0, "Watcher", 2000, OnSilence::Continue),
        Behavior::Go,
    )]);
    let caller = thread::current().id();
    hooked(&fake, &runner)
        .plan_with(&copy(), &JobControl::new(), &Script::default())
        .unwrap();
    let asked = runner.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_ne!(asked[0].1, caller);
}

#[test]
fn the_after_event_is_sent_once_with_the_report() {
    let fake = tree();
    let runner = FakeRunner::with(vec![(
        hook(0, "Logger", 2000, OnSilence::Continue),
        Behavior::Go,
    )]);
    let engine = hooked(&fake, &runner);
    let plan = engine
        .plan_with(&copy(), &JobControl::new(), &Script::default())
        .unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.done.len(), 1);
    let start = Instant::now();
    while runner.afters.lock().unwrap().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5), "after never sent");
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(50));
    let afters = runner.afters.lock().unwrap();
    assert_eq!(afters.len(), 1, "once");
    assert_eq!(afters[0].event.after(), "after_copy");
    assert_eq!(afters[0].job, JobId::new(6));
    assert_eq!(afters[0].status, HookStatus::Done);
    assert_eq!(afters[0].items[0].result, Some(HookResult::Done));
}

#[test]
fn a_job_cancelled_while_a_hook_is_asked_stops_at_once() {
    let fake = tree();
    let runner = FakeRunner::with(vec![(
        hook(0, "Slow", 5000, OnSilence::Continue),
        Behavior::Sleep(Duration::from_millis(800)),
    )]);
    let control = JobControl::new();
    let cancel = control.cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        cancel.cancel();
    });
    let start = Instant::now();
    let result = hooked(&fake, &runner).plan_with(&copy(), &control, &Script::default());
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_millis(600));
}

#[test]
fn jobs_without_an_event_wake_no_hook() {
    let fake = tree();
    let runner = FakeRunner::with(vec![(
        hook(0, "Guard", 2000, OnSilence::Cancel),
        Behavior::Veto("Pack stopped: no (Guard)"),
    )]);
    let engine = hooked(&fake, &runner);
    let pack = JobRequest::new(JobId::new(7), JobKind::Pack, vec![p("/w/a.txt")]);
    assert!(matches!(
        engine.plan_with(&pack, &JobControl::new(), &Script::default()),
        Err(OpsError::NotBuilt(_))
    ));
    assert!(runner.calls.lock().unwrap().is_empty());
}
