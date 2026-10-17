//! The undo journal in an injected state folder: written before the first
//! step, a line per step, an end line; read back after a restart.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use diroxide_core::{JobId, JobKind, JobOrigin, JobRequest};
use diroxide_ops::journal::KEEP_FINISHED;
use diroxide_ops::{FakeProvider, JobControl, Journal, UndoLog};

use crate::support::{Answers, engine, p};

fn state() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

fn tree() -> FakeProvider {
    let fake = FakeProvider::new();
    for n in 0..5 {
        fake.add_file(&p(&format!("/a/{n}.txt")), format!("file {n}").as_bytes());
    }
    fake.add_folder(&p("/b"));
    fake
}

fn copy_request(job: u64) -> JobRequest {
    JobRequest::new(
        JobId::new(job),
        JobKind::Copy,
        (0..5).map(|n| p(&format!("/a/{n}.txt"))).collect(),
    )
    .to(p("/b"))
}

#[test]
fn the_journal_is_written_before_the_first_step_and_read_back_after() {
    let folder = state();
    let journal = Journal::new(folder.path());
    let fake = tree();
    let engine = engine(&fake).with_journal(journal.clone());
    let plan = engine.plan(&copy_request(1)).unwrap();
    let seen = Arc::new(Mutex::new(None));
    let looker = Arc::clone(&seen);
    let watched = journal.clone();
    let sink = Answers::default().on_progress(move |_| {
        let mut seen = looker.lock().unwrap();
        if seen.is_none() {
            *seen = Some(watched.interrupted());
        }
    });
    let report = engine.run(&plan, &JobControl::new(), &sink);
    let first = seen.lock().unwrap().clone().unwrap();
    assert_eq!(first.len(), 1, "the header is there before any step");
    assert_eq!(first[0].finished, 0);
    assert!(journal.interrupted().is_empty(), "the end line is written");
    let records = journal.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].created, report.undo.created);
    assert_eq!(records[0].left, report.undo.left);
    assert_eq!(records[0].kind, Some(JobKind::Copy));
    let mut log = UndoLog::default();
    for record in records {
        log.record(record);
    }
    let after_restart = crate::support::engine(&fake);
    let undo = after_restart
        .undo_last(
            &mut log,
            JobId::new(2),
            &JobControl::new(),
            &Answers::default(),
        )
        .unwrap();
    assert!(undo.failed.is_empty(), "{undo:?}");
    assert!(
        !fake.exists(&p("/b/0.txt")),
        "undone from the journal's record"
    );
}

/// An undo of a trash restores and records nothing, yet its journal must
/// still come back after a restart, so the app can take away the record
/// it reversed (W1d-1's contract request 3).
#[test]
fn a_finished_job_with_nothing_to_undo_is_still_returned() {
    let folder = state();
    let journal = Journal::new(folder.path());
    let fake = tree();
    let engine = engine(&fake).with_journal(journal.clone());
    let trash = JobRequest::new(JobId::new(1), JobKind::Trash, vec![p("/a/0.txt")]);
    let plan = engine.plan(&trash).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Answers::default());
    assert!(report.failed.is_empty(), "{report:?}");
    let mut log = UndoLog::default();
    log.record(report.undo);
    let undo = engine
        .undo_last(
            &mut log,
            JobId::new(2),
            &JobControl::new(),
            &Answers::default(),
        )
        .unwrap();
    assert!(undo.failed.is_empty(), "{undo:?}");
    assert!(fake.exists(&p("/a/0.txt")), "restored");
    let records = journal.records();
    assert_eq!(records.len(), 2, "the undo's empty record is returned too");
    assert_eq!(records[0].job, Some(JobId::new(1)));
    assert!(!records[0].is_empty());
    assert_eq!(records[1].job, Some(JobId::new(2)));
    assert_eq!(records[1].origin, JobOrigin::Undo);
    assert!(records[1].is_empty(), "{:?}", records[1]);
}

#[test]
fn a_job_that_dies_mid_way_is_reported_and_can_be_settled() {
    let folder = state();
    let journal = Journal::new(folder.path());
    let fake = tree();
    let engine = engine(&fake).with_journal(journal.clone());
    let plan = engine.plan(&copy_request(7)).unwrap();
    let sink = Answers::default().on_progress(|progress| {
        if progress.items_done == 3 {
            panic!("the process dies here");
        }
    });
    let died = catch_unwind(AssertUnwindSafe(|| {
        engine.run(&plan, &JobControl::new(), &sink);
    }));
    assert!(died.is_err());
    let interrupted = journal.interrupted();
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].job, JobId::new(7));
    assert_eq!(interrupted[0].done, 3);
    assert_eq!(
        interrupted[0].sentence(),
        "Last session ended during a copy: 3 of 5 steps done."
    );
    assert_eq!(interrupted[0].record.created.len(), 3);
    journal.settle(JobId::new(7)).unwrap();
    assert!(journal.interrupted().is_empty());
}

