//! Host services, part five: programs (`tools/run`, `tools/find`,
//! `tools/launch`), through the process helper (R3, section 3.6; R22,
//! sections 2.3 N5 and 7.2).
//!
//! - **Named, never pathed.** A tool is a name the manifest lists in
//!   `permissions.tools` (or `permissions.launch`), found by the one PATH
//!   rule (absolute folders only, never the current folder) at every run.
//!   Interpreters and shells are refused as names unless the plugin
//!   declares "any program", which its consent text says.
//! - **An argument list, never a shell line,** no console window, the whole
//!   tree killed on cancel, on the time limit, or when the output passes
//!   its cap.
//! - **The environment** is diroxide's (inherited, `NO_COLOR=1` and the
//!   rest, `DIRO_*` removed) plus a short allow-list of names the plugin
//!   may set ([`ENV_ALLOWED`]).
//! - **Output** comes back as text (UTF-8, lossy) or base64, the raw bytes
//!   of names that are not Unicode kept; with `stream`, each line (or each
//!   NUL-separated record) also arrives at once as a `tools/output`
//!   notification and is not repeated in the answer.
//! - **`tools/launch`** starts a program detached, outside every job the
//!   host makes, and answers at once.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::{Place, PlaceKind};
use diroxide_tools::process::{ProcessSpec, Stream};
use diroxide_tools::{ToolRegistry, ToolStatus, which};
use serde_json::{Value, json};

use crate::base64;
use crate::manifest::{Permissions, is_interpreter};
use crate::places::plugin_place;
use crate::protocol::{ErrorKind, Params, RpcError};
use crate::router::CancelFlag;
use crate::services::clean_text;
use crate::spool::Spool;

/// The environment variables a plugin may set for a tool.
pub const ENV_ALLOWED: &[&str] = &[
    "LC_ALL",
    "LANG",
    "LANGUAGE",
    "TZ",
    "NO_COLOR",
    "TERM",
    "PAGER",
    "GIT_PAGER",
    "GIT_TERMINAL_PROMPT",
    "GIT_OPTIONAL_LOCKS",
    "GIT_EDITOR",
    "GH_PROMPT_DISABLED",
    "GH_NO_UPDATE_NOTIFIER",
];

/// How long a tool may run by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// The longest a plugin may let a tool run.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3600);

/// The output a tool may make by default, both streams together.
pub const DEFAULT_MAX_OUTPUT: u64 = 16 * 1024 * 1024;

/// The most output a plugin may ask for.
pub const MAX_OUTPUT: u64 = 64 * 1024 * 1024;

/// Where the host finds tools.
#[derive(Clone, Debug)]
pub struct ToolBox {
    /// The PATH the one rule reads; the process's own by default.
    pub path: Option<OsString>,
    /// The registry, for known tools' version arguments and the owner's
    /// `tools.toml` paths.
    pub registry: ToolRegistry,
}

impl Default for ToolBox {
    fn default() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            registry: ToolRegistry::builtin(),
        }
    }
}

impl ToolBox {
    /// A tool box reading another PATH, for tests.
    pub fn with_path(path: Option<OsString>) -> Self {
        Self {
            registry: ToolRegistry::builtin().with_path(path.clone()),
            path,
        }
    }

    /// Where a tool is: a registry tool by its id (and the owner's
    /// override), anything else by the one PATH rule.
    pub fn locate(&self, name: &str) -> Option<PathBuf> {
        if self.registry.spec(name).is_some() {
            return match self.registry.find(name) {
                Ok(ToolStatus::Found(path)) => Some(path),
                _ => None,
            };
        }
        which(name, self.path.as_deref())
    }
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

/// A program name a plugin may use: plain, and not an interpreter unless
/// it declared any program.
fn check_name(name: &str, permissions: &Permissions) -> Result<(), RpcError> {
    if name.is_empty()
        || name.contains(['/', '\\', '\0'])
        || name.starts_with('.')
        || name.contains(':')
    {
        return Err(invalid(format!("{name} is not a tool name")));
    }
    if is_interpreter(name) && !permissions.any_program() {
        return Err(RpcError::new(
            ErrorKind::DeniedByHost,
            format!("{name} runs any code; the plugin did not declare any program"),
        ));
    }
    Ok(())
}

fn string_list(params: &Params, key: &str) -> Result<Vec<String>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|s| !s.contains('\0'))
                    .map(str::to_owned)
                    .ok_or_else(|| invalid(format!("{key} must be a list of texts")))
            })
            .collect(),
        Some(_) => Err(invalid(format!("{key} must be a list of texts"))),
    }
}

