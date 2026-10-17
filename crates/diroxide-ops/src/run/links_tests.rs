//! Link creation on the fake provider: relative and absolute targets, a
//! link's target changed in one undoable step, the privilege said in
//! words, and the refusals before planning.

use diroxide_core::{EntryKind, JobId, JobKind, JobRequest};
use diroxide_vfs::{LinkKind, VfsError};

use crate::engine::JobControl;
use crate::error::OpsError;
use crate::fake::{CallFault, FakeCall, FakeProvider};
use crate::links::{LinkForm, NEEDS_DEVELOPER_MODE};
use crate::plan::Step;
use crate::testing::{Script, engine, p};

fn tree() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.add_file(&p("/proj/data/x.txt"), b"x");
    fake.add_folder(&p("/proj/data/sub"));
    fake.add_folder(&p("/proj/links"));
    fake
}

fn link_request(sources: &[&str]) -> JobRequest {
    JobRequest::new(
        JobId::new(26),
        JobKind::Link,
        sources.iter().map(|s| p(s)).collect(),
    )
    .to(p("/proj/links"))
}

fn sep(text: &str) -> String {
    if cfg!(windows) {
        text.replace('/', "\\")
    } else {
        text.to_owned()
    }
}

#[test]
fn a_relative_link_points_the_way_from_its_folder() {
    let fake = tree();
    let engine = engine(&fake);
    let plan = engine
        .plan_links_as(
            &link_request(&["/proj/data/x.txt", "/proj/data/sub"]),
            LinkKind::Symbolic,
            LinkForm::Relative,
        )
        .unwrap();
    let targets: Vec<String> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Link { request, .. } => Some(request.target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(targets, vec![sep("../data/x.txt"), sep("../data/sub")]);
    assert_eq!(
        plan.summary(),
        format!("Make 2 links to {}", p("/proj/links"))
    );
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let made = fake.entry(&p("/proj/links/sub")).unwrap();
    assert_eq!(made.kind, EntryKind::Link);
    assert_eq!(made.link.unwrap().points_to, Some(EntryKind::Folder));
    let absolute = engine
        .plan_links_as(
            &link_request(&["/proj/data/x.txt"]),
            LinkKind::Symbolic,
            LinkForm::Absolute,
        )
        .unwrap();
    assert!(matches!(
        &absolute.steps[0],
        Step::Link { request, .. } if request.target == p("/proj/data/x.txt").to_string()
    ));
    let junction = engine
        .plan_links_as(
            &link_request(&["/proj/data/sub"]),
            LinkKind::Junction,
            LinkForm::Relative,
        )
        .unwrap();
    assert_eq!(
        junction.note.as_deref(),
        Some("A junction points by the full path.")
    );
}

#[test]
fn changing_a_links_target_is_one_step_and_undo_brings_the_old_one_back() {
    let fake = tree();
    fake.add_link(&p("/proj/links/latest"), "old-target", false);
    let engine = engine(&fake);
    let plan = engine
        .plan_relink(
            JobId::new(27),
            &p("/proj/links/latest"),
            &p("/proj/data/x.txt"),
            LinkForm::Relative,
        )
        .unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.summary(), "Change where 1 link points");
    assert!(plan.listing()[0].starts_with("point the link "));
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let now = fake.entry(&p("/proj/links/latest")).unwrap();
    assert_eq!(now.link.unwrap().target, sep("../data/x.txt"));
    assert!(
        fake.in_trash(&p("/proj/links/latest")),
        "the old link is kept"
    );
    let leftovers: Vec<String> = fake
        .snapshot()
        .keys()
        .filter(|key| key.contains("diro-link"))
        .cloned()
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    let undo = engine.plan_undo(&report.undo, JobId::new(28)).unwrap();
    let undone = engine.run(&undo, &JobControl::new(), &Script::default());
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    let back = fake.entry(&p("/proj/links/latest")).unwrap();
    assert_eq!(back.link.unwrap().target, "old-target");
}

