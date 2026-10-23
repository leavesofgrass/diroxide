//! `diro serve --stdio`: the app core over JSON-RPC 2.0, one JSON object a
//! line, the same protocol the plugins speak, with the connecting program
//! in the plugin's seat (research/plugin-protocol.md, sections 3.1 and
//! 3.6; R22 section 2.5 and commit 42).
//!
//! The session is the plugins' own ([`Lifecycle`]): the connecting program
//! sends `initialize` first, with the protocol's major version (1); before
//! it is answered every other request gets -32002 and notifications are
//! dropped; another protocol is refused in words and the session waits
//! for one it can take; after `shutdown` is answered only `exit` counts,
//! and a request gets -32600. `exit`, or the end of the input, ends the
//! server. The same fixture dialogues that check a plugin's answering side
//! (`fixtures/plugins/dialogues/`) check this one.
//!
//! The services a client calls:
//!
//! - `command/execute`: a command by id, with `args`, or in the command
//!   text form as `text` ("file.copy dest=D:\backup"); the result is what
//!   was said and the dialog left open, if any.
//! - `key/press`: one key by name ("ctrl+q", "y"), as a frontend sends it.
//! - `panel/get` and `panel/set`: a panel's place, cursor, selection, sort
//!   and filter; set changes the place, the cursor and the marks.
//! - `dialog/get`: the open dialog's name and focus.
//! - `ui/announce`: says a line through diroxide, cleaned, with the
//!   client's name last (it comes back as the announcement event).
//! - `shutdown`: answered with null.
//!
//! The events, each a notification sent before the answer to the request
//! that caused it, so a self-voicing or Braille helper can follow along
//! (R2):
//!
//! - `ui/announce`: every announcement, with its importance (`status`,
//!   `info`, `warning`, `error`);
//! - `panel/changed`: a panel whose folder, focus or selection changed,
//!   with `changes` naming which, then its place, its cursor and its
//!   selection.

use std::io::{BufRead, Write};

use diroxide_app::commands::parse_command_text;
use diroxide_app::{App, AppEvent, Runtime};
use diroxide_config::Paths;
use diroxide_core::{
    ArgValue, Command, CommandId, Effect, Importance, Key, Message, MessageKind, Place, Side,
};
use diroxide_plugins::protocol::{
    self, ErrorKind, Id, Incoming, Notification, Request, Response, RpcError, methods,
};
use diroxide_plugins::services::{attribute, clean_text};
use diroxide_plugins::session::{Gate, Lifecycle};
use serde_json::{Map, Value, json};

use crate::CliError;
use crate::open::{OpenOptions, build, current_place, paths};

/// The methods the server answers.
pub const METHODS: [&str; 8] = [
    methods::COMMAND_EXECUTE,
    "key/press",
    methods::PANEL_GET,
    methods::PANEL_SET,
    "dialog/get",
    methods::UI_ANNOUNCE,
    methods::INITIALIZE,
    methods::SHUTDOWN,
];

/// The notification of a panel's change.
pub const PANEL_CHANGED: &str = "panel/changed";

/// The notifications the server sends.
pub const EVENTS: [&str; 2] = [methods::UI_ANNOUNCE, PANEL_CHANGED];

/// Argument names whose values are places.
const PLACE_ARGUMENTS: [&str; 6] = ["place", "dest", "to", "from", "source", "target"];

/// The longest line a client may announce, in characters.
const MAX_CLIENT_ANNOUNCE: usize = 300;

fn importance(message: &Message) -> &'static str {
    match (message.importance, message.kind) {
        (Importance::Low, _) => "status",
        (Importance::Normal, _) => "info",
        (Importance::High, MessageKind::Error) => "error",
        (Importance::High, _) => "warning",
    }
}

/// Serves until `exit` or the end of the input, with the platform's
/// config and state folders. Returns the exit code.
pub fn run(input: &mut dyn BufRead, out: &mut dyn Write) -> Result<i32, CliError> {
    let here = current_place()?;
    run_with(input, out, paths(false).as_ref(), &here)
}

/// What a panel showed: for the change events.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PanelState {
    place: String,
    cursor: Option<String>,
    selection: Vec<String>,
    active: bool,
}

fn panel_state(app: &App, side: Side) -> PanelState {
    let panel = app.panel(side);
    let list = panel.list();
    PanelState {
        place: panel.place().to_string(),
        cursor: list.focused().map(|r| r.id.clone()),
        selection: list.selected_ids().into_iter().map(str::to_owned).collect(),
        active: app.active_side() == side,
    }
}

