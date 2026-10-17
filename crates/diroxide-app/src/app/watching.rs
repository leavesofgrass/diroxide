//! Watch this folder in the app (W3t, plan commit 34): the command, the
//! notes of the runtime's arrival watch, the status line and Say Status.
//! The state is [`crate::tracker::FolderWatch`]; the tracker is
//! `diroxide_ops::tracker`, run by the runtime.
//!
//! - The command on the active panel's folder starts a watch and says
//!   "Watching Downloads for new files. Escape stops."; the command again,
//!   or Cancel with no job running and no dialog open, stops it:
//!   "Stopped watching Downloads: 3 arrived".
//! - Each settled arrival is said politely, one line at most every
//!   progress interval in screen-reader mode (half a second otherwise):
//!   the arrival, or the summary when several came. The runtime's watch
//!   paces them.
//! - "Waiting for scan-004.pdf to settle" is shown on the status line,
//!   never said; in screen-reader mode it changes at most every progress
//!   interval, so a burst of files does not chatter.
//! - A place inside an archive is refused: it changes only through Pack.

use std::time::Duration;

use diroxide_core::{Command, Effect, Message, PlaceKind};
use diroxide_ops::tracker::{WatchNote, between, start_line, stopped_line, summary_line};

use super::{Action, App};
use crate::commands::ids;
use crate::jobs::SIGHTED_INTERVAL_MS;
use crate::request::Request;
use crate::tracker::FolderWatch;

/// The app's watch for arrivals: the one running, and the next number.
#[derive(Clone, Debug, Default)]
pub(super) struct WatchState {
    current: Option<FolderWatch>,
    next: u64,
}

impl App {
    /// The folder watched for arrivals, if a watch runs.
    pub fn folder_watch(&self) -> Option<&FolderWatch> {
        self.watching.current.as_ref()
    }

    /// Runs Watch this folder, and Cancel while a watch runs with no job
    /// to cancel and no dialog open; `None` when the command is another's.
    pub(super) fn watch_dispatch(
        &mut self,
        command: &Command,
        action: Option<Action>,
    ) -> Option<Vec<Effect>> {
        if command.id == ids::WATCH_FOLDER {
            if !self.dialogs.is_empty() {
                return Some(vec![self.say(Message::status("Close this dialog first."))]);
            }
            return Some(match self.watching.current {
                Some(_) => self.stop_watching(),
                None => self.start_watching(),
            });
        }
        let cancels = action == Some(Action::Cancel)
            && self.dialogs.is_empty()
            && self.jobs.newest().is_none()
            && self.watching.current.is_some();
        cancels.then(|| self.stop_watching())
    }

    /// The pace of the watch's lines: the progress interval in
    /// screen-reader mode, half a second otherwise.
    fn watch_interval(&self) -> Duration {
        if self.screen_reader {
            Duration::from_secs(self.settings.progress_interval_seconds.max(1))
        } else {
            Duration::from_millis(SIGHTED_INTERVAL_MS)
        }
    }

    fn start_watching(&mut self) -> Vec<Effect> {
        let place = self.panel(self.active).place().clone();
        if place.kind() == PlaceKind::Archive {
            let leaf = crate::panel::place_leaf(&place);
            return vec![self.say(Message::could_not(
                "watch",
                &leaf,
                "it is inside an archive, which changes only through Pack",
                "",
            ))];
        }
        self.watching.next += 1;
        let watch = FolderWatch::new(self.watching.next, place.clone());
        let stops = match self.key(&ids::CANCEL) {
            key if key.is_empty() => "Watch this folder again stops.".to_owned(),
            key => format!("{key} stops."),
        };
        let line = start_line(watch.folder(), None, &stops);
        self.request(Request::WatchArrivals {
            watch: watch.id,
            place,
            pace: self.watch_interval(),
        });
        self.watching.current = Some(watch);
        vec![self.say(Message::info(line))]
    }

    fn stop_watching(&mut self) -> Vec<Effect> {
        let Some(watch) = self.watching.current.take() else {
            return Vec::new();
        };
        self.request(Request::StopArrivals);
        vec![self.say(Message::info(stopped_line(watch.folder(), watch.arrived)))]
    }

    /// A note of the runtime's arrival watch. A note of a watch already
    /// stopped is dropped.
    pub(super) fn arrivals(&mut self, id: u64, note: WatchNote) -> Vec<Effect> {
        let Some(watch) = self.watching.current.as_mut().filter(|w| w.id == id) else {
            return Vec::new();
        };
        match note {
            WatchNote::Started { polled, .. } => {
                watch.started = true;
                watch.polled = polled;
                match polled {
                    Some(every) => {
                        let line = format!(
                            "Checking {} every {}: it cannot be watched",
                            watch.folder(),
                            diroxide_core::format::count(every.as_secs(), "second", "seconds")
                        );
                        vec![self.say(Message::info(line))]
                    }
                    None => Vec::new(),
                }
            }
            WatchNote::Arrived(_) => {
                watch.arrived += 1;
                Vec::new()
            }
            WatchNote::Say(line) => vec![self.say(Message::done(line))],
            WatchNote::Skipped { line, .. } | WatchNote::GaveUp { line, .. } => {
                vec![self.say(Message::info(line))]
            }
            WatchNote::Waiting(line) => self.show_waiting(line),
            WatchNote::Failed { line } | WatchNote::Ended { line } => {
                // Said politely: a watch never interrupts.
                self.watching.current = None;
                self.request(Request::StopArrivals);
                vec![self.say(Message::info(line))]
            }
            _ => Vec::new(),
        }
    }

    /// The waiting line on the status line, shown and never said; in
    /// screen-reader mode a new one at most every progress interval. When
    /// nothing waits any more, a waiting line still showing gives way to
    /// the count.
    fn show_waiting(&mut self, line: Option<String>) -> Vec<Effect> {
        let now = self.now();
        let interval = self.watch_interval();
        let screen_reader = self.screen_reader;
        let status = self.status.clone();
        let Some(watch) = self.watching.current.as_mut() else {
            return Vec::new();
        };
        watch.waiting = line.clone();
        let text = match line {
            Some(text) => {
                let recent = watch
                    .shown
                    .as_ref()
                    .is_some_and(|(_, at)| between(*at, now) < interval);
                if screen_reader && recent {
                    return Vec::new();
                }
                watch.shown = Some((text.clone(), now));
                text
            }
            None => {
                let showing = watch
                    .shown
                    .as_ref()
                    .is_some_and(|(shown, _)| *shown == status);
                watch.shown = None;
                if !showing {
                    return Vec::new();
                }
                summary_line(watch.folder(), watch.arrived)
            }
        };
        vec![self.say(Message::status(text))]
    }

    /// The watch for Say Status, when one runs.
    pub(super) fn watch_status(&self) -> Option<String> {
        self.watching.current.as_ref().map(FolderWatch::status)
    }
}
