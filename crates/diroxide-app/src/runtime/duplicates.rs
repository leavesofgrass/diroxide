//! The runtime half of duplicates (alpha 5, lane DA): the search and the
//! check before the trash, each a job on a worker, answered as
//! [`DupsNote`]. Each job's cancel token is kept with the metadata jobs'
//! (`meta_runs`), so Cancel in the Jobs list stops it between reads.
//!
//! The search never asks a question on the worker: its sink answers No,
//! so a search that would hash more than the setup allows stops with
//! `DupError::Declined`, and the app asks and searches again.

use std::sync::Arc;

use diroxide_core::JobId;
use diroxide_meta::MetaError;
use diroxide_meta::checksum::{Algorithm, compute_reader};
use diroxide_ops::JobControl;
use diroxide_ops::SystemPacer;
use diroxide_ops::dups::{
    ContentHasher, DupEnv, DupProgress, DupSink, find_duplicates, recheck_selection,
};
use diroxide_vfs::{CancelToken, Registry, VfsError};

use super::{Runtime, Shared, lock};
use crate::app::AppEvent;
use crate::dups_ui::{DupsAsk, DupsNote};

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
            // Hidden files follow the setup inside the engine (question
            // 136): they are never hashed nor counted in the estimate.
            let found = find_duplicates(&setup.roots, &setup.options(), control, &env);
            let hidden_left_out = found.as_ref().map_or(0, |r| r.hidden_left_out);
            let result = found.map(Box::new);
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
