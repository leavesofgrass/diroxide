//! Spike S1 of `research/alpha4-plan.md` (section 10): line-delimited
//! JSON over real pipes with the interpreters the owner automates in.
//! Ignored in normal runs because it needs Python, bash and PowerShell 7;
//! run it by hand on each system with
//! `cargo nextest run -p diroxide-plugins --run-ignored only spike_s1 --no-capture`,
//! and record what it prints in the builder's report.
//!
//! Each interpreter runs a tiny echo plugin through the real stdio
//! transport and process helper: non-ASCII names, a 1 MiB line, 10,000
//! small lines, a 9 MiB line (refused, and the next line still read), and
//! for Python stray bytes from 0x80 to 0xFF. Pass: every message arrives
//! byte-exact and nothing deadlocks.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use diroxide_plugins::framing::Frame;
use diroxide_plugins::log::PluginLog;
use diroxide_plugins::protocol::MAX_LINE_BYTES;
use diroxide_plugins::{PluginError, PluginTransport, StdioTransport};
use diroxide_tools::process::ProcessSpec;
use diroxide_tools::which;

const WAIT: Duration = Duration::from_secs(60);

const PYTHON: &str = r#"import sys, json
for line in sys.stdin:
    message = json.loads(line)
    if message.get("stray"):
        sys.stdout.flush()
        sys.stdout.buffer.write(bytes(range(0x80, 0x100)) + b"\n")
        sys.stdout.buffer.flush()
        continue
    print(json.dumps(message, ensure_ascii=False, separators=(",", ":")))
    sys.stdout.flush()
"#;

const BASH: &str = r#"while IFS= read -r line; do printf '%s\n' "$line"; done
"#;

const PWSH_PLAIN: &str = r#"while ($null -ne ($line = [Console]::In.ReadLine())) {
    [Console]::Out.WriteLine($line)
    [Console]::Out.Flush()
}
"#;

const PWSH_UTF8: &str = r#"$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
while ($null -ne ($line = [Console]::In.ReadLine())) {
    [Console]::Out.WriteLine($line)
    [Console]::Out.Flush()
}
"#;

