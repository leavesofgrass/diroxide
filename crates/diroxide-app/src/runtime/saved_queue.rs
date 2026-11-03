//! The queue saved across restarts (lane Q2): the engine's waiting, held
//! and running jobs written to `queue.json` in the state folder, beside
//! the ledger of partial files, on every change, and removed when the
//! queue is empty.
//!
//! - **Off the input thread.** [`QueueFile::save`] turns the jobs into
//!   text and hands it to one writer thread, which writes only the latest
//!   text it has (to `queue.json.new`, flushed, then renamed over the old
//!   file), so a burst of changes is one write and a crash leaves the old
//!   file or the new one, never half of either.
//! - **Read once, at start** ([`QueueFile::open`]): every job found there
//!   is interrupted now. Nothing starts on its own; the Jobs list offers
//!   Retry and Forget, and the jobs stay in the file until one of those.
//! - **A file that does not read** is said in words and renamed aside
//!   (`queue.json.bad`, then `queue.json.bad2` and on), never deleted;
//!   when it cannot be renamed, nothing is saved this session, so it is
//!   never written over.
//! - **One session keeps the queue.** `queue.lock` beside it is held
//!   locked for the session; a second session on the same state folder
//!   finds it taken and neither reads nor writes the queue.
//! - **Quit** writes the queue as it stands, marked as a quit, and stops
//!   writing ([`QueueFile::freeze`]): the jobs the quit stops are the
//!   ones the next start lists.
//!
//! The file, `{"version": 1, "quit": true, "jobs": [...]}`, keeps each
//! job's id, kind, state (`waiting`, `held`, `running` or
//! `interrupted`), whether it had started, its words ("12 items to
//! build") and its request when the queue knew it.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use diroxide_core::{JobId, JobKind, JobRequest, Message};
use serde::{Deserialize, Serialize};

/// The saved queue's file in the state folder.
pub(crate) const QUEUE_FILE: &str = "queue.json";
/// The lock the session that keeps the queue holds.
pub(crate) const QUEUE_LOCK: &str = "queue.lock";
/// The file's version; a newer one is set aside like a damaged file.
const VERSION: u32 = 1;
/// How long quit waits for the last write.
const FLUSH_WAIT: Duration = Duration::from_secs(2);

/// Where a saved job was when the queue was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SavedState {
    /// Waiting its turn, never started.
    Waiting,
    /// Held in its place by Pause, never started.
    Held,
    /// Started: running, paused, asking, or waiting for its host.
    Running,
    /// Interrupted by an earlier end, not yet retried or forgotten.
    Interrupted,
}

/// One job in the saved queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedJob {
    /// The id it had in its session.
    pub job: JobId,
    /// What kind of job it is.
    pub kind: JobKind,
    /// Where it was.
    pub state: SavedState,
    /// Whether it had started.
    #[serde(default)]
    pub started: bool,
    /// What it is on: "12 items to build".
    #[serde(default)]
    pub what: String,
    /// Its request, when the queue knew it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<JobRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedFile {
    version: u32,
    /// Whether a quit wrote it last, rather than a change while running.
    #[serde(default)]
    quit: bool,
    jobs: Vec<SavedJob>,
}

/// What the start read from the saved queue.
#[derive(Debug, Default)]
pub(crate) struct Loaded {
    /// The jobs found, each interrupted now, in the saved order.
    pub jobs: Vec<SavedJob>,
    /// How many of them the last session's end interrupted.
    pub new: usize,
    /// Whether a quit wrote the file last.
    pub by_quit: bool,
    /// A file that did not read, in words.
    pub problem: Option<Message>,
}

/// What the writer thread is told.
enum Order {
    /// Write this text, or remove the file when there is none.
    Write(Option<String>),
    /// Say when everything before this is on the disk.
    Flush(Sender<()>),
}

/// The saved queue's file for this session: the writer, the lock, and
/// what was written last.
pub(crate) struct QueueFile {
    orders: Sender<Order>,
    /// Held for the session, so a second session leaves the file alone.
    _lock: Option<File>,
    /// The text last handed to the writer; `None` when the file is gone.
    last: Option<String>,
    /// Set at quit: nothing more is written.
    frozen: bool,
}

