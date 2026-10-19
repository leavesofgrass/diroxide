//! Watch this folder through the app (plan commit 34): the command on a
//! folder, each settled arrival said at most once every ten seconds in
//! screen-reader mode, the waiting line shown and never said, Say Status,
//! and Cancel or the command again to stop. On the session's fixed clock,
//! with the runtime's arrival watch stepped by the test over the fake
//! provider; then once through the real runtime, in real time.

use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::{AppEvent, Request};
use diroxide_core::{Clock, Command, Effect, Importance, Message, Place, Timestamp};
use diroxide_ops::FakeProvider;
use diroxide_ops::tracker::{ArrivalWatch, SettleRules, WatchNote, later_by};
use diroxide_vfs::{Registry, WatchEvent};

use crate::support::{Session, fixture, place, settings, temp};

/// The test's side of the runtime: the arrival watch the app asked for,
/// over the fake provider, stepped on the session's clock.
struct Stepped {
    watch: ArrivalWatch,
    fake: Arc<FakeProvider>,
    folder: Place,
    id: u64,
    start: Timestamp,
}

impl Stepped {
    /// Takes the app's request and starts its watch at the clock's time.
    fn take(s: &mut Session) -> Self {
        let requests = s.app.take_requests();
        let (id, folder, pace) = requests
            .iter()
            .find_map(|r| match r {
                Request::WatchArrivals { watch, place, pace } => {
                    Some((*watch, place.clone(), *pace))
                }
                _ => None,
            })
            .expect("a watch was asked for");
        let fake = Arc::new(FakeProvider::new());
        fake.add_folder(&folder);
        let watch = ArrivalWatch::new(
            Arc::clone(&fake) as Arc<dyn diroxide_vfs::Provider>,
            folder.clone(),
            SettleRules::WATCHED,
        )
        .with_pace(pace)
        .with_writer_probe(false);
        let start = s.clock.now();
        let mut stepped = Self {
            watch,
            fake,
            folder,
            id,
            start,
        };
        let notes = stepped.watch.start(start).unwrap();
        assert!(
            feed(s, id, notes).is_empty(),
            "nothing is said at the start"
        );
        stepped
    }

    fn at(&self, ms: u64) -> Timestamp {
        later_by(self.start, Duration::from_millis(ms))
    }

    /// A file appears at `ms`, `size` bytes, and the watch hears of it.
    fn arrive(&mut self, s: &mut Session, name: &str, size: u64, ms: u64) -> Vec<Message> {
        let now = self.at(ms);
        s.clock.set(now);
        let file = self.folder.join(name).unwrap();
        self.fake.add_file(&file, b"");
        self.fake.set_listed_size(&file, size);
        self.fake.set_modified(&file, now);
        let notes = self.watch.event(&WatchEvent::Created(file), now);
        feed(s, self.id, notes)
    }

    /// The watch's time-driven work every 50 ms from `from` to `until`.
    fn run(&mut self, s: &mut Session, from: u64, until: u64) -> Vec<(u64, Message)> {
        let mut said = Vec::new();
        let mut ms = from;
        while ms <= until {
            let now = self.at(ms);
            s.clock.set(now);
            let notes = self.watch.wake(now);
            said.extend(feed(s, self.id, notes).into_iter().map(|m| (ms, m)));
            ms += 50;
        }
        said
    }
}

/// Hands notes to the app as the runtime would; returns what it said.
fn feed(s: &mut Session, watch: u64, notes: Vec<WatchNote>) -> Vec<Message> {
    let mut said = Vec::new();
    for note in notes {
        for effect in s.app.handle(AppEvent::Arrivals { watch, note }) {
            if let Effect::Announce(message) = effect {
                said.push(message);
            }
        }
    }
    said
}

/// Runs a command without carrying out its requests; returns what it said.
fn quiet_command(s: &mut Session, id: diroxide_core::CommandId) -> Vec<String> {
    let effects = s.app.handle(AppEvent::Command(Command::new(id)));
    s.run_quiet(effects)
}

