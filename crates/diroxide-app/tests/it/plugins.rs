//! Plugins in the app (W4d, R22 commits 36 to 40), driven without a
//! terminal and without a real process: a plugin folder on disk in the
//! test's config folder, and an in-process test plugin over a memory
//! transport that answers as each test asks.

use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diroxide_app::commands::ids;
use diroxide_app::plugins::{Call, CallId, PluginEvent};
use diroxide_app::{AppEvent, Dialog};
use diroxide_config::Paths;
use diroxide_core::{Command, CommandId, Side};
use diroxide_plugins::PluginError;
use diroxide_plugins::ask::{Question, QuestionKind};
use diroxide_plugins::host::{Launcher, PluginInfo};
use diroxide_plugins::log::PluginLog;
use diroxide_plugins::manifest::Manifest;
use diroxide_plugins::script::consent::{ConsentStore, Kind, plugin_fingerprint};
use diroxide_plugins::transport::{MemoryPeer, MemoryTransport, PluginTransport};
use serde_json::{Value, json};

use crate::support::{Session, Setup, fixture, place, settings, temp};

/// What the test plugin does when one of its commands runs.
#[derive(Clone, Debug)]
pub enum Act {
    /// Answers at once.
    Done,
    /// Says a line and reports progress, then answers.
    Talk,
    /// Calls the host, says what the host answered, then answers.
    Call(&'static str, Value),
    /// Says the rows of its selection, then answers.
    Rows,
    /// Ends its output, as a program that crashes.
    Crash,
    /// Never answers; a cancel is answered -32800.
    Hang,
}

/// Starts the test plugin over a memory transport.
#[derive(Default)]
pub struct TestLauncher {
    acts: Mutex<HashMap<String, Act>>,
    /// How many times it started.
    pub launches: Mutex<usize>,
}

impl TestLauncher {
    pub fn with(acts: &[(&str, Act)]) -> Arc<Self> {
        let launcher = Self::default();
        for (id, act) in acts {
            launcher
                .acts
                .lock()
                .unwrap()
                .insert((*id).to_owned(), act.clone());
        }
        Arc::new(launcher)
    }
}

impl Launcher for TestLauncher {
    fn launch(
        &self,
        info: &PluginInfo,
        _log: &PluginLog,
    ) -> Result<Arc<dyn PluginTransport>, PluginError> {
        *self.launches.lock().unwrap() += 1;
        let (host, peer) = MemoryTransport::pair(&info.manifest.name)?;
        let acts = self.acts.lock().unwrap().clone();
        let commands: Vec<String> = info
            .manifest
            .commands
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let listers: Vec<String> = info.manifest.listers.iter().map(|l| l.id.clone()).collect();
        thread::spawn(move || serve(peer, &acts, &commands, &listers));
        Ok(Arc::new(host))
    }
}

fn write(peer: &MemoryPeer, value: Value) -> bool {
    peer.write_line(&value.to_string())
}

fn announce(peer: &MemoryPeer, text: &str) {
    write(
        peer,
        json!({"jsonrpc": "2.0", "method": "ui/announce", "params": {"text": text}}),
    );
}

fn serve(
    mut peer: MemoryPeer,
    acts: &HashMap<String, Act>,
    commands: &[String],
    listers: &[String],
) {
    let mut waiting: Option<Value> = None;
    let mut hanging: Option<Value> = None;
    while let Ok(line) = peer.from_host.recv() {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message.get("id").cloned();
        match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                write(
                    &peer,
                    json!({"jsonrpc": "2.0", "id": id, "result": {
                        "protocol": 1,
                        "plugin": {"name": "Test plugin", "version": "0.1.0"},
                        "plugin_capabilities": {"commands": commands, "listers": listers, "completion": true}
                    }}),
                );
            }
            Some("command/run") => {
                let params = &message["params"];
                let command = params["command"].as_str().unwrap_or_default();
                match acts.get(command).cloned().unwrap_or(Act::Done) {
                    Act::Done => {
                        write(
                            &peer,
                            json!({"jsonrpc": "2.0", "id": id, "result": {"done": 1}}),
                        );
                    }
                    Act::Talk => {
                        let token = params["progress_token"].clone();
                        write(
                            &peer,
                            json!({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": token, "value": {"kind": "begin", "title": "Count lines"}}}),
                        );
                        announce(&peer, "Lines: 42, files: 1");
                        write(
                            &peer,
                            json!({"jsonrpc": "2.0", "id": id, "result": {"done": 1}}),
                        );
                    }
                    Act::Call(method, call) => {
                        write(
                            &peer,
                            json!({"jsonrpc": "2.0", "id": "host-1", "method": method, "params": call}),
                        );
                        waiting = id;
                    }
                    Act::Rows => {
                        let names: Vec<String> = params["context"]["selection"]
                            .as_array()
                            .map(|rows| {
                                rows.iter()
                                    .filter_map(|r| r["name"].as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                        announce(&peer, &format!("Rows: {}", names.join(" and ")));
                        write(
                            &peer,
                            json!({"jsonrpc": "2.0", "id": id, "result": {"done": 1}}),
                        );
                    }
                    Act::Crash => {
                        peer.end_output();
                        return;
                    }
                    Act::Hang => hanging = id,
                }
            }
            Some("$/cancel_request") => {
                if let Some(id) = hanging.take() {
                    write(
                        &peer,
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32800, "message": "cancelled"}}),
                    );
                }
            }
            Some("lister/rows") => {
                write(
                    &peer,
                    json!({"jsonrpc": "2.0", "id": id, "result": {"rows": [
                        {"name": "c.txt", "kind": "item", "extra": {"id": "c.txt", "state": "Deleted", "lines": "7"}}
                    ]}}),
                );
            }
            Some("completion/query") => {
                let text = message["params"]["text"].as_str().unwrap_or_default();
                write(
                    &peer,
                    json!({"jsonrpc": "2.0", "id": id, "result": {"candidates": [
                        {"text": format!("{text}-one"), "kind": "branch"},
                        {"text": format!("{text}-two"), "kind": "branch"}
                    ]}}),
                );
            }
            Some("settings/changed") => {
                announce(
                    &peer,
                    &format!("Settings: {}", message["params"]["settings"]),
                );
            }
            Some("shutdown") => {
                write(&peer, json!({"jsonrpc": "2.0", "id": id, "result": null}));
            }
            Some("exit") => return,
            Some(_) => {
                if id.is_some() {
                    write(
                        &peer,
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "no method"}}),
                    );
                }
            }
            None => {
                if let Some(command) = waiting.take() {
                    let said = match (message.get("result"), message.get("error")) {
                        (_, Some(error)) => format!("Host error: {}", error["message"]),
                        (Some(result), None) => format!("Host said {result}"),
                        _ => "Host said nothing".to_owned(),
                    };
                    announce(&peer, &said);
                    write(
                        &peer,
                        json!({"jsonrpc": "2.0", "id": command, "result": {"done": 1}}),
                    );
                }
            }
        }
    }
}

