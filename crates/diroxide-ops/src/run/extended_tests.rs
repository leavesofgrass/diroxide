//! Extended items in the engine, on the fake provider: the internet
//! mark removed as one step with its bytes kept in the undo record, Undo
//! writing them back, and a record the app builds itself undone the same
//! way (lane MA's request 5).

use diroxide_core::{JobId, JobKind};

use crate::engine::JobControl;
use crate::fake::FakeProvider;
use crate::plan::{Plan, Step};
use crate::testing::{Script, engine, p};
use crate::undo::{ExtendedBefore, UndoLog, UndoRecord};

const MARK: &str = "Zone.Identifier";
const BYTES: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

fn marked() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_file(&p("/dl/setup.exe"), b"MZ");
    fake.set_extended_now(&p("/dl/setup.exe"), MARK, BYTES);
    fake
}

#[test]
fn removing_the_mark_keeps_its_bytes_and_undo_writes_them_back() {
    let fake = marked();
    let engine = engine(&fake);
    let plan = Plan::extended(
        JobId::new(40),
        JobKind::EditMetadata,
        [(p("/dl/setup.exe"), MARK.to_owned(), None)],
    );
    assert_eq!(
        plan.steps[0].describe(),
        format!("remove Zone.Identifier from {}", p("/dl/setup.exe"))
    );
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fake.extended_now(&p("/dl/setup.exe"), MARK), None);
    assert_eq!(
        report.undo.extended,
        [ExtendedBefore {
            place: p("/dl/setup.exe"),
            name: MARK.to_owned(),
            before: Some(BYTES.to_vec()),
        }]
    );
    assert!(!report.undo.is_empty());

    let mut log = UndoLog::default();
    log.record(report.undo);
    let undone = engine
        .undo_last(
            &mut log,
            JobId::new(41),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(
        fake.extended_now(&p("/dl/setup.exe"), MARK).as_deref(),
        Some(BYTES)
    );
    assert!(log.is_empty());
}

#[test]
fn a_record_the_app_builds_is_undone_through_the_engine() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/dl/setup.exe"), b"MZ");
    // The app removed the mark itself (the metadata crate's call) and
    // records what it took, with no state of its own.
    let mut record = UndoRecord::for_job(JobId::new(42), JobKind::EditMetadata);
    record.extended.push(ExtendedBefore {
        place: p("/dl/setup.exe"),
        name: MARK.to_owned(),
        before: Some(BYTES.to_vec()),
    });
    let plan = engine(&fake).plan_undo(&record, JobId::new(43)).unwrap();
    assert!(matches!(
        plan.steps.as_slice(),
        [Step::SetExtended { value: Some(_), .. }]
    ));
    let report = engine(&fake).run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(
        fake.extended_now(&p("/dl/setup.exe"), MARK).as_deref(),
        Some(BYTES)
    );
    // The undo's own record takes it away again: redo is one more undo.
    assert_eq!(report.undo.extended[0].before, None);
}

#[test]
fn nothing_to_remove_is_done_with_nothing_to_undo() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/dl/notes.txt"), b"x");
    let plan = Plan::extended(
        JobId::new(44),
        JobKind::EditMetadata,
        [(p("/dl/notes.txt"), MARK.to_owned(), None)],
    );
    let report = engine(&fake).run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.undo.extended.is_empty());
    assert!(report.undo.is_empty());
}

#[test]
fn an_undo_record_with_extended_items_survives_the_journal_form() {
    let mut record = UndoRecord::for_job(JobId::new(45), JobKind::EditMetadata);
    record.extended.push(ExtendedBefore {
        place: p("/dl/setup.exe"),
        name: MARK.to_owned(),
        before: Some(BYTES.to_vec()),
    });
    let text = serde_json::to_string(&record).unwrap();
    let back: UndoRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(back, record);
    // A record without extended items writes none, so older journals
    // and newer ones read each other.
    let plain = UndoRecord::for_job(JobId::new(46), JobKind::EditMetadata);
    let text = serde_json::to_string(&plain).unwrap();
    assert!(!text.contains("extended"), "{text}");
    let back: UndoRecord = serde_json::from_str(&text).unwrap();
    assert!(back.extended.is_empty());
}
