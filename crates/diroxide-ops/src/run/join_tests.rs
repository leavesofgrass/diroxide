//! The join plan and runner on the fake provider: a verified join, a gap,
//! a wrong size, a mismatch, no sidecar, a part changed under the join, a
//! cancel, a name taken, the two-set question, and undo.

use diroxide_core::job::PartSize;
use diroxide_core::{ConflictAction, ConflictAnswer, JobId, JobKind, JobRequest, Place};

use crate::engine::{Engine, JobControl};
use crate::error::OpsError;
use crate::fake::FakeProvider;
use crate::join::Verdict;
use crate::plan::{Preflight, Step};
use crate::testing::{Script, engine, p};

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 13 % 253) as u8).collect()
}

/// A fake with `report.pdf` of 223 bytes split by the engine itself into
/// twelve parts of 20 bytes in `/b`, with its sidecar, and an empty `/c`.
fn split_set() -> (FakeProvider, Engine) {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/report.pdf"), &data(223));
    fake.add_folder(&p("/b"));
    fake.add_folder(&p("/c"));
    let engine = engine(&fake);
    let split = JobRequest::new(JobId::new(1), JobKind::Split, vec![p("/a/report.pdf")])
        .to(p("/b"))
        .split_into(PartSize::Bytes(20));
    let plan = engine.plan(&split).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    (fake, engine)
}

fn join_from(part: &str) -> JobRequest {
    JobRequest::new(JobId::new(2), JobKind::Join, vec![p(part)]).to(p("/c"))
}

#[test]
fn a_verified_join_of_twelve_parts_from_any_part() {
    let (fake, engine) = split_set();
    let plan = engine.plan(&join_from("/b/report.pdf.005")).unwrap();
    assert_eq!(plan.summary(), "Join 12 parts into report.pdf, 223 bytes");
    let facts = plan.facts.join.clone().unwrap();
    let rows = facts.rows();
    assert_eq!(rows[0], "Join 12 parts into report.pdf, 223 bytes");
    assert_eq!(rows[1], "Checked against report.pdf.crc");
    assert_eq!(rows[2], "Part 001, 20 bytes");
    assert_eq!(rows.last().unwrap(), "Part 012, 3 bytes");
    assert_eq!(plan.steps.len(), 12);
    assert_eq!(
        plan.steps[0],
        Step::JoinPart {
            from: p("/b/report.pdf.001"),
            to: p("/c/report.pdf")
        }
    );
    let sink = Script::default();
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fake.contents(&p("/c/report.pdf")).unwrap(), data(223));
    let joined = report.joined.clone().unwrap();
    assert_eq!(joined.verdict, Verdict::Matches);
    assert_eq!(joined.sentence(), "Joined report.pdf: checksum matches.");
    assert_eq!(
        joined.trash_offer().unwrap().text,
        "Move the 12 parts to the trash? y or n"
    );
    assert_eq!(joined.parts.len(), 12);
    assert_eq!(report.undo.created, vec![p("/c/report.pdf")]);
    assert!(fake.exists(&p("/b/report.pdf.001")), "the parts stay");
    assert!(fake.unfinished().is_empty());
    let reconciled = report.reconciliation.clone().unwrap();
    assert_eq!(reconciled.summary(), "Checked 1 of 1: all present");
    let said = sink.said.lock().unwrap();
    assert!(
        said.iter()
            .any(|line| line.starts_with("Joining 1 of 12 parts")),
        "{said:?}"
    );
    // Undo trashes the whole, and leaves the parts.
    let undo = engine.plan_undo(&report.undo, JobId::new(3)).unwrap();
    let undone = engine.run(&undo, &JobControl::new(), &Script::default());
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert!(!fake.exists(&p("/c/report.pdf")));
    assert!(fake.in_trash(&p("/c/report.pdf")));
    assert!(fake.exists(&p("/b/report.pdf.012")));
}

#[test]
fn a_gap_is_named_before_anything_is_written() {
    let (fake, engine) = split_set();
    fake.vanish(&p("/b/report.pdf.004"));
    assert_eq!(
        engine.plan(&join_from("/b/report.pdf.001")),
        Err(OpsError::CannotJoin(
            "Part 004 is missing. Nothing was joined.".into()
        ))
    );
    assert!(!fake.exists(&p("/c/report.pdf")));
}

