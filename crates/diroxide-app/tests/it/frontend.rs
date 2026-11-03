//! What a frontend reaches beside keys and commands (lane A5): the job
//! history's dialogs. Every place is the fake provider's; nothing touches
//! a disk, a trash or the network.

use std::sync::Arc;

use diroxide_app::commands::ids;
use diroxide_app::dialog::Dialog;
use diroxide_core::{Place, Side};
use diroxide_ops::FakeProvider;
use diroxide_vfs::Registry;

use crate::support::{Session, settings};

fn local(path: &str) -> Place {
    if cfg!(windows) {
        Place::parse(&format!("D:{}", path.replace('/', "\\"))).unwrap()
    } else {
        Place::parse(path).unwrap()
    }
}

/// Two files in one folder of the fake provider, both panels on it.
fn world() -> (FakeProvider, Session) {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/a"));
    fake.add_file(&local("/a/f1.txt"), b"one");
    fake.add_file(&local("/a/f2.txt"), b"two");
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    let session = Session::start(local("/a"), local("/a"), registry, None, settings());
    (fake, session)
}

/// The labels of the top dialog's list.
fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

/// Job history is the Jobs list's Finished group (lane A2), so the
/// command opens the Jobs list there; a finished job's Show what failed
/// opens its items. The two are the history's only dialogs.
#[test]
fn job_history_opens_the_jobs_list_and_show_what_failed_the_items() {
    let (fake, mut s) = world();
    assert_eq!(s.command(ids::JOB_HISTORY), ["No jobs have run yet."]);
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    s.key("f8");
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Moved 1 item to the trash.")),
        "{said:?}"
    );
    assert_eq!(fake.trashed(), 1);

    let said = s.command(ids::JOB_HISTORY);
    assert!(said[0].starts_with("Jobs, Done: trash f1.txt"), "{said:?}");
    assert_eq!(s.dialog_title().as_deref(), Some("Jobs"));
    assert_eq!(rows(&s), ["Finished, 1 job", "Done: trash f1.txt"]);

    // Enter opens the job's actions, the first of them Show what failed.
    s.key("enter");
    assert_eq!(s.dialog_title().as_deref(), Some("Done: trash f1.txt"));
    assert_eq!(rows(&s)[0], "Show what failed");
    let said = s.key("enter");
    let title = s.dialog_title().unwrap();
    assert!(title.starts_with("Move to the trash finished"), "{title}");
    assert_eq!(rows(&s), ["Nothing failed or was skipped."]);
    assert!(
        said[0].starts_with("Move to the trash finished"),
        "{said:?}"
    );

    // Escape walks back to the Jobs list, then to the panel.
    s.key("escape");
    assert_eq!(s.dialog_title().as_deref(), Some("Jobs"));
    s.key("escape");
    assert!(s.app.dialog().is_none(), "{:?}", s.dialog_title());
}
