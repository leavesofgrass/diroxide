//! What the tests share: a scratch folder under the build's target folder,
//! real git with a placeholder identity and none of the owner's
//! configuration, and a fake host that speaks the protocol to the plugin
//! and answers its `tools/run` by running that git.
//!
//! Every git the tests run, directly or for the plugin, gets
//! `GIT_CONFIG_GLOBAL` (an empty file of the test's own),
//! `GIT_CONFIG_NOSYSTEM`, a `HOME` of the test's own, "Ada Example" with an
//! `example.invalid` address, `GIT_TERMINAL_PROMPT=0` and
//! `GIT_CEILING_DIRECTORIES` at the test's folder, every inherited `GIT_*`
//! variable removed, so the owner's identity, configuration and credential
//! helpers never enter a test. Remotes are `file://` folders of the test's
//! own; nothing touches the network.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use diro_git::plugin::Plugin;
use diro_git::rpc;
use diroxide_plugins::base64;
use serde_json::{Value, json};

/// A folder for the test under the build's target folder, never the
/// system's temporary folder.
pub fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("git-it-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("git-")
        .tempdir_in(base)
        .unwrap()
}

/// A path as git's shell and the scripts read it: forward slashes.
pub fn slashed(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Real git, kept away from the owner's configuration.
pub struct Git {
    /// The test's folder; repositories live inside it.
    pub root: tempfile::TempDir,
    home: PathBuf,
}

impl Git {
    /// A new folder with an empty global configuration and a home.
    pub fn new() -> Self {
        let root = scratch();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("gitconfig"), "").unwrap();
        Self { root, home }
    }

    /// The test's folder.
    pub fn path(&self) -> &Path {
        self.root.path()
    }

    /// A git command in `cwd` with the placeholder identity and nothing of
    /// the owner's.
    pub fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new("git");
        command.current_dir(cwd);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("GIT_") {
                command.env_remove(name);
            }
        }
        command
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("GIT_AUTHOR_NAME", "Ada Example")
            .env("GIT_AUTHOR_EMAIL", "ada@example.invalid")
            .env("GIT_COMMITTER_NAME", "Ada Example")
            .env("GIT_COMMITTER_EMAIL", "ada@example.invalid")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CEILING_DIRECTORIES", self.root.path())
            .env("LC_ALL", "C")
            .stdin(Stdio::null());
        command
    }

    /// Runs git and gives its output; fails the test when git does.
    pub fn run(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.command(cwd).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Runs git and gives its whole output, whatever its code.
    pub fn output(&self, cwd: &Path, args: &[&str]) -> std::process::Output {
        self.command(cwd).args(args).output().unwrap()
    }

    /// Runs git with text on its standard input; fails the test when git
    /// does.
    pub fn run_with_input(&self, cwd: &Path, args: &[&str], input: &[u8]) -> String {
        let mut child = self
            .command(cwd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Puts a file in the index under a name that is never made on disk
    /// (spaces, glob characters, letters beyond ASCII live only inside
    /// git), without committing it.
    pub fn index_only(&self, repo: &Path, name: &str, text: &str) {
        let hash = self.run_with_input(repo, &["hash-object", "-w", "--stdin"], text.as_bytes());
        let info = format!("100644,{},{name}", hash.trim());
        self.run(repo, &["update-index", "--add", "--cacheinfo", &info]);
    }

    /// A new repository on `main` with one committed file, `hello.txt`.
    pub fn repo(&self, name: &str) -> PathBuf {
        let repo = self.path().join(name);
        std::fs::create_dir_all(&repo).unwrap();
        self.run(&repo, &["init", "-q", "--initial-branch=main"]);
        self.run(&repo, &["config", "core.autocrlf", "false"]);
        std::fs::write(repo.join("hello.txt"), "one\ntwo\nthree\n").unwrap();
        self.run(&repo, &["add", "hello.txt"]);
        self.run(&repo, &["commit", "-q", "-m", "First"]);
        repo
    }

    /// A bare repository to push to and fetch from, as a `file://` remote
    /// of `repo`, with `main` pushed and tracked.
    pub fn remote_for(&self, repo: &Path, name: &str) -> PathBuf {
        let bare = self.path().join(name);
        self.run(
            self.path(),
            &["init", "-q", "--bare", "--initial-branch=main", name],
        );
        let address = format!("file://{}", slashed(&bare));
        self.run(repo, &["remote", "add", "origin", &address]);
        self.run(repo, &["push", "-q", "-u", "origin", "main"]);
        bare
    }

    /// Runs git as the host does for `tools/run`: the plugin's arguments,
    /// folder, input and allowed variables.
    fn tools_run(&self, params: &Value) -> (Value, Vec<u8>) {
        let args: Vec<String> = params["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_owned())
            .collect();
        let cwd = params["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.path().to_path_buf());
        let mut command = self.command(&cwd);
        command.args(&args);
        if let Some(env) = params["env"].as_object() {
            for (key, value) in env {
                command.env(key, value.as_str().unwrap());
            }
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let input = params["stdin"].as_str().unwrap_or("").to_owned();
        let mut stdin = child.stdin.take().unwrap();
        let writer = thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
        let out = child.wait_with_output().unwrap();
        writer.join().unwrap();
        let answer = json!({
            "exit_code": out.status.code(),
            "stdout": base64::encode(&out.stdout),
            "stderr": base64::encode(&out.stderr),
            "encoding": "base64",
            "truncated": false,
        });
        (answer, out.stderr)
    }
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
    /// Each `ui/show_diff`'s parameters.
    pub diffs: Vec<Value>,
    /// Each `ui/show_text`'s parameters.
    pub texts: Vec<Value>,
    /// Each `lister/open`'s parameters.
    pub listers: Vec<Value>,
    /// Each `$/progress` value.
    pub progress: Vec<Value>,
    /// Each `fs/write`: its place and bytes.
    pub writes: Vec<(String, Vec<u8>)>,
    /// Each `fs/delete`'s parameters.
    pub deletes: Vec<Value>,
    /// The arguments of each `tools/run`.
    pub tools: Vec<Vec<String>>,
}

impl Seen {
    /// Whether any git run had this word among its arguments.
    pub fn ran(&self, word: &str) -> bool {
        self.tools.iter().any(|args| args.iter().any(|a| a == word))
    }

    /// The last sentence said.
    pub fn last_said(&self) -> &str {
        self.said.last().map(|(t, _)| t.as_str()).unwrap_or("")
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

/// An answer the fake host gives for `tools/run` in place of git.
pub type Fake = Box<dyn Fn(&[String]) -> Option<Value> + Send>;

/// A host that drives the plugin over the protocol, in this process.
pub struct FakeHost {
    to_plugin: Option<Sender<Vec<u8>>>,
    from_plugin: Receiver<String>,
    server: Option<JoinHandle<i32>>,
    next: i64,
    /// The git it runs for the plugin.
    pub git: Arc<Git>,
    /// The plugin, to look at what it keeps.
    pub plugin: Arc<Plugin>,
    /// Answers to `ui/ask` and `ui/form`, in order; Escape when empty.
    pub answers: VecDeque<Value>,
    /// What `fs/write` answers: written (true) or the owner's no.
    pub write_answer: bool,
    /// Answers for `tools/run` in place of git, by its arguments.
    pub fake: Option<Fake>,
    /// What it saw.
    pub seen: Seen,
}

/// How long the fake host waits for one line from the plugin.
const LINE_WAIT: Duration = Duration::from_secs(120);

impl FakeHost {
    /// Starts the plugin and initializes it, its state folder inside the
    /// test's folder and its repository walk stopping there.
    pub fn start(git: Arc<Git>) -> Self {
        let plugin = Plugin::with_ceilings(vec![git.path().to_path_buf()]);
        let (to_plugin, rx) = mpsc::channel::<Vec<u8>>();
        let (tx, from_plugin) = mpsc::channel::<String>();
        let server = {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || {
                rpc::serve(
                    io::BufReader::new(Input {
                        rx,
                        buffer: Vec::new(),
                    }),
                    Box::new(Lines {
                        tx,
                        buffer: Vec::new(),
                    }),
                    plugin,
                )
            })
        };
        let mut host = Self {
            to_plugin: Some(to_plugin),
            from_plugin,
            server: Some(server),
            next: 0,
            git,
            plugin,
            answers: VecDeque::new(),
            write_answer: true,
            fake: None,
            seen: Seen::default(),
        };
        let state = host.git.path().join("state");
        let answer = host
            .request(
                "initialize",
                json!({"protocol": 1, "plugin_id": "git", "dirs": {"state": state.display().to_string()}}),
            )
            .unwrap();
        assert_eq!(answer["plugin"]["name"], "Git");
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
            "tools/run" => {
                let args: Vec<String> = params["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap().to_owned())
                    .collect();
                self.seen.tools.push(args.clone());
                if let Some(answer) = self.fake.as_ref().and_then(|f| f(&args)) {
                    // A fake answer holding "error" is sent as the error.
                    match answer.get("error") {
                        Some(error) => {
                            self.send(&json!({"jsonrpc": "2.0", "id": id, "error": error}))
                        }
                        None => self.answer(&id, answer),
                    }
                    return;
                }
                let (answer, stderr) = self.git.tools_run(&params);
                if params.get("stream").is_some() {
                    // As the host's split "cr": each rewrite of a line.
                    for record in stderr.split(|b| *b == b'\r' || *b == b'\n') {
                        if record.is_empty() {
                            continue;
                        }
                        self.send(&json!({"jsonrpc": "2.0", "method": "tools/output",
                            "params": {"id": id, "stream": "stderr", "chunk": base64::encode(record)}}));
                    }
                    let mut answer = answer;
                    answer["stderr"] = json!(base64::encode(b""));
                    self.answer(&id, answer);
                } else {
                    self.answer(&id, answer);
                }
            }
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
            "ui/show_diff" => {
                self.seen.diffs.push(params);
                self.answer(&id, json!({"shown": true}));
            }
            "ui/show_text" => {
                self.seen.texts.push(params);
                self.answer(&id, json!({"shown": true}));
            }
            "lister/open" => {
                let rows = params["rows"].as_array().map_or(0, Vec::len);
                self.seen.listers.push(params);
                self.answer(&id, json!({"opened": true, "rows": rows}));
            }
            "fs/write" => {
                let place = params["place"].as_str().unwrap().to_owned();
                let bytes = base64::decode(params["data"].as_str().unwrap()).unwrap();
                if self.write_answer {
                    std::fs::write(&place, &bytes).unwrap();
                }
                self.seen.writes.push((place, bytes));
                self.answer(&id, json!({"written": self.write_answer}));
            }
            "fs/delete" => {
                self.seen.deletes.push(params);
                self.answer(
                    &id,
                    json!({"done": 0, "skipped": 1, "failed": [], "cancelled": true}),
                );
            }
            other => {
                self.send(&json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("the fake host does not serve {other}")}}));
            }
        }
    }

    /// The title line's words for a folder, or `None`.
    pub fn describe(&mut self, folder: &Path) -> Option<String> {
        let answer = self
            .request(
                "place/describe",
                json!({"place": folder.display().to_string()}),
            )
            .unwrap();
        answer["text"].as_str().map(str::to_owned)
    }

    /// A page of the Git column for files of a folder, in their order.
    pub fn column(&mut self, folder: &Path, names: &[&str]) -> Vec<String> {
        let places: Vec<String> = names
            .iter()
            .map(|n| folder.join(n).display().to_string())
            .collect();
        let answer = self
            .request(
                "column/format",
                json!({"column": "git", "folder": folder.display().to_string(), "places": places}),
            )
            .unwrap();
        places
            .iter()
            .map(|p| answer["values"][p].as_str().unwrap_or("pending").to_owned())
            .collect()
    }

    /// Runs a command on files of a panel's folder: the marked names (the
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
            "protocol": 1, "command": format!("git.{command}"), "locale": "en-US",
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

    /// Runs a command on rows of one of the plugin's lists, by their
    /// index in the last list of that id it opened.
    pub fn on_rows(
        &mut self,
        command: &str,
        lister: &str,
        folder: &Path,
        rows: &[usize],
    ) -> Result<Value, Value> {
        let opened = self
            .seen
            .listers
            .iter()
            .rev()
            .find(|l| l["lister"] == lister)
            .cloned()
            .expect("the list was opened");
        let selection: Vec<Value> = rows.iter().map(|n| opened["rows"][n].clone()).collect();
        let context = json!({
            "protocol": 1, "command": format!("git.{command}"), "locale": "en-US",
            "active": {"place": folder.display().to_string(), "provider": "local",
                "list": {"plugin": "git", "id": lister, "title": "x", "rows": 1}},
            "other": {"place": folder.display().to_string(), "provider": "local", "list": null},
            "cursor": Value::Null,
            "selection": selection,
            "selection_count": rows.len(), "selection_from_cursor": false, "selection_truncated": false,
        });
        self.request(
            "command/run",
            json!({"command": command, "lister": lister, "context": context, "progress_token": format!("p-{command}")}),
        )
    }

    /// The rows' words of the last list of that id.
    pub fn rows(&self, lister: &str) -> Vec<String> {
        self.seen
            .listers
            .iter()
            .rev()
            .find(|l| l["lister"] == lister)
            .map(|l| {
                l["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["name"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
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

/// A one-line script for git's shell that appends to a marker file, then
/// passes its input through (so it also serves as a filter driver).
pub fn marker_script(git: &Git, name: &str) -> (PathBuf, PathBuf) {
    let script = git.path().join(format!("{name}.sh"));
    let marker = git.path().join(format!("{name}.ran"));
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> \"{}\"\ncat\n", slashed(&marker)),
    )
    .unwrap();
    (script, marker)
}

/// Whether git is on this computer's PATH; every runner of CI has it and
/// so does the development container.
pub fn git_found() -> bool {
    Command::new("git")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A shared handle to a fake host's git, for tests that need both.
pub fn host() -> (Arc<Git>, FakeHost) {
    assert!(git_found(), "these tests need git on the PATH");
    let git = Arc::new(Git::new());
    let host = FakeHost::start(Arc::clone(&git));
    (git, host)
}
