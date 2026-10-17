//! The tracker over a provider's watch: [`ArrivalWatch`] turns watch
//! events, listings and looks into [`WatchNote`]s, stepped by its caller
//! with the time; [`spawn_watch`] runs one on its own thread over a real
//! watch and a real clock, for the app and for `diro watch`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use diroxide_core::{Clock, Place, PlaceKind, Timestamp};
use diroxide_vfs::{OpContext, Provider, VfsError, WatchEvent, WatchHandle};

use super::ignore::is_ignored;
use super::lines::{Teller, gone_line, skipped_line, waiting_line};
use super::machine::{Arrival, Facts, Look, SettleRules, SkipReason, Tracker, TrackerEvent};
use super::probe::writer_open;
use super::{between, later_by};

/// How often a place that cannot be watched is listed instead: the
/// shortest interval R16 allows, since a session's watch is interactive.
pub const POLL_EVERY: Duration = Duration::from_secs(5);

/// What a watch tells its frontend.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatchNote {
    /// Watching began: how many entries were there already, which are not
    /// arrivals, and how often the place is listed when it cannot be
    /// watched.
    Started {
        /// The entries there when watching began.
        already: usize,
        /// The listing interval of a polled place; `None` when watched.
        polled: Option<Duration>,
    },
    /// A file settled: what `diro watch` prints, and what a frontend
    /// counts.
    Arrived(Arrival),
    /// A line to say now, paced: one arrival, or the summary (only when
    /// the watch was given a pace).
    Say(String),
    /// A file is skipped, and said: "Skipped scan-004.pdf: online only".
    Skipped {
        /// Its name.
        name: String,
        /// Why.
        reason: SkipReason,
        /// The line.
        line: String,
    },
    /// The watch stopped waiting for a file that kept changing.
    GaveUp {
        /// Its name.
        name: String,
        /// The line: "Gave up waiting for app.log".
        line: String,
    },
    /// What the status line shows while files settle, never said;
    /// `None` when nothing waits any more.
    Waiting(Option<String>),
    /// The watch could not begin: the folder is not there or cannot be
    /// read. Nothing more follows.
    Failed {
        /// The sentence: "Could not watch in: it does not exist."
        line: String,
    },
    /// The watch ended by itself: the folder went away. Nothing more
    /// follows.
    Ended {
        /// The sentence: "Stopped watching in: the folder is gone".
        line: String,
    },
}

/// The tracker over one folder of one provider, stepped by its caller:
/// [`ArrivalWatch::start`] once, then [`ArrivalWatch::event`] for each
/// watch event and [`ArrivalWatch::wake`] when [`ArrivalWatch::next_wake`]
/// says. Every step takes the time, so tests run it on a fixed clock.
pub struct ArrivalWatch {
    provider: Arc<dyn Provider>,
    place: Place,
    folder: String,
    tracker: Tracker,
    teller: Option<Teller>,
    probe: bool,
    poll_every: Option<Duration>,
    poll_at: Option<Timestamp>,
    tick_at: Option<Timestamp>,
    waiting: Option<String>,
}

impl std::fmt::Debug for ArrivalWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArrivalWatch")
            .field("place", &self.place)
            .field("tracker", &self.tracker)
            .field("poll_every", &self.poll_every)
            .finish_non_exhaustive()
    }
}

/// The name a line uses for a folder: its last name, or the whole place
/// at a top ("D:\").
fn folder_name(place: &Place) -> String {
    place
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| place.to_string())
}

impl ArrivalWatch {
    /// A watch of `place`, through its provider, by these rules. The
    /// writer probe is on for a local place.
    pub fn new(provider: Arc<dyn Provider>, place: Place, rules: SettleRules) -> Self {
        Self {
            folder: folder_name(&place),
            probe: place.kind() == PlaceKind::Local,
            provider,
            place,
            tracker: Tracker::new(rules),
            teller: None,
            poll_every: None,
            poll_at: None,
            tick_at: None,
            waiting: None,
        }
    }

    /// The same watch, saying paced lines ([`WatchNote::Say`]) at most one
    /// every `interval`.
    pub fn with_pace(mut self, interval: Duration) -> Self {
        self.teller = Some(Teller::new(self.folder.clone(), interval));
        self
    }

    /// The same watch, listing the folder every `every` because it cannot
    /// be watched.
    pub fn with_polling(mut self, every: Duration) -> Self {
        self.poll_every = Some(every);
        self
    }

    /// The same watch with the writer probe on or off.
    pub fn with_writer_probe(mut self, on: bool) -> Self {
        self.probe = on;
        self
    }

    /// The folder's name, as the lines say it.
    pub fn folder(&self) -> &str {
        &self.folder
    }

    /// The place watched.
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// The tracker.
    pub fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    /// How many files arrived.
    pub fn arrived(&self) -> usize {
        self.tracker.arrived()
    }

    /// What the status line shows now, if anything waits.
    pub fn waiting_line(&self) -> Option<String> {
        self.waiting.clone()
    }

