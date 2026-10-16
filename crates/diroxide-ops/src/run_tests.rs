//! The run's tests on the fake provider: research/file-operations.md's
//! edge cases tagged `fake`, and the data-loss patterns of
//! research/file-management-lessons.md part 4.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use diroxide_core::format::within_cells;
use diroxide_core::{
    Attributes, ConflictAction, ConflictAnswer, ConflictKind, ConflictPolicy, EntryKind,
    ErrorAction, JobId, JobKind, JobRequest, JobState, Timestamp,
};
use diroxide_vfs::{Provider, VfsError, VolumeKind};

use crate::engine::{Engine, JobControl, JobReport};
use crate::fake::{CallFault, FakeCall, FakeProvider, FakeSwitches, Fault};
use crate::options::EngineOptions;
use crate::pace::{FakePacer, Pacer};
use crate::plan::{Plan, Step};
use crate::testing::{Script, engine, p};
use crate::undo::UndoLog;

const DAY: i64 = 86_400;

fn at(days: i64) -> Timestamp {
    Timestamp::from_unix(1_790_000_000 + days * DAY, 0)
}

fn request(kind: JobKind, sources: &[&str]) -> JobRequest {
    JobRequest::new(JobId::new(11), kind, sources.iter().map(|s| p(s)).collect())
}

fn run(engine: &Engine, request: &JobRequest, sink: &Script) -> JobReport {
    let plan = engine.plan(request).unwrap();
    engine.run(&plan, &JobControl::new(), sink)
}

/// A tree with times: two photos in a folder with a subfolder, and notes.
fn tree() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.switches_mut().now = Some(at(-30));
    fake.add_file(&p("/a/photos/one.jpg"), b"one-one-one");
    fake.add_file(&p("/a/photos/trip/two.jpg"), b"two");
    fake.add_file(&p("/a/notes.txt"), b"notes");
    fake.set_modified(&p("/a/photos"), at(-20));
    fake.set_modified(&p("/a/photos/one.jpg"), at(-10));
    fake.switches_mut().now = Some(at(0));
    fake.add_folder(&p("/b"));
    fake
}

// ----- copy -----

#[test]
fn a_copy_keeps_contents_and_dates_folders_after_their_contents() {
    let fake = tree();
    let created = at(-40);
    {
        let place = p("/a/photos/one.jpg");
        let change = diroxide_vfs::AttributeChange {
            created: Some(created),
            ..Default::default()
        };
        fake.set_attributes(&place, &change, &diroxide_vfs::OpContext::new())
            .unwrap();
    }
    let sink = Script::default();
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/photos", "/a/notes.txt"]).to(p("/b")),
        &sink,
    );
    assert_eq!(report.summary(), "Copy finished: 5 done.");
    assert_eq!(
        fake.contents(&p("/b/photos/one.jpg")).unwrap(),
        b"one-one-one"
    );
    assert_eq!(fake.contents(&p("/b/photos/trip/two.jpg")).unwrap(), b"two");
    let copy = fake.entry(&p("/b/photos/one.jpg")).unwrap();
    assert_eq!(copy.modified, Some(at(-10)), "the modified time kept");
    assert_eq!(copy.created, Some(created), "the created time kept");
    assert_eq!(
        fake.entry(&p("/b/photos")).unwrap().modified,
        Some(at(-20)),
        "a folder's time set after its contents"
    );
    let check = report.reconciliation.unwrap();
    assert_eq!(check.summary(), "Checked 5 of 5: all present");
    assert!(fake.unfinished().is_empty());
    assert_eq!(report.undo.created.len(), 5);
}

#[test]
fn a_created_time_the_place_cannot_keep_is_said_only_when_asked() {
    let fake = tree();
    fake.switches_mut().no_created_time = true;
    let place = p("/a/notes.txt");
    fake.set_modified(&place, at(-3));
    // The fake refuses to set a created time, so give the source one
    // through the switch's back door: a place that can, then cannot.
    fake.switches_mut().no_created_time = false;
    fake.set_attributes(
        &place,
        &diroxide_vfs::AttributeChange {
            created: Some(at(-50)),
            ..Default::default()
        },
        &diroxide_vfs::OpContext::new(),
    )
    .unwrap();
    fake.switches_mut().no_created_time = true;
    let quiet = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    assert!(quiet.warnings.is_empty(), "{:?}", quiet.warnings);
    assert_eq!(
        fake.entry(&p("/b/notes.txt")).unwrap().modified,
        Some(at(-3))
    );
    let asked = engine(&fake).with_options(EngineOptions {
        warn_created_time: true,
        ..EngineOptions::default()
    });
    let report = run(
        &asked,
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/a/photos")),
        &Script::default(),
    );
    assert_eq!(report.warnings.len(), 1);
    assert!(
        report.warnings[0]
            .1
            .starts_with("copied, but its created time")
    );
    assert!(report.summary().starts_with("Copy finished with 1 warning"));
}

#[test]
fn sizes_around_the_chunk_copy_exactly() {
    let fake = FakeProvider::new();
    fake.add_folder(&p("/b"));
    let mib = 1024 * 1024;
    let sizes = [0usize, 1, mib - 1, mib, mib + 1, 3 * mib + 7];
    let mut sources = Vec::new();
    for (n, size) in sizes.iter().enumerate() {
        let data: Vec<u8> = (0..*size).map(|i| (i * 7 + n) as u8).collect();
        let place = format!("/a/f{n}");
        fake.add_file(&p(&place), &data);
        sources.push(place);
    }
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    let sink = Script::default();
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &sources).to(p("/b")),
        &sink,
    );
    assert_eq!(report.done.len(), sizes.len());
    for (n, _) in sizes.iter().enumerate() {
        assert_eq!(
            fake.contents(&p(&format!("/b/f{n}"))),
            fake.contents(&p(&format!("/a/f{n}")))
        );
    }
    let chunks = sink
        .progress
        .lock()
        .unwrap()
        .iter()
        .filter(|progress| progress.state == JobState::Running)
        .count();
    assert!(
        chunks >= 10,
        "progress on every entry and every chunk: {chunks}"
    );
}

/// A sink that cancels the job once this many bytes are done.
fn cancel_after(control: &JobControl, bytes: u64) -> Script {
    let control = control.clone();
    Script::default().on_progress(move |progress| {
        if progress.bytes_done >= bytes {
            control.cancel.cancel();
        }
    })
}