impl std::fmt::Debug for QueueFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueFile")
            .field("frozen", &self.frozen)
            .finish_non_exhaustive()
    }
}

/// An I/O error's cause in lowercase words, for the house style.
fn cause(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "permission denied".to_owned(),
        io::ErrorKind::StorageFull => "the disk is full".to_owned(),
        io::ErrorKind::ReadOnlyFilesystem => "the disk is read-only".to_owned(),
        _ => {
            let text = error.to_string();
            let mut chars = text.chars();
            match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect(),
                None => "an unknown error".to_owned(),
            }
        }
    }
}

/// Renames a damaged file aside, never over another: `queue.json.bad`,
/// then `queue.json.bad2` and on. Returns the new name.
fn set_aside(path: &Path) -> io::Result<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(QUEUE_FILE)
        .to_owned();
    for n in 1..=99 {
        let candidate = if n == 1 {
            format!("{name}.bad")
        } else {
            format!("{name}.bad{n}")
        };
        let aside = path.with_file_name(&candidate);
        if aside.exists() {
            continue;
        }
        fs::rename(path, &aside)?;
        return Ok(candidate);
    }
    Err(io::Error::other(
        "ninety-nine damaged copies are already kept",
    ))
}

/// Takes the lock beside the file: `Ok(Some(file))` held, `Ok(None)` when
/// the folder cannot lock (the queue is kept without one), `Err` when
/// another session holds it.
fn take_lock(folder: &Path) -> Result<Option<File>, ()> {
    if fs::create_dir_all(folder).is_err() {
        return Ok(None);
    }
    let Ok(file) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(QUEUE_LOCK))
    else {
        return Ok(None);
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Err(()),
        Err(TryLockError::Error(_)) => Ok(None),
    }
}

