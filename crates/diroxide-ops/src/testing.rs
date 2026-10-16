//! Helpers for the crate's own tests: places in the platform's style, an
//! engine over a fake provider, and a sink that answers from a script and
//! records what it was told.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use diroxide_core::{ConflictAnswer, ErrorAction, JobState, Place};
use diroxide_vfs::Registry;

use crate::engine::{Conflict, Engine, ItemError, JobSink};
use crate::fake::FakeProvider;
use crate::progress::JobProgress;

/// A local place written Unix-style, on `D:` on Windows.
pub(crate) fn p(text: &str) -> Place {
    Place::parse(&if cfg!(windows) {
        format!("D:{}", text.replace('/', "\\"))
    } else {
        text.to_owned()
    })
    .unwrap()
}

/// An engine whose local places are served by the fake.
pub(crate) fn engine(fake: &FakeProvider) -> Engine {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    Engine::new(Arc::new(registry))
}

/// A sink that answers conflicts and errors from scripts (Skip when a
/// script runs out) and records everything it was told.
#[derive(Default)]
pub(crate) struct Script {
    pub conflicts: Mutex<VecDeque<ConflictAnswer>>,
    pub errors: Mutex<VecDeque<ErrorAction>>,
    pub said: Mutex<Vec<String>>,
    pub states: Mutex<Vec<JobState>>,
    pub progress: Mutex<Vec<JobProgress>>,
    pub asked: Mutex<Vec<Conflict>>,
    pub failed: Mutex<Vec<ItemError>>,
}

#[allow(dead_code)] // The executor's tests use the rest (the next commit).
impl Script {
    /// A sink that answers these conflicts in order.
    pub(crate) fn conflicts(answers: Vec<ConflictAnswer>) -> Self {
        let script = Self::default();
        *script.conflicts.lock().unwrap() = answers.into();
        script
    }

    /// A sink that answers these errors in order.
    pub(crate) fn errors(answers: Vec<ErrorAction>) -> Self {
        let script = Self::default();
        *script.errors.lock().unwrap() = answers.into();
        script
    }

    /// The states said, with repeats folded.
    pub(crate) fn state_changes(&self) -> Vec<JobState> {
        let mut changes: Vec<JobState> = Vec::new();
        for state in self.states.lock().unwrap().iter() {
            if changes.last() != Some(state) {
                changes.push(*state);
            }
        }
        changes
    }
}

impl JobSink for Script {
    fn progress(&self, progress: &JobProgress) {
        self.said.lock().unwrap().push(progress.describe());
        self.states.lock().unwrap().push(progress.state);
        self.progress.lock().unwrap().push(progress.clone());
    }

    fn conflict(&self, conflict: &Conflict) -> ConflictAnswer {
        self.asked.lock().unwrap().push(conflict.clone());
        self.conflicts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(ConflictAnswer::skip)
    }

    fn error(&self, error: &ItemError) -> ErrorAction {
        self.failed.lock().unwrap().push(error.clone());
        self.errors
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ErrorAction::Skip)
    }
}
