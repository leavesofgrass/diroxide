//! Hooks wired in the app (lane A2, R25 commit 81), driven without a
//! terminal and without a real process: a guard plugin in the test's
//! config folder, served in-process over a memory transport, answers
//! `hook/before` as the example `folder-guard` does, slowly when a test
//! asks, and keeps every `hook/after` it hears. Nothing touches the
//! network.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_config::Paths;
use diroxide_plugins::PluginError;
use diroxide_plugins::host::{Launcher, PluginInfo};
use diroxide_plugins::log::PluginLog;
use diroxide_plugins::manifest::Manifest;
use diroxide_plugins::script::consent::{ConsentStore, Kind, plugin_fingerprint};
use diroxide_plugins::transport::{MemoryPeer, MemoryTransport, PluginTransport};
use serde_json::{Value, json};

use crate::support::{Session, Setup, fixture, place, settings, temp};

/// The guard's manifest: before and after deletes, and the folders it
/// enters, with a hook limit of one second.
const MANIFEST: &str = r#"
[plugin]
id = "folder-guard"
name = "Folder guard"
version = "0.1.0"
protocol = 1

[run]
command = ["test-guard"]
startup_timeout = "2s"

[timeouts]
hook = "1s"

[permissions]
fs = "none"
panel = "none"

[permissions.declared]
network = "no network"

[[hook]]
event = "before_delete"
on_timeout = "continue"

[[hook]]
event = "after_delete"

[[hook]]
event = "enter_folder"
"#;

/// The in-process guard: what it protects, how long it thinks, and what
/// it heard after.
#[derive(Default)]
struct Guard {
    protect: Mutex<String>,
    think: Mutex<Duration>,
    heard: Arc<Mutex<Vec<Value>>>,
}

impl Launcher for Guard {
    fn launch(
        &self,
        info: &PluginInfo,
        _log: &PluginLog,
    ) -> Result<Arc<dyn PluginTransport>, PluginError> {
        let (host, peer) = MemoryTransport::pair(&info.manifest.name)?;
        let protect = self.protect.lock().unwrap().clone();
        let think = *self.think.lock().unwrap();
        let heard = Arc::clone(&self.heard);
        thread::spawn(move || serve(&peer, &protect, think, &heard));
        Ok(Arc::new(host))
    }
}

fn write(peer: &MemoryPeer, value: Value) {
    peer.write_line(&value.to_string());
}

fn serve(peer: &MemoryPeer, protect: &str, think: Duration, heard: &Mutex<Vec<Value>>) {
    while let Ok(line) = peer.from_host.recv() {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message.get("id").cloned();
        match message.get("method").and_then(Value::as_str) {
            Some("initialize") => write(
                peer,
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocol": 1,
                    "plugin": {"name": "Folder guard", "version": "0.1.0"},
                    "plugin_capabilities": {"commands": []}
                }}),
            ),
            Some("hook/before") => {
                thread::sleep(think);
                let items = message["params"]["items"].as_array().cloned();
                let guarded = items.unwrap_or_default().iter().any(|item| {
                    item["from"]
                        .as_str()
                        .is_some_and(|from| !protect.is_empty() && from.contains(protect))
                });
                let answer = if guarded {
                    json!({"action": "cancel", "reason": "folder is protected"})
                } else {
                    json!({"action": "continue"})
                };
                write(peer, json!({"jsonrpc": "2.0", "id": id, "result": answer}));
            }
            Some("hook/after") => heard.lock().unwrap().push(message["params"].clone()),
            Some("shutdown") => write(peer, json!({"jsonrpc": "2.0", "id": id, "result": null})),
            Some("exit") => return,
            Some(_) if id.is_some() => write(
                peer,
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "no method"}}),
            ),
            _ => {}
        }
    }
}

