//! The runtime half of compare folders and synchronize (alpha 5, lane
//! SA): the compare and the run as jobs on workers, answered as
//! [`crate::sync_ui::SyncNote`].
//!
//! A compare is a `JobKind::Compare` job whose control sits with the
//! running jobs, so Cancel trips it between folders and blocks; its
//! progress goes to the app at the app's own interval. A run is one
//! `JobKind::Synchronize` job through [`diroxide_ops::sync::run_sync`]:
//! every place it writes over or removes is read again first, and its
//! report reaches the app as any job's does.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use diroxide_core::{JobKind, Message};
use diroxide_ops::sync::{CompareProgress, SyncEnv, compare_dirs, run_sync};
use diroxide_ops::{JobControl, JobReport, OpsError};
use diroxide_vfs::VfsError;

use super::{Runtime, forward_sink, lock};
use crate::app::AppEvent;
use crate::sync_ui::{SyncAsk, SyncNote};

impl Runtime {
    /// Carries out one ask of lane SA.
    pub(super) fn sync_request(&mut self, ask: SyncAsk) {
        match ask {
            SyncAsk::Compare {
                job,
                here,
                there,
                options,
                every_ms,
                then_plan,
            } => {
                let control = self.register(job);
                let registry = Arc::clone(&self.registry);
                let jobs = Arc::clone(&self.jobs);
                self.worker(move |shared| {
                    let last: Mutex<Option<Instant>> = Mutex::new(None);
                    let every = Duration::from_millis(every_ms);
                    let speaker = Arc::clone(shared);
                    let sink = move |progress: &CompareProgress| {
                        if progress.done {
                            return;
                        }
                        let mut at = last.lock().unwrap_or_else(PoisonError::into_inner);
                        let now = Instant::now();
                        // The first update waits the interval too, so a
                        // quick compare says only its result.
                        let due = match *at {
                            None => {
                                *at = Some(now);
                                false
                            }
                            Some(then) => now.duration_since(then) >= every,
                        };
                        if due || progress.paused {
                            *at = Some(now);
                            speaker.send(AppEvent::Syncing(Box::new(SyncNote::Progress {
                                job,
                                line: progress.describe(),
                            })));
                        }
                    };
                    let criteria = options.criteria();
                    let result = compare_dirs(&registry, &here, &there, &criteria, &control, &sink)
                        .map(Box::new)
                        .map_err(|error| refusal(&error, "compare"));
                    lock(&jobs).remove(&job);
                    shared.send(AppEvent::Syncing(Box::new(SyncNote::Compared {
                        job,
                        here,
                        there,
                        options,
                        then_plan,
                        result,
                    })));
                });
            }
            SyncAsk::Files {
                request,
                before,
                after,
                options,
            } => {
                let registry = Arc::clone(&self.registry);
                self.worker(move |shared| {
                    let result =
                        super::compare_two(shared, &registry, request, &before, &after, &options);
                    shared.send(AppEvent::Syncing(Box::new(SyncNote::Files {
                        request,
                        before,
                        after,
                        options,
                        result,
                    })));
                });
            }
            SyncAsk::Hooks { job, plan } => {
                let engine = self.engine.clone();
                self.worker(move |shared| {
                    let control = JobControl::new();
                    let result = plan
                        .before_hooks(&engine, job, &control)
                        .map_err(|error| refusal(&error, "synchronize"));
                    shared.send(AppEvent::Syncing(Box::new(SyncNote::Hooked {
                        job,
                        plan,
                        result,
                    })));
                });
            }
            SyncAsk::Run { job, plan } => {
                let control = self.register(job);
                let engine = self.engine.clone();
                let jobs = Arc::clone(&self.jobs);
                self.worker(move |shared| {
                    let sink = forward_sink(job, shared, &engine, &control);
                    let env = SyncEnv {
                        engine: &engine,
                        job,
                        control: &control,
                        sink: &sink,
                        hooks_asked: true,
                    };
                    let report = match run_sync(&plan, &env) {
                        Ok(report) => report,
                        Err(error) => {
                            let mut report = JobReport::new(job, JobKind::Synchronize);
                            if matches!(error, OpsError::Vfs(VfsError::Cancelled)) {
                                report.cancelled = true;
                            } else {
                                report
                                    .failed
                                    .push((plan.left.clone(), refusal(&error, "synchronize")));
                            }
                            report
                        }
                    };
                    lock(&jobs).remove(&job);
                    shared.send(AppEvent::JobFinished(Box::new(report)));
                });
            }
        }
    }
}

/// An engine error as the line to say: a refusal's own sentence, else
/// "Could not VERB: CAUSE."
fn refusal(error: &OpsError, verb: &str) -> String {
    match error {
        OpsError::Refused(line) | OpsError::HookStopped(line) => line.clone(),
        other => match other.sentence() {
            Some(sentence) => sentence,
            None => Message::could_not(verb, "", &other.to_string(), "").text,
        },
    }
}
