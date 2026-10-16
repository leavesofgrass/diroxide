//! The undo log: what the last jobs did that can be undone, which the
//! Undo command replays backwards. Records are serializable, so W1b's
//! journal in the state folder can write one before the first step and
//! update it per step, and Undo works after a restart.

use diroxide_core::{ConflictPolicy, JobId, JobKind, JobOrigin, Place};
use serde::{Deserialize, Serialize};

use crate::plan::{Plan, Step};

/// What one job did that can be undone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UndoRecord {
    /// The job.
    pub job: Option<JobId>,
    /// What kind of job it was, for "Undo rename of 47 files".
    pub kind: Option<JobKind>,
    /// Who started it.
    pub origin: JobOrigin,
    /// Entries moved or renamed, as (from, to), in the order done.
    pub moves: Vec<(Place, Place)>,
    /// Entries moved to the trash, by their former places, in the order
    /// done: deleted entries, and the old copies a replace sent to the
    /// trash first.
    pub trashed: Vec<Place>,
    /// Entries the job made, in the order made: copies, new folders,
    /// links, split parts, a joined file (R20). Undo trashes the files
    /// that are unchanged and removes the folders that are still empty,
    /// deepest first.
    pub created: Vec<Place>,
}

impl UndoRecord {
    /// An empty record for a job.
    pub fn for_job(job: JobId, kind: JobKind) -> Self {
        Self {
            job: Some(job),
            kind: Some(kind),
            ..Self::default()
        }
    }

    /// Whether there is nothing to undo.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty() && self.trashed.is_empty() && self.created.is_empty()
    }

    /// The plan that undoes this record: what it made goes first, newest
    /// first (files to the trash, folders removed if still empty, so the
    /// deepest go first); then moves backwards in reverse order; then
    /// restores from the trash. `folders` says which created places were
    /// folders. The undo-time checks (a changed copy is kept, a non-empty
    /// folder stays) are the engine's.
    pub fn undo_plan(&self, job: JobId, folders: &[Place]) -> Plan {
        let mut steps: Vec<Step> = self
            .created
            .iter()
            .rev()
            .map(|place| {
                if folders.contains(place) {
                    Step::RemoveEmptyFolder {
                        place: place.clone(),
                    }
                } else {
                    Step::Trash {
                        place: place.clone(),
                    }
                }
            })
            .collect();
        steps.extend(self.moves.iter().rev().map(|(from, to)| Step::Move {
            from: to.clone(),
            to: from.clone(),
        }));
        steps.extend(self.trashed.iter().map(|place| Step::Restore {
            place: place.clone(),
        }));
        Plan::new(job, JobKind::Undo, steps, ConflictPolicy::Skip)
    }
}

/// The records of the last jobs, newest last, up to a limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoLog {
    records: Vec<UndoRecord>,
    limit: usize,
}

impl UndoLog {
    /// How many jobs are kept by default.
    pub const DEFAULT_LIMIT: usize = 20;

    /// An empty log that keeps at most `limit` jobs, at least one.
    pub fn new(limit: usize) -> Self {
        Self {
            records: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Adds a job's record, dropping the oldest past the limit. Empty
    /// records are not kept.
    pub fn record(&mut self, record: UndoRecord) {
        if record.is_empty() {
            return;
        }
        self.records.push(record);
        if self.records.len() > self.limit {
            self.records.remove(0);
        }
    }

    /// The newest record.
    pub fn last(&self) -> Option<&UndoRecord> {
        self.records.last()
    }

    /// Takes the newest record off the log, to undo it.
    pub fn pop(&mut self) -> Option<UndoRecord> {
        self.records.pop()
    }

    /// How many jobs can be undone.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether nothing can be undone.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new(Self::DEFAULT_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;

    fn p(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    #[test]
    fn undo_replays_backwards() {
        let record = UndoRecord {
            moves: vec![(p("/a/1"), p("/b/1")), (p("/a/2"), p("/b/2"))],
            trashed: vec![p("/a/3")],
            ..UndoRecord::for_job(JobId::new(4), JobKind::Move)
        };
        let plan = record.undo_plan(JobId::new(5), &[]);
        assert_eq!(plan.kind, JobKind::Undo);
        assert_eq!(
            plan.steps,
            vec![
                Step::Move {
                    from: p("/b/2"),
                    to: p("/a/2")
                },
                Step::Move {
                    from: p("/b/1"),
                    to: p("/a/1")
                },
                Step::Restore { place: p("/a/3") },
            ]
        );
    }

    #[test]
    fn undo_of_a_copy_and_a_make_folder_removes_what_was_made() {
        let record = UndoRecord {
            created: vec![p("/b/new"), p("/b/new/inner"), p("/b/copy.txt")],
            ..UndoRecord::for_job(JobId::new(6), JobKind::Copy)
        };
        assert!(!record.is_empty());
        let plan = record.undo_plan(JobId::new(7), &[p("/b/new"), p("/b/new/inner")]);
        assert_eq!(
            plan.steps,
            vec![
                Step::Trash {
                    place: p("/b/copy.txt")
                },
                Step::RemoveEmptyFolder {
                    place: p("/b/new/inner")
                },
                Step::RemoveEmptyFolder { place: p("/b/new") },
            ]
        );
    }

    #[test]
    fn a_record_travels_as_json() {
        let native = Place::parse(if cfg!(windows) { r"D:\b\x" } else { "/b/x" }).unwrap();
        let record = UndoRecord {
            created: vec![native],
            ..UndoRecord::for_job(JobId::new(8), JobKind::Split)
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: UndoRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, record);
        let old: UndoRecord = serde_json::from_str(r#"{"job":2,"moves":[]}"#).unwrap();
        assert_eq!(old.origin, JobOrigin::User);
    }

    #[test]
    fn the_log_keeps_the_newest() {
        let mut log = UndoLog::new(2);
        log.record(UndoRecord::default());
        assert!(log.is_empty());
        for n in 1..=3 {
            log.record(UndoRecord {
                trashed: vec![p("/x")],
                ..UndoRecord::for_job(JobId::new(n), JobKind::Trash)
            });
        }
        assert_eq!(log.len(), 2);
        assert_eq!(log.pop().unwrap().job, Some(JobId::new(3)));
        assert_eq!(log.last().unwrap().job, Some(JobId::new(2)));
    }
}