/// W2c's request 2 (P2b): where the place renames over a link in one
/// step, the new link takes the name at once, nothing goes to the trash,
/// and undo points the link back the same way; a place with no trash may
/// then change a link too.
#[test]
fn a_place_that_replaces_in_one_step_changes_a_link_atomically() {
    let fake = tree();
    fake.switches_mut().replace_rename = true;
    fake.switches_mut().no_trash = true;
    fake.add_link(&p("/proj/links/latest"), "old-target", false);
    let engine = engine(&fake);
    let plan = engine
        .plan_relink(
            JobId::new(29),
            &p("/proj/links/latest"),
            &p("/proj/data/x.txt"),
            LinkForm::Absolute,
        )
        .unwrap();
    assert_eq!(plan.steps.len(), 1);
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let now = fake.entry(&p("/proj/links/latest")).unwrap();
    assert_eq!(now.link.unwrap().target, p("/proj/data/x.txt").to_string());
    assert!(!fake.in_trash(&p("/proj/links/latest")), "nothing trashed");
    assert!(report.undo.trashed.is_empty());
    assert!(report.undo.created.is_empty());
    assert_eq!(report.undo.relinked.len(), 1);
    assert_eq!(report.undo.relinked[0].target, "old-target");
    let leftovers: Vec<String> = fake
        .snapshot()
        .keys()
        .filter(|key| key.contains("diro-link"))
        .cloned()
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // The record travels as JSON, as the journal writes it.
    let text = serde_json::to_string(&report.undo).unwrap();
    let read: crate::UndoRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(read, report.undo);
    let undo = engine.plan_undo(&report.undo, JobId::new(30)).unwrap();
    assert!(matches!(undo.steps[0], Step::Relink { .. }));
    let undone = engine.run(&undo, &JobControl::new(), &Script::default());
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    let back = fake.entry(&p("/proj/links/latest")).unwrap();
    assert_eq!(back.link.unwrap().target, "old-target");
    // Neither a trash nor a one-step rename: refused before planning.
    fake.switches_mut().replace_rename = false;
    assert_eq!(
        engine.plan_relink(
            JobId::new(31),
            &p("/proj/links/latest"),
            &p("/proj/data/x.txt"),
            LinkForm::Absolute
        ),
        Err(OpsError::CannotLink(
            "Cannot change a link here: no trash keeps the old one".into()
        ))
    );
}

/// A refused one-step rename (a folder link on Windows) falls back to the
/// trash's way, and the old link is kept there.
#[test]
fn a_refused_one_step_rename_falls_back_to_the_trash() {
    let fake = tree();
    fake.switches_mut().replace_rename = true;
    fake.add_link(&p("/proj/links/latest"), "old-target", false);
    let engine = engine(&fake);
    let plan = engine
        .plan_relink(
            JobId::new(32),
            &p("/proj/links/latest"),
            &p("/proj/data/x.txt"),
            LinkForm::Absolute,
        )
        .unwrap();
    fake.switches_mut().call_faults.push(
        CallFault::new(
            FakeCall::Rename,
            p("/proj/links/diro-link-32-1"),
            VfsError::PermissionDenied,
        )
        .times(1),
    );
    let report = engine.run(&plan, &JobControl::new(), &Script::default());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(
        fake.in_trash(&p("/proj/links/latest")),
        "the old link is kept"
    );
    assert!(report.undo.relinked.is_empty());
    let now = fake.entry(&p("/proj/links/latest")).unwrap();
    assert_eq!(now.link.unwrap().target, p("/proj/data/x.txt").to_string());
}

#[test]
fn a_missing_privilege_is_said_as_the_catalogs_sentence() {
    let fake = tree();
    fake.switches_mut().call_faults.push(CallFault::new(
        FakeCall::CreateLink,
        p("/proj/links/x.txt"),
        VfsError::NeedsPrivilege {
            what: "making a symbolic link".into(),
        },
    ));
    let engine = engine(&fake);
    let plan = engine.plan(&link_request(&["/proj/data/x.txt"])).unwrap();
    let sink = Script::default();
    let report = engine.run(&plan, &JobControl::new(), &sink);
    assert_eq!(report.failed[0].1, NEEDS_DEVELOPER_MODE);
    assert_eq!(report.failed[0].1, "Cannot link: needs developer mode.");
    assert!(
        sink.failed.lock().unwrap().is_empty(),
        "nothing to retry: not asked"
    );
    assert!(!fake.exists(&p("/proj/links/x.txt")));
}

#[test]
fn places_without_links_or_trash_refuse_before_planning() {
    let fake = tree();
    fake.add_link(&p("/proj/links/latest"), "old", false);
    fake.switches_mut().no_trash = true;
    let engine = engine(&fake);
    assert_eq!(
        engine.plan_relink(
            JobId::new(1),
            &p("/proj/links/latest"),
            &p("/proj/data/x.txt"),
            LinkForm::Absolute
        ),
        Err(OpsError::CannotLink(
            "Cannot change a link here: no trash keeps the old one".into()
        ))
    );
    fake.switches_mut().no_trash = false;
    assert!(matches!(
        engine.plan_relink(
            JobId::new(1),
            &p("/proj/data/x.txt"),
            &p("/proj/data/sub"),
            LinkForm::Absolute
        ),
        Err(OpsError::NameRefused(line)) if line == "Not a link: x.txt"
    ));
    fake.switches_mut().no_links = true;
    assert_eq!(
        engine.plan_links_as(
            &link_request(&["/proj/data/x.txt"]),
            LinkKind::Symbolic,
            LinkForm::Relative
        ),
        Err(OpsError::CannotLink(
            "Cannot link here: this place holds no links".into()
        ))
    );
}
