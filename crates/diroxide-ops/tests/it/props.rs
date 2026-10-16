//! The property tests of research/file-operations.md and R23 on the fake
//! provider:
//!
//! - over every path through Skip, Replace, Keep both, Rename, Retry,
//!   Skip all like this and Cancel, with failures injected at random calls,
//!   nothing a job copies, moves, renames or trashes is ever lost, no file
//!   is left half written, and every step is accounted for;
//! - a cancel at any moment leaves the tree consistent;
//! - undo returns the tree to its state before the job;
//! - the operations that ran equal the plan's list;
//! - the boundary rule holds across a link out of the root.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use diroxide_core::{
    ConflictAction, ConflictAnswer, EntryKind, ErrorAction, JobId, JobKind, JobRequest, Place,
};
use diroxide_ops::{
    CallFault, Engine, FakeCall, FakePacer, FakeProvider, FakeSwitches, JobControl, JobReport,
    UndoLog,
};
use diroxide_vfs::VfsError;
use proptest::prelude::*;

use crate::support::{Answers, engine, p};

/// A small tree: files at paths of one to three names, each with its own
/// content, so a content can be followed wherever it goes.
#[derive(Clone, Debug)]
struct Tree {
    files: Vec<(String, Vec<u8>)>,
}

fn name() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["a", "b", "c", "d"]).prop_map(str::to_owned)
}

fn tree(prefix: &'static str) -> impl Strategy<Value = Tree> {
    prop::collection::vec(prop::collection::vec(name(), 1..=3), 1..=7).prop_map(move |paths| {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut folders: BTreeSet<String> = BTreeSet::new();
        for (index, parts) in paths.into_iter().enumerate() {
            let path = parts.join("/");
            let prefixes: Vec<String> = (1..parts.len()).map(|n| parts[..n].join("/")).collect();
            let clash = files
                .iter()
                .any(|(f, _)| *f == path || prefixes.contains(f))
                || folders.contains(&path);
            if clash {
                continue;
            }
            folders.extend(prefixes);
            files.push((path, format!("{prefix}{index}").into_bytes()));
        }
        Tree { files }
    })
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Copy,
    MoveSameDrive,
    MoveAcross,
    Trash,
    Rename,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Copy),
        Just(Op::MoveSameDrive),
        Just(Op::MoveAcross),
        Just(Op::Trash),
        Just(Op::Rename),
    ]
}

fn conflict_answer() -> impl Strategy<Value = ConflictAnswer> {
    (
        prop_oneof![
            Just(ConflictAction::Skip),
            Just(ConflictAction::Replace),
            Just(ConflictAction::KeepBoth),
            Just(ConflictAction::ReplaceIfNewer),
            Just(ConflictAction::Rename("renamed".into())),
            Just(ConflictAction::Cancel),
        ],
        any::<bool>(),
    )
        .prop_map(|(action, all)| ConflictAnswer {
            action,
            apply_to_rest: all,
        })
}

fn error_answer() -> impl Strategy<Value = ErrorAction> {
    prop_oneof![
        3 => Just(ErrorAction::Skip),
        1 => Just(ErrorAction::SkipAllLikeThis),
        2 => Just(ErrorAction::Retry),
        1 => Just(ErrorAction::Cancel),
    ]
}

fn fault() -> impl Strategy<Value = (FakeCall, usize, VfsError, Option<u32>)> {
    (
        prop::sample::select(vec![
            FakeCall::Read,
            FakeCall::Write,
            FakeCall::Commit,
            FakeCall::Remove,
            FakeCall::Rename,
            FakeCall::Trash,
            FakeCall::SetAttributes,
            FakeCall::Stat,
        ]),
        0usize..8,
        prop::sample::select(vec![
            VfsError::InUse,
            VfsError::NetworkLost,
            VfsError::PermissionDenied,
            VfsError::StorageFull,
        ]),
        prop::option::of(1u32..3),
    )
}

/// The fake: the tree under `/src`, a few files already in `/dst` (so
/// names clash), and a second drive at `/usb`.
fn setup(source: &Tree, there: &Tree) -> FakeProvider {
    let fake = FakeProvider::with_switches(FakeSwitches {
        second_volume: Some(p("/usb")),
        ..FakeSwitches::default()
    });
    for (path, data) in &source.files {
        fake.add_file(&p(&format!("/src/{path}")), data);
    }
    for (path, data) in &there.files {
        fake.add_file(&p(&format!("/dst/src/{path}")), data);
    }
    fake.add_folder(&p("/dst"));
    fake.add_folder(&p("/usb"));
    fake
}

