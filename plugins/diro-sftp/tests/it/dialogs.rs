//! The host key and password dialogs (R25 commit 53): scripted sessions
//! with the fake ssh, its helper the plugin's own program, and a fake host
//! that answers the plugin's `ui/ask` from a script.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diro_sftp::askpass::Listener;
use diro_sftp::askpass::dialogs::{Asker, host_key_refusal};
use diro_sftp::rpc::Peer;
use diro_sftp::ssh::Launch;
use diroxide_plugins::protocol::{ErrorKind, Id, Response};
use serde_json::{Value, json};

use crate::fakes::{fake_ssh_in, log_lines};
use crate::plugin_exe;

const NEW_KEY: &str =
    include_str!("../../../../fixtures/sftp/prompts/new-host-key-windows-9.5p2.txt");
const CHANGED: &str =
    include_str!("../../../../fixtures/sftp/stderr/changed-host-key-windows-9.5p2.txt");

/// Lines the plugin writes, sent on as they come.
struct Lines {
    buffer: Vec<u8>,
    tx: Sender<String>,
}

impl Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let _ = self
                .tx
                .send(String::from_utf8_lossy(&line).trim().to_owned());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A host that answers each `ui/ask` with the next scripted answer and
/// keeps every line the plugin wrote.
struct FakeHost {
    peer: Arc<Peer>,
    written: Arc<Mutex<Vec<String>>>,
}

impl FakeHost {
    fn new(answers: Vec<Value>) -> Self {
        let (tx, rx) = mpsc::channel::<String>();
        let peer = Arc::new(Peer::new(Box::new(Lines {
            buffer: Vec::new(),
            tx,
        })));
        let written = Arc::new(Mutex::new(Vec::new()));
        let (weak, kept) = (Arc::downgrade(&peer), Arc::clone(&written));
        let mut answers: VecDeque<Value> = answers.into();
        thread::spawn(move || {
            for line in rx {
                kept.lock().unwrap().push(line.clone());
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message["method"] == "ui/ask"
                    && let (Some(id), Some(peer)) = (message["id"].as_i64(), weak.upgrade())
                {
                    let answer = answers.pop_front().unwrap_or(json!({"cancelled": true}));
                    peer.deliver(Response::success(Id::Number(id), answer));
                }
            }
        });
        Self { peer, written }
    }

    /// The questions the plugin asked, as sent.
    fn asked(&self) -> Vec<Value> {
        self.written
            .lock()
            .unwrap()
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|m| m["method"] == "ui/ask")
            .map(|m| m["params"].clone())
            .collect()
    }
}

/// Runs one connection: the fake ssh with `script`, the plugin's helper
/// and endpoint, the host answering `answers`. Gives the host, ssh's exit
/// code, its standard error and its log.
fn session(script: Value, answers: Vec<Value>) -> (FakeHost, Option<i32>, String, Vec<Value>) {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("ssh.log");
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"log": log, "script": script}),
    );
    let host = FakeHost::new(answers);
    let asker = Asker::new(Arc::clone(&host.peer), "build.example", 3);
    let mut listener = Listener::open(Some(root.path()), asker.handler()).unwrap();
    let mut process = Launch {
        program: ssh,
        args: vec!["--".into(), "build.example".into(), "sftp".into()],
        askpass: Some(listener.askpass_env(plugin_exe())),
    }
    .spawn()
    .unwrap();
    let status = process.wait_timeout(Duration::from_secs(30));
    for _ in 0..200 {
        if process.stderr_ended() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let stderr = process.stderr_text();
    listener.close();
    let log = log_lines(&log);
    (host, status.and_then(|s| s.code()), stderr, log)
}

#[test]
fn a_new_key_is_asked_on_no_and_a_yes_goes_to_ssh() {
    let (host, code, _, log) = session(
        json!([{"ask": NEW_KEY, "accept": "yes", "refused": "Host key verification failed."},
               {"exit": 0}]),
        vec![json!({"answer": true})],
    );
    assert_eq!(code, Some(0));
    let asked = host.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["kind"], "confirm");
    assert_eq!(asked[0]["destructive"], true);
    assert_eq!(asked[0]["title"], "New server key");
    assert_eq!(
        asked[0]["message"][0],
        "New server key for [127.0.0.1]:2222."
    );
    assert_eq!(asked[0]["message"][4], "Part 1 of 4: 1dc2 1573 dcf0 c230");
    let answered: Vec<&Value> = log.iter().filter(|l| l.get("asked").is_some()).collect();
    assert_eq!(answered[0]["answer_word"], "yes");
}

