//! The three commands (R29 3.2): what each acts on, read from the
//! command's context (the marked files, the cursor's when none is
//! marked), and the answer every one gives back.
//!
//! Every command checks its files before it asks anything, so no secret
//! is typed for nothing; says its plan line, each result and each
//! refusal itself (`ui/announce`), the meaning first; and answers what it
//! made, so the app can record each new file in its undo list and offer
//! the trash for an original:
//!
//! ```json
//! {"done": 2, "skipped": 1, "failed": ["notes.txt"],
//!  "made": [{"from": "D:\\work\\report.pdf", "to": "D:\\work\\report.pdf.age"}]}
//! ```
//!
//! Escape on a form or a question is an answer: nothing more is done for
//! that file, the rest go on, and what was skipped is said.

mod decrypt;
mod encrypt;
pub mod forms;

use std::path::PathBuf;

use diroxide_core::Place;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError};
use serde_json::{Map, Value, json};

use crate::crypt::{End, Failure};
use crate::error::CryptError;
use crate::host::{Host, Progress};
use crate::lines;

/// The commands this build answers (plugin-local ids), in the manifest's
/// order.
pub const IDS: &[&str] = &["encrypt_passphrase", "encrypt_recipient", "decrypt"];

/// A file to work on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// Its path on this computer.
    pub path: PathBuf,
    /// Its name, as said.
    pub name: String,
    /// Its place, as the host gave it.
    pub place: String,
}

/// What a command did, file by file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Files done.
    pub done: usize,
    /// Files passed over (a folder, one not here, a question escaped).
    pub skipped: usize,
    /// The names of files that failed.
    pub failed: Vec<String>,
    /// Each file made, from its source.
    pub made: Vec<(String, String)>,
}

impl Tally {
    /// The command's answer.
    pub fn answer(&self) -> Value {
        let made: Vec<Value> = self
            .made
            .iter()
            .map(|(from, to)| json!({"from": from, "to": to}))
            .collect();
        json!({
            "done": self.done,
            "skipped": self.skipped,
            "failed": self.failed,
            "made": made,
        })
    }
}

/// A command's request: the host, the progress and the context.
pub struct Ctx<'a> {
    /// The way to the host.
    pub host: Host<'a>,
    /// The job's progress.
    pub progress: Progress,
    params: &'a Params,
}