#[test]
fn a_cancel_inside_a_large_file_leaves_nothing_half_written() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/big"), &vec![7u8; 5 * 1024 * 1024]);
    fake.add_folder(&p("/b"));
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::Copy, &["/a/big"]).to(p("/b")))
        .unwrap();
    let control = JobControl::new();
    let sink = cancel_after(&control, 2 * 1024 * 1024);
    let report = engine.run(&plan, &control, &sink);
    assert!(report.cancelled);
    assert!(!fake.exists(&p("/b/big")), "nothing under the final name");
    assert!(fake.unfinished().is_empty(), "no unfinished name");
    assert_eq!(report.summary(), "Copy cancelled: 0 done.");
}

#[test]
fn a_cancel_inside_a_folder_keeps_what_was_finished() {
    let fake = FakeProvider::new();
    for n in 0..4 {
        fake.add_file(&p(&format!("/a/f/{n}.bin")), &vec![n as u8; 1024 * 1024]);
    }
    fake.add_folder(&p("/b"));
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::Copy, &["/a/f"]).to(p("/b")))
        .unwrap();
    let control = JobControl::new();
    let sink = cancel_after(&control, 2 * 1024 * 1024 + 1);
    let report = engine.run(&plan, &control, &sink);
    assert!(report.cancelled);
    assert!(fake.exists(&p("/b/f/0.bin")));
    assert!(fake.exists(&p("/b/f/1.bin")));
    assert!(!fake.exists(&p("/b/f/3.bin")));
    assert!(fake.unfinished().is_empty());
    assert_eq!(report.done.len(), 3, "the folder and two files");
    for place in &report.undo.created {
        assert!(fake.exists(place), "the record lists what is there");
    }
}

#[test]
fn a_pause_holds_between_chunks_and_a_cancel_while_paused_ends_the_job() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/big"), &vec![1u8; 3 * 1024 * 1024]);
    fake.add_folder(&p("/b"));
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::Copy, &["/a/big"]).to(p("/b")))
        .unwrap();
    let control = JobControl::new();
    let pauser = control.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.bytes_done == 1024 * 1024 && progress.state == JobState::Running {
            pauser.pause();
            let resumer = pauser.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(60));
                resumer.resume();
            });
        }
    });
    let report = engine.run(&plan, &control, &sink);
    assert_eq!(report.done.len(), 1);
    let changes = sink.state_changes();
    let paused = changes.iter().position(|s| *s == JobState::Paused).unwrap();
    assert_eq!(changes[paused + 1], JobState::Running, "{changes:?}");
    assert!(
        sink.said
            .lock()
            .unwrap()
            .contains(&"Copying paused at 33 percent".to_owned())
    );
    // Cancel while paused.
    let fake2 = FakeProvider::new();
    fake2.add_file(&p("/a/big"), &vec![1u8; 3 * 1024 * 1024]);
    fake2.add_folder(&p("/b"));
    let engine2 = crate::testing::engine(&fake2);
    let plan2 = engine2
        .plan(&request(JobKind::Copy, &["/a/big"]).to(p("/b")))
        .unwrap();
    let control2 = JobControl::new();
    let both = control2.clone();
    let sink2 = Script::default().on_progress(move |progress| {
        if progress.bytes_done >= 1024 * 1024 {
            both.pause();
            let canceller = both.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(40));
                canceller.cancel.cancel();
            });
        }
    });
    let report2 = engine2.run(&plan2, &control2, &sink2);
    assert!(report2.cancelled);
    assert!(fake2.unfinished().is_empty());
}

#[test]
fn a_full_drive_at_byte_n_keeps_the_old_target_and_says_why() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x"), &vec![2u8; 4096]);
    fake.add_file(&p("/b/x"), b"old");
    fake.switches_mut().faults.push(Fault {
        place: p("/b/x"),
        at_byte: Some(1000),
        error: VfsError::StorageFull,
    });
    let sink = Script::conflicts(vec![ConflictAnswer::once(ConflictAction::Replace)]);
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/x"]).to(p("/b")),
        &sink,
    );
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].1, "the drive is full");
    assert_eq!(
        fake.contents(&p("/b/x")).unwrap(),
        b"old",
        "the old one kept"
    );
    assert!(fake.unfinished().is_empty());
    assert!(
        !fake.in_trash(&p("/b/x")),
        "nothing trashed before the bytes"
    );
    let asked = sink.failed.lock().unwrap();
    assert!(within_cells(&asked[0].message().text, "Could not copy x"));
}

#[test]
fn a_read_error_fails_one_item_and_the_rest_go_on() {
    let fake = tree();
    fake.switches_mut().faults.push(Fault {
        place: p("/a/photos/one.jpg"),
        at_byte: Some(2),
        error: VfsError::PermissionDenied,
    });
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/photos", "/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.done.len(), 4);
    assert!(fake.exists(&p("/b/notes.txt")));
    assert!(report.summary().starts_with("Copy had 1 failure: 4 done"));
}

#[test]
fn a_destination_folder_removed_mid_job_fails_each_item_once() {
    let fake = FakeProvider::new();
    for n in 0..3 {
        fake.add_file(&p(&format!("/a/f/{n}")), b"x");
    }
    fake.add_folder(&p("/b"));
    let remover = fake.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.items_done == 2 && remover.exists(&p("/b/f")) {
            remover.vanish(&p("/b/f"));
        }
    });
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/f"]).to(p("/b")),
        &sink,
    );
    assert_eq!(report.failed.len(), 2, "{report:?}");
    assert_eq!(sink.failed.lock().unwrap().len(), 2, "asked once each");
}

#[test]
fn a_source_that_changed_since_the_plan_is_a_warning() {
    let fake = tree();
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")))
        .unwrap();
    fake.add_file(&p("/a/notes.txt"), b"notes, longer now");
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.done.len(), 1);
    assert_eq!(report.warnings[0].1, "it changed since the plan");
    assert_eq!(
        fake.contents(&p("/b/notes.txt")).unwrap(),
        b"notes, longer now"
    );
}

