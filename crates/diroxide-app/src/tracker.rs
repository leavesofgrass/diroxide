//! The watch command's state in the app (W3t, plan commit 34): which
//! folder is watched, how many files arrived in it, and what still
//! settles. The tracker itself is `diroxide_ops::tracker`, run by the
//! runtime on its own thread; the app only keeps the count and the words.
//!
//! Watch this folder (the palette and the View menu; no key) is
//! session-only: nothing is saved, moved or run. It says one polite line
//! per settled arrival at most every progress interval in screen-reader
//! mode ("Arrived: scan-004.pdf, 1.2 MB", or "Watching Downloads: 3
//! arrived" when several came), shows "Waiting for scan-004.pdf to
//! settle" on the status line without saying it, and stops on the command
//! again or on Cancel ("Stopped watching Downloads").

use std::time::Duration;

use diroxide_core::{Place, Timestamp};
use diroxide_ops::tracker::summary_line;

/// The folder being watched for arrivals, and what came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderWatch {
    /// The watch's number, so a note from a watch already stopped is
    /// dropped.
    pub(crate) id: u64,
    place: Place,
    folder: String,
    pub(crate) arrived: usize,
    /// What waits now, as the status line words it.
    pub(crate) waiting: Option<String>,
    /// The waiting line last shown, and when.
    pub(crate) shown: Option<(String, Timestamp)>,
    /// How often a place that cannot be watched is listed.
    pub(crate) polled: Option<Duration>,
    /// The folder was listed and the watch listens.
    pub(crate) started: bool,
}

impl FolderWatch {
    /// A watch of `place`, numbered `id`, with nothing arrived yet.
    pub fn new(id: u64, place: Place) -> Self {
        Self {
            id,
            folder: crate::panel::place_leaf(&place),
            place,
            arrived: 0,
            waiting: None,
            shown: None,
            polled: None,
            started: false,
        }
    }

    /// The folder watched.
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// The folder's name, as the lines say it.
    pub fn folder(&self) -> &str {
        &self.folder
    }

    /// How many files arrived.
    pub fn arrived(&self) -> usize {
        self.arrived
    }

    /// What waits to settle, in words, if anything does.
    pub fn waiting(&self) -> Option<&str> {
        self.waiting.as_deref()
    }

    /// How often the folder is listed, when it cannot be watched.
    pub fn polled(&self) -> Option<Duration> {
        self.polled
    }

    /// Whether the folder was listed and the watch listens: what is there
    /// before this is not an arrival.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// The watch for Say Status: "Watching Downloads: 3 arrived", then
    /// what waits ("Waiting for scan-004.pdf to settle").
    pub fn status(&self) -> String {
        let mut line = summary_line(&self.folder, self.arrived);
        if let Some(waiting) = &self.waiting {
            line.push_str(". ");
            line.push_str(waiting);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_says_the_count_and_what_waits() {
        let place = Place::parse(if cfg!(windows) { r"D:\in" } else { "/in" }).unwrap();
        let mut watch = FolderWatch::new(1, place.clone());
        assert_eq!(watch.folder(), "in");
        assert_eq!(watch.place(), &place);
        assert_eq!(watch.status(), "Watching in: 0 arrived");
        watch.arrived = 3;
        watch.waiting = Some("Waiting for scan-004.pdf to settle".to_owned());
        assert_eq!(
            watch.status(),
            "Watching in: 3 arrived. Waiting for scan-004.pdf to settle"
        );
        assert!(diroxide_core::format::within_cells(
            &watch.status(),
            "Watching in: 3 arrived"
        ));
    }
}
