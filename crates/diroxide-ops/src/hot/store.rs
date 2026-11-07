//! A task's state, in its own folder under the state folder's `hot`
//! (`hot/scans/`), never in the task file the owner writes: whether it is
//! on, the approval key, the answer to the first catch-up question and why
//! it is paused. The ledger, the log and the lock live beside it.
//!
//! State files are small TOML, written whole to `NAME.new` and renamed
//! into place, so a crash leaves the old file or the new one, never half.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use diroxide_core::Timestamp;
use serde::{Deserialize, Serialize};

use super::approval::Approval;
use super::ledger::{LEDGER_FILE, Ledger};
use super::log::{LOG_FILE, TaskLog};
use super::task::CatchUpChoice;
use crate::convert::recipe::is_plain_id;

/// The state file's name in a task's folder.
pub const STATE_FILE: &str = "state.toml";

/// The lock file's name in a task's folder.
pub const LOCK_FILE: &str = "lock";

/// A task's lock, held while it lives ([`TaskStore::lock`]).
#[derive(Debug)]
pub struct TaskLock {
    _file: fs::File,
}

/// What a task's state file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TaskState {
    /// Whether the owner turned it on.
    pub on: bool,
    /// The approval key the owner said yes to.
    pub approved: Option<String>,
    /// When, in seconds since 1970 (UTC).
    pub approved_at: Option<i64>,
    /// The answer to the first catch-up question: `all` or `none`.
    pub catch_up: Option<String>,
    /// Why it is paused, in words, until the owner resumes it.
    pub paused: Option<String>,
}

impl TaskState {
    /// Where the task stands with the approval its key needs now.
    pub fn approval(&self, key: &str) -> Approval {
        Approval::of(self.approved.as_deref(), key)
    }

    /// The catch-up answer remembered, if the question was asked.
    pub fn catch_up(&self) -> Option<CatchUpChoice> {
        self.catch_up
            .as_deref()
            .and_then(CatchUpChoice::from_word)
            .filter(|c| *c != CatchUpChoice::Ask)
    }
}

/// Why a task's folder could not be used.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The task's name is not plain, so it cannot name a folder.
    #[error("the task's name {0} cannot name a folder")]
    Name(String),
    /// The folder or a file in it could not be read or written.
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// A task's folder under the state folder's `hot`. See the module
/// documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskStore {
    folder: PathBuf,
    name: String,
}

impl TaskStore {
    /// The folder of the task named `task` under `hot` (the state folder's
    /// `hot`); nothing is made until something is written. The name must
    /// be plain, as every checked task's is.
    pub fn new(hot: &Path, task: &str) -> Result<Self, StoreError> {
        if !is_plain_id(task) {
            return Err(StoreError::Name(task.to_owned()));
        }
        Ok(Self {
            folder: hot.join(task),
            name: task.to_owned(),
        })
    }

