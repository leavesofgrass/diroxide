//! The speed limit's tests (R25 commit 70) on a fake clock: three rates
//! within 10 percent, a change in the middle of a copy, the global limit
//! for remote jobs only unless all are chosen, and pause and cancel still
//! honored between chunks. Spike Q3's engine half is measured here.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diroxide_core::{JobId, JobKind, JobRequest, JobState, Place};
use diroxide_vfs::Registry;

use crate::engine::{Engine, JobControl, JobReport};
use crate::fake::FakeProvider;
use crate::limiter::{LimitScope, SpeedLimit};
use crate::options::EngineOptions;
use crate::pace::{FakePacer, Pacer};
use crate::testing::{Script, p};

const SIZE: usize = 2_000_000;

fn remote(path: &str) -> Place {
    Place::parse(&format!("sftp://build{path}")).unwrap()
}

/// A local file and a remote one of [`SIZE`] bytes, an engine on a fake
/// clock with chunks of 64 KiB.
fn world() -> (FakeProvider, Engine, Arc<FakePacer>) {
    let fake = FakeProvider::new();
    let data: Vec<u8> = (0..SIZE).map(|i| (i % 249) as u8).collect();
    fake.add_file(&p("/a/local.bin"), &data);
    fake.add_file(&remote("/srv/far.bin"), &data);
    fake.add_folder(&p("/b"));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    registry.register("sftp", Arc::new(fake.clone()));
    let pacer = Arc::new(FakePacer::new());
    let options = EngineOptions {
        chunk_bytes: 64 * 1024,
        ..EngineOptions::default()
    };
    let engine = Engine::new(Arc::new(registry))
        .with_options(options)
        .with_pacer(pacer.clone());
    (fake, engine, pacer)
}

fn copy(source: Place) -> JobRequest {
    JobRequest::new(JobId::new(1), JobKind::Copy, vec![source]).to(p("/b"))
}

fn timed(
    engine: &Engine,
    pacer: &FakePacer,
    request: &JobRequest,
    control: &JobControl,
    sink: &Script,
) -> (JobReport, Duration) {
    let plan = engine.plan(request).unwrap();
    let start = pacer.now();
    let report = engine.run(&plan, control, sink);
    (report, pacer.now() - start)
}

fn within_ten_percent(took: Duration, expected: f64) -> bool {
    let took = took.as_secs_f64();
    (took - expected).abs() <= expected * 0.1
}

/// Spike Q3's engine half: at 200 KB, 1 MB and 20 MB a second the
/// average over the copy is the limit within a chunk's rounding (the
/// fake clock shows the limiter alone; the multiplexer's read-ahead is the
/// SFTP lane's half).
#[test]
fn three_rates_hold_within_ten_percent() {
    for rate in [200_000u64, 1_000_000, 20_000_000] {
        let (fake, engine, pacer) = world();
        let control = JobControl::new();
        control.set_speed_limit(SpeedLimit::bytes_per_second(rate));
        let (report, took) = timed(
            &engine,
            &pacer,
            &copy(p("/a/local.bin")),
            &control,
            &Script::default(),
        );
        assert_eq!(report.done.len(), 1);
        assert_eq!(fake.contents(&p("/b/local.bin")).unwrap().len(), SIZE);
        let expected = SIZE as f64 / rate as f64;
        assert!(
            within_ten_percent(took, expected),
            "{rate}: took {took:?}, expected {expected} s"
        );
    }
}

#[test]
fn a_change_in_the_middle_of_a_copy_takes_effect_at_the_next_chunk() {
    let (_fake, engine, pacer) = world();
    let control = JobControl::new();
    control.set_speed_limit(SpeedLimit::bytes_per_second(200_000));
    let change = control.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.bytes_done >= SIZE as u64 / 2
            && change.speed_limit().is_some_and(|l| l.get() == 200_000)
        {
            change.set_speed_limit(SpeedLimit::bytes_per_second(1_000_000));
        }
    });
    let (report, took) = timed(&engine, &pacer, &copy(p("/a/local.bin")), &control, &sink);
    assert_eq!(report.done.len(), 1);
    let half = SIZE as f64 / 2.0;
    let expected = half / 200_000.0 + half / 1_000_000.0;
    assert!(
        within_ten_percent(took, expected),
        "took {took:?}, expected {expected} s"
    );
    assert_eq!(
        control.speed_limit(),
        SpeedLimit::bytes_per_second(1_000_000)
    );
}

#[test]
fn the_global_limit_slows_remote_jobs_only_unless_all_are_chosen() {
    let (_fake, engine, pacer) = world();
    engine.speed_limit().set(SpeedLimit::parse("1 MB"));
    let (_, local) = timed(
        &engine,
        &pacer,
        &copy(p("/a/local.bin")),
        &JobControl::new(),
        &Script::default(),
    );
    assert_eq!(local, Duration::ZERO, "a local job is not slowed");
    let (report, far) = timed(
        &engine,
        &pacer,
        &copy(remote("/srv/far.bin")),
        &JobControl::new(),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 1);
    assert!(within_ten_percent(far, 2.0), "{far:?}");
    let (_fake, engine, pacer) = world();
    engine.speed_limit().set(SpeedLimit::parse("1 MB"));
    engine.speed_limit().set_scope(LimitScope::AllJobs);
    let (_, all) = timed(
        &engine,
        &pacer,
        &copy(p("/a/local.bin")),
        &JobControl::new(),
        &Script::default(),
    );
    assert!(within_ten_percent(all, 2.0), "{all:?}");
    // Both limits: the slower wins.
    let (_fake, engine, pacer) = world();
    engine.speed_limit().set(SpeedLimit::parse("1 MB"));
    let control = JobControl::new();
    control.set_speed_limit(SpeedLimit::parse("500 KB"));
    let (_, both) = timed(
        &engine,
        &pacer,
        &copy(remote("/srv/far.bin")),
        &control,
        &Script::default(),
    );
    assert!(within_ten_percent(both, 4.0), "{both:?}");
}

#[test]
fn cancel_and_pause_are_still_honored_between_chunks() {
    let (fake, engine, pacer) = world();
    let control = JobControl::new();
    control.set_speed_limit(SpeedLimit::parse("100 KB"));
    let cancel = control.cancel.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.bytes_done > 0 {
            cancel.cancel();
        }
    });
    let (report, took) = timed(&engine, &pacer, &copy(p("/a/local.bin")), &control, &sink);
    assert!(report.cancelled);
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert!(fake.unfinished().is_empty());
    let (fake, engine, pacer) = world();
    let control = JobControl::new();
    control.set_speed_limit(SpeedLimit::parse("1 MB"));
    let pause = control.clone();
    let paused = Arc::new(Mutex::new(false));
    let once = Arc::clone(&paused);
    let sink = Script::default().on_progress(move |progress| {
        let mut done = once.lock().unwrap();
        if progress.bytes_done > 0 && !*done {
            *done = true;
            pause.pause();
            let resume = pause.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(60));
                resume.resume();
            });
        }
    });
    let (report, _) = timed(&engine, &pacer, &copy(p("/a/local.bin")), &control, &sink);
    assert_eq!(report.done.len(), 1);
    assert!(sink.state_changes().contains(&JobState::Paused));
    assert_eq!(fake.contents(&p("/b/local.bin")).unwrap().len(), SIZE);
}
