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
            Question::Conflict { job, .. } | Question::Error { job, .. } => Some(*job),
            Question::Secret { job, .. } | Question::Limit { job, .. } => *job,
        }
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

/// A request from the app to its runtime. See the module documentation.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Request {
    /// Plan a job without touching anything; the answer comes back as
    /// [`AppEvent::Planned`](crate::AppEvent::Planned). Counting progress
    /// arrives as job progress meanwhile.
    Plan(Box<JobRequest>),
    /// Plan the undo of a job's record; answered like [`Request::Plan`].
    PlanUndo {
        /// The undo job's id.
        job: JobId,
        /// What the job to undo did.
        record: Box<UndoRecord>,
    },
    /// Run a plan the user confirmed, exactly as it was shown.
    Run(Box<Plan>),
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
