//! The jobs line: what is running, in words.
//!
//! Wave 1 (W1d) owns this module.

use diroxide_core::JobId;

/// The running jobs and their latest progress sentences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobTable {
    running: Vec<(JobId, String)>,
}

impl JobTable {
    /// No jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes a job that started.
    pub fn start(&mut self, job: JobId, text: impl Into<String>) {
        self.running.push((job, text.into()));
    }

    /// Updates a job's progress sentence.
    pub fn update(&mut self, job: JobId, text: impl Into<String>) {
        if let Some(entry) = self.running.iter_mut().find(|(id, _)| *id == job) {
            entry.1 = text.into();
        }
    }

    /// Removes a finished job.
    pub fn finish(&mut self, job: JobId) {
        self.running.retain(|(id, _)| *id != job);
    }

    /// How many jobs are running.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    /// Whether nothing is running.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// The jobs line: "No jobs", the one job's sentence, or "2 jobs:
    /// Copying 12 of 300; Moving to the trash 1 of 3".
    pub fn line(&self) -> String {
        match self.running.as_slice() {
            [] => "No jobs".to_owned(),
            [(_, only)] => only.clone(),
            many => {
                let texts: Vec<&str> = many.iter().map(|(_, text)| text.as_str()).collect();
                format!("{} jobs: {}", many.len(), texts.join("; "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jobs_line_says_what_runs() {
        let mut jobs = JobTable::new();
        assert_eq!(jobs.line(), "No jobs");
        jobs.start(JobId::new(1), "Copying 0 of 3");
        jobs.update(JobId::new(1), "Copying 1 of 3");
        assert_eq!(jobs.line(), "Copying 1 of 3");
        jobs.start(JobId::new(2), "Moving 0 of 1");
        assert_eq!(jobs.line(), "2 jobs: Copying 1 of 3; Moving 0 of 1");
        jobs.finish(JobId::new(1));
        assert_eq!(jobs.len(), 1);
    }
}