/// The test's folders, the guard on disk with its consent, and a session.
fn session(guard: &Arc<Guard>) -> (tempfile::TempDir, Paths, std::path::PathBuf, Session) {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let folder = paths.plugins_dir().join("folder-guard");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(Manifest::FILE), MANIFEST).unwrap();
    let manifest = Manifest::from_toml(MANIFEST).unwrap();
    let fingerprint =
        plugin_fingerprint(&manifest, &folder, diroxide_components::sha256_hex).unwrap();
    ConsentStore::open(&paths.state_dir.join("plugins"))
        .enable("folder-guard", Kind::Plugin, fingerprint)
        .unwrap();
    let work = fixture(dir.path());
    let launcher: Arc<dyn Launcher> = guard.clone();
    let s = Session::open(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings: settings(),
            paths: Some(paths.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    );
    (dir, paths, work, s)
}

fn wait_for(what: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    what()
}

#[test]
fn a_veto_stops_the_delete_before_its_question_with_the_plugin_last() {
    let guard = Arc::new(Guard::default());
    *guard.protect.lock().unwrap() = "Backup".to_owned();
    let (_dir, _paths, work, mut s) = session(&guard);
    s.focus("Backup");
    let said = s.key("f8");
    assert!(
        said.contains(&"Delete stopped: folder is protected. By Folder guard.".to_owned()),
        "{said:?}"
    );
    assert!(s.app.dialog().is_none(), "no question was asked");
    assert!(work.join("Backup").join("report.pdf").exists());
    for line in &said {
        assert!(line.chars().count() <= 60, "{line}");
    }
    // A delete elsewhere goes on, asks as always, and the guard hears it.
    s.focus("notes.txt");
    s.key("f8");
    assert!(
        s.dialog_title()
            .unwrap()
            .starts_with("Move 1 item to the trash")
    );
    s.key("y");
    let heard = Arc::clone(&guard.heard);
    assert!(
        wait_for(|| heard
            .lock()
            .unwrap()
            .iter()
            .any(|h| h["event"] == "after_delete" && h["status"] == "done")),
        "{:?}",
        guard.heard.lock().unwrap()
    );
    assert!(!work.join("notes.txt").exists() || cfg!(target_os = "macos"));
}

#[test]
fn a_slow_hook_lets_the_delete_go_on_and_says_so() {
    let guard = Arc::new(Guard::default());
    *guard.think.lock().unwrap() = Duration::from_secs(6);
    let (_dir, _paths, _work, mut s) = session(&guard);
    s.focus("notes.txt");
    let started = Instant::now();
    let said = s.key("f8");
    assert!(
        said.contains(&"Delete goes on: no answer. By Folder guard.".to_owned()),
        "{said:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "never waits for the hook"
    );
    assert!(
        s.dialog_title()
            .unwrap()
            .starts_with("Move 1 item to the trash"),
        "the question still comes"
    );
    s.key("escape");
}

#[test]
fn entering_folders_is_told_once_things_are_still() {
    let guard = Arc::new(Guard::default());
    let (_dir, _paths, work, mut s) = session(&guard);
    // Two folders in a row: only the last is told, once.
    s.focus("Photos");
    s.key("enter");
    s.key("backspace");
    s.focus("Backup");
    s.key("enter");
    let heard = Arc::clone(&guard.heard);
    let backup = place(&work.join("Backup")).to_string();
    assert!(
        wait_for(|| heard
            .lock()
            .unwrap()
            .iter()
            .any(|h| h["event"] == "enter_folder" && h["place"] == backup.as_str())),
        "{:?}",
        guard.heard.lock().unwrap()
    );
    thread::sleep(Duration::from_millis(400));
    let entered: Vec<Value> = guard
        .heard
        .lock()
        .unwrap()
        .iter()
        .filter(|h| h["event"] == "enter_folder")
        .cloned()
        .collect();
    assert_eq!(entered.last().unwrap()["place"], backup.as_str());
    assert!(entered.len() <= 2, "{entered:?}");
    let _ = Path::new(&backup);
}
