//! The runtime half of duplicates (alpha 5, lane DA): the search and the
//! check before the trash, each a job on a worker, answered as
//! [`DupsNote`]. Each job's cancel token is kept with the metadata jobs'
//! (`meta_runs`), so Cancel in the Jobs list stops it between reads.
//!
//! The search never asks a question on the worker: its sink answers No,
//! so a search that would hash more than the setup allows stops with
//! `DupError::Declined`, and the app asks and searches again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use diroxide_core::{JobId, Place};
use diroxide_meta::MetaError;
use diroxide_meta::checksum::{Algorithm, compute_reader};
use diroxide_ops::JobControl;
use diroxide_ops::SystemPacer;
use diroxide_ops::dups::{
    ContentHasher, DupEnv, DupProgress, DupSink, find_duplicates, recheck_selection,
};
use diroxide_vfs::{CancelToken, OpContext, Registry, VfsError};

use super::{Runtime, Shared, lock};
use crate::app::AppEvent;
use crate::dups_ui::{DupSetup, DupsAsk, DupsNote, leave_out};

/// BLAKE3 over `diroxide-meta`'s checksum reader: the full hash of the
/// duplicate search, the same the Checksum command computes and the same
/// the command line uses.
#[derive(Clone, Copy, Debug, Default)]
struct Blake3Hasher;

impl ContentHasher for Blake3Hasher {
    fn name(&self) -> &'static str {
        "BLAKE3"
    }

    fn hash(
        &self,
        reader: &mut dyn std::io::Read,
        cancel: &CancelToken,
        on_step: &mut dyn FnMut(u64),
    ) -> Result<Vec<u8>, VfsError> {
        let digests =
            compute_reader(reader, &[Algorithm::Blake3], cancel, on_step).map_err(|e| match e {
                MetaError::Place(error) => error,
                other => VfsError::Corrupt {
                    what: other.to_string(),
                },
            })?;
        Ok(digests
            .into_iter()
            .next()
            .map(|digest| digest.bytes)
            .unwrap_or_default())
    }
}

/// Sends a job's progress to the app.
struct Sink {
    shared: Arc<Shared>,
    job: JobId,
}

impl DupSink for Sink {
    fn progress(&self, progress: &DupProgress) {
        self.shared
            .send(AppEvent::Duplicates(Box::new(DupsNote::Progress {
                job: self.job,
                progress: progress.clone(),
            })));
    }
}

impl Runtime {
    /// Carries out one ask of duplicates on a worker.
    pub(super) fn dups_request(&mut self, ask: DupsAsk) {
        let job = match &ask {
            DupsAsk::Find { job, .. } | DupsAsk::Recheck { job, .. } => *job,
        };
        let control = JobControl::new();
        lock(&self.meta_runs).insert(job, control.cancel.clone());
        let runs = Arc::clone(&self.meta_runs);
        let registry = Arc::clone(&self.registry);
        self.worker(move |shared| {
            let note = run(shared, &registry, ask, &control);
            lock(&runs).remove(&job);
            shared.send(AppEvent::Duplicates(Box::new(note)));
        });
    }
}

/// One ask, on the worker it runs on.
fn run(shared: &Arc<Shared>, registry: &Registry, ask: DupsAsk, control: &JobControl) -> DupsNote {
    let pacer = SystemPacer::new();
    let hasher = Blake3Hasher;
    match ask {
        DupsAsk::Find { job, setup } => {
            let sink = Sink {
                shared: Arc::clone(shared),
                job,
            };
            let env = DupEnv {
                registry,
                sink: &sink,
                hasher: &hasher,
                pacer: &pacer,
            };
            let found = find_duplicates(&setup.roots, &setup.options(), control, &env);
            let mut hidden_left_out = 0;
            let result = found.map(|mut result| {
                if !setup.hidden {
                    hidden_left_out = leave_out(&mut result, hidden_test(registry, &setup));
                }
                Box::new(result)
            });
            DupsNote::Found {
                job,
                setup,
                result,
                hidden_left_out,
            }
        }
        DupsAsk::Recheck { job, view } => {
            let sink = Sink {
                shared: Arc::clone(shared),
                job,
            };
            let env = DupEnv {
                registry,
                sink: &sink,
                hasher: &hasher,
                pacer: &pacer,
            };
            let selection = view.selection();
            let result = recheck_selection(&view.result, &selection, control, &env);
            DupsNote::Rechecked { job, view, result }
        }
    }
}

/// Whether a copy is hidden, or sits in a hidden folder below the root
/// it was found under, as its place's provider says; each folder is
/// asked once.
fn hidden_test<'a>(registry: &'a Registry, setup: &'a DupSetup) -> impl Fn(&Place) -> bool + 'a {
    let known: Mutex<HashMap<String, bool>> = Mutex::new(HashMap::new());
    let ctx = OpContext::new();
    move |place: &Place| {
        let Some(root) = setup
            .roots
            .iter()
            .filter(|root| place == *root || place.is_within(root))
            .max_by_key(|root| root.to_string().len())
        else {
            return false;
        };
        let mut at = Some(place.clone());
        while let Some(here) = at {
            if &here == root {
                return false;
            }
            let key = here.to_string();
            let cached = lock(&known).get(&key).copied();
            let hidden = cached.unwrap_or_else(|| {
                let hidden = registry
                    .resolve(&here)
                    .and_then(|provider| provider.stat(&here, &ctx))
                    .map(|entry| entry.hidden)
                    .unwrap_or(false);
                lock(&known).insert(key, hidden);
                hidden
            });
            if hidden {
                return true;
            }
            at = here.parent();
        }
        false
    }
}
