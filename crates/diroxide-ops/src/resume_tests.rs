//! Resume's tests (R25 commit 69) on the fake provider's drop mode and a
//! fake clock, downloads and uploads: a cut copy goes on at its offset and
//! equals its source; the source changed, a part file longer than the
//! source and a corrupted tail start again and say why; Cancel removes the
//! part file; a lost job keeps it for a later one; two jobs with one
//! target never share a part file; a resumed move reads the prefix again
//! before the read-back. The cases of spike Q1 that the engine can show
//! without a server are here too.

use std::sync::{Arc, Mutex};

use diroxide_core::{ConflictAnswer, ErrorAction, JobId, JobKind, JobRequest, JobState, Place};
use diroxide_vfs::Registry;

use crate::engine::{Conflict, Engine, ItemError, JobControl, JobReport, JobSink};
use crate::fake::{FakeProvider, LinkDrop};
use crate::gate::{ConnectionAnswer, ConnectionQuestion};
use crate::options::EngineOptions;
use crate::pace::FakePacer;
use crate::progress::JobProgress;
use crate::resume::{JobNotice, NotResumed};
use crate::testing::p;

const SIZE: usize = 3 * 1024 * 1024;
const CUT: u64 = 1536 * 1024;

fn host_root() -> Place {
    Place::parse("sftp://build/").unwrap()
}

fn remote(path: &str) -> Place {
    Place::parse(&format!("sftp://build{path}")).unwrap()
}

/// Bytes that differ from place to place, so a window that is off by a
/// byte never matches by chance.
fn data(seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    (0..SIZE)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

/// A fake serving local places and `sftp://build`, both resumable, a
/// file on each side, and an engine on a fake clock that resumes files of
/// 1 MiB and more, in chunks of 256 KiB.
fn world() -> (FakeProvider, Engine) {
    let fake = FakeProvider::new();
    fake.switches_mut().resume = true;
    fake.add_file(&remote("/srv/big.bin"), &data(1));
    fake.add_file(&p("/a/up.bin"), &data(2));
    fake.add_folder(&remote("/inbox"));
    fake.add_folder(&p("/b"));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    registry.register("sftp", Arc::new(fake.clone()));
    let options = EngineOptions {
        resume_over: 1024 * 1024,
        chunk_bytes: 256 * 1024,
        ..EngineOptions::default()
    };
    let engine = Engine::new(Arc::new(registry))
        .with_options(options)
        .with_pacer(Arc::new(FakePacer::new()));
    (fake, engine)
}

type OnWait = Box<dyn FnMut() + Send>;

/// A sink that runs `on_wait` the first time the job waits for the
/// connection, answers the question as told, skips errors, and records
/// its notices and states.
struct Watch {
    on_wait: Mutex<Option<OnWait>>,
    answer: ConnectionAnswer,
    notices: Mutex<Vec<JobNotice>>,
    states: Mutex<Vec<JobState>>,
}

impl Watch {
    fn new(answer: ConnectionAnswer, on_wait: impl FnMut() + Send + 'static) -> Self {
        Self {
            on_wait: Mutex::new(Some(Box::new(on_wait))),
            answer,
            notices: Mutex::new(Vec::new()),
            states: Mutex::new(Vec::new()),
        }
    }

    fn lines(&self) -> Vec<String> {
        self.notices
            .lock()
            .unwrap()
            .iter()
            .flat_map(JobNotice::lines)
            .collect()
    }
}

impl JobSink for Watch {
    fn progress(&self, progress: &JobProgress) {
        self.states.lock().unwrap().push(progress.state);
        if progress.state == JobState::WaitingForConnection
            && let Some(mut hook) = self.on_wait.lock().unwrap().take()
        {
            hook();
        }
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }

    fn connection(&self, _question: &ConnectionQuestion) -> ConnectionAnswer {
        self.answer
    }

    fn notice(&self, notice: &JobNotice) {
        self.notices.lock().unwrap().push(notice.clone());
    }
}

fn run(engine: &Engine, request: &JobRequest, sink: &Watch) -> JobReport {
    let plan = engine.plan(request).unwrap();
    engine.run(&plan, &JobControl::new(), sink)
}

fn download(job: u64) -> JobRequest {
    JobRequest::new(JobId::new(job), JobKind::Copy, vec![remote("/srv/big.bin")]).to(p("/b"))
}

/// The part file the fake holds now, if any.
fn part(fake: &FakeProvider) -> Option<Place> {
    fake.unfinished()
        .first()
        .map(|text| Place::parse(text).unwrap())
}

#[test]
fn a_cut_download_goes_on_at_its_offset_and_equals_its_source() {
    let (fake, engine) = world();
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let back = fake.clone();
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || {
        back.bring_back(&host_root());
    });
    let report = run(&engine, &download(1), &sink);
    assert_eq!(report.done.len(), 1, "{}", report.summary());
    assert_eq!(fake.contents(&p("/b/big.bin")).unwrap(), data(1));
    assert_eq!(
        *sink.notices.lock().unwrap(),
        [JobNotice::Resuming {
            name: "big.bin".into(),
            at: CUT,
            of: SIZE as u64
        }]
    );
    assert_eq!(sink.lines(), ["Resuming big.bin at 1.6 MB of 3.1 MB"]);
    assert!(
        fake.unfinished().is_empty(),
        "the part file became the file"
    );
    assert!(engine.ledger().lines().is_empty(), "its line is gone");
}