#[test]
fn a_link_is_copied_as_a_link_and_never_followed() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/outside/secret.txt"), b"keep");
    fake.add_link(&p("/a/f/out"), &p("/outside").to_string(), true);
    fake.add_file(&p("/a/f/in.txt"), b"in");
    fake.add_folder(&p("/b"));
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/f"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 3);
    let link = fake.entry(&p("/b/f/out")).unwrap();
    assert_eq!(link.kind, EntryKind::Link);
    assert_eq!(link.link.unwrap().target, p("/outside").to_string());
    assert!(!fake.exists(&p("/b/f/out/secret.txt")));
    let delete = run(
        &engine(&fake),
        &request(JobKind::DeletePermanently, &["/a/f"]),
        &Script::default(),
    );
    assert_eq!(delete.done.len(), 3);
    assert_eq!(fake.contents(&p("/outside/secret.txt")).unwrap(), b"keep");
}

#[test]
fn another_drive_inside_the_tree_is_skipped_and_said() {
    let fake = FakeProvider::with_switches(FakeSwitches {
        second_volume: Some(p("/a/f/usb")),
        ..FakeSwitches::default()
    });
    fake.add_file(&p("/a/f/usb/x"), b"1");
    fake.add_file(&p("/a/f/y"), b"2");
    fake.add_folder(&p("/b"));
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/f"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].1, "another drive is mounted here");
    assert!(!fake.exists(&p("/b/f/usb")));
}

// ----- conflicts -----

fn conflict_tree() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.switches_mut().now = Some(at(0));
    fake.add_file(&p("/a/1.txt"), b"new one");
    fake.add_file(&p("/a/2.txt"), b"new two");
    fake.add_file(&p("/a/3.txt"), b"new three");
    fake.switches_mut().now = Some(at(-5));
    fake.add_file(&p("/b/1.txt"), b"old one");
    fake.add_file(&p("/b/2.txt"), b"old two");
    fake.add_file(&p("/b/3.txt"), b"old three");
    fake
}

fn copy_all(fake: &FakeProvider, sink: &Script) -> JobReport {
    run(
        &engine(fake),
        &request(JobKind::Copy, &["/a/1.txt", "/a/2.txt", "/a/3.txt"]).to(p("/b")),
        sink,
    )
}

#[test]
fn each_conflict_is_asked_once_and_an_answer_for_all_settles_the_rest() {
    let fake = conflict_tree();
    let sink = Script::conflicts(vec![
        ConflictAnswer::once(ConflictAction::Replace),
        ConflictAnswer::for_all(ConflictAction::KeepBoth),
    ]);
    let report = copy_all(&fake, &sink);
    assert_eq!(sink.asked.lock().unwrap().len(), 2, "the third is settled");
    let first = &sink.asked.lock().unwrap()[0];
    assert_eq!(first.kind, ConflictKind::FileOverFile);
    assert_eq!(first.keep_both_name, "1 (2).txt");
    assert_eq!(fake.contents(&p("/b/1.txt")).unwrap(), b"new one");
    assert_eq!(fake.contents(&p("/b/2 (2).txt")).unwrap(), b"new two");
    assert_eq!(fake.contents(&p("/b/3 (2).txt")).unwrap(), b"new three");
    assert_eq!(fake.contents(&p("/b/2.txt")).unwrap(), b"old two");
    assert_eq!(report.done.len(), 3);
    assert!(
        fake.in_trash(&p("/b/1.txt")),
        "the replaced file went to the trash"
    );
    assert_eq!(report.undo.trashed, vec![p("/b/1.txt")]);
    assert!(sink.state_changes().contains(&JobState::WaitingForAnswer));
}

#[test]
fn skip_rename_cancel_and_the_policies_are_honored() {
    let fake = conflict_tree();
    let sink = Script::conflicts(vec![
        ConflictAnswer::skip(),
        ConflictAnswer::once(ConflictAction::Rename("renamed.txt".into())),
        ConflictAnswer::once(ConflictAction::Cancel),
    ]);
    let report = copy_all(&fake, &sink);
    assert_eq!(report.skipped[0].1, "the name is taken");
    assert_eq!(fake.contents(&p("/b/renamed.txt")).unwrap(), b"new two");
    assert!(report.cancelled);
    assert_eq!(fake.contents(&p("/b/3.txt")).unwrap(), b"old three");
    for (policy, expect) in [
        (ConflictPolicy::Skip, b"old one".as_slice()),
        (ConflictPolicy::Overwrite, b"new one".as_slice()),
        (ConflictPolicy::Newer, b"new one".as_slice()),
    ] {
        let fake = conflict_tree();
        let report = run(
            &engine(&fake),
            &JobRequest {
                conflicts: policy,
                ..request(JobKind::Copy, &["/a/1.txt"]).to(p("/b"))
            },
            &Script::default(),
        );
        assert_eq!(fake.contents(&p("/b/1.txt")).unwrap(), expect, "{policy:?}");
        assert!(!report.cancelled);
    }
    let fake = conflict_tree();
    fake.set_modified(&p("/a/1.txt"), at(-5));
    fake.set_modified(
        &p("/b/1.txt"),
        Timestamp::from_unix(at(-5).unix_seconds() - 1, 0),
    );
    let tie = run(
        &engine(&fake),
        &JobRequest {
            conflicts: ConflictPolicy::Newer,
            ..request(JobKind::Copy, &["/a/1.txt"]).to(p("/b"))
        },
        &Script::default(),
    );
    assert_eq!(
        tie.skipped[0].1, "not newer than the one there",
        "a tie skips"
    );
}

#[test]
fn a_read_only_target_needs_its_own_answer() {
    let fake = conflict_tree();
    fake.set_attributes_now(&p("/b/1.txt"), Attributes::Windows(0x1));
    let sink = Script::default();
    let report = run(
        &engine(&fake),
        &JobRequest {
            conflicts: ConflictPolicy::Overwrite,
            ..request(JobKind::Copy, &["/a/1.txt", "/a/2.txt"]).to(p("/b"))
        },
        &sink,
    );
    let asked = sink.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].kind, ConflictKind::ReadOnlyTarget);
    assert_eq!(
        fake.contents(&p("/b/1.txt")).unwrap(),
        b"old one",
        "skipped"
    );
    assert_eq!(fake.contents(&p("/b/2.txt")).unwrap(), b"new two");
    assert_eq!(report.done.len(), 1);
}

