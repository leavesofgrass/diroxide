//! The owner's side of a prompt: the app's own dialog through `ui/ask`
//! (R24 sections 1.6, 3.5 and 5.3; answers 84 and 90).
//!
//! - **A new host key** asks a destructive confirmation, which starts on
//!   No: the host, the key's type, the fingerprint in hexadecimal groups of
//!   four, one part a row, then the text as ssh printed it, then the
//!   question. A yes goes back to ssh as "yes" and ssh writes the key to
//!   known hosts itself; anything else is no.
//! - **ssh's own password prompt** asks a secret titled with the account
//!   and the host, "Password for alice at build.example", and on a retry
//!   says which try it is, "Try 2 of 3."
//! - **A key's passphrase** asks a secret titled with the key's name.
//! - **A server's question** asks a secret titled "Question from
//!   build.example:", the server's own words cleaned and last.
//! - **A confirmation** asks yes or no, starting on No; **a notice** is
//!   said and wants no answer.
//! - **A changed host key** never comes as a prompt: ssh refuses it and
//!   says so on standard error. [`host_key_refusal`] reads that and makes
//!   the refusal, with the `ssh-keygen -R` line to forget the old key
//!   shown and never run (answer 84). There is no road to accept it.
//!
//! The host adds the plugin's name last to every title (answer 90). A
//! secret goes from the host's answer to the helper and nowhere else: it
//! is never logged and never kept here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use diroxide_core::Secret;
use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use serde_json::{Value, json};

use super::classify::{PromptKind, classify};
use super::listener::{Handler, Prompt, Reply};
use crate::rpc::Peer;

/// How long the owner may take over one question: the host pauses the
/// plugin's timer while it asks (R22 N7); this is the plugin's own limit.
pub const ANSWER_LIMIT: Duration = Duration::from_secs(300);

/// ssh's own count of password prompts, when the configuration says
/// nothing (`NumberOfPasswordPrompts`).
pub const PASSWORD_TRIES: u32 = 3;

fn params(value: Value) -> Params {
    match value {
        Value::Object(map) => map,
        _ => Params::new(),
    }
}

/// The `ui/ask` question for a prompt; `None` for a notice, which is said
/// instead. `host` is the connection's host as the owner knows it; `try`
/// counts the password prompts of this connection, from 1.
pub fn question(kind: &PromptKind, host: &str, attempt: u32, tries: u32) -> Option<Params> {
    let value = match kind {
        PromptKind::NewHostKey {
            host: named,
            key_type,
            fingerprint,
        } => {
            let mut message = vec![
                format!("New server key for {named}."),
                "Check it before you accept it.".to_owned(),
                format!("Key type: {key_type}."),
                "Fingerprint, in groups of four:".to_owned(),
            ];
            message.extend(fingerprint.hex_rows());
            message.push("As ssh printed it:".to_owned());
            message.push(fingerprint.original.clone());
            message.push("Trust this key and connect?".to_owned());
            json!({
                "kind": "confirm",
                "destructive": true,
                "yes_label": "Trust and connect",
                "no_label": "Do not connect",
                "title": "New server key",
                "message": message,
            })
        }
        PromptKind::HostPassword { user, host: named } => {
            let message: Vec<String> = if attempt > 1 {
                vec![format!("Try {attempt} of {tries}.")]
            } else {
                Vec::new()
            };
            json!({
                "kind": "secret",
                "title": format!("Password for {user} at {named}"),
                "message": message,
            })
        }
        PromptKind::KeyPassphrase { key } => json!({
            "kind": "secret",
            "title": format!("Key passphrase for {key}"),
            "message": [],
        }),
        PromptKind::Confirm { text } => json!({
            "kind": "confirm",
            "destructive": true,
            "title": "ssh asks you to confirm",
            "message": [text],
        }),
        PromptKind::Server { text } => json!({
            "kind": "secret",
            "title": format!("Question from {host}:"),
            "message": [text],
        }),
        PromptKind::Notice { .. } => return None,
    };
    Some(params(value))
}