#[test]
fn a_wrong_size_asks_without_a_sidecar_and_stops_with_one() {
    let (fake, engine) = split_set();
    fake.add_file(&p("/b/report.pdf.007"), &data(18));
    let refused = engine.plan(&join_from("/b/report.pdf.001")).unwrap_err();
    assert_eq!(
        refused,
        OpsError::CannotJoin("Part 007 is 18 bytes, expected 20 bytes.".into()),
        "with a sidecar the sum decides"
    );
    fake.vanish(&p("/b/report.pdf.crc"));
    let plan = engine.plan(&join_from("/b/report.pdf.001")).unwrap();
    let Some(Preflight::OddPart { place, line }) = plan.facts.questions.first() else {
        panic!("{:?}", plan.facts.questions);
    };
    assert_eq!(place, &p("/b/report.pdf.007"));
    assert_eq!(line, "Part 007 is 18 bytes, expected 20 bytes.");
    assert_eq!(plan.facts.questions[0].lines()[1], "Join anyway? y or n");
    // A last part longer than the first stops the join.
    fake.add_file(&p("/b/report.pdf.007"), &data(20));
    fake.add_file(&p("/b/report.pdf.012"), &data(30));
    assert!(matches!(
        engine.plan(&join_from("/b/report.pdf.001")),
        Err(OpsError::CannotJoin(line)) if line.starts_with("Part 012 is 30 bytes, longer")
    ));
}

#[test]
fn a_damaged_part_gives_the_mismatch_verdict() {
    let (fake, engine) = split_set();
    let mut damaged = fake.contents(&p("/b/report.pdf.003")).unwrap();
    damaged[0] ^= 0xFF;
    fake.add_file(&p("/b/report.pdf.003"), &damaged);
    let plan = engine.plan(&join_from("/b/report.pdf.001")).unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    let joined = report.joined.clone().unwrap();
    assert_eq!(joined.verdict, Verdict::Differs);
    assert_eq!(
        joined.sentence(),
        "Checksum differs: report.pdf may be damaged."
    );
    assert!(joined.trash_offer().is_none(), "the parts stay unoffered");
    assert!(
        report
            .warnings
            .iter()
            .any(|(_, line)| line == "Checksum differs: report.pdf may be damaged.")
    );
    assert!(report.summary().starts_with("Join finished with 1 warning"));
}

#[test]
fn without_a_sidecar_the_join_says_it_was_not_checked() {
    let (fake, engine) = split_set();
    fake.vanish(&p("/b/report.pdf.crc"));
    let plan = engine.plan(&join_from("/b/report.pdf.012")).unwrap();
    assert_eq!(
        plan.facts.join.clone().unwrap().rows()[1],
        "No checksum file: the join is not checked."
    );
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    let joined = report.joined.unwrap();
    assert_eq!(joined.verdict, Verdict::NoSidecar);
    assert_eq!(
        joined.sentence(),
        "Joined report.pdf. No checksum file found."
    );
    assert_eq!(fake.contents(&p("/c/report.pdf")).unwrap(), data(223));
}

#[test]
fn a_part_changed_after_the_plan_stops_the_join_and_leaves_nothing() {
    let (fake, engine) = split_set();
    let plan = engine.plan(&join_from("/b/report.pdf.001")).unwrap();
    // Rewritten by another program after the plan, longer.
    fake.add_file(&p("/b/report.pdf.003"), &data(25));
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(
        report.failed.first().map(|(_, why)| why.as_str()),
        Some("Part 003 changed while it was joined")
    );
    assert!(!fake.exists(&p("/c/report.pdf")));
    assert!(fake.unfinished().is_empty());
    assert!(report.joined.is_none());
    assert!(
        report
            .skipped
            .iter()
            .all(|(_, why)| why.starts_with("the join stopped: Part 003 changed"))
    );
}

