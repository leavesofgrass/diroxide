//! JSON-RPC 2.0, one JSON object per line, UTF-8, in both directions.
//! The headless server (`diro serve --stdio`) speaks the same protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::PluginError;

/// The protocol version, sent in every message.
pub const VERSION: &str = "2.0";

/// Standard JSON-RPC error codes.
pub mod codes {
    /// The line was not valid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON was not a valid request.
    pub const INVALID_REQUEST: i64 = -32600;
    /// No such method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The parameters were wrong.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Something failed inside.
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// Method names.
pub mod methods {
    /// Host to plugin: start, with the host's version and the plugin's
    /// settings.
    pub const INITIALIZE: &str = "initialize";
    /// Host to plugin: run one of the plugin's commands.
    pub const COMMAND_RUN: &str = "command/run";
    /// Host to plugin: the prefix of provider methods (`provider/list`).
    pub const PROVIDER_PREFIX: &str = "provider/";
    /// Host to plugin: make a preview.
    pub const PREVIEW_RENDER: &str = "preview/render";
    /// Host to plugin: run a conversion.
    pub const CONVERT_RUN: &str = "convert/run";
    /// Host to plugin: a hooked event happened.
    pub const HOOK_EVENT: &str = "hook/event";
    /// Host to plugin: stop.
    pub const SHUTDOWN: &str = "shutdown";
    /// Plugin to host: say something.
    pub const UI_ANNOUNCE: &str = "ui/announce";
    /// Plugin to host: ask yes or no, choose, or type.
    pub const UI_ASK: &str = "ui/ask";
    /// Plugin to host: report progress.
    pub const UI_PROGRESS: &str = "ui/progress";
    /// Plugin to host: read a panel's place, selection and cursor.
    pub const PANEL_GET: &str = "panel/get";
    /// Plugin to host: change a panel.
    pub const PANEL_SET: &str = "panel/set";
    /// Plugin to host: the prefix of file operations through the job
    /// engine (`fs/list`, `fs/copy`).
    pub const FS_PREFIX: &str = "fs/";
    /// Plugin to host: run a registered tool.
    pub const TOOLS_RUN: &str = "tools/run";
    /// Plugin to host: read a setting.
    pub const CONFIG_GET: &str = "config/get";
    /// Plugin to host: write to the plugin's log.
    pub const LOG: &str = "log";
}

/// A request id: a number or a string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    /// A number.
    Number(i64),
    /// A string.
    Text(String),
}

/// A request, or a notification when it has no id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Always "2.0".
    pub jsonrpc: String,
    /// The id; `None` for a notification, which gets no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    /// The method.
    pub method: String,
    /// The parameters.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    /// A request with an id.
    pub fn new(id: Id, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            id: Some(id),
            method: method.to_owned(),
            params,
        }
    }

    /// A notification.
    pub fn notification(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            id: None,
            method: method.to_owned(),
            params,
        }
    }
}

/// An error in a response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    /// The code; see [`codes`].
    pub code: i64,
    /// A short sentence.
    pub message: String,
    /// Anything more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A response: a result or an error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Always "2.0".
    pub jsonrpc: String,
    /// The request's id; `None` (null) when the request could not be read.
    pub id: Option<Id>,
    /// The result, on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error, on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A successful response.
    pub fn success(id: Id, result: Value) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    pub fn failure(id: Option<Id>, code: i64, message: &str) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.to_owned(),
                data: None,
            }),
        }
    }
}

/// A message read from a line.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A request that wants an answer.
    Request(Request),
    /// A notification.
    Notification(Request),
    /// An answer to one of our requests.
    Response(Response),
}

#[derive(Deserialize)]
struct Raw {
    jsonrpc: Option<String>,
    id: Option<Id>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
    result: Option<Value>,
    error: Option<RpcError>,
}

/// Reads one line as a message. The error text says what was wrong, for
/// the plugin's log and a [`codes::PARSE_ERROR`] or
/// [`codes::INVALID_REQUEST`] answer.
pub fn parse_line(line: &str) -> Result<Incoming, PluginError> {
    let raw: Raw = serde_json::from_str(line.trim_end_matches(['\r', '\n']))
        .map_err(|e| PluginError::Protocol(e.to_string()))?;
    if raw.jsonrpc.as_deref() != Some(VERSION) {
        return Err(PluginError::Protocol("jsonrpc is not \"2.0\"".to_owned()));
    }
    match (raw.method, raw.result, raw.error) {
        (Some(method), None, None) => {
            let request = Request {
                jsonrpc: VERSION.to_owned(),
                id: raw.id,
                method,
                params: raw.params,
            };
            Ok(if request.id.is_some() {
                Incoming::Request(request)
            } else {
                Incoming::Notification(request)
            })
        }
        (None, result, error) if result.is_some() != error.is_some() => {
            Ok(Incoming::Response(Response {
                jsonrpc: VERSION.to_owned(),
                id: raw.id,
                result,
                error,
            }))
        }
        _ => Err(PluginError::Protocol(
            "a message needs a method, or one of result and error".to_owned(),
        )),
    }
}

/// Writes a message as one line, without the line break.
pub fn to_line<T: Serialize>(message: &T) -> Result<String, PluginError> {
    serde_json::to_string(message).map_err(|e| PluginError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_notifications_and_responses_are_told_apart() {
        let request =
            parse_line(r#"{"jsonrpc":"2.0","id":1,"method":"command/run","params":{"id":"x"}}"#)
                .unwrap();
        assert!(matches!(request, Incoming::Request(ref r) if r.id == Some(Id::Number(1))));
        let note = parse_line(r#"{"jsonrpc":"2.0","method":"log","params":["hi"]}"#).unwrap();
        assert!(matches!(note, Incoming::Notification(_)));
        let answer = parse_line(r#"{"jsonrpc":"2.0","id":"a","result":true}"#).unwrap();
        assert!(matches!(answer, Incoming::Response(ref r) if r.result == Some(json!(true))));
    }

    #[test]
    fn bad_lines_are_refused_with_words() {
        assert!(parse_line("not json").is_err());
        let err = parse_line(r#"{"id":1,"method":"x"}"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the message is not valid JSON-RPC: jsonrpc is not \"2.0\""
        );
        assert!(parse_line(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
        assert!(
            parse_line(r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"x"}}"#)
                .is_err()
        );
    }

    #[test]
    fn messages_round_trip_as_one_line() {
        let request = Request::new(
            Id::Number(7),
            methods::INITIALIZE,
            json!({"host": "diroxide"}),
        );
        let line = to_line(&request).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(parse_line(&line).unwrap(), Incoming::Request(request));
        let failure = Response::failure(None, codes::METHOD_NOT_FOUND, "no such method");
        let line = to_line(&failure).unwrap();
        assert_eq!(
            line,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32601,"message":"no such method"}}"#
        );
    }
}