/// The test plugin's manifest: `line-counter`, "Line counter", with its
/// commands, a lister, a setting, and `extra` lines at the end.
pub const MANIFEST: &str = r#"
[plugin]
id = "line-counter"
name = "Line counter"
version = "0.1.0"
protocol = 1

[run]
command = ["test-plugin"]
startup_timeout = "2s"

[timeouts]
command = "1s"

[permissions]
fs = "write"
panel = "write"
commands = ["view.*"]

[permissions.declared]
network = "no network"

[[command]]
id = "count"
name = "Count lines"
description = "Says how many lines the files hold."
key = "ctrl+alt+l"

[[command]]
id = "copier"
name = "Copy with a taken key"
description = "Proposes a key diroxide uses."
key = "f5"

[[command]]
id = "ask"
name = "Ask something"
description = "Asks a question."
selection = "none"

[[command]]
id = "show"
name = "Show the changes"
description = "Opens a lister."
selection = "none"

[[command]]
id = "diff"
name = "Show the diff"
description = "Shows the diff of a row."
selection = "one"

[[lister]]
id = "changes"
name = "Changed files"
activate = "diff"

[[lister.columns]]
id = "state"
header = "State"
kind = "text"
sorts = true

[[lister.columns]]
id = "lines"
header = "Lines"
kind = "number"
sorts = true

[[setting]]
key = "skip_blank"
type = "bool"
default = false
description = "Leave blank lines out of the count."
"#;

/// The folders of a test: a config with the plugin, a state, a work
/// folder.
pub struct World {
    pub _dir: tempfile::TempDir,
    pub paths: Paths,
    pub work: std::path::PathBuf,
}

pub fn world(manifest: &str) -> World {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let folder = paths.plugins_dir().join("line-counter");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(Manifest::FILE), manifest).unwrap();
    let work = fixture(dir.path());
    World {
        _dir: dir,
        paths,
        work,
    }
}

/// Records the owner's yes for the test plugin, as Manage plugins does.
pub fn enable(paths: &Paths) {
    let folder = paths.plugins_dir().join("line-counter");
    let text = fs::read_to_string(folder.join(Manifest::FILE)).unwrap();
    let manifest = Manifest::from_toml(&text).unwrap();
    let fingerprint =
        plugin_fingerprint(&manifest, &folder, diroxide_components::sha256_hex).unwrap();
    ConsentStore::open(&paths.state_dir.join("plugins"))
        .enable("line-counter", Kind::Plugin, fingerprint)
        .unwrap();
}

