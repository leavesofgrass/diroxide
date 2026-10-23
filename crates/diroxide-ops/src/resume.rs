//! Resume of a partial file (R25, 2.5): a copy cut by a lost connection
//! goes on where it stopped, but only when everything agrees.
//!
//! - **The ledger** ([`Ledger`]) lists the partial files a kept write left:
//!   the target, the unfinished place, the source, the source's size and
//!   modified time, the bytes written, and the job. A line is written when
//!   the kept write starts, so a crash leaves it too, updated when a
//!   connection is lost, and removed on success, on Cancel, and when a
//!   resume is refused. A leftover line of a job that is not running is
//!   listed ([`Ledger::leftovers`]), never deleted unasked.
//! - **Resume only when everything agrees**: the same target
//!   and source; the source's size and time equal the ledger's; the part
//!   file exists and is not longer than the source; and the window check
//!   passes: the first 64 KiB and the last 1 MiB of the part file equal the
//!   same ranges of the source. Anything else removes the part file and
//!   starts again, saying why ([`JobNotice::NotResumed`]).
//! - **Cancel removes the part file**; a lost connection, a pause or a
//!   crash keeps it.
//! - **A move never trusts a resumed prefix**: the run hashes the source's
//!   prefix again before the read-back, so resume saves the writing of the
//!   prefix, never its reading.
//!
//! The ledger is generic over providers: it holds places, and the part
//! file is reached through [`ResumableWrite`], which the SFTP plugin's
//! provider answers too (lane S-B).

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use diroxide_core::format::human_size;
use diroxide_core::{Entry, JobId, Place, Timestamp};
use diroxide_vfs::provider::ResumableWrite;
use diroxide_vfs::{OpContext, Provider, VfsError};
use serde::{Deserialize, Serialize};

/// The head of the window check: the first 64 KiB.
pub const WINDOW_HEAD: u64 = 64 * 1024;
/// The tail of the window check: the last 1 MiB.
pub const WINDOW_TAIL: u64 = 1024 * 1024;

/// The ledger file's name in the state folder.
pub const LEDGER_FILE: &str = "partials.json";

/// One partial file the ledger knows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partial {
    /// The job that wrote it.
    pub job: JobId,
    /// The file it is to become.
    pub target: Place,
    /// Where its bytes are until the commit: the unfinished name.
    pub unfinished: Place,
    /// What it is a copy of.
    pub source: Place,
    /// The source's size when the copy started.
    pub source_size: u64,
    /// The source's modified time when the copy started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_modified: Option<Timestamp>,
    /// The bytes written when the line was last updated; the part file's
    /// own length is what a resume reads.
    pub bytes: u64,
}

/// Why a part file was not resumed: said, then the file starts again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotResumed {
    /// The source's size or time differs from the ledger's.
    SourceChanged,
    /// The part file is longer than the source.
    TooLong,
    /// The window check failed: the part file's first 64 KiB or last 1 MiB
    /// differ from the source.
    Differs,
    /// The part file is not there any more, or cannot be read.
    Gone,
}

impl NotResumed {
    /// The reason, as its line: "Not resumed: the source changed." (32
    /// cells, R25).
    pub const fn line(self) -> &'static str {
        match self {
            NotResumed::SourceChanged => "Not resumed: the source changed.",
            NotResumed::TooLong => "Not resumed: the part file is too long.",
            NotResumed::Differs => "Not resumed: the part file differs.",
            NotResumed::Gone => "Not resumed: the part file is gone.",
        }
    }
}

/// What a running job says beside its progress, once, at the moment it
/// happens (lane A2 says each line; the default sink ignores them).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobNotice {
    /// A file goes on where it stopped.
    Resuming {
        /// The file's name.
        name: String,
        /// Where it goes on, in bytes.
        at: u64,
        /// Its size.
        of: u64,
    },
    /// A part file was not resumed; the file starts again from the start.
    NotResumed {
        /// The file's name.
        name: String,
        /// Why.
        why: NotResumed,
    },
}

