//! The quick view (Shift+F3): the other panel turned into a live preview
//! of the file under the cursor, the orthodox quick view (plan section
//! 5.9; research/alpha2-plan.md parts 2.5 and 3.6, commits 52 to 54).
//!
//! What it does:
//!
//! - **It follows the cursor after the wait** (`[quick_view] wait_ms`,
//!   half a second): holding Down through a folder starts no render until
//!   the cursor stops, and a newer move cancels a render in flight
//!   ([`LatestOnly`] generations and the read's cancel token). At most one
//!   render runs and at most one waits, on one worker thread.
//! - **It reads the previewed file and only it**, at most `max_kib`, and
//!   never follows an include or an embed (the preview crate's readers
//!   take bytes, never a path); Markdown and carta's formats run under
//!   the preview crate's guard, so a hostile file is cut off at
//!   `deadline_ms` ("Could not read notes.org: took too long." then
//!   "Showing the text.").
//! - **It shows the file again when it changes on disk**, through the
//!   panel's folder watch ([`QuickView::watch_event`], `follow_changes`):
//!   a burst of changes renders once, after the wait, and says "Preview
//!   updated: report.md changed".
//! - **It says one line, never the text**: "Preview: report.md, 12
//!   headings"; "Preview: main.rs, Rust, 412 lines"; "Preview: big.log,
//!   first 1 MiB of 2.0 GB"; "No preview: photo.png, PNG image, 2.0 MB";
//!   "Quick view on" and "Quick view off".
//! - **Tab enters it as a region** (`region`): the document as a list in
//!   the list model, moved through by line, heading, paragraph, table
//!   and cell, links and find, with the safe opener rule for links.
//! - **"Open in textweaver"** hands the file to `tw` through the tools
//!   registry ([`QuickView::open_in_textweaver`]), or says how to install
//!   it.
//!
//! How a frontend drives it, as W2d's viewer is driven: every command is
//! a plain method that returns the [`Message`]s to say, in order, and
//! takes the time as an argument, so the tests need no clock. W2f wires
//! [`QuickView::toggle`] to Shift+F3, [`QuickView::cursor_moved`] to the
//! active panel's cursor, [`QuickView::tick`] to the app's timer while
//! [`QuickView::wants_tick`], and [`QuickView::watch_event`] to the
//! panel's watch.

mod region;
mod textweaver;
mod worker;

#[cfg(test)]
mod region_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_config::{Highlighting, QuickViewSettings, Settings, ViewerSettings};
use diroxide_core::format::human_size;
use diroxide_core::names::display_name;
use diroxide_core::{Entry, Message, Place};
use diroxide_filetype::TypeRegistry;
use diroxide_preview::LatestOnly;
use diroxide_preview::doc::text::{HEX_ROW, HEX_ROW_SCREEN_READER};
use diroxide_preview::doc::{self, DocKind, DocView, Guard, ReadOptions, SHOWING_THE_TEXT};
use diroxide_preview::highlight::{Caps, Job, Session};
use diroxide_vfs::{CancelToken, Registry, WatchEvent};

use region::Region;
pub use region::{LinkTarget, Outcome, QuickViewRequest};
pub use textweaver::TEXTWEAVER;
use worker::{Answer, Rendered, Request, Shared, Worker};

/// What the quick view reads from the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickViewOptions {
    /// The `[quick_view]` table.
    pub settings: QuickViewSettings,
    /// The `[viewer]` table, for the highlight caps.
    pub viewer: ViewerSettings,
    /// Whether code is colored.
    pub highlighting: Highlighting,
    /// Behave for a screen reader: eight bytes a hex row, no colors in
    /// automatic mode.
    pub screen_reader: bool,
}

impl Default for QuickViewOptions {
    fn default() -> Self {
        Self {
            settings: QuickViewSettings::default(),
            viewer: ViewerSettings::default(),
            highlighting: Highlighting::Auto,
            screen_reader: true,
        }
    }
}

impl QuickViewOptions {
    /// The options from the app's settings.
    pub fn from_settings(settings: &Settings, screen_reader: bool) -> Self {
        Self {
            settings: settings.quick_view,
            viewer: settings.viewer,
            highlighting: settings.highlighting,
            screen_reader,
        }
    }

