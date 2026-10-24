//! The undo journal in the state folder (research/bulk-rename.md, "The
//! journal"; research/file-management-lessons.md, "Interrupted processes").
//!
//! Before a job's first step the engine writes a header (the job, its
//! kind, its origin, its step count) and makes it durable; after each step
//! it appends one line with the step's outcome and what it added to the
//! undo record; at the end it appends an end line with what the job left
//! behind. A job whose journal has no end line was interrupted (a crash,
//! a kill, a power cut): the next start reads it and says so in words, and
//! its record can still be undone. Finished journals are the undo log
//! after a restart; the oldest beyond twenty are removed.
//!
//! One file per job, `journal/job-00000000000000000042.jsonl`, plain names
//! only. The folder is the app's state folder, injected; tests give a
//! temporary folder and never touch the real one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use diroxide_core::format::count;
use diroxide_core::{JobId, JobKind, JobOrigin, Timestamp};
use serde::{Deserialize, Serialize};

use crate::undo::{LeftAs, UndoRecord};

/// How many finished journals are kept.
pub const KEEP_FINISHED: usize = 20;

/// The journal folder inside a state folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    folder: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Header {
    journal: u32,
    job: JobId,
    kind: JobKind,
    #[serde(default)]
    origin: JobOrigin,
    steps: u64,
    #[serde(default)]
    started: Option<Timestamp>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Line {
    End {
        end: String,
        #[serde(default)]
        left: Vec<LeftAs>,
    },
    Step {
        step: u64,
        outcome: String,
        #[serde(default)]
        moves: Vec<(diroxide_core::Place, diroxide_core::Place)>,
        #[serde(default)]
        trashed: Vec<diroxide_core::Place>,
        #[serde(default)]
        created: Vec<diroxide_core::Place>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attributes: Vec<(diroxide_core::Place, diroxide_vfs::AttributeChange)>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        relinked: Vec<crate::undo::Relinked>,
    },
    /// A write begun: its target and the unfinished name its bytes go to
    /// until the commit (`FileWriter::unfinished_place`).
    Writing {
        writing: diroxide_core::Place,
        unfinished: diroxide_core::Place,
    },
}

/// A job the last session did not finish, as its journal tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interrupted {
    /// The job.
    pub job: JobId,
    /// What kind of job it was.
    pub kind: JobKind,
    /// The steps it planned.
    pub steps: u64,
    /// The steps that finished, done or not.
    pub finished: u64,
    /// The steps done.
    pub done: u64,
    /// What it did that can be undone.
    pub record: UndoRecord,
    /// The write it was in the middle of, as (target, unfinished name):
    /// the unfinished file is the job's, left beside its target. It is
    /// never removed on its own; the next start lists it.
    pub unfinished: Vec<(diroxide_core::Place, diroxide_core::Place)>,
}

impl Interrupted {
    /// The notice for the next start, the fact first: "Last session ended
    /// during a move: 3 of 5 done."
    pub fn sentence(&self) -> String {
        let noun = match self.kind {
            JobKind::Copy => "a copy",
            JobKind::Move => "a move",
            JobKind::Rename => "a rename",
            JobKind::Trash => "a move to the trash",
            JobKind::DeletePermanently => "a permanent delete",
            JobKind::Duplicate => "a duplicate",
            JobKind::MakeFolder => "making folders",
            JobKind::Link => "making links",
            JobKind::Pack => "a pack",
            JobKind::Unpack => "an unpack",
            JobKind::TestArchive => "an archive test",
            JobKind::Split => "a split",
            JobKind::Join => "a join",
            JobKind::Undo => "an undo",
            JobKind::Restore => "a restore from the trash",
            JobKind::Purge => "a delete from the trash",
            _ => "a job",
        };
        format!(
            "Last session ended during {noun}: {} of {} done.",
            self.done,
            count(self.steps, "step", "steps")
        )
    }
}

/// Writes one job's journal, a line per step.
#[derive(Debug)]
pub(crate) struct JournalWriter {
    file: File,
}

impl JournalWriter {
    fn line<T: Serialize>(&mut self, line: &T) -> io::Result<()> {
        let mut text = serde_json::to_string(line).map_err(io::Error::other)?;
        text.push('\n');
        self.file.write_all(text.as_bytes())?;
        self.file.flush()
    }

    /// Appends a step's outcome and what it added to the undo record.
    pub(crate) fn step(&mut self, step: u64, outcome: &str, delta: &UndoRecord) -> io::Result<()> {
        self.line(&Line::Step {
            step,
            outcome: outcome.to_owned(),
            moves: delta.moves.clone(),
            trashed: delta.trashed.clone(),
            created: delta.created.clone(),
            attributes: delta.attributes.clone(),
            relinked: delta.relinked.clone(),
        })
    }

    /// Appends a write begun: the target, and the unfinished name its
    /// bytes go to until the commit, so an interrupted job's journal says
    /// which unfinished file belongs to which target.
    pub(crate) fn writing(
        &mut self,
        target: &diroxide_core::Place,
        unfinished: &diroxide_core::Place,
    ) -> io::Result<()> {
        self.line(&Line::Writing {
            writing: target.clone(),
            unfinished: unfinished.clone(),
        })
    }

