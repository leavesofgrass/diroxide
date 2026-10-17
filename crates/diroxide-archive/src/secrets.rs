//! The passwords of one job (spike K1, ADR-0009): asked through the job's
//! own [`SecretSource`] when the first locked member is reached, the same
//! password tried for the rest of the job, three tries at most, and
//! forgotten when the job ends.
//!
//! [`JobSecrets`] stands between the provider and the app's masked prompt.
//! It holds at most one password, in a [`Secret`] that is wiped when it is
//! dropped. When a password proves wrong the caller says so
//! ([`JobSecrets::reject`]); the next ask is the next try, its request
//! numbered so the app can say "Wrong password for course.zip. Try again?
//! y or n" first ([`crate::words::wrong_password`]). After the third wrong
//! try, or when the user gives no password (Escape at the prompt), nothing
//! more is asked in that job, and every locked member is skipped with
//! "Gave up on course.zip after 3 wrong passwords. Skipped."
//!
//! The format crates need the text itself. zip borrows it for one read;
//! sevenz-rust2 copies it into its own `Password`, a plain byte vector
//! that is not wiped, kept only for the one call (a listing, a read, a
//! block) or, when packing, until the archive is finished.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use diroxide_core::{Message, Secret, SecretKind, SecretRequest, catalog};
use diroxide_vfs::{CancelToken, OpContext, SecretSource, VfsError};

use crate::error::ArchiveError;

#[derive(Default)]
struct State {
    held: Option<Secret>,
    wrong: u32,
    asked: u32,
    declined: bool,
    subject: Option<String>,
}

/// The passwords of one job. See the module documentation.
pub struct JobSecrets {
    inner: Arc<dyn SecretSource>,
    state: Mutex<State>,
}

impl fmt::Debug for JobSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state();
        f.debug_struct("JobSecrets")
            .field("holds_a_password", &state.held.is_some())
            .field("wrong", &state.wrong)
            .field("asked", &state.asked)
            .finish_non_exhaustive()
    }
}

impl JobSecrets {
    /// The secrets of a job that asks `inner` (the app's masked prompt).
    pub fn new(inner: Arc<dyn SecretSource>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            state: Mutex::new(State::default()),
        })
    }

    /// The secrets of a job that asks the context's source.
    pub fn of(ctx: &OpContext) -> Arc<Self> {
        Self::new(ctx.secrets.clone())
    }

    /// The same, with a password already known (typed in the dialog): it
    /// is the first try, and the prompt is asked only when it proves
    /// wrong.
    pub fn knowing(inner: Arc<dyn SecretSource>, password: Option<Secret>) -> Arc<Self> {
        let secrets = Self::new(inner);
        if let Some(password) = password.filter(|p| !p.is_empty()) {
            let mut state = secrets.state();
            state.held = Some(password);
            state.asked = 1;
        }
        secrets
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The context with these secrets in place of its own source.
    pub fn context(self: &Arc<Self>, ctx: &OpContext) -> OpContext {
        let source: Arc<dyn SecretSource> = self.clone();
        ctx.clone().with_secrets(source)
    }

    /// Says the password held proved wrong: it is forgotten, and the next
    /// ask is the next try. Returns whether another try is allowed.
    pub fn reject(&self) -> bool {
        let mut state = self.state();
        if state.held.take().is_some() {
            state.wrong += 1;
        }
        !state.declined && state.wrong < SecretRequest::ATTEMPTS
    }

    /// The wrong tries so far.
    pub fn wrong_tries(&self) -> u32 {
        self.state().wrong
    }

    /// How many times the prompt was asked (a password from the dialog
    /// counts as the first).
    pub fn asked(&self) -> u32 {
        self.state().asked
    }

    /// Whether the job stopped asking: three wrong tries, or no password
    /// given.
    pub fn gave_up(&self) -> bool {
        let state = self.state();
        state.declined || state.wrong >= SecretRequest::ATTEMPTS
    }

    /// Whether a password is held now.
    pub fn holds_a_password(&self) -> bool {
        self.state().held.is_some()
    }

    /// Forgets the password held, wiping it. The job's end does the same
    /// when the last handle is dropped.
    pub fn forget(&self) {
        self.state().held = None;
    }

    /// The line to say when the job stopped asking: "Gave up on course.zip
    /// after 3 wrong passwords. Skipped.", or, when no password was given,
    /// "No password given. Skipped 10 members of course.zip."
    pub fn gave_up_line(&self, subject: &str, skipped: u64) -> Message {
        let state = self.state();
        if state.wrong >= SecretRequest::ATTEMPTS {
            catalog::gave_up_on_secret(subject, SecretRequest::ATTEMPTS)
        } else {
            catalog::no_secret_given(skipped, subject)
        }
    }

    /// The archive the job last asked about.
    pub fn subject(&self) -> Option<String> {
        self.state().subject.clone()
    }

    /// Runs `op` with these secrets in its context, again after each wrong
    /// password while tries are left: the provider asks for the password
    /// through the context, and a wrong one means the next try. A wrong
    /// password after the last try is returned as it is.
    pub fn with_tries<T>(
        self: &Arc<Self>,
        ctx: &OpContext,
        mut op: impl FnMut(&OpContext) -> Result<T, ArchiveError>,
    ) -> Result<T, ArchiveError> {
        let ctx = self.context(ctx);
        loop {
            match op(&ctx) {
                Err(ArchiveError::Vfs(VfsError::WrongPassword)) if self.reject() => {}
                other => return other,
            }
        }
    }
}

