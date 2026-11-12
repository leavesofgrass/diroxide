//! The plugin's calls to the host: a sentence (`ui/announce`), a hidden
//! or path question (`ui/ask`), a form (`ui/form`), a job's progress
//! (`$/progress`), and an output's stage and commit (`fs/stage`,
//! `fs/commit`). The plugin reads the owner's files itself and writes
//! each output to the stage diroxide gives; diroxide moves it into place.
//!
//! A secret the host sends back is taken into age's own wiping string at
//! once, and never written to the log, standard error or a sentence.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use age::secrecy::SecretString;
use diroxide_plugins::protocol::{Params, methods};
use serde_json::{Map, Value, json};

use crate::crypt::{Failure, Staged, Stager};
use crate::error::CryptError;
use crate::rpc::{Cancel, Peer};

/// How long a question to the owner may stay open: the owner's thinking
/// time is the owner's, and the host stops the wait if it goes away.
pub const OWNER_WAIT: Duration = Duration::from_secs(24 * 3600);

/// How long a stage or a commit may take: a rename in one folder, or a
/// numbered name found; far less than this.
pub const HOST_WAIT: Duration = Duration::from_secs(120);

/// A request's way to the host: the peer and the request's cancel flag.
#[derive(Clone, Copy)]
pub struct Host<'a> {
    /// The connection.
    pub peer: &'a Arc<Peer>,
    /// The cancel flag of the request being served.
    pub cancel: &'a Cancel,
}

impl std::fmt::Debug for Host<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

fn object(value: Value) -> Params {
    match value {
        Value::Object(map) => map,
        _ => Params::new(),
    }
}

/// Whether an answer is the owner's Escape.
fn cancelled(answer: &Value) -> bool {
    answer
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Takes a secret's text out of an answer into a wiping string, and
/// blanks it in the answer, so the parsed message holds no copy that
/// outlives this call.
pub fn take_secret(answer: &mut Value, key: &str) -> Option<SecretString> {
    let slot = answer.get_mut(key)?;
    let text = match slot.take() {
        Value::String(text) => text,
        _ => return None,
    };
    Some(SecretString::from(text))
}

impl Host<'_> {
    /// Calls the host and waits for the owner.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, CryptError> {
        self.peer
            .call(method, object(params), Some(self.cancel), OWNER_WAIT)
            .map_err(CryptError::from)
    }

    /// Says a sentence: `status`, `info`, `warning` or `error`.
    pub fn announce(&self, text: &str, importance: &str) {
        self.peer.notify(
            methods::UI_ANNOUNCE,
            object(json!({"text": text, "importance": importance})),
        );
    }

    /// Shows a form; its values by field id, or `None` for Escape. Secret
    /// fields are left in the map for [`take_secret`].
    pub fn form(&self, form: Value) -> Result<Option<Map<String, Value>>, CryptError> {
        let mut answer = self.call(methods::UI_FORM, form)?;
        if cancelled(&answer) {
            return Ok(None);
        }
        Ok(match answer.get_mut("values").map(Value::take) {
            Some(Value::Object(values)) => Some(values),
            _ => Some(Map::new()),
        })
    }

    /// Asks for a secret in a hidden field, with the lines read before it
    /// (a wrong try's words). `None` is Escape.
    pub fn ask_secret(
        &self,
        title: &str,
        lines: &[String],
    ) -> Result<Option<SecretString>, CryptError> {
        let mut answer = self.call(
            methods::UI_ASK,
            json!({"kind": "secret", "title": title, "message": lines}),
        )?;
        if cancelled(&answer) {
            return Ok(None);
        }
        Ok(take_secret(&mut answer, "text"))
    }

    /// Asks for the path of a file that exists. `None` is Escape.
    pub fn ask_file(&self, title: &str, lines: &[String]) -> Result<Option<String>, CryptError> {
        let answer = self.call(
            methods::UI_ASK,
            json!({"kind": "path", "mode": "file", "must_exist": true,
                "title": title, "message": lines}),
        )?;
        if cancelled(&answer) {
            return Ok(None);
        }
        Ok(answer
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }
}