#[test]
fn type_clashes_skip_and_folders_merge_and_nothing_is_deleted() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/f/inner.txt"), b"in");
    fake.add_file(&p("/a/g"), b"a file");
    fake.add_file(&p("/b/f/kept.txt"), b"kept");
    fake.add_folder(&p("/b/g"));
    let sink = Script::default();
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/f", "/a/g"]).to(p("/b")),
        &sink,
    );
    assert_eq!(fake.contents(&p("/b/f/kept.txt")).unwrap(), b"kept");
    assert_eq!(fake.contents(&p("/b/f/inner.txt")).unwrap(), b"in");
    assert!(fake.exists(&p("/b/g")));
    assert_eq!(report.skipped[0].1, "a folder has that name here");
    let asked = sink.asked.lock().unwrap();
    assert_eq!(asked.len(), 1, "asked, so the user could cancel");
    assert_eq!(asked[0].kind, ConflictKind::FileOverFolder);
}

#[test]
fn the_same_file_is_never_asked_and_loses_nothing() {
    let fake = conflict_tree();
    fake.add_hard_link(&p("/c/alias.txt"), &p("/a/1.txt"));
    let engine = engine(&fake);
    let plan = Plan::new(
        JobId::new(2),
        JobKind::Copy,
        vec![Step::Copy {
            from: p("/a/1.txt"),
            to: p("/c/alias.txt"),
        }],
        ConflictPolicy::Ask,
    );
    let sink = Script::default();
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert!(sink.asked.lock().unwrap().is_empty());
    assert_eq!(report.skipped[0].1, "Same file: alias.txt. Nothing to do.");
    assert_eq!(fake.contents(&p("/a/1.txt")).unwrap(), b"new one");
}

#[test]
fn keep_both_numbers_past_taken_names_and_case_twins() {
    let fake = FakeProvider::with_switches(FakeSwitches {
        case_insensitive: true,
        ..FakeSwitches::default()
    });
    fake.add_file(&p("/a/report.pdf"), b"new");
    fake.add_file(&p("/b/report.pdf"), b"1");
    fake.add_file(&p("/b/REPORT (2).pdf"), b"2");
    fake.add_file(&p("/a/archive.tar.gz"), b"new");
    fake.add_file(&p("/b/archive.tar.gz"), b"old");
    let report = run(
        &engine(&fake),
        &JobRequest {
            conflicts: ConflictPolicy::KeepBoth,
            ..request(JobKind::Copy, &["/a/report.pdf", "/a/archive.tar.gz"]).to(p("/b"))
        },
        &Script::default(),
    );
    assert_eq!(report.done.len(), 2);
    assert_eq!(fake.contents(&p("/b/report (3).pdf")).unwrap(), b"new");
    assert_eq!(fake.contents(&p("/b/archive (2).tar.gz")).unwrap(), b"new");
}

#[test]
fn replace_where_there_is_no_trash_says_it_cannot_be_undone() {
    let fake = conflict_tree();
    fake.switches_mut().no_trash = true;
    let report = run(
        &engine(&fake),
        &JobRequest {
            conflicts: ConflictPolicy::Overwrite,
            ..request(JobKind::Copy, &["/a/1.txt"]).to(p("/b"))
        },
        &Script::default(),
    );
    assert_eq!(fake.contents(&p("/b/1.txt")).unwrap(), b"new one");
    assert_eq!(report.warnings[0].1, "replaced; this cannot be undone");
    let engine = engine(&fake);
    let conflict = crate::engine::Conflict {
        kind: ConflictKind::FileOverFile,
        source: p("/a/2.txt"),
        target: p("/b/2.txt"),
        source_entry: fake.entry(&p("/a/2.txt")).unwrap(),
        target_entry: fake.entry(&p("/b/2.txt")).unwrap(),
        keep_both_name: String::new(),
    };
    assert!(!engine.replace_can_be_undone(&conflict));
}

// ----- clones and lies -----

#[test]
fn a_clone_is_tried_first_and_a_late_failure_falls_back_cleanly() {
    let fake = tree();
    fake.switches_mut().clone_copy = true;
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 1);
    assert_eq!(fake.contents(&p("/b/notes.txt")).unwrap(), b"notes");
    fake.switches_mut().lies.clone_fails_late = true;
    let late = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/photos/one.jpg"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(late.done.len(), 1, "{late:?}");
    assert_eq!(fake.contents(&p("/b/one.jpg")).unwrap(), b"one-one-one");
    assert!(fake.unfinished().is_empty());
}

#[test]
fn a_write_that_lies_is_a_failure_never_done() {
    let fake = tree();
    fake.switches_mut().lies.write_without_writing = true;
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 0);
    assert!(report.failed[0].1.starts_with("nothing was written"));
    assert!(report.summary().starts_with("Copy had 1 failure"));
}

#[test]
fn a_dropped_case_twin_is_caught() {
    let fake = FakeProvider::with_switches(FakeSwitches {
        case_insensitive: true,
        ..FakeSwitches::default()
    });
    fake.add_file(&p("/a/x/README"), b"upper case one");
    fake.add_file(&p("/a/y/readme"), b"lower");
    fake.add_folder(&p("/b"));
    fake.switches_mut().lie_about.push(p("/b/readme"));
    let report = run(
        &engine(&fake),
        &JobRequest {
            conflicts: ConflictPolicy::Overwrite,
            ..request(JobKind::Copy, &["/a/x/README", "/a/y/readme"]).to(p("/b"))
        },
        &Script::default(),
    );
    assert!(
        report.summary().starts_with("Copy had 1 failure"),
        "{}",
        report.summary()
    );
}

#[test]
fn the_reconciliation_catches_a_result_gone_by_the_end() {
    let fake = tree();
    let remover = fake.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.items_done == 2 {
            remover.vanish(&p("/b/photos/one.jpg"));
        }
    });
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/photos"]).to(p("/b")),
        &sink,
    );
    let check = report.reconciliation.clone().unwrap();
    assert_eq!(check.missing, vec![p("/b/photos/one.jpg")]);
    assert_eq!(check.summary(), "Checked 4 of 4: 1 missing");
    assert!(report.summary().starts_with("Copy had 1 failure"));
}

// ----- move -----