#[test]
fn the_oldest_finished_journals_are_pruned() {
    let folder = state();
    let journal = Journal::new(folder.path());
    let fake = tree();
    let engine = engine(&fake).with_journal(journal.clone());
    for job in 0..(KEEP_FINISHED as u64 + 3) {
        let request = JobRequest::new(JobId::new(100 + job), JobKind::MakeFolder, Vec::new())
            .to(p("/b"))
            .named(format!("made{job}"));
        let plan = engine.plan(&request).unwrap();
        engine.run(&plan, &JobControl::new(), &Answers::default());
    }
    assert!(journal.records().len() <= KEEP_FINISHED + 1);
    assert!(journal.records().len() >= KEEP_FINISHED);
}

#[test]
fn a_journal_that_cannot_be_written_is_a_warning_not_a_stop() {
    let folder = state();
    let blocker = folder.path().join("journal");
    std::fs::write(&blocker, b"a file where the folder should be").unwrap();
    let fake = tree();
    let engine = engine(&fake).with_journal(Journal::new(folder.path()));
    let plan = engine.plan(&copy_request(9)).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Answers::default());
    assert_eq!(report.done.len(), 5);
    assert!(
        report.warnings[0]
            .1
            .starts_with("the undo journal could not be written")
    );
}

/// A split's journal names the unfinished file of the part being written,
/// so an interrupted split says what it left beside its target.
#[test]
fn a_split_journals_the_unfinished_name_of_each_part() {
    let folder = state();
    let journal = Journal::new(folder.path());
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/report.pdf"), &[9u8; 50]);
    fake.add_folder(&p("/b"));
    let engine = engine(&fake).with_journal(journal.clone());
    let request = JobRequest::new(JobId::new(30), JobKind::Split, vec![p("/a/report.pdf")])
        .to(p("/b"))
        .split_into(diroxide_core::job::PartSize::Bytes(20));
    let plan = engine.plan(&request).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let looker = Arc::clone(&seen);
    let watched = journal.clone();
    let sink = Answers::default().on_progress(move |progress| {
        if progress.bytes_done == 40 && looker.lock().unwrap().is_empty() {
            looker.lock().unwrap().extend(watched.interrupted());
        }
    });
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let during = seen.lock().unwrap().clone();
    assert_eq!(during.len(), 1, "the split, unfinished while it ran");
    assert_eq!(during[0].kind, JobKind::Split);
    let (target, unfinished) = &during[0].unfinished[0];
    assert_eq!(target, &p("/b/report.pdf.002"));
    assert_eq!(unfinished.parent(), Some(p("/b")));
    assert!(
        diroxide_vfs::names::is_unfinished(unfinished.name().unwrap()),
        "{unfinished}"
    );
    assert_eq!(during[0].record.created, vec![p("/b/report.pdf.001")]);
    assert!(journal.interrupted().is_empty(), "finished at the end");
    assert_eq!(
        journal.records()[0].created.last(),
        Some(&p("/b/report.pdf.crc"))
    );
}

/// A permissions change's inverses travel through the journal, so Undo
/// after a restart restores the old values.
#[test]
fn a_permissions_change_journals_its_inverses() {
    use diroxide_core::Attributes;
    use diroxide_ops::{AttributeEdit, ModeChange, Recursion};

    let folder = state();
    let journal = Journal::new(folder.path());
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x.txt"), b"x");
    fake.set_attributes_now(&p("/a/x.txt"), Attributes::Unix(0o600));
    let engine = engine(&fake).with_journal(journal.clone());
    let edit =
        AttributeEdit::new(Recursion::ThisItem).with_mode(ModeChange::Bits { on: 0o044, off: 0 });
    let request = JobRequest::new(JobId::new(40), JobKind::SetAttributes, vec![p("/a/x.txt")]);
    let plan = engine.plan_attributes(&request, &edit).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Answers::default());
    assert_eq!(report.undo.attributes.len(), 1);
    let records = journal.records();
    assert_eq!(records[0].attributes, report.undo.attributes);
    let mut log = UndoLog::default();
    log.record(records[0].clone());
    engine
        .undo_last(
            &mut log,
            JobId::new(41),
            &JobControl::new(),
            &Answers::default(),
        )
        .unwrap();
    assert_eq!(
        fake.entry(&p("/a/x.txt")).unwrap().attributes,
        Some(Attributes::Unix(0o600))
    );
}