impl JobNotice {
    /// The lines to say, meaning first: "Resuming report.iso at 410 MB of
    /// 800 MB"; "Not resumed: the source changed.", then "Starting again
    /// from the start.".
    pub fn lines(&self) -> Vec<String> {
        match self {
            JobNotice::Resuming { name, at, of } => vec![format!(
                "Resuming {name} at {} of {}",
                human_size(*at),
                human_size(*of)
            )],
            JobNotice::NotResumed { why, .. } => vec![
                why.line().to_owned(),
                "Starting again from the start.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Default)]
struct State {
    lines: Vec<Partial>,
    live: HashSet<JobId>,
}

/// The ledger of partial files: in memory, or kept in a file in the state
/// folder so the next start finds what a crash left.
#[derive(Debug, Default)]
pub struct Ledger {
    file: Option<PathBuf>,
    state: Mutex<State>,
}

impl Ledger {
    /// A ledger kept in memory only (tests, and a run with no state
    /// folder).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The ledger in a state folder: its file is read now, and written
    /// again on every change. A missing file is an empty ledger; one that
    /// cannot be read is an error, and nothing is overwritten.
    pub fn open(folder: &Path) -> io::Result<Self> {
        let file = folder.join(LEDGER_FILE);
        let lines = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            file: Some(file),
            state: Mutex::new(State {
                lines,
                live: HashSet::new(),
            }),
        })
    }

    /// The partial files of jobs that are not running: what a crash, or a
    /// job that ended on a lost connection, left. Listed for the owner,
    /// never removed unasked.
    pub fn leftovers(&self) -> Vec<Partial> {
        let state = self.lock();
        state
            .lines
            .iter()
            .filter(|line| !state.live.contains(&line.job))
            .cloned()
            .collect()
    }

    /// Every line, running jobs' included.
    pub fn lines(&self) -> Vec<Partial> {
        self.lock().lines.clone()
    }

    /// Forgets the line of a part file the owner removed (or kept) by
    /// hand.
    pub fn forget(&self, unfinished: &Place) -> io::Result<()> {
        self.change(|lines| lines.retain(|line| line.unfinished != *unfinished))
    }

    // ----- for the run -----

    /// A running job's lines are its own: another job does not resume
    /// them.
    pub(crate) fn claim(&self, job: JobId) {
        self.lock().live.insert(job);
    }

    /// The job ended: its lines, if any are left, are leftovers.
    pub(crate) fn release(&self, job: JobId) {
        self.lock().live.remove(&job);
    }

    /// The line to resume a copy of `source` to `target` from: this job's
    /// own, else one of a job that is not running.
    pub(crate) fn find(&self, job: JobId, target: &Place, source: &Place) -> Option<Partial> {
        let state = self.lock();
        let matches = |line: &&Partial| line.target == *target && line.source == *source;
        state
            .lines
            .iter()
            .filter(matches)
            .find(|line| line.job == job)
            .or_else(|| {
                state
                    .lines
                    .iter()
                    .filter(matches)
                    .find(|line| !state.live.contains(&line.job))
            })
            .cloned()
    }

    /// Adds or replaces the line of a part file.
    pub(crate) fn record(&self, partial: Partial) -> io::Result<()> {
        self.change(|lines| {
            lines.retain(|line| line.unfinished != partial.unfinished);
            lines.push(partial);
        })
    }

    /// The bytes written to a part file, and the job that owns it now.
    pub(crate) fn update(&self, unfinished: &Place, job: JobId, bytes: u64) -> io::Result<()> {
        self.change(|lines| {
            if let Some(line) = lines.iter_mut().find(|line| line.unfinished == *unfinished) {
                line.job = job;
                line.bytes = bytes;
            }
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn change(&self, edit: impl FnOnce(&mut Vec<Partial>)) -> io::Result<()> {
        let mut state = self.lock();
        edit(&mut state.lines);
        let Some(file) = &self.file else {
            return Ok(());
        };
        let text = serde_json::to_vec_pretty(&state.lines)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let new = file.with_extension("json.new");
        fs::write(&new, text)?;
        fs::rename(&new, file)
    }
}

/// Whether a part file may be resumed: its length when everything agrees,
/// else why not. `fresh` is the source as it is now. A call that fails on
/// a lost connection is the outer error: nothing is decided, and the try
/// waits for the connection like any other.
pub(crate) fn check(
    line: &Partial,
    fresh: &Entry,
    source: &dyn Provider,
    resumable: &dyn ResumableWrite,
    ctx: &OpContext,
) -> Result<Result<u64, NotResumed>, VfsError> {
    // A lost connection decides nothing; every other failure refuses.
    let lost = |error: VfsError, why: NotResumed| -> Result<NotResumed, VfsError> {
        if keeps_part(&error) || error == VfsError::Cancelled {
            Err(error)
        } else {
            Ok(why)
        }
    };
    if fresh.size != Some(line.source_size) || fresh.modified != line.source_modified {
        return Ok(Err(NotResumed::SourceChanged));
    }
    let length = match resumable.unfinished_len(&line.unfinished, ctx) {
        Ok(length) => length,
        Err(error) => return Ok(Err(lost(error, NotResumed::Gone)?)),
    };
    if length > line.source_size {
        return Ok(Err(NotResumed::TooLong));
    }
    let head = length.min(WINDOW_HEAD);
    let tail_from = length.saturating_sub(WINDOW_TAIL);
    let mut reader = match source.open_seekable(&line.source, ctx) {
        Ok(reader) => reader,
        Err(error) => return Ok(Err(lost(error, NotResumed::Differs)?)),
    };
    for (from, to) in [(0, head), (tail_from, length)] {
        let Ok(size) = usize::try_from(to - from) else {
            return Ok(Err(NotResumed::Differs));
        };
        let part = match resumable.read_unfinished(&line.unfinished, from, size, ctx) {
            Ok(part) => part,
            Err(error) => return Ok(Err(lost(error, NotResumed::Gone)?)),
        };
        let mut original = vec![0u8; size];
        let read = reader
            .seek(SeekFrom::Start(from))
            .and_then(|_| reader.read_exact(&mut original));
        if let Err(error) = read {
            return Ok(Err(lost(VfsError::from(error), NotResumed::Differs)?));
        }
        if part != original {
            return Ok(Err(NotResumed::Differs));
        }
    }
    Ok(Ok(length))
}

/// Whether an error leaves a part file worth keeping: a lost connection.
pub(crate) fn keeps_part(error: &VfsError) -> bool {
    error.class().retries_on_its_own()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn place(text: &str) -> Place {
        Place::parse(text).unwrap()
    }

    fn line(job: u64, unfinished: &str) -> Partial {
        Partial {
            job: JobId::new(job),
            target: place("sftp://build/srv/big.iso"),
            unfinished: place(&format!("sftp://build/srv/{unfinished}")),
            source: place("sftp://store/big.iso"),
            source_size: 800_000_000,
            source_modified: Some(Timestamp::from_unix(1_790_000_000, 0)),
            bytes: 410_000_000,
        }
    }

    #[test]
    fn the_lines_fit_and_say_the_fact_first() {
        let resuming = JobNotice::Resuming {
            name: "report.iso".into(),
            at: 410_000_000,
            of: 800_000_000,
        };
        assert_eq!(
            resuming.lines(),
            ["Resuming report.iso at 410 MB of 800 MB"]
        );
        for why in [
            NotResumed::SourceChanged,
            NotResumed::TooLong,
            NotResumed::Differs,
            NotResumed::Gone,
        ] {
            let notice = JobNotice::NotResumed {
                name: "report.iso".into(),
                why,
            };
            let lines = notice.lines();
            assert!(within_cells(&lines[0], "Not resumed"), "{}", lines[0]);
            assert!(lines[0].chars().count() <= 40, "{}", lines[0]);
            assert_eq!(lines[1], "Starting again from the start.");
        }
    }

    #[test]
    fn a_job_finds_its_own_line_or_a_leftover_never_a_running_jobs() {
        let ledger = Ledger::in_memory();
        let a = line(1, ".diro-part-00000001");
        ledger.record(a.clone()).unwrap();
        ledger.claim(JobId::new(1));
        assert_eq!(
            ledger.find(JobId::new(1), &a.target, &a.source),
            Some(a.clone())
        );
        assert_eq!(
            ledger.find(JobId::new(2), &a.target, &a.source),
            None,
            "a running job's part file is its own"
        );
        assert!(ledger.leftovers().is_empty());
        ledger.release(JobId::new(1));
        assert_eq!(ledger.leftovers(), std::slice::from_ref(&a));
        assert_eq!(
            ledger.find(JobId::new(3), &a.target, &a.source),
            Some(a.clone())
        );
        ledger.update(&a.unfinished, JobId::new(3), 5).unwrap();
        assert_eq!(ledger.lines()[0].job, JobId::new(3));
        ledger.forget(&a.unfinished).unwrap();
        assert!(ledger.lines().is_empty());
    }

    #[test]
    fn the_ledger_survives_a_restart_in_its_folder() {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("ops-unit-tmp");
        fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("ledger-")
            .tempdir_in(base)
            .unwrap();
        let ledger = Ledger::open(dir.path()).unwrap();
        assert!(ledger.lines().is_empty(), "no file is an empty ledger");
        ledger.record(line(1, ".diro-part-0000000a")).unwrap();
        ledger.record(line(2, ".diro-part-0000000b")).unwrap();
        drop(ledger);
        let again = Ledger::open(dir.path()).unwrap();
        assert_eq!(again.leftovers().len(), 2, "what a crash left is listed");
        fs::write(dir.path().join(LEDGER_FILE), b"not json").unwrap();
        assert!(Ledger::open(dir.path()).is_err());
    }
}