fn local_folder(params: &Params) -> Result<Option<PathBuf>, RpcError> {
    let Some(text) = params.get("cwd").and_then(Value::as_str) else {
        return Ok(None);
    };
    let place: Place = plugin_place(text)?;
    let path = match (place.kind(), place.local_path()) {
        (PlaceKind::Local, Some(path)) => PathBuf::from(path),
        _ => return Err(invalid("cwd must be a local folder")),
    };
    if !path.is_dir() {
        return Err(RpcError::new(
            ErrorKind::NotADirectory,
            "cwd is not a folder",
        ));
    }
    Ok(Some(path))
}

/// How a run's output travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Text,
    Base64,
}

impl Encoding {
    fn of(bytes: &[u8], encoding: Encoding) -> Value {
        match encoding {
            Encoding::Text => json!(String::from_utf8_lossy(bytes)),
            Encoding::Base64 => json!(base64::encode(bytes)),
        }
    }
}

/// Sends one streamed chunk: the stream's name and its bytes.
pub(crate) type Emit = Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

struct Pump {
    bytes: Vec<u8>,
}

/// Reads one stream until it ends, keeping what fits under the shared cap
/// and streaming records when asked.
fn pump(
    mut reader: impl Read,
    name: &'static str,
    budget: Arc<AtomicU64>,
    over: Arc<AtomicBool>,
    stream: Option<(Emit, u8)>,
) -> Pump {
    let mut kept = Vec::new();
    let mut pending = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let chunk = &buffer[..n];
        let allowed = budget
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                Some(left.saturating_sub(n as u64))
            })
            .unwrap_or(0)
            .min(n as u64) as usize;
        if allowed < n {
            over.store(true, Ordering::SeqCst);
        }
        let chunk = &chunk[..allowed];
        match &stream {
            Some((emit, separator)) => {
                pending.extend_from_slice(chunk);
                while let Some(at) = pending.iter().position(|b| b == separator) {
                    let record: Vec<u8> = pending.drain(..=at).collect();
                    emit(name, &record[..record.len() - 1]);
                }
            }
            None => kept.extend_from_slice(chunk),
        }
        if over.load(Ordering::SeqCst) {
            break;
        }
    }
    if let Some((emit, _)) = &stream
        && !pending.is_empty()
    {
        emit(name, &pending);
    }
    Pump { bytes: kept }
}

