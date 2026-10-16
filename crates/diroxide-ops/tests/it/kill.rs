//! The kill test (research/file-management-lessons.md, "Interrupted
//! processes"): a job runs in a child process, the child is killed mid-way,
//! and the journal it left says so to the next start.

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::{JobId, JobKind, JobRequest};
use diroxide_ops::{FakeProvider, JobControl, Journal, Latency};

use crate::support::{Answers, engine, p};

const STATE: &str = "DIROXIDE_KILL_TEST_STATE";

/// The child: a slow copy of many files, journaled into the folder the
/// parent names. Run only by the parent test.
#[test]
#[ignore = "the child of the kill test; the parent runs it"]
fn kill_child() {
    let Some(folder) = std::env::var_os(STATE) else {
        return;
    };
    let fake = FakeProvider::new();
    for n in 0..FILES {
        fake.add_file(&p(&format!("/a/{n}.txt")), b"slow");
    }
    fake.add_folder(&p("/b"));
    let engine = engine(&fake).with_journal(Journal::new(folder));
    let request = JobRequest::new(
        JobId::new(42),
        JobKind::Copy,
        (0..FILES).map(|n| p(&format!("/a/{n}.txt"))).collect(),
    )
    .to(p("/b"));
    let plan = engine.plan(&request).unwrap();
    // Slow from here on, so the parent can kill the run mid-way.
    fake.switches_mut().latency = Latency {
        per_call: Duration::from_millis(15),
        ..Latency::default()
    };
    engine.run(&plan, &JobControl::new(), &Answers::default());
}

/// The files the child copies.
const FILES: u64 = 80;

#[test]
fn a_killed_job_leaves_a_journal_the_next_start_reports() {
    let state = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let journal = Journal::new(state.path());
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "kill::kill_child",
            "--exact",
            "--ignored",
            "--test-threads=1",
        ])
        .env(STATE, state.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let mut seen = 0;
    while start.elapsed() < Duration::from_secs(60) {
        if let Some(job) = journal.interrupted().first() {
            seen = job.finished;
            if seen >= 3 {
                break;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(seen >= 3, "the child got going: {seen} steps");
    let interrupted = journal.interrupted();
    assert_eq!(interrupted.len(), 1, "the next start sees the job");
    let job = &interrupted[0];
    assert_eq!(job.job, JobId::new(42));
    assert!(job.finished >= 3 && job.finished < FILES);
    assert!(
        job.sentence()
            .starts_with("Last session ended during a copy: ")
    );
    assert_eq!(job.record.created.len() as u64, job.done);
}