impl SecretSource for JobSecrets {
    fn ask(&self, request: &SecretRequest, cancel: &CancelToken) -> Option<Secret> {
        let try_number = {
            let mut state = self.state();
            if let Some(held) = &state.held {
                return Some(held.clone());
            }
            if state.declined || state.wrong >= SecretRequest::ATTEMPTS {
                return None;
            }
            state.subject = Some(request.subject.clone());
            state.wrong + 1
        };
        if cancel.is_cancelled() {
            return None;
        }
        let asking = SecretRequest {
            attempt: try_number,
            attempts_allowed: SecretRequest::ATTEMPTS,
            ..request.clone()
        };
        // The lock is not held while the prompt waits for the user.
        let answer = self.inner.ask(&asking, cancel);
        let mut state = self.state();
        state.asked += 1;
        match answer {
            Some(secret) if !secret.is_empty() && !cancel.is_cancelled() => {
                state.held = Some(secret.clone());
                Some(secret)
            }
            _ => {
                state.declined = true;
                None
            }
        }
    }
}

/// The request for a new archive password, typed twice in the masked
/// prompt: "New password for reports.7z (hidden): ". The pack dialog asks
/// it and puts the answer in [`crate::PackRequest::password`].
pub fn new_password_request(archive_name: &str) -> SecretRequest {
    SecretRequest {
        new_secret: true,
        ..SecretRequest::new(SecretKind::ArchivePassword, archive_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "canary-7f3a-not-a-password";

    struct Answers(Mutex<Vec<Option<&'static str>>>, Mutex<Vec<SecretRequest>>);

    impl SecretSource for Answers {
        fn ask(&self, request: &SecretRequest, _cancel: &CancelToken) -> Option<Secret> {
            self.1.lock().unwrap().push(request.clone());
            let next = self.0.lock().unwrap().pop().flatten();
            next.map(|text| Secret::new(text.to_owned()))
        }
    }

    fn answers(list: &[Option<&'static str>]) -> Arc<Answers> {
        let mut list = list.to_vec();
        list.reverse();
        Arc::new(Answers(Mutex::new(list), Mutex::new(Vec::new())))
    }

    #[test]
    fn a_password_is_asked_once_and_tried_for_the_rest_of_the_job() {
        let source = answers(&[Some(CANARY)]);
        let secrets = JobSecrets::new(source.clone());
        let request = SecretRequest::new(SecretKind::ArchivePassword, "course.zip");
        let token = CancelToken::new();
        for _ in 0..3 {
            assert_eq!(secrets.ask(&request, &token).unwrap().expose(), CANARY);
        }
        assert_eq!(source.1.lock().unwrap().len(), 1);
        assert!(!format!("{secrets:?}").contains(CANARY));
        secrets.forget();
        assert!(!secrets.holds_a_password());
    }

    #[test]
    fn three_wrong_tries_then_nothing_is_asked() {
        let source = answers(&[Some("a"), Some("b"), Some("c"), Some("d")]);
        let secrets = JobSecrets::new(source.clone());
        let request = SecretRequest::new(SecretKind::ArchivePassword, "course.zip");
        let token = CancelToken::new();
        for attempt in 1..=3 {
            assert!(secrets.ask(&request, &token).is_some());
            assert_eq!(secrets.reject(), attempt < 3);
        }
        assert!(secrets.ask(&request, &token).is_none());
        let asked = source.1.lock().unwrap();
        assert_eq!(
            asked.iter().map(|r| r.attempt).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(asked.iter().all(|r| r.attempts_allowed == 3 && !r.echo));
        assert!(secrets.gave_up());
        assert_eq!(
            secrets.gave_up_line("course.zip", 4).text,
            "Gave up on course.zip after 3 wrong passwords. Skipped."
        );
    }

    #[test]
    fn no_password_given_stops_asking() {
        let source = answers(&[None, Some("late")]);
        let secrets = JobSecrets::new(source.clone());
        let request = SecretRequest::new(SecretKind::ArchivePassword, "course.zip");
        let token = CancelToken::new();
        assert!(secrets.ask(&request, &token).is_none());
        assert!(secrets.ask(&request, &token).is_none());
        assert_eq!(source.1.lock().unwrap().len(), 1);
        assert_eq!(
            secrets.gave_up_line("course.zip", 10).text,
            "No password given. Skipped 10 members of course.zip."
        );
    }

    #[test]
    fn a_known_password_is_the_first_try() {
        let source = answers(&[Some("right")]);
        let secrets = JobSecrets::knowing(source.clone(), Some(Secret::new("wrong".to_owned())));
        let request = SecretRequest::new(SecretKind::ArchivePassword, "a.7z");
        let token = CancelToken::new();
        assert_eq!(secrets.ask(&request, &token).unwrap().expose(), "wrong");
        assert!(source.1.lock().unwrap().is_empty());
        assert!(secrets.reject());
        assert_eq!(secrets.ask(&request, &token).unwrap().expose(), "right");
        assert_eq!(source.1.lock().unwrap()[0].attempt, 2);
    }

    #[test]
    fn a_new_password_is_typed_twice() {
        let request = new_password_request("reports.7z");
        assert!(request.new_secret);
        assert_eq!(
            request.prompt_line(),
            "New password for reports.7z (hidden): "
        );
    }
}
