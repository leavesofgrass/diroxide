//! JSON-RPC 2.0, one message per line, UTF-8, in both directions. The
//! headless server (`diro serve --stdio`) speaks the same protocol.
//!
//! The conventions, from R3's review (`research/plugin-protocol.md`,
//! sections 2 and 7):
//!
//! - One message per line. The host ends lines with `\n` and accepts `\n`
//!   or `\r\n`. Batches (JSON arrays) are refused with
//!   [`ErrorKind::InvalidRequest`].
//! - A standard-output line that does not start with `{` (after spaces) is
//!   stray output, logged and not answered ([`is_message_line`]); a line
//!   that starts with `{` and does not parse is answered with a parse error
//!   and a null id.
//! - Requests have an id; notifications do not; `null` ids appear only in
//!   the parse-error answer.
//! - `params` is always an object, absent meaning `{}`.
//! - Names are `snake_case`, methods `area/verb_noun` ([`methods`]).
//! - Errors carry a code from [`ErrorKind`]: JSON-RPC's five, two borrowed
//!   from LSP, and diroxide's range from -32010 to -32099, with `data.kind`
//!   naming the kind.
//!
//! The full message list is [`methods::ALL`], and the protocol reference,
//! `docs/dev/plugin-protocol.md`, is generated from it by
//! [`reference::page`] (`cargo xtask regen`).
//! Lines are read with a bound and written from a queue by
//! [`crate::framing`].

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// The JSON-RPC version, sent in every message.
pub const VERSION: &str = "2.0";

/// diroxide's protocol major version, carried by `initialize`. Only
/// additions are made within a major version.
pub const PROTOCOL: u32 = 1;

/// The longest line read by default, in bytes.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// The longest line a plugin may ask for, in bytes.
pub const MAX_LINE_BYTES_LIMIT: usize = 64 * 1024 * 1024;

/// A message's parameters: always an object.
pub type Params = Map<String, Value>;

/// Error codes, as constants.
pub mod codes {
    /// The line was not valid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON was not a valid message.
    pub const INVALID_REQUEST: i64 = -32600;
    /// No such method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The parameters were wrong.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Something failed inside.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// A request came before `initialize` was answered (LSP's code).
    pub const NOT_INITIALIZED: i64 = -32002;
    /// The request was cancelled (LSP's code).
    pub const REQUEST_CANCELLED: i64 = -32800;
}

/// Every error a message can carry, with its code and its `snake_case`
/// name, which `error.data.kind` repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// -32700: the line was not valid JSON.
    ParseError,
    /// -32600: not a valid message, or a batch.
    InvalidRequest,
    /// -32601: no such method.
    MethodNotFound,
    /// -32602: the parameters were wrong.
    InvalidParams,
    /// -32603: something failed inside.
    InternalError,
    /// -32002: a request came before `initialize` was answered.
    NotInitialized,
    /// -32800: the request was cancelled.
    RequestCancelled,
    /// -32010: the place does not exist.
    NotFound,
    /// -32011: the place exists and the call does not overwrite.
    AlreadyExists,
    /// -32012: the backend refused.
    PermissionDenied,
    /// -32013: a folder was needed.
    NotADirectory,
    /// -32014: a file was needed.
    IsADirectory,
    /// -32015: a folder to remove has entries.
    NotEmpty,
    /// -32016: the name breaks a rule of the provider.
    InvalidName,
    /// -32017: the method or an option is not supported.
    Unsupported,
    /// -32018: offline or busy; retry later.
    Unavailable,
    /// -32019: credentials are missing or were rejected.
    AuthRequired,
    /// -32020: the quota or the disk is full.
    NoSpace,
    /// -32021: the item exceeds a limit.
    TooLarge,
    /// -32022: the item changed under the call.
    Conflict,
    /// -32030: from the host: the manifest does not permit this call.
    DeniedByHost,
    /// -32031: from the host: a message size or rate limit.
    LimitExceeded,
    /// -32032: from the host: the call would loop back into the plugin.
    Reentrant,
    /// -32033: from the host: the plugin is not running, crashed or is
    /// disabled.
    PluginUnavailable,
    /// -32034: from the host: no answer in time.
    TimedOut,
    /// -32099: anything else, in the plugin's own words.
    Failed,
}

