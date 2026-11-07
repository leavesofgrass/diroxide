//! The gpg jobs and signature checks on workers (alpha 4, lane AE; R30
//! commits 72 and 73's app half): a plan through ops' gpg driver with the
//! tools registry the engine carries, its passphrases asked through the
//! app's masked prompt; the run as a job Cancel stops, its report and its
//! undo record sent to the app; one signature checked through gpg or
//! minisign. A passphrase reaches gpg on standard input only, never a
//! command line, and is dropped with the job.

use std::sync::Arc;

use diroxide_core::{JobId, Secret, SecretKind, SecretRequest};
use diroxide_ops::JobControl;
use diroxide_ops::crypt::{GpgRequest, verify_signature};
use diroxide_vfs::{CancelToken, SecretSource};

use super::{ForwardSecrets, Runtime, Shared, forward_sink, lock};
use crate::app::AppEvent;
use crate::crypt::{CryptAsk, CryptNote};

/// What the runtime sends for a note of the gpg commands.
fn note(note: CryptNote) -> AppEvent {
    AppEvent::Crypt(Box::new(note))
}

/// The verb of a gpg job's words.
fn verb_of(options: &diroxide_ops::crypt::GpgOptions) -> &'static str {
    use diroxide_ops::crypt::GpgAction;
    match options.action {
        GpgAction::Decrypt => "decrypt",
        GpgAction::Sign { .. } => "sign",
        _ => "encrypt",
    }
}

/// Asks the app's masked prompt; a new passphrase twice, the second
/// "Same passphrase again for report.pdf", until the two agree or the
/// tries run out (R29 3.3: diroxide asks twice, so gpg asks nothing).
pub(super) struct TwiceSecrets {
    asker: ForwardSecrets,
}

impl TwiceSecrets {
    /// A source asking through the app for `job`.
    pub(super) fn new(shared: &Arc<Shared>, job: JobId) -> Self {
        Self {
            asker: ForwardSecrets {
                job: Some(job),
                shared: Arc::clone(shared),
            },
        }
    }
}

impl SecretSource for TwiceSecrets {
    fn ask(&self, request: &SecretRequest, cancel: &CancelToken) -> Option<Secret> {
        if !request.new_secret {
            return self.asker.ask(request, cancel);
        }
        for attempt in 1..=request.attempts_allowed.max(1) {
            let first_request = if attempt == 1 {
                request.clone()
            } else {
                // The two differed: said in the prompt's own name.
                SecretRequest::new(
                    SecretKind::ServerPrompt,
                    format!("Not the same. New passphrase for {}", request.subject),
                )
            };
            let first = self.asker.ask(&first_request, cancel)?;
            if first.is_empty() {
                return None;
            }
            let again = SecretRequest::new(
                SecretKind::ServerPrompt,
                format!("Same passphrase again for {}", request.subject),
            );
            let second = self.asker.ask(&again, cancel)?;
            if first == second {
                return Some(first);
            }
        }
        None
    }
}

impl Runtime {
    /// Carries out one of the gpg commands' asks, or a signature check.
    pub(super) fn crypt(&mut self, ask: CryptAsk) {
        match ask {
            CryptAsk::Plan {
                job,
                sources,
                options,
            } => {
                let engine = self.engine.clone();
                self.worker(move |shared| {
                    let secrets: Arc<dyn SecretSource> = Arc::new(TwiceSecrets::new(shared, job));
                    let verb = verb_of(&options);
                    let request = GpgRequest::new(job, sources, options).with_secrets(secrets);
                    let result = engine
                        .plan_gpg(&request, &JobControl::new())
                        .map(Box::new)
                        .map_err(|error| crate::crypt::plan_refused(verb, &error));
                    shared.send(note(CryptNote::Planned { job, result }));
                });
            }
            CryptAsk::Run(plan) => {
                let engine = self.engine.clone();
                let job = plan.0.job;
                let control = self.register(job);
                let jobs = Arc::clone(&self.jobs);
                self.worker(move |shared| {
                    let sink = forward_sink(job, shared, &engine, &control);
                    let report = engine.run_gpg(&plan.0, &control, &sink);
                    shared.send(note(CryptNote::Finished(Box::new(report))));
                    lock(&jobs).remove(&job);
                });
            }
            CryptAsk::Verify { file, check } => {
                let tools = Arc::clone(self.engine.tools());
                self.worker(move |shared| {
                    let verdict = verify_signature(tools.as_ref(), &check, &CancelToken::new());
                    shared.send(note(CryptNote::Verified {
                        file,
                        check: Box::new(check),
                        verdict: Box::new(verdict),
                    }));
                });
            }
        }
    }
}