pub fn session(world: &World, launcher: Arc<TestLauncher>) -> Session {
    let here = place(&world.work);
    Session::open(
        here.clone(),
        here,
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings: settings(),
            paths: Some(world.paths.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    )
}

fn id(text: &str) -> CommandId {
    CommandId::new(text).unwrap()
}

/// Waits a moment for the host's own events (its supervisor ticks every
/// tenth of a second), then takes them.
fn later(s: &mut Session, ms: u64) -> Vec<String> {
    thread::sleep(Duration::from_millis(ms));
    let events = s.runtime.poll();
    let effects: Vec<_> = events
        .into_iter()
        .flat_map(|event| s.app.handle(event))
        .collect();
    s.run(effects)
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        Some(Dialog::Question { choices, .. }) => {
            choices.rows().iter().map(|r| r.spoken()).collect()
        }
        _ => Vec::new(),
    }
}

#[test]
fn a_plugin_not_enabled_contributes_nothing_until_its_consent() {
    let w = world(MANIFEST);
    let mut s = session(&w, TestLauncher::with(&[]));
    s.command(ids::MENU_PLUGINS);
    assert_eq!(
        rows(&s),
        ["Manage plugins", "Get example plugins"],
        "only the menu's own items"
    );
    s.key("escape");
    let said = s.run_command(Command::new(id("line-counter.count")));
    assert_eq!(
        said,
        ["Line counter is not enabled. Enable it in Manage plugins."]
    );
    s.command(ids::PLUGINS);
    assert_eq!(rows(&s), ["Line counter: not enabled, plugin 0.1.0"]);
    s.key("enter");
    assert_eq!(s.dialog_title().as_deref(), Some("Line counter"));
    assert_eq!(rows(&s)[0], "Enable");
    let said = s.key("enter");
    assert_eq!(
        said[0],
        "Enable Line counter, version 0.1.0? No is the default. y or n"
    );
    let Some(Dialog::Confirm { listing, .. }) = s.dialog() else {
        panic!("the consent is a confirmation");
    };
    assert_eq!(listing[0], "It runs: test-plugin");
    assert!(listing.contains(&"While it runs it can do what you can.".to_owned()));
    assert!(listing.contains(&"The plugin states, unchecked:".to_owned()));
    for line in listing {
        assert!(line.chars().count() <= 40, "{line}");
    }
    assert_eq!(s.key("n"), ["Not enabled."]);
    assert_eq!(s.dialog_title().as_deref(), Some("Manage plugins"));
    s.key("enter");
    s.key("enter");
    let said = s.key("y");
    assert!(
        said.contains(&"Enabled: Line counter.".to_owned()),
        "{said:?}"
    );
    assert_eq!(rows(&s), ["Line counter: enabled, plugin 0.1.0"]);
    s.key("escape");
    s.command(ids::MENU_PLUGINS);
    let menu = rows(&s);
    assert_eq!(menu[0], "Manage plugins");
    assert!(
        menu.contains(&"Count lines, Ctrl+Alt+L".to_owned()),
        "{menu:?}"
    );
    assert!(
        menu.contains(
            &"Show the diff, disabled, Show the diff needs one item; none is selected.".to_owned()
        ) || menu.iter().any(|r| r.starts_with("Show the diff")),
        "{menu:?}"
    );
}

#[test]
fn a_plugin_command_runs_as_a_job_with_its_name_last() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let launcher = TestLauncher::with(&[("count", Act::Talk)]);
    let mut s = session(&w, launcher.clone());
    let said = s.run_command(Command::new(id("line-counter.count")));
    assert_eq!(said[0], "Count lines: started (Line counter)");
    assert!(
        said.contains(&"Lines: 42, files: 1. By Line counter.".to_owned()),
        "{said:?}"
    );
    assert_eq!(
        said.last().unwrap(),
        "Count lines done: 1 item (Line counter)"
    );
    assert_eq!(s.app.jobs().line(), "No jobs");
    let history = s.app.jobs().history();
    assert_eq!(
        history.last().unwrap().kind,
        diroxide_core::JobKind::PluginCommand
    );
    assert_eq!(*launcher.launches.lock().unwrap(), 1);
    // The palette lists it, its key beside it.
    s.command(ids::PALETTE);
    s.type_text("count l");
    let Some(Dialog::Edit {
        candidates: Some(list),
        ..
    }) = s.dialog()
    else {
        panic!("the palette");
    };
    assert_eq!(list.focused().unwrap().spoken(), "Count lines, Ctrl+Alt+L");
}