impl ErrorKind {
    /// Every kind, in code order within each group.
    pub const ALL: [ErrorKind; 26] = [
        ErrorKind::ParseError,
        ErrorKind::InvalidRequest,
        ErrorKind::MethodNotFound,
        ErrorKind::InvalidParams,
        ErrorKind::InternalError,
        ErrorKind::NotInitialized,
        ErrorKind::RequestCancelled,
        ErrorKind::NotFound,
        ErrorKind::AlreadyExists,
        ErrorKind::PermissionDenied,
        ErrorKind::NotADirectory,
        ErrorKind::IsADirectory,
        ErrorKind::NotEmpty,
        ErrorKind::InvalidName,
        ErrorKind::Unsupported,
        ErrorKind::Unavailable,
        ErrorKind::AuthRequired,
        ErrorKind::NoSpace,
        ErrorKind::TooLarge,
        ErrorKind::Conflict,
        ErrorKind::DeniedByHost,
        ErrorKind::LimitExceeded,
        ErrorKind::Reentrant,
        ErrorKind::PluginUnavailable,
        ErrorKind::TimedOut,
        ErrorKind::Failed,
    ];

    /// The code.
    pub const fn code(self) -> i64 {
        match self {
            ErrorKind::ParseError => codes::PARSE_ERROR,
            ErrorKind::InvalidRequest => codes::INVALID_REQUEST,
            ErrorKind::MethodNotFound => codes::METHOD_NOT_FOUND,
            ErrorKind::InvalidParams => codes::INVALID_PARAMS,
            ErrorKind::InternalError => codes::INTERNAL_ERROR,
            ErrorKind::NotInitialized => codes::NOT_INITIALIZED,
            ErrorKind::RequestCancelled => codes::REQUEST_CANCELLED,
            ErrorKind::NotFound => -32010,
            ErrorKind::AlreadyExists => -32011,
            ErrorKind::PermissionDenied => -32012,
            ErrorKind::NotADirectory => -32013,
            ErrorKind::IsADirectory => -32014,
            ErrorKind::NotEmpty => -32015,
            ErrorKind::InvalidName => -32016,
            ErrorKind::Unsupported => -32017,
            ErrorKind::Unavailable => -32018,
            ErrorKind::AuthRequired => -32019,
            ErrorKind::NoSpace => -32020,
            ErrorKind::TooLarge => -32021,
            ErrorKind::Conflict => -32022,
            ErrorKind::DeniedByHost => -32030,
            ErrorKind::LimitExceeded => -32031,
            ErrorKind::Reentrant => -32032,
            ErrorKind::PluginUnavailable => -32033,
            ErrorKind::TimedOut => -32034,
            ErrorKind::Failed => -32099,
        }
    }

    /// The `snake_case` name, as `error.data.kind` carries it.
    pub const fn name(self) -> &'static str {
        match self {
            ErrorKind::ParseError => "parse_error",
            ErrorKind::InvalidRequest => "invalid_request",
            ErrorKind::MethodNotFound => "method_not_found",
            ErrorKind::InvalidParams => "invalid_params",
            ErrorKind::InternalError => "internal_error",
            ErrorKind::NotInitialized => "not_initialized",
            ErrorKind::RequestCancelled => "request_cancelled",
            ErrorKind::NotFound => "not_found",
            ErrorKind::AlreadyExists => "already_exists",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::NotADirectory => "not_a_directory",
            ErrorKind::IsADirectory => "is_a_directory",
            ErrorKind::NotEmpty => "not_empty",
            ErrorKind::InvalidName => "invalid_name",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Unavailable => "unavailable",
            ErrorKind::AuthRequired => "auth_required",
            ErrorKind::NoSpace => "no_space",
            ErrorKind::TooLarge => "too_large",
            ErrorKind::Conflict => "conflict",
            ErrorKind::DeniedByHost => "denied_by_host",
            ErrorKind::LimitExceeded => "limit_exceeded",
            ErrorKind::Reentrant => "reentrant",
            ErrorKind::PluginUnavailable => "plugin_unavailable",
            ErrorKind::TimedOut => "timed_out",
            ErrorKind::Failed => "failed",
        }
    }

    /// The kind with a code, if any.
    pub fn from_code(code: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// The kind by its `snake_case` name, if any.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// What the kind means, one sentence, for the protocol reference.
    pub const fn meaning(self) -> &'static str {
        match self {
            ErrorKind::ParseError => "The line was not valid JSON.",
            ErrorKind::InvalidRequest => "The JSON was not a valid message, or it was a batch.",
            ErrorKind::MethodNotFound => {
                "No such method; unknown notifications are ignored instead."
            }
            ErrorKind::InvalidParams => "The parameters were wrong.",
            ErrorKind::InternalError => "Something failed inside the receiver.",
            ErrorKind::NotInitialized => "A request came before initialize was answered.",
            ErrorKind::RequestCancelled => "The request was cancelled.",
            ErrorKind::NotFound => "The place does not exist.",
            ErrorKind::AlreadyExists => "The place exists and the call does not overwrite.",
            ErrorKind::PermissionDenied => "The backend refused.",
            ErrorKind::NotADirectory => "A folder was needed.",
            ErrorKind::IsADirectory => "A file was needed.",
            ErrorKind::NotEmpty => "A folder to remove has entries.",
            ErrorKind::InvalidName => "The name breaks a rule of the provider.",
            ErrorKind::Unsupported => "The method or an option is not supported.",
            ErrorKind::Unavailable => "Offline or busy; retry later.",
            ErrorKind::AuthRequired => "Credentials are missing or were rejected.",
            ErrorKind::NoSpace => "The quota or the disk is full.",
            ErrorKind::TooLarge => "The item exceeds a limit.",
            ErrorKind::Conflict => "The item changed under the call.",
            ErrorKind::DeniedByHost => "From the host: the manifest does not permit this call.",
            ErrorKind::LimitExceeded => "From the host: a size, queue or rate limit.",
            ErrorKind::Reentrant => "From the host: the call would loop back into the plugin.",
            ErrorKind::PluginUnavailable => {
                "From the host: the plugin is not running, crashed or is off."
            }
            ErrorKind::TimedOut => "From the host: no answer in time.",
            ErrorKind::Failed => "Anything else, in the plugin's own words.",
        }
    }
}

