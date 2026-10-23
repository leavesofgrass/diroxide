//! What the app asks of the runtime beyond the core's effects.
//!
//! The core's [`Effect`](diroxide_core::Effect) names the I/O every
//! frontend shares (list a place, start a job, cancel it, open a file).
//! The app needs a few more conversations with its runtime: a job is
//! planned first and run only after the user confirms the frozen plan; a
//! running job's question is answered; a folder is watched; the session,
//! a prompt's history and a changed setting are written to disk; a program
//! runs with the screen handed over. The app queues these as [`Request`]s
//! and [`Runtime::carry_out`](crate::Runtime::carry_out) takes them, so a
//! frontend's loop stays one call: effects in, screen effects out.
//!
//! A request never holds a secret, except [`Reply::Secret`], which goes
//! straight to the job that asked and is never logged or serialized
//! (ADR-0009).

use std::fmt;

use diroxide_config::SessionState;
use diroxide_core::{
    ConflictAnswer, ErrorAction, JobId, JobRequest, Place, Secret, SecretRequest, Side,
};
use diroxide_ops::{Conflict, ItemError, Plan, UndoRecord};

/// One question a job or a listing is waiting on, numbered by the runtime
/// so the answer finds its way back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AskId(u64);

impl AskId {
    /// Wraps a raw number.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "question {}", self.0)
    }
}

/// What a job or a listing asks the user, on a worker thread that waits
/// for the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// A name is taken (ADR-0004): the lines and the nine verb-first
    /// choices come from the conflict itself.
    Conflict {
        /// The job that asks.
        job: JobId,
        /// The clash.
        conflict: Box<Conflict>,
        /// Whether a replace keeps the old file in the trash, so Undo can
        /// bring it back; when false the question adds "Replacing cannot be
        /// undone."
        replace_undoable: bool,
    },
    /// An item failed: Retry, Skip, Skip all like this or Cancel.
    Error {
        /// The job that asks.
        job: JobId,
        /// The failure.
        error: Box<ItemError>,
    },
    /// A password or passphrase (ADR-0009).
    Secret {
        /// The job that asks, or `None` for a listing.
        job: Option<JobId>,
        /// What is asked.
        request: SecretRequest,
    },
    /// A content search reached a file whose data is not on this computer,
    /// and the scope says to read such files: reading downloads it. Asked
    /// once a search; the answer is [`Reply::Confirm`].
    OnlineOnly {
        /// The file's name.
        name: String,
        /// Its size, when known.
        size: Option<u64>,
    },
    /// A host whose connection did not come back within its budget of
    /// tries (R25, 2.4): one question per host, asked by the job that
    /// probes for every job waiting on it; Keep waiting first. The answer
    /// is [`Reply::Connection`].
    Connection {
        /// The job that asks for every job on the host.
        job: JobId,
        /// The host and its waiting jobs.
        question: diroxide_ops::ConnectionQuestion,
    },
    /// An archive limit's question ("Expands to 40 GB from 40 KB. Unpack?
    /// y or n"), asked once per kind in a job, starting on No; the answer
    /// is [`Reply::Confirm`].
    Limit {
        /// The job that asks, or `None` for a listing.
        job: Option<JobId>,
        /// The question.
        question: diroxide_archive::Question,
    },
}

impl Question {
    /// The job that asks, when a job does.
    pub fn job(&self) -> Option<JobId> {
        match self {
            Question::Conflict { job, .. }
            | Question::Error { job, .. }
            | Question::Connection { job, .. } => Some(*job),
            Question::Secret { job, .. } | Question::Limit { job, .. } => *job,
            Question::OnlineOnly { .. } => None,
        }
    }

    /// Whether the job that asks gives its queue slot back while it waits
    /// for the answer (R25, 2.2): every job's question does, except the
    /// connection question, whose job probes for every job on its host.
    pub fn gives_slot_back(&self) -> bool {
        self.job().is_some() && !matches!(self, Question::Connection { .. })
    }
}

/// The answer to a [`Question`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The answer to a conflict.
    Conflict(ConflictAnswer),
    /// The answer to an item's error.
    Error(ErrorAction),
    /// A typed secret, or `None` when the user gave none.
    Secret(Option<Secret>),
    /// Yes or no, to a limit's question.
    Confirm(bool),
    /// The answer to the connection question.
    Connection(diroxide_ops::ConnectionAnswer),
}

/// An archive job planned and waiting to run: what the pack, unpack and
/// test dialogs confirm (plan commits 60 and 61). A pack's password,
/// typed twice in its dialog, travels with its plan to the runner and is
/// dropped with the job; it is never serialized or shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchivePlan {
    /// An unpack, or a copy out of an archive, with the answers given to
    /// its limit questions while it was planned.
    Unpack {
        /// The plan.
        plan: Box<diroxide_archive::UnpackPlan>,
        /// The limits the user let pass.
        allow: diroxide_archive::Allowances,
    },
    /// A pack.
    Pack {
        /// The plan.
        plan: Box<diroxide_archive::PackPlan>,
        /// The new archive's password, when one was set.
        password: Option<Secret>,
    },
}