/// What ssh gets for the host's answer to a question.
pub fn reply(kind: &PromptKind, answer: Result<Value, RpcError>) -> Reply {
    let Ok(answer) = answer else {
        return Reply::Refuse;
    };
    if answer.get("cancelled").and_then(Value::as_bool) == Some(true) {
        return Reply::Refuse;
    }
    match kind {
        PromptKind::NewHostKey { .. } => {
            if answer.get("answer").and_then(Value::as_bool) == Some(true) {
                Reply::Answer(Secret::new("yes".to_owned()))
            } else {
                Reply::Refuse
            }
        }
        PromptKind::Confirm { .. } => {
            if answer.get("answer").and_then(Value::as_bool) == Some(true) {
                Reply::Confirm
            } else {
                Reply::Refuse
            }
        }
        PromptKind::HostPassword { .. }
        | PromptKind::KeyPassphrase { .. }
        | PromptKind::Server { .. } => match answer.get("text").and_then(Value::as_str) {
            Some(text) => Reply::Answer(Secret::new(text.to_owned())),
            None => Reply::Refuse,
        },
        PromptKind::Notice { .. } => Reply::Confirm,
    }
}

/// Answers one connection's prompts by asking the owner through the host.
pub struct Asker {
    peer: Arc<Peer>,
    host: String,
    tries: u32,
    attempts: Mutex<HashMap<String, u32>>,
}

impl std::fmt::Debug for Asker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Asker")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl Asker {
    /// An asker for a connection to `host` (as the owner knows it), with
    /// ssh's count of password prompts.
    pub fn new(peer: Arc<Peer>, host: &str, tries: u32) -> Arc<Self> {
        Arc::new(Self {
            peer,
            host: host.to_owned(),
            tries: tries.max(1),
            attempts: Mutex::new(HashMap::new()),
        })
    }

    /// Answers one prompt: says a notice, asks everything else.
    pub fn answer(&self, prompt: &Prompt) -> Reply {
        let kind = classify(prompt);
        if let PromptKind::Notice { text } = &kind {
            self.peer.notify(
                methods::UI_ANNOUNCE,
                params(json!({"text": text, "importance": "warning"})),
            );
            return Reply::Confirm;
        }
        let attempt = match &kind {
            PromptKind::HostPassword { user, host } => {
                let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
                let n = attempts.entry(format!("{user}@{host}")).or_insert(0);
                *n += 1;
                *n
            }
            _ => 1,
        };
        let Some(question) = question(&kind, &self.host, attempt, self.tries) else {
            return Reply::Refuse;
        };
        let answer = self
            .peer
            .call(methods::UI_ASK, question, None, ANSWER_LIMIT);
        reply(&kind, answer)
    }

    /// Says a line to the owner, as a status (the sites lane's notice that
    /// a saved password was refused).
    pub fn say(&self, text: &str) {
        self.peer.notify(
            methods::UI_ANNOUNCE,
            params(json!({"text": text, "importance": "status"})),
        );
    }

    /// The endpoint's handler for this connection.
    pub fn handler(self: &Arc<Self>) -> Handler {
        let asker = Arc::clone(self);
        Arc::new(move |prompt: &Prompt| asker.answer(prompt))
    }
}

/// ssh's words when it refuses a changed host key.
const CHANGED: &str = "REMOTE HOST IDENTIFICATION HAS CHANGED";

/// The lines said for a changed host key, one fact a line: the refusal,
/// what it may mean, what to do, and the line that forgets the old key,
/// which the owner runs and the plugin never does.
pub fn changed_key_lines(forget: &str) -> Vec<String> {
    vec![
        "Host key changed. Not connected.".to_owned(),
        "The server may be fake or rebuilt.".to_owned(),
        "Ask the server's owner first.".to_owned(),
        "diroxide never edits known_hosts.".to_owned(),
        "To forget the old key, run:".to_owned(),
        forget.to_owned(),
    ]
}

/// The command that forgets a host's recorded key, as the owner would
/// type it: `ssh-keygen -R build.example`, or with the port in brackets
/// when it is not 22.
pub fn forget_command(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) if port != 22 => format!("ssh-keygen -R \"[{host}]:{port}\""),
        _ => format!("ssh-keygen -R {host}"),
    }
}