#[test]
fn a_cut_upload_goes_on_at_its_offset_too() {
    let (fake, engine) = world();
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let back = fake.clone();
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || {
        back.bring_back(&host_root());
    });
    let upload =
        JobRequest::new(JobId::new(1), JobKind::Copy, vec![p("/a/up.bin")]).to(remote("/inbox"));
    let report = run(&engine, &upload, &sink);
    assert_eq!(report.done.len(), 1, "{}", report.summary());
    assert_eq!(fake.contents(&remote("/inbox/up.bin")).unwrap(), data(2));
    assert!(matches!(
        sink.notices.lock().unwrap()[0],
        JobNotice::Resuming { at: CUT, .. }
    ));
}

/// Each refusal says why, removes the part file and starts again; the
/// result still equals the source.
#[test]
fn a_part_file_that_does_not_agree_starts_again_and_says_why() {
    type Spoil = fn(&FakeProvider, &Place);
    let cases: [(NotResumed, Spoil); 3] = [
        (NotResumed::SourceChanged, |fake, _| {
            let source = remote("/srv/big.bin");
            let modified = fake.entry(&source).unwrap().modified;
            let later = modified.map_or(diroxide_core::Timestamp::from_unix(1, 0), |t| {
                t.plus_seconds(60)
            });
            fake.set_modified(&source, later);
        }),
        (NotResumed::TooLong, |fake, part| {
            let mut longer = fake.contents(part).unwrap();
            longer.resize(SIZE + 10, 7);
            fake.add_file(part, &longer);
        }),
        (NotResumed::Differs, |fake, part| {
            let mut bytes = fake.contents(part).unwrap();
            let last = bytes.len() - 100;
            bytes[last] ^= 0xFF;
            fake.add_file(part, &bytes);
        }),
    ];
    for (why, spoil) in cases {
        let (fake, engine) = world();
        fake.switches_mut()
            .drops
            .push(LinkDrop::after(host_root(), CUT));
        let back = fake.clone();
        let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || {
            let part = part(&back).expect("the part file is kept");
            spoil(&back, &part);
            back.bring_back(&host_root());
        });
        let report = run(&engine, &download(1), &sink);
        assert_eq!(report.done.len(), 1, "{why:?}: {}", report.summary());
        assert_eq!(fake.contents(&p("/b/big.bin")).unwrap(), data(1), "{why:?}");
        assert_eq!(
            sink.lines(),
            [why.line(), "Starting again from the start."],
            "{why:?}"
        );
        assert!(fake.unfinished().is_empty(), "{why:?}");
    }
}

/// Spike Q1, the engine's half: a server that crashed before it flushed
/// keeps the part file's length with zeros at its tail; the window check
/// catches it.
#[test]
fn q1_a_zero_filled_tail_is_caught_by_the_window_check() {
    let (fake, engine) = world();
    fake.switches_mut().lies.kept_tail_zeroed = 4096;
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let back = fake.clone();
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || {
        back.bring_back(&host_root());
    });
    let report = run(&engine, &download(1), &sink);
    assert_eq!(report.done.len(), 1);
    assert_eq!(fake.contents(&p("/b/big.bin")).unwrap(), data(1));
    assert_eq!(sink.lines()[0], NotResumed::Differs.line());
}

/// Spike Q1's limit: a byte spoiled between the two windows passes the
/// window check. A plain copy keeps it unless copies are read back; a
/// move, whose source prefix is read again for its hash, never removes
/// the source over it.
#[test]
fn q1_a_spoiled_middle_passes_the_window_and_only_a_read_back_catches_it() {
    let spoil_middle = |back: FakeProvider| {
        move || {
            let part = part(&back).expect("the part file is kept");
            let mut bytes = back.contents(&part).unwrap();
            bytes[200 * 1024] ^= 0xFF;
            back.add_file(&part, &bytes);
            back.bring_back(&host_root());
        }
    };
    // A plain copy with its read-back on: caught, the target removed.
    let (fake, engine) = world();
    let options = EngineOptions {
        verify_copies: true,
        ..engine.options().clone()
    };
    let engine = engine.with_options(options);
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, spoil_middle(fake.clone()));
    let report = run(&engine, &download(1), &sink);
    assert_eq!(report.failed.len(), 1, "{}", report.summary());
    assert!(!fake.exists(&p("/b/big.bin")));
    // A move: the source is read again for the hash and the read-back
    // disagrees, so the source stays where it is.
    let (fake, engine) = world();
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, spoil_middle(fake.clone()));
    let moving =
        JobRequest::new(JobId::new(1), JobKind::Move, vec![remote("/srv/big.bin")]).to(p("/b"));
    let report = run(&engine, &moving, &sink);
    assert_eq!(report.failed.len(), 1, "{}", report.summary());
    assert!(
        report.failed[0].1.contains("did not match"),
        "{}",
        report.failed[0].1
    );
    assert_eq!(fake.contents(&remote("/srv/big.bin")).unwrap(), data(1));
}