#[test]
fn a_move_on_one_drive_is_a_rename_and_keeps_times() {
    let fake = tree();
    let report = run(
        &engine(&fake),
        &request(JobKind::Move, &["/a/photos"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.summary(), "Move finished: 1 done.");
    assert_eq!(
        fake.entry(&p("/b/photos/one.jpg")).unwrap().modified,
        Some(at(-10))
    );
    assert!(!fake.exists(&p("/a/photos")));
    assert_eq!(report.undo.moves, vec![(p("/a/photos"), p("/b/photos"))]);
}

fn two_drives() -> FakeProvider {
    let fake = tree();
    fake.switches_mut().second_volume = Some(p("/usb"));
    fake.add_folder(&p("/usb"));
    fake
}

#[test]
fn a_move_across_drives_copies_verifies_then_removes() {
    let fake = two_drives();
    let sink = Script::default();
    let report = run(
        &engine(&fake),
        &request(JobKind::Move, &["/a/photos", "/a/notes.txt"]).to(p("/usb")),
        &sink,
    );
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(
        fake.contents(&p("/usb/photos/trip/two.jpg")).unwrap(),
        b"two"
    );
    assert!(!fake.exists(&p("/a/photos")), "the emptied folders removed");
    assert!(!fake.exists(&p("/a/notes.txt")));
    assert!(sink.state_changes().contains(&JobState::Verifying));
    assert!(
        sink.said
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("Verifying"))
    );
}

#[test]
fn a_copy_that_does_not_match_keeps_the_source() {
    let fake = two_drives();
    fake.switches_mut().corrupt.push(p("/usb/notes.txt"));
    let report = run(
        &engine(&fake),
        &request(JobKind::Move, &["/a/notes.txt"]).to(p("/usb")),
        &Script::default(),
    );
    assert_eq!(
        report.failed[0].1,
        "the copy did not match its source; the source is kept"
    );
    assert_eq!(fake.contents(&p("/a/notes.txt")).unwrap(), b"notes");
    assert!(
        !fake.exists(&p("/usb/notes.txt")),
        "the bad copy is not left"
    );
}

#[test]
fn a_source_that_cannot_be_removed_leaves_both_and_says_so() {
    let fake = two_drives();
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::Remove,
        p("/a/notes.txt"),
        VfsError::InUse,
    ));
    let report = run(
        &engine(&fake),
        &request(JobKind::Move, &["/a/notes.txt"]).to(p("/usb")),
        &Script::default(),
    );
    assert!(
        report.failed[0]
            .1
            .starts_with("copied, but the original could not be removed")
    );
    assert!(fake.exists(&p("/a/notes.txt")));
    assert!(fake.exists(&p("/usb/notes.txt")));
    assert_eq!(report.undo.created, vec![p("/usb/notes.txt")]);
}

#[test]
fn a_source_changed_while_moving_is_kept() {
    let fake = two_drives();
    let changer = fake.clone();
    let sink = Script::default().on_progress(move |progress| {
        if progress.state == JobState::Verifying {
            changer.set_modified(&p("/a/notes.txt"), at(1));
        }
    });
    let report = run(
        &engine(&fake),
        &request(JobKind::Move, &["/a/notes.txt"]).to(p("/usb")),
        &sink,
    );
    assert_eq!(
        report.failed[0].1,
        "it changed while moving; both copies are kept"
    );
    assert!(fake.exists(&p("/a/notes.txt")));
}

#[test]
fn a_skipped_file_keeps_its_folder_and_nothing_else_is_removed() {
    let fake = two_drives();
    fake.add_file(&p("/usb/photos/one.jpg"), b"already there");
    let report = run(
        &engine(&fake),
        &request(JobKind::Move, &["/a/photos"]).to(p("/usb")),
        &Script::default(),
    );
    assert_eq!(
        fake.contents(&p("/a/photos/one.jpg")).unwrap(),
        b"one-one-one"
    );
    assert_eq!(
        fake.contents(&p("/usb/photos/one.jpg")).unwrap(),
        b"already there"
    );
    assert!(fake.exists(&p("/usb/photos/trip/two.jpg")));
    assert!(fake.exists(&p("/a/photos")), "kept: not empty");
    assert!(
        !fake.exists(&p("/a/photos/trip")),
        "the emptied subfolder went"
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|(_, reason)| reason == "Folder kept: it is not empty")
    );
}

#[test]
fn a_rename_that_crosses_drives_at_run_time_falls_back() {
    let fake = two_drives();
    let plan = Plan::new(
        JobId::new(3),
        JobKind::Move,
        vec![Step::Move {
            from: p("/a/photos"),
            to: p("/usb/photos"),
        }],
        ConflictPolicy::Ask,
    );
    let report = engine(&fake).run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{report:?}");
    assert!(!fake.exists(&p("/a/photos")));
    assert_eq!(
        fake.contents(&p("/usb/photos/one.jpg")).unwrap(),
        b"one-one-one"
    );
    assert_eq!(report.undo.moves, vec![(p("/a/photos"), p("/usb/photos"))]);
}

// ----- the trash and removal -----

#[test]
fn the_trash_takes_items_and_undo_brings_them_back() {
    let fake = tree();
    let engine = engine(&fake);
    let before = fake.snapshot();
    let report = run(
        &engine,
        &request(JobKind::Trash, &["/a/photos", "/a/notes.txt"]),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 2);
    assert_eq!(fake.trashed(), 2);
    let mut log = UndoLog::default();
    log.record(report.undo);
    let undo = engine
        .undo_last(
            &mut log,
            JobId::new(12),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(undo.failed.is_empty(), "{undo:?}");
    assert_eq!(fake.snapshot(), before);
}

#[test]
fn a_trash_that_fails_or_lies_never_falls_through_to_a_delete() {
    for case in 0..3 {
        let fake = tree();
        fake.add_folder(&p("/home/ada"));
        match case {
            0 => fake.switches_mut().lies.trash_does_nothing = true,
            1 => fake.switches_mut().call_faults.push(CallFault::new(
                FakeCall::Trash,
                p("/a/photos"),
                VfsError::unsupported("file", "move to the trash"),
            )),
            _ => fake.switches_mut().call_faults.push(CallFault::new(
                FakeCall::Trash,
                p("/a/photos"),
                VfsError::Io {
                    kind: std::io::ErrorKind::Other,
                    message: "the shell said no".into(),
                },
            )),
        }
        let before = fake.snapshot();
        let report = run(
            &engine(&fake),
            &request(JobKind::Trash, &["/a/photos"]),
            &Script::default(),
        );
        assert_eq!(report.failed.len(), 1, "case {case}");
        assert_eq!(
            fake.snapshot(),
            before,
            "case {case}: the tree is untouched"
        );
        if case == 0 {
            assert_eq!(report.failed[0].1, "Still there after the trash.");
        }
    }
}

#[test]
fn where_there_is_no_trash_nothing_is_deleted_and_the_question_is_asked() {
    let fake = tree();
    fake.switches_mut().no_trash = true;
    fake.switches_mut().volume_kind = Some(VolumeKind::Removable);
    let before = fake.snapshot();
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::Trash, &["/a/notes.txt"]))
        .unwrap();
    assert!(!plan.facts.questions.is_empty());
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.skipped[0].1, "this removable drive has no trash");
    assert_eq!(fake.snapshot(), before);
}

