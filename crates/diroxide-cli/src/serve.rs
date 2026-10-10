//! `diro serve --stdio`: the app core over JSON-RPC 2.0, one JSON object a
//! line, the same protocol the plugins speak.
//!
//! Phase 0 answers `initialize` with the server's name and version and
//! `shutdown` with null (then stops), refuses every other request with
//! "method not found", ignores notifications, and answers a bad line with
//! a parse error. Wave 1 (W1d) adds the app's commands and events.

use std::io::{BufRead, Write};

use diroxide_plugins::protocol::{self, Incoming, Response, codes, methods};
use serde_json::json;

use crate::CliError;

/// Serves until `shutdown` or the end of the input. Returns the exit code.
pub fn run(input: &mut dyn BufRead, out: &mut dyn Write) -> Result<i32, CliError> {
    let failed = |e: std::io::Error| CliError::Failed(e.to_string());
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(failed)? == 0 {
            return Ok(0);
        }
        if line.trim().is_empty() {
            continue;
        }
        let (answer, stop) = match protocol::parse_line(&line) {
            Ok(Incoming::Request(request)) => {
                let id = request.id.clone();
                match (request.method.as_str(), id) {
                    (methods::INITIALIZE, Some(id)) => (
                        Some(Response::success(
                            id,
                            json!({ "name": "diroxide", "version": env!("CARGO_PKG_VERSION") }),
                        )),
                        false,
                    ),
                    (methods::SHUTDOWN, Some(id)) => {
                        (Some(Response::success(id, serde_json::Value::Null)), true)
                    }
                    (method, id) => (
                        Some(Response::failure(
                            id,
                            codes::METHOD_NOT_FOUND,
                            &format!("{method} is not built yet"),
                        )),
                        false,
                    ),
                }
            }
            Ok(Incoming::Notification(_) | Incoming::Response(_)) => (None, false),
            Err(error) => (
                Some(Response::failure(
                    None,
                    codes::PARSE_ERROR,
                    &error.to_string(),
                )),
                false,
            ),
        };
        if let Some(answer) = answer {
            let text = protocol::to_line(&answer).map_err(|e| CliError::Failed(e.to_string()))?;
            writeln!(out, "{text}").map_err(failed)?;
            out.flush().map_err(failed)?;
        }
        if stop {
            return Ok(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_initializes_fails_politely_and_shuts_down() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"log","params":["ignored"]}"#,
            "\n",
            "not json\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"panel/get"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":4,"method":"initialize"}"#,
            "\n",
        );
        let mut out = Vec::new();
        assert_eq!(run(&mut input.as_bytes(), &mut out), Ok(0));
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains(r#""result":{"name":"diroxide""#));
        assert!(lines[1].contains(r#""code":-32700"#));
        assert!(lines[2].contains("panel/get is not built yet"));
        assert_eq!(lines[3], r#"{"jsonrpc":"2.0","id":3,"result":null}"#);
    }
}