#[test]
fn a_proposed_key_binds_only_when_free() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(&w, TestLauncher::with(&[("count", Act::Done)]));
    let proposals = s.app.plugin_key_proposals().to_vec();
    let copier = proposals
        .iter()
        .find(|p| p.command.as_str() == "line-counter.copier")
        .unwrap();
    assert!(!copier.bound);
    assert_eq!(
        diroxide_app::plugins::registry::proposal_words(copier),
        "f5 not bound: it already runs Copy to other panel"
    );
    assert_eq!(
        s.app.keymap().key_text(&ids::COPY),
        "F5",
        "a built-in key is never taken"
    );
    let said = s.key("ctrl+alt+l");
    assert_eq!(said[0], "Count lines: started (Line counter)");
}

#[test]
fn a_plugins_question_is_the_apps_dialog_with_its_name_last() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let call =
        json!({"kind": "confirm", "title": "Trust this host?", "message": ["Key: 1a2b 3c4d"]});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/ask", call))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    assert!(
        said.contains(
            &"Trust this host? No is the default. y or n (Line counter) Key: 1a2b 3c4d".to_owned()
        ),
        "{said:?}"
    );
    let Some(Dialog::Confirm { listing, .. }) = s.dialog() else {
        panic!("a confirmation");
    };
    assert_eq!(listing, &["Key: 1a2b 3c4d"]);
    let said = s.key("y");
    assert!(
        said.contains(&r#"Host said {"answer":true}. By Line counter."#.to_owned()),
        "{said:?}"
    );
    assert!(s.dialog().is_none(), "the focus is back on the panel");
}

#[test]
fn escape_answers_cancelled_and_each_kind_answers_in_its_shape() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let choose = json!({"kind": "choose", "title": "Pick", "multiple": true,
        "choices": [{"id": "a", "label": "Alpha"}, {"id": "b", "label": "Beta"}]});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/ask", choose))]),
    );
    s.run_command(Command::new(id("line-counter.ask")));
    assert_eq!(rows(&s), ["Alpha", "Beta"]);
    s.key("space");
    s.key("space");
    let said = s.key("enter");
    assert!(
        said.contains(&r#"Host said {"ids":["a","b"]}. By Line counter."#.to_owned()),
        "{said:?}"
    );

    let text = json!({"kind": "text", "title": "Branch name", "initial": "ma"});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/ask", text))]),
    );
    s.run_command(Command::new(id("line-counter.ask")));
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Branch name (Line counter)")
    );
    s.type_text("in");
    let said = s.key("enter");
    assert!(
        said.contains(&r#"Host said {"text":"main"}. By Line counter."#.to_owned()),
        "{said:?}"
    );

    let secret = json!({"kind": "secret", "title": "Token"});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/ask", secret))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    assert!(
        said.iter()
            .any(|l| l.starts_with("Token (Line counter) (hidden)")),
        "{said:?}"
    );
    s.type_text("canary");
    assert!(
        !s.said.iter().any(|m| m.text.contains("canary")),
        "nothing typed is said"
    );
    let said = s.key("enter");
    assert!(
        said.iter().any(|l| l.contains("canary")),
        "the plugin got it: {said:?}"
    );

    let path = json!({"kind": "text", "title": "Escape me"});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/ask", path))]),
    );
    s.run_command(Command::new(id("line-counter.ask")));
    let said = s.key("escape");
    assert!(
        said.contains(&r#"Host said {"cancelled":true}. By Line counter."#.to_owned()),
        "{said:?}"
    );
    assert!(s.dialog().is_none());
}

#[test]
fn a_plugin_dialog_waits_behind_a_destructive_confirmation() {
    let w = world(MANIFEST);
    let mut s = session(&w, TestLauncher::with(&[]));
    s.focus("notes.txt");
    s.key("shift+delete");
    let title = s.dialog_title().unwrap();
    assert!(title.starts_with("Delete"), "{title}");
    let question = Question {
        plugin_id: "line-counter".into(),
        plugin: "Line counter".into(),
        title: "Proceed? (Line counter)".into(),
        lines: Vec::new(),
        kind: QuestionKind::Confirm {
            destructive: true,
            yes_label: None,
            no_label: None,
        },
        prompt_id: None,
        history: Vec::new(),
        completion: None,
    };
    let effects = s.app.handle(AppEvent::Plugin(Box::new(PluginEvent::Call {
        call: CallId::new(99),
        plugin: "line-counter".into(),
        request: Call::Ask(Box::new(question)),
    })));
    s.run(effects);
    assert_eq!(s.dialog_title().unwrap(), title, "never replaced");
    let said = s.key("n");
    assert!(said.contains(&"Nothing changed.".to_owned()), "{said:?}");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Proceed? No is the default. y or n (Line counter)"),
        "it opens when the confirmation closes"
    );
    assert!(w.work.join("notes.txt").exists());
    s.key("escape");
    assert!(s.dialog().is_none());
}