    /// The task's folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The task's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The task's state; the defaults (off, never approved) when there is
    /// none or it cannot be read.
    pub fn state(&self) -> TaskState {
        fs::read_to_string(self.folder.join(STATE_FILE))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the task's state, whole.
    pub fn save_state(&self, state: &TaskState) -> Result<(), StoreError> {
        let text = toml::to_string(state).map_err(io::Error::other)?;
        self.write_whole(STATE_FILE, text.as_bytes())?;
        Ok(())
    }

    /// Notes the owner's yes to the task as it is now, keyed by `key`.
    pub fn approve(&self, key: &str, now: Timestamp) -> Result<(), StoreError> {
        let mut state = self.state();
        state.approved = Some(key.to_owned());
        state.approved_at = Some(now.unix_seconds());
        self.save_state(&state)
    }

    /// Turns the task on or off; turning it on clears a pause.
    pub fn set_on(&self, on: bool) -> Result<(), StoreError> {
        let mut state = self.state();
        state.on = on;
        if on {
            state.paused = None;
        }
        self.save_state(&state)
    }

    /// Remembers the answer to the first catch-up question.
    pub fn remember_catch_up(&self, choice: CatchUpChoice) -> Result<(), StoreError> {
        let mut state = self.state();
        state.catch_up = Some(choice.word().to_owned());
        self.save_state(&state)
    }

    /// Notes why the task is paused, or that it is not (`None`).
    pub fn set_paused(&self, reason: Option<&str>) -> Result<(), StoreError> {
        let mut state = self.state();
        state.paused = reason.map(str::to_owned);
        self.save_state(&state)
    }

    /// The path of a file in the task's folder.
    pub fn file(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }

    /// The task's ledger, read.
    pub fn ledger(&self) -> Result<Ledger, StoreError> {
        Ok(Ledger::open(self.file(LEDGER_FILE))?)
    }

    /// The task's log.
    pub fn log(&self) -> TaskLog {
        TaskLog::new(self.file(LOG_FILE))
    }

    /// Takes the task's lock (R16 2.10): an exclusive lock on
    /// `hot/NAME/lock`, held for as long as the returned [`TaskLock`]
    /// lives. A second runner, in this program or another `diro`, gets
    /// `Ok(None)`. The lock is the open file, not its existence, so a
    /// crashed runner never leaves a stale one.
    pub fn lock(&self) -> Result<Option<TaskLock>, StoreError> {
        fs::create_dir_all(&self.folder)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.file(LOCK_FILE))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(TaskLock { _file: file })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Writes a file in the task's folder whole: to `NAME.new`, flushed,
    /// then renamed into place. The folder is made when missing.
    pub(crate) fn write_whole(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let path = self.folder.join(name);
        let new = self.folder.join(format!("{name}.new"));
        {
            let mut file = fs::File::create(&new)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::rename(&new, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::testing::scratch;

    #[test]
    fn state_starts_off_and_keeps_the_approval_and_the_answers() {
        let dir = scratch();
        let store = TaskStore::new(dir.path(), "scans").unwrap();
        assert_eq!(store.state(), TaskState::default());
        assert_eq!(store.state().approval("k1"), Approval::NotYet);
        store
            .approve("k1", Timestamp::from_unix(1_790_000_000, 0))
            .unwrap();
        store.set_on(true).unwrap();
        store.remember_catch_up(CatchUpChoice::None).unwrap();
        store
            .set_paused(Some("pdftotext is not installed"))
            .unwrap();
        let state = store.state();
        assert!(state.on);
        assert_eq!(state.approval("k1"), Approval::Approved);
        assert_eq!(state.approval("k2"), Approval::Changed);
        assert_eq!(state.approved_at, Some(1_790_000_000));
        assert_eq!(state.catch_up(), Some(CatchUpChoice::None));
        assert_eq!(state.paused.as_deref(), Some("pdftotext is not installed"));
        // Turning on again clears the pause.
        store.set_on(true).unwrap();
        assert_eq!(store.state().paused, None);
        assert!(!store.file("state.toml.new").exists());
        assert!(TaskStore::new(dir.path(), "../up").is_err());
        assert!(TaskStore::new(dir.path(), "").is_err());
    }

    #[test]
    fn a_second_runner_finds_the_task_locked_until_the_first_ends() {
        let dir = scratch();
        let store = TaskStore::new(dir.path(), "scans").unwrap();
        let first = store.lock().unwrap();
        assert!(first.is_some());
        assert!(store.lock().unwrap().is_none());
        drop(first);
        assert!(store.lock().unwrap().is_some());
    }

    #[test]
    fn a_state_file_that_is_not_toml_reads_as_off() {
        let dir = scratch();
        let store = TaskStore::new(dir.path(), "scans").unwrap();
        fs::create_dir_all(store.folder()).unwrap();
        fs::write(store.file(STATE_FILE), "on = maybe\n").unwrap();
        assert_eq!(store.state(), TaskState::default());
    }
}
