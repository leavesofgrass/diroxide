//! The tracker's state machine: one record a name in one folder, and the
//! settle rule. Pure: the caller does every look and gives the time.
//!
//! A record's state is one of:
//!
//! - **New**: seen once, not looked at again yet.
//! - **Growing**: the last look found it different, so its quiet timer
//!   started again.
//! - **Paused**: the last look found it the same; it settles once the
//!   quiet period has passed with no change and no writer holding it.
//! - **Settled**: it arrived, or it was there when watching began. A later
//!   change makes it grow again, and its next arrival is said "again".
//! - **Skipped**: a folder, a link, something that is not a file, or a
//!   file that is online only.
//! - **Gave up**: it kept changing for longer than the longest wait.
//!
//! A name that goes away is forgotten; a name that goes away while it
//! settles is reported gone.

use std::collections::HashMap;
use std::time::Duration;

use diroxide_core::sort::{NameCase, compare_names};
use diroxide_core::{Entry, EntryKind, Timestamp};

use super::between;
use super::ignore::is_ignored;

/// How long a file must stay the same before it counts as arrived, and how
/// often the settling files are looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettleRules {
    /// The quiet period: a file is ready once its size, modified time and
    /// identity have stayed the same this long.
    pub quiet: Duration,
    /// The quiet period of a name a producer renamed into the folder,
    /// finished elsewhere: enough to let the events pass, no more.
    pub renamed_quiet: Duration,
    /// How often the settling files are looked at, only while one is
    /// settling.
    pub tick: Duration,
    /// How long a file may keep changing before the tracker stops waiting
    /// for it ("Gave up waiting for app.log").
    pub max_wait: Duration,
}

impl SettleRules {
    /// A watched local folder: 2 seconds of quiet, star's number.
    pub const WATCHED: SettleRules = SettleRules {
        quiet: Duration::from_secs(2),
        renamed_quiet: Duration::from_millis(500),
        tick: Duration::from_millis(500),
        max_wait: Duration::from_secs(3600),
    };

    /// A place that is polled rather than watched (a network share): 10
    /// seconds of quiet, since a look sees less and coarse clocks hide
    /// small changes.
    pub const POLLED: SettleRules = SettleRules {
        quiet: Duration::from_secs(10),
        ..SettleRules::WATCHED
    };

    /// The same rules with another quiet period, for a producer that
    /// pauses longer than the default. A renamed-in name never waits
    /// longer than the quiet period.
    pub fn with_quiet(self, quiet: Duration) -> Self {
        Self {
            quiet,
            renamed_quiet: self.renamed_quiet.min(quiet),
            ..self
        }
    }
}

impl Default for SettleRules {
    fn default() -> Self {
        Self::WATCHED
    }
}

/// What a look at one name found about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    /// What the entry is.
    pub kind: EntryKind,
    /// The size, when known.
    pub size: Option<u64>,
    /// The modified time, when known.
    pub modified: Option<Timestamp>,
    /// The inode or file index, when known.
    pub inode: Option<u64>,
    /// The volume, when known.
    pub volume: Option<u64>,
    /// The file's data is not on this computer.
    pub online_only: bool,
    /// Whether a writer still holds the file open, when the look asked
    /// (the writer probe; `None` where the platform cannot tell).
    pub writer_open: Option<bool>,
}

impl Facts {
    /// The facts of a listed or looked-at entry.
    pub fn from_entry(entry: &Entry) -> Self {
        Self {
            kind: entry.kind,
            size: entry.size,
            modified: entry.modified,
            inode: entry.inode,
            volume: entry.volume,
            online_only: diroxide_vfs::facts::is_online_only(entry),
            writer_open: None,
        }
    }

    /// A plain file of this size, modified then, with nothing else known.
    pub fn file(size: u64, modified: Timestamp) -> Self {
        Self {
            kind: EntryKind::File,
            size: Some(size),
            modified: Some(modified),
            inode: None,
            volume: None,
            online_only: false,
            writer_open: None,
        }
    }

    /// The same facts, with what the writer probe found.
    pub fn with_writer_open(mut self, open: Option<bool>) -> Self {
        self.writer_open = open;
        self
    }

