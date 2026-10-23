//! The misbehaving plugin of the test plan (R22, section 8.1): a real
//! process plugin, chosen by a mode word, that drives the host through the
//! cases a careless or hostile plugin makes. Built only with the
//! `fake-plugin` feature, for the host's tests and the conformance suite.
//!
//! `diroxide-fake-plugin MODE [--after N]`; `--help` describes every mode.
//! The protocol itself is the in-process fake plugin's
//! (`diroxide_plugins::testing::FakePlugin`); this program adds what only
//! a process can do: wait, crash, leave a child, write half a line, end
//! lines with CRLF, and write more than the limit.

use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diroxide_plugins::protocol::{Incoming, Params, methods, parse_line, to_line};
use diroxide_plugins::testing::memfs::MemFs;
use diroxide_plugins::testing::{FakePlugin, Mode};
use diroxide_plugins::{Notification, Response};
use serde_json::{Value, json};

/// Every mode, with what it does, meaning first.
const MODES: &[(&str, &str)] = &[
    (
        "echo",
        "Correct: answers what it knows, -32601 for the rest.",
    ),
    (
        "talk",
        "Says something and reports progress while a command runs.",
    ),
    (
        "slow",
        "Reports progress six times, half a second apart, then answers.",
    ),
    ("silent", "Never answers anything, not even initialize."),
    (
        "cancel-deaf",
        "Never answers a command and ignores $/cancel_request.",
    ),
    (
        "flood",
        "Sends a hundred announcements while a command runs.",
    ),
    (
        "big",
        "Writes a line over the 8 MiB limit, then answers with 7 MiB.",
    ),
    (
        "bad",
        "Writes broken JSON, bytes that are not UTF-8, a batch and an answer with no id; answers requests before initialize and unknown methods.",
    ),
    (
        "crash",
        "Exits with code 3 on its Nth message (--after N, 3 by default).",
    ),
    (
        "orphan",
        "Starts a child that sleeps and shares its output, then answers.",
    ),
    ("crlf", "Ends every line it writes with CR LF."),
    (
        "partial",
        "Writes half a line when a command runs, then exits.",
    ),
    (
        "reentrant",
        "Calls fs/stat on its own memfs scheme while a command runs.",
    ),
    (
        "spoof",
        "Announces text that imitates the app, with hidden characters.",
    ),
    (
        "escape",
        "Asks fs/read to write outside its spool while a command runs.",
    ),
    ("double-answer", "Answers every request twice."),
    ("stray", "Prints stray lines while a command runs."),
    (
        "call",
        "Makes the host call its command's args.call and answers with the result.",
    ),
    ("provider", "Serves the toy file system memfs://box."),
    (
        "lost",
        "Says its provider lost a device while a command runs.",
    ),
    ("sleep", "Sleeps two minutes: the orphan's child."),
];

fn help() -> String {
    let mut text = String::from(
        "diroxide-fake-plugin: a misbehaving plugin for the host's tests.\nUsage: diroxide-fake-plugin MODE [--after N]\nModes:\n",
    );
    for (word, what) in MODES {
        text.push_str(&format!("  {word}: {what}\n"));
    }
    text
}

/// Writes lines to the host, with LF or CR LF.
struct Out {
    crlf: bool,
}

impl Out {
    fn line(&self, text: &str) {
        let mut out = io::stdout().lock();
        let end = if self.crlf { "\r\n" } else { "\n" };
        let _ = out.write_all(text.as_bytes());
        let _ = out.write_all(end.as_bytes());
        let _ = out.flush();
    }

    fn raw(&self, bytes: &[u8]) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }
}