pub mod methods;
pub mod reference;

/// A request id: a number or a string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    /// A number; the host counts from 1.
    Number(i64),
    /// A string.
    Text(String),
}

/// A request: it has an id and wants an answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Always "2.0".
    pub jsonrpc: String,
    /// The id, unique per sender while outstanding.
    pub id: Id,
    /// The method.
    pub method: String,
    /// The parameters; absent means `{}`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Params,
}

impl Request {
    /// A request.
    pub fn new(id: Id, method: &str, params: Params) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            id,
            method: method.to_owned(),
            params,
        }
    }
}

/// A notification: no id, no answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// Always "2.0".
    pub jsonrpc: String,
    /// The method.
    pub method: String,
    /// The parameters; absent means `{}`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Params,
}

impl Notification {
    /// A notification.
    pub fn new(method: &str, params: Params) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            method: method.to_owned(),
            params,
        }
    }
}

/// The structured part of an error.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    /// The kind's `snake_case` name, from [`ErrorKind::name`].
    #[serde(default)]
    pub kind: String,
    /// The place the error is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// The next step, one sentence, for the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// More, for the log only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When to try again, for `unavailable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// An error in a response. `message` is one short sentence naming the
/// cause only, meaning first; the host builds the user's line from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    /// The code; see [`ErrorKind`].
    pub code: i64,
    /// The cause, in lowercase words.
    pub message: String,
    /// The structured part, boxed to keep the error small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Box<ErrorData>>,
}

impl RpcError {
    /// An error of a kind, with `data.kind` filled in.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            code: kind.code(),
            message: message.into(),
            data: Some(Box::new(ErrorData {
                kind: kind.name().to_owned(),
                ..ErrorData::default()
            })),
        }
    }

    /// The kind, by `data.kind` when it names one, else by the code.
    pub fn kind(&self) -> Option<ErrorKind> {
        self.data
            .as_ref()
            .and_then(|d| ErrorKind::ALL.into_iter().find(|k| k.name() == d.kind))
            .or_else(|| ErrorKind::from_code(self.code))
    }
}

/// A response: a result or an error, never both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Always "2.0".
    pub jsonrpc: String,
    /// The request's id; `None` (null) only when the request could not be
    /// read.
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
    pub fn failure(id: Option<Id>, error: RpcError) -> Self {
        Self {
            jsonrpc: VERSION.to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A message read from a line.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A request that wants an answer.
    Request(Request),
    /// A notification.
    Notification(Notification),
    /// An answer to one of our requests.
    Response(Response),
}

/// Deserializes a field that is present (even as `null`) into `Some`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
struct Raw {
    jsonrpc: Option<String>,
    #[serde(default, deserialize_with = "present")]
    id: Option<Value>,
    method: Option<String>,
    #[serde(default, deserialize_with = "present")]
    params: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    result: Option<Value>,
    error: Option<RpcError>,
}

/// Whether a line is meant as a message: it starts with `{` after optional
/// spaces. Any other output line is stray, logged and not answered.
pub fn is_message_line(line: &str) -> bool {
    line.trim_start().starts_with('{')
}

/// Reads one line as a message. The error is the answer to send back, with
/// a null id: a parse error, an invalid request (including a batch), or
/// invalid params (params that are not an object).
pub fn parse_line(line: &str) -> Result<Incoming, RpcError> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim_start().starts_with('[') {
        return Err(RpcError::new(
            ErrorKind::InvalidRequest,
            "batches are not supported",
        ));
    }
    let raw: Raw = serde_json::from_str(line)
        .map_err(|e| RpcError::new(ErrorKind::ParseError, format!("the line is not JSON: {e}")))?;
    let invalid = |message: &str| RpcError::new(ErrorKind::InvalidRequest, message);
    if raw.jsonrpc.as_deref() != Some(VERSION) {
        return Err(invalid("jsonrpc is not \"2.0\""));
    }
    let params = match raw.params {
        None => Params::new(),
        Some(Value::Object(map)) => map,
        Some(_) => {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "params is not an object",
            ));
        }
    };
    let id = match raw.id {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => {
            Some(Some(serde_json::from_value::<Id>(value).map_err(|_| {
                invalid("the id is not a number or a string")
            })?))
        }
    };
    match (raw.method, raw.result, raw.error) {
        (Some(method), None, None) => match id {
            None => Ok(Incoming::Notification(Notification {
                jsonrpc: VERSION.to_owned(),
                method,
                params,
            })),
            Some(Some(id)) => Ok(Incoming::Request(Request {
                jsonrpc: VERSION.to_owned(),
                id,
                method,
                params,
            })),
            Some(None) => Err(invalid("a request's id is null")),
        },
        (None, result, error) if result.is_some() != error.is_some() => {
            Ok(Incoming::Response(Response {
                jsonrpc: VERSION.to_owned(),
                id: id.flatten(),
                result,
                error,
            }))
        }
        _ => Err(invalid(
            "a message needs a method, or one of result and error",
        )),
    }
}