fn quick_engine(fake: &FakeProvider) -> Engine {
    engine(fake).with_pacer(Arc::new(FakePacer::new()))
}

fn request(op: Op, source: &Tree) -> JobRequest {
    let id = JobId::new(1);
    match op {
        Op::Copy => JobRequest::new(id, JobKind::Copy, vec![p("/src")]).to(p("/dst")),
        Op::MoveSameDrive => JobRequest::new(id, JobKind::Move, vec![p("/src")]).to(p("/dst")),
        Op::MoveAcross => JobRequest::new(id, JobKind::Move, vec![p("/src")]).to(p("/usb")),
        Op::Trash => JobRequest::new(id, JobKind::Trash, vec![p("/src")]),
        Op::Rename => {
            // Every top-level name of the tree, rotated: a cycle.
            let mut tops: Vec<String> = source
                .files
                .iter()
                .map(|(path, _)| path.split('/').next().unwrap_or_default().to_owned())
                .collect();
            tops.sort();
            tops.dedup();
            let mut rotated = tops.clone();
            rotated.rotate_left(1);
            let renames = tops
                .iter()
                .zip(rotated.iter())
                .map(|(from, to)| (p(&format!("/src/{from}")), format!("{to}-new")))
                .collect();
            JobRequest::new(id, JobKind::Rename, Vec::new()).renaming(renames)
        }
    }
}

/// The contents of the files in the tree and in the trash.
fn contents(fake: &FakeProvider) -> (BTreeSet<Vec<u8>>, BTreeSet<Vec<u8>>) {
    let tree: BTreeSet<Vec<u8>> = fake
        .snapshot()
        .into_values()
        .filter(|(kind, _)| *kind == EntryKind::File)
        .map(|(_, data)| data)
        .collect();
    let trash: BTreeSet<Vec<u8>> = fake.trash_contents().into_iter().collect();
    (tree, trash)
}