#[test]
fn a_cancelled_join_leaves_nothing() {
    let (fake, engine) = split_set();
    let plan = engine.plan(&join_from("/b/report.pdf.001")).unwrap();
    let control = JobControl::new();
    let cancel = control.cancel.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.bytes_done >= 100 {
            cancel.cancel();
        }
    });
    let report = engine.run(&plan, &control, &sink);
    assert!(report.cancelled);
    assert!(!fake.exists(&p("/c/report.pdf")));
    assert!(fake.unfinished().is_empty());
    assert!(report.undo.created.is_empty());
}

#[test]
fn a_name_taken_is_the_conflict_question_and_keep_both_names_it() {
    let (fake, engine) = split_set();
    fake.add_file(&p("/c/report.pdf"), b"old");
    let plan = engine.plan(&join_from("/b/report.pdf.001")).unwrap();
    let sink = Script::conflicts(vec![ConflictAnswer::once(ConflictAction::KeepBoth)]);
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert_eq!(sink.asked.lock().unwrap().len(), 1);
    assert_eq!(fake.contents(&p("/c/report.pdf")).unwrap(), b"old");
    assert_eq!(fake.contents(&p("/c/report (2).pdf")).unwrap(), data(223));
    assert_eq!(
        report.joined.unwrap().sentence(),
        "Joined report (2).pdf: checksum matches."
    );
}

#[test]
fn spanned_zip_sets_are_refused_in_words() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/b/a.z01"), b"x");
    fake.add_file(&p("/b/a.zip"), b"y");
    fake.add_folder(&p("/c"));
    assert_eq!(
        engine(&fake).plan(&join_from("/b/a.z01")),
        Err(OpsError::CannotJoin(
            "Spanned zip sets need the 7z tool. Nothing was joined.".into()
        ))
    );
}

#[test]
fn two_sets_ask_and_yes_plans_the_other() {
    let fake = FakeProvider::new();
    for n in 0..=2 {
        fake.add_file(&p(&format!("/b/k.bin.{n:03}")), &[n as u8; 4]);
    }
    fake.add_folder(&p("/c"));
    let engine = engine(&fake);
    let plan = engine.plan(&join_from("/b/k.bin.001")).unwrap();
    let Some(Preflight::WhichParts { question, yes }) = plan.facts.questions.first() else {
        panic!("{:?}", plan.facts.questions);
    };
    assert_eq!(question, "Part 000 is there too. Join from 000? y or n");
    assert_eq!(plan.steps.len(), 2, "No joins from 001");
    let again = JobRequest::new(JobId::new(4), JobKind::Join, yes.clone()).to(p("/c"));
    let from_zero = engine.plan(&again).unwrap();
    assert_eq!(from_zero.steps.len(), 3);
    let report = engine.run(&from_zero, &JobControl::new(), &Script::default());
    assert_eq!(
        fake.contents(&p("/c/k.bin")).unwrap(),
        [[0u8; 4], [1u8; 4], [2u8; 4]].concat()
    );
    assert_eq!(report.joined.unwrap().verdict, Verdict::NoSidecar);
    // A sidecar's size settles it without asking.
    fake.add_file(&p("/b/k.bin.crc"), b"filename=k.bin\r\nsize=12\r\n");
    let settled = engine.plan(&join_from("/b/k.bin.001")).unwrap();
    assert!(settled.facts.questions.is_empty());
    assert_eq!(settled.steps.len(), 3);
}

#[test]
fn parts_named_without_the_extension_take_the_sidecars_name() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/b/report.001"), b"abcd");
    fake.add_file(&p("/b/report.002"), b"ef");
    let crc = crc32fast::hash(b"abcdef");
    fake.add_file(
        &p("/b/report.crc"),
        format!("filename=report.pdf size=6 crc32={crc:08x}").as_bytes(),
    );
    fake.add_folder(&p("/c"));
    let engine = engine(&fake);
    let plan = engine.plan(&join_from("/b/report.002")).unwrap();
    assert_eq!(plan.summary(), "Join 2 parts into report.pdf, 6 bytes");
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(fake.contents(&p("/c/report.pdf")).unwrap(), b"abcdef");
    assert_eq!(report.joined.unwrap().verdict, Verdict::Matches);
    let _: Option<Place> = None;
}