    /// Appends the end line, with what the job left behind, and makes the
    /// journal durable.
    pub(crate) fn end(&mut self, how: &str, left: &[LeftAs]) -> io::Result<()> {
        self.line(&Line::End {
            end: how.to_owned(),
            left: left.to_vec(),
        })?;
        self.file.sync_data()
    }
}

impl Journal {
    /// The journal of a state folder: its `journal` folder.
    pub fn new(state_folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: state_folder.into().join("journal"),
        }
    }

    /// The journal folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn path_of(&self, job: JobId) -> PathBuf {
        self.folder.join(format!("job-{:020}.jsonl", job.get()))
    }

    /// Starts a job's journal: the header, durable before the first step.
    pub(crate) fn start(
        &self,
        job: JobId,
        kind: JobKind,
        origin: &JobOrigin,
        steps: u64,
    ) -> io::Result<JournalWriter> {
        fs::create_dir_all(&self.folder)?;
        self.prune();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.path_of(job))?;
        let mut writer = JournalWriter { file };
        writer.line(&Header {
            journal: 1,
            job,
            kind,
            origin: origin.clone(),
            steps,
            started: Some(Timestamp::from_system_time(std::time::SystemTime::now())),
        })?;
        writer.file.sync_data()?;
        Ok(writer)
    }

    /// Every journal, oldest job first, read leniently: a line that cannot
    /// be read (the last, half written, after a crash) ends that journal.
    fn read_all(&self) -> Vec<(PathBuf, Interrupted, bool)> {
        let Ok(items) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = items
            .filter_map(|item| item.ok().map(|i| i.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("job-") && n.ends_with(".jsonl"))
            })
            .collect();
        paths.sort();
        paths
            .into_iter()
            .filter_map(|path| Self::read_one(&path).map(|(job, ended)| (path, job, ended)))
            .collect()
    }

    fn read_one(path: &Path) -> Option<(Interrupted, bool)> {
        let file = File::open(path).ok()?;
        let mut lines = BufReader::new(file).lines();
        let header: Header = serde_json::from_str(&lines.next()?.ok()?).ok()?;
        let mut job = Interrupted {
            job: header.job,
            kind: header.kind,
            steps: header.steps,
            finished: 0,
            done: 0,
            record: UndoRecord {
                origin: header.origin,
                ..UndoRecord::for_job(header.job, header.kind)
            },
            unfinished: Vec::new(),
        };
        let mut ended = false;
        let mut writing = None;
        for line in lines {
            let Ok(text) = line else { break };
            match serde_json::from_str::<Line>(&text) {
                Ok(Line::Step {
                    outcome,
                    moves,
                    trashed,
                    created,
                    attributes,
                    relinked,
                    ..
                }) => {
                    job.finished += 1;
                    writing = None;
                    if outcome == "done" {
                        job.done += 1;
                    }
                    job.record.moves.extend(moves);
                    job.record.trashed.extend(trashed);
                    job.record.created.extend(created);
                    job.record.attributes.extend(attributes);
                    job.record.relinked.extend(relinked);
                }
                Ok(Line::End { left, .. }) => {
                    job.record.left = left;
                    ended = true;
                    writing = None;
                }
                Ok(Line::Writing {
                    writing: target,
                    unfinished,
                }) => writing = Some((target, unfinished)),
                Err(_) => break,
            }
        }
        job.unfinished.extend(writing);
        Some((job, ended))
    }

    /// The jobs the last session did not finish, oldest first: the next
    /// start says each one's sentence before anything else.
    pub fn interrupted(&self) -> Vec<Interrupted> {
        self.read_all()
            .into_iter()
            .filter(|(_, _, ended)| !ended)
            .map(|(_, job, _)| job)
            .collect()
    }

    /// The undo records of the finished jobs, oldest first, to fill the
    /// undo log after a restart: every finished job, those that left
    /// nothing to undo included, so an undo after a restart can be matched
    /// with the record it reversed even when the undo itself left nothing
    /// to undo (an undo of a trash restores and records nothing). The app
    /// pairs them (each undo takes away the record before it) and drops the
    /// empty ones, as [`crate::UndoLog::record`] does.
    pub fn records(&self) -> Vec<UndoRecord> {
        self.read_all()
            .into_iter()
            .filter(|(_, _, ended)| *ended)
            .map(|(_, job, _)| job.record)
            .collect()
    }

    /// Marks an interrupted job as settled, once the user has heard about
    /// it: its journal gets the end line it never wrote.
    pub fn settle(&self, job: JobId) -> io::Result<()> {
        let file = OpenOptions::new().append(true).open(self.path_of(job))?;
        JournalWriter { file }.end("settled", &[])
    }

    /// Removes the oldest finished journals beyond [`KEEP_FINISHED`].
    /// Interrupted ones stay until settled.
    fn prune(&self) {
        let finished: Vec<PathBuf> = self
            .read_all()
            .into_iter()
            .filter(|(_, _, ended)| *ended)
            .map(|(path, _, _)| path)
            .collect();
        let extra = finished.len().saturating_sub(KEEP_FINISHED);
        for path in finished.into_iter().take(extra) {
            if path.starts_with(&self.folder) {
                let _ = fs::remove_file(&path);
            }
        }
    }
}
