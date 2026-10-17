//! Permissions and times in the engine, on the fake provider: the plan in
//! words with the recursion first, grouped changes, the order of folders,
//! the danger questions, compare-and-set, undo, times kept for a folder
//! after its contents.

use diroxide_core::{Attributes, JobId, JobKind, JobRequest, Timestamp, UtcOffset};

use crate::access::{AttributeEdit, ModeChange, Recursion, WindowsChange};
use crate::engine::JobControl;
use crate::error::OpsError;
use crate::fake::FakeProvider;
use crate::plan::{Preflight, Step};
use crate::testing::{Script, engine, p};

fn unix(fake: &FakeProvider, path: &str) -> u32 {
    match fake.entry(&p(path)).unwrap().attributes {
        Some(Attributes::Unix(mode)) => mode,
        other => panic!("{other:?}"),
    }
}

fn set(fake: &FakeProvider, path: &str, mode: u32) {
    fake.set_attributes_now(&p(path), Attributes::Unix(mode));
}

fn request(kind: JobKind, sources: &[&str]) -> JobRequest {
    JobRequest::new(JobId::new(27), kind, sources.iter().map(|s| p(s)).collect())
}

fn rows(plan: &crate::plan::Plan) -> Vec<String> {
    plan.facts.attributes.clone().unwrap().rows
}

fn t(seconds: i64) -> Timestamp {
    Timestamp::from_unix(1_790_000_000 + seconds, 0)
}

#[test]
fn one_file_says_the_new_permissions_in_words_with_the_octal_second() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/report.pdf"), b"x");
    set(&fake, "/a/report.pdf", 0o600);
    let engine = engine(&fake);
    let edit =
        AttributeEdit::new(Recursion::ThisItem).with_mode(ModeChange::Bits { on: 0o044, off: 0 });
    let plan = engine
        .plan_attributes(
            &request(JobKind::SetAttributes, &["/a/report.pdf"]).changing(edit.clone()),
        )
        .unwrap();
    assert_eq!(
        rows(&plan),
        vec![
            "Apply to: this item only",
            "Owner can read and write, group can read, others can read (644)"
        ]
    );
    assert_eq!(plan.summary(), "Change permissions of 1 item");
    assert_eq!(plan.question().text, "Change permissions of 1 item? y or n");
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(unix(&fake, "/a/report.pdf"), 0o644);
    // A second run of the same edit has nothing to do.
    let again = engine
        .plan_attributes(
            &request(JobKind::SetAttributes, &["/a/report.pdf"]).changing(edit.clone()),
        )
        .unwrap();
    assert!(again.steps.is_empty());
    assert_eq!(
        rows(&again).last().unwrap(),
        "Nothing to change: permissions already match."
    );
    // Undo restores the old value.
    let undo = engine.plan_undo(&report.undo, JobId::new(28)).unwrap();
    let undone = engine.run(&undo, &JobControl::new(), &Script::default());
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(unix(&fake, "/a/report.pdf"), 0o600);
}

fn tree() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_file(&p("/p/a.txt"), b"a");
    fake.add_file(&p("/p/sub/b.txt"), b"b");
    set(&fake, "/p", 0o755);
    set(&fake, "/p/sub", 0o700);
    set(&fake, "/p/a.txt", 0o600);
    set(&fake, "/p/sub/b.txt", 0o600);
    fake
}

#[test]
fn a_change_inside_folders_lists_the_recursion_first_and_groups_the_changes() {
    let fake = tree();
    let engine = engine(&fake);
    let edit = AttributeEdit::new(Recursion::FilesAndFolders)
        .with_mode(ModeChange::Bits { on: 0o040, off: 0 });
    let plan = engine
        .plan_attributes(&request(JobKind::SetAttributes, &["/p"]).changing(edit.clone()))
        .unwrap();
    assert_eq!(
        rows(&plan),
        vec![
            "Apply to: files and folders inside",
            "2 files: group gains read",
            "1 folder: group gains list and enter",
            "Files keep their run setting.",
            "Folders stay enterable.",
        ]
    );
    assert_eq!(plan.summary(), "Change permissions of 3 items");
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(unix(&fake, "/p/sub"), 0o750);
    assert_eq!(unix(&fake, "/p/sub/b.txt"), 0o640);
    assert_eq!(unix(&fake, "/p"), 0o755, "already had it");
    let undo = engine.plan_undo(&report.undo, JobId::new(29)).unwrap();
    engine.run(&undo, &JobControl::new(), &Script::default());
    for (path, mode) in [
        ("/p/sub", 0o700),
        ("/p/a.txt", 0o600),
        ("/p/sub/b.txt", 0o600),
    ] {
        assert_eq!(unix(&fake, path), mode, "{path}");
    }
}