#[test]
fn a_permanent_delete_keeps_a_folder_with_a_survivor_and_goes_on() {
    let fake = tree();
    fake.switches_mut().locked.push(p("/a/photos/trip/two.jpg"));
    let report = run(
        &engine(&fake),
        &request(JobKind::DeletePermanently, &["/a/photos", "/a/notes.txt"]),
        &Script::default(),
    );
    assert!(!fake.exists(&p("/a/notes.txt")));
    assert!(!fake.exists(&p("/a/photos/one.jpg")));
    assert!(fake.exists(&p("/a/photos/trip/two.jpg")));
    assert_eq!(report.failed.len(), 1, "the locked file, once");
    assert_eq!(report.failed[0].1, "another program is using it");
    let kept: Vec<&String> = report.skipped.iter().map(|(_, reason)| reason).collect();
    assert_eq!(kept, vec!["Folder kept: it is not empty"; 2]);
}

#[test]
fn a_read_only_entry_is_cleared_as_part_of_the_listed_delete() {
    let fake = tree();
    fake.set_attributes_now(&p("/a/notes.txt"), Attributes::Windows(0x21));
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::DeletePermanently, &["/a/notes.txt"]))
        .unwrap();
    assert!(plan.listing()[0].ends_with(", read-only"));
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.done.len(), 1, "{report:?}");
    assert!(!fake.exists(&p("/a/notes.txt")));
}

#[test]
fn a_folder_that_is_not_empty_for_a_moment_is_tried_again() {
    let fake = tree();
    fake.switches_mut()
        .call_faults
        .push(CallFault::new(FakeCall::Remove, p("/a/photos/trip"), VfsError::NotEmpty).times(2));
    let report = run(
        &engine(&fake).with_pacer(Arc::new(FakePacer::new())),
        &request(JobKind::DeletePermanently, &["/a/photos"]),
        &Script::default(),
    );
    assert!(
        report.failed.is_empty() && report.skipped.is_empty(),
        "{report:?}"
    );
    assert!(!fake.exists(&p("/a/photos")));
}

#[test]
fn a_folder_that_became_a_link_is_not_followed() {
    let fake = tree();
    fake.add_file(&p("/outside/keep.txt"), b"keep");
    let engine = engine(&fake);
    let plan = engine
        .plan(&request(JobKind::DeletePermanently, &["/a/photos"]))
        .unwrap();
    fake.vanish(&p("/a/photos/trip"));
    fake.add_link(&p("/a/photos/trip"), &p("/outside").to_string(), true);
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(fake.contents(&p("/outside/keep.txt")).unwrap(), b"keep");
    assert!(
        report
            .skipped
            .iter()
            .any(|(_, reason)| reason.starts_with("it changed since the plan"))
    );
}

// ----- rename, make folder, links -----

#[test]
fn a_swap_runs_whole_and_undoes() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x.txt"), b"x");
    fake.add_file(&p("/a/y.txt"), b"y");
    let engine = engine(&fake);
    let before = fake.snapshot();
    let swap = JobRequest::new(JobId::new(9), JobKind::Rename, Vec::new()).renaming(vec![
        (p("/a/x.txt"), "y.txt".into()),
        (p("/a/y.txt"), "x.txt".into()),
    ]);
    let report = run(&engine, &swap, &Script::default());
    assert_eq!(report.done.len(), 2, "{report:?}");
    assert_eq!(fake.contents(&p("/a/x.txt")).unwrap(), b"y");
    assert_eq!(fake.contents(&p("/a/y.txt")).unwrap(), b"x");
    assert!(!fake.exists(&p("/a/diro-tmp-9-1")));
    let mut log = UndoLog::default();
    log.record(report.undo);
    engine
        .undo_last(
            &mut log,
            JobId::new(10),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert_eq!(fake.snapshot(), before);
}

#[test]
fn a_swap_that_fails_half_way_puts_every_name_back() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x.txt"), b"x");
    fake.add_file(&p("/a/y.txt"), b"y");
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::Rename,
        p("/a/y.txt"),
        VfsError::InUse,
    ));
    let before = fake.snapshot();
    let swap = JobRequest::new(JobId::new(9), JobKind::Rename, Vec::new()).renaming(vec![
        (p("/a/x.txt"), "y.txt".into()),
        (p("/a/y.txt"), "x.txt".into()),
    ]);
    let report = run(&engine(&fake), &swap, &Script::default());
    assert_eq!(report.failed.len(), 2);
    assert!(report.failed[0].1.starts_with("the swap could not finish"));
    assert_eq!(fake.snapshot(), before);
}

#[test]
fn a_rename_never_replaces_and_a_case_only_rename_works() {
    let fake = FakeProvider::with_switches(FakeSwitches {
        case_insensitive: true,
        ..FakeSwitches::default()
    });
    fake.add_file(&p("/a/x.txt"), b"x");
    fake.add_file(&p("/a/taken.txt"), b"t");
    let engine = engine(&fake);
    let report = run(
        &engine,
        &request(JobKind::Rename, &["/a/x.txt"]).named("taken.txt"),
        &Script::default(),
    );
    assert_eq!(report.failed[0].1, "the name is already taken");
    assert_eq!(fake.contents(&p("/a/taken.txt")).unwrap(), b"t");
    let case = run(
        &engine,
        &request(JobKind::Rename, &["/a/x.txt"]).named("X.txt"),
        &Script::default(),
    );
    assert_eq!(case.done.len(), 1, "{case:?}");
    assert_eq!(fake.entry(&p("/a/x.txt")).unwrap().name, "X.txt");
}