    /// The wait after the cursor stops.
    pub fn wait(&self) -> Duration {
        Duration::from_millis(self.settings.wait_ms)
    }

    /// The most of a file read, in bytes: `max_kib`, at least 1 KiB.
    pub fn max_bytes(&self) -> u64 {
        self.settings.max_kib.max(1).saturating_mul(1024)
    }

    /// How long a document's reader may take.
    pub fn deadline(&self) -> Duration {
        Duration::from_millis(self.settings.deadline_ms.max(1))
    }

    /// The bytes a hex row shows: eight for a screen reader, inside 40
    /// cells; sixteen otherwise.
    pub fn hex_row(&self) -> usize {
        if self.screen_reader {
            HEX_ROW_SCREEN_READER
        } else {
            HEX_ROW
        }
    }

    /// Whether code gets its colors: on, off, or off in screen reader
    /// mode when automatic.
    pub fn colors_code(&self) -> bool {
        match self.highlighting {
            Highlighting::On => true,
            Highlighting::Off => false,
            Highlighting::Auto => !self.screen_reader,
        }
    }

    /// The highlight caps from the `[viewer]` table.
    pub fn caps(&self) -> Caps {
        let s = &self.viewer;
        Caps {
            max_bytes: s.highlight_max_kib.saturating_mul(1024),
            max_lines: s.highlight_max_lines,
            max_line_chars: s.highlight_max_line_chars,
            deadline: Duration::from_millis(s.highlight_deadline_ms),
        }
    }
}

/// Why a render was asked for: it decides the words said when it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    /// The cursor moved to it, or the quick view was turned on.
    Moved,
    /// It changed on disk.
    Changed,
}

/// A render waiting for its time.
#[derive(Clone, Debug)]
struct Target {
    place: Place,
    entry: Entry,
    /// When the cursor got there, or the change came.
    since: Instant,
    /// Start at once, without the wait (turning on).
    now: bool,
    reason: Reason,
}

/// A render the worker has.
#[derive(Debug)]
struct InFlight {
    generation: u64,
    place: Place,
    reason: Reason,
    cancel: CancelToken,
}

/// What the quick view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Showing {
    /// A document's rows.
    Document(DocView),
    /// Nothing for this kind of file yet, said in words: "PNG image, 2.0
    /// MB".
    NoPreview(String),
    /// The file could not be read: the message said.
    Unreadable(String),
}

/// The file the quick view shows, and what it shows of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    /// The file's place.
    pub place: Place,
    /// Its name as shown, hidden characters spelled out.
    pub name: String,
    /// What is shown.
    pub showing: Showing,
    /// Its size, as listed or found when it changed.
    pub size: Option<u64>,
    /// Whether the cap stopped the read short of its end.
    pub capped: bool,
}

/// The quick view. See the module documentation.
#[derive(Debug)]
pub struct QuickView {
    options: QuickViewOptions,
    on: bool,
    worker: Worker,
    latest: Arc<LatestOnly>,
    target: Option<Target>,
    in_flight: Option<InFlight>,
    shown: Option<Shown>,
    session: Option<Session>,
    job: Option<Job>,
    renders: u64,
    region: Region,
    siblings: Vec<(Place, Entry)>,
}

impl QuickView {
    /// A quick view, off, reading through the registry (the runtime's,
    /// whose providers are shared, not copied).
    pub fn new(registry: Registry, options: QuickViewOptions) -> Self {
        let latest = Arc::new(LatestOnly::new());
        let worker = Worker::new(Self::shared(
            registry,
            TypeRegistry::builtin(),
            &latest,
            &options,
        ));
        Self {
            options,
            on: false,
            worker,
            latest,
            target: None,
            in_flight: None,
            shown: None,
            session: None,
            job: None,
            renders: 0,
            region: Region::default(),
            siblings: Vec::new(),
        }
    }

    fn shared(
        registry: Registry,
        types: TypeRegistry,
        latest: &Arc<LatestOnly>,
        options: &QuickViewOptions,
    ) -> Shared {
        Shared {
            registry,
            types,
            latest: Arc::clone(latest),
            read: ReadOptions {
                guard: Guard::default().with_deadline(options.deadline()),
                hex_row: options.hex_row(),
            },
            max_bytes: options.max_bytes(),
        }
    }

