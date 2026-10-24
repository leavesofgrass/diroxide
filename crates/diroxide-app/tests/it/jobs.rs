//! The transfer queue and the Jobs list (lane A2, R25 commit 79), driven
//! without a terminal: copies to a remote place wait their turn two to a
//! host, the jobs line says the count first, and the Jobs list reorders,
//! pauses and cancels, every row and line its state first. The place is
//! the fake provider behind a gate; nothing touches the network.

use std::sync::Arc;

use diroxide_app::commands::ids;
use diroxide_app::dialog::Dialog;
use diroxide_core::{Place, Side};
use diroxide_ops::FakeProvider;
use diroxide_vfs::Registry;

use crate::held::{Held, command_held, key_held, key_until, said_start};
use crate::support::{Session, settings};

fn local(path: &str) -> Place {
    if cfg!(windows) {
        Place::parse(&format!("D:{}", path.replace('/', "\\"))).unwrap()
    } else {
        Place::parse(path).unwrap()
    }
}

fn remote(path: &str) -> Place {
    Place::parse(&format!("sftp://build{path}")).unwrap()
}

/// Four files here, a folder on `build` there, the reads held.
fn world() -> (Held, Session) {
    let fake = FakeProvider::new();
    for n in 1..=4 {
        fake.add_file(&local(&format!("/a/f{n}.txt")), b"data");
    }
    fake.add_folder(&remote("/srv"));
    let held = Held::new(fake);
    let mut registry = Registry::new();
    registry.set_local(Arc::new(held.clone()));
    registry.register("sftp", Arc::new(held.clone()));
    let session = Session::start(local("/a"), remote("/srv"), registry, None, settings());
    held.close();
    (held, session)
}

/// Copies one file to the other panel, answering the question with y.
fn copy(s: &mut Session, name: &str) -> Vec<String> {
    let side = s.app.active_side();
    assert!(s.app.focus_name(side, name), "{name}");
    let said = key_until(s, "f5", |s, _| {
        matches!(s.dialog(), Some(Dialog::Confirm { .. }))
    });
    assert!(
        matches!(s.dialog(), Some(Dialog::Confirm { .. })),
        "{said:?} {:?}",
        s.dialog()
    );
    let queued = format!("Queued: copy {name} to srv");
    let wanted = queued.clone();
    let said = key_until(s, "y", move |_, said| {
        said_start(said, "Copy started: 1 item to srv") || said_start(said, &wanted)
    });
    assert!(
        said_start(&said, "Copy started: 1 item to srv") || said_start(&said, &queued),
        "{said:?}"
    );
    said
}

/// The labels of the top dialog's list.
fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

/// Moves the top list's focus down until its row starts with `start`.
fn focus_row(s: &mut Session, start: &str) {
    key_held(s, "home");
    for _ in 0..20 {
        let focused = match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.label.clone()),
            _ => None,
        };
        if focused.as_deref().is_some_and(|l| l.starts_with(start)) {
            return;
        }
        key_held(s, "down");
    }
    panic!("no row starts with {start}: {:?}", rows(s));
}