/// The writer: writes the latest text it has, atomically, and says a
/// failure once until a write succeeds again.
fn writer(path: PathBuf, orders: Receiver<Order>, report: Box<dyn Fn(Message) + Send>) {
    let mut failed = false;
    while let Ok(first) = orders.recv() {
        let mut latest = None;
        let mut flushes = Vec::new();
        let mut take = |order: Order| match order {
            Order::Write(text) => latest = Some(text),
            Order::Flush(done) => flushes.push(done),
        };
        take(first);
        while let Ok(next) = orders.try_recv() {
            take(next);
        }
        if let Some(text) = latest {
            let result = match text {
                Some(text) => diroxide_config::write_atomic(&path, &text),
                None => match fs::remove_file(&path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            match result {
                Ok(()) => failed = false,
                Err(error) if !failed => {
                    failed = true;
                    report(Message::could_not(
                        "save",
                        "the job queue",
                        &cause(&error),
                        "The jobs go on, but a restart will not list them",
                    ));
                }
                Err(_) => {}
            }
        }
        for done in flushes {
            let _ = done.send(());
        }
    }
}

impl QueueFile {
    /// Opens the saved queue in a state folder: takes its lock, reads it,
    /// and starts its writer. `None` when another session keeps the
    /// queue, or when a file that cannot be read could not be set aside
    /// (said in [`Loaded::problem`]); nothing is saved then.
    pub(crate) fn open(
        folder: &Path,
        report: Box<dyn Fn(Message) + Send>,
    ) -> (Option<Self>, Loaded) {
        let Ok(lock) = take_lock(folder) else {
            return (None, Loaded::default());
        };
        let path = folder.join(QUEUE_FILE);
        let mut loaded = Loaded::default();
        let mut last = None;
        match fs::read_to_string(&path) {
            Ok(text) => match read(&text) {
                Some(file) => {
                    loaded.by_quit = file.quit;
                    for mut job in file.jobs {
                        if job.state != SavedState::Interrupted {
                            loaded.new += 1;
                            job.started |= job.state == SavedState::Running;
                            job.state = SavedState::Interrupted;
                        }
                        loaded.jobs.push(job);
                    }
                    last = Some(text);
                }
                None => match set_aside(&path) {
                    Ok(aside) => {
                        loaded.problem = Some(Message::could_not(
                            "read",
                            "the saved job queue",
                            "the file is damaged",
                            &format!("It is kept as {aside} in the state folder"),
                        ));
                    }
                    Err(error) => {
                        loaded.problem = Some(Message::could_not(
                            "read",
                            "the saved job queue",
                            &format!("the file is damaged and {}", cause(&error)),
                            "Jobs are not saved this session. Move queue.json out of the state folder",
                        ));
                        return (None, loaded);
                    }
                },
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                loaded.problem = Some(Message::could_not(
                    "read",
                    "the saved job queue",
                    &cause(&error),
                    "Jobs are not saved this session",
                ));
                return (None, loaded);
            }
        }
        let (orders, receiver) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("diroxide-queue-file".to_owned())
            .spawn(move || writer(path, receiver, report));
        if spawned.is_err() {
            return (None, loaded);
        }
        let file = Self {
            orders,
            _lock: lock,
            last,
            frozen: false,
        };
        (Some(file), loaded)
    }

    /// Writes the queue as it stands, off this thread; nothing when it is
    /// what was written last, the file removed when there is no job.
    pub(crate) fn save(&mut self, jobs: &[SavedJob]) {
        self.write(jobs, false);
    }

    /// Writes the queue as it stands at quit, marked so, and stops
    /// writing: the jobs the quit stops stay in the file.
    pub(crate) fn freeze(&mut self, jobs: &[SavedJob]) {
        self.write(jobs, true);
        self.frozen = true;
    }

    /// Waits until every write handed over is on the disk, at most
    /// `limit`; whether it was.
    pub(crate) fn flush(&self, limit: Duration) -> bool {
        let (done, wait) = mpsc::channel();
        if self.orders.send(Order::Flush(done)).is_err() {
            return false;
        }
        wait.recv_timeout(limit).is_ok()
    }

    fn write(&mut self, jobs: &[SavedJob], quit: bool) {
        if self.frozen {
            return;
        }
        let text = if jobs.is_empty() {
            None
        } else {
            let file = SavedFile {
                version: VERSION,
                quit,
                jobs: jobs.to_vec(),
            };
            match serde_json::to_string_pretty(&file) {
                Ok(text) => Some(text),
                Err(_) => return,
            }
        };
        if text == self.last {
            return;
        }
        self.last.clone_from(&text);
        let _ = self.orders.send(Order::Write(text));
    }
}

impl Drop for QueueFile {
    /// The last change reaches the disk before the session ends, within
    /// two seconds.
    fn drop(&mut self) {
        self.flush(FLUSH_WAIT);
    }
}

/// The saved queue in a text, when it reads and is not from a newer
/// diroxide.
fn read(text: &str) -> Option<SavedFile> {
    let file: SavedFile = serde_json::from_str(text).ok()?;
    (file.version <= VERSION).then_some(file)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use diroxide_core::Place;

    use super::*;

    fn folder() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("queue-file-")
            .tempdir()
            .unwrap()
    }

    type Said = Arc<Mutex<Vec<String>>>;

    fn reporter() -> (Said, Box<dyn Fn(Message) + Send>) {
        let said: Said = Arc::default();
        let into = Arc::clone(&said);
        let report = Box::new(move |message: Message| {
            into.lock().unwrap().push(message.text);
        });
        (said, report)
    }

    fn job(n: u64, state: SavedState) -> SavedJob {
        let source = Place::parse("sftp://build/srv/report.iso").unwrap();
        let request = JobRequest::new(JobId::new(n), JobKind::Copy, vec![source]);
        SavedJob {
            job: JobId::new(n),
            kind: JobKind::Copy,
            state,
            started: state == SavedState::Running,
            what: "report.iso to build".to_owned(),
            request: Some(request),
        }
    }

    fn wait_for(what: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if what() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        what()
    }

    #[test]
    fn the_queue_is_written_on_change_removed_when_empty_and_read_back() {
        let dir = folder();
        let path = dir.path().join(QUEUE_FILE);
        let (_, report) = reporter();
        let (file, loaded) = QueueFile::open(dir.path(), report);
        let mut file = file.expect("the lock is free");
        assert!(loaded.jobs.is_empty() && loaded.problem.is_none());
        file.save(&[job(1, SavedState::Running), job(2, SavedState::Held)]);
        assert!(file.flush(Duration::from_secs(10)));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"state\": \"held\""), "{text}");
        assert!(!dir.path().join("queue.json.new").exists());
        file.save(&[]);
        assert!(file.flush(Duration::from_secs(10)));
        assert!(!path.exists(), "an empty queue removes the file");
        file.freeze(&[job(3, SavedState::Waiting), job(4, SavedState::Running)]);
        file.save(&[]);
        drop(file);
        assert!(path.exists(), "nothing is written after the quit's queue");

        // The next start reads both, each interrupted now, by a quit.
        let (_, report) = reporter();
        let (file, loaded) = QueueFile::open(dir.path(), report);
        assert!(file.is_some());
        assert_eq!(loaded.new, 2);
        assert!(loaded.by_quit);
        let states: Vec<(u64, SavedState, bool)> = loaded
            .jobs
            .iter()
            .map(|j| (j.job.get(), j.state, j.started))
            .collect();
        assert_eq!(
            states,
            [
                (3, SavedState::Interrupted, false),
                (4, SavedState::Interrupted, true)
            ]
        );
        assert_eq!(loaded.jobs[0].request, job(3, SavedState::Waiting).request);
    }

    #[test]
    fn a_second_session_leaves_the_queue_alone() {
        let dir = folder();
        let (_, report) = reporter();
        let (first, _) = QueueFile::open(dir.path(), report);
        let mut first = first.expect("the lock is free");
        first.save(&[job(1, SavedState::Waiting)]);
        assert!(first.flush(Duration::from_secs(10)));
        let (_, report) = reporter();
        let (second, loaded) = QueueFile::open(dir.path(), report);
        assert!(second.is_none(), "the first session keeps the queue");
        assert!(loaded.jobs.is_empty(), "nothing is read under its feet");
        drop(first);
        let (_, report) = reporter();
        let (third, loaded) = QueueFile::open(dir.path(), report);
        assert!(third.is_some());
        assert_eq!(loaded.new, 1);
        assert!(!loaded.by_quit, "no quit wrote it: a crash or a kill");
    }

    #[test]
    fn a_damaged_file_is_said_and_set_aside_never_deleted() {
        let dir = folder();
        let path = dir.path().join(QUEUE_FILE);
        fs::write(&path, b"{ not a queue").unwrap();
        fs::write(dir.path().join("queue.json.bad"), b"older").unwrap();
        let (_, report) = reporter();
        let (file, loaded) = QueueFile::open(dir.path(), report);
        assert!(file.is_some(), "the queue is kept again from now");
        let problem = loaded.problem.expect("said");
        assert_eq!(
            problem.text,
            "Could not read the saved job queue: the file is damaged. \
             It is kept as queue.json.bad2 in the state folder."
        );
        assert!(diroxide_core::format::within_cells(
            &problem.text,
            "Could not read the saved job queue"
        ));
        assert_eq!(
            fs::read(dir.path().join("queue.json.bad2")).unwrap(),
            b"{ not a queue"
        );
        assert_eq!(
            fs::read(dir.path().join("queue.json.bad")).unwrap(),
            b"older"
        );
        assert!(!path.exists());
        assert!(loaded.jobs.is_empty());

        // A file from a newer diroxide is set aside the same way.
        fs::write(&path, br#"{"version": 9, "jobs": []}"#).unwrap();
        drop(file);
        let (_, report) = reporter();
        let (_, loaded) = QueueFile::open(dir.path(), report);
        assert!(loaded.problem.is_some());
        assert!(dir.path().join("queue.json.bad3").exists());
    }

    #[test]
    fn a_write_that_fails_is_said_once() {
        let dir = folder();
        let (said, report) = reporter();
        let (file, _) = QueueFile::open(dir.path(), report);
        let mut file = file.unwrap();
        // A folder where the file goes: the rename over it fails.
        fs::create_dir(dir.path().join(QUEUE_FILE)).unwrap();
        file.save(&[job(1, SavedState::Waiting)]);
        file.save(&[job(1, SavedState::Held)]);
        assert!(file.flush(Duration::from_secs(10)));
        file.save(&[job(1, SavedState::Waiting)]);
        assert!(file.flush(Duration::from_secs(10)));
        assert!(wait_for(|| !said.lock().unwrap().is_empty()));
        let said = said.lock().unwrap().clone();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("Could not save the job queue: "),
            "{said:?}"
        );
        assert!(said[0].ends_with("The jobs go on, but a restart will not list them."));
    }
}