impl ArchivePlan {
    /// The job.
    pub fn job(&self) -> JobId {
        match self {
            ArchivePlan::Unpack { plan, .. } => plan.job,
            ArchivePlan::Pack { plan, .. } => plan.job,
        }
    }

    /// The question before it runs, starting on No: "Unpack 4 members
    /// into out? y or n", "Pack 2 items into reports.zip? y or n".
    pub fn question(&self) -> diroxide_core::Message {
        match self {
            ArchivePlan::Unpack { plan, .. } => plan.question(),
            ArchivePlan::Pack { plan, .. } => plan.question(),
        }
    }

    /// The plan's rows, each a labeled setting with its meaning first.
    pub fn rows(&self, other_panel: bool) -> Vec<String> {
        match self {
            ArchivePlan::Unpack { plan, .. } => plan.rows(),
            ArchivePlan::Pack { plan, .. } => plan.rows_for(other_panel),
        }
    }

    /// The job request kept in the jobs list: no password in it.
    pub fn job_request(&self) -> JobRequest {
        match self {
            ArchivePlan::Unpack { plan, .. } => plan.job_request(),
            ArchivePlan::Pack { plan, .. } => plan.job_request(),
        }
    }
}

/// What the program that runs is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    /// The user's shell, interactive, in the folder.
    Shell,
    /// A command line the user typed, run by the user's shell.
    CommandLine(String),
    /// The editor, on one file.
    Editor(Place),
    /// The editor, on one file, at a line (a search result): editors that
    /// take `+LINE` get it; others open the file.
    EditorAt {
        /// The file.
        file: Place,
        /// The line, from 1.
        line: u64,
    },
    /// A program the tools registry found, with its arguments and no
    /// shell: "Open in textweaver" (plan commit 64).
    Program {
        /// What it is called when it is said: "textweaver".
        name: String,
        /// The program.
        program: std::path::PathBuf,
        /// Its arguments, each one argument.
        args: Vec<std::ffi::OsString>,
    },
}

/// A program to run with the screen handed over: the run-command prompt,
/// shell here, and the F4 editor. The context goes in environment
/// variables and a NUL-separated list file, never into the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    /// What runs.
    pub kind: SpawnKind,
    /// The working folder: the active panel's folder.
    pub folder: Place,
    /// The other panel's folder (`DIRO_OTHER`).
    pub other: Place,
    /// The focused entry (`DIRO_CURSOR`).
    pub cursor: Option<Place>,
    /// The marked entries, or the focused one (`DIRO_SELECTION_FILE`).
    pub selection: Vec<Place>,
}

/// The app's clock, handed to a worker that reads date words (a content
/// filter, a search), so "today" means the app's day. Two are equal when
/// they are the same clock.
#[derive(Clone, Debug)]
pub struct SharedClock(pub std::sync::Arc<dyn diroxide_core::Clock>);

