//! The queue saved across restarts (lane Q2), driven without a terminal:
//! the queue file in the test's own state folder follows every change,
//! a quit keeps what it stops, and the next start lists those jobs first
//! as Interrupted, with Retry and Forget, starting none of them. The
//! places are the fake provider behind a gate; nothing touches the
//! network or the owner's folders.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::dialog::Dialog;
use diroxide_config::Paths;
use diroxide_core::Place;
use diroxide_core::format::within_cells;
use diroxide_ops::FakeProvider;
use diroxide_vfs::Registry;

use crate::held::{Held, command_held, drive, key_held, key_until, said_start, settled_after};
use crate::support::{Session, settings, temp};

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

/// Three files here and a folder on `build`, a session over both with
/// this state folder.
fn world(paths: &Paths) -> (Held, Session) {
    let fake = FakeProvider::new();
    for n in 1..=3 {
        fake.add_file(&local(&format!("/a/f{n}.txt")), b"data");
    }
    fake.add_folder(&remote("/srv"));
    let held = Held::new(fake);
    let mut registry = Registry::new();
    registry.set_local(Arc::new(held.clone()));
    registry.register("sftp", Arc::new(held.clone()));
    let session = Session::start(
        local("/a"),
        remote("/srv"),
        registry,
        Some(paths),
        settings(),
    );
    (held, session)
}

fn folders() -> (tempfile::TempDir, Paths) {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    (dir, paths)
}

/// Copies one file to the other panel, answering y.
fn copy(s: &mut Session, name: &str) {
    let side = s.app.active_side();
    assert!(s.app.focus_name(side, name), "{name}");
    key_until(s, "f5", |s, _| {
        matches!(s.dialog(), Some(Dialog::Confirm { .. }))
    });
    let queued = format!("Queued: copy {name} to srv");
    key_until(s, "y", move |_, said| {
        said_start(said, "Copy started: 1 item to srv") || said_start(said, &queued)
    });
}

/// The labels of the top dialog's list.
fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

/// The label of the top list's focused row.
fn focused(s: &Session) -> String {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.label.clone()).unwrap(),
        other => panic!("not a list: {other:?}"),
    }
}

/// Moves the top list's focus down until its row starts with `start`.
fn focus_row(s: &mut Session, start: &str) {
    key_held(s, "home");
    for _ in 0..20 {
        if focused(s).starts_with(start) {
            return;
        }
        key_held(s, "down");
    }
    panic!("no row starts with {start}: {:?}", rows(s));
}

/// Waits for the queue's writer, which works off the input thread.
fn wait_until(what: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    what()
}

fn saved(file: &Path) -> String {
    fs::read_to_string(file).unwrap_or_default()
}

