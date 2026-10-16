//! Shared pieces: places on the fake, an engine over it, and a sink that
//! answers from a script.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use diroxide_core::{ConflictAnswer, ErrorAction, JobState, Place};
use diroxide_ops::{Conflict, Engine, FakeProvider, ItemError, JobProgress, JobSink};
use diroxide_vfs::Registry;

/// A local place written Unix-style, on `D:` on Windows.
pub fn p(text: &str) -> Place {
    Place::parse(&if cfg!(windows) {
        format!("D:{}", text.replace('/', "\\"))
    } else {
        text.to_owned()
    })
    .unwrap()
}

/// An engine whose local places are served by the fake.
pub fn engine(fake: &FakeProvider) -> Engine {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    Engine::new(Arc::new(registry))
}

type Hook = Box<dyn FnMut(&JobProgress) + Send>;

/// A sink that answers from scripts, Skip when they run out, and records
/// the states it was told.
#[derive(Default)]
pub struct Answers {
    pub conflicts: Mutex<VecDeque<ConflictAnswer>>,
    pub errors: Mutex<VecDeque<ErrorAction>>,
    pub states: Mutex<Vec<JobState>>,
    pub asked: Mutex<usize>,
    hook: Mutex<Option<Hook>>,
}

impl Answers {
    pub fn new(conflicts: Vec<ConflictAnswer>, errors: Vec<ErrorAction>) -> Self {
        let answers = Self::default();
        *answers.conflicts.lock().unwrap() = conflicts.into();
        *answers.errors.lock().unwrap() = errors.into();
        answers
    }

    pub fn on_progress(self, hook: impl FnMut(&JobProgress) + Send + 'static) -> Self {
        *self.hook.lock().unwrap() = Some(Box::new(hook));
        self
    }
}

impl JobSink for Answers {
    fn progress(&self, progress: &JobProgress) {
        self.states.lock().unwrap().push(progress.state);
        if let Some(hook) = self.hook.lock().unwrap().as_mut() {
            hook(progress);
        }
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        *self.asked.lock().unwrap() += 1;
        self.conflicts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(ConflictAnswer::skip)
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        *self.asked.lock().unwrap() += 1;
        self.errors
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ErrorAction::Skip)
    }
}