/// What every run must leave: no unfinished write, no half-written file,
/// nothing lost, every step accounted for.
fn check_consistent(
    fake: &FakeProvider,
    before: &BTreeSet<Vec<u8>>,
    plan_subjects: &[Place],
    report: &JobReport,
) -> Result<(), TestCaseError> {
    prop_assert!(
        fake.unfinished().is_empty(),
        "unfinished: {:?}",
        fake.unfinished()
    );
    let (tree, trash) = contents(fake);
    for data in &tree {
        prop_assert!(before.contains(data), "a half-written file: {:?}", data);
    }
    for data in before {
        prop_assert!(
            tree.contains(data) || trash.contains(data),
            "lost: {}",
            String::from_utf8_lossy(data)
        );
    }
    if !report.cancelled {
        let told: BTreeSet<String> = report
            .done
            .iter()
            .chain(report.skipped.iter().map(|(place, _)| place))
            .chain(report.failed.iter().map(|(place, _)| place))
            .map(Place::to_string)
            .collect();
        for subject in plan_subjects {
            prop_assert!(told.contains(&subject.to_string()), "not told: {subject}");
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Over every answer and every injected failure, nothing is lost,
    /// nothing is half written, and every step is told.
    #[test]
    fn no_path_through_the_answers_loses_anything(
        source in tree("s"),
        there in tree("t"),
        op in op(),
        conflicts in prop::collection::vec(conflict_answer(), 0..6),
        errors in prop::collection::vec(error_answer(), 0..6),
        faults in prop::collection::vec(fault(), 0..4),
    ) {
        let fake = setup(&source, &there);
        let (tree_before, _) = contents(&fake);
        let engine = quick_engine(&fake);
        let Ok(plan) = engine.plan(&request(op, &source)) else {
            return Ok(());
        };
        let places: Vec<Place> = source
            .files
            .iter()
            .map(|(path, _)| p(&format!("/src/{path}")))
            .chain(there.files.iter().map(|(path, _)| p(&format!("/dst/src/{path}"))))
            .collect();
        for (call, index, error, times) in faults {
            if let Some(place) = places.get(index % places.len().max(1)) {
                let mut fault = CallFault::new(call, place.clone(), error);
                fault.times = times;
                fake.switches_mut().call_faults.push(fault);
            }
        }
        let subjects: Vec<Place> = plan.steps.iter().map(|s| s.subject().clone()).collect();
        let sink = Answers::new(conflicts, errors);
        let report = engine.run(&plan, &JobControl::new(), &sink);
        check_consistent(&fake, &tree_before, &subjects, &report)?;
    }

    /// A cancel at any moment leaves a consistent tree.
    #[test]
    fn a_cancel_at_any_moment_leaves_the_tree_consistent(
        source in tree("s"),
        op in op(),
        at in 0usize..40,
    ) {
        let fake = setup(&source, &Tree { files: Vec::new() });
        let (tree_before, _) = contents(&fake);
        let engine = quick_engine(&fake);
        let Ok(plan) = engine.plan(&request(op, &source)) else {
            return Ok(());
        };
        let control = JobControl::new();
        let canceller = control.clone();
        let mut seen = 0;
        let sink = Answers::default().on_progress(move |_| {
            seen += 1;
            if seen == at {
                canceller.cancel.cancel();
            }
        });
        let report = engine.run(&plan, &control, &sink);
        check_consistent(&fake, &tree_before, &[], &report)?;
    }

    /// Undo returns the tree to what it was before the job.
    #[test]
    fn undo_returns_the_tree_to_its_state_before_the_job(
        source in tree("s"),
        op in op(),
    ) {
        let fake = setup(&source, &Tree { files: Vec::new() });
        let before = fake.snapshot();
        let engine = quick_engine(&fake);
        let Ok(plan) = engine.plan(&request(op, &source)) else {
            return Ok(());
        };
        let report = engine.run(&plan, &JobControl::new(), &Answers::default());
        prop_assert!(report.failed.is_empty(), "{report:?}");
        let mut log = UndoLog::default();
        log.record(report.undo);
        let undo = engine
            .undo_last(&mut log, JobId::new(2), &JobControl::new(), &Answers::default())
            .unwrap();
        prop_assert!(undo.failed.is_empty() && undo.skipped.is_empty(), "{undo:?}");
        prop_assert!(
            undo.summary().starts_with("Undo finished"),
            "{}: {undo:?}",
            undo.summary()
        );
        let after = fake.snapshot();
        prop_assert_eq!(after, before);
    }

    /// With nothing in the way, the operations that ran are the plan's
    /// steps, in order.
    #[test]
    fn the_operations_that_ran_equal_the_plans_list(
        source in tree("s"),
        kind in prop::sample::select(vec![JobKind::Copy, JobKind::Trash, JobKind::DeletePermanently]),
    ) {
        let fake = setup(&source, &Tree { files: Vec::new() });
        let engine = quick_engine(&fake);
        let mut request = JobRequest::new(JobId::new(3), kind, vec![p("/src")]);
        if kind == JobKind::Copy {
            request = request.to(p("/dst"));
        }
        let plan = engine.plan(&request).unwrap();
        let report = engine.run(&plan, &JobControl::new(), &Answers::default());
        let subjects: Vec<Place> = plan.steps.iter().map(|s| s.subject().clone()).collect();
        prop_assert_eq!(&report.done, &subjects);
        prop_assert!(report.skipped.is_empty() && report.failed.is_empty());
        if kind == JobKind::Copy {
            let check = report.reconciliation.unwrap();
            prop_assert!(check.is_whole());
            prop_assert_eq!(check.present.len(), plan.steps.len());
        }
    }

    /// A link out of the root is acted on as a link; what it points to is
    /// never touched.
    #[test]
    fn the_boundary_holds_across_a_link_out_of_the_root(
        source in tree("s"),
        outside in tree("o"),
        kind in prop::sample::select(vec![
            JobKind::Copy, JobKind::Move, JobKind::Trash, JobKind::DeletePermanently,
        ]),
        across in any::<bool>(),
    ) {
        let fake = setup(&source, &Tree { files: Vec::new() });
        for (path, data) in &outside.files {
            fake.add_file(&p(&format!("/outside/{path}")), data);
        }
        fake.add_link(&p("/src/zlink"), &p("/outside").to_string(), true);
        let before: BTreeMap<String, (EntryKind, Vec<u8>)> = fake
            .snapshot()
            .into_iter()
            .filter(|(place, _)| place.contains("outside"))
            .collect();
        let engine = quick_engine(&fake);
        let target = if across { p("/usb") } else { p("/dst") };
        let mut request = JobRequest::new(JobId::new(4), kind, vec![p("/src")]);
        if matches!(kind, JobKind::Copy | JobKind::Move) {
            request = request.to(target);
        }
        let plan = engine.plan(&request).unwrap();
        engine.run(&plan, &JobControl::new(), &Answers::default());
        let after: BTreeMap<String, (EntryKind, Vec<u8>)> = fake
            .snapshot()
            .into_iter()
            .filter(|(place, _)| place.contains("outside") && !place.contains("zlink"))
            .collect();
        prop_assert_eq!(after, before);
    }
}