fn next_message(transport: &dyn PluginTransport) -> Result<String, String> {
    loop {
        match transport.receive(WAIT) {
            Ok(Some(Frame::Message(text))) => return Ok(text),
            Ok(Some(Frame::Stray(_) | Frame::Blank)) => continue,
            Ok(Some(other)) => return Err(format!("{other:?}")),
            Ok(None) => return Err("no answer in 60 seconds".into()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn send(transport: &dyn PluginTransport, line: &str) {
    loop {
        match transport.send(line.to_owned()) {
            Err(PluginError::Limit(_)) => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => panic!("{e}"),
            Ok(()) => return,
        }
    }
}

/// Runs the checks against one echo plugin and prints a line per check.
fn exercise(label: &str, program: PathBuf, args: &[&str], folder: &Path, python: bool) {
    let started = Instant::now();
    let spec = ProcessSpec::new(&program)
        .args(args)
        .cwd(folder)
        .diroxide_environment();
    let transport = match StdioTransport::start(label, spec, PluginLog::in_memory(), MAX_LINE_BYTES)
    {
        Ok(transport) => transport,
        Err(e) => {
            println!("{label}: could not start: {e}");
            return;
        }
    };
    let names = "{\"name\":\"caf\u{e9}, \u{65e5}\u{672c}, \u{1F980}, \u{05D0}\"}";
    send(&transport, names);
    let first = next_message(&transport);
    println!(
        "{label}: first answer after {} ms",
        started.elapsed().as_millis()
    );
    println!(
        "{label}: non-ASCII names {}",
        match &first {
            Ok(text) if text == names => "Pass, byte-exact".to_owned(),
            Ok(text) => format!("Fail, got {text}"),
            Err(e) => format!("Fail, {e}"),
        }
    );
    let mib = format!("{{\"data\":\"{}\"}}", "m".repeat(1024 * 1024));
    let t = Instant::now();
    send(&transport, &mib);
    let back = next_message(&transport);
    println!(
        "{label}: 1 MiB line {} in {} ms",
        if back.as_deref() == Ok(mib.as_str()) {
            "Pass"
        } else {
            "Fail"
        },
        t.elapsed().as_millis()
    );
    let t = Instant::now();
    let count = 10_000;
    let mut exact = true;
    std::thread::scope(|scope| {
        let transport = &transport;
        scope.spawn(move || {
            for i in 0..count {
                send(transport, &format!("{{\"n\":{i}}}"));
            }
        });
        for i in 0..count {
            exact &= next_message(transport).as_deref() == Ok(format!("{{\"n\":{i}}}").as_str());
        }
    });
    println!(
        "{label}: {count} small lines {} in {} ms",
        if exact { "Pass" } else { "Fail" },
        t.elapsed().as_millis()
    );
    let over = format!("{{\"data\":\"{}\"}}", "o".repeat(9 * 1024 * 1024));
    send(&transport, &over);
    send(&transport, "{\"after\":true}");
    let mut refused = false;
    let after = loop {
        match transport.receive(WAIT) {
            Ok(Some(Frame::TooLong { .. })) => refused = true,
            Ok(Some(Frame::Message(text))) => break Ok(text),
            Ok(Some(_)) => continue,
            Ok(None) => break Err("no answer".to_owned()),
            Err(e) => break Err(e.to_string()),
        }
    };
    println!(
        "{label}: 9 MiB line refused {}, next line read {}",
        if refused { "Pass" } else { "Fail" },
        if after.as_deref() == Ok("{\"after\":true}") {
            "Pass"
        } else {
            "Fail"
        }
    );
    if python {
        send(&transport, "{\"stray\":true}");
        send(&transport, "{\"still\":\"here\"}");
        let mut stray = None;
        let still = loop {
            match transport.receive(WAIT) {
                Ok(Some(Frame::Stray(text))) => stray = Some(text),
                Ok(Some(Frame::Message(text))) => break Ok(text),
                Ok(Some(_)) => continue,
                Ok(None) => break Err("no answer".to_owned()),
                Err(e) => break Err(e.to_string()),
            }
        };
        println!(
            "{label}: stray bytes 0x80 to 0xFF {}, conversation goes on {}",
            if stray.is_some_and(|t| t.chars().all(|c| c == '\u{FFFD}')) {
                "Pass, lossy"
            } else {
                "Fail"
            },
            if still.as_deref() == Ok("{\"still\":\"here\"}") {
                "Pass"
            } else {
                "Fail"
            }
        );
    }
    let t = Instant::now();
    transport.close_input();
    let ended = matches!(transport.receive(WAIT), Err(PluginError::Stopped(_)));
    println!(
        "{label}: ends when its input ends {} in {} ms",
        if ended { "Pass" } else { "Fail" },
        t.elapsed().as_millis()
    );
}

#[test]
#[ignore = "spike S1: needs Python, bash and PowerShell 7; run by hand"]
fn spike_s1_interpreters_over_real_pipes() {
    let folder = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("echo.py", PYTHON),
        ("echo.sh", BASH),
        ("echo-plain.ps1", PWSH_PLAIN),
        ("echo-utf8.ps1", PWSH_UTF8),
    ] {
        fs::write(folder.path().join(name), text).unwrap();
    }
    let path = env::var_os("PATH");
    let path = path.as_deref();
    let script = |name: &str| folder.path().join(name).to_string_lossy().into_owned();
    if cfg!(windows) {
        match which("py", path) {
            Some(py) => exercise(
                "py -3",
                py,
                &["-3", &script("echo.py")],
                folder.path(),
                true,
            ),
            None => println!("py: not found"),
        }
    } else {
        match which("python3", path) {
            Some(python) => exercise(
                "python3",
                python,
                &[&script("echo.py")],
                folder.path(),
                true,
            ),
            None => println!("python3: not found"),
        }
    }
    // Git for Windows' bash, next to git, never the system folder's WSL
    // launcher (spike S6 decides the chooser).
    let bash = if cfg!(windows) {
        which("git", path).and_then(|git| {
            // git.exe is in cmd or in mingw64/bin; bash.exe is in bin.
            git.ancestors()
                .skip(1)
                .take(3)
                .map(|folder| folder.join("bin").join("bash.exe"))
                .find(|bash| bash.is_file())
        })
    } else {
        which("bash", path)
    };
    match bash {
        Some(bash) => {
            let shown = bash.display().to_string();
            exercise(
                &format!("bash ({shown})"),
                bash,
                &[&script("echo.sh")],
                folder.path(),
                false,
            );
        }
        None => println!("bash: not found"),
    }
    match which("pwsh", path) {
        Some(pwsh) => {
            for name in ["echo-plain.ps1", "echo-utf8.ps1"] {
                exercise(
                    &format!("pwsh {name}"),
                    pwsh.clone(),
                    &["-NoProfile", "-NonInteractive", "-File", &script(name)],
                    folder.path(),
                    false,
                );
            }
        }
        None => println!("pwsh: not found"),
    }
    // Windows PowerShell 5.1, without an execution-policy override (that
    // is the owner's call, R22 question 8).
    if cfg!(windows) {
        match which("powershell", path) {
            Some(powershell) => {
                for name in ["echo-plain.ps1", "echo-utf8.ps1"] {
                    exercise(
                        &format!("powershell {name}"),
                        powershell.clone(),
                        &["-NoProfile", "-NonInteractive", "-File", &script(name)],
                        folder.path(),
                        false,
                    );
                }
            }
            None => println!("powershell: not found"),
        }
    }
}