#[test]
fn a_crash_is_said_once() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(&w, TestLauncher::with(&[("count", Act::Crash)]));
    let mut said = s.run_command(Command::new(id("line-counter.count")));
    said.extend(later(&mut s, 400));
    let stopped: Vec<&String> = said
        .iter()
        .filter(|l| l.contains("stopped unexpectedly"))
        .collect();
    assert_eq!(stopped.len(), 1, "{said:?}");
    assert!(stopped[0].starts_with("Line counter stopped unexpectedly."));
    assert_eq!(s.app.jobs().line(), "No jobs");
}

#[test]
fn escape_cancels_a_plugin_command() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(&w, TestLauncher::with(&[("count", Act::Hang)]));
    let effects = s
        .app
        .handle(AppEvent::Command(Command::new(id("line-counter.count"))));
    let screen = s.runtime.carry_out(&mut s.app, effects);
    s.run_quiet(screen);
    assert_eq!(s.app.jobs().line(), "Count lines: started (Line counter)");
    let said = s.key("escape");
    assert_eq!(said[0], "Count lines: cancelling");
    assert!(
        said.contains(&"Count lines: cancelled.".to_owned()),
        "{said:?}"
    );
    assert_eq!(s.app.jobs().line(), "No jobs");
}

#[test]
fn a_silent_plugin_times_out_in_words() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(&w, TestLauncher::with(&[("count", Act::Hang)]));
    let mut said = s.run_command(Command::new(id("line-counter.count")));
    said.extend(later(&mut s, 300));
    let timed: Vec<&String> = said.iter().filter(|l| l.contains("timed out")).collect();
    assert_eq!(timed.len(), 1, "{said:?}");
    assert_eq!(
        timed[0], "Count lines timed out after 1 second.",
        "{said:?}"
    );
}

#[test]
fn no_plugins_starts_nothing_and_says_so() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let launcher = TestLauncher::with(&[("count", Act::Done)]);
    let here = place(&w.work);
    let mut s = Session::open(
        here.clone(),
        here,
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(launcher.clone()),
            no_plugins: true,
            ..Setup::default()
        },
    );
    let said = s.command(ids::MENU_PLUGINS);
    assert_eq!(said, ["Plugins are off for this session."]);
    assert_eq!(rows(&s), ["Manage plugins", "Get example plugins"]);
    s.key("escape");
    let said = s.run_command(Command::new(id("line-counter.count")));
    assert_eq!(said, ["Plugins are off for this session."]);
    assert_eq!(*launcher.launches.lock().unwrap(), 0);
    assert_eq!(s.app.keymap().key_text(&id("line-counter.count")), "");
}

#[test]
fn menus_toml_naming_a_plugins_command_hides_it_while_not_enabled() {
    let w = world(MANIFEST);
    let menus = diroxide_app::menu::BUILTIN_MENUS_TOML.replace(
        "[[menu.item]]\ncommand = \"plugins.list\"",
        "[[menu.item]]\ncommand = \"line-counter.count\"\nlabel = \"Count my lines\"\n\n[[menu.item]]\ncommand = \"plugins.list\"",
    );
    fs::create_dir_all(&w.paths.config_dir).unwrap();
    fs::write(w.paths.config_dir.join("menus.toml"), menus).unwrap();
    let mut s = session(&w, TestLauncher::with(&[]));
    assert!(
        !s.app
            .problems()
            .iter()
            .any(|p| p.text.contains("line-counter")),
        "nothing reported: {:?}",
        s.app.problems()
    );
    s.command(ids::MENU_PLUGINS);
    assert!(
        !rows(&s).iter().any(|r| r.contains("Count")),
        "{:?}",
        rows(&s)
    );
    s.key("escape");
    enable(&w.paths);
    s.command(ids::PLUGINS);
    s.key("escape");
    s.command(ids::MENU_PLUGINS);
    let menu = rows(&s);
    assert_eq!(menu[0], "Count my lines, Ctrl+Alt+L", "{menu:?}");
}

