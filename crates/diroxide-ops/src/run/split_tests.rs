//! The split runner's tests on the fake provider: the parts and the
//! sidecar, progress by parts, cancel and fault cleanup, the source
//! changing under the split, and undo.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use diroxide_core::job::PartSize;
use diroxide_core::{ErrorAction, JobId, JobKind, JobRequest, Place, Timestamp};
use diroxide_vfs::VfsError;

use crate::engine::JobControl;
use crate::fake::{CallFault, FakeCall, FakeProvider, Fault};
use crate::sidecar::{Sidecar, SidecarRead};
use crate::testing::{Script, engine, p};

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

fn fake_with(len: usize) -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/report.pdf"), &data(len));
    fake.add_folder(&p("/b"));
    fake
}

fn request() -> JobRequest {
    JobRequest::new(JobId::new(24), JobKind::Split, vec![p("/a/report.pdf")])
        .to(p("/b"))
        .split_into(PartSize::Bytes(20))
}

fn part(n: u64) -> Place {
    p(&format!("/b/report.pdf.{n:03}"))
}

fn parts_in_b(fake: &FakeProvider) -> Vec<String> {
    let mut names: Vec<String> = fake
        .snapshot()
        .keys()
        .filter(|key| key.contains("report"))
        .filter(|key| Place::parse(key).is_ok_and(|place| place.parent() == Some(p("/b"))))
        .cloned()
        .collect();
    names.sort();
    names
}

#[test]
fn a_split_writes_every_part_then_the_sidecar() {
    let fake = fake_with(223);
    let engine = engine(&fake);
    let plan = engine.plan(&request()).unwrap();
    let sink = Script::default();
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.summary(), "Split finished: 13 done.");
    let mut joined = Vec::new();
    for n in 1..=12 {
        joined.extend(fake.contents(&part(n)).unwrap());
    }
    assert_eq!(joined, data(223), "the parts are the whole, in order");
    assert_eq!(fake.contents(&part(12)).unwrap().len(), 3);
    let sidecar = fake.contents(&p("/b/report.pdf.crc")).unwrap();
    let crc = crc32fast::hash(&data(223));
    assert_eq!(sidecar, Sidecar::new("report.pdf", 223, crc).to_bytes());
    assert_eq!(
        SidecarRead::parse(&sidecar).complete(),
        Some(Sidecar::new("report.pdf", 223, crc))
    );
    let mut made: Vec<Place> = (1..=12).map(part).collect();
    made.push(p("/b/report.pdf.crc"));
    assert_eq!(report.undo.created, made, "the sidecar last");
    assert!(fake.unfinished().is_empty());
    assert!(report.reconciliation.unwrap().is_whole());
}

#[test]
fn progress_is_said_by_parts() {
    let fake = fake_with(223);
    let engine = engine(&fake);
    let plan = engine.plan(&request()).unwrap();
    let sink = Script::default();
    engine.run(&plan, &JobControl::new(), &sink);
    let said = sink.said.lock().unwrap();
    assert!(
        said.iter()
            .any(|line| line == "Splitting 1 of 12 parts, 20 of 223 bytes"),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|line| line.starts_with("Splitting 12 of 12 parts")),
        "{said:?}"
    );
    for line in said.iter() {
        // The part count, the meaning, inside the first 40 cells.
        let meaning = line.find(" parts").map_or(line.len(), |at| at + 6);
        assert!(meaning <= 40, "{line}");
    }
}

#[test]
fn a_cancelled_split_leaves_no_part_and_no_sidecar() {
    let fake = fake_with(223);
    let engine = engine(&fake);
    let plan = engine.plan(&request()).unwrap();
    let control = JobControl::new();
    let cancel = control.cancel.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.bytes_done >= 90 {
            cancel.cancel();
        }
    });
    let report = engine.run(&plan, &control, &sink);
    assert!(report.cancelled);
    assert!(parts_in_b(&fake).is_empty(), "{:?}", parts_in_b(&fake));
    assert!(fake.unfinished().is_empty());
    assert!(report.undo.created.is_empty(), "nothing left to undo");
    assert_eq!(fake.trashed(), 0, "nothing goes to the trash");
    assert!(
        report
            .warnings
            .iter()
            .any(|(_, line)| line == "Split of report.pdf stopped: its parts were removed."),
        "{:?}",
        report.warnings
    );
    assert!(fake.exists(&p("/a/report.pdf")), "the original is kept");
}