fn spoken(said: &[(u64, Message)]) -> Vec<(u64, String)> {
    said.iter()
        .filter(|(_, m)| m.importance != Importance::Low)
        .map(|(ms, m)| (*ms, m.text.clone()))
        .collect()
}

#[test]
fn each_arrival_is_said_at_most_every_ten_seconds_and_the_waiting_line_is_only_shown() {
    let dir = temp();
    let work = fixture(dir.path());
    let mut s = Session::start(
        place(&work),
        place(&work),
        Registry::with_local(),
        None,
        settings(),
    );
    assert_eq!(
        quiet_command(&mut s, ids::WATCH_FOLDER),
        vec!["Watching work for new files. Escape stops."]
    );
    let mut w = Stepped::take(&mut s);
    assert_eq!(w.folder, place(&work));
    assert_eq!(s.app.folder_watch().map(|f| f.folder()), Some("work"));

    // One file: the waiting line is shown, not said; the arrival is said
    // once it has been quiet for two seconds.
    let said = w.arrive(&mut s, "scan-004.pdf", 1_200_000, 0);
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].text, "Waiting for scan-004.pdf to settle");
    assert_eq!(said[0].importance, Importance::Low, "shown, never said");
    assert_eq!(s.app.status_line(), "Waiting for scan-004.pdf to settle");
    let said = w.run(&mut s, 50, 2_950);
    assert_eq!(
        spoken(&said),
        vec![(2_000, "Arrived: scan-004.pdf, 1.2 MB".to_owned())]
    );
    assert_eq!(s.app.status_line(), "Arrived: scan-004.pdf, 1.2 MB");

    // Two more inside ten seconds: nothing is said until the interval is
    // over, then one summary for both.
    let mut said = Vec::new();
    said.extend(
        w.arrive(&mut s, "scan-005.pdf", 300, 3_000)
            .into_iter()
            .map(|m| (3_000, m)),
    );
    said.extend(w.run(&mut s, 3_050, 3_950));
    said.extend(
        w.arrive(&mut s, "scan-006.pdf", 300, 4_000)
            .into_iter()
            .map(|m| (4_000, m)),
    );
    said.extend(w.run(&mut s, 4_050, 15_000));
    assert_eq!(
        spoken(&said),
        vec![(12_000, "Watching work: 3 arrived".to_owned())]
    );
    // The waiting line changed at most once inside the interval, and gave
    // way to the count when nothing waited.
    let shown: Vec<String> = said
        .iter()
        .filter(|(_, m)| m.importance == Importance::Low)
        .map(|(_, m)| m.text.clone())
        .collect();
    assert_eq!(
        shown,
        vec![
            "Waiting for scan-005.pdf to settle".to_owned(),
            "Watching work: 3 arrived".to_owned()
        ]
    );
    assert_eq!(s.app.folder_watch().map(|f| f.arrived()), Some(3));

    // Say Status adds the watch.
    let said = s.command(ids::SAY_STATUS);
    assert!(
        said.last().unwrap().ends_with(". Watching work: 3 arrived"),
        "{said:?}"
    );

    // Cancel with a dialog open closes the dialog, and the watch goes on.
    s.command(ids::PALETTE);
    assert!(s.app.dialog().is_some());
    s.key_quiet("escape");
    assert!(s.app.dialog().is_none());
    assert!(s.app.folder_watch().is_some());

    // Cancel stops it.
    assert_eq!(
        s.key_quiet("escape"),
        vec!["Stopped watching work: 3 arrived"]
    );
    assert!(s.app.folder_watch().is_none());
    assert!(
        s.app
            .take_requests()
            .iter()
            .any(|r| matches!(r, Request::StopArrivals))
    );
    // A note from the stopped watch is dropped.
    let late = feed(
        &mut s,
        w.id,
        vec![WatchNote::Say("Arrived: late.pdf".to_owned())],
    );
    assert!(late.is_empty());
    // Escape with no watch says nothing about watching.
    assert!(s.key_quiet("escape").is_empty());

    // The command again stops, too; a new watch has a new number.
    assert_eq!(
        quiet_command(&mut s, ids::WATCH_FOLDER),
        vec!["Watching work for new files. Escape stops."]
    );
    let second = Stepped::take(&mut s);
    assert_ne!(second.id, w.id);
    assert_eq!(
        quiet_command(&mut s, ids::WATCH_FOLDER),
        vec!["Stopped watching work"]
    );
}