    /// Whether two looks found the same contents: the same kind, size,
    /// modified time and, where both looks know it, identity. A listing
    /// that leaves the identity unknown does not count as a change.
    pub fn same(&self, other: &Facts) -> bool {
        let same_known = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        self.kind == other.kind
            && self.size == other.size
            && self.modified == other.modified
            && self.online_only == other.online_only
            && same_known(self.inode, other.inode)
            && same_known(self.volume, other.volume)
    }

    /// Why the tracker leaves an entry with these facts alone, if it does.
    pub fn skip_reason(&self) -> Option<SkipReason> {
        match self.kind {
            EntryKind::Folder => Some(SkipReason::Folder),
            EntryKind::Link => Some(SkipReason::Link),
            EntryKind::Item | EntryKind::Special => Some(SkipReason::NotAFile),
            EntryKind::File | EntryKind::Archive if self.online_only => {
                Some(SkipReason::OnlineOnly)
            }
            EntryKind::File | EntryKind::Archive => None,
        }
    }
}

/// What a look at one name found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Look {
    /// Nothing has that name now.
    Absent,
    /// It could not be looked at this time (a passing problem): nothing is
    /// decided on this look.
    Unreadable,
    /// An entry, with its facts.
    Present(Facts),
}

/// Why an entry is not tracked as an arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// A folder: arrivals are files.
    Folder,
    /// A symbolic link, junction or shortcut: never followed.
    Link,
    /// A device, pipe or anything else that is not a file.
    NotAFile,
    /// A file whose data is not on this computer: reading it would
    /// download it.
    OnlineOnly,
}

impl SkipReason {
    /// The reason in words: "online only", "a folder".
    pub const fn words(self) -> &'static str {
        match self {
            SkipReason::Folder => "a folder",
            SkipReason::Link => "a link, not followed",
            SkipReason::NotAFile => "not a file",
            SkipReason::OnlineOnly => "online only",
        }
    }

    /// Whether the skip is said: only online-only files are, since a
    /// folder or a link arriving is not news a watch was asked for.
    pub const fn is_said(self) -> bool {
        matches!(self, SkipReason::OnlineOnly)
    }
}

/// Where one name stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrivalState {
    /// Seen once, not looked at again yet.
    New,
    /// The last look found it different.
    Growing,
    /// The last look found it the same.
    Paused,
    /// It arrived, or it was there when watching began.
    Settled,
    /// Not tracked, for this reason.
    Skipped(SkipReason),
    /// It kept changing for longer than the longest wait.
    GaveUp,
}

impl ArrivalState {
    /// Whether a name in this state is still settling.
    pub const fn is_settling(self) -> bool {
        matches!(
            self,
            ArrivalState::New | ArrivalState::Growing | ArrivalState::Paused
        )
    }
}

/// A file that settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    /// Its name in the folder.
    pub name: String,
    /// Its size when it settled, when known.
    pub size: Option<u64>,
    /// The name had settled before while watching, or was there when
    /// watching began: its contents arrived again.
    pub again: bool,
    /// How long it took from the first sighting to settling.
    pub waited: Duration,
}

/// Something the tracker decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackerEvent {
    /// A file settled.
    Arrived(Arrival),
    /// An entry is skipped, for this reason.
    Skipped {
        /// Its name.
        name: String,
        /// Why.
        reason: SkipReason,
    },
    /// A file kept changing for longer than the longest wait; it is left
    /// alone from now on.
    GaveUp {
        /// Its name.
        name: String,
    },
    /// A name went away before it settled.
    Gone {
        /// Its name.
        name: String,
    },
}

#[derive(Clone, Debug)]
struct Record {
    state: ArrivalState,
    facts: Facts,
    /// When this round of settling began: the first sighting, or the
    /// change that woke a settled file.
    first_seen: Timestamp,
    /// When a look last found the facts different.
    since: Timestamp,
    /// This record's quiet period.
    quiet: Duration,
    /// It settled before, or was there at the start.
    known: bool,
}

/// The arrival tracker for one folder. See the module documentation.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    rules: SettleRules,
    records: HashMap<String, Record>,
    arrived: usize,
}