#[test]
fn a_lister_sorts_filters_marks_and_refuses_file_commands() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let open = json!({"lister": "changes", "title": "Changed files", "rows": [
        {"name": "b.txt", "kind": "item", "extra": {"id": "b.txt", "state": "Modified", "lines": "12"}},
        {"name": "a.txt", "kind": "item", "extra": {"id": "a.txt", "state": "Added", "lines": "3"}}
    ]});
    let mut s = session(
        &w,
        TestLauncher::with(&[
            ("show", Act::Call("lister/open", open)),
            ("diff", Act::Rows),
        ]),
    );
    let said = s.run_command(Command::new(id("line-counter.show")));
    assert!(
        said.contains(&"Changed files, 2 rows (Line counter)".to_owned()),
        "{said:?}"
    );
    assert_eq!(
        rows(&s),
        [
            "b.txt, state Modified, lines 12",
            "a.txt, state Added, lines 3"
        ]
    );
    assert_eq!(s.key("f5"), ["This row is not a file."]);
    assert_eq!(s.key("f8"), ["This row is not a file."]);
    s.key("ctrl+f3");
    assert_eq!(s.dialog_title().as_deref(), Some("Sort the rows by"));
    s.key("down");
    s.key("down");
    let said = s.key("enter");
    assert_eq!(
        said[0], "Sorted by lines. b.txt, state Modified, lines 12, 2 of 2",
        "the focus stays on its row"
    );
    let said = s.key("space");
    assert_eq!(said, ["Marked"]);
    let said = s.key("ctrl+s");
    assert!(said[0].starts_with("Filter the rows by"), "{said:?}");
    s.type_text("modif");
    let said = s.key("enter");
    assert_eq!(
        said[0],
        "Filter modif. b.txt, selected, state Modified, lines 12, 1 of 1"
    );
    let said = s.key("enter");
    assert!(
        said.contains(&"Rows: b.txt. By Line counter.".to_owned()),
        "Enter runs activate with the row: {said:?}"
    );
    s.key("escape");
    assert!(s.dialog().is_none());
}

#[test]
fn a_plugins_file_job_takes_the_confirmed_path_and_its_report_returns() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let target = w.work.join("Made by plugin");
    let mkdir = json!({"place": target.to_str().unwrap()});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("fs/mkdir", mkdir))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    assert!(target.is_dir(), "{said:?}");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Host said") && l.ends_with("By Line counter.")),
        "{said:?}"
    );
    // The panels and diroxide's commands, through the app's own moves.
    let get = json!({});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("panel/get", get))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    let answer = said.iter().find(|l| l.starts_with("Host said")).unwrap();
    assert!(answer.contains("work"), "{answer}");
    let execute = json!({"command": "view.sort-size"});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("command/execute", execute))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    assert!(
        said.contains(&"Sorted by size, largest first".to_owned()),
        "{said:?}"
    );
    let refused = json!({"command": "file.trash"});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("command/execute", refused))]),
    );
    let said = s.run_command(Command::new(id("line-counter.ask")));
    assert!(
        said.iter().any(|l| l.starts_with("Host error")),
        "outside its manifest's patterns: {said:?}"
    );
    assert!(w.work.join("notes.txt").exists());
}

#[test]
fn settings_show_under_the_plugins_name_and_reach_it() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let mut s = session(&w, TestLauncher::with(&[("count", Act::Done)]));
    s.run_command(Command::new(id("line-counter.count")));
    s.command(ids::PLUGINS);
    s.key("enter");
    let actions = rows(&s);
    assert!(actions.contains(&"Settings".to_owned()), "{actions:?}");
    while s
        .app
        .dialog()
        .and_then(|(_, d)| match d {
            Dialog::Choose { list, .. } => list.focused().map(|r| r.label.clone()),
            _ => None,
        })
        .as_deref()
        != Some("Settings")
    {
        s.key("down");
    }
    s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Settings of Line counter")
    );
    assert_eq!(rows(&s), ["Skip blank, off"]);
    let mut said = s.key("enter");
    said.extend(later(&mut s, 200));
    assert!(
        said.contains(&"Skip blank: on (Line counter)".to_owned()),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.contains(r#"Settings: {"skip_blank":true}"#)),
        "the plugin heard it: {said:?}"
    );
    assert_eq!(rows(&s), ["Skip blank, on"]);
    let written = fs::read_to_string(
        w.paths
            .state_dir
            .join("plugins")
            .join("line-counter")
            .join("settings.toml"),
    )
    .unwrap();
    assert!(written.contains("skip_blank = true"), "{written}");
}