struct Server<'a> {
    out: &'a mut dyn Write,
    paths: Option<&'a Paths>,
    here: &'a Place,
    app: Option<(App, Runtime)>,
    life: Lifecycle,
    /// The client's name, from its `initialize`, last in what it says.
    client: String,
    /// Whether `shutdown` was answered, for the exit code.
    shut_down: bool,
}

impl Server<'_> {
    fn send<T: serde::Serialize>(&mut self, message: &T) -> Result<(), CliError> {
        let failed = |e: std::io::Error| CliError::Failed(e.to_string());
        let text = protocol::to_line(message).map_err(|e| CliError::Failed(e.to_string()))?;
        writeln!(self.out, "{text}").map_err(failed)?;
        self.out.flush().map_err(failed)
    }

    fn panels(&self) -> Option<[PanelState; 2]> {
        let (app, _) = self.app.as_ref()?;
        Some([panel_state(app, Side::Left), panel_state(app, Side::Right)])
    }

    /// Carries effects out, sends each announcement as a notification,
    /// then each panel that changed, and returns what was said.
    fn carry(&mut self, effects: Vec<Effect>) -> Result<Vec<String>, CliError> {
        let before = self.panels();
        let said = self.carry_only(effects)?;
        self.notify_changes(before)?;
        Ok(said)
    }

    /// Carries effects out and sends each announcement as a notification.
    fn carry_only(&mut self, effects: Vec<Effect>) -> Result<Vec<String>, CliError> {
        let Some((app, runtime)) = self.app.as_mut() else {
            return Ok(Vec::new());
        };
        let screen = runtime.run_until_idle(app, effects);
        let mut said = Vec::new();
        for effect in screen {
            if let Effect::Announce(message) = effect {
                let mut params = Map::new();
                params.insert("text".into(), json!(message.text));
                params.insert("importance".into(), json!(importance(&message)));
                self.send(&Notification::new(methods::UI_ANNOUNCE, params))?;
                said.push(message.text);
            }
        }
        Ok(said)
    }

    /// Sends `panel/changed` for each panel whose folder, focus or
    /// selection differs from `before`.
    fn notify_changes(&mut self, before: Option<[PanelState; 2]>) -> Result<(), CliError> {
        if let (Some(before), Some(after)) = (before, self.panels()) {
            for (side, (old, new)) in Side::BOTH.into_iter().zip(before.iter().zip(after.iter())) {
                let mut changes = Vec::new();
                if old.place != new.place {
                    changes.push("folder");
                }
                if old.cursor != new.cursor || (new.active && !old.active) {
                    changes.push("focus");
                }
                if old.selection != new.selection {
                    changes.push("selection");
                }
                if changes.is_empty() {
                    continue;
                }
                let mut params = Map::new();
                params.insert("panel".into(), json!(side.word()));
                params.insert("active".into(), json!(new.active));
                params.insert("changes".into(), json!(changes));
                params.insert("place".into(), json!(new.place));
                params.insert("cursor".into(), json!(new.cursor));
                params.insert("selection".into(), json!(new.selection));
                self.send(&Notification::new(PANEL_CHANGED, params))?;
            }
        }
        Ok(())
    }

    fn outcome(&self, said: Vec<String>) -> Value {
        let dialog = self
            .app
            .as_ref()
            .and_then(|(app, _)| app.dialog().map(|(_, d)| d.title()));
        json!({ "said": said, "dialog": dialog, "quit": self.app.as_ref().is_some_and(|(app, _)| app.wants_quit()) })
    }

    fn initialize(&mut self, params: &Map<String, Value>) -> Result<Value, RpcError> {
        let asked = self.life.initialize(params)?;
        if !asked.host.name.trim().is_empty() {
            self.client = clean_text(&asked.host.name, 40);
        }
        let places: Vec<String> = params
            .get("places")
            .and_then(Value::as_array)
            .map(|places| {
                places
                    .iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let options = OpenOptions {
            places,
            ..OpenOptions::default()
        };
        let built = build(&options, self.paths, self.here)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()));
        let (mut app, runtime) = match built {
            Ok(built) => built,
            Err(error) => {
                // The session waits for an initialize it can take.
                self.life = Lifecycle::new();
                return Err(error);
            }
        };
        app.set_screen_reader(true);
        let start = app.start();
        self.app = Some((app, runtime));
        self.carry(start)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        let version = env!("CARGO_PKG_VERSION");
        Ok(json!({
            "protocol": protocol::PROTOCOL,
            "name": "diroxide",
            "version": version,
            "plugin": { "name": "diroxide", "version": version },
            "server_capabilities": { "methods": METHODS, "events": EVENTS },
        }))
    }

    fn side(&self, params: &Map<String, Value>) -> Result<Side, RpcError> {
        let Some((app, _)) = self.app.as_ref() else {
            return Err(RpcError::new(ErrorKind::NotInitialized, "not initialized"));
        };
        Ok(
            match params
                .get("panel")
                .and_then(Value::as_str)
                .unwrap_or("active")
            {
                "active" => app.active_side(),
                "other" => app.active_side().other(),
                "left" => Side::Left,
                "right" => Side::Right,
                other => {
                    return Err(RpcError::new(
                        ErrorKind::InvalidParams,
                        format!(
                            "there is no panel named {other}; use active, other, left or right"
                        ),
                    ));
                }
            },
        )
    }

    fn command(&mut self, params: &Map<String, Value>) -> Result<Value, RpcError> {
        let command = if let Some(text) = params.get("text").and_then(Value::as_str) {
            parse_command_text(text).map_err(|e| RpcError::new(ErrorKind::InvalidParams, e))?
        } else {
            let id = params
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "command is missing"))?;
            let id = CommandId::new(id).map_err(|_| {
                RpcError::new(
                    ErrorKind::InvalidParams,
                    format!("{id} is not a command id"),
                )
            })?;
            let mut command = Command::new(id);
            if let Some(Value::Object(args)) = params.get("args") {
                for (name, value) in args {
                    let value = argument(name, value).ok_or_else(|| {
                        RpcError::new(
                            ErrorKind::InvalidParams,
                            format!("the argument {name} has a value of the wrong kind"),
                        )
                    })?;
                    command = command.with_arg(name.clone(), value);
                }
            }
            command
        };
        self.event(AppEvent::Command(command))
    }

    fn event(&mut self, event: AppEvent) -> Result<Value, RpcError> {
        let before = self.panels();
        let Some((app, _)) = self.app.as_mut() else {
            return Err(RpcError::new(ErrorKind::NotInitialized, "not initialized"));
        };
        let effects = app.handle(event);
        let failed = |e: CliError| RpcError::new(ErrorKind::Failed, e.to_string());
        let said = self.carry_only(effects).map_err(failed)?;
        self.notify_changes(before).map_err(failed)?;
        Ok(self.outcome(said))
    }

    /// `ui/announce` from the client: cleaned, cut, its name last, and
    /// said as the app says anything.
    fn announce(&mut self, params: &Map<String, Value>) -> Result<Value, RpcError> {
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .map(|t| clean_text(t, MAX_CLIENT_ANNOUNCE))
            .filter(|t| !t.is_empty())
            .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "text is missing"))?;
        let message = Message::info(attribute(&text, &self.client));
        self.event(AppEvent::Problem(message))
    }

    fn panel_get(&self, params: &Map<String, Value>) -> Result<Value, RpcError> {
        let side = self.side(params)?;
        let Some((app, _)) = self.app.as_ref() else {
            return Err(RpcError::new(ErrorKind::NotInitialized, "not initialized"));
        };
        let panel = app.panel(side);
        let list = panel.list();
        Ok(json!({
            "panel": side.word(),
            "active": side == app.active_side(),
            "place": panel.place().to_string(),
            "title": list.title(),
            "count": list.len(),
            "cursor": list.focused().map(|r| json!({ "name": r.id, "row": r.spoken(), "position": list.position_text() })),
            "selection": list.selected_ids(),
            "sort": panel.sort().describe(),
            "filter": panel.filter().map(|f| f.text().to_owned()),
        }))
    }

    fn panel_set(&mut self, params: &Map<String, Value>) -> Result<Value, RpcError> {
        let side = self.side(params)?;
        let names = |key: &str| -> Vec<String> {
            params
                .get(key)
                .and_then(Value::as_array)
                .map(|v| {
                    v.iter()
                        .filter_map(|n| n.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut said = Vec::new();
        if let Some(place) = params.get("place").and_then(Value::as_str) {
            let here = self.here.clone();
            let place = Place::resolve(place, &here)
                .map_err(|e| RpcError::new(ErrorKind::InvalidParams, e.to_string()))?;
            let before = self.panels();
            let effects = self
                .app
                .as_mut()
                .map(|(app, _)| app.open_place(side, place))
                .unwrap_or_default();
            let failed = |e: CliError| RpcError::new(ErrorKind::Failed, e.to_string());
            said.extend(self.carry_only(effects).map_err(failed)?);
            self.notify_changes(before).map_err(failed)?;
        }
        let refresh = params.get("refresh").and_then(Value::as_bool) == Some(true);
        let clear = params.get("clear_selection").and_then(Value::as_bool) == Some(true);
        let (select, deselect) = (names("select"), names("deselect"));
        let cursor = params
            .get("cursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut found = true;
        let marks_before = self.panels();
        if let Some((app, _)) = self.app.as_mut() {
            if clear {
                let marked: Vec<String> = app
                    .panel(side)
                    .list()
                    .selected_ids()
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                app.mark_names(side, &marked, false);
            }
            app.mark_names(side, &select, true);
            app.mark_names(side, &deselect, false);
            if let Some(name) = cursor {
                found = app.focus_name(side, &name);
            }
        }
        // The marks and the cursor changed outside a command: their events.
        self.notify_changes(marks_before)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        if refresh {
            let reload = AppEvent::Command(Command::new(diroxide_app::commands::ids::RELOAD));
            let said_now = self.event(reload)?;
            if let Some(lines) = said_now["said"].as_array() {
                said.extend(lines.iter().filter_map(|l| l.as_str().map(str::to_owned)));
            }
        }
        Ok(json!({ "applied": true, "cursor_found": found, "said": said }))
    }

    fn dialog_get(&self) -> Result<Value, RpcError> {
        let Some((app, _)) = self.app.as_ref() else {
            return Err(RpcError::new(ErrorKind::NotInitialized, "not initialized"));
        };
        Ok(match app.dialog() {
            Some((_, dialog)) => json!({ "title": dialog.title(), "focus": dialog.focus_line() }),
            None => Value::Null,
        })
    }

    fn answer(&mut self, request: &Request) -> Result<Value, RpcError> {
        let params = &request.params;
        match request.method.as_str() {
            methods::INITIALIZE => self.initialize(params),
            methods::SHUTDOWN => {
                self.life.shut_down();
                self.shut_down = true;
                if let Some((_, runtime)) = self.app.as_mut() {
                    runtime.stop_plugins();
                }
                Ok(Value::Null)
            }
            methods::COMMAND_EXECUTE => self.command(params),
            "key/press" => match params.get("key").and_then(Value::as_str).map(Key::parse) {
                Some(Ok(key)) => self.event(AppEvent::Key(key)),
                _ => Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    "key is missing or not a key name",
                )),
            },
            methods::PANEL_GET => self.panel_get(params),
            methods::PANEL_SET => self.panel_set(params),
            "dialog/get" => self.dialog_get(),
            methods::UI_ANNOUNCE => self.announce(params),
            method => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("{method} is not a method of diro serve"),
            )),
        }
    }
}