#[test]
fn make_folder_makes_the_missing_levels_and_undo_removes_only_those() {
    let fake = tree();
    let engine = engine(&fake);
    let make = |name: &str| {
        JobRequest::new(JobId::new(5), JobKind::MakeFolder, Vec::new())
            .to(p("/a"))
            .named(name)
    };
    let report = run(&engine, &make("photos/new/deeper"), &Script::default());
    assert_eq!(report.done.len(), 3);
    assert_eq!(
        report.undo.created,
        vec![p("/a/photos/new"), p("/a/photos/new/deeper")],
        "the level that was there is not the job's"
    );
    let again = run(&engine, &make("photos/new/deeper"), &Script::default());
    assert_eq!(
        again.done.len(),
        3,
        "two jobs making one folder both succeed"
    );
    let mut log = UndoLog::default();
    log.record(report.undo);
    engine
        .undo_last(
            &mut log,
            JobId::new(6),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(!fake.exists(&p("/a/photos/new")));
    assert!(fake.exists(&p("/a/photos")));
}

#[test]
fn links_are_made_as_jobs_and_undone() {
    let fake = tree();
    let engine = engine(&fake);
    let report = run(
        &engine,
        &request(JobKind::Link, &["/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 1);
    assert_eq!(
        fake.entry(&p("/b/notes.txt")).unwrap().kind,
        EntryKind::Link
    );
    let mut log = UndoLog::default();
    log.record(report.undo);
    engine
        .undo_last(
            &mut log,
            JobId::new(6),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(!fake.exists(&p("/b/notes.txt")));
    assert!(fake.exists(&p("/a/notes.txt")));
}

// ----- errors, retries, stalls -----

#[test]
fn a_lost_network_is_retried_on_its_own_then_asked_and_skip_all_skips_its_class() {
    let fake = tree();
    for place in ["/a/notes.txt", "/a/photos/one.jpg"] {
        fake.switches_mut().call_faults.push(CallFault::new(
            FakeCall::Read,
            p(place),
            VfsError::NetworkLost,
        ));
    }
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::Read,
        p("/a/photos/trip/two.jpg"),
        VfsError::InUse,
    ));
    let pacer = Arc::new(FakePacer::new());
    let engine = engine(&fake).with_pacer(pacer.clone());
    let sink = Script::errors(vec![ErrorAction::SkipAllLikeThis]);
    let report = run(
        &engine,
        &request(JobKind::Copy, &["/a/notes.txt", "/a/photos"]).to(p("/b")),
        &sink,
    );
    let asked = sink.failed.lock().unwrap();
    assert_eq!(asked.len(), 2, "the network once, in use once");
    assert_eq!(asked[0].error, VfsError::NetworkLost);
    assert_eq!(asked[1].error, VfsError::InUse);
    assert_eq!(report.failed.len(), 3);
    assert!(
        pacer.now() >= Duration::from_secs(15),
        "three waits of five seconds"
    );
    assert!(pacer.now() < Duration::from_secs(16));
}

#[test]
fn retry_tries_again_and_succeeds() {
    let fake = tree();
    fake.switches_mut()
        .call_faults
        .push(CallFault::new(FakeCall::Read, p("/a/notes.txt"), VfsError::InUse).times(1));
    let sink = Script::errors(vec![ErrorAction::Retry]);
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")),
        &sink,
    );
    assert_eq!(report.done.len(), 1);
    assert_eq!(
        sink.failed.lock().unwrap()[0].choices()[0],
        ErrorAction::Retry
    );
    let automatic = tree();
    automatic
        .switches_mut()
        .call_faults
        .push(CallFault::new(FakeCall::Read, p("/a/notes.txt"), VfsError::NetworkLost).times(2));
    let quiet = Script::default();
    let report = run(
        &engine(&automatic).with_pacer(Arc::new(FakePacer::new())),
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")),
        &quiet,
    );
    assert_eq!(report.done.len(), 1);
    assert!(quiet.failed.lock().unwrap().is_empty(), "never asked");
    assert_eq!(report.warnings[0].1, "done after 2 automatic retries");
}

#[test]
fn a_call_that_hangs_says_stalled_then_goes_on() {
    let fake = tree();
    fake.switches_mut().latency.stall_after_calls = Some(fake.calls() + 4);
    fake.switches_mut().latency.stall_for = Duration::from_millis(400);
    let engine = engine(&fake).with_options(EngineOptions {
        stall_after: Duration::from_millis(60),
        ..EngineOptions::default()
    });
    let plan = engine
        .plan(&request(JobKind::Copy, &["/a/photos"]).to(p("/b")))
        .unwrap();
    fake.switches_mut().latency.stall_after_calls = Some(fake.calls() + 3);
    let sink = Script::default();
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert_eq!(report.done.len(), 4);
    let changes = sink.state_changes();
    let stalled = changes
        .iter()
        .position(|s| *s == JobState::Stalled)
        .expect("stalled said");
    assert_eq!(changes[stalled + 1], JobState::Running);
    assert!(
        sink.said
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("Copying stalled at"))
    );
}

#[test]
fn times_that_cannot_be_kept_are_warnings_not_failures() {
    let fake = tree();
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::SetAttributes,
        p("/b/notes.txt"),
        VfsError::PermissionDenied,
    ));
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    assert_eq!(report.done.len(), 1);
    assert_eq!(
        report.warnings[0].1,
        "copied, but could not keep its modified time: permission was denied"
    );
}

#[test]
fn a_folder_that_cannot_be_made_is_reported_once() {
    let fake = tree();
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::CreateDir,
        p("/b/photos"),
        VfsError::PermissionDenied,
    ));
    let sink = Script::default();
    let report = run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/photos"]).to(p("/b")),
        &sink,
    );
    assert_eq!(report.failed.len(), 1);
    assert_eq!(sink.failed.lock().unwrap().len(), 1, "asked once");
    assert_eq!(report.skipped.len(), 3);
    assert!(report.skipped[0].1.starts_with("its folder was not made"));
}