/// Runs a tool for `tools/run`. `emit` sends streamed records.
pub(crate) fn run(
    params: &Params,
    permissions: &Permissions,
    tools: &ToolBox,
    spool: Option<&Spool>,
    cancel: &CancelFlag,
    emit: Option<Emit>,
) -> Result<Value, RpcError> {
    let name = params
        .get("tool")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("tool is missing"))?;
    check_name(name, permissions)?;
    let args = string_list(params, "args")?;
    let cwd = local_folder(params)?;
    let encoding = match params.get("encoding").and_then(Value::as_str) {
        None | Some("text") => Encoding::Text,
        Some("base64") => Encoding::Base64,
        Some(other) => return Err(invalid(format!("encoding {other} is not text or base64"))),
    };
    let streamed = string_list(params, "stream")?;
    if streamed.iter().any(|s| s != "stdout" && s != "stderr") {
        return Err(invalid("stream lists stdout and stderr only"));
    }
    let separator = match params.get("split").and_then(Value::as_str) {
        None | Some("lines") => b'\n',
        Some("nul") => 0,
        Some(other) => return Err(invalid(format!("split {other} is not lines or nul"))),
    };
    let timeout = params
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_TIMEOUT, Duration::from_millis)
        .min(MAX_TIMEOUT);
    let cap = params
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_OUTPUT)
        .min(MAX_OUTPUT);
    let input: Option<Box<dyn Read + Send>> = match (
        params.get("stdin").and_then(Value::as_str),
        params.get("stdin_file").and_then(Value::as_str),
    ) {
        (Some(_), Some(_)) => return Err(invalid("give stdin or stdin_file, not both")),
        (Some(text), None) => Some(Box::new(std::io::Cursor::new(text.as_bytes().to_vec()))),
        (None, Some(spool_path)) => {
            let spool = spool
                .ok_or_else(|| RpcError::new(ErrorKind::Unsupported, "this host has no spool"))?;
            Some(Box::new(
                spool.open_to_read(spool_path).map_err(|r| r.to_rpc())?,
            ))
        }
        (None, None) => None,
    };
    let mut spec = ProcessSpec::new(
        tools
            .locate(name)
            .ok_or_else(|| RpcError::new(ErrorKind::NotFound, format!("{name} was not found")))?,
    )
    .args(&args)
    .diroxide_environment()
    .stdin(if input.is_some() {
        Stream::Piped
    } else {
        Stream::Null
    })
    .stdout(Stream::Piped)
    .stderr(Stream::Piped);
    if let Some(cwd) = cwd {
        spec = spec.cwd(cwd);
    }
    if let Some(Value::Object(env)) = params.get("env") {
        for (key, value) in env {
            if !ENV_ALLOWED.contains(&key.as_str()) {
                return Err(RpcError::new(
                    ErrorKind::DeniedByHost,
                    format!("{key} is not a variable a plugin may set"),
                ));
            }
            let value = value
                .as_str()
                .ok_or_else(|| invalid(format!("{key} must be a text")))?;
            spec = spec.env(key, value);
        }
    }
    let mut child = spec
        .spawn()
        .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
    if let (Some(mut input), Some(mut stdin)) = (input, child.take_stdin()) {
        thread::Builder::new()
            .name("tool-stdin".to_owned())
            .spawn(move || {
                let _ = std::io::copy(&mut input, &mut stdin);
                let _ = stdin.flush();
            })
            .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
    }
    let budget = Arc::new(AtomicU64::new(cap));
    let over = Arc::new(AtomicBool::new(false));
    let start_pump = |reader: Option<Box<dyn Read + Send>>, label: &'static str| {
        let budget = Arc::clone(&budget);
        let over = Arc::clone(&over);
        let stream = emit
            .clone()
            .filter(|_| streamed.iter().any(|s| s == label))
            .map(|emit| (emit, separator));
        reader.and_then(|reader| {
            thread::Builder::new()
                .name(format!("tool-{label}"))
                .spawn(move || pump(reader, label, budget, over, stream))
                .ok()
        })
    };
    let stdout = start_pump(
        child
            .take_stdout()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
        "stdout",
    );
    let stderr = start_pump(
        child
            .take_stderr()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
        "stderr",
    );
    let started = Instant::now();
    let ending = loop {
        if let Ok(Some(status)) = child.try_wait() {
            // A child it left behind must not hold the pipes open.
            let _ = child.start_kill();
            break Ok(status.code());
        }
        if cancel.is_cancelled() {
            let _ = child.kill_tree();
            break Err(RpcError::new(ErrorKind::RequestCancelled, "cancelled"));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill_tree();
            break Err(RpcError::new(
                ErrorKind::TimedOut,
                format!(
                    "{name} did not finish in {}",
                    crate::lines::duration_words(timeout)
                ),
            ));
        }
        if over.load(Ordering::SeqCst) {
            let _ = child.kill_tree();
            break Ok(None);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let collected = Mutex::new((Vec::new(), Vec::new()));
    for (handle, index) in [(stdout, 0), (stderr, 1)] {
        if let Some(handle) = handle
            && let Ok(pump) = handle.join()
        {
            let mut out = collected.lock().unwrap_or_else(PoisonError::into_inner);
            if index == 0 {
                out.0 = pump.bytes;
            } else {
                out.1 = pump.bytes;
            }
        }
    }
    let exit_code = ending?;
    let (out, err) = collected
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(json!({
        "exit_code": exit_code,
        "stdout": Encoding::of(&out, encoding),
        "stderr": Encoding::of(&err, encoding),
        "encoding": if encoding == Encoding::Base64 { "base64" } else { "text" },
        "truncated": over.load(Ordering::SeqCst),
    }))
}

/// `tools/find`: whether a tool is found, where, and, for a known tool,
/// its version line.
pub(crate) fn find(params: &Params, tools: &ToolBox) -> Result<Value, RpcError> {
    let name = params
        .get("tool")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("tool is missing"))?;
    if name.is_empty() || name.contains(['/', '\\', '\0', ':']) {
        return Err(invalid(format!("{name} is not a tool name")));
    }
    let Some(path) = tools.locate(name) else {
        return Ok(json!({"found": false, "path": null, "version": null}));
    };
    // Only a known tool is run for its version: running anything else
    // would be running a program the manifest never named.
    let version = tools
        .registry
        .spec(name)
        .filter(|spec| !spec.version_args.is_empty())
        .and_then(|spec| version_line(&path, spec.version_args));
    Ok(json!({"found": true, "path": path.display().to_string(), "version": version}))
}