    /// Uses the user's file types (with their own types) to tell a picture
    /// or a PDF from bytes to show as hex.
    pub fn set_types(&mut self, types: TypeRegistry) {
        let registry = self.worker.shared().registry.clone();
        let shared = Self::shared(registry, types, &self.latest, &self.options);
        self.cancel_in_flight();
        // Its answer would come on the old thread's channel, which goes.
        self.in_flight = None;
        self.worker.reset(shared);
    }

    /// Gives the quick view the session's highlighter, which colors code
    /// when the options say so; the app keeps one session for the run, as
    /// for the viewer.
    pub fn set_highlighter(&mut self, session: Session) {
        self.session = Some(session);
    }

    /// The options.
    pub fn options(&self) -> &QuickViewOptions {
        &self.options
    }

    /// Whether it is on.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// What it shows now.
    pub fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    /// The document it shows, when it shows one.
    pub fn view(&self) -> Option<&DocView> {
        match &self.shown.as_ref()?.showing {
            Showing::Document(view) => Some(view),
            _ => None,
        }
    }

    /// How many renders it has started.
    pub fn renders(&self) -> u64 {
        self.renders
    }

    /// The worker threads alive: none before the first render, then one.
    pub fn worker_threads(&self) -> usize {
        self.worker.threads()
    }

    /// The document readers running under the guard, those left past
    /// their deadline among them.
    pub fn readers_running(&self) -> usize {
        self.worker.shared().read.guard.running()
    }

    /// Whether it waits on something: a render due or running, colors
    /// being found. The frontend calls [`QuickView::tick`] while it does.
    pub fn wants_tick(&self) -> bool {
        self.on && (self.target.is_some() || self.in_flight.is_some() || self.job.is_some())
    }