#[test]
fn a_watch_that_ends_by_itself_says_why_and_a_place_that_is_polled_says_so() {
    let dir = temp();
    let work = fixture(dir.path());
    let mut s = Session::start(
        place(&work),
        place(&work),
        Registry::with_local(),
        None,
        settings(),
    );
    quiet_command(&mut s, ids::WATCH_FOLDER);
    let w = Stepped::take(&mut s);
    let said = feed(
        &mut s,
        w.id,
        vec![WatchNote::Started {
            already: 0,
            polled: Some(Duration::from_secs(5)),
        }],
    );
    assert_eq!(
        said[0].text,
        "Checking work every 5 seconds: it cannot be watched"
    );
    assert_eq!(
        s.app.folder_watch().and_then(|f| f.polled()),
        Some(Duration::from_secs(5))
    );
    let said = feed(
        &mut s,
        w.id,
        vec![WatchNote::Ended {
            line: "Stopped watching work: the folder is gone".to_owned(),
        }],
    );
    assert_eq!(said[0].text, "Stopped watching work: the folder is gone");
    assert_ne!(
        said[0].importance,
        Importance::High,
        "a watch never interrupts"
    );
    assert!(s.app.folder_watch().is_none());
}

#[test]
fn watch_this_folder_is_in_the_view_menu_and_the_palette_with_no_key() {
    let info = diroxide_app::commands::info(&ids::WATCH_FOLDER).unwrap();
    assert_eq!(info.name, "Watch this folder");
    assert!(info.palette_only);
    assert!(info.gui.is_empty());
    let dir = temp();
    let work = fixture(dir.path());
    let mut s = Session::start(
        place(&work),
        place(&work),
        Registry::with_local(),
        None,
        settings(),
    );
    // From the palette, by name.
    s.command(ids::PALETTE);
    s.type_text("watch this");
    let said = s.key("enter");
    assert!(
        said.contains(&"Watching work for new files. Escape stops.".to_owned()),
        "{said:?}"
    );
    assert!(s.app.folder_watch().is_some());
    // From the View menu, by its access key: the command again stops.
    let said = s.command(ids::MENU_VIEW);
    let menu = s.app.dialog().expect("the View menu");
    assert!(
        format!("{menu:?}").contains("Watch this folder"),
        "{said:?}"
    );
    let said = s.key("a");
    assert!(
        said.contains(&"Stopped watching work".to_owned()),
        "{said:?}"
    );
    assert!(s.app.folder_watch().is_none());
}

#[test]
fn the_real_runtime_tells_of_a_file_written_into_the_watched_folder() {
    // Real time through the runtime's thread: generous bounds, and the
    // test checks what is said, not when.
    let dir = temp();
    let work = fixture(dir.path());
    let mut s = Session::start(
        place(&work),
        place(&work),
        Registry::with_local(),
        None,
        settings(),
    );
    assert_eq!(
        s.command(ids::WATCH_FOLDER),
        vec!["Watching work for new files. Escape stops."]
    );
    // The file comes once the watch has listed the folder, so it is an
    // arrival and not part of what was there.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut said: Vec<String> = Vec::new();
    let mut wrote = false;
    while Instant::now() < deadline {
        for event in s.runtime.wait(Duration::from_millis(50)) {
            let effects = s.app.handle(event);
            said.extend(s.run(effects));
        }
        if !wrote && s.app.folder_watch().is_some_and(|w| w.is_started()) {
            std::fs::write(work.join("arrived.txt"), b"hello").unwrap();
            wrote = true;
        }
        if said
            .iter()
            .any(|line| line.starts_with("Arrived: arrived.txt"))
        {
            break;
        }
    }
    assert!(wrote, "the watch never started: {said:?}");
    assert!(
        said.iter()
            .any(|line| line == "Arrived: arrived.txt, 5 bytes"),
        "{said:?}"
    );
    assert_eq!(s.key("escape"), vec!["Stopped watching work: 1 arrived"]);
}
