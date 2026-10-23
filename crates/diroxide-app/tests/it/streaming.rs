//! Streamed listings reach the panel (lane A2, R25 commit 77): a folder of
//! 5,000 entries on a fake remote place comes in batches of 1,000; the rows
//! appear with the first batch, the count is said at most every progress
//! interval in screen-reader mode, the focus and the marks keep to the id
//! rule, and Cancel at the first batch keeps what came. Nothing touches
//! the network.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use diroxide_app::AppEvent;
use diroxide_config::ScreenReaderMode;
use diroxide_core::{Place, Side};
use diroxide_ops::FakeProvider;
use diroxide_vfs::Registry;

use crate::held::{Held, drive, key_held};
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

/// A folder of 5,000 files on `build`, listed in batches of 1,000, the
/// rest held after the first; a session in screen-reader mode.
fn world() -> (Held, Session) {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/a"));
    for n in 0..5000 {
        fake.add_file(&remote(&format!("/reports/f{n:04}.txt")), b"x");
    }
    let held = Held::new(fake);
    let mut registry = Registry::new();
    registry.set_local(Arc::new(held.clone()));
    registry.register("sftp", Arc::new(held.clone()));
    let mut reader = settings();
    reader.screen_reader_mode = ScreenReaderMode::On;
    let session = Session::start(local("/a"), local("/a"), registry, None, reader);
    held.set_batch(1000);
    held.close();
    (held, session)
}

fn all(s: &Session) -> Vec<String> {
    s.said.iter().map(|m| m.text.clone()).collect()
}

/// Opens the remote folder in the active panel and waits for its first
/// batch.
fn open_reports(s: &mut Session) {
    let effects = s.app.open_place(Side::Left, remote("/reports"));
    drive(s, effects, |s, _| {
        s.app.panel(Side::Left).list().len() >= 1000
    });
}

#[test]
fn rows_come_in_batches_and_keep_their_focus_and_marks() {
    let (held, mut s) = world();
    open_reports(&mut s);
    let panel = s.app.panel(Side::Left);
    assert_eq!(panel.place(), &remote("/reports"));
    assert_eq!(panel.list().len(), 1000, "the first batch is shown");
    assert!(panel.is_streaming());
    assert!(
        s.app.title_line().contains("loading"),
        "{}",
        s.app.title_line()
    );
    // The owner moves and marks while the rest is on its way.
    assert!(s.app.focus_name(Side::Left, "f0500.txt"));
    s.app.mark_names(Side::Left, &["f0001.txt".into()], true);
    // Ten seconds on, the next batch says the count, once.
    s.clock.advance(11);
    held.open();
    drive(&mut s, Vec::new(), |s, _| {
        !s.app.panel(Side::Left).is_loading()
    });
    let said = all(&s);
    let counts: Vec<&String> = said.iter().filter(|l| l.starts_with("Listing ")).collect();
    assert_eq!(counts, ["Listing reports: 2,000 so far"], "{said:?}");
    assert!(
        said.contains(&"reports, 5,000 items, in sftp://build/".to_owned()),
        "{said:?}"
    );
    let panel = s.app.panel(Side::Left);
    assert_eq!(panel.list().len(), 5000);
    assert_eq!(panel.list().focused().unwrap().id, "f0500.txt");
    assert_eq!(panel.list().selected_ids(), ["f0001.txt"]);
    assert_eq!(held.batches.load(Ordering::SeqCst), 5);
}

#[test]
fn cancel_at_the_first_batch_keeps_what_came() {
    let (held, mut s) = world();
    open_reports(&mut s);
    let said = key_held(&mut s, "escape");
    assert!(
        said.contains(&"Stopping the listing.".to_owned()),
        "{said:?}"
    );
    drive(&mut s, Vec::new(), |s, _| {
        !s.app.panel(Side::Left).is_loading()
    });
    let said = all(&s);
    assert!(
        said.contains(&"Listing stopped: 1,000 items shown, in sftp://build/".to_owned()),
        "{said:?}"
    );
    assert_eq!(held.batches.load(Ordering::SeqCst), 1);
    assert_eq!(s.app.panel(Side::Left).list().len(), 1000);
    // The folder reads again in full on Reload.
    held.open();
    let effects = s
        .app
        .handle(AppEvent::Key(diroxide_core::Key::parse("ctrl+r").unwrap()));
    drive(&mut s, effects, |s, _| {
        !s.app.panel(Side::Left).is_loading()
    });
    assert_eq!(s.app.panel(Side::Left).list().len(), 5000);
}

#[test]
fn leaving_a_folder_that_lists_stops_it() {
    let (held, mut s) = world();
    open_reports(&mut s);
    let effects = s.app.open_place(Side::Left, local("/a"));
    drive(&mut s, effects, |s, _| {
        !s.app.panel(Side::Left).is_loading()
    });
    assert_eq!(s.app.panel(Side::Left).place(), &local("/a"));
    held.open();
    drive(&mut s, Vec::new(), |_, _| true);
    assert_eq!(held.batches.load(Ordering::SeqCst), 1, "it stopped");
}
