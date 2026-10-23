//! The misbehaving plugin against the real host, one test per mode (R22,
//! section 8.1): a real process through the process helper and the stdio
//! transport. Built with the `fake-plugin` feature.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use diroxide_config::PluginSettings;
use diroxide_plugins::host::QUIT_BUDGET;
use diroxide_plugins::log::PluginLog;
use diroxide_plugins::protocol::methods;
use diroxide_plugins::provider::PluginProvider;
use diroxide_plugins::{Host, HostEvent, Manifest, PluginError, PluginInfo, PluginTransport};
use diroxide_plugins::{StdioTransport, session};
use diroxide_vfs::{OpContext, Provider};
use serde_json::{Value, json};

const EXE: &str = env!("CARGO_BIN_EXE_diroxide-fake-plugin");

/// A plugin folder whose manifest runs the fake in `mode`.
fn plugin(folder: &Path, mode: &[&str], extra: &str) -> PluginInfo {
    let command: Vec<String> = std::iter::once(EXE)
        .chain(mode.iter().copied())
        .map(|a| format!("'{a}'"))
        .collect();
    let startup = if extra.contains("startup_timeout") {
        ""
    } else {
        "startup_timeout = \"5s\"\n"
    };
    let text = format!(
        "[plugin]\nid = \"fake\"\nname = \"Fake plugin\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[run]\ncommand = [{}]\n{startup}{extra}\n[[command]]\nid = \"echo\"\nname = \"Echo\"\n",
        command.join(", ")
    );
    let report = Manifest::check(&text);
    let problems = report.text();
    let manifest = report
        .manifest
        .unwrap_or_else(|| panic!("the manifest does not read: {problems}"));
    PluginInfo {
        manifest,
        folder: folder.to_path_buf(),
        enabled: true,
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    host: Arc<Host>,
}

fn host(mode: &[&str], extra: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let mut host = Host::new(&PluginSettings::default())
        .with_state_dir(path.join("state"))
        .with_registry(Arc::new(diroxide_vfs::Registry::with_local()));
    host.add(plugin(&path, mode, extra));
    Fixture {
        _root: root,
        root: path,
        host: Arc::new(host),
    }
}

fn run(host: &Host, args: Value) -> Result<Value, PluginError> {
    host.call(
        "fake",
        methods::COMMAND_RUN,
        json!({"command": "echo", "args": args}),
    )
}

fn said(host: &Host) -> Vec<String> {
    host.take_events()
        .into_iter()
        .filter_map(|e| match e {
            HostEvent::Said { message, .. } => Some(message.text),
            _ => None,
        })
        .collect()
}

fn log_text(host: &Host) -> String {
    host.log("fake")
        .and_then(|log| log.path())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn help_describes_every_mode() {
    let output = Command::new(EXE).arg("--help").output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    for mode in [
        "echo",
        "talk",
        "slow",
        "silent",
        "cancel-deaf",
        "flood",
        "big",
        "bad",
        "crash",
        "orphan",
        "crlf",
        "partial",
        "reentrant",
        "spoof",
        "escape",
        "double-answer",
        "stray",
        "call",
        "provider",
        "lost",
    ] {
        assert!(
            text.lines()
                .any(|line| line.trim_start().starts_with(&format!("{mode}: "))),
            "{mode} is not described:\n{text}"
        );
    }
    let unknown = Command::new(EXE).arg("dance").output().unwrap();
    assert_eq!(unknown.status.code(), Some(2));
}

#[test]
fn echo_answers_and_stops_politely() {
    let f = host(&["echo"], "");
    assert_eq!(run(&f.host, Value::Null).unwrap()["done"], 1);
    assert!(f.host.is_running("fake"));
    f.host.shutdown_all(QUIT_BUDGET);
    assert!(!f.host.is_running("fake"));
}

#[test]
fn talk_is_said_with_the_plugin_last() {
    let f = host(&["talk"], "");
    run(&f.host, Value::Null).unwrap();
    let said = said(&f.host);
    assert!(
        said.contains(&"Lines: 42, files: 1. By Fake plugin.".to_owned()),
        "{said:?}"
    );
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn slow_progress_keeps_a_command_with_a_short_limit_alive() {
    let f = host(&["slow"], "\n[timeouts]\ncommand = \"1s\"\n");
    let _supervisor = Host::start_supervisor(&f.host).unwrap();
    let started = Instant::now();
    assert_eq!(run(&f.host, Value::Null).unwrap()["done"], 1);
    assert!(started.elapsed() >= Duration::from_millis(2500));
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn silent_never_starts() {
    let f = host(&["silent"], "startup_timeout = \"1s\"\n");
    let error = run(&f.host, Value::Null).unwrap_err();
    assert!(
        error.to_string().contains("did not start in 1 second"),
        "{error}"
    );
}

#[test]
fn cancel_deaf_times_out_and_is_said() {
    let f = host(&["cancel-deaf"], "\n[timeouts]\ncommand = \"1s\"\n");
    let _supervisor = Host::start_supervisor(&f.host).unwrap();
    let error = run(&f.host, Value::Null).unwrap_err();
    assert_eq!(error, PluginError::Timeout("Fake plugin".into()));
    // The caller's wait ends on its own deadline; the supervisor's tick
    // records the event on its own thread, a moment later on a loaded
    // runner, so the event is waited for, not expected at once.
    let timed_out = |events: &[HostEvent]| {
        events.iter().any(|e| {
            matches!(e, HostEvent::TimedOut { message, .. }
            if message.text == "Echo timed out after 1 second.")
        })
    };
    let started = Instant::now();
    let mut events = f.host.take_events();
    while !timed_out(&events) && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
        f.host.tick();
        events.extend(f.host.take_events());
    }
    assert!(timed_out(&events), "{events:?}");
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn flood_is_folded() {
    let f = host(&["flood"], "");
    run(&f.host, Value::Null).unwrap();
    assert_eq!(said(&f.host).len(), 20);
    std::thread::sleep(Duration::from_millis(1100));
    f.host.tick();
    let fold = said(&f.host);
    assert!(
        fold.iter()
            .any(|s| s.ends_with("more messages from Fake plugin")),
        "{fold:?}"
    );
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn big_lines_are_refused_and_the_next_line_still_read() {
    let f = host(&["big"], "");
    let answer = run(&f.host, Value::Null).unwrap();
    assert_eq!(answer["data"].as_str().unwrap().len(), 7 * 1024 * 1024);
    f.host.shutdown_all(QUIT_BUDGET);
    assert!(log_text(&f.host).contains("over the limit"));
}

#[test]
fn bad_output_is_logged_and_the_host_goes_on() {
    let f = host(&["bad"], "");
    assert_eq!(run(&f.host, Value::Null).unwrap()["done"], 1);
    f.host.shutdown_all(QUIT_BUDGET);
    let log = log_text(&f.host);
    assert!(log.contains("could not be read"), "{log}");
    assert!(log.contains("not UTF-8"), "{log}");
    assert!(log.contains("nobody waited for"), "{log}");
}

#[test]
fn crash_fails_the_command_and_is_said() {
    let f = host(&["crash", "--after", "3"], "");
    let error = run(&f.host, Value::Null).unwrap_err();
    assert_eq!(error, PluginError::Stopped("Fake plugin".into()));
    f.host.tick();
    let events = f.host.take_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HostEvent::Crashed { message, .. }
            if message.text.starts_with("Fake plugin stopped unexpectedly."))),
        "{events:?}"
    );
}

/// The test plan's tree kill: the plugin's child shares its output, so
/// the output ends only when the kill took the child too.
#[test]
fn orphan_dies_with_its_plugin() {
    let folder = tempfile::tempdir().unwrap();
    let transport = StdioTransport::start(
        "Fake plugin",
        diroxide_tools::ProcessSpec::new(EXE).arg("orphan"),
        PluginLog::in_memory(),
        diroxide_plugins::protocol::MAX_LINE_BYTES,
    )
    .unwrap();
    let params = session::InitializeParams::for_plugin("fake");
    session::handshake(&transport, "Fake plugin", &params, Duration::from_secs(5)).unwrap();
    transport.kill().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match transport.receive(Duration::from_millis(100)) {
            Err(_) => break,
            Ok(_) => assert!(
                Instant::now() < deadline,
                "the orphan outlived the kill of its tree"
            ),
        }
    }
    drop(folder);
}

#[test]
fn crlf_lines_are_read() {
    let f = host(&["crlf"], "");
    assert_eq!(run(&f.host, Value::Null).unwrap()["done"], 1);
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn half_a_line_then_exit_is_a_stop() {
    let f = host(&["partial"], "");
    assert_eq!(
        run(&f.host, Value::Null).unwrap_err(),
        PluginError::Stopped("Fake plugin".into())
    );
}

#[test]
fn reentrant_calls_are_refused() {
    let f = host(
        &["reentrant"],
        "\n[permissions]\nfs = \"read\"\n\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\n",
    );
    let answer = run(&f.host, Value::Null).unwrap();
    assert_eq!(answer["host_error"]["code"], -32032, "{answer}");
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn spoof_is_cleaned_and_attributed() {
    let f = host(&["spoof"], "");
    run(&f.host, Value::Null).unwrap();
    assert_eq!(said(&f.host), ["Moved to the trash. By Fake plugin."]);
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn escape_from_the_spool_is_refused() {
    let f = host(&["escape"], "\n[permissions]\nfs = \"read\"\n");
    let file = f.root.join("a.txt");
    std::fs::write(&file, b"alpha").unwrap();
    let answer = run(&f.host, json!({"place": file.display().to_string()})).unwrap();
    assert_eq!(answer["host_error"]["code"], -32030, "{answer}");
    assert!(
        !f.root
            .join("state")
            .join("fake")
            .join("escape.part")
            .exists()
    );
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn a_double_answer_is_dropped() {
    let f = host(&["double-answer"], "");
    assert_eq!(run(&f.host, Value::Null).unwrap()["done"], 1);
    assert_eq!(run(&f.host, Value::Null).unwrap()["done"], 1);
    f.host.shutdown_all(QUIT_BUDGET);
    assert!(log_text(&f.host).contains("nobody waited for"));
}

#[test]
fn stray_text_is_said_once() {
    let f = host(&["stray"], "");
    run(&f.host, Value::Null).unwrap();
    assert_eq!(
        said(&f.host),
        ["Fake plugin wrote stray text; see its log."]
    );
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn call_without_a_frontend_is_cancelled() {
    let f = host(&["call"], "");
    let answer = run(
        &f.host,
        json!({"call": {"method": "ui/ask", "params": {"kind": "text"}}}),
    )
    .unwrap();
    assert_eq!(answer["host"], json!({"cancelled": true}));
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn the_toy_provider_serves_its_place() {
    let f = host(
        &["provider"],
        "\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\nmethods = [\"read\"]\n",
    );
    let provider = PluginProvider::new(&f.host, "fake", "memfs").unwrap();
    let place = diroxide_core::Place::parse("memfs://box/a.txt").unwrap();
    assert_eq!(
        provider.stat(&place, &OpContext::new()).unwrap().size,
        Some(5)
    );
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut provider.read(&place, &OpContext::new()).unwrap(),
        &mut text,
    )
    .unwrap();
    assert_eq!(text, "alpha");
    f.host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn a_lost_device_is_an_event() {
    let f = host(&["lost"], "");
    run(&f.host, Value::Null).unwrap();
    wait_until("the lost event", || {
        f.host
            .take_events()
            .iter()
            .any(|e| matches!(e, HostEvent::ProviderLost { .. }))
    });
    f.host.shutdown_all(QUIT_BUDGET);
}