impl Ctx<'_> {
    fn context(&self) -> Option<&Map<String, Value>> {
        self.params.get("context").and_then(Value::as_object)
    }

    /// Whether the host cancelled the request.
    pub fn cancelled(&self) -> bool {
        self.host.cancel.is_cancelled()
    }

    /// The files to work on: the marked ones, the cursor's when none is
    /// marked. A folder, a file not on this computer and one that is not
    /// there are said and counted as skipped.
    pub fn targets(&self, tally: &mut Tally) -> Vec<Target> {
        let Some(context) = self.context() else {
            return Vec::new();
        };
        let mut entries: Vec<&Value> = context
            .get("selection")
            .and_then(Value::as_array)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        if entries.is_empty()
            && let Some(cursor) = context.get("cursor").filter(|c| !c.is_null())
        {
            entries.push(cursor);
        }
        let mut out = Vec::new();
        for entry in entries {
            let place = entry
                .get("place")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| place.clone());
            if entry.get("kind").and_then(Value::as_str) == Some("folder") {
                self.host.announce(&lines::skipped_folder(&name), "warning");
                self.host.announce(lines::PACK_FIRST, "info");
                tally.skipped += 1;
                continue;
            }
            let Some(path) = Place::parse(&place)
                .ok()
                .and_then(|p| p.local_path().map(PathBuf::from))
            else {
                self.host.announce(&lines::not_here(&name), "warning");
                tally.skipped += 1;
                continue;
            };
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => {
                    self.host.announce(&lines::skipped_folder(&name), "warning");
                    self.host.announce(lines::PACK_FIRST, "info");
                    tally.skipped += 1;
                }
                Ok(_) => out.push(Target { path, name, place }),
                Err(_) => {
                    self.host.announce(&lines::not_there(&name), "warning");
                    tally.skipped += 1;
                }
            }
        }
        out
    }

    /// Nothing to work on: said, and answered.
    pub fn nothing(&self, tally: &Tally) -> Value {
        if tally.skipped == 0 {
            self.host.announce(lines::NOTHING_MARKED, "warning");
        }
        tally.answer()
    }

    /// The owner's Escape on the first form: nothing was done.
    pub fn escaped(&self, tally: &Tally) -> Value {
        self.host.announce(lines::CANCELLED, "info");
        tally.answer()
    }

    /// A progress reporter for one file, saying each tenth of a file of a
    /// megabyte or more: "Encrypting 40 percent of backup.tar".
    pub fn reporter<'b>(&'b self, verb: &'b str, name: &'b str) -> impl FnMut(u64, u64) + 'b {
        let mut last = 0u64;
        move |done, size| {
            if size < 1024 * 1024 {
                return;
            }
            let percent = done.saturating_mul(100) / size.max(1);
            let step = percent / 10 * 10;
            if step > last && step < 100 {
                last = step;
                self.progress.report(&lines::progress(verb, step, name));
            }
        }
    }

    /// Notes how one file ended and says it: the result line, or "Could
    /// not VERB NAME" and its cause.
    pub fn record(&self, tally: &mut Tally, target: &Target, end: End, verb: Verb) {
        match end {
            End::Made(path) => {
                let to_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let line = match verb {
                    Verb::Encrypt => lines::encrypted(&target.name, &to_name),
                    Verb::Decrypt => lines::decrypted(&target.name, &to_name),
                };
                self.host.announce(&line, "info");
                tally.done += 1;
                tally
                    .made
                    .push((target.place.clone(), path.display().to_string()));
            }
            End::Failed(failure) => self.failed(tally, target, &failure, verb),
            End::Cancelled => {}
        }
    }

    /// Says a file's failure, the verb and name first, the cause after.
    pub fn failed(&self, tally: &mut Tally, target: &Target, failure: &Failure, verb: Verb) {
        let head = match verb {
            Verb::Encrypt => lines::could_not_encrypt(&target.name),
            Verb::Decrypt => lines::could_not_decrypt(&target.name),
        };
        self.host.announce(&head, "warning");
        self.host.announce(&capital(&failure.cause()), "info");
        tally.failed.push(target.name.clone());
    }

    /// The run's last words: the summary for several files, and what
    /// always follows.
    pub fn finish(&self, tally: &Tally, of: usize, verb: Verb) -> Value {
        let word = match verb {
            Verb::Encrypt => "Encrypted",
            Verb::Decrypt => "Decrypted",
        };
        if of > 1 || tally.skipped > 0 {
            let line = lines::summary(word, tally.done, of, tally.failed.len(), tally.skipped);
            self.host.announce(&line, "status");
        }
        if tally.done > 0 {
            let after = match verb {
                Verb::Encrypt => lines::ORIGINAL_STAYS,
                Verb::Decrypt => lines::NOT_PROOF,
            };
            self.host.announce(after, "info");
        }
        if self.cancelled() {
            self.host.announce(lines::CANCELLED, "info");
        }
        tally.answer()
    }
}

/// Which way a command goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Encrypt.
    Encrypt,
    /// Decrypt.
    Decrypt,
}

/// A cause made a sentence: its first letter capitalized.
pub fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `command/run`: runs one command.
pub fn run(host: Host<'_>, params: &Params) -> Result<Value, CryptError> {
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let command = command.rsplit('.').next().unwrap_or(command).to_owned();
    let token = params.get("progress_token").cloned();
    let ctx = Ctx {
        host,
        progress: Progress::new(host.peer, token),
        params,
    };
    let result = match command.as_str() {
        "encrypt_passphrase" => encrypt::with_passphrase(&ctx),
        "encrypt_recipient" => encrypt::for_recipients(&ctx),
        "decrypt" => decrypt::decrypt(&ctx),
        other => Err(CryptError::Host(RpcError::new(
            ErrorKind::MethodNotFound,
            format!("no command {other}"),
        ))),
    };
    ctx.progress.end(None);
    result
}
