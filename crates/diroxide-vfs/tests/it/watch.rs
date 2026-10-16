//! Watching temporary folders (spike three's table as tests): events for a
//! few changes, one Rescan for a burst, "gone" when the watched folder is
//! renamed or removed, nothing after the watch is dropped, and no
//! unfinished names.

use std::fs;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use diroxide_core::Place;
use diroxide_vfs::{LocalProvider, Provider, WatchEvent, WatchHandle, WatchSink};

use crate::local::place_of;
use crate::scratch;

fn sink() -> (WatchSink, Receiver<WatchEvent>) {
    let (sender, receiver) = mpsc::channel();
    let sender = Mutex::new(sender);
    let sink: WatchSink = Box::new(move |event| {
        if let Ok(sender) = sender.lock() {
            let _ = sender.send(event);
        }
    });
    (sink, receiver)
}

fn watch(place: &Place) -> (WatchHandle, Receiver<WatchEvent>) {
    let (sink, events) = sink();
    let handle = LocalProvider::new()
        .watch(place, sink)
        .unwrap()
        .expect("a folder on a fixed local drive is watched");
    (handle, events)
}

/// Every event until `quiet` passes without one, or `limit` in all.
fn gather(events: &Receiver<WatchEvent>, quiet: Duration, limit: Duration) -> Vec<WatchEvent> {
    let started = Instant::now();
    let mut seen = Vec::new();
    while started.elapsed() < limit {
        match events.recv_timeout(quiet) {
            Ok(event) => seen.push(event),
            Err(_) => break,
        }
    }
    seen
}

#[test]
fn a_few_new_files_are_heard() {
    let folder = scratch();
    let (_handle, events) = watch(&place_of(&folder));
    for n in 0..10 {
        fs::write(folder.path().join(format!("new-{n}.txt")), b"n").unwrap();
    }
    let seen = gather(
        &events,
        Duration::from_millis(1500),
        Duration::from_secs(10),
    );
    let rescan = seen.contains(&WatchEvent::Rescan);
    for n in 0..10 {
        let place = place_of(&folder).join(&format!("new-{n}.txt")).unwrap();
        assert!(
            rescan
                || seen.contains(&WatchEvent::Created(place.clone()))
                || seen.contains(&WatchEvent::Changed(place)),
            "{seen:?}"
        );
    }
}

#[test]
fn a_burst_of_10000_files_is_one_rescan_a_second_at_most() {
    let folder = scratch();
    let (_handle, events) = watch(&place_of(&folder));
    let started = Instant::now();
    for n in 0..10_000 {
        fs::write(folder.path().join(format!("f{n:05}")), b"").unwrap();
    }
    let seen = gather(
        &events,
        Duration::from_millis(1500),
        Duration::from_secs(60),
    );
    let seconds = started.elapsed().as_secs() + 2;
    let rescans = seen.iter().filter(|e| **e == WatchEvent::Rescan).count();
    assert!(rescans >= 1, "{} events, no Rescan", seen.len());
    assert!(
        rescans as u64 <= seconds,
        "{rescans} Rescans in {seconds} seconds"
    );
    // Never more than the threshold of single events in a window, so
    // never anything like ten thousand single events.
    assert!(seen.len() < 500, "{} events", seen.len());
}

#[test]
fn renaming_the_watched_folder_says_gone() {
    let parent = scratch();
    fs::create_dir(parent.path().join("watched")).unwrap();
    let place = place_of(&parent).join("watched").unwrap();
    let (_handle, events) = watch(&place);
    fs::rename(parent.path().join("watched"), parent.path().join("moved")).unwrap();
    let seen = gather(&events, Duration::from_secs(2), Duration::from_secs(10));
    assert!(seen.contains(&WatchEvent::Gone), "{seen:?}");
}

#[test]
fn removing_the_watched_folder_says_gone() {
    let parent = scratch();
    fs::create_dir(parent.path().join("doomed")).unwrap();
    let place = place_of(&parent).join("doomed").unwrap();
    let (_handle, events) = watch(&place);
    fs::remove_dir(parent.path().join("doomed")).unwrap();
    let seen = gather(&events, Duration::from_secs(2), Duration::from_secs(10));
    assert!(seen.contains(&WatchEvent::Gone), "{seen:?}");
}

#[test]
fn a_dropped_watch_hears_nothing_and_the_other_panel_still_does() {
    let folder = scratch();
    let place = place_of(&folder);
    let (first, first_events) = watch(&place);
    let (_second, second_events) = watch(&place);
    drop(first);
    fs::write(folder.path().join("after.txt"), b"a").unwrap();
    let heard = gather(
        &second_events,
        Duration::from_secs(1),
        Duration::from_secs(10),
    );
    assert!(!heard.is_empty());
    assert!(first_events.try_recv().is_err());
}

#[test]
fn unfinished_names_are_never_passed_on() {
    let folder = scratch();
    let (_handle, events) = watch(&place_of(&folder));
    fs::write(folder.path().join(".diro-part-0000cafe"), b"half").unwrap();
    fs::write(folder.path().join("whole.txt"), b"w").unwrap();
    let seen = gather(
        &events,
        Duration::from_millis(1500),
        Duration::from_secs(10),
    );
    let unfinished = place_of(&folder).join(".diro-part-0000cafe").unwrap();
    assert!(!seen.iter().any(|event| matches!(
        event,
        WatchEvent::Created(p) | WatchEvent::Changed(p) | WatchEvent::Removed(p) if *p == unfinished
    )));
    assert!(!seen.is_empty(), "the whole file was heard");
}

#[test]
fn a_folder_that_is_not_there_is_not_watched() {
    let folder = scratch();
    let missing = place_of(&folder).join("missing").unwrap();
    let (sink, _events) = sink();
    assert!(
        LocalProvider::new()
            .watch(&missing, sink)
            .unwrap()
            .is_none()
    );
}