/// A host's answer that failed, as a file's failure in words.
fn host_failure(error: &CryptError) -> Failure {
    let words = error.to_string();
    let mut chars = words.chars();
    let lower: String = match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "diroxide gave no reason".to_owned(),
    };
    Failure::Words(lower)
}

/// Outputs go through diroxide (`fs/stage` and `fs/commit`, the
/// manifest's `permissions.fs = "write"`): the made file joins the
/// running command's Undo, and the rename never replaces a file.
impl Stager for Host<'_> {
    fn stage(&self, output: &Path) -> Result<Staged, Failure> {
        let answer = self
            .peer
            .call(
                methods::FS_STAGE,
                object(json!({"place": output.display().to_string()})),
                Some(self.cancel),
                HOST_WAIT,
            )
            .map_err(|e| host_failure(&CryptError::from(e)))?;
        let id = answer.get("stage").and_then(Value::as_str);
        let path = answer.get("path").and_then(Value::as_str);
        match (id, path) {
            (Some(id), Some(path)) => Ok(Staged {
                id: id.to_owned(),
                path: PathBuf::from(path),
            }),
            _ => Err(Failure::Words("diroxide gave no stage for it".to_owned())),
        }
    }

    fn commit(&self, staged: &Staged) -> Result<PathBuf, Failure> {
        // The commit is not cancelled with the request: a written output
        // is placed, or its stage is removed by the caller.
        let answer = self
            .peer
            .call(
                methods::FS_COMMIT,
                object(json!({"stage": staged.id})),
                None,
                HOST_WAIT,
            )
            .map_err(|e| host_failure(&CryptError::from(e)))?;
        answer
            .get("place")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| Failure::Words("diroxide did not say where it is".to_owned()))
    }
}

/// A job's progress for the host: begin, reports, end, each sent as the
/// host's `$/progress`. Without a token from the request it says nothing.
#[derive(Clone)]
pub struct Progress {
    peer: Arc<Peer>,
    token: Option<Value>,
    begun: Arc<AtomicBool>,
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("token", &self.token)
            .finish()
    }
}

impl Progress {
    /// Progress under a request's token, if it gave one.
    pub fn new(peer: &Arc<Peer>, token: Option<Value>) -> Self {
        Self {
            peer: Arc::clone(peer),
            token,
            begun: Arc::new(AtomicBool::new(false)),
        }
    }

    fn send(&self, value: Value) {
        if let Some(token) = &self.token {
            self.peer.notify(
                methods::PROGRESS,
                object(json!({"token": token, "value": value})),
            );
        }
    }

    /// The job begins, with its plan line. Said once.
    pub fn begin(&self, title: &str) {
        if !self.begun.swap(true, Ordering::SeqCst) {
            self.send(json!({"kind": "begin", "title": title}));
        }
    }

    /// A sentence while it runs ("Encrypting 40 percent of backup.tar");
    /// the host says it at its own interval.
    pub fn report(&self, message: &str) {
        if !self.begun.load(Ordering::SeqCst) {
            self.begin(message);
        }
        self.send(json!({"kind": "report", "message": message}));
    }

    /// The job ends; nothing when it never began, and once.
    pub fn end(&self, message: Option<&str>) {
        if !self.begun.swap(false, Ordering::SeqCst) {
            return;
        }
        match message {
            Some(message) => self.send(json!({"kind": "end", "message": message})),
            None => self.send(json!({"kind": "end"})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    #[test]
    fn a_secret_is_taken_out_of_its_answer() {
        let mut answer = json!({"text": "thrown-away-7f3a"});
        let secret = take_secret(&mut answer, "text").unwrap();
        assert_eq!(secret.expose_secret(), "thrown-away-7f3a");
        assert!(!answer.to_string().contains("thrown-away"));
        assert!(!format!("{secret:?}").contains("thrown-away"));
        assert!(take_secret(&mut answer, "text").is_none());
        assert!(take_secret(&mut json!({"text": 3}), "text").is_none());
    }
}