    /// Lists the folder once: what is there is the baseline, not an
    /// arrival. An error means the watch cannot begin.
    pub fn start(&mut self, now: Timestamp) -> Result<Vec<WatchNote>, VfsError> {
        let entries = self.provider.list(&self.place, &OpContext::new())?;
        let already = entries.iter().filter(|e| !is_ignored(&e.name)).count();
        self.tracker.baseline(
            entries
                .iter()
                .map(|entry| (entry.name.clone(), Facts::from_entry(entry))),
            now,
        );
        self.poll_at = self.poll_every.map(|every| later_by(now, every));
        Ok(vec![WatchNote::Started {
            already,
            polled: self.poll_every,
        }])
    }

    /// Takes a watch event: the name it names is looked at; a rescan
    /// lists the folder; the folder gone ends the watch.
    pub fn event(&mut self, event: &WatchEvent, now: Timestamp) -> Vec<WatchNote> {
        let events = match event {
            WatchEvent::Created(place)
            | WatchEvent::Changed(place)
            | WatchEvent::Removed(place) => {
                let Some(name) = place.name().map(str::to_owned) else {
                    return Vec::new();
                };
                if is_ignored(&name) {
                    return Vec::new();
                }
                let look = self.look(&name, false);
                self.tracker.observe(&name, look, now)
            }
            WatchEvent::Rescan => return self.rescan(now),
            WatchEvent::Gone => {
                return vec![WatchNote::Ended {
                    line: gone_line(&self.folder),
                }];
            }
        };
        self.notes(events, now)
    }

    /// Lists the folder and compares, as after an overflow or on a poll.
    pub fn rescan(&mut self, now: Timestamp) -> Vec<WatchNote> {
        match self.provider.list(&self.place, &OpContext::new()) {
            Ok(entries) => {
                let listing: Vec<(String, Facts)> = entries
                    .iter()
                    .map(|entry| (entry.name.clone(), Facts::from_entry(entry)))
                    .collect();
                let events = self.tracker.reconcile(listing, now);
                self.notes(events, now)
            }
            Err(VfsError::NotFound | VfsError::NotAFolder) => vec![WatchNote::Ended {
                line: gone_line(&self.folder),
            }],
            // A passing problem: the next event or poll looks again.
            Err(_) => Vec::new(),
        }
    }

    /// The time-driven work, when it is due: the tick over the settling
    /// files, a poll of a place that cannot be watched, a paced line.
    pub fn wake(&mut self, now: Timestamp) -> Vec<WatchNote> {
        let mut notes = Vec::new();
        if self.poll_at.is_some_and(|at| at <= now) {
            self.poll_at = self.poll_every.map(|every| later_by(now, every));
            notes.extend(self.rescan(now));
            if notes.iter().any(ends) {
                return notes;
            }
        }
        if self.tick_at.is_some_and(|at| at <= now) {
            self.tick_at = None;
            let provider = Arc::clone(&self.provider);
            let place = self.place.clone();
            let probe = self.probe;
            let mut look = |name: &str, due: bool| look_at(&*provider, &place, name, probe && due);
            let events = self.tracker.tick(now, &mut look);
            notes.extend(self.notes(events, now));
        }
        if let Some(line) = self.teller.as_mut().and_then(|t| t.due(now)) {
            notes.push(WatchNote::Say(line));
        }
        notes
    }

    /// How long until [`ArrivalWatch::wake`] has work; `None` when only an
    /// event can bring any.
    pub fn next_wake(&self, now: Timestamp) -> Option<Duration> {
        [
            self.tick_at.map(|at| between(now, at)),
            self.poll_at.map(|at| between(now, at)),
            self.teller.as_ref().and_then(|t| t.next_due(now)),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn look(&self, name: &str, probe: bool) -> Look {
        look_at(&*self.provider, &self.place, name, self.probe && probe)
    }

    /// The tracker's events as notes, the tick scheduled while anything
    /// settles, and the status line when it changed.
    fn notes(&mut self, events: Vec<TrackerEvent>, now: Timestamp) -> Vec<WatchNote> {
        let mut notes = Vec::new();
        for event in events {
            match event {
                TrackerEvent::Arrived(arrival) => {
                    let said = self
                        .teller
                        .as_mut()
                        .and_then(|t| t.arrived(arrival.clone(), now));
                    notes.push(WatchNote::Arrived(arrival));
                    if let Some(line) = said {
                        notes.push(WatchNote::Say(line));
                    }
                }
                TrackerEvent::Skipped { name, reason } if reason.is_said() => {
                    let line = skipped_line(&name, reason);
                    notes.push(WatchNote::Skipped { name, reason, line });
                }
                TrackerEvent::GaveUp { name } => {
                    let line = super::lines::gave_up_line(&name);
                    notes.push(WatchNote::GaveUp { name, line });
                }
                TrackerEvent::Skipped { .. } | TrackerEvent::Gone { .. } => {}
            }
        }
        if self.tracker.is_settling() {
            if self.tick_at.is_none() {
                self.tick_at = Some(later_by(now, self.tracker.rules().tick));
            }
        } else {
            self.tick_at = None;
        }
        let waiting = waiting_line(&self.tracker.waiting());
        if waiting != self.waiting {
            self.waiting = waiting.clone();
            notes.push(WatchNote::Waiting(waiting));
        }
        notes
    }
}

/// Whether a note ends the watch.
fn ends(note: &WatchNote) -> bool {
    matches!(note, WatchNote::Ended { .. } | WatchNote::Failed { .. })
}

/// One look at a name in a folder: its facts from the provider, and, when
/// asked, whether a writer still holds it (local files only, never an
/// online-only one).
fn look_at(provider: &dyn Provider, folder: &Place, name: &str, probe: bool) -> Look {
    let Ok(place) = folder.join(name) else {
        return Look::Unreadable;
    };
    match provider.stat(&place, &OpContext::new()) {
        Ok(entry) => {
            let mut facts = Facts::from_entry(&entry);
            if probe
                && facts.skip_reason().is_none()
                && let Some(path) = place.local_path()
            {
                facts.writer_open = writer_open(std::path::Path::new(path));
            }
            Look::Present(facts)
        }
        Err(VfsError::NotFound) => Look::Absent,
        Err(_) => Look::Unreadable,
    }
}

/// How [`spawn_watch`] watches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatchOptions {
    /// The quiet period, instead of the rules' own (2 seconds watched, 10
    /// polled).
    pub quiet: Option<Duration>,
    /// Say paced lines ([`WatchNote::Say`]) at most one this often.
    pub pace: Option<Duration>,
}

/// A watch running on its own thread. Dropping it stops the watch at
/// once: no note is sent after the drop.
#[derive(Debug)]
pub struct WatchRun {
    stop: Arc<AtomicBool>,
    wake: Sender<Wake>,
    _handle: Option<WatchHandle>,
}

impl Drop for WatchRun {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.wake.send(Wake::Stop);
    }
}

