//! Outputs a plugin staged and committed (`fs/stage` and `fs/commit`;
//! alpha 4, lane AE, E's request 1). The host moves each staged file into
//! place, never over a file, and tells the app the job it made; the file
//! joins the undo record of the plugin's running command, so one Undo
//! puts every file the command made in the trash ("Undid plugin command:
//! 2 items are in the trash"). A commit with no command running (a serve
//! client) is a record of its own.

use std::collections::HashMap;

use diroxide_core::{Effect, JobId, JobKind};
use diroxide_ops::JobReport;
use diroxide_ops::undo::UndoRecord;

use super::App;

/// The files each running plugin command committed, until it ends.
#[derive(Clone, Debug, Default)]
pub(super) struct StagedState {
    by_job: HashMap<JobId, UndoRecord>,
}

impl App {
    /// A staged output the host moved into place for `plugin`: kept with
    /// its running command's undo record, or recorded at once.
    pub(super) fn plugin_committed(&mut self, plugin: &str, report: JobReport) -> Vec<Effect> {
        let undo = report.undo;
        match self.running_of(plugin) {
            Some(job) => {
                let record = self.staged.by_job.entry(job).or_insert_with(|| {
                    let mut record = UndoRecord::for_job(job, JobKind::PluginCommand);
                    record.origin = undo.origin.clone();
                    record
                });
                record.created.extend(undo.created);
                record.left.extend(undo.left);
            }
            None => {
                let job = self.next_job_id();
                let mut record = undo;
                record.job = Some(job);
                record.kind = Some(JobKind::PluginCommand);
                self.undo.record(record);
            }
        }
        Vec::new()
    }

    /// A plugin command ended: what it committed is one undo record. True
    /// when there was one.
    pub(super) fn commits_ended(&mut self, job: JobId) -> bool {
        match self.staged.by_job.remove(&job) {
            Some(record) if !record.is_empty() => {
                self.undo.record(record);
                true
            }
            _ => false,
        }
    }
}