#[test]
fn a_resumed_move_verifies_whole_before_the_source_goes() {
    let (fake, engine) = world();
    let options = EngineOptions {
        verify_moves: false,
        ..engine.options().clone()
    };
    let engine = engine.with_options(options);
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let back = fake.clone();
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || {
        back.bring_back(&host_root());
    });
    let moving =
        JobRequest::new(JobId::new(1), JobKind::Move, vec![remote("/srv/big.bin")]).to(p("/b"));
    let report = run(&engine, &moving, &sink);
    assert_eq!(report.done.len(), 1, "{}", report.summary());
    assert_eq!(fake.contents(&p("/b/big.bin")).unwrap(), data(1));
    assert!(!fake.exists(&remote("/srv/big.bin")));
    assert!(
        sink.states.lock().unwrap().contains(&JobState::Verifying),
        "a remote move reads back even with the setting off (answer 96)"
    );
}

#[test]
fn cancel_removes_the_part_file() {
    let (fake, engine) = world();
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let control = JobControl::new();
    let cancel = control.cancel.clone();
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || cancel.cancel());
    let plan = engine.plan(&download(1)).unwrap();
    let report = engine.run(&plan, &control, &sink);
    assert!(report.cancelled);
    assert!(fake.unfinished().is_empty(), "{:?}", fake.unfinished());
    assert!(engine.ledger().lines().is_empty());
}

/// A job that ends on a lost connection keeps its part file, listed; a
/// later job resumes it. A running job's part file is its own: another
/// job with the same target starts its own.
#[test]
fn a_kept_part_file_is_resumed_later_and_never_shared_with_a_running_job() {
    let (fake, engine) = world();
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let lost = Watch::new(ConnectionAnswer::AskEachJob, || {});
    let first = run(&engine, &download(1), &lost);
    assert_eq!(first.failed.len(), 1, "{}", first.summary());
    let kept = part(&fake).expect("a lost connection keeps the part file");
    let leftovers = engine.ledger().leftovers();
    assert_eq!(leftovers.len(), 1, "listed, never removed unasked");
    assert_eq!(leftovers[0].unfinished, kept);
    fake.bring_back(&host_root());
    // While job 1 counts as running, job 2 does not take its part file.
    engine.ledger().claim(JobId::new(1));
    let second = Watch::new(ConnectionAnswer::KeepWaiting, || {});
    let other = engine.plan(&download(2)).unwrap();
    let report = engine.run(&other, &JobControl::new(), &second);
    assert_eq!(report.done.len(), 1);
    assert!(second.notices.lock().unwrap().is_empty(), "nothing resumed");
    assert!(fake.exists(&kept), "job 1's part file is untouched");
    engine.ledger().release(JobId::new(1));
    assert_eq!(engine.ledger().leftovers().len(), 1, "still listed");
    // The owner removes the copy job 2 made; job 4 resumes job 1's part.
    fake.vanish(&p("/b/big.bin"));
    let fourth = Watch::new(ConnectionAnswer::KeepWaiting, || {});
    let report = run(&engine, &download(4), &fourth);
    assert_eq!(report.done.len(), 1, "{}", report.summary());
    assert!(matches!(
        fourth.notices.lock().unwrap()[0],
        JobNotice::Resuming { at: CUT, .. }
    ));
    assert_eq!(fake.contents(&p("/b/big.bin")).unwrap(), data(1));
    assert!(fake.unfinished().is_empty());
    assert!(engine.ledger().lines().is_empty());
}

/// A small file is not worth a part file: it starts again, as before.
#[test]
fn a_small_file_starts_again_without_a_part_file() {
    let (fake, engine) = world();
    let options = EngineOptions {
        resume_over: 64 * 1024 * 1024,
        ..engine.options().clone()
    };
    let engine = engine.with_options(options);
    fake.switches_mut()
        .drops
        .push(LinkDrop::after(host_root(), CUT));
    let back = fake.clone();
    let sink = Watch::new(ConnectionAnswer::KeepWaiting, move || {
        assert!(back.unfinished().is_empty(), "nothing kept");
        back.bring_back(&host_root());
    });
    let report = run(&engine, &download(1), &sink);
    assert_eq!(report.done.len(), 1);
    assert!(sink.notices.lock().unwrap().is_empty());
    assert_eq!(fake.contents(&p("/b/big.bin")).unwrap(), data(1));
}