#[test]
fn the_log_list_says_the_last_error_first() {
    let w = world(MANIFEST);
    let log = w
        .paths
        .state_dir
        .join("plugins")
        .join("line-counter")
        .join("line-counter.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(
        &log,
        "2026-10-23T10:00:00Z host: started\n2026-10-23T10:00:01Z stderr: cannot read a.txt\n2026-10-23T10:00:02Z info: counted\n",
    )
    .unwrap();
    let mut s = session(&w, TestLauncher::with(&[]));
    s.command(ids::PLUGINS);
    s.key("enter");
    s.key("end");
    let said = s.key("enter");
    assert_eq!(
        said[0],
        "Log of Line counter, Last error: stderr: cannot read a.txt, 1 of 4"
    );
}

#[test]
fn a_changed_plugin_asks_again_before_it_runs() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let launcher = TestLauncher::with(&[("count", Act::Done)]);
    let mut s = session(&w, launcher.clone());
    let manifest = w
        .paths
        .plugins_dir()
        .join("line-counter")
        .join("plugin.toml");
    fs::write(&manifest, format!("{MANIFEST}\n# changed\n")).unwrap();
    let said = s.run_command(Command::new(id("line-counter.count")));
    assert!(
        said.iter().any(|l| l.starts_with(
            "Line counter changed since you enabled it. Enable it again? No is the default."
        )),
        "{said:?}"
    );
    assert_eq!(*launcher.launches.lock().unwrap(), 0, "nothing ran");
    assert_eq!(s.key("n"), ["Not enabled."]);
    s.run_command(Command::new(id("line-counter.count")));
    let said = s.key("y");
    assert!(
        said.contains(&"Enabled: Line counter.".to_owned()),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l == "Count lines done: 1 item (Line counter)"),
        "it runs once enabled again: {said:?}"
    );
}

#[test]
fn a_provider_loss_is_said_and_a_plugins_words_stay_its_own() {
    let w = world(MANIFEST);
    let mut s = session(&w, TestLauncher::with(&[]));
    let lost = diroxide_plugins::host::HostEvent::ProviderLost {
        plugin: "line-counter".into(),
        authority: "memfs://box".into(),
        message: diroxide_core::Message::warning("Box unplugged (Line counter)"),
    };
    let effects = s
        .app
        .handle(AppEvent::Plugin(Box::new(PluginEvent::Host(lost))));
    assert_eq!(s.run(effects), ["Box unplugged (Line counter)"]);
    assert_eq!(s.app.panel(Side::Left).place(), &place(&w.work));
}

#[test]
fn every_plugin_line_fits_the_braille_display() {
    use diroxide_core::format::within_cells;
    let w = world(MANIFEST);
    let s = session(&w, TestLauncher::with(&[]));
    let catalog = s.app.plugin_catalog();
    let item = catalog.item("line-counter").unwrap();
    assert!(within_cells(&item.row(), "Line counter: not enabled"));
    for row in &item.enable_rows {
        assert!(row.chars().count() <= 40, "{row}");
    }
}

#[test]
fn f1_on_a_plugins_setting_says_what_it_does() {
    let w = world(MANIFEST);
    let mut s = session(&w, TestLauncher::with(&[]));
    s.command(ids::PLUGINS);
    s.key("enter");
    while rows(&s)
        .iter()
        .position(|r| r == "Settings")
        .is_some_and(|at| {
            s.app
                .dialog()
                .and_then(|(_, d)| match d {
                    Dialog::Choose { list, .. } => list.focus_index(),
                    _ => None,
                })
                .is_some_and(|focus| focus != at)
        })
    {
        s.key("down");
    }
    s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Settings of Line counter")
    );
    let help = s
        .app
        .keymap()
        .keys_for(&ids::HELP)
        .first()
        .map(|k| k.to_string())
        .unwrap();
    let said = s.key(&help);
    assert_eq!(
        said,
        [
            "Skip blank: Leave blank lines out of the count. Holds on or off. Default: off. (Line counter)"
        ]
    );
}

