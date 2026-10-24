//! The queue's settings, messages, the quit rule, the connection question
//! and Retry the failed items (lane A2, R25 commit 80), driven without a
//! terminal on the fake provider behind a gate; nothing touches the
//! network.

use std::sync::Arc;

use diroxide_app::AppEvent;
use diroxide_app::commands::ids;
use diroxide_app::dialog::Dialog;
use diroxide_app::request::{AskId, Question, Reply, Request};
use diroxide_core::{JobId, JobKind, Place};
use diroxide_ops::{ConnectionAnswer, ConnectionQuestion, FakeProvider, Waiter};
use diroxide_vfs::Registry;

use crate::held::{Held, command_held, drive, key_held, key_until, said_start, settled_after};
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

fn all(s: &Session) -> Vec<String> {
    s.said.iter().map(|m| m.text.clone()).collect()
}

/// Files here and on `build`, a session over both with these settings.
fn world_with(settings: diroxide_config::Settings, left: Place, right: Place) -> (Held, Session) {
    let fake = FakeProvider::new();
    for n in 1..=3 {
        fake.add_file(&local(&format!("/a/f{n}.txt")), b"data");
        fake.add_file(&remote(&format!("/srv/r{n}.bin")), &[7; 64]);
    }
    fake.add_folder(&local("/b"));
    let held = Held::new(fake);
    let mut registry = Registry::new();
    registry.set_local(Arc::new(held.clone()));
    registry.register("sftp", Arc::new(held.clone()));
    let session = Session::start(left, right, registry, None, settings);
    (held, session)
}

/// Copies the focused file to the other panel, answering y.
fn copy(s: &mut Session, name: &str) -> Vec<String> {
    let side = s.app.active_side();
    assert!(s.app.focus_name(side, name), "{name}");
    key_until(s, "f5", |s, _| {
        matches!(s.dialog(), Some(Dialog::Confirm { .. }))
    });
    key_until(s, "y", |_, said| {
        said_start(said, "Copy started: ") || said_start(said, "Queued: ")
    })
}

#[test]
fn the_queue_settings_are_listed_with_their_help_and_apply_at_once() {
    let (held, mut s) = world_with(settings(), local("/a"), remote("/srv"));
    command_held(&mut s, ids::SETTINGS);
    key_held(&mut s, "j");
    let said = command_held(&mut s, ids::HELP);
    assert!(
        said_start(&said, "Jobs at once per server: How many jobs run at once"),
        "{said:?}"
    );
    // Enter on a number opens its field; 1 job a server takes effect at
    // once, said with its unit in the singular.
    key_held(&mut s, "enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Jobs at once per server, a number of jobs, 1 to 8")
    );
    let effects = s.app.handle(AppEvent::Answer {
        dialog: s.app.dialog().unwrap().0,
        answer: diroxide_app::Answer::Text("1".into()),
    });
    let said = drive(&mut s, effects, |_, _| true);
    assert!(
        said.contains(&"Jobs at once per server: 1 job".to_owned()),
        "{said:?}"
    );
    assert_eq!(s.app.settings().jobs.per_host, 1);
    key_held(&mut s, "escape");
    held.close();
    copy(&mut s, "f1.txt");
    let said = copy(&mut s, "f2.txt");
    assert!(said_start(&said, "Queued: copy f2.txt to srv"), "{said:?}");

    // The speed limit, in plain units, kept as the setting.
    let said = command_held(&mut s, ids::JOBS_SPEED_LIMIT);
    assert!(said_start(&said, "Speed limit of remote jobs"), "{said:?}");
    let dialog = s.app.dialog().unwrap().0;
    let effects = s.app.handle(AppEvent::Answer {
        dialog,
        answer: diroxide_app::Answer::Text("fast".into()),
    });
    let said = drive(&mut s, effects, |_, _| true);
    assert_eq!(said, ["Not a speed: type 500 KB, 2 MB or off."]);
    let effects = s.app.handle(AppEvent::Answer {
        dialog,
        answer: diroxide_app::Answer::Text("500 KB".into()),
    });
    let said = drive(&mut s, effects, |_, _| true);
    assert!(
        said.contains(&"Speed limit: 500 KB a second".to_owned()),
        "{said:?}"
    );
    assert_eq!(s.app.settings().jobs.speed_limit_kb, 500);
    assert_eq!(
        s.app.settings().value_words("jobs.speed_limit_kb"),
        "500 KB a second"
    );
    held.open();
    drive(&mut s, Vec::new(), |s, _| settled_after(s, 2));
    assert!(s.app.jobs().is_empty(), "{}", s.app.jobs().line());
}

#[test]
fn quit_with_unfinished_jobs_says_how_many_and_asks_on_no() {
    let (held, mut s) = world_with(settings(), local("/a"), remote("/srv"));
    held.close();
    copy(&mut s, "f1.txt");
    copy(&mut s, "f2.txt");
    copy(&mut s, "f3.txt");
    let said = key_held(&mut s, "ctrl+q");
    assert_eq!(said[0], "3 jobs are not finished.");
    assert_eq!(said[1], "Quit and stop them? No is the default. y or n");
    let said = key_held(&mut s, "n");
    assert!(!s.app.wants_quit(), "{said:?}");
    held.open();
    drive(&mut s, Vec::new(), |s, _| settled_after(s, 3));
    key_held(&mut s, "ctrl+q");
    assert!(s.app.wants_quit());
}

