//! Spikes S2 and S9 of `research/alpha4-plan.md` (section 10), ignored in
//! normal runs; run by hand with
//! `cargo nextest run -p diroxide-plugins --all-features --run-ignored only spike_s --no-capture`
//! and record what they print in the builder's report.
//!
//! - **S2, a plugin that never answers:** the `cancel-deaf` fake with the
//!   default 30-second limit, the real supervisor, and a heartbeat thread
//!   standing in for the app's loop. Pass: the loop never stalls beyond
//!   50 ms; "waiting for the plugin" comes after two seconds; the cancel
//!   goes out at 30 seconds and the time-out is said two seconds later; a
//!   second time-out inside five minutes restarts the plugin, whose tree
//!   is then gone.
//! - **S9, start latency:** spawn to the `initialize` answer for Python,
//!   bash and Windows PowerShell, 100 starts each, median and worst.

use std::env;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_plugins::log::PluginLog;
use diroxide_plugins::protocol::MAX_LINE_BYTES;
use diroxide_plugins::session::{InitializeParams, handshake};
use diroxide_plugins::{PluginTransport, StdioTransport};
use diroxide_tools::process::ProcessSpec;
use diroxide_tools::which;

#[cfg(feature = "fake-plugin")]
#[test]
#[ignore = "spike S2: about 70 seconds of real time; run by hand"]
fn spike_s2_a_plugin_that_never_answers() {
    use diroxide_config::PluginSettings;
    use diroxide_plugins::protocol::methods;
    use diroxide_plugins::{Host, HostEvent, Manifest, PluginInfo};
    use serde_json::json;

    let exe = env!("CARGO_BIN_EXE_diroxide-fake-plugin");
    let folder = tempfile::tempdir().unwrap();
    let text = format!(
        "[plugin]\nid = \"fake\"\nname = \"Fake plugin\"\nversion = \"0.1.0\"\nprotocol = 1\n[run]\ncommand = ['{exe}', 'cancel-deaf']\n[timeouts]\ncommand = \"30s\"\n[[command]]\nid = \"echo\"\nname = \"Echo\"\n"
    );
    let mut host =
        Host::new(&PluginSettings::default()).with_state_dir(folder.path().join("state"));
    host.add(PluginInfo {
        manifest: Manifest::from_toml(&text).unwrap(),
        folder: folder.path().to_path_buf(),
        enabled: true,
    });
    let host = Arc::new(host);
    let _supervisor = Host::start_supervisor(&host).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let heartbeat = {
        let host = Arc::clone(&host);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut worst = Duration::ZERO;
            let mut seen = Vec::new();
            let mut last = Instant::now();
            while !stop.load(Ordering::SeqCst) {
                for event in host.take_events() {
                    seen.push((started.elapsed(), event));
                }
                thread::sleep(Duration::from_millis(10));
                worst = worst.max(last.elapsed());
                last = Instant::now();
            }
            (worst, seen)
        })
    };
    for round in 1..=2 {
        let t = Instant::now();
        let outcome = host.call("fake", methods::COMMAND_RUN, json!({"command": "echo"}));
        println!(
            "S2 round {round}: the caller got {:?} after {} ms",
            outcome.err().map(|e| e.to_string()),
            t.elapsed().as_millis()
        );
    }
    thread::sleep(Duration::from_millis(500));
    println!(
        "S2: running after the second time-out: {}",
        host.is_running("fake")
    );
    stop.store(true, Ordering::SeqCst);
    let (worst, seen) = heartbeat.join().unwrap();
    println!(
        "S2: the loop's longest gap {} ms (Pass at 50 ms or less: {})",
        worst.as_millis(),
        worst <= Duration::from_millis(60)
    );
    for (at, event) in seen {
        let text = match &event {
            HostEvent::Waiting { message, .. }
            | HostEvent::TimedOut { message, .. }
            | HostEvent::Crashed { message, .. } => message.text.clone(),
            other => format!("{other:?}"),
        };
        println!("S2: at {} ms: {text}", at.as_millis());
    }
    host.shutdown_all(Duration::from_secs(2));
}

const PYTHON: &str = "import sys\nsys.stdin.readline()\nsys.stdout.write('{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocol\":1}}\\n')\nsys.stdout.flush()\nsys.stdin.readline()\n";

const BASH: &str = "IFS= read -r line\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocol\":1}}'\nIFS= read -r line\n";

const PWSH: &str = "$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocol\":1}}')\n[Console]::Out.Flush()\n$null = [Console]::In.ReadLine()\n";

fn measure(label: &str, program: &std::path::Path, args: &[String], starts: usize) {
    let mut times = Vec::new();
    let params = InitializeParams::for_plugin("latency");
    for _ in 0..starts {
        let t = Instant::now();
        let spec = ProcessSpec::new(program).args(args).diroxide_environment();
        let transport =
            match StdioTransport::start(label, spec, PluginLog::in_memory(), MAX_LINE_BYTES) {
                Ok(transport) => transport,
                Err(e) => {
                    println!("S9 {label}: could not start: {e}");
                    return;
                }
            };
        match handshake(&transport, label, &params, Duration::from_secs(30)) {
            Ok(_) => times.push(t.elapsed()),
            Err(e) => println!("S9 {label}: {e}"),
        }
        let _ = transport.kill();
    }
    times.sort();
    if times.is_empty() {
        return;
    }
    println!(
        "S9 {label}: {} starts, median {} ms, worst {} ms",
        times.len(),
        times[times.len() / 2].as_millis(),
        times[times.len() - 1].as_millis()
    );
}

#[test]
#[ignore = "spike S9: needs Python, bash and PowerShell; minutes of real time; run by hand"]
fn spike_s9_start_latency() {
    let folder = tempfile::tempdir().unwrap();
    for (name, text) in [("a.py", PYTHON), ("a.sh", BASH), ("a.ps1", PWSH)] {
        fs::write(folder.path().join(name), text).unwrap();
    }
    let script = |name: &str| folder.path().join(name).display().to_string();
    let path = env::var_os("PATH");
    let path = path.as_deref();
    let starts = 100;
    if cfg!(windows) {
        if let Some(py) = which("py", path) {
            measure("py -3", &py, &["-3".into(), script("a.py")], starts);
        }
        let bash = which("git", path).and_then(|git| {
            git.ancestors()
                .skip(1)
                .take(3)
                .map(|f| f.join("bin").join("bash.exe"))
                .find(|b| b.is_file())
        });
        if let Some(bash) = bash {
            measure("bash", &bash, &[script("a.sh")], starts);
        }
        for shell in ["pwsh", "powershell"] {
            if let Some(ps) = which(shell, path) {
                measure(
                    shell,
                    &ps,
                    &[
                        "-NoLogo".into(),
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-File".into(),
                        script("a.ps1"),
                    ],
                    starts,
                );
            }
        }
    } else {
        for (label, program, file) in [("python3", "python3", "a.py"), ("bash", "bash", "a.sh")] {
            if let Some(found) = which(program, path) {
                measure(label, &found, &[script(file)], starts);
            }
        }
    }
}
