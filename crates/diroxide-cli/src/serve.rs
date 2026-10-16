//! `diro serve --stdio`: the app core over JSON-RPC 2.0, one JSON object a
//! line, the same protocol the plugins speak, with the connecting program
//! in the plugin's seat (research/plugin-protocol.md, sections 3.1 and
//! 3.6; the Decisions settled for Wave 1).
//!
//! The lifecycle: the connecting program sends `initialize` first; before
//! it, every other request is answered -32002 and notifications are
//! dropped. Then:
//!
//! - `command/execute`: a command by id, with `args`, or in the command
//!   text form as `text` ("file.copy dest=D:\backup"); the result is what
//!   was said and the dialog left open, if any.
//! - `key/press`: one key by name ("ctrl+q", "y"), as a frontend sends it.
//! - `panel/get` and `panel/set`: a panel's place, cursor, selection, sort
//!   and filter; set changes the place, the cursor and the marks.
//! - `dialog/get`: the open dialog's name and focus.
//! - `shutdown`: answered with null; the server ends.
//!
//! Every announcement goes out as a `ui/announce` notification, before the
//! answer to the request that caused it, with its importance (`status`,
//! `info`, `warning`, `error`), so a self-voicing or Braille helper can
//! follow along (R2). The end of the input ends the server cleanly.

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
use serde_json::{Map, Value, json};

use crate::CliError;
use crate::open::{OpenOptions, build, current_place, paths};

/// The methods the server answers.
pub const METHODS: [&str; 7] = [
    "command/execute",
    "key/press",
    methods::PANEL_GET,
    methods::PANEL_SET,
    "dialog/get",
    methods::INITIALIZE,
    methods::SHUTDOWN,
];

/// Argument names whose values are places.
const PLACE_ARGUMENTS: [&str; 6] = ["place", "dest", "to", "from", "source", "target"];

fn importance(message: &Message) -> &'static str {
    match (message.importance, message.kind) {
        (Importance::Low, _) => "status",
        (Importance::Normal, _) => "info",
        (Importance::High, MessageKind::Error) => "error",
        (Importance::High, _) => "warning",
    }
}

/// Serves until `shutdown` or the end of the input, with the platform's
/// config and state folders. Returns the exit code.
pub fn run(input: &mut dyn BufRead, out: &mut dyn Write) -> Result<i32, CliError> {
    let here = current_place()?;
    run_with(input, out, paths(false).as_ref(), &here)
}

struct Server<'a> {
    out: &'a mut dyn Write,
    paths: Option<&'a Paths>,
    here: &'a Place,
    app: Option<(App, Runtime)>,
}

impl Server<'_> {
    fn send<T: serde::Serialize>(&mut self, message: &T) -> Result<(), CliError> {
        let failed = |e: std::io::Error| CliError::Failed(e.to_string());
        let text = protocol::to_line(message).map_err(|e| CliError::Failed(e.to_string()))?;
        writeln!(self.out, "{text}").map_err(failed)?;
        self.out.flush().map_err(failed)
    }

    /// Carries effects out, sends each announcement as a notification, and
    /// returns what was said.
    fn carry(&mut self, effects: Vec<Effect>) -> Result<Vec<String>, CliError> {
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

    fn outcome(&self, said: Vec<String>) -> Value {
        let dialog = self
            .app
            .as_ref()
            .and_then(|(app, _)| app.dialog().map(|(_, d)| d.title()));
        json!({ "said": said, "dialog": dialog, "quit": self.app.as_ref().is_some_and(|(app, _)| app.wants_quit()) })
    }

    fn initialize(&mut self, params: &Map<String, Value>) -> Result<Value, RpcError> {
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
        let (mut app, runtime) = build(&options, self.paths, self.here)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        app.set_screen_reader(true);
        let start = app.start();
        self.app = Some((app, runtime));
        self.carry(start)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        Ok(json!({
            "name": "diroxide",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": protocol::PROTOCOL,
            "server_capabilities": { "methods": METHODS },
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
        let Some((app, _)) = self.app.as_mut() else {
            return Err(RpcError::new(ErrorKind::NotInitialized, "not initialized"));
        };
        let effects = app.handle(event);
        let said = self
            .carry(effects)
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
        Ok(self.outcome(said))
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
            let effects = self
                .app
                .as_mut()
                .map(|(app, _)| app.open_place(side, place))
                .unwrap_or_default();
            said.extend(
                self.carry(effects)
                    .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?,
            );
        }
        let refresh = params.get("refresh").and_then(Value::as_bool) == Some(true);
        let clear = params.get("clear_selection").and_then(Value::as_bool) == Some(true);
        let (select, deselect) = (names("select"), names("deselect"));
        let cursor = params
            .get("cursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut found = true;
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
        if refresh {
            let effects = self
                .app
                .as_mut()
                .map(|(app, _)| {
                    app.handle(AppEvent::Command(Command::new(
                        diroxide_app::commands::ids::RELOAD,
                    )))
                })
                .unwrap_or_default();
            said.extend(
                self.carry(effects)
                    .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?,
            );
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

    fn answer(&mut self, request: &Request) -> (Result<Value, RpcError>, bool) {
        let params = &request.params;
        if self.app.is_none() && request.method != methods::INITIALIZE {
            if request.method == methods::SHUTDOWN {
                return (Ok(Value::Null), true);
            }
            return (
                Err(RpcError::new(
                    ErrorKind::NotInitialized,
                    "initialize comes first",
                )),
                false,
            );
        }
        let result = match request.method.as_str() {
            methods::INITIALIZE if self.app.is_some() => Err(RpcError::new(
                ErrorKind::InvalidRequest,
                "the server is already initialized",
            )),
            methods::INITIALIZE => self.initialize(params),
            methods::SHUTDOWN => return (Ok(Value::Null), true),
            "command/execute" => self.command(params),
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
            method => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("{method} is not a method of diro serve"),
            )),
        };
        (result, false)
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

/// Serves with these folders, the current folder `here`.
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
    };
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(failed)? == 0 {
            return Ok(0);
        }
        if line.trim().is_empty() || !protocol::is_message_line(&line) {
            continue;
        }
        match protocol::parse_line(&line) {
            Ok(Incoming::Request(request)) => {
                let id: Id = request.id.clone();
                let (result, stop) = server.answer(&request);
                let response = match result {
                    Ok(value) => Response::success(id, value),
                    Err(error) => Response::failure(Some(id), error),
                };
                server.send(&response)?;
                if stop {
                    return Ok(0);
                }
            }
            Ok(Incoming::Notification(_) | Incoming::Response(_)) => {}
            Err(error) => server.send(&Response::failure(None, error))?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn initialize_comes_first_and_a_session_runs_commands_by_name() {
        let here = env!("CARGO_MANIFEST_DIR");
        let input = [
            line(json!({"jsonrpc": "2.0", "id": 1, "method": "panel/get"})),
            "stray output, not answered\n".to_owned(),
            "{not json\n".to_owned(),
            line(json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"places": [here, format!("{here}/src")]}})),
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
    fn shutdown_and_the_end_of_input_end_cleanly() {
        let input = [
            line(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"})),
            line(json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"})),
            line(json!({"jsonrpc": "2.0", "id": 3, "method": "initialize"})),
        ]
        .concat();
        let messages = session(&input);
        let last = messages.last().unwrap();
        assert_eq!(last["id"], 2);
        assert_eq!(last["result"], Value::Null);
        assert!(session("").is_empty());
    }
}
