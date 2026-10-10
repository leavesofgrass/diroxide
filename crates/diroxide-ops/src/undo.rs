//! The undo log: the last jobs' renames, moves and trashings, which the
//! Undo command replays backwards.

use diroxide_core::{ConflictPolicy, JobId, JobKind, Place};

use crate::plan::{Plan, Step};

/// What one job did that can be undone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoRecord {
    /// The job.
    pub job: Option<JobId>,
    /// Entries moved or renamed, as (from, to), in the order done.
    pub moves: Vec<(Place, Place)>,
    /// Entries moved to the trash, by their former places.
    pub trashed: Vec<Place>,
}

impl UndoRecord {
    /// Whether there is nothing to undo.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty() && self.trashed.is_empty()
    }

    /// The plan that undoes this record: moves backwards in reverse order,
    /// then restores from the trash.
    pub fn undo_plan(&self, job: JobId) -> Plan {
        let mut steps: Vec<Step> = self
            .moves
            .iter()
            .rev()
            .map(|(from, to)| Step::Move {
                from: to.clone(),
                to: from.clone(),
            })
            .collect();
        steps.extend(self.trashed.iter().map(|place| Step::Restore {
            place: place.clone(),
        }));
        Plan {
            job,
            kind: JobKind::Undo,
            steps,
            conflicts: ConflictPolicy::Skip,
        }
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
            job: Some(JobId::new(4)),
            moves: vec![(p("/a/1"), p("/b/1")), (p("/a/2"), p("/b/2"))],
            trashed: vec![p("/a/3")],
        };
        let plan = record.undo_plan(JobId::new(5));
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
    fn the_log_keeps_the_newest() {
        let mut log = UndoLog::new(2);
        log.record(UndoRecord::default());
        assert!(log.is_empty());
        for n in 1..=3 {
            log.record(UndoRecord {
                job: Some(JobId::new(n)),
                trashed: vec![p("/x")],
                ..UndoRecord::default()
            });
        }
        assert_eq!(log.len(), 2);
        assert_eq!(log.pop().unwrap().job, Some(JobId::new(3)));
        assert_eq!(log.last().unwrap().job, Some(JobId::new(2)));
    }
}