#[test]
fn a_folder_that_loses_its_owners_enter_is_changed_after_its_contents() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/q/inner.txt"), b"i");
    set(&fake, "/q", 0o700);
    set(&fake, "/q/inner.txt", 0o600);
    let edit = AttributeEdit::new(Recursion::FilesAndFolders).with_mode(ModeChange::Exact {
        files: 0o200,
        folders: 0o200,
    });
    let plan = engine(&fake)
        .plan_attributes(&request(JobKind::SetAttributes, &["/q"]).changing(edit.clone()))
        .unwrap();
    let order: Vec<_> = plan.steps.iter().map(|s| s.subject().clone()).collect();
    assert_eq!(order, vec![p("/q/inner.txt"), p("/q")]);
    assert!(
        plan.facts
            .questions
            .iter()
            .any(|q| q.lines()[0] == "You would lose read access to 2 items. Continue? y or n"),
        "{:?}",
        plan.facts.questions
    );
}

#[test]
fn an_entry_changed_after_the_plan_is_skipped_never_overwritten() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x"), b"x");
    set(&fake, "/a/x", 0o600);
    let engine = engine(&fake);
    let edit =
        AttributeEdit::new(Recursion::ThisItem).with_mode(ModeChange::Bits { on: 0o040, off: 0 });
    let plan = engine
        .plan_attributes(&request(JobKind::SetAttributes, &["/a/x"]).changing(edit.clone()))
        .unwrap();
    set(&fake, "/a/x", 0o666);
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.skipped[0].1, "it changed after the plan");
    assert_eq!(unix(&fake, "/a/x"), 0o666);
    assert!(report.undo.is_empty());
}

#[test]
fn links_are_never_changed_and_dangers_ask_first() {
    let fake = tree();
    fake.add_link(&p("/p/latest"), "a.txt", false);
    let engine = engine(&fake);
    let edit =
        AttributeEdit::new(Recursion::ThisItem).with_mode(ModeChange::Bits { on: 0o002, off: 0 });
    assert_eq!(
        engine.plan_attributes(
            &request(JobKind::SetAttributes, &["/p/latest"]).changing(edit.clone())
        ),
        Err(OpsError::LinkSelected("latest".into()))
    );
    let open = engine
        .plan_attributes(&request(JobKind::SetAttributes, &["/p/a.txt"]).changing(edit.clone()))
        .unwrap();
    assert_eq!(
        open.facts.questions[0].lines()[0],
        "Let everyone change a.txt? y or n"
    );
    let inside = engine
        .plan_attributes(
            &request(JobKind::SetAttributes, &["/p"]).changing(
                AttributeEdit::new(Recursion::FilesAndFolders)
                    .with_mode(ModeChange::Bits { on: 0o004, off: 0 }),
            ),
        )
        .unwrap();
    assert!(
        inside
            .facts
            .left_out
            .iter()
            .any(|(place, why)| *place == p("/p/latest") && why.starts_with("a link")),
        "{:?}",
        inside.facts.left_out
    );
    let setuid = engine
        .plan_attributes(
            &request(JobKind::SetAttributes, &["/p/a.txt"]).changing(
                AttributeEdit::new(Recursion::ThisItem)
                    .with_mode(ModeChange::Bits { on: 0o4000, off: 0 }),
            ),
        )
        .unwrap();
    assert!(matches!(
        setuid.facts.questions.first(),
        Some(Preflight::RunsAsOwner { .. })
    ));
    assert_eq!(
        setuid.facts.questions[0].lines()[0],
        "Let a.txt run as its owner (setuid)? y or n"
    );
    assert_eq!(
        engine.plan(&request(JobKind::SetAttributes, &["/p/a.txt"])),
        Err(OpsError::NoChange)
    );
}

