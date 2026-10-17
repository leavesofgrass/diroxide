//! What the runner tests share: a destination folder on the fake disk, an
//! engine over the world's registry, a sink that keeps every progress
//! line and answers conflicts as a test says, and scripted answers to a
//! limit's questions.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use diroxide_archive::guard::Question;
use diroxide_archive::{ArchiveQuestions, JobEnv};
use diroxide_core::{ConflictAction, ConflictAnswer, ErrorAction, JobId, Place};
use diroxide_ops::{Conflict, Engine, ItemError, JobControl, JobProgress, JobSink};
use diroxide_vfs::SecretSource;

use crate::world::World;

/// The destination folder on the fake disk: `out` beside the archives.
pub fn out(world: &World) -> Place {
    let out = world.root.parent().unwrap().join("out").unwrap();
    world.fake.add_folder(&out);
    out
}

/// A sink that keeps the progress lines and answers every conflict with
/// one answer.
pub struct Sink {
    /// The jobs-line words, in order.
    pub lines: Mutex<Vec<String>>,
    /// The conflicts asked.
    pub conflicts: Mutex<Vec<Conflict>>,
    answer: ConflictAnswer,
    /// Cancels this control at the first progress report past this many
    /// items, when set.
    pub cancel_after: Option<(u64, JobControl)>,
    /// Cancels this control when a conflict is asked, when set: a cancel
    /// that lands in the middle of a job, whatever its speed.
    pub cancel_on_conflict: Option<JobControl>,
}

impl Sink {
    /// A sink that skips every conflict.
    pub fn skipping() -> Self {
        Self::answering(ConflictAnswer::skip())
    }

    /// A sink that answers every conflict so.
    pub fn answering(answer: ConflictAnswer) -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
            conflicts: Mutex::new(Vec::new()),
            answer,
            cancel_after: None,
            cancel_on_conflict: None,
        }
    }

    /// A sink that replaces everything.
    pub fn replacing() -> Self {
        Self::answering(ConflictAnswer::for_all(ConflictAction::Replace))
    }
}

impl JobSink for Sink {
    fn progress(&self, progress: &JobProgress) {
        self.lines.lock().unwrap().push(progress.describe());
        if let Some((items, control)) = &self.cancel_after
            && progress.items_done >= *items
        {
            control.cancel.cancel();
        }
    }

    fn conflict(&self, conflict: &Conflict) -> ConflictAnswer {
        self.conflicts.lock().unwrap().push(conflict.clone());
        if let Some(control) = &self.cancel_on_conflict {
            control.cancel.cancel();
        }
        self.answer.clone()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

/// Answers a limit's questions from a list, keeping what was asked.
#[derive(Default)]
pub struct Answers {
    /// The questions asked.
    pub asked: Mutex<Vec<Question>>,
    yes: bool,
}

impl Answers {
    /// Says yes to every question.
    pub fn yes() -> Self {
        Self {
            asked: Mutex::new(Vec::new()),
            yes: true,
        }
    }

    /// Says no to every question.
    pub fn no() -> Self {
        Self::default()
    }
}

impl ArchiveQuestions for Answers {
    fn ask(&self, question: &Question) -> bool {
        self.asked.lock().unwrap().push(question.clone());
        self.yes
    }
}

/// The engine over the world's registry.
pub fn engine(world: &World) -> Engine {
    Engine::new(world.registry.clone())
}

/// A job's surroundings over the world: its engine, the archive provider,
/// this control and sink, these answers and this prompt.
pub fn env<'a>(
    world: &'a World,
    engine: &'a Engine,
    control: &'a JobControl,
    sink: &'a Sink,
    answers: &'a Answers,
    secrets: Option<Arc<dyn SecretSource>>,
) -> JobEnv<'a> {
    let env =
        JobEnv::new(engine, &world.provider, JobId::new(41), control, sink).with_questions(answers);
    match secrets {
        Some(secrets) => env.with_secrets(secrets),
        None => env,
    }
}

/// The files and folders under `root` on the fake disk, as paths relative
/// to it with forward slashes, with their contents (empty for a folder).
pub fn tree(world: &World, root: &Place) -> Vec<(String, Vec<u8>)> {
    let root_text = root.to_string();
    let mut found = Vec::new();
    for (key, (kind, data)) in world.fake.snapshot() {
        let place = Place::parse(&key).unwrap();
        if place == *root || !place.is_within(root) {
            continue;
        }
        let relative = key[root_text.len()..]
            .trim_start_matches(['/', '\\'])
            .replace('\\', "/");
        let data = if kind == diroxide_core::EntryKind::Folder {
            Vec::new()
        } else {
            data
        };
        found.push((relative, data));
    }
    found.sort();
    found
}

/// Every place on the fake disk that is neither under `root` nor one of
/// the places that were there before.
pub fn outside(world: &World, root: &Place, before: &[String]) -> Vec<String> {
    world
        .fake
        .snapshot()
        .into_keys()
        .filter(|key| {
            let place = Place::parse(key).unwrap();
            !(place.is_within(root) || root.is_within(&place) || before.contains(key))
        })
        .collect()
}
