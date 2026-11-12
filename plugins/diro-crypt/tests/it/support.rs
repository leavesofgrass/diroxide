//! What the tests share: a scratch folder under the build's target
//! folder, never the system's temporary folder; and a fake host that
//! speaks the protocol to the plugin in this process, answers its forms
//! and questions from a queue, and keeps every line the plugin wrote, so
//! a test can look for a secret in all of them.
//!
//! Every passphrase and key is made at test time and thrown away; nothing
//! of the owner's is ever encrypted, decrypted or read.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use diro_crypt::plugin::Plugin;
use diro_crypt::rpc;
use serde_json::{Value, json};

/// A folder for the test under the build's target folder.
pub fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("crypt-it-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("crypt-")
        .tempdir_in(base)
        .unwrap()
}

/// A passphrase made for one test: a canary a test can look for.
pub fn canary(tag: &str) -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("canary-{tag}-{t:x}")
}

/// The names in a folder, sorted.
pub fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Collects what the plugin writes, a line at a time.
struct Lines {
    tx: Sender<String>,
    buffer: Vec<u8>,
}

impl Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        while let Some(at) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=at).collect();
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

/// The host's end of the plugin's input.
struct Input {
    rx: Receiver<Vec<u8>>,
    buffer: Vec<u8>,
}

impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.buffer.is_empty() {
            match self.rx.recv() {
                Ok(bytes) => self.buffer = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buffer.len());
        out[..n].copy_from_slice(&self.buffer[..n]);
        self.buffer.drain(..n);
        Ok(n)
    }
}

/// What the fake host saw.
#[derive(Debug, Default)]
pub struct Seen {
    /// Each `ui/announce`: its words and importance.
    pub said: Vec<(String, String)>,
    /// Each `ui/ask`'s parameters.
    pub asked: Vec<Value>,
    /// Each `ui/form`'s parameters.
    pub forms: Vec<Value>,
    /// Each `$/progress` value.
    pub progress: Vec<Value>,
    /// Every line the plugin wrote, whole.
    pub wire: Vec<String>,
    /// Each output diroxide placed through `fs/commit`.
    pub committed: Vec<PathBuf>,
    /// The stages handed out and not committed yet: id, file, output.
    pub open_stages: Vec<(String, PathBuf, PathBuf)>,
}

impl Seen {
    /// Every sentence said, in order.
    pub fn sentences(&self) -> Vec<&str> {
        self.said.iter().map(|(t, _)| t.as_str()).collect()
    }

    /// Whether a sentence was said.
    pub fn heard(&self, sentence: &str) -> bool {
        self.said.iter().any(|(t, _)| t == sentence)
    }