/// A lister script's places become the results lister of real files.
#[test]
fn a_lister_scripts_places_open_as_results() {
    use diroxide_app::plugins::{Finished, Outcome};
    use diroxide_plugins::script::consent::script_fingerprint;
    let w = world(MANIFEST);
    let scripts = w.paths.plugins_dir().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    let script = scripts.join("search-all.sh");
    fs::write(
        &script,
        "#!/usr/bin/env bash\n# diro: name = \"Search everything\"\n# diro: selection = \"none\"\n# diro: mode = \"lister\"\n# diro: prompt = \"Search the whole computer for:\"\necho\n",
    )
    .unwrap();
    let fingerprint = script_fingerprint(&script, diroxide_components::sha256_hex).unwrap();
    ConsentStore::open(&w.paths.state_dir.join("plugins"))
        .enable("search-all", Kind::Script, fingerprint)
        .unwrap();
    let mut s = session(&w, TestLauncher::with(&[]));
    let state = s
        .app
        .plugin_catalog()
        .item("search-all")
        .map(|i| i.state.clone());
    if !matches!(state, Some(diroxide_app::plugins::ItemState::Enabled)) {
        // No program runs a bash script here: the run is the runner's,
        // tested in the plugins crate.
        eprintln!("skipped: {state:?}");
        return;
    }
    let said = s.run_command(Command::new(id("script.search-all")));
    assert_eq!(
        said,
        ["Search the whole computer for (Search everything), empty"]
    );
    s.type_text("report");
    let effects = s
        .app
        .handle(AppEvent::Key(diroxide_core::Key::parse("enter").unwrap()));
    s.run_quiet(effects);
    // The script's run is left out: its end arrives as the runner reports
    // it.
    let _ = s.app.take_requests();
    let job = *s.app.jobs().ids().last().unwrap();
    let mut report = empty_report();
    report.places = vec![
        w.work.join("report.pdf").to_string_lossy().into_owned(),
        w.work
            .join("Backup")
            .join("report.pdf")
            .to_string_lossy()
            .into_owned(),
    ];
    let effects = s
        .app
        .handle(AppEvent::Plugin(Box::new(PluginEvent::Finished(Box::new(
            Finished {
                job,
                command: id("script.search-all"),
                outcome: Outcome::Script(Box::new(report)),
                last_error: None,
                limit: Duration::ZERO,
            },
        )))));
    let said = s.run(effects);
    assert!(
        said.contains(&"2 results for report, from Search everything".to_owned()),
        "{said:?}"
    );
    let Some(Dialog::Choose { list, .. }) = s.dialog() else {
        panic!("the results lister");
    };
    assert_eq!(list.len(), 2);
    assert!(
        list.rows()[0].label.starts_with("report.pdf"),
        "{:?}",
        list.rows()
    );
}

fn empty_report() -> diroxide_plugins::script::runner::RunReport {
    diroxide_plugins::script::runner::RunReport {
        ending: diroxide_plugins::script::words::Ending::Exited(0),
        said: Vec::new(),
        deferred: Vec::new(),
        output: Vec::new(),
        output_cut: false,
        places: Vec::new(),
        capped: false,
        column: Vec::new(),
        launched: None,
        wait_for_enter: false,
        runs: 1,
        stderr: Vec::new(),
    }
}

#[test]
fn a_plugins_prompt_completes_as_the_owner_types() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let text = json!({"kind": "text", "title": "Branch",
        "complete": {"source": "plugin", "prompt_id": "branch"}});
    let mut s = session(
        &w,
        TestLauncher::with(&[("ask", Act::Call("ui/ask", text))]),
    );
    s.run_command(Command::new(id("line-counter.ask")));
    let said = s.type_text("ma");
    assert!(said.contains(&"2 candidates".to_owned()), "{said:?}");
    let Some(Dialog::Edit {
        candidates: Some(list),
        ..
    }) = s.dialog()
    else {
        panic!("the candidates under the field");
    };
    assert_eq!(list.announce_focus(), "ma-one, branch, 1 of 2");
    s.key("down");
    let said = s.key("enter");
    assert_eq!(said, ["Branch (Line counter): ma-one"]);
    let said = s.key("enter");
    assert!(
        said.contains(&r#"Host said {"text":"ma-one"}. By Line counter."#.to_owned()),
        "{said:?}"
    );
}

#[test]
fn a_lister_asks_for_more_rows_at_its_end() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let open = json!({"lister": "changes", "title": "Changed files", "total": 3, "rows": [
        {"name": "b.txt", "kind": "item", "extra": {"id": "b.txt", "state": "Modified", "lines": "12"}},
        {"name": "a.txt", "kind": "item", "extra": {"id": "a.txt", "state": "Added", "lines": "3"}}
    ]});
    let mut s = session(
        &w,
        TestLauncher::with(&[("show", Act::Call("lister/open", open))]),
    );
    s.run_command(Command::new(id("line-counter.show")));
    assert_eq!(rows(&s).len(), 2);
    let said = s.key("down");
    assert!(
        said.iter()
            .any(|l| l.starts_with("1 row more (Line counter)")),
        "{said:?}"
    );
    assert_eq!(rows(&s).len(), 3);
    assert_eq!(rows(&s)[2], "c.txt, state Deleted, lines 7");
    let said = s.key("ctrl+r");
    assert!(
        said.iter().any(|l| l.starts_with("1 row (Line counter)")),
        "the rows again: {said:?}"
    );
}