impl Tracker {
    /// A tracker with these rules and nothing seen yet.
    pub fn new(rules: SettleRules) -> Self {
        Self {
            rules,
            records: HashMap::new(),
            arrived: 0,
        }
    }

    /// The rules it settles by.
    pub fn rules(&self) -> SettleRules {
        self.rules
    }

    /// Takes what was in the folder when watching began: none of it is an
    /// arrival, though a later change to a file is.
    pub fn baseline(&mut self, entries: impl IntoIterator<Item = (String, Facts)>, now: Timestamp) {
        for (name, facts) in entries {
            if is_ignored(&name) {
                continue;
            }
            let state = match facts.skip_reason() {
                Some(reason) => ArrivalState::Skipped(reason),
                None => ArrivalState::Settled,
            };
            self.records.insert(
                name,
                Record {
                    state,
                    facts,
                    first_seen: now,
                    since: now,
                    quiet: self.rules.quiet,
                    known: true,
                },
            );
        }
    }

    /// Takes a look at one name, made because a watch event named it or a
    /// listing showed it: a new name gets a record, a change starts its
    /// quiet timer again, an absence forgets it.
    pub fn observe(&mut self, name: &str, look: Look, now: Timestamp) -> Vec<TrackerEvent> {
        if is_ignored(name) {
            return Vec::new();
        }
        match look {
            Look::Unreadable => Vec::new(),
            Look::Absent => self.forget(name),
            Look::Present(facts) => match self.records.get(name) {
                Some(_) => self.update(name, facts, now),
                None => self.first_sighting(name, facts, now, self.rules.quiet),
            },
        }
    }

    /// Takes a name that a producer renamed into the folder, finished: it
    /// settles after the shorter quiet period. An existing name renamed
    /// over counts as a change.
    pub fn renamed_in(&mut self, name: &str, look: Look, now: Timestamp) -> Vec<TrackerEvent> {
        if is_ignored(name) {
            return Vec::new();
        }
        match (self.records.contains_key(name), look) {
            (false, Look::Present(facts)) => {
                self.first_sighting(name, facts, now, self.rules.renamed_quiet)
            }
            (_, look) => self.observe(name, look, now),
        }
    }

    /// Takes a full listing of the folder: the names in it are looked at,
    /// and the names missing from it are gone.
    pub fn reconcile(
        &mut self,
        listing: impl IntoIterator<Item = (String, Facts)>,
        now: Timestamp,
    ) -> Vec<TrackerEvent> {
        let mut events = Vec::new();
        let mut present = std::collections::HashSet::new();
        for (name, facts) in listing {
            events.extend(self.observe(&name, Look::Present(facts), now));
            present.insert(name);
        }
        let mut missing: Vec<String> = self
            .records
            .keys()
            .filter(|name| !present.contains(*name))
            .cloned()
            .collect();
        sort_natural(&mut missing);
        for name in missing {
            events.extend(self.observe(&name, Look::Absent, now));
        }
        events
    }

    /// The tick: looks at every settling name again, through `look`, and
    /// settles the ones whose quiet period has passed with no change and
    /// no writer holding them. `look` is told whether the name's quiet
    /// period has passed, so it can ask the writer probe only then. Files
    /// that settle on one tick arrive in natural name order.
    pub fn tick(
        &mut self,
        now: Timestamp,
        look: &mut dyn FnMut(&str, bool) -> Look,
    ) -> Vec<TrackerEvent> {
        let mut events = Vec::new();
        let mut ready = Vec::new();
        for name in self.waiting() {
            let Some(record) = self.records.get_mut(&name) else {
                continue;
            };
            if between(record.first_seen, now) >= self.rules.max_wait {
                record.state = ArrivalState::GaveUp;
                events.push(TrackerEvent::GaveUp { name });
                continue;
            }
            let due = between(record.since, now) >= record.quiet;
            match look(&name, due) {
                Look::Unreadable => continue,
                Look::Absent => {
                    events.extend(self.forget(&name));
                    continue;
                }
                Look::Present(facts) => events.extend(self.update(&name, facts, now)),
            }
            if let Some(record) = self.records.get(&name)
                && record.state == ArrivalState::Paused
                && between(record.since, now) >= record.quiet
                && record.facts.writer_open != Some(true)
            {
                ready.push(name);
            }
        }
        for name in ready {
            let Some(record) = self.records.get_mut(&name) else {
                continue;
            };
            let arrival = Arrival {
                size: record.facts.size,
                again: record.known,
                waited: between(record.first_seen, now),
                name,
            };
            record.state = ArrivalState::Settled;
            record.known = true;
            self.arrived += 1;
            events.push(TrackerEvent::Arrived(arrival));
        }
        events
    }