#[test]
fn two_copies_run_on_a_host_the_rest_wait_and_the_list_reorders_and_cancels() {
    let (held, mut s) = world();
    let said = copy(&mut s, "f1.txt");
    assert!(said_start(&said, "Copy started: 1 item to srv"), "{said:?}");
    copy(&mut s, "f2.txt");
    let said = copy(&mut s, "f3.txt");
    assert!(said_start(&said, "Queued: copy f3.txt to srv"), "{said:?}");
    let said = copy(&mut s, "f4.txt");
    assert!(said_start(&said, "Queued: copy f4.txt to srv"), "{said:?}");
    // The count first, then the front job's sentence.
    let line = s.app.jobs().line();
    assert!(line.starts_with("4 jobs, 2 waiting: "), "{line}");

    // Say Status gives every job, the waiting ones in their rows' words.
    let said = key_held(&mut s, "ctrl+d");
    assert!(
        said.iter()
            .any(|l| l.contains("Waiting 1 of 2: copy f3.txt to srv")),
        "{said:?}"
    );

    // The Jobs list: each group under its heading, the state first.
    command_held(&mut s, ids::JOBS_LIST);
    let labels = rows(&s);
    assert_eq!(labels[0], "Running, 2 jobs");
    assert_eq!(
        labels[3..],
        [
            "Waiting, 2 jobs",
            "Waiting 1 of 2: copy f3.txt to srv",
            "Waiting 2 of 2: copy f4.txt to srv",
        ]
    );
    for label in &labels {
        let head = label.split(':').next().unwrap();
        assert!(diroxide_core::format::within_cells(label, head), "{label}");
    }

    // Enter opens the row's actions; Move to the front says its place.
    focus_row(&mut s, "Waiting 2 of 2");
    key_held(&mut s, "enter");
    let actions = rows(&s);
    assert_eq!(
        actions[..5],
        [
            "Run now",
            "Move to the front",
            "Move up",
            "Move down",
            "Move to the end"
        ]
    );
    assert!(actions.contains(&"Cancel this job".to_owned()));
    key_held(&mut s, "m");
    let said = key_held(&mut s, "enter");
    assert!(
        said.contains(&"Waiting 1 of 2: copy f4.txt to srv".to_owned()),
        "{said:?}"
    );
    assert_eq!(rows(&s)[4], "Waiting 1 of 2: copy f4.txt to srv");

    // Delete cancels a waiting job without a question.
    focus_row(&mut s, "Waiting 2 of 2: copy f3.txt");
    let said = key_held(&mut s, "delete");
    assert!(
        said.contains(&"Copy cancelled: f3.txt to srv, 0 done.".to_owned()),
        "{said:?}"
    );
    assert!(
        rows(&s).contains(&"Waiting, 1 job".to_owned()),
        "{:?}",
        rows(&s)
    );
    assert!(rows(&s).contains(&"Cancelled: copy f3.txt to srv, 0 done".to_owned()));

    // A running one asks first, starting on No.
    focus_row(&mut s, "Running");
    key_held(&mut s, "down");
    key_held(&mut s, "delete");
    let question = s.dialog_title().unwrap();
    assert!(question.starts_with("Cancel copy f"), "{question}");
    assert!(
        question.ends_with("No is the default. y or n"),
        "{question}"
    );
    let said = key_held(&mut s, "n");
    assert!(said.contains(&"Not cancelled.".to_owned()), "{said:?}");

    // Pause a running job from its actions.
    focus_row(&mut s, "Running");
    key_held(&mut s, "down");
    key_held(&mut s, "enter");
    assert_eq!(rows(&s)[0], "Pause");
    let said = key_held(&mut s, "enter");
    assert!(said_start(&said, "Pausing copy f"), "{said:?}");
    key_held(&mut s, "escape");
    assert!(s.app.dialog().is_none());

    // The gate opens: the paused one stops at its chunk, the rest run, and
    // the waiting one starts when a slot frees.
    held.open();
    let said = command_held(&mut s, ids::JOBS_RESUME_ALL);
    assert!(said.contains(&"All jobs go on.".to_owned()), "{said:?}");
    let effects = Vec::new();
    // f1, f2 and f4 copied, f3 cancelled before it began.
    let said = crate::held::drive(&mut s, effects, |s, _| crate::held::settled_after(s, 4));
    assert!(
        s.app.jobs().is_empty(),
        "{:?} {said:?}",
        s.app.jobs().line()
    );
    let all: Vec<String> = s.said.iter().map(|m| m.text.clone()).collect();
    assert!(
        all.iter()
            .filter(|l| l.starts_with("Copy started: 1 item to srv"))
            .count()
            == 3,
        "{all:?}"
    );
    let fake = &held.fake;
    for name in ["f1.txt", "f2.txt", "f4.txt"] {
        assert!(fake.exists(&remote(&format!("/srv/{name}"))), "{name}");
    }
    assert!(!fake.exists(&remote("/srv/f3.txt")));
    assert_eq!(s.app.panel(Side::Left).place(), &local("/a"));
}

#[test]
fn escape_names_the_job_in_front_when_others_run_beside_it() {
    let (held, mut s) = world();
    copy(&mut s, "f1.txt");
    copy(&mut s, "f2.txt");
    let line = s.app.jobs().line();
    assert!(line.starts_with("2 jobs: "), "{line}");
    let said = key_held(&mut s, "escape");
    assert!(
        said.contains(&"Cancelling: copy f2.txt to srv".to_owned()),
        "{said:?}"
    );
    let all = |s: &Session| -> Vec<String> { s.said.iter().map(|m| m.text.clone()).collect() };
    crate::held::drive(&mut s, Vec::new(), |s, _| {
        all(s).contains(&"Copy cancelled: f2.txt to srv, 0 done.".to_owned())
    });
    assert!(
        all(&s).contains(&"Copy cancelled: f2.txt to srv, 0 done.".to_owned()),
        "{:?}",
        all(&s)
    );
    held.open();
    crate::held::drive(&mut s, Vec::new(), |s, _| crate::held::settled_after(s, 2));
    assert!(held.fake.exists(&remote("/srv/f1.txt")));
    assert!(!held.fake.exists(&remote("/srv/f2.txt")));
}

#[test]
fn a_job_with_no_jobs_running_says_the_list_is_empty() {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/a"));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake));
    let mut s = Session::start(local("/a"), local("/a"), registry, None, settings());
    let said = s.command(ids::JOBS_LIST);
    assert_eq!(said, ["No jobs have run yet."]);
    assert!(s.app.dialog().is_none());
}