/// When ssh ended because the host's key changed, the refusal: an
/// `untrusted` error whose hint holds the lines of [`changed_key_lines`]
/// after the first. `None` for any other ending.
pub fn host_key_refusal(stderr: &str, host: &str, port: Option<u16>) -> Option<RpcError> {
    if !stderr.to_ascii_uppercase().contains(CHANGED) {
        return None;
    }
    let lines = changed_key_lines(&forget_command(host, port));
    Some(RpcError::with_hint(
        ErrorKind::Untrusted,
        lines[0].clone(),
        lines[1..].join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::fingerprint::Fingerprint;
    use super::*;

    #[test]
    fn the_new_key_question_is_the_golden_dialog() {
        let kind = PromptKind::NewHostKey {
            host: "build.example".into(),
            key_type: "ED25519".into(),
            fingerprint: Fingerprint::parse("SHA256:ITQUG4Q2ipSm2NXBIVWHWY00pQAnxvt9xPmEMC5l+x0")
                .unwrap(),
        };
        let made = Value::Object(question(&kind, "build.example", 1, 3).unwrap());
        let golden: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/sftp/dialogs/new-host-key.json"
        ))
        .unwrap();
        assert_eq!(made, golden);
    }

    #[test]
    fn password_questions_name_the_account_and_count_the_tries() {
        let kind = PromptKind::HostPassword {
            user: "alice".into(),
            host: "build.example".into(),
        };
        let first = Value::Object(question(&kind, "build.example", 1, 3).unwrap());
        assert_eq!(first["title"], "Password for alice at build.example");
        assert_eq!(first["message"], json!([]));
        let second = Value::Object(question(&kind, "build.example", 2, 3).unwrap());
        assert_eq!(
            second,
            serde_json::from_str::<Value>(include_str!(
                "../../../../fixtures/sftp/dialogs/password.json"
            ))
            .map(|mut v| {
                v["title"] = json!("Password for alice at build.example");
                v
            })
            .unwrap()
        );
        let server = Value::Object(
            question(
                &PromptKind::Server {
                    text: "Verification code:".into(),
                },
                "build.example",
                1,
                3,
            )
            .unwrap(),
        );
        assert_eq!(server["title"], "Question from build.example:");
        assert_eq!(server["kind"], "secret");
    }

    #[test]
    fn only_a_typed_yes_trusts_a_key() {
        let kind = PromptKind::NewHostKey {
            host: "build.example".into(),
            key_type: "ED25519".into(),
            fingerprint: Fingerprint::parse("SHA256:ITQUG4Q2ipSm2NXBIVWHWY00pQAnxvt9xPmEMC5l+x0")
                .unwrap(),
        };
        assert!(matches!(
            reply(&kind, Ok(json!({"answer": true}))),
            Reply::Answer(s) if s.expose() == "yes"
        ));
        for answer in [
            Ok(json!({"answer": false})),
            Ok(json!({"cancelled": true})),
            Ok(json!({"text": "yes"})),
            Ok(json!({})),
            Err(RpcError::new(ErrorKind::TimedOut, "no answer")),
        ] {
            assert!(matches!(reply(&kind, answer), Reply::Refuse));
        }
    }

    #[test]
    fn a_changed_key_is_refused_with_the_line_to_forget_it() {
        let stderr =
            include_str!("../../../../fixtures/sftp/stderr/changed-host-key-windows-9.5p2.txt");
        let refusal = host_key_refusal(stderr, "build.example", Some(22)).unwrap();
        assert_eq!(refusal.kind(), Some(ErrorKind::Untrusted));
        assert_eq!(refusal.message, "Host key changed. Not connected.");
        assert_eq!(
            refusal.hint(),
            Some(
                "The server may be fake or rebuilt.\nAsk the server's owner first.\ndiroxide never edits known_hosts.\nTo forget the old key, run:\nssh-keygen -R build.example"
            )
        );
        assert_eq!(
            forget_command("build.example", Some(2222)),
            "ssh-keygen -R \"[build.example]:2222\""
        );
        assert!(host_key_refusal("Host key verification failed.", "x", None).is_none());
        for line in changed_key_lines(&forget_command("build.example", None)) {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }
}