#[test]
fn a_no_or_an_escape_refuses_the_key() {
    for answer in [json!({"answer": false}), json!({"cancelled": true})] {
        let (host, code, stderr, log) = session(
            json!([{"ask": NEW_KEY, "accept": "yes", "refused": "Host key verification failed."},
                   {"exit": 0}]),
            vec![answer],
        );
        assert_eq!(code, Some(255));
        assert!(stderr.contains("Host key verification failed."));
        assert_eq!(host.asked().len(), 1);
        let answered: Vec<&Value> = log.iter().filter(|l| l.get("asked").is_some()).collect();
        assert_eq!(answered[0]["matched"], false);
        assert_eq!(answered[0]["exit"], 1);
    }
}

/// Three wrong passwords: each try is asked with its number, ssh gives up
/// after the third, and no secret appears in any log or line the plugin
/// wrote.
#[test]
fn three_wrong_passwords_end_the_connection_and_leave_no_secret() {
    let right = "right-horse-fixture";
    let wrong = [
        "wrong-one-fixture",
        "wrong-two-fixture",
        "wrong-three-fixture",
    ];
    let (host, code, stderr, log) = session(
        json!([{"ask": "ada@build.example's password: ", "accept": right, "tries": 3,
                "refused": "ada@build.example: Permission denied (password)."},
               {"exit": 0}]),
        wrong.iter().map(|w| json!({"text": w})).collect(),
    );
    assert_eq!(code, Some(255));
    assert!(stderr.contains("Permission denied"));
    let asked = host.asked();
    let titles: Vec<&Value> = asked.iter().map(|a| &a["title"]).collect();
    assert_eq!(titles, [&json!("Password for ada at build.example"); 3]);
    let messages: Vec<&Value> = asked.iter().map(|a| &a["message"]).collect();
    assert_eq!(
        messages,
        [&json!([]), &json!(["Try 2 of 3."]), &json!(["Try 3 of 3."])]
    );
    assert!(asked.iter().all(|a| a["kind"] == "secret"));
    // The log scan.
    let everything = format!(
        "{}\n{}\n{stderr}",
        host.written.lock().unwrap().join("\n"),
        serde_json::to_string(&log).unwrap()
    );
    for secret in std::iter::once(&right).chain(wrong.iter()) {
        assert!(!everything.contains(secret), "{secret} reached a log");
    }
    // The right one on the second try connects.
    let (_, code, _, _) = session(
        json!([{"ask": "ada@build.example's password: ", "accept": right, "tries": 3,
                "refused": "ada@build.example: Permission denied (password)."},
               {"exit": 0}]),
        vec![json!({"text": wrong[0]}), json!({"text": right})],
    );
    assert_eq!(code, Some(0));
}

/// A changed key never reaches a dialog: ssh refuses it, the refusal is
/// an `untrusted` error with the line to forget the old key, and nothing
/// runs that line.
#[test]
fn a_changed_key_is_refused_and_the_forget_line_never_run() {
    let (host, code, stderr, log) =
        session(json!([{"stderr": CHANGED}, {"exit": 255}]), Vec::new());
    assert_eq!(code, Some(255));
    assert!(host.asked().is_empty());
    let refusal = host_key_refusal(&stderr, "build.example", None).unwrap();
    assert_eq!(refusal.kind(), Some(ErrorKind::Untrusted));
    assert!(
        refusal
            .hint()
            .unwrap()
            .ends_with("ssh-keygen -R build.example")
    );
    // One program ran: ssh. Never ssh-keygen.
    let runs: Vec<&Value> = log.iter().filter(|l| l.get("exe").is_some()).collect();
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0]["exe"]
            .as_str()
            .unwrap()
            .ends_with(crate::fakes::ssh_name())
    );
}
