//! A host for the provider's tests: the plugin's own JSON-RPC loop on a
//! thread, spoken to as the app speaks to it, answering `ui/ask` from a
//! script and keeping every notice the plugin sends.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use diro_sftp::plugin::Plugin;
use diro_sftp::rpc;
use diro_sftp::sftp::fake::{EndWriter, duplex};
use serde_json::{Value, json};

/// How long a test waits for one answer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(60);

/// The host side of one plugin.
pub struct TestHost {
    out: Mutex<EndWriter>,
    next: AtomicI64,
    waiting: Arc<Mutex<HashMap<i64, Sender<Value>>>>,
    notices: Arc<Mutex<Vec<Value>>>,
    asked: Arc<Mutex<Vec<Value>>>,
    plugin: Arc<Plugin>,
}

impl TestHost {
    /// Starts `plugin` with `settings`, its folders under `root`, and the
    /// answers the owner gives to its questions, in order.
    pub fn start(plugin: Arc<Plugin>, root: &Path, settings: Value, answers: Vec<Value>) -> Self {
        let (host_end, plugin_end) = duplex();
        let (plugin_in, plugin_out) = plugin_end.split();
        let (host_in, host_out) = host_end.split();
        {
            let plugin = Arc::clone(&plugin);
            thread::spawn(move || {
                rpc::serve(BufReader::new(plugin_in), Box::new(plugin_out), plugin)
            });
        }
        let waiting: Arc<Mutex<HashMap<i64, Sender<Value>>>> = Arc::default();
        let notices: Arc<Mutex<Vec<Value>>> = Arc::default();
        let asked: Arc<Mutex<Vec<Value>>> = Arc::default();
        {
            let (waiting, notices, asked) = (
                Arc::clone(&waiting),
                Arc::clone(&notices),
                Arc::clone(&asked),
            );
            let mut out = host_out.clone();
            let mut answers: VecDeque<Value> = answers.into();
            thread::spawn(move || {
                for line in BufReader::new(host_in).lines() {
                    let Ok(line) = line else { return };
                    let Ok(message) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if message.get("method").is_some() && message.get("id").is_some() {
                        // The plugin asks the host: ui/ask.
                        asked.lock().unwrap().push(message["params"].clone());
                        let answer = answers.pop_front().unwrap_or(json!({"cancelled": true}));
                        let reply =
                            json!({"jsonrpc": "2.0", "id": message["id"], "result": answer});
                        let _ = writeln!(out, "{reply}");
                    } else if message.get("method").is_some() {
                        notices.lock().unwrap().push(message);
                    } else if let Some(id) = message["id"].as_i64()
                        && let Some(tx) = waiting.lock().unwrap().remove(&id)
                    {
                        let _ = tx.send(message);
                    }
                }
            });
        }
        let spool = root.join("spool");
        let state = root.join("state");
        std::fs::create_dir_all(&spool).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let host = Self {
            out: Mutex::new(host_out),
            next: AtomicI64::new(1),
            waiting,
            notices,
            asked,
            plugin,
        };
        let init = host.call(
            "initialize",
            json!({
                "protocol": 1,
                "settings": settings,
                "dirs": {"state": state, "spool": spool, "plugin": root},
            }),
        );
        assert!(init.is_ok(), "{init:?}");
        host
    }

    /// The plugin.
    pub fn plugin(&self) -> &Arc<Plugin> {
        &self.plugin
    }

    fn send(&self, message: &Value) {
        let mut out = self.out.lock().unwrap();
        writeln!(out, "{message}").unwrap();
    }

    /// Sends a request and gives its id and a receiver for its answer.
    pub fn send_request(&self, method: &str, params: Value) -> (i64, Receiver<Value>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, tx);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        (id, rx)
    }

    /// Calls the plugin and waits for its answer: the result, or the
    /// error object.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, Value> {
        let (_, rx) = self.send_request(method, params);
        answer_of(&rx)
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Cancels one of the host's requests.
    pub fn cancel(&self, id: i64) {
        self.notify("$/cancel_request", json!({"id": id}));
    }

    /// The notices with a method, their parameters, in order.
    pub fn notices(&self, method: &str) -> Vec<Value> {
        self.notices
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m["method"] == method)
            .map(|m| m["params"].clone())
            .collect()
    }

    /// The questions the plugin asked.
    pub fn asked(&self) -> Vec<Value> {
        self.asked.lock().unwrap().clone()
    }

    /// Waits until `done` holds, up to ten seconds.
    pub fn wait_for(&self, mut done: impl FnMut(&Self) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if done(self) {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

/// The answer that arrives on `rx`: the result, or the error object.
pub fn answer_of(rx: &Receiver<Value>) -> Result<Value, Value> {
    let message = rx.recv_timeout(ANSWER_WAIT).expect("an answer in time");
    match message.get("error") {
        Some(error) => Err(error.clone()),
        None => Ok(message["result"].clone()),
    }
}

/// The error's kind name, from its data.
pub fn kind(error: &Value) -> &str {
    error["data"]["kind"].as_str().unwrap_or("")
}