    /// Whether any name is settling, so the tick should run.
    pub fn is_settling(&self) -> bool {
        self.records.values().any(|r| r.state.is_settling())
    }

    /// The names settling now, in natural order: what the status line
    /// says it waits for.
    pub fn waiting(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .records
            .iter()
            .filter(|(_, r)| r.state.is_settling())
            .map(|(name, _)| name.clone())
            .collect();
        sort_natural(&mut names);
        names
    }

    /// Where a name stands, if the tracker knows it.
    pub fn state(&self, name: &str) -> Option<ArrivalState> {
        self.records.get(name).map(|r| r.state)
    }

    /// How many arrivals there have been.
    pub fn arrived(&self) -> usize {
        self.arrived
    }

    /// How many names it holds a record for.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether it holds no record.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn first_sighting(
        &mut self,
        name: &str,
        facts: Facts,
        now: Timestamp,
        quiet: Duration,
    ) -> Vec<TrackerEvent> {
        let skip = facts.skip_reason();
        let state = match skip {
            Some(reason) => ArrivalState::Skipped(reason),
            None => ArrivalState::New,
        };
        self.records.insert(
            name.to_owned(),
            Record {
                state,
                facts,
                first_seen: now,
                since: now,
                quiet,
                known: false,
            },
        );
        match skip {
            Some(reason) => vec![TrackerEvent::Skipped {
                name: name.to_owned(),
                reason,
            }],
            None => Vec::new(),
        }
    }

    fn update(&mut self, name: &str, facts: Facts, now: Timestamp) -> Vec<TrackerEvent> {
        let quiet = self.rules.quiet;
        let Some(record) = self.records.get_mut(name) else {
            return Vec::new();
        };
        if record.state == ArrivalState::GaveUp {
            record.facts = facts;
            return Vec::new();
        }
        if let Some(reason) = facts.skip_reason() {
            let newly = record.state != ArrivalState::Skipped(reason);
            record.state = ArrivalState::Skipped(reason);
            record.facts = facts;
            return if newly {
                vec![TrackerEvent::Skipped {
                    name: name.to_owned(),
                    reason,
                }]
            } else {
                Vec::new()
            };
        }
        let changed = !record.facts.same(&facts);
        let was = record.state;
        record.facts = facts;
        let restart = match was {
            // No longer skipped (an online-only file whose data came).
            ArrivalState::Skipped(_) => true,
            // A settled file changed.
            ArrivalState::Settled => changed,
            _ => false,
        };
        if restart {
            // A new round, with the usual quiet period and the longest
            // wait from now.
            record.state = ArrivalState::Growing;
            record.first_seen = now;
            record.since = now;
            record.quiet = quiet;
            return Vec::new();
        }
        match was {
            ArrivalState::New | ArrivalState::Growing | ArrivalState::Paused if changed => {
                record.state = ArrivalState::Growing;
                record.since = now;
            }
            ArrivalState::New | ArrivalState::Growing => record.state = ArrivalState::Paused,
            _ => {}
        }
        Vec::new()
    }

    fn forget(&mut self, name: &str) -> Vec<TrackerEvent> {
        match self.records.remove(name) {
            Some(record) if record.state.is_settling() => vec![TrackerEvent::Gone {
                name: name.to_owned(),
            }],
            _ => Vec::new(),
        }
    }
}

/// Sorts names in natural order ("scan-9.pdf" before "scan-10.pdf"),
/// letters without regard to case.
fn sort_natural(names: &mut [String]) {
    names.sort_by(|a, b| compare_names(a, b, NameCase::Ignore, true));
}