#[test]
fn a_quit_keeps_the_queue_and_the_next_start_lists_it_interrupted() {
    let (_dir, paths) = folders();
    let file = paths.state_dir.join("queue.json");
    {
        let (held, mut s) = world(&paths);
        held.close();
        copy(&mut s, "f1.txt");
        copy(&mut s, "f2.txt");
        copy(&mut s, "f3.txt");
        // Written as the queue changes, off the input thread.
        assert!(
            wait_until(|| saved(&file).contains("f3.txt to srv")),
            "{}",
            saved(&file)
        );
        // Pause holds the waiting one; the file says so.
        command_held(&mut s, ids::JOBS_LIST);
        focus_row(&mut s, "Waiting 1 of 1");
        key_held(&mut s, "enter");
        key_held(&mut s, "down");
        let said = key_held(&mut s, "enter");
        assert!(
            said.contains(&"Held: copy f3.txt to srv".to_owned()),
            "{said:?}"
        );
        key_held(&mut s, "escape");
        assert!(wait_until(|| saved(&file).contains("\"held\"")));
        // The quit's question keeps its words; yes stops the jobs.
        let said = key_held(&mut s, "ctrl+q");
        assert_eq!(said[0], "3 jobs are not finished.");
        key_held(&mut s, "y");
        assert!(s.app.wants_quit());
        drive(&mut s, Vec::new(), |s, _| s.app.jobs().is_empty());
    }
    let text = saved(&file);
    assert!(text.contains("\"quit\": true"), "{text}");
    for name in ["f1.txt", "f2.txt", "f3.txt"] {
        assert!(text.contains(&format!("{name} to srv")), "{name}: {text}");
    }

    // The next start: one sentence, three rows first, nothing started.
    let (held, mut s) = world(&paths);
    let said = all(&s);
    let sentence = "3 jobs were interrupted by the last quit. The Jobs list has them.";
    assert_eq!(
        said.iter().filter(|l| *l == sentence).count(),
        1,
        "{said:?}"
    );
    assert!(within_cells(sentence, "3 jobs were interrupted"));
    assert!(s.app.jobs().is_empty(), "{}", s.app.jobs().line());
    for name in ["f1.txt", "f2.txt", "f3.txt"] {
        assert!(
            !held.fake.exists(&remote(&format!("/srv/{name}"))),
            "{name}"
        );
    }
    command_held(&mut s, ids::JOBS_LIST);
    let labels = rows(&s);
    assert_eq!(
        labels,
        [
            "Interrupted, 3 jobs",
            "Interrupted: copy f1.txt to srv",
            "Interrupted: copy f2.txt to srv",
            "Interrupted: copy f3.txt to srv",
        ]
    );
    for label in &labels {
        let head = label.split(':').next().unwrap();
        assert!(within_cells(label, head), "{label}");
    }
    assert_eq!(focused(&s), "Interrupted: copy f1.txt to srv");

    // A row's actions are Retry and Forget.
    focus_row(&mut s, "Interrupted: copy f2.txt");
    key_held(&mut s, "enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Interrupted: copy f2.txt to srv")
    );
    assert_eq!(rows(&s), ["Retry", "Forget"]);
    key_held(&mut s, "down");
    let said = key_held(&mut s, "enter");
    assert!(
        said.contains(&"Forgotten: copy f2.txt to srv".to_owned()),
        "{said:?}"
    );
    assert_eq!(rows(&s)[0], "Interrupted, 2 jobs");
    // Delete forgets one too.
    focus_row(&mut s, "Interrupted: copy f3.txt");
    let said = key_held(&mut s, "delete");
    assert!(
        said.contains(&"Forgotten: copy f3.txt to srv".to_owned()),
        "{said:?}"
    );

    // Retry plans the job again and asks; it had started, so what is
    // already there is skipped. Confirmed, it runs and leaves the list.
    focus_row(&mut s, "Interrupted: copy f1.txt");
    key_held(&mut s, "enter");
    key_until(&mut s, "enter", |s, _| {
        matches!(s.dialog(), Some(Dialog::Confirm { .. }))
    });
    match s.dialog() {
        Some(Dialog::Confirm { listing, .. }) => {
            assert_eq!(listing[0], "Retry: what is already there is skipped");
        }
        other => panic!("{other:?}"),
    }
    key_until(&mut s, "y", |s, _| settled_after(s, 1));
    assert!(held.fake.exists(&remote("/srv/f1.txt")));
    assert!(s.app.jobs().interrupted().is_empty());
    // Nothing is left: the file goes.
    assert!(wait_until(|| !file.exists()), "{}", saved(&file));
    drop(s);

    // A third start has nothing to say about it.
    let (_held, s) = world(&paths);
    assert!(
        !all(&s).iter().any(|l| l.contains("interrupted")),
        "{:?}",
        all(&s)
    );
}

const MIB: u64 = 1024 * 1024;

