//! What a conversion did: each output made with its source, each failure
//! and skip with its reason, and the job's report, whose undo record
//! names every file the job made.

use diroxide_core::{JobId, JobKind, Place};

use crate::engine::JobReport;

/// What a conversion did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertReport {
    /// The job's report: the outputs made in `done`, the failures and
    /// skips by their sources, and the undo record whose `created` names
    /// every output, so Undo trashes the files the job made.
    pub job: JobReport,
    /// Each output made, with its source: (source, output).
    pub made: Vec<(Place, Place)>,
    /// The files the plan refused, each with its reason.
    pub refused: Vec<(Place, String)>,
    /// The recipe's name.
    pub recipe: String,
    /// The tool's name as said.
    pub tool: String,
}

impl ConvertReport {
    /// An empty report for a conversion job.
    pub fn new(job: JobId, recipe: &str, tool: &str) -> Self {
        Self {
            job: JobReport::new(job, JobKind::Convert),
            made: Vec::new(),
            refused: Vec::new(),
            recipe: recipe.to_owned(),
            tool: tool.to_owned(),
        }
    }
}