enum Wake {
    Event(WatchEvent),
    Stop,
}

/// Starts watching `place` through `provider` on a thread of its own,
/// sending each note to `sink` as it comes; the folder is listed on that
/// thread, so a slow place never holds the caller. A place the provider
/// cannot watch is listed every [`POLL_EVERY`] with the polled rules.
/// The watch ends after [`WatchNote::Failed`] or [`WatchNote::Ended`], or
/// when the returned [`WatchRun`] is dropped. An error means the thread
/// could not start, and nothing will be sent.
pub fn spawn_watch(
    provider: Arc<dyn Provider>,
    place: Place,
    options: WatchOptions,
    clock: Arc<dyn Clock>,
    sink: Box<dyn Fn(WatchNote) + Send>,
) -> Result<WatchRun, VfsError> {
    let (sender, receiver) = mpsc::channel::<Wake>();
    let events = sender.clone();
    let watched = provider.watch(
        &place,
        Box::new(move |event| {
            let _ = events.send(Wake::Event(event));
        }),
    );
    let handle = watched.ok().flatten();
    let polled = handle.is_none();
    let mut rules = if polled {
        SettleRules::POLLED
    } else {
        SettleRules::WATCHED
    };
    if let Some(quiet) = options.quiet {
        rules = rules.with_quiet(quiet);
    }
    let mut watch = ArrivalWatch::new(provider, place, rules);
    if let Some(pace) = options.pace {
        watch = watch.with_pace(pace);
    }
    if polled {
        watch = watch.with_polling(POLL_EVERY);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let send = move |notes: Vec<WatchNote>| -> bool {
        for note in notes {
            if stopped.load(Ordering::SeqCst) {
                return false;
            }
            let last = ends(&note);
            sink(note);
            if last {
                return false;
            }
        }
        !stopped.load(Ordering::SeqCst)
    };
    let spawned = std::thread::Builder::new()
        .name("diroxide-arrivals".to_owned())
        .spawn(move || {
            let started = match watch.start(clock.now()) {
                Ok(notes) => notes,
                Err(error) => {
                    let line = diroxide_core::Message::could_not(
                        "watch",
                        watch.folder(),
                        &error.to_string(),
                        "",
                    )
                    .text;
                    send(vec![WatchNote::Failed { line }]);
                    return;
                }
            };
            if !send(started) {
                return;
            }
            loop {
                let wait = watch
                    .next_wake(clock.now())
                    .unwrap_or(Duration::from_secs(60));
                let notes = match receiver.recv_timeout(wait) {
                    Ok(Wake::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    Ok(Wake::Event(event)) => {
                        let mut notes = watch.event(&event, clock.now());
                        // Take what else waits before looking again.
                        while let Ok(next) = receiver.try_recv() {
                            match next {
                                Wake::Event(event) => {
                                    notes.extend(watch.event(&event, clock.now()));
                                }
                                Wake::Stop => return,
                            }
                        }
                        notes.extend(watch.wake(clock.now()));
                        notes
                    }
                    Err(RecvTimeoutError::Timeout) => watch.wake(clock.now()),
                };
                if !send(notes) {
                    return;
                }
            }
        });
    spawned?;
    Ok(WatchRun {
        stop,
        wake: sender,
        _handle: handle,
    })
}