impl PartialEq for SharedClock {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A request from the app to its runtime. See the module documentation.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Request {
    /// Plan a job without touching anything; the answer comes back as
    /// [`AppEvent::Planned`](crate::AppEvent::Planned). Counting progress
    /// arrives as job progress meanwhile.
    Plan(Box<JobRequest>),
    /// Plugins and script commands: scan, enable, run, answer a plugin's
    /// call (W4d).
    Plugins(Box<crate::plugins::PluginRequest>),
    /// Optional components: list, download, verify, remove, install
    /// (W4d).
    Components(Box<crate::components::ComponentRequest>),
    /// Plan the undo of a job's record; answered like [`Request::Plan`].
    PlanUndo {
        /// The undo job's id.
        job: JobId,
        /// What the job to undo did.
        record: Box<UndoRecord>,
    },
    /// Run a plan the user confirmed, exactly as it was shown: the queue
    /// starts it now or when its turn comes, and says which as
    /// [`AppEvent::Queue`](crate::AppEvent::Queue).
    Run(Box<Plan>),
    /// One of the Jobs list's actions, or a queue setting (lane A2).
    Queue(crate::jobs::QueueRequest),
    /// Answer a question a worker waits on.
    Answer {
        /// The question.
        ask: AskId,
        /// The answer.
        reply: Reply,
    },
    /// Watch a panel's folder, replacing that panel's earlier watch, or
    /// stop watching with `None`.
    Watch {
        /// The panel.
        side: Side,
        /// The folder, or `None`.
        place: Option<Place>,
    },
    /// List the drives, the root, the home folder and the mounts, for the
    /// go-to-place list.
    Places,
    /// Write the session to the state folder.
    SaveSession(Box<SessionState>),
    /// Write a prompt's history (newest first) to the state folder.
    SaveHistory {
        /// The prompt's history key: "go-path", "rename".
        key: String,
        /// The entries, newest first.
        entries: Vec<String>,
    },
    /// Write one setting to `config.toml`, or put it back to its default
    /// with `None`.
    SaveSetting {
        /// The setting's dotted key.
        key: String,
        /// Its new value.
        value: Option<toml::Value>,
    },
    /// Mark an interrupted job of the last session as heard.
    Settle(JobId),
    /// Run a program with the screen handed over.
    Spawn(Box<Spawn>),
    /// Plan an unpack, or a copy out of an archive, on a worker: the
    /// archive opened (its password asked through the app, three tries)
    /// and each limit's question asked once; answered as
    /// [`AppEvent::ArchivePlanned`](crate::AppEvent::ArchivePlanned).
    PlanUnpack {
        /// The job.
        job: JobId,
        /// What to unpack and where.
        request: Box<diroxide_archive::UnpackRequest>,
    },
    /// Plan a pack on a worker; answered like [`Request::PlanUnpack`].
    PlanPack {
        /// The job.
        job: JobId,
        /// What to pack and where, with its password.
        request: Box<diroxide_archive::PackRequest>,
    },
    /// Run an archive job the user confirmed, exactly as planned; its end
    /// comes as [`AppEvent::ArchiveFinished`](crate::AppEvent::ArchiveFinished).
    RunArchive(Box<ArchivePlan>),
    /// Test an archive: read every member and check it, writing nothing.
    TestArchive {
        /// The job.
        job: JobId,
        /// The archive file.
        archive: Place,
    },
    /// Read a file's bytes for the viewer on a worker (a seekable read
    /// where the provider has one, a stream kept up to `cap` otherwise; a
    /// locked member asks its password); answered as
    /// [`AppEvent::ViewerSource`](crate::AppEvent::ViewerSource).
    OpenViewer {
        /// The request, so a late answer to an old one is dropped.
        request: diroxide_core::RequestId,
        /// The file.
        place: Place,
        /// Its listed size, for the large-file policy.
        size: Option<u64>,
        /// The bytes a stream keeps at most.
        cap: u64,
    },
    /// Count an archive's entries for the details list, on a worker;
    /// answered as [`AppEvent::ArchiveEntries`](crate::AppEvent::ArchiveEntries).
    ArchiveEntries(Place),
    /// Drop the passwords an archive job's plan holds: its confirmation
    /// said no, or it was never run.
    DropArchiveJob(JobId),
    /// Make the archive provider forget an archive's cached index, and
    /// with it a password a locked 7z keeps while it is open: the panel
    /// left it (W2b's notes).
    ForgetArchive(Place),
    /// Sweep what an interrupted run left, at start: the spool folder's
    /// spool files, and the temporary archives packs left in these
    /// folders ([`crate::archives::sweep`]).
    Sweep {
        /// The folders packs wrote into.
        folders: Vec<Place>,
    },
    /// Write the Keys list's change to `keymap.toml`, one line at a time,
    /// keeping the owner's comments (W3k); a failure comes back in words.
    EditKeymap(Box<crate::keymap::file::KeymapEdit>),
    /// Write the labels to `labels.toml` in the state folder (W3l).
    SaveLabels(Box<crate::labels::LabelStore>),
    /// Write the bookmarks to `bookmarks.toml` in the state folder (W3l).
    SaveBookmarks(Box<crate::bookmarks::Bookmarks>),
    /// Write the folder views to `folder-views.toml` in the state folder
    /// (W3l).
    SaveViews(Box<crate::views::FolderViews>),
    /// Find which of these places are gone, on a worker, for the Labels
    /// list's missing counts (W3l); answered as
    /// [`AppEvent::PlacesChecked`](crate::AppEvent::PlacesChecked).
    CheckPlaces {
        /// The request, so a late answer to an old one is dropped.
        request: diroxide_core::RequestId,
        /// The places to look for.
        places: Vec<Place>,
    },
    /// Watch a folder for arriving files, on a thread of the runtime,
    /// replacing an earlier watch (W3t); its notes come back as
    /// [`AppEvent::Arrivals`](crate::AppEvent::Arrivals) tagged with
    /// `watch`.
    WatchArrivals {
        /// The watch's number.
        watch: u64,
        /// The folder.
        place: Place,
        /// The least time between two paced lines.
        pace: std::time::Duration,
    },
    /// Stop watching for arriving files (W3t).
    StopArrivals,
    /// Run a search on a worker, stopping the one before (W3a): its hits
    /// come back in the order found as
    /// [`AppEvent::SearchFound`](crate::AppEvent::SearchFound), its
    /// progress as [`AppEvent::SearchProgress`](crate::AppEvent::SearchProgress),
    /// and its end as [`AppEvent::SearchDone`](crate::AppEvent::SearchDone).
    Search {
        /// The search's number.
        search: u64,
        /// What to look for and where.
        query: Box<diroxide_search::SearchQuery>,
        /// The app's clock, for the date words.
        clock: SharedClock,
    },
    /// Look up a label's items for the results lister (W3a); answered as
    /// [`AppEvent::LabeledFound`](crate::AppEvent::LabeledFound).
    StatPlaces {
        /// The search's number.
        search: u64,
        /// The places.
        places: Vec<Place>,
    },
    /// Compare two files on a worker (W3a); answered as
    /// [`AppEvent::Compared`](crate::AppEvent::Compared), with
    /// [`AppEvent::CompareProgress`](crate::AppEvent::CompareProgress)
    /// meanwhile.
    Compare {
        /// The request.
        request: diroxide_core::RequestId,
        /// The file read from.
        before: Place,
        /// The file read to.
        after: Place,
        /// What it ignores, its limits.
        options: diroxide_diff::FileOptions,
    },
    /// Write a saved search to the state folder's `saved` folder (W3a).
    SaveSearch(Box<diroxide_search::Saved>),
    /// Read the saved searches (W3a); answered as
    /// [`AppEvent::SavedListed`](crate::AppEvent::SavedListed).
    ListSaved {
        /// The request.
        request: diroxide_core::RequestId,
    },
    /// Delete a saved search's file, only inside the `saved` folder (W3a).
    DeleteSaved(std::path::PathBuf),
    /// Count a folder's Space list (W3a); answered as
    /// [`AppEvent::SpaceListed`](crate::AppEvent::SpaceListed).
    SpaceList {
        /// The request.
        request: diroxide_core::RequestId,
        /// The folder.
        place: Place,
        /// By items, by kind or by age.
        grouping: diroxide_search::Grouping,
        /// The app's clock, for the ages.
        clock: SharedClock,
    },
    /// Find the largest items below a folder (W3a); answered as
    /// [`AppEvent::LargestFound`](crate::AppEvent::LargestFound).
    Largest {
        /// The results' search number.
        search: u64,
        /// The folder.
        place: Place,
        /// How many.
        limit: usize,
    },
    /// Size folders (W3a); answered as
    /// [`AppEvent::DirSized`](crate::AppEvent::DirSized).
    DirSizes {
        /// The request.
        request: diroxide_core::RequestId,
        /// The folders.
        places: Vec<Place>,
    },
    /// Find the newest files (W3a); answered as
    /// [`AppEvent::NewestFound`](crate::AppEvent::NewestFound).
    Newest {
        /// The results' search number.
        search: u64,
        /// Where.
        scope: Box<diroxide_search::Scope>,
        /// The day to look before, for Earlier day.
        before: Option<diroxide_search::Day>,
        /// The app's clock.
        clock: SharedClock,
    },
    /// Stop the running search: its summary says where it stopped.
    StopSearch,
    /// Hold the running search (`true`) or let it go on (`false`).
    PauseSearch(bool),
    /// Check a result's file when its row takes the focus (W3a); answered
    /// as [`AppEvent::StaleChecked`](crate::AppEvent::StaleChecked).
    CheckStale {
        /// The search.
        search: u64,
        /// The result's row.
        row: String,
        /// The result.
        hit: Box<diroxide_search::SearchHit>,
    },
    /// Read the files of a panel's folder for its content filter, on a
    /// worker (W3a): the filter is run over the folder alone, and the
    /// names it matched come back as
    /// [`AppEvent::FilterContents`](crate::AppEvent::FilterContents).
    FilterContents {
        /// The request, so a late answer to an old one is dropped.
        request: diroxide_core::RequestId,
        /// The panel.
        side: Side,
        /// The panel's folder.
        place: Place,
        /// The filter.
        filter: Box<diroxide_core::filter::Filter>,
        /// Whether hidden files are read too.
        hidden: bool,
        /// The app's clock, for the date words.
        clock: SharedClock,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::SecretKind;

    #[test]
    fn a_reply_never_shows_its_secret() {
        let reply = Reply::Secret(Some(Secret::new("canary-7f3a-not-a-password".to_owned())));
        let request = Request::Answer {
            ask: AskId::new(3),
            reply,
        };
        assert!(!format!("{request:?}").contains("canary"));
        let question = Question::Secret {
            job: None,
            request: SecretRequest::new(SecretKind::ArchivePassword, "course.zip"),
        };
        assert_eq!(question.job(), None);
        assert_eq!(AskId::new(7).to_string(), "question 7");
    }
}