/// A command argument from its JSON value: a place for the place
/// arguments, else text, a number, a flag, or several places.
fn argument(name: &str, value: &Value) -> Option<ArgValue> {
    let place = |text: &str| Place::parse(text).ok();
    Some(match value {
        Value::String(text) if PLACE_ARGUMENTS.contains(&name) => ArgValue::Place(place(text)?),
        Value::String(text) => ArgValue::Text(text.clone()),
        Value::Number(n) => ArgValue::Number(n.as_i64()?),
        Value::Bool(b) => ArgValue::Flag(*b),
        Value::Array(items) => ArgValue::Places(
            items
                .iter()
                .map(|i| i.as_str().and_then(place))
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

/// Serves with these folders, the current folder `here`. The exit code is
/// 0 after `shutdown` and `exit` or at the end of the input, 1 when `exit`
/// came without `shutdown`.
pub fn run_with(
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    paths: Option<&Paths>,
    here: &Place,
) -> Result<i32, CliError> {
    let failed = |e: std::io::Error| CliError::Failed(e.to_string());
    let mut server = Server {
        out,
        paths,
        here,
        app: None,
        life: Lifecycle::new(),
        client: "the client".to_owned(),
        shut_down: false,
    };
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(failed)? == 0 {
            if let Some((_, runtime)) = server.app.as_mut() {
                runtime.stop_plugins();
            }
            return Ok(0);
        }
        if line.trim().is_empty() || !protocol::is_message_line(&line) {
            continue;
        }
        let incoming = match protocol::parse_line(&line) {
            Ok(incoming) => incoming,
            Err(error) => {
                server.send(&Response::failure(None, error))?;
                continue;
            }
        };
        match server.life.gate(&incoming) {
            Gate::Proceed => {}
            Gate::Drop => continue,
            Gate::Exit => {
                if let Some((_, runtime)) = server.app.as_mut() {
                    runtime.stop_plugins();
                }
                // `exit` after `shutdown` is a clean end; before it, not.
                return Ok(i32::from(!server.shut_down));
            }
            Gate::Refuse(error) => {
                if let Incoming::Request(request) = incoming {
                    server.send(&Response::failure(Some(request.id), error))?;
                }
                continue;
            }
        }
        if let Incoming::Request(request) = incoming {
            let id: Id = request.id.clone();
            let response = match server.answer(&request) {
                Ok(value) => Response::success(id, value),
                Err(error) => Response::failure(Some(id), error),
            };
            server.send(&response)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_plugins::testing::json_matches;
    use std::path::PathBuf;

    fn session(input: &str) -> Vec<Value> {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let mut out = Vec::new();
        assert_eq!(
            run_with(&mut input.as_bytes(), &mut out, Some(&paths), &here),
            Ok(0)
        );
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn line(value: Value) -> String {
        format!("{value}\n")
    }

    fn initialize(id: u64, places: &[String]) -> String {
        line(
            json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": {"protocol": 1, "host": {"name": "Braille helper", "version": "1.0"}, "places": places}}),
        )
    }

    #[test]
    fn initialize_comes_first_and_a_session_runs_commands_by_name() {
        let here = env!("CARGO_MANIFEST_DIR").to_owned();
        let input = [
            line(json!({"jsonrpc": "2.0", "id": 1, "method": "panel/get"})),
            "stray output, not answered\n".to_owned(),
            "{not json\n".to_owned(),
            initialize(2, &[here.clone(), format!("{here}/src")]),
            line(json!({"jsonrpc": "2.0", "id": 3, "method": "panel/get", "params": {"panel": "right"}})),
            line(json!({"jsonrpc": "2.0", "id": 4, "method": "command/execute", "params": {"command": "view.sort-size"}})),
            line(json!({"jsonrpc": "2.0", "id": 5, "method": "panel/set", "params": {"cursor": "Cargo.toml", "select": ["Cargo.toml"]}})),
            line(json!({"jsonrpc": "2.0", "id": 6, "method": "panel/get"})),
            line(json!({"jsonrpc": "2.0", "id": 7, "method": "key/press", "params": {"key": "f7"}})),
            line(json!({"jsonrpc": "2.0", "id": 8, "method": "dialog/get"})),
            line(json!({"jsonrpc": "2.0", "id": 9, "method": "key/press", "params": {"key": "escape"}})),
            line(json!({"jsonrpc": "2.0", "id": 10, "method": "nothing/here"})),
        ]
        .concat();
        let messages = session(&input);
        let responses: Vec<&Value> = messages.iter().filter(|m| m.get("id").is_some()).collect();
        assert_eq!(responses[0]["error"]["code"], -32002);
        assert_eq!(responses[1]["error"]["code"], -32700);
        assert_eq!(responses[2]["result"]["name"], "diroxide");
        assert_eq!(responses[2]["result"]["protocol"], 1);
        assert_eq!(
            responses[3]["result"]["title"]
                .as_str()
                .unwrap()
                .split(',')
                .next(),
            Some("src")
        );
        assert_eq!(
            responses[4]["result"]["said"][0],
            "Sorted by size, largest first"
        );
        assert_eq!(responses[6]["result"]["selection"][0], "Cargo.toml");
        assert_eq!(responses[6]["result"]["cursor"]["name"], "Cargo.toml");
        let prompt = responses[8]["result"]["title"].as_str().unwrap();
        assert_eq!(prompt, "New folder in diroxide-cli");
        assert_eq!(responses[9]["result"]["dialog"], Value::Null);
        assert_eq!(responses[10]["error"]["code"], -32601);
        let announced: Vec<&Value> = messages
            .iter()
            .filter(|m| m["method"] == "ui/announce")
            .collect();
        assert!(
            announced
                .iter()
                .any(|m| m["params"]["text"] == "Sorted by size, largest first"
                    && m["params"]["importance"] == "info")
        );
    }

    #[test]
    fn a_client_hears_focus_folder_and_selection_before_the_answer() {
        let here = env!("CARGO_MANIFEST_DIR").to_owned();
        let input = [
            initialize(1, &[here.clone(), here.clone()]),
            line(json!({"jsonrpc": "2.0", "id": 2, "method": "panel/set", "params": {"cursor": "Cargo.toml"}})),
            line(json!({"jsonrpc": "2.0", "id": 3, "method": "key/press", "params": {"key": "space"}})),
            line(json!({"jsonrpc": "2.0", "id": 4, "method": "panel/set", "params": {"place": format!("{here}/src")}})),
            line(json!({"jsonrpc": "2.0", "id": 5, "method": "ui/announce", "params": {"text": "Reading \u{1b}[31mnow"}})),
        ]
        .concat();
        let messages = session(&input);
        let changed = |id: u64| -> Vec<Vec<String>> {
            let at = messages.iter().position(|m| m["id"] == id).unwrap();
            let start = messages[..at]
                .iter()
                .rposition(|m| m.get("id").is_some())
                .map_or(0, |p| p + 1);
            messages[start..at]
                .iter()
                .filter(|m| m["method"] == PANEL_CHANGED)
                .map(|m| {
                    m["params"]["changes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|c| c.as_str().unwrap().to_owned())
                        .collect()
                })
                .collect()
        };
        assert_eq!(changed(2), [vec!["focus".to_owned()]]);
        assert!(
            changed(3)[0].contains(&"selection".to_owned()),
            "{:?}",
            changed(3)
        );
        assert!(
            changed(4)[0].contains(&"folder".to_owned()),
            "{:?}",
            changed(4)
        );
        let said = messages.iter().find(|m| {
            m["method"] == "ui/announce" && m["params"]["text"] == "Reading now. By Braille helper."
        });
        assert!(
            said.is_some(),
            "cleaned, the client's name last: {messages:#?}"
        );
    }

    #[test]
    fn shutdown_then_only_exit_counts() {
        let input = [
            line(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": 1}})),
            line(json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"})),
            line(json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {"protocol": 1}})),
            line(json!({"jsonrpc": "2.0", "method": "exit"})),
            line(json!({"jsonrpc": "2.0", "id": 4, "method": "panel/get"})),
        ]
        .concat();
        let messages = session(&input);
        let last = messages.last().unwrap();
        assert_eq!(last["id"], 3, "nothing is answered after exit");
        assert_eq!(last["error"]["code"], -32600);
        let shutdown = messages.iter().find(|m| m["id"] == 2).unwrap();
        assert_eq!(shutdown["result"], Value::Null);
        assert!(session("").is_empty());
    }

    /// The plugins' fixture dialogues, replayed against the server, whose
    /// answering side keeps the same lifecycle. `command/run` (a plugin's
    /// request) becomes `dialog/get`, a request this server answers with a
    /// result in any state; the hooks dialogue is a plugin's alone.
    #[test]
    fn the_session_dialogues_hold_against_diro_serve() {
        let folder =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plugins/dialogues");
        let mut count = 0;
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "txt")
                || path.file_stem().is_some_and(|s| s == "hooks")
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let sent: String = text
                .lines()
                .filter_map(|l| l.strip_prefix("> "))
                .map(|l| format!("{}\n", l.replace("\"command/run\"", "\"dialog/get\"")))
                .collect();
            let wanted: Vec<&str> = text
                .lines()
                .filter_map(|l| l.strip_prefix("< "))
                .filter(|l| *l != "exit")
                .collect();
            let written: Vec<Value> = session(&sent)
                .into_iter()
                .filter(|m| m.get("id").is_some())
                .collect();
            assert_eq!(
                written.len(),
                wanted.len(),
                "{}: {written:#?}",
                path.display()
            );
            for (want, got) in wanted.iter().zip(&written) {
                let want: Value = serde_json::from_str(want).unwrap();
                assert!(
                    json_matches(&want, got),
                    "{}: wanted {want}, wrote {got}",
                    path.display()
                );
            }
            count += 1;
        }
        assert!(count >= 5, "{count} dialogues");
    }
}
