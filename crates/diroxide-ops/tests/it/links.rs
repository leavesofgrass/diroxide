//! Spike L1 (research/alpha2-plan.md 4.14): links on this machine's own
//! disk, under the target's temporary folder. A file link and a folder
//! link are made relative to their folder; where the account lacks the
//! privilege (Windows without Developer Mode) the failure is the
//! catalog's sentence; where links can be made, the folder holding both
//! is moved and the links still resolve.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use diroxide_core::{JobId, JobKind, JobRequest, Place};
use diroxide_ops::links::{LinkForm, NEEDS_DEVELOPER_MODE};
use diroxide_ops::{Engine, JobControl};
use diroxide_vfs::{LinkKind, Registry};

use crate::support::Answers;

fn place_of(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

#[test]
fn relative_links_resolve_after_their_folder_moves_or_say_the_privilege() {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let project = root.path().join("proj");
    fs::create_dir_all(project.join("data").join("sub")).unwrap();
    fs::create_dir_all(project.join("links")).unwrap();
    fs::write(project.join("data").join("x.txt"), b"linked").unwrap();
    fs::write(project.join("data").join("sub").join("inner.txt"), b"in").unwrap();
    let engine = Engine::new(Arc::new(Registry::with_local()));
    let request = JobRequest::new(
        JobId::new(1),
        JobKind::Link,
        vec![
            place_of(&project.join("data").join("x.txt")),
            place_of(&project.join("data").join("sub")),
        ],
    )
    .to(place_of(&project.join("links")));
    let plan = engine
        .plan_links_as(&request, LinkKind::Symbolic, LinkForm::Relative)
        .unwrap();
    let report = engine.run(&plan, &JobControl::new(), &Answers::default());
    if let Some((_, why)) = report.failed.first() {
        assert_eq!(why, NEEDS_DEVELOPER_MODE, "{:?}", report.failed);
        assert_eq!(report.failed.len(), 2, "both links say it");
        println!("L1: no privilege for symbolic links here: {why}");
        return;
    }
    println!("L1: symbolic links can be made here (Developer Mode or administrator)");
    let moved = root.path().join("moved");
    fs::rename(&project, &moved).unwrap();
    let file_link = moved.join("links").join("x.txt");
    assert!(
        fs::symlink_metadata(&file_link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let target = fs::read_link(&file_link).unwrap();
    assert!(target.is_relative(), "{}", target.display());
    assert_eq!(
        fs::read(&file_link).unwrap(),
        b"linked",
        "resolves after the move"
    );
    let folder_link = moved.join("links").join("sub");
    assert_eq!(fs::read(folder_link.join("inner.txt")).unwrap(), b"in");
    println!("L1: the relative links resolve after the folder moved");
}
