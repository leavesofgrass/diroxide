//! The plugin's calls to the host's services, besides `tools/run`: a
//! sentence (`ui/announce`), a question (`ui/ask`), a form (`ui/form`),
//! two texts in the compare screens (`ui/show_diff`), a document
//! (`ui/show_text`), a list of rows (`lister/open`), a job's progress
//! (`$/progress`) and the few file jobs it asks of the host (`fs/write`
//! for `.gitignore`, `fs/delete` for a cancelled clone's folder).
//!
//! The host adds the plugin's name to every title and line, so nothing
//! here does (the host-pieces lane's finding: a title that names the
//! plugin hears it twice).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diroxide_plugins::base64;
use diroxide_plugins::protocol::{Params, methods};
use serde_json::{Map, Value, json};

use crate::error::GitError;
use crate::rpc::{Cancel, Peer};

/// How long a question to the owner may stay open: the owner's thinking
/// time is the owner's, and the host stops the wait if it goes away.
pub const OWNER_WAIT: Duration = Duration::from_secs(24 * 3600);

/// How long a host service other than a question may take.
pub const SERVICE_WAIT: Duration = Duration::from_secs(120);

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

impl Host<'_> {
    /// Calls the host and waits.
    pub fn call(&self, method: &str, params: Value, wait: Duration) -> Result<Value, GitError> {
        self.peer
            .call(method, object(params), Some(self.cancel), wait)
            .map_err(GitError::from)
    }

    /// Says a sentence: `status`, `info`, `warning` or `error`.
    pub fn announce(&self, text: &str, importance: &str) {
        self.peer.notify(
            methods::UI_ANNOUNCE,
            object(json!({"text": text, "importance": importance})),
        );
    }

    /// Asks yes or no. A destructive question starts on No. `None` is
    /// Escape.
    pub fn confirm(
        &self,
        title: &str,
        lines: &[String],
        destructive: bool,
    ) -> Result<Option<bool>, GitError> {
        let answer = self.call(
            methods::UI_ASK,
            json!({"kind": "confirm", "title": title, "message": lines, "destructive": destructive}),
            OWNER_WAIT,
        )?;
        if cancelled(&answer) {
            return Ok(None);
        }
        Ok(answer.get("answer").and_then(Value::as_bool))
    }

    /// Asks for a line of text, with a pattern the answer must match and
    /// the words said when it does not. `None` is Escape.
    pub fn ask_text(
        &self,
        title: &str,
        lines: &[String],
        initial: &str,
        pattern: Option<(&str, &str)>,
    ) -> Result<Option<String>, GitError> {
        let mut params =
            json!({"kind": "text", "title": title, "message": lines, "initial": initial});
        if let Some((pattern, words)) = pattern {
            params["pattern"] = json!(pattern);
            params["pattern_message"] = json!(words);
        }
        let answer = self.call(methods::UI_ASK, params, OWNER_WAIT)?;
        if cancelled(&answer) {
            return Ok(None);
        }
        Ok(answer
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// Shows a form; its values by field id, or `None` for Escape.
    pub fn form(&self, form: Value) -> Result<Option<Map<String, Value>>, GitError> {
        let answer = self.call(methods::UI_FORM, form, OWNER_WAIT)?;
        if cancelled(&answer) {
            return Ok(None);
        }
        Ok(answer.get("values").and_then(Value::as_object).cloned())
    }

    /// Shows two texts in the compare screens, the verdict first.
    pub fn show_diff(
        &self,
        title: &str,
        old: (&str, &str),
        new: (&str, &str),
    ) -> Result<(), GitError> {
        self.call(
            methods::UI_SHOW_DIFF,
            json!({
                "title": title,
                "old": {"title": old.0, "text": old.1},
                "new": {"title": new.0, "text": new.1},
            }),
            OWNER_WAIT,
        )
        .map(drop)
    }

    /// Shows a text document in the viewer: `plain` or `diff`.
    pub fn show_text(&self, title: &str, text: &str, kind: &str) -> Result<(), GitError> {
        self.call(
            methods::UI_SHOW_TEXT,
            json!({"title": title, "text": text, "kind": kind}),
            OWNER_WAIT,
        )
        .map(drop)
    }

    /// Shows rows in the active panel as one of the manifest's listers.
    pub fn open_lister(&self, lister: &str, title: &str, rows: Vec<Value>) -> Result<(), GitError> {
        let total = rows.len();
        self.call(
            methods::LISTER_OPEN,
            json!({"lister": lister, "title": title, "rows": rows, "total": total, "panel": "active"}),
            SERVICE_WAIT,
        )
        .map(drop)
    }

    /// Writes a whole file through the host: a new file at once, an
    /// existing one after the host's own question, which starts on No.
    /// False when the owner said no.
    pub fn write_file(&self, place: &str, bytes: &[u8]) -> Result<bool, GitError> {
        let answer = self.call(
            methods::FS_WRITE,
            json!({"place": place, "data": base64::encode(bytes)}),
            OWNER_WAIT,
        )?;
        Ok(answer
            .get("written")
            .and_then(Value::as_bool)
            .unwrap_or(true))
    }

    /// Hands places to the host's delete job, to the trash, with its
    /// listing and its question; not cancelled with the request, since it
    /// tidies after one.
    pub fn trash(&self, places: &[String]) -> Result<Value, GitError> {
        self.peer
            .call(
                methods::FS_DELETE,
                object(json!({"items": places})),
                None,
                OWNER_WAIT,
            )
            .map_err(GitError::from)
    }
}

/// Whether an answer is the owner's Escape.
fn cancelled(answer: &Value) -> bool {
    answer
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// A job's progress for the host: begin, reports, end, each said at the
/// host's interval. Without a token from the request it says nothing.
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

    /// The job begins: "Cloning". Said once; reports before it begin it.
    pub fn begin(&self, title: &str) {
        if !self.begun.swap(true, Ordering::SeqCst) {
            self.send(json!({"kind": "begin", "title": title}));
        }
    }

    /// A sentence while it runs.
    pub fn report(&self, message: &str) {
        if !self.begun.load(Ordering::SeqCst) {
            self.begin(message);
        }
        self.send(json!({"kind": "report", "message": message}));
    }

    /// The job ends, with its own words or none; nothing when it never
    /// began, and once.
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