fn request_of(line: &str) -> Option<(Value, String, Params)> {
    match parse_line(line).ok()? {
        Incoming::Request(request) => Some((json!(request.id), request.method, request.params)),
        _ => None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(word) = args.first().map(String::as_str) else {
        print!("{}", help());
        return;
    };
    if word == "--help" || word == "-h" {
        print!("{}", help());
        return;
    }
    if !MODES.iter().any(|(w, _)| *w == word) {
        eprintln!("no mode {word}; see --help");
        std::process::exit(2);
    }
    if word == "sleep" {
        thread::sleep(Duration::from_secs(120));
        return;
    }
    let after = args
        .iter()
        .position(|a| a == "--after")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(3);
    let mode = match word {
        "talk" => Mode::Talk,
        "silent" => Mode::Silent,
        "cancel-deaf" => Mode::CancelDeaf,
        "flood" => Mode::Flood,
        "bad" => Mode::Bad,
        "spoof" => Mode::Spoof,
        "stray" => Mode::Stray,
        "call" | "reentrant" | "escape" => Mode::Call,
        "lost" => Mode::Lost,
        _ => Mode::Echo,
    };
    let mut plugin = FakePlugin::new(mode);
    if word == "provider" {
        let mut fs = MemFs::default();
        fs.add_file("/a.txt", b"alpha");
        plugin.memfs = Some(Arc::new(Mutex::new(fs)));
    }
    let out = Out {
        crlf: word == "crlf",
    };
    let mut child = None;
    if word == "orphan"
        && let Ok(exe) = std::env::current_exe()
    {
        // The child shares this process's output, so the host's reading
        // ends only when both are gone.
        child = Command::new(exe)
            .arg("sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .spawn()
            .ok();
    }
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut buffer = Vec::new();
    let mut count = 0;
    let mut initialized = false;
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        count += 1;
        if word == "crash" && count >= after {
            std::process::exit(3);
        }
        let line = String::from_utf8_lossy(&buffer);
        let mut line = line.trim_end_matches(['\r', '\n']).to_owned();
        let request = request_of(&line);
        let method = request.as_ref().map(|(_, m, _)| m.as_str()).unwrap_or("");
        if method == methods::COMMAND_RUN {
            match word {
                "slow" => {
                    let token = request
                        .as_ref()
                        .and_then(|(_, _, p)| p.get("progress_token").cloned())
                        .unwrap_or(Value::Null);
                    for i in 0..6 {
                        let mut params = Params::new();
                        params.insert("token".into(), token.clone());
                        params.insert(
                            "value".into(),
                            json!({"kind": if i == 0 { "begin" } else { "report" }, "title": "Slow work", "current": i, "total": 6}),
                        );
                        out.line(
                            &to_line(&Notification::new(methods::PROGRESS, params))
                                .unwrap_or_default(),
                        );
                        thread::sleep(Duration::from_millis(500));
                    }
                }
                "big" => {
                    out.line(&"x".repeat(9 * 1024 * 1024));
                    if let Some((id, _, _)) = &request {
                        let id = serde_json::from_value(id.clone()).ok();
                        let answer = Response::success(
                            id.unwrap_or(diroxide_plugins::Id::Number(0)),
                            json!({"done": 1, "data": "y".repeat(7 * 1024 * 1024)}),
                        );
                        out.line(&to_line(&answer).unwrap_or_default());
                    }
                    continue;
                }
                "partial" => {
                    out.raw(b"{\"jsonrpc\":\"2.0\",\"id\":");
                    std::process::exit(0);
                }
                "reentrant" | "escape" => {
                    if let Ok(Incoming::Request(mut request)) = parse_line(&line) {
                        let place = request
                            .params
                            .get("args")
                            .and_then(|a| a.get("place"))
                            .cloned()
                            .unwrap_or(json!("memfs://box/a.txt"));
                        let call = if word == "reentrant" {
                            json!({"method": "fs/stat", "params": {"place": "memfs://box/a.txt"}})
                        } else {
                            json!({"method": "fs/read", "params": {"place": place, "transfer": "file", "spool_path": "../escape.part"}})
                        };
                        request.params.insert("args".into(), json!({"call": call}));
                        line = to_line(&request).unwrap_or(line);
                    }
                }
                _ => {}
            }
        }
        let answers = plugin.handle_line(&line);
        let was_initialize = method == methods::INITIALIZE;
        for answer in &answers {
            out.line(answer);
            if word == "double-answer"
                && answer.contains("\"id\"")
                && !answer.contains("\"method\"")
            {
                out.line(answer);
            }
        }
        if word == "bad" && was_initialize && !initialized {
            initialized = true;
            out.line("{not json");
            out.raw(
                b"{\"jsonrpc\":\"2.0\",\"method\":\"log\",\"params\":{\"message\":\"\xff\xfe\"}}\n",
            );
            out.line("[{\"jsonrpc\":\"2.0\",\"method\":\"log\",\"params\":{\"message\":\"in a batch\"}}]");
            out.line("{\"jsonrpc\":\"2.0\",\"result\":1}");
        }
        if plugin.exited() {
            break;
        }
    }
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}