// ----- progress -----

#[test]
fn every_line_says_its_fact_first_and_the_last_says_finished() {
    let fake = tree();
    for (kind, request) in [
        (
            JobKind::Copy,
            request(JobKind::Copy, &["/a/photos"]).to(p("/b")),
        ),
        (JobKind::Trash, request(JobKind::Trash, &["/a/notes.txt"])),
    ] {
        let sink = Script::default();
        run(&engine(&fake), &request, &sink);
        let said = sink.said.lock().unwrap();
        let word = kind.progress_word();
        for line in said.iter() {
            assert!(
                within_cells(line, word) || line.starts_with("Verifying"),
                "{line}"
            );
        }
        assert_eq!(said.last().unwrap(), &format!("{word} finished"));
        let progress = sink.progress.lock().unwrap();
        let mut bytes = 0;
        let mut items = 0;
        for update in progress.iter() {
            assert!(update.bytes_done >= bytes && update.items_done >= items);
            bytes = update.bytes_done;
            items = update.items_done;
            if let Some(total) = update.bytes_total {
                assert!(update.bytes_done <= total);
            }
        }
    }
}

#[test]
fn a_copy_counts_its_bytes_and_items() {
    let fake = tree();
    let sink = Script::default();
    run(
        &engine(&fake),
        &request(JobKind::Copy, &["/a/photos"]).to(p("/b")),
        &sink,
    );
    let last = sink.progress.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.bytes_total, Some(14));
    assert_eq!(last.bytes_done, 14);
    assert_eq!(last.items_total, 4);
    assert_eq!(last.items_done, 4);
    assert_eq!(last.detail(), "4 of 4 items, 14 of 14 bytes");
}

// ----- undo -----

#[test]
fn undo_of_a_copy_trashes_only_unchanged_copies_never_the_sources() {
    let fake = tree();
    let engine = engine(&fake);
    let report = run(
        &engine,
        &request(JobKind::Copy, &["/a/photos", "/a/notes.txt"]).to(p("/b")),
        &Script::default(),
    );
    fake.add_file(&p("/b/notes.txt"), b"edited after the copy");
    let mut log = UndoLog::default();
    log.record(report.undo);
    let undo = engine
        .undo_last(
            &mut log,
            JobId::new(20),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert_eq!(
        fake.contents(&p("/b/notes.txt")).unwrap(),
        b"edited after the copy",
        "a changed copy is kept"
    );
    assert!(undo.skipped.iter().any(
        |(place, reason)| *place == p("/b/notes.txt") && reason == "it changed since the plan"
    ));
    assert!(!fake.exists(&p("/b/photos")));
    assert_eq!(
        fake.contents(&p("/a/photos/one.jpg")).unwrap(),
        b"one-one-one"
    );
    assert!(log.is_empty());
    assert_eq!(
        engine
            .undo_last(
                &mut log,
                JobId::new(21),
                &JobControl::new(),
                &Script::default()
            )
            .unwrap_err(),
        crate::error::OpsError::NothingToUndo
    );
}

#[test]
fn undo_of_a_replace_brings_the_old_file_back() {
    let fake = conflict_tree();
    let engine = engine(&fake);
    let report = run(
        &engine,
        &JobRequest {
            conflicts: ConflictPolicy::Overwrite,
            ..request(JobKind::Copy, &["/a/1.txt"]).to(p("/b"))
        },
        &Script::default(),
    );
    assert_eq!(fake.contents(&p("/b/1.txt")).unwrap(), b"new one");
    let mut log = UndoLog::default();
    log.record(report.undo);
    let undo = engine
        .undo_last(
            &mut log,
            JobId::new(22),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(undo.failed.is_empty(), "{undo:?}");
    assert_eq!(fake.contents(&p("/b/1.txt")).unwrap(), b"old one");
}

#[test]
fn undo_of_a_trash_whose_name_is_taken_fails_that_item_only() {
    let fake = tree();
    let engine = engine(&fake);
    let report = run(
        &engine,
        &request(JobKind::Trash, &["/a/notes.txt", "/a/photos"]),
        &Script::default(),
    );
    fake.add_file(&p("/a/notes.txt"), b"a new file with the name");
    let mut log = UndoLog::default();
    log.record(report.undo);
    let undo = engine
        .undo_last(
            &mut log,
            JobId::new(23),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert_eq!(undo.failed.len(), 1);
    assert_eq!(undo.failed[0].1, "the name is already taken");
    assert!(fake.exists(&p("/a/photos/one.jpg")), "the other came back");
}

#[test]
fn undo_of_a_move_across_drives_moves_it_back() {
    let fake = two_drives();
    let before = fake.snapshot();
    let engine = engine(&fake);
    let report = run(
        &engine,
        &request(JobKind::Move, &["/a/photos"]).to(p("/usb")),
        &Script::default(),
    );
    let mut log = UndoLog::default();
    log.record(report.undo);
    let undo = engine
        .undo_last(
            &mut log,
            JobId::new(24),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(undo.failed.is_empty(), "{undo:?}");
    let mut after = fake.snapshot();
    after.retain(|place, _| !place.contains("usb"));
    let mut expected = before;
    expected.retain(|place, _| !place.contains("usb"));
    assert_eq!(after, expected);
}

#[test]
fn the_report_lists_done_skipped_failed_and_warned_in_words() {
    let fake = conflict_tree();
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::Read,
        p("/a/3.txt"),
        VfsError::PermissionDenied,
    ));
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::SetAttributes,
        p("/b/2.txt"),
        VfsError::PermissionDenied,
    ));
    let sink = Script::conflicts(vec![
        ConflictAnswer::skip(),
        ConflictAnswer::once(ConflictAction::Replace),
        ConflictAnswer::once(ConflictAction::Replace),
    ]);
    let report = copy_all(&fake, &sink);
    assert_eq!(
        report.skipped,
        vec![(p("/a/1.txt"), "the name is taken".to_owned())]
    );
    assert_eq!(report.done, vec![p("/a/2.txt")]);
    assert_eq!(
        report.failed,
        vec![(p("/a/3.txt"), "permission was denied".to_owned())]
    );
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.summary(), "Copy had 1 failure: 1 done, 1 skipped.");
}
