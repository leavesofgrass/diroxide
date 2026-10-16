//! Cancellation, progress and secrets, carried by every provider
//! operation.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use diroxide_core::{Secret, SecretRequest};

use crate::error::VfsError;

/// A shared flag that asks an operation to stop. Cloning shares the flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks every holder of this token to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether stopping was asked for.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Err(VfsError::Cancelled)` once stopping was asked for; long loops
    /// call this between steps.
    pub fn check(&self) -> Result<(), VfsError> {
        if self.is_cancelled() {
            Err(VfsError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// How far an operation has got. The job engine turns these into words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgressUpdate {
    /// Items finished.
    pub items_done: u64,
    /// Items in all, when known.
    pub items_total: Option<u64>,
    /// Bytes finished.
    pub bytes_done: u64,
    /// Bytes in all, when known.
    pub bytes_total: Option<u64>,
    /// The name being worked on.
    pub current: Option<String>,
}

/// Receives progress. Called often; implementations keep it cheap.
pub trait ProgressSink: Send + Sync {
    /// Takes one update.
    fn report(&self, update: ProgressUpdate);
}

/// A sink that drops every update.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn report(&self, _update: ProgressUpdate) {}
}

/// Asks the user for a secret: an archive's password, a key's passphrase.
/// The call blocks the worker thread while the app shows the masked
/// prompt, the way a conflict question does (ADR-0009), and the job line
/// says "Waiting for the password for course.zip." at once.
pub trait SecretSource: Send + Sync {
    /// Asks for a secret; `None` when the user gave none, or the token was
    /// cancelled while asking.
    fn ask(&self, request: &SecretRequest, cancel: &CancelToken) -> Option<Secret>;
}

/// A source that never has a secret: the default, for tests and for
/// operations that must not ask.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSecrets;

impl SecretSource for NoSecrets {
    fn ask(&self, _request: &SecretRequest, _cancel: &CancelToken) -> Option<Secret> {
        None
    }
}

/// What every provider operation receives: how to know it should stop,
/// where to report progress, and whom to ask for a secret.
#[derive(Clone)]
pub struct OpContext {
    /// Checked between steps.
    pub cancel: CancelToken,
    /// Told how far the operation has got.
    pub progress: Arc<dyn ProgressSink>,
    /// Asked for a password or passphrase when one is needed.
    pub secrets: Arc<dyn SecretSource>,
}

impl OpContext {
    /// A context with a fresh token, no progress reporting and no secrets.
    pub fn new() -> Self {
        Self {
            cancel: CancelToken::new(),
            progress: Arc::new(NoProgress),
            secrets: Arc::new(NoSecrets),
        }
    }

    /// The same context with this token.
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// The same context reporting to this sink.
    pub fn with_progress(mut self, progress: Arc<dyn ProgressSink>) -> Self {
        self.progress = progress;
        self
    }

    /// The same context asking this source for secrets.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretSource>) -> Self {
        self.secrets = secrets;
        self
    }

    /// Asks the context's source for a secret.
    pub fn ask_secret(&self, request: &SecretRequest) -> Option<Secret> {
        self.secrets.ask(request, &self.cancel)
    }

    /// Shorthand for `self.cancel.check()`.
    pub fn check(&self) -> Result<(), VfsError> {
        self.cancel.check()
    }
}

impl Default for OpContext {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OpContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpContext")
            .field("cancel", &self.cancel)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_context_has_no_secrets() {
        use diroxide_core::SecretKind;
        let request = SecretRequest::new(SecretKind::ArchivePassword, "a.zip");
        assert!(OpContext::new().ask_secret(&request).is_none());
        struct Always;
        impl SecretSource for Always {
            fn ask(&self, _: &SecretRequest, _: &CancelToken) -> Option<Secret> {
                Some(Secret::new("canary-7f3a-not-a-password".to_owned()))
            }
        }
        let ctx = OpContext::new().with_secrets(Arc::new(Always));
        let secret = ctx.ask_secret(&request).unwrap();
        assert!(!format!("{ctx:?} {secret:?}").contains("canary"));
    }

    #[test]
    fn cancelling_is_shared_by_clones() {
        let token = CancelToken::new();
        let ctx = OpContext::new().with_cancel(token.clone());
        assert!(ctx.check().is_ok());
        token.cancel();
        assert!(matches!(ctx.check(), Err(VfsError::Cancelled)));
    }
}