fn version_line(path: &std::path::Path, args: &[&str]) -> Option<String> {
    let mut child = ProcessSpec::new(path)
        .args(args)
        .diroxide_environment()
        .stdin(Stream::Null)
        .stdout(Stream::Piped)
        .stderr(Stream::Null)
        .spawn()
        .ok()?;
    let mut out = child.take_stdout()?;
    let reader = thread::spawn(move || {
        let mut text = Vec::new();
        let _ = (&mut out).take(64 * 1024).read_to_end(&mut text);
        text
    });
    if child.wait_timeout(Duration::from_secs(2)).ok()?.is_none() {
        let _ = child.kill_tree();
    }
    let text = reader.join().ok()?;
    String::from_utf8_lossy(&text)
        .lines()
        .map(|line| clean_text(line, 80))
        .find(|line| !line.is_empty())
}

/// `tools/launch`: a program started detached; answers at once.
pub(crate) fn launch(
    params: &Params,
    permissions: &Permissions,
    tools: &ToolBox,
) -> Result<Value, RpcError> {
    let name = params
        .get("program")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("program is missing"))?;
    check_name(name, permissions)?;
    let args = string_list(params, "args")?;
    let cwd = local_folder(params)?;
    let path = tools
        .locate(name)
        .ok_or_else(|| RpcError::new(ErrorKind::NotFound, format!("{name} was not found")))?;
    let mut spec = ProcessSpec::new(path)
        .args(&args)
        .diroxide_environment()
        .stdin(Stream::Null)
        .stdout(Stream::Null)
        .stderr(Stream::Null);
    if let Some(cwd) = cwd {
        spec = spec.cwd(cwd);
    }
    let pid = spec
        .launch()
        .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
    Ok(json!({"launched": true, "pid": pid}))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::env;
    use std::io::Write;

    use super::*;

    /// The name the fake tool goes by on the test PATH.
    pub(crate) const FAKE: &str = "faketool";

    /// The fake tool: this test binary, run again with a mode among its
    /// arguments; an ordinary test run does nothing here.
    #[test]
    fn fake_tool() {
        let Some(mode) = env::args().find_map(|a| a.strip_prefix("mode=").map(str::to_owned))
        else {
            return;
        };
        let mut out = std::io::stdout();
        match mode.as_str() {
            "echo" => {
                for arg in env::args().filter(|a| a.starts_with("arg=")) {
                    writeln!(out, "{arg}").unwrap();
                }
                if let Ok(lang) = env::var("LC_ALL") {
                    writeln!(out, "LC_ALL={lang}").unwrap();
                }
            }
            "stdin" => {
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text).unwrap();
                writeln!(out, "read:{}", text.trim()).unwrap();
            }
            "big" => {
                let line = vec![b'x'; 1024 * 1024];
                for _ in 0..100 {
                    if out.write_all(&line).is_err() {
                        break;
                    }
                }
            }
            "forever" => {
                writeln!(out, "started").unwrap();
                out.flush().unwrap();
                thread::sleep(Duration::from_secs(600));
            }
            "fail" => {
                eprintln!("cannot read a.txt");
                out.flush().unwrap();
                std::process::exit(3);
            }
            "bytes" => {
                out.write_all(b"name-\xff\xfe\0next\0").unwrap();
            }
            _ => {}
        }
        out.flush().unwrap();
        std::process::exit(0);
    }

    /// A folder holding the fake tool, for a test PATH.
    pub(crate) fn fake_tool_folder() -> (tempfile::TempDir, ToolBox) {
        let folder = tempfile::tempdir().unwrap();
        let file = if cfg!(windows) {
            format!("{FAKE}.exe")
        } else {
            FAKE.to_owned()
        };
        std::fs::copy(env::current_exe().unwrap(), folder.path().join(file)).unwrap();
        let tools = ToolBox::with_path(Some(folder.path().as_os_str().to_owned()));
        (folder, tools)
    }

    /// The arguments that make the copy run the fake tool in a mode.
    pub(crate) fn fake_args(mode: &str, extra: &[&str]) -> Value {
        let mut args = vec![
            "--exact".to_owned(),
            "tools::tests::fake_tool".to_owned(),
            "--nocapture".to_owned(),
            "--test-threads".to_owned(),
            "1".to_owned(),
            format!("mode={mode}"),
        ];
        args.extend(extra.iter().map(|a| (*a).to_owned()));
        json!(args)
    }

    fn permissions(tools: &[&str]) -> Permissions {
        Permissions {
            tools: tools.iter().map(|t| (*t).to_owned()).collect(),
            ..Permissions::default()
        }
    }

    fn params(value: Value) -> Params {
        value.as_object().cloned().unwrap()
    }

    fn run_fake(tools: &ToolBox, mode: &str, more: Value) -> Result<Value, RpcError> {
        let mut p = params(json!({"tool": FAKE, "args": fake_args(mode, &[])}));
        if let Value::Object(more) = more {
            p.extend(more);
        }
        run(
            &p,
            &permissions(&[FAKE]),
            tools,
            None,
            &CancelFlag::default(),
            None,
        )
    }

    #[test]
    fn a_tool_runs_with_an_argument_array_and_allowed_variables() {
        let (_folder, tools) = fake_tool_folder();
        let mut p = params(json!({
            "tool": FAKE,
            "args": fake_args("echo", &["arg=two words", "arg=--dash", "arg=caf\u{e9}"]),
            "env": {"LC_ALL": "C"}
        }));
        let done = run(
            &p,
            &permissions(&[FAKE]),
            &tools,
            None,
            &CancelFlag::default(),
            None,
        )
        .unwrap();
        assert_eq!(done["exit_code"], 0);
        let text = done["stdout"].as_str().unwrap();
        assert!(
            text.contains("arg=two words\n") || text.contains("arg=two words\r\n"),
            "{text}"
        );
        assert!(text.contains("arg=--dash"), "{text}");
        assert!(text.contains("arg=caf\u{e9}"), "{text}");
        assert!(text.contains("LC_ALL=C"), "{text}");
        p.insert("env".into(), json!({"PATH": "/evil"}));
        let refused = run(
            &p,
            &permissions(&[FAKE]),
            &tools,
            None,
            &CancelFlag::default(),
            None,
        )
        .unwrap_err();
        assert_eq!(refused.code, -32030);
    }

    #[test]
    fn interpreters_are_refused_unless_any_program_is_declared() {
        let (_folder, tools) = fake_tool_folder();
        for name in ["sh", "bash", "python3", "pwsh", "cmd", "env", "node"] {
            let error = run(
                &params(json!({"tool": name})),
                &permissions(&[name]),
                &tools,
                None,
                &CancelFlag::default(),
                None,
            )
            .unwrap_err();
            assert_eq!(error.code, -32030, "{name}");
        }
        let mut any = permissions(&["sh"]);
        any.declared.programs = Some(crate::manifest::ANY_PROGRAM.to_owned());
        let missing = run(
            &params(json!({"tool": "sh"})),
            &any,
            &ToolBox::with_path(None),
            None,
            &CancelFlag::default(),
            None,
        )
        .unwrap_err();
        assert_eq!(
            missing.code, -32010,
            "an allowed sh is looked up, not refused"
        );
        for bad in ["../x", "C:\\x.exe", "/bin/ls", ""] {
            let error = run(
                &params(json!({"tool": bad})),
                &permissions(&[bad]),
                &tools,
                None,
                &CancelFlag::default(),
                None,
            )
            .unwrap_err();
            assert_eq!(error.code, -32602, "{bad}");
        }
    }

    #[test]
    fn a_failing_tool_is_a_successful_run_with_its_code() {
        let (_folder, tools) = fake_tool_folder();
        let failed = run_fake(&tools, "fail", Value::Null).unwrap();
        assert_eq!(failed["exit_code"], 3);
        assert!(
            failed["stderr"]
                .as_str()
                .unwrap()
                .contains("cannot read a.txt")
        );
    }

    #[test]
    fn standard_input_reaches_the_tool() {
        let (_folder, tools) = fake_tool_folder();
        let done = run_fake(&tools, "stdin", json!({"stdin": "hello"})).unwrap();
        assert!(
            done["stdout"].as_str().unwrap().contains("read:hello"),
            "{done}"
        );
    }

    /// The test plan's tool that prints 100 MB: the output stops at its
    /// cap, the tree is killed, and the answer says so.
    #[test]
    fn output_is_capped_and_the_tool_stopped() {
        let (_folder, tools) = fake_tool_folder();
        let started = Instant::now();
        let capped = run_fake(&tools, "big", json!({"max_output_bytes": 2_000_000})).unwrap();
        assert_eq!(capped["truncated"], true);
        assert!(capped["stdout"].as_str().unwrap().len() <= 2_000_000);
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    /// The test plan's tool that never exits: the time limit kills the
    /// tree and says so.
    #[test]
    fn a_tool_that_never_ends_is_stopped_at_its_limit() {
        let (_folder, tools) = fake_tool_folder();
        let started = Instant::now();
        let error = run_fake(&tools, "forever", json!({"timeout_ms": 1500})).unwrap_err();
        assert_eq!(error.code, -32034);
        assert_eq!(error.message, "faketool did not finish in 1 second");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn cancel_stops_the_tool() {
        let (_folder, tools) = fake_tool_folder();
        let cancel = CancelFlag::default();
        let raise = {
            let cancel = cancel.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(500));
                cancel.cancel();
            })
        };
        let error = run(
            &params(json!({"tool": FAKE, "args": fake_args("forever", &[])})),
            &permissions(&[FAKE]),
            &tools,
            None,
            &cancel,
            None,
        )
        .unwrap_err();
        raise.join().unwrap();
        assert_eq!(error.code, -32800);
    }

    #[test]
    fn raw_bytes_come_back_as_base64_and_records_stream() {
        let (_folder, tools) = fake_tool_folder();
        let raw = run_fake(&tools, "bytes", json!({"encoding": "base64"})).unwrap();
        let bytes = base64::decode(raw["stdout"].as_str().unwrap()).unwrap();
        assert!(bytes.windows(9).any(|w| w == b"name-\xff\xfe\0n"));
        let records = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&records);
        let emit: Emit = Arc::new(move |stream: &str, chunk: &[u8]| {
            seen.lock()
                .unwrap()
                .push((stream.to_owned(), chunk.to_vec()));
        });
        let streamed = run(
            &params(json!({"tool": FAKE, "args": fake_args("bytes", &[]),
                "stream": ["stdout"], "split": "nul"})),
            &permissions(&[FAKE]),
            &tools,
            None,
            &CancelFlag::default(),
            Some(emit),
        )
        .unwrap();
        assert_eq!(streamed["stdout"], "");
        let records = records.lock().unwrap();
        assert!(
            records
                .iter()
                .any(|(s, r)| s == "stdout" && r.ends_with(b"name-\xff\xfe"))
        );
        assert!(records.iter().any(|(_, r)| r == b"next"));
    }

    #[test]
    fn find_says_where_and_never_runs_an_unknown_tool() {
        let (_folder, tools) = fake_tool_folder();
        let found = find(&params(json!({"tool": FAKE})), &tools).unwrap();
        assert_eq!(found["found"], true);
        assert_eq!(found["version"], Value::Null);
        let missing = find(&params(json!({"tool": "no-such-tool"})), &tools).unwrap();
        assert_eq!(
            missing,
            json!({"found": false, "path": null, "version": null})
        );
        assert!(find(&params(json!({"tool": "../x"})), &tools).is_err());
    }

    #[test]
    fn a_launch_answers_at_once() {
        let (_folder, tools) = fake_tool_folder();
        let started = Instant::now();
        let allowed = Permissions {
            launch: vec![FAKE.to_owned()],
            ..Permissions::default()
        };
        let launched = launch(
            &params(json!({"program": FAKE, "args": fake_args("echo", &[])})),
            &allowed,
            &tools,
        )
        .unwrap();
        assert_eq!(launched["launched"], true);
        assert!(started.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(300));
    }
}