/// Five mebibytes that differ from place to place, so a window check off
/// by a byte never matches by chance.
fn big() -> Vec<u8> {
    let mut state: u32 = 2_654_435_761;
    (0..5 * MIB)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

/// A session over a fake whose places keep part files, with this state
/// folder. The fake is the test's, so a second start finds what the first
/// left on it, as a disk would.
fn resumable_world(fake: &FakeProvider, paths: &Paths) -> (Held, Session) {
    let held = Held::new(fake.clone());
    let mut registry = Registry::new();
    registry.set_local(Arc::new(held.clone()));
    registry.register("sftp", Arc::new(held.clone()));
    let session = Session::start(
        local("/a"),
        remote("/srv"),
        registry,
        Some(paths),
        settings(),
    );
    (held, session)
}

/// The fake with a big file here and a folder on `build`, both places
/// resumable.
fn resumable_fake() -> FakeProvider {
    let fake = FakeProvider::new();
    fake.switches_mut().resume = true;
    fake.add_file(&local("/a/big.bin"), &big());
    fake.add_folder(&remote("/srv"));
    fake
}

/// Copies big.bin to `build` and holds it at two mebibytes.
fn copy_held_at_two(held: &Held, s: &mut Session) {
    held.hold_reads_at(2 * MIB);
    copy(s, "big.bin");
    drive(s, Vec::new(), |_, _| held.reached());
    assert!(held.reached(), "the copy is held in the middle of its file");
}

/// Lane A4: quit stops a running copy the way a lost connection does,
/// keeping its part file and its ledger line, so Retry at the next start
/// goes on at the offset; the quit's question keeps its words.
#[test]
fn a_copy_cut_by_a_quit_resumes_at_its_offset_on_retry() {
    let (_dir, paths) = folders();
    let fake = resumable_fake();
    let target = remote("/srv/big.bin");
    {
        let (held, mut s) = resumable_world(&fake, &paths);
        copy_held_at_two(&held, &mut s);
        let said = key_held(&mut s, "ctrl+q");
        assert_eq!(said[0], "1 job is not finished.");
        assert!(
            said.contains(&"Quit and stop them? No is the default. y or n".to_owned()),
            "{said:?}"
        );
        key_held(&mut s, "y");
        assert!(s.app.wants_quit());
        drive(&mut s, Vec::new(), |s, _| s.app.jobs().is_empty());
    }
    // The part file stays at the offset, and its ledger line with it.
    let parts = fake.unfinished();
    assert_eq!(parts.len(), 1, "{parts:?}");
    let part = Place::parse(&parts[0]).unwrap();
    assert_eq!(fake.contents(&part).unwrap().len() as u64, 2 * MIB);
    let ledger = fs::read_to_string(paths.state_dir.join("partials.json")).unwrap();
    assert!(
        ledger.contains(&format!("\"bytes\": {}", 2 * MIB)),
        "{ledger}"
    );
    assert!(!fake.exists(&target));

    // The next start lists it; Retry goes on where the quit cut it.
    let (_held, mut s) = resumable_world(&fake, &paths);
    command_held(&mut s, ids::JOBS_LIST);
    focus_row(&mut s, "Interrupted: copy big.bin");
    key_held(&mut s, "enter");
    assert_eq!(rows(&s), ["Retry", "Forget"]);
    key_until(&mut s, "enter", |s, _| {
        matches!(s.dialog(), Some(Dialog::Confirm { .. }))
    });
    let said = key_until(&mut s, "y", |s, _| settled_after(s, 1));
    let resuming = "Resuming big.bin at 2.1 MB of 5.2 MB";
    assert!(said.contains(&resuming.to_owned()), "{said:?}");
    assert!(within_cells(resuming, "Resuming big.bin"));
    assert_eq!(
        fake.contents(&target).unwrap(),
        big(),
        "equal to its source"
    );
    assert!(
        fake.unfinished().is_empty(),
        "the part file became the file"
    );
    let ledger = fs::read_to_string(paths.state_dir.join("partials.json")).unwrap();
    assert!(!ledger.contains("big.bin"), "its line is gone: {ledger}");
}

/// Cancel, unlike quit, removes the part file and its ledger line: the
/// copy cannot be resumed, and nothing of it is left.
#[test]
fn cancel_still_removes_the_part_file() {
    let (_dir, paths) = folders();
    let fake = resumable_fake();
    let (held, mut s) = resumable_world(&fake, &paths);
    copy_held_at_two(&held, &mut s);
    assert_eq!(fake.unfinished().len(), 1, "the kept write is under way");
    let said = key_until(&mut s, "escape", |s, _| settled_after(s, 1));
    assert!(said_start(&said, "Copying: cancelling"), "{said:?}");
    assert!(fake.unfinished().is_empty(), "{:?}", fake.unfinished());
    let ledger = fs::read_to_string(paths.state_dir.join("partials.json")).unwrap_or_default();
    assert!(!ledger.contains("big.bin"), "{ledger}");
    assert!(!fake.exists(&remote("/srv/big.bin")));
}

/// A queue a crash left: the last write was a change, not a quit.
#[test]
fn after_a_crash_the_sentence_says_the_session_ended_and_a_second_window_keeps_out() {
    let (_dir, paths) = folders();
    fs::create_dir_all(&paths.state_dir).unwrap();
    let file = paths.state_dir.join("queue.json");
    let request = diroxide_core::JobRequest::new(
        diroxide_core::JobId::new(7),
        diroxide_core::JobKind::Copy,
        vec![local("/a/f2.txt")],
    )
    .to(remote("/srv"));
    let queue = serde_json::json!({
        "version": 1,
        "jobs": [{
            "job": 7,
            "kind": serde_json::to_value(diroxide_core::JobKind::Copy).unwrap(),
            "state": "running",
            "started": true,
            "what": "f2.txt to srv",
            "request": serde_json::to_value(&request).unwrap(),
        }],
    });
    fs::write(&file, serde_json::to_string_pretty(&queue).unwrap()).unwrap();
    let (_held, mut s) = world(&paths);
    let said = all(&s);
    let sentence = "1 job was interrupted when the last session ended. The Jobs list has it.";
    assert!(said.contains(&sentence.to_owned()), "{said:?}");
    assert!(within_cells(sentence, "1 job was interrupted"));
    command_held(&mut s, ids::JOBS_LIST);
    assert_eq!(
        rows(&s),
        ["Interrupted, 1 job", "Interrupted: copy f2.txt to srv"]
    );

    // A second window on the same state folder, while this one runs,
    // neither lists nor writes the queue: it is this window's.
    let (_other_held, other) = world(&paths);
    assert!(
        !all(&other).iter().any(|l| l.contains("interrupted")),
        "{:?}",
        all(&other)
    );
    assert!(other.app.jobs().interrupted().is_empty());
    drop(other);
    assert!(saved(&file).contains("f2.txt to srv"), "never written over");
    drop(s);
    let (_held, s) = world(&paths);
    assert!(
        !all(&s).iter().any(|l| l.contains("interrupted")),
        "said once: {:?}",
        all(&s)
    );
    assert_eq!(s.app.jobs().interrupted().len(), 1, "kept until forgotten");
}

#[test]
fn a_damaged_queue_file_is_said_and_set_aside() {
    let (_dir, paths) = folders();
    fs::create_dir_all(&paths.state_dir).unwrap();
    let file = paths.state_dir.join("queue.json");
    fs::write(&file, b"{ not a queue").unwrap();
    let (_held, s) = world(&paths);
    let said = all(&s);
    let line = "Could not read the saved job queue: the file is damaged. \
                It is kept as queue.json.bad in the state folder.";
    assert!(said.contains(&line.to_owned()), "{said:?}");
    assert_eq!(
        fs::read(paths.state_dir.join("queue.json.bad")).unwrap(),
        b"{ not a queue"
    );
    assert!(!file.exists());
    assert!(s.app.jobs().interrupted().is_empty());
}
