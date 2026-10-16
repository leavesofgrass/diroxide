//! The dialog contract (R23, the owner's answer 75, ADR-0009): the safe
//! choice focused when a dialog is made, the default said in the
//! question, confirming keys ignored within the settle time, every path
//! listed with hidden characters spelled out, the masked prompt's canary,
//! and the jobs line changing no more often than the setting says.

use std::sync::Arc;

use diroxide_app::{AppEvent, AskId, Dialog, Question, Reply, Request};
use diroxide_core::{
    EntryKind, ErrorAction, JobId, JobKind, JobState, Place, SecretKind, SecretRequest, Side,
};
use diroxide_ops::{FakeProvider, ItemError, JobProgress};
use diroxide_vfs::{Registry, VfsError};

use crate::support::{Session, Setup, fixture, place, registry_with_trash, settings, temp};

fn p(text: &str) -> Place {
    if cfg!(windows) {
        Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
    } else {
        Place::parse(text).unwrap()
    }
}

fn fake_session(fake: &FakeProvider, left: &str, right: &str) -> Session {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    Session::open(p(left), p(right), registry, Setup::default())
}

#[test]
fn a_confirming_key_within_the_settle_time_is_ignored() {
    let dir = temp();
    let work = fixture(dir.path());
    let (registry, trash) = registry_with_trash(&dir.path().join("bin"));
    let mut slow = settings();
    slow.dialog_settle_ms = 250;
    let mut s = Session::open(
        place(&work),
        place(&work.join("Backup")),
        registry,
        Setup {
            settings: slow,
            ..Setup::default()
        },
    );
    s.focus("notes.txt");
    s.key("f8");
    assert!(s.dialog().is_some());
    assert!(s.key("y").is_empty(), "the held key cannot answer it");
    assert!(s.dialog().is_some());
    assert_eq!(trash.count(), 0);
    s.clock.advance(1);
    s.key("y");
    assert!(s.dialog().is_none());
    assert_eq!(trash.count(), 1);
}

#[test]
fn every_question_starts_on_its_safe_choice_and_says_the_default() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x.txt"), b"x");
    fake.add_folder(&p("/b"));
    let mut s = fake_session(&fake, "/a", "/b");
    // A confirmation: Enter is No.
    s.key("f8");
    let title = s.dialog_title().unwrap();
    assert!(title.ends_with("? No is the default. y or n"), "{title}");
    s.key("enter");
    assert!(s.dialog().is_none());
    assert!(fake.exists(&p("/a/x.txt")));
    // An item's error: Skip is focused, Escape is Skip.
    let error = ItemError {
        kind: JobKind::Copy,
        place: p("/a/x.txt"),
        error: VfsError::InUse,
        attempt: 1,
    };
    let effects = s.app.handle(AppEvent::Asks {
        ask: AskId::new(9),
        question: Question::Error {
            job: JobId::new(40),
            error: Box::new(error),
        },
    });
    let said = s.run(effects);
    assert!(said[0].starts_with("Could not copy x.txt: "), "{said:?}");
    let Some(Dialog::Question { choices, .. }) = s.dialog() else {
        panic!("no question");
    };
    assert_eq!(choices.focused().unwrap().label, "Skip");
    s.app.take_requests();
    s.key_quiet("escape");
    let requests = s.app.take_requests();
    assert!(
        requests.iter().any(|r| matches!(
            r,
            Request::Answer {
                reply: Reply::Error(ErrorAction::Skip),
                ..
            }
        )),
        "{requests:?}"
    );
}

#[test]
fn a_hostile_name_is_spelled_out_in_the_listing_and_never_drawn_raw() {
    let fake = FakeProvider::new();
    let hostile = "evil\u{1b}[2J\u{202e}txt.exe";
    fake.add_file(&p(&format!("/a/{hostile}")), b"x");
    fake.add_folder(&p("/b"));
    let mut s = fake_session(&fake, "/a", "/b");
    let row = &s.app.active_panel().list().rows()[0];
    assert!(!row.label.contains('\u{1b}') && !row.label.contains('\u{202e}'));
    assert!(row.spoken().contains("name needs care"), "{}", row.spoken());
    assert!(!s.app.title_line().contains('\u{1b}'));
    s.key("shift+delete");
    let Some(Dialog::Confirm { listing, .. }) = s.dialog() else {
        panic!("no question");
    };
    assert!(
        listing
            .iter()
            .any(|l| l.contains("[U+001B]") && l.contains("[U+202E]")),
        "{listing:?}"
    );
    for message in &s.said {
        assert!(!message.text.contains('\u{1b}'), "{}", message.text);
    }
}

#[test]
fn the_masked_prompt_never_lets_its_canary_out() {
    let fake = FakeProvider::new();
    fake.add_folder(&p("/a"));
    let mut s = fake_session(&fake, "/a", "/a");
    let request = SecretRequest::new(SecretKind::ArchivePassword, "course.zip");
    let effects = s.app.handle(AppEvent::Asks {
        ask: AskId::new(3),
        question: Question::Secret { job: None, request },
    });
    let said = s.run(effects);
    assert_eq!(said, ["Password for course.zip (hidden):"]);
    let canary = "canary-7f3a-not-a-password";
    s.type_text(canary);
    assert_eq!(
        s.dialog().unwrap().masked_text(false).unwrap(),
        "*".repeat(canary.len())
    );
    s.key("ctrl+d");
    let said = s.key("alt+s");
    assert_eq!(
        said,
        ["Showing the password. Anyone who can see or hear your screen can read it."]
    );
    assert_eq!(s.key("alt+s"), ["Hidden."]);
    s.key_quiet("enter");
    let requests = s.app.take_requests();
    let answered = requests.iter().find_map(|r| match r {
        Request::Answer {
            reply: Reply::Secret(Some(secret)),
            ..
        } => Some(secret.clone()),
        _ => None,
    });
    assert_eq!(answered.unwrap().expose(), canary);
    let everything = format!(
        "{:?} {:?} {:?} {}",
        s.said,
        requests,
        s.app,
        s.app.status_line()
    );
    assert!(!everything.contains("canary"), "{everything}");
}

#[test]
fn the_jobs_line_changes_no_more_often_than_the_setting_says() {
    let fake = FakeProvider::new();
    fake.add_folder(&p("/a"));
    let mut s = fake_session(&fake, "/a", "/a");
    assert!(s.app.screen_reader());
    let job = JobId::new(77);
    let mut progress = JobProgress::start(JobKind::Copy, 100, None);
    progress.state = JobState::Running;
    let mut lines = vec![s.app.jobs().line()];
    for second in 0..30 {
        progress.items_done = second * 3;
        let effects = s.app.handle(AppEvent::JobProgress {
            job,
            progress: progress.clone(),
        });
        s.run_quiet(effects);
        let line = s.app.jobs().line();
        if lines.last() != Some(&line) {
            lines.push(line);
        }
        s.clock.advance(1);
    }
    // The first update starts the job (a change of state); then one
    // change every ten seconds: at 10 and at 20.
    assert_eq!(lines.len(), 4, "{lines:?}");
    s.app.set_screen_reader(false);
    let before = lines.len();
    for _ in 0..4 {
        progress.items_done += 1;
        let effects = s.app.handle(AppEvent::JobProgress {
            job,
            progress: progress.clone(),
        });
        s.run_quiet(effects);
        let line = s.app.jobs().line();
        if lines.last() != Some(&line) {
            lines.push(line);
        }
        s.clock.advance(1);
    }
    assert_eq!(
        lines.len(),
        before + 4,
        "twice a second at most when sighted"
    );
    let _ = (Side::Left, EntryKind::File);
}