/// Writes a message as one line, without the line break.
pub fn to_line<T: Serialize>(message: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(message)
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
        assert!(
            matches!(request, Incoming::Request(ref r) if r.id == Id::Number(1) && r.params["id"] == "x")
        );
        let note =
            parse_line("{\"jsonrpc\":\"2.0\",\"method\":\"log\",\"params\":{\"text\":\"hi\"}}\r\n")
                .unwrap();
        assert!(matches!(note, Incoming::Notification(ref n) if n.method == "log"));
        let answer = parse_line(r#"{"jsonrpc":"2.0","id":"a","result":true}"#).unwrap();
        assert!(matches!(answer, Incoming::Response(ref r) if r.result == Some(json!(true))));
        let nothing = parse_line(r#"{"jsonrpc":"2.0","id":2,"result":null}"#).unwrap();
        assert!(matches!(nothing, Incoming::Response(ref r) if r.result == Some(Value::Null)));
    }

    #[test]
    fn bad_lines_get_the_right_answer() {
        assert_eq!(
            parse_line("{not json").unwrap_err().kind(),
            Some(ErrorKind::ParseError)
        );
        assert_eq!(parse_line("[{}]").unwrap_err().code, codes::INVALID_REQUEST);
        let no_version = parse_line(r#"{"id":1,"method":"x"}"#).unwrap_err();
        assert_eq!(no_version.message, "jsonrpc is not \"2.0\"");
        let array_params =
            parse_line(r#"{"jsonrpc":"2.0","id":1,"method":"x","params":[1]}"#).unwrap_err();
        assert_eq!(array_params.kind(), Some(ErrorKind::InvalidParams));
        let null_id = parse_line(r#"{"jsonrpc":"2.0","id":null,"method":"x"}"#).unwrap_err();
        assert_eq!(null_id.kind(), Some(ErrorKind::InvalidRequest));
        assert!(parse_line(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
        assert!(!is_message_line("debug: still here"));
        assert!(is_message_line("  {\"jsonrpc\":\"2.0\"}"));
    }

    #[test]
    fn messages_round_trip_as_one_line() {
        let mut params = Params::new();
        params.insert("protocol".into(), json!(PROTOCOL));
        let request = Request::new(Id::Number(7), methods::INITIALIZE, params);
        let line = to_line(&request).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(parse_line(&line).unwrap(), Incoming::Request(request));
        let failure = Response::failure(
            None,
            RpcError::new(ErrorKind::MethodNotFound, "no such method"),
        );
        assert_eq!(
            to_line(&failure).unwrap(),
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32601,"message":"no such method","data":{"kind":"method_not_found"}}}"#
        );
    }

    #[test]
    fn every_kind_has_one_code_and_a_snake_case_name() {
        for kind in ErrorKind::ALL {
            assert_eq!(ErrorKind::from_code(kind.code()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.name()));
            assert!(
                kind.name()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
            );
        }
        let plugin_error = RpcError {
            code: -32010,
            message: "no such object".into(),
            data: None,
        };
        assert_eq!(plugin_error.kind(), Some(ErrorKind::NotFound));
    }

    #[test]
    fn method_names_are_snake_case() {
        for method in [
            methods::PROVIDER_CREATE_DIR,
            methods::PROVIDER_WRITE_BEGIN,
            methods::PROVIDER_SET_ATTRIBUTES,
            methods::CANCEL_REQUEST,
            methods::UI_ANNOUNCE,
        ] {
            assert!(
                method
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || "/_$".contains(c)),
                "{method}"
            );
            assert!(!method.starts_with("rpc."));
        }
    }
}