    /// Every sentence said, joined for a message.
    pub fn all_said(&self) -> String {
        self.said
            .iter()
            .map(|(t, i)| format!("{i}: {t}"))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// A host that drives the plugin over the protocol, in this process.
pub struct FakeHost {
    to_plugin: Option<Sender<Vec<u8>>>,
    from_plugin: Receiver<String>,
    server: Option<JoinHandle<i32>>,
    next: i64,
    /// Answers to `ui/ask` and `ui/form`, in order; Escape when empty.
    pub answers: VecDeque<Value>,
    /// What it saw.
    pub seen: Seen,
}

/// How long the fake host waits for one line from the plugin.
const LINE_WAIT: Duration = Duration::from_secs(120);

impl FakeHost {
    /// Starts the plugin and initializes it.
    pub fn start() -> Self {
        let (to_plugin, rx) = mpsc::channel::<Vec<u8>>();
        let (tx, from_plugin) = mpsc::channel::<String>();
        let server = thread::spawn(move || {
            rpc::serve(
                io::BufReader::new(Input {
                    rx,
                    buffer: Vec::new(),
                }),
                Box::new(Lines {
                    tx,
                    buffer: Vec::new(),
                }),
                Plugin::new(),
            )
        });
        let mut host = Self {
            to_plugin: Some(to_plugin),
            from_plugin,
            server: Some(server),
            next: 0,
            answers: VecDeque::new(),
            seen: Seen::default(),
        };
        let answer = host
            .request("initialize", json!({"protocol": 1, "plugin_id": "crypt"}))
            .unwrap();
        assert_eq!(answer["plugin"]["name"], "Encryption");
        host
    }

    fn send(&self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        if let Some(tx) = &self.to_plugin {
            tx.send(line).unwrap();
        }
    }

    /// Sends a request and serves the plugin's own requests until it is
    /// answered: its result, or its error object.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next += 1;
        let id = format!("host-{}", self.next);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .from_plugin
                .recv_timeout(LINE_WAIT)
                .expect("the plugin answered in time");
            if line.is_empty() {
                continue;
            }
            self.seen.wire.push(line.clone());
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").is_none() {
                if message["id"] == json!(id) {
                    return match message.get("error") {
                        Some(error) => Err(error.clone()),
                        None => Ok(message["result"].clone()),
                    };
                }
                continue;
            }
            self.serve(&message);
        }
    }

    fn answer(&self, id: &Value, result: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn serve(&mut self, message: &Value) {
        let method = message["method"].as_str().unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let Some(id) = message.get("id") else {
            match method {
                "ui/announce" => self.seen.said.push((
                    params["text"].as_str().unwrap_or("").to_owned(),
                    params["importance"].as_str().unwrap_or("info").to_owned(),
                )),
                "$/progress" => self.seen.progress.push(params["value"].clone()),
                _ => {}
            }
            return;
        };
        let id = id.clone();
        match method {
            "ui/ask" | "ui/form" => {
                if method == "ui/ask" {
                    self.seen.asked.push(params);
                } else {
                    self.seen.forms.push(params);
                }
                let answer = self
                    .answers
                    .pop_front()
                    .unwrap_or_else(|| json!({"cancelled": true}));
                self.answer(&id, answer);
            }
            // As the host's staging: a `.diro-part-` name beside the
            // output, refused when the output's name is taken; the commit
            // a rename never over a file, a taken name numbered.
            "fs/stage" => {
                let output = PathBuf::from(params["place"].as_str().unwrap_or(""));
                if output.exists() {
                    self.send(&json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32010, "message": "it exists; stage another name"}}));
                    return;
                }
                self.next += 1;
                let stage = format!("{:08x}", self.next);
                let path = output
                    .parent()
                    .unwrap()
                    .join(format!(".diro-part-{stage}.age"));
                self.seen
                    .open_stages
                    .push((stage.clone(), path.clone(), output));
                self.answer(
                    &id,
                    json!({"stage": stage, "path": path.display().to_string()}),
                );
            }
            "fs/commit" => {
                let stage = params["stage"].as_str().unwrap_or("");
                let at = self.seen.open_stages.iter().position(|(s, ..)| s == stage);
                let Some(at) = at else {
                    self.send(&json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32004, "message": format!("no stage is named {stage}")}}));
                    return;
                };
                let (_, path, output) = self.seen.open_stages.remove(at);
                let mut target = output.clone();
                let mut n = 1;
                while target.exists() {
                    n += 1;
                    let name = output.file_name().unwrap().to_string_lossy();
                    target = output.with_file_name(format!("{name} ({n})"));
                }
                std::fs::rename(&path, &target).unwrap();
                self.seen.committed.push(target.clone());
                let name = target.file_name().unwrap().to_string_lossy().into_owned();
                self.answer(
                    &id,
                    json!({"committed": true, "place": target.display().to_string(), "name": name}),
                );
            }
            other => {
                self.send(&json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("the fake host does not serve {other}")}}));
            }
        }
    }

    /// Runs a command on files of a folder: the marked names (the
    /// cursor's when none), each a file unless it ends with `/`.
    pub fn command(
        &mut self,
        command: &str,
        folder: &Path,
        marked: &[&str],
    ) -> Result<Value, Value> {
        let selection: Vec<Value> = marked
            .iter()
            .map(|name| {
                let folder_like = name.ends_with('/');
                let name = name.trim_end_matches('/');
                json!({"name": name, "kind": if folder_like {"folder"} else {"file"},
                    "place": folder.join(name).display().to_string()})
            })
            .collect();
        let context = json!({
            "protocol": 1, "command": format!("crypt.{command}"), "locale": "en-US",
            "active": {"place": folder.display().to_string(), "provider": "local", "list": null},
            "other": {"place": folder.display().to_string(), "provider": "local", "list": null},
            "cursor": selection.first().cloned().unwrap_or(Value::Null),
            "selection": selection,
            "selection_count": marked.len(), "selection_from_cursor": false, "selection_truncated": false,
        });
        self.request(
            "command/run",
            json!({"command": command, "context": context, "progress_token": format!("p-{command}")}),
        )
    }

    /// The answer the passphrase form gets: the same passphrase twice.
    pub fn passphrase_form(&mut self, passphrase: &str, armor: bool) {
        self.answers.push_back(json!({"values":
            {"passphrase": passphrase, "repeat": passphrase, "armor": armor}}));
    }

    /// The answer a hidden question gets.
    pub fn secret(&mut self, passphrase: &str) {
        self.answers.push_back(json!({"text": passphrase}));
    }

    /// Escape, for the next question or form.
    pub fn escape(&mut self) {
        self.answers.push_back(json!({"cancelled": true}));
    }
}

impl Drop for FakeHost {
    fn drop(&mut self) {
        // The end of input ends the session.
        self.to_plugin = None;
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

/// A file of `size` bytes of a pattern, in `folder`.
pub fn file(folder: &Path, name: &str, size: usize) -> PathBuf {
    let path = folder.join(name);
    let body: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
    std::fs::write(&path, body).unwrap();
    path
}