#[test]
fn times_are_set_for_a_folder_after_its_contents_and_undone() {
    let fake = FakeProvider::new();
    fake.switches_mut().now = Some(t(0));
    fake.add_file(&p("/t/one.txt"), b"1");
    fake.add_file(&p("/t/sub/two.txt"), b"2");
    let engine = engine(&fake);
    let edit = AttributeEdit::new(Recursion::FilesAndFolders)
        .with_modified(t(3600))
        .at_offset(UtcOffset::UTC);
    let plan = engine
        .plan_attributes(&request(JobKind::SetTimes, &["/t"]).changing(edit.clone()))
        .unwrap();
    assert_eq!(plan.kind, JobKind::SetTimes);
    assert_eq!(plan.summary(), "Change the times of 4 items");
    let order: Vec<_> = plan.steps.iter().map(|s| s.subject().clone()).collect();
    assert_eq!(
        order,
        vec![p("/t/one.txt"), p("/t/sub/two.txt"), p("/t/sub"), p("/t")],
        "each folder after its contents"
    );
    assert_eq!(rows(&plan)[1], "Modified time to 2026-09-21 15:13");
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    for path in ["/t", "/t/sub", "/t/one.txt", "/t/sub/two.txt"] {
        assert_eq!(
            fake.entry(&p(path)).unwrap().modified,
            Some(t(3600)),
            "{path}"
        );
    }
    let undo = engine.plan_undo(&report.undo, JobId::new(30)).unwrap();
    engine.run(&undo, &JobControl::new(), &Script::default());
    assert_eq!(fake.entry(&p("/t/sub")).unwrap().modified, Some(t(0)));
    // A place that cannot keep a created time says so, and sets the rest.
    fake.switches_mut().no_created_time = true;
    let created = engine
        .plan_attributes(
            &request(JobKind::SetTimes, &["/t/one.txt"]).changing(
                AttributeEdit::new(Recursion::ThisItem)
                    .with_created(t(7))
                    .with_modified(t(9)),
            ),
        )
        .unwrap();
    assert!(
        rows(&created).contains(&"Created time not set: this drive cannot keep one.".to_owned())
    );
    assert!(matches!(
        &created.steps[0],
        Step::SetAttributes { change, .. } if change.created.is_none() && change.modified == Some(t(9))
    ));
}

#[test]
fn windows_attributes_change_in_words() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/w/report.pdf"), b"w");
    fake.set_attributes_now(&p("/w/report.pdf"), Attributes::Windows(0x20));
    let engine = engine(&fake);
    let edit = AttributeEdit::new(Recursion::ThisItem).with_windows(WindowsChange {
        on: crate::access::HIDDEN,
        off: 0,
    });
    let plan = engine
        .plan_attributes(
            &request(JobKind::SetAttributes, &["/w/report.pdf"]).changing(edit.clone()),
        )
        .unwrap();
    assert_eq!(rows(&plan)[1], "Turns on hidden");
    engine.run(&plan, &JobControl::new(), &Script::default());
    assert_eq!(
        fake.entry(&p("/w/report.pdf")).unwrap().attributes,
        Some(Attributes::Windows(0x22))
    );
}

#[test]
fn a_copy_keeps_a_folders_time_after_its_contents_with_keep_dates() {
    for keep in [true, false] {
        let fake = FakeProvider::new();
        fake.switches_mut().now = Some(t(0));
        fake.add_file(&p("/src/f/inner.txt"), b"x");
        fake.set_modified(&p("/src/f"), t(-500));
        fake.switches_mut().now = Some(t(9_000));
        fake.add_folder(&p("/dst"));
        let options = crate::options::EngineOptions {
            keep_dates: keep,
            ..crate::options::EngineOptions::default()
        };
        let engine = engine(&fake).with_options(options);
        let plan = engine
            .plan(&JobRequest::new(JobId::new(31), JobKind::Copy, vec![p("/src/f")]).to(p("/dst")))
            .unwrap();
        engine.run(&plan, &JobControl::new(), &Script::default());
        let folder = fake.entry(&p("/dst/f")).unwrap().modified;
        if keep {
            assert_eq!(folder, Some(t(-500)), "the folder's own time, set last");
            assert_eq!(
                fake.entry(&p("/dst/f/inner.txt")).unwrap().modified,
                Some(t(0))
            );
        } else {
            assert_eq!(folder, Some(t(9_000)), "the copy's own moment");
        }
    }
}