    /// How long until the waiting render is due, for the frontend's timer;
    /// `None` when nothing waits.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        let target = self.target.as_ref()?;
        if target.now {
            return Some(Duration::ZERO);
        }
        Some((target.since + self.options.wait()).saturating_duration_since(now))
    }

    /// Turns it on or off. On, it shows the file under the cursor at once,
    /// without the wait: "Quick view on", then the preview's line when it
    /// is ready. Off, it drops what it shows: "Quick view off".
    pub fn toggle(&mut self, cursor: Option<(&Place, &Entry)>, now: Instant) -> Vec<Message> {
        if self.on {
            self.turn_off()
        } else {
            self.turn_on(cursor, now)
        }
    }

    /// Turns it on, showing the file under the cursor at once.
    pub fn turn_on(&mut self, cursor: Option<(&Place, &Entry)>, now: Instant) -> Vec<Message> {
        if self.on {
            return Vec::new();
        }
        self.on = true;
        if let Some((place, entry)) = cursor {
            self.target = Some(Target {
                place: place.clone(),
                entry: entry.clone(),
                since: now,
                now: true,
                reason: Reason::Moved,
            });
        }
        let mut said = vec![Message::info("Quick view on")];
        said.extend(self.tick(now));
        said
    }

    /// Turns it off: "Quick view off".
    pub fn turn_off(&mut self) -> Vec<Message> {
        if !self.on {
            return Vec::new();
        }
        self.on = false;
        self.target = None;
        self.cancel_in_flight();
        self.shown = None;
        self.job = None;
        self.rebuild_region(false);
        vec![Message::info("Quick view off")]
    }

    /// The active panel's cursor moved to an entry: its render waits for
    /// the cursor to stop. Says nothing; a move to what is shown already
    /// does nothing.
    pub fn cursor_moved(&mut self, place: &Place, entry: &Entry, now: Instant) -> Vec<Message> {
        if !self.on {
            return Vec::new();
        }
        if self.target.as_ref().is_some_and(|t| t.place == *place) {
            return Vec::new();
        }
        if let Some(flight) = self.in_flight.as_ref() {
            if flight.place == *place {
                // The render in flight is this file's.
                self.target = None;
                return Vec::new();
            }
            self.cancel_in_flight();
        }
        if self.shown.as_ref().is_some_and(|s| s.place == *place) {
            // Back on what is shown.
            self.target = None;
            return Vec::new();
        }
        self.target = Some(Target {
            place: place.clone(),
            entry: entry.clone(),
            since: now,
            now: false,
            reason: Reason::Moved,
        });
        Vec::new()
    }

    /// The panel's folder watch reported a change: when it is the file
    /// shown (or the folder must be read again), the file renders again
    /// after the wait, once however many changes come, and says "Preview
    /// updated: report.md changed". A removed file says so.
    pub fn watch_event(&mut self, event: &WatchEvent, now: Instant) -> Vec<Message> {
        if !self.on {
            return Vec::new();
        }
        let Some(shown) = self.shown.as_ref() else {
            return Vec::new();
        };
        match event {
            WatchEvent::Changed(p) | WatchEvent::Created(p) if *p == shown.place => {}
            WatchEvent::Rescan => {}
            WatchEvent::Removed(p) if *p == shown.place => {
                let name = shown.name.clone();
                self.shown = None;
                self.job = None;
                self.rebuild_region(false);
                if self.target.as_ref().is_some_and(|t| t.place == *p) {
                    self.target = None;
                }
                return vec![Message::info(format!("Preview: {name} was removed"))];
            }
            WatchEvent::Gone => {
                self.shown = None;
                self.job = None;
                self.rebuild_region(false);
                return Vec::new();
            }
            _ => return Vec::new(),
        }
        if !self.options.settings.follow_changes {
            return Vec::new();
        }
        if self.target.as_ref().is_some_and(|t| t.place != shown.place) {
            // The cursor already moved on; that render comes first.
            return Vec::new();
        }
        let mut entry = Entry::new(shown.name.clone(), diroxide_core::EntryKind::File);
        entry.size = shown.size;
        self.target = Some(Target {
            place: shown.place.clone(),
            entry,
            since: now,
            now: false,
            reason: Reason::Changed,
        });
        Vec::new()
    }

    /// Shows an entry at once, without the wait (the region's previous
    /// and next file).
    fn show_now(&mut self, place: &Place, entry: &Entry, now: Instant) {
        if self.in_flight.as_ref().is_some_and(|f| f.place != *place) {
            self.cancel_in_flight();
        }
        self.target = Some(Target {
            place: place.clone(),
            entry: entry.clone(),
            since: now,
            now: true,
            reason: Reason::Moved,
        });
    }

    /// Carries on: takes the renders that are done (a stale one is
    /// dropped), starts the waiting render once its wait is over and the
    /// worker is free, and puts the colors on code when they are ready.
    /// Returns what to say: one line per render, never the text.
    pub fn tick(&mut self, now: Instant) -> Vec<Message> {
        let mut said = Vec::new();
        for answer in self.worker.answers() {
            said.extend(self.take(answer));
        }
        if self.in_flight.is_some() && !self.worker.is_running() {
            // The worker stopped without an answer; the next render
            // starts it again.
            self.in_flight = None;
        }
        if self.on && self.in_flight.is_none() {
            let due = self.target.as_ref().is_some_and(|t| {
                t.now || now.saturating_duration_since(t.since) >= self.options.wait()
            });
            if due && let Some(target) = self.target.take() {
                said.extend(self.start(target));
            }
        }
        said.extend(self.poll_colors());
        said
    }

    /// Waits for the render in flight, at most `limit`, ticking: for tests
    /// and for a caller that may wait.
    pub fn settle(&mut self, now: Instant, limit: Duration) -> Vec<Message> {
        let mut said = self.tick(now);
        let until = Instant::now() + limit;
        while (self.in_flight.is_some() || self.target.as_ref().is_some_and(|t| t.now))
            && Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(5));
            said.extend(self.tick(now));
        }
        said
    }

    fn start(&mut self, target: Target) -> Vec<Message> {
        let generation = self.latest.next();
        let cancel = CancelToken::new();
        self.renders += 1;
        self.in_flight = Some(InFlight {
            generation,
            place: target.place.clone(),
            reason: target.reason,
            cancel: cancel.clone(),
        });
        let request = Request {
            generation,
            place: target.place,
            entry: target.entry,
            refresh: target.reason == Reason::Changed,
            cancel,
        };
        match self.worker.send(request) {
            Some(answer) => self.take(answer),
            None => Vec::new(),
        }
    }

    fn cancel_in_flight(&mut self) {
        if let Some(flight) = self.in_flight.as_ref() {
            flight.cancel.cancel();
            // A newer generation makes the answer stale wherever it is.
            self.latest.next();
        }
    }

    /// A finished render: installed and said when it is the newest,
    /// dropped when it is not.
    fn take(&mut self, answer: Answer) -> Vec<Message> {
        let Some(flight) = self
            .in_flight
            .take_if(|f| f.generation == answer.generation)
        else {
            return Vec::new();
        };
        if !self.on
            || !self.latest.is_current(answer.generation)
            || matches!(answer.rendered, Rendered::Superseded)
        {
            return Vec::new();
        }
        let name = display_name(&answer.entry.name).into_owned();
        let size = answer.entry.size;
        let mut said = Vec::new();
        let (showing, capped) = match answer.rendered {
            Rendered::Document {
                view,
                error,
                capped,
            } => {
                if let Some(error) = &error {
                    said.push(Message::from_error("read", &name, error));
                    said.push(Message::info(SHOWING_THE_TEXT));
                } else {
                    said.push(Message::info(match flight.reason {
                        Reason::Changed => format!("Preview updated: {name} changed"),
                        Reason::Moved if capped => self.capped_line(&name, size),
                        Reason::Moved => view.preview_line(&name),
                    }));
                }
                said.extend(view.notes().iter().map(Message::status));
                (Showing::Document(view), capped)
            }
            Rendered::NoPreview(words) => {
                said.push(Message::info(format!("No preview: {name}, {words}")));
                (Showing::NoPreview(words), false)
            }
            Rendered::Failed(error) => {
                let message = Message::from_error("read", &name, &error);
                let text = message.text.clone();
                said.push(message);
                (Showing::Unreadable(text), false)
            }
            Rendered::Superseded => return Vec::new(),
        };
        let same = self.shown.as_ref().is_some_and(|s| s.place == answer.place);
        self.shown = Some(Shown {
            place: answer.place,
            name,
            showing,
            size,
            capped,
        });
        self.rebuild_region(same && flight.reason == Reason::Changed);
        self.start_colors(size);
        said
    }

    /// "Preview: big.log, first 1 MiB of 2.0 GB", the cap in the unit the
    /// setting uses.
    fn capped_line(&self, name: &str, size: Option<u64>) -> String {
        let cap = cap_words(self.options.max_bytes());
        match size {
            Some(total) => format!("Preview: {name}, first {cap} of {}", human_size(total)),
            None => format!("Preview: {name}, first {cap}"),
        }
    }

    /// Starts coloring code, when the options and the session say so.
    fn start_colors(&mut self, size: Option<u64>) {
        self.job = None;
        if !self.options.colors_code() {
            return;
        }
        let (Some(session), Some(view)) = (self.session.as_ref(), self.view()) else {
            return;
        };
        if !matches!(view.kind, DocKind::Code(_)) {
            return;
        }
        let bytes = size.unwrap_or(0);
        if let Some(Ok(job)) = doc::text::highlight(view, session, self.options.caps(), bytes) {
            self.job = Some(job);
        }
    }

    /// Puts the colors on when they are ready; a refusal or an overrun is
    /// shown, not said (the viewer says it; the quick view's one line is
    /// its preview).
    fn poll_colors(&mut self) -> Vec<Message> {
        let Some(job) = self.job.as_mut() else {
            return Vec::new();
        };
        let Some(result) = job.poll() else {
            return Vec::new();
        };
        self.job = None;
        match result {
            Ok(done) => {
                if let Some(Shown {
                    showing: Showing::Document(view),
                    ..
                }) = self.shown.as_mut()
                {
                    let plain = std::mem::take(view);
                    *view = plain.with_runs(done.runs);
                }
                Vec::new()
            }
            Err(refusal) => vec![Message::status(refusal.line())],
        }
    }
}

/// A cap in the unit the settings use: "64 KiB", "1 MiB".
fn cap_words(bytes: u64) -> String {
    let kib = bytes / 1024;
    if kib >= 1024 && kib.is_multiple_of(1024) {
        format!("{} MiB", kib / 1024)
    } else {
        format!("{} KiB", diroxide_core::format::grouped(kib))
    }
}