#[test]
fn the_connection_question_waits_behind_a_dialog_and_starts_on_keep_waiting() {
    let (_held, mut s) = world_with(settings(), local("/a"), remote("/srv"));
    command_held(&mut s, ids::SETTINGS);
    let question = ConnectionQuestion {
        host: "build".into(),
        jobs: vec![Waiter {
            job: JobId::new(40),
            kind: JobKind::Copy,
            item: Some("r1.bin".into()),
        }],
    };
    let effects = s.app.handle(AppEvent::Asks {
        ask: AskId::new(77),
        question: Question::Connection {
            job: JobId::new(40),
            question,
        },
    });
    s.run_quiet(effects);
    assert_eq!(s.dialog_title().as_deref(), Some("Settings"), "it waits");
    let said = s.key_quiet("escape");
    assert!(
        said.contains(&"build is not answering.".to_owned()),
        "{said:?}"
    );
    match s.dialog() {
        Some(Dialog::Question { lines, choices }) => {
            assert_eq!(lines, &["build is not answering.", "1 job waits for it."]);
            let labels: Vec<&str> = choices.rows().iter().map(|r| r.label.as_str()).collect();
            assert_eq!(
                labels,
                ["Keep waiting", "Try again now", "Cancel these jobs"]
            );
            assert_eq!(choices.focused().unwrap().label, "Keep waiting");
        }
        other => panic!("{other:?}"),
    }
    s.app.take_requests();
    s.key_quiet("enter");
    let requests = s.app.take_requests();
    assert!(
        requests.contains(&Request::Answer {
            ask: AskId::new(77),
            reply: Reply::Connection(ConnectionAnswer::KeepWaiting),
        }),
        "{requests:?}"
    );
}

#[test]
fn a_lost_host_asks_once_and_goes_on_when_it_is_back() {
    let mut quick = settings();
    quick.jobs.network_retries = 1;
    quick.jobs.network_retry_seconds = 1;
    let (held, mut s) = world_with(quick, remote("/srv"), local("/b"));
    held.close();
    copy(&mut s, "r1.bin");
    // The link drops under the running copy.
    held.fake
        .take_offline(&Place::parse("sftp://build/").unwrap());
    held.open();
    drive(&mut s, Vec::new(), |s, _| {
        matches!(s.dialog(), Some(Dialog::Question { .. }))
    });
    let said = all(&s);
    assert!(
        said.contains(&"Copy waits for build: connection lost".to_owned()),
        "{said:?}"
    );
    assert_eq!(s.dialog_title().as_deref(), Some("build is not answering."));
    held.fake
        .bring_back(&Place::parse("sftp://build/").unwrap());
    key_held(&mut s, "down");
    key_until(&mut s, "enter", |s, _| settled_after(s, 1));
    let said = all(&s);
    assert!(
        said.contains(&"build is back. Copy goes on.".to_owned()),
        "{said:?}"
    );
    assert!(said_start(&said, "Copy finished: 1 done."), "{said:?}");
    assert!(held.fake.exists(&local("/b/r1.bin")));
}

/// No job runs or waits, and the jobs that ran hold a copy that failed:
/// what Retry the failed items needs.
fn failed_copy_ended(s: &Session) -> bool {
    s.app.jobs().is_empty()
        && s.app
            .jobs()
            .history()
            .iter()
            .any(|job| job.kind == JobKind::Copy && !job.failed.is_empty())
}

#[test]
fn retry_the_failed_items_makes_a_new_job_that_skips_what_is_there() {
    let (held, mut s) = world_with(settings(), local("/a"), remote("/srv"));
    // Two copies: one fails at its source, which is then put right.
    held.fake
        .switches_mut()
        .call_faults
        .push(diroxide_ops::CallFault::new(
            diroxide_ops::FakeCall::Read,
            local("/a/f2.txt"),
            diroxide_vfs::VfsError::PermissionDenied,
        ));
    let side = s.app.active_side();
    s.app
        .mark_names(side, &["f1.txt".into(), "f2.txt".into()], true);
    key_until(&mut s, "f5", |s, _| {
        matches!(s.dialog(), Some(Dialog::Confirm { .. }))
    });
    // The copy asks about the failed item, or (with no one to ask) ends
    // with it failed; either way the test waits for that state, not for a
    // count of keys.
    key_until(&mut s, "y", |s, _| {
        matches!(s.dialog(), Some(Dialog::Question { .. })) || failed_copy_ended(s)
    });
    if matches!(s.dialog(), Some(Dialog::Question { .. })) {
        // The error's question: Skip.
        key_until(&mut s, "escape", |s, _| failed_copy_ended(s));
    }
    drive(&mut s, Vec::new(), |s, _| failed_copy_ended(s));
    assert!(
        failed_copy_ended(&s),
        "the copy has not ended with its failure: {} {:?}",
        s.app.jobs().line(),
        s.app.jobs().history()
    );
    held.fake.switches_mut().call_faults.clear();
    let said = command_held(&mut s, ids::JOBS_RETRY_FAILED);
    assert!(
        said.iter().any(|l| l.starts_with("Copy 1 item to srv")),
        "{said:?}"
    );
    match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => {
            assert_eq!(listing[0], "Retry: what is already there is skipped");
        }
        other => panic!("{other:?}"),
    }
    key_until(&mut s, "y", |s, _| settled_after(s, 2));
    assert!(held.fake.exists(&remote("/srv/f2.txt")));
    let said = all(&s);
    assert!(said_start(&said, "Copy finished: 1 done"), "{said:?}");
}