#[test]
fn a_fault_after_part_3_removes_the_parts_or_retries() {
    for answer in [ErrorAction::Skip, ErrorAction::Cancel] {
        let fake = fake_with(223);
        fake.switches_mut().faults.push(Fault {
            place: part(4),
            at_byte: Some(5),
            error: VfsError::StorageFull,
        });
        let engine = engine(&fake);
        let plan = engine.plan(&request()).unwrap();
        let sink = Script::errors(vec![answer]);
        let report = engine.run(&plan, &JobControl::new(), &sink);
        assert_eq!(sink.failed.lock().unwrap().len(), 1, "asked once");
        assert!(
            parts_in_b(&fake).is_empty(),
            "{answer:?}: {:?}",
            parts_in_b(&fake)
        );
        assert!(fake.unfinished().is_empty(), "{answer:?}");
        assert!(!fake.exists(&p("/b/report.pdf.crc")));
        if answer == ErrorAction::Skip {
            assert_eq!(report.failed.len(), 1);
            assert!(
                report
                    .skipped
                    .iter()
                    .all(|(_, why)| why == "an earlier part could not be written"),
                "{:?}",
                report.skipped
            );
            assert_eq!(report.skipped.len(), 9, "parts 5 to 12 and the sidecar");
        } else {
            assert!(report.cancelled);
        }
    }
    // Retry after a failure that passes: every part once, the CRC right.
    let fake = fake_with(223);
    fake.switches_mut()
        .call_faults
        .push(CallFault::new(FakeCall::Write, part(4), VfsError::InUse).times(1));
    let engine = engine(&fake);
    let plan = engine.plan(&request()).unwrap();
    let report = engine.run(
        &plan,
        &JobControl::new(),
        &Script::errors(vec![ErrorAction::Retry]),
    );
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let sidecar = SidecarRead::parse(&fake.contents(&p("/b/report.pdf.crc")).unwrap());
    assert_eq!(
        sidecar.crc32,
        Some(crc32fast::hash(&data(223))),
        "counted once"
    );
}

#[test]
fn a_source_that_changes_during_the_split_fails_it_and_trashes_nothing() {
    let fake = fake_with(223);
    let engine = engine(&fake);
    let plan = engine.plan(&request()).unwrap();
    let touched = Arc::new(AtomicBool::new(false));
    let hook_fake = fake.clone();
    let flag = Arc::clone(&touched);
    let sink = Script::default().on_progress(move |progress| {
        if progress.bytes_done >= 120 && !flag.swap(true, Ordering::SeqCst) {
            hook_fake.set_modified(&p("/a/report.pdf"), Timestamp::from_unix(1_800_000_000, 0));
        }
    });
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert!(touched.load(Ordering::SeqCst));
    assert_eq!(
        report.failed.last().map(|(_, why)| why.as_str()),
        Some("report.pdf changed while it was split")
    );
    assert!(parts_in_b(&fake).is_empty(), "{:?}", parts_in_b(&fake));
    assert_eq!(fake.trashed(), 0, "nothing goes to the trash");
}

#[test]
fn undo_of_a_split_trashes_its_parts_and_sidecar() {
    let fake = fake_with(45);
    let engine = engine(&fake);
    let plan = engine.plan(&request()).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(parts_in_b(&fake).len(), 4);
    let undo = engine.plan_undo(&report.undo, JobId::new(25)).unwrap();
    let undone = engine.run(&undo, &JobControl::new(), &Script::default());
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert!(parts_in_b(&fake).is_empty());
    assert_eq!(fake.trashed(), 4, "three parts and the sidecar");
    assert!(fake.exists(&p("/a/report.pdf")));
}

#[test]
fn several_files_split_in_turn_and_a_failure_keeps_the_finished_ones() {
    let fake = fake_with(50);
    fake.add_file(&p("/a/notes.txt"), &data(30));
    fake.switches_mut().faults.push(Fault {
        place: p("/b/notes.txt.002"),
        at_byte: None,
        error: VfsError::StorageFull,
    });
    let engine = engine(&fake);
    let request = JobRequest::new(
        JobId::new(26),
        JobKind::Split,
        vec![p("/a/report.pdf"), p("/a/notes.txt")],
    )
    .to(p("/b"))
    .split_into(PartSize::Bytes(20));
    let plan = engine.plan(&request).unwrap();
    let sink = Script::errors(vec![ErrorAction::Skip]);
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert!(
        fake.exists(&p("/b/report.pdf.crc")),
        "the first file finished"
    );
    assert!(fake.exists(&p("/b/report.pdf.003")));
    assert!(!fake.exists(&p("/b/notes.txt.001")), "the second removed");
    assert_eq!(report.undo.created.len(), 4);
}
