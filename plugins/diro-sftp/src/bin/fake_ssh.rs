//! The fake ssh of the test plan (R25 commit 55): a program the plugin's
//! tests start in place of OpenSSH's `ssh`. Built only with the
//! `fake-ssh` feature.
//!
//! **As ssh.** A test copies this program into a folder of its own as
//! `ssh` (or `ssh.exe`) and puts a `fake-ssh.json` beside it; the program
//! reads that file, never its environment, so tests need not change the
//! test process's environment:
//!
//! - `version`: what `-V` prints to standard error.
//! - `log`: a file to which every run appends one JSON line: the program's
//!   own path, its arguments, and which askpass variables it was given
//!   (values only for the ones that hold no secret).
//! - `g_output`: what `-G` prints to standard output.
//! - `script`: what a connection does, step by step: `{"ask": PROMPT}`
//!   runs `SSH_ASKPASS` with the prompt as ssh does and logs whether it
//!   answered and its exit code (`"accept"` names the one answer that
//!   counts as right, and `"tries"` how many times a prompt is asked
//!   before ssh gives up); `{"stderr": TEXT}`, `{"exit": CODE}` and
//!   `{"sleep_ms": N}` do what they say; `{"child_sleep_ms": N}` starts a
//!   copy of the program that sleeps, as ssh's own children would, and
//!   logs its process id.
//!
//! **As an askpass recorder** (spike SF1): when `FAKE_ASKPASS_RECORD`
//! names a file, the program is an `SSH_ASKPASS` stand-in that appends one
//! JSON line describing how it was started (its arguments, the askpass
//! variables, whether it has a console) to that file, waits
//! `FAKE_ASKPASS_PAUSE_MS`, prints `FAKE_ASKPASS_ANSWER` (default "no")
//! and exits 0.
//!
//! It never logs an answer, only whether one came and whether it matched.

use std::env;
use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// Whether this process is attached to a console: Windows opens the
/// console's output only for a process that has one.
fn has_console() -> bool {
    if cfg!(windows) {
        OpenOptions::new().write(true).open("CONOUT$").is_ok()
    } else {
        OpenOptions::new().read(true).open("/dev/tty").is_ok()
    }
}

fn append(file: &str, line: &Value) {
    if let Ok(mut out) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(out, "{line}");
    }
}

fn record(file: &str) -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let var = |name: &str| env::var(name).ok();
    append(
        file,
        &json!({
            "pid": std::process::id(),
            "args": args,
            "ssh_askpass_prompt": var("SSH_ASKPASS_PROMPT"),
            "ssh_askpass_require": var("SSH_ASKPASS_REQUIRE"),
            "display": var("DISPLAY"),
            "has_console": has_console(),
            "stdin_is_terminal": io::stdin().is_terminal(),
            "stdout_is_terminal": io::stdout().is_terminal(),
            "stderr_is_terminal": io::stderr().is_terminal(),
        }),
    );
    let pause = var("FAKE_ASKPASS_PAUSE_MS")
        .and_then(|ms| ms.parse::<u64>().ok())
        .unwrap_or(0);
    thread::sleep(Duration::from_millis(pause));
    let answer = var("FAKE_ASKPASS_ANSWER").unwrap_or_else(|| "no".to_owned());
    let mut stdout = io::stdout();
    let _ = writeln!(stdout, "{answer}");
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

fn config() -> Value {
    let Some(dir) = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    else {
        return Value::Null;
    };
    std::fs::read_to_string(dir.join("fake-ssh.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// The askpass variables ssh would pass on, for the log: values only for
/// those that hold no secret.
fn askpass_env() -> Value {
    let var = |name: &str| env::var(name).ok();
    json!({
        "SSH_ASKPASS": var("SSH_ASKPASS"),
        "SSH_ASKPASS_REQUIRE": var("SSH_ASKPASS_REQUIRE"),
        "DISPLAY": var("DISPLAY"),
        "DIRO_ASKPASS_ENDPOINT": var("DIRO_ASKPASS_ENDPOINT").is_some(),
        "DIRO_ASKPASS_TOKEN": var("DIRO_ASKPASS_TOKEN").is_some(),
    })
}

/// Runs `SSH_ASKPASS` with a prompt as ssh does: the prompt is its one
/// argument and its answer is its standard output.
fn ask(prompt: &str) -> (Option<i32>, Option<String>) {
    let Ok(helper) = env::var("SSH_ASKPASS") else {
        return (None, None);
    };
    let output = Command::new(helper)
        .arg(prompt)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            (out.status.code(), Some(text))
        }
        Err(_) => (None, None),
    }
}

fn run_script(script: &[Value], log: Option<&str>) -> ExitCode {
    let mut stderr = io::stderr();
    for step in script {
        if let Some(prompt) = step.get("ask").and_then(Value::as_str) {
            let accept = step.get("accept").and_then(Value::as_str);
            let tries = step.get("tries").and_then(Value::as_u64).unwrap_or(1);
            let mut matched = false;
            for attempt in 1..=tries {
                let (code, answer) = ask(prompt);
                matched = match accept {
                    Some(right) => answer.as_deref() == Some(right),
                    None => code == Some(0),
                };
                if let Some(log) = log {
                    append(
                        log,
                        &json!({
                            "asked": prompt,
                            "attempt": attempt,
                            "exit": code,
                            "answered": answer.as_ref().is_some_and(|a| !a.is_empty()),
                            "answer_word": answer
                                .as_deref()
                                .filter(|a| ["yes", "no"].contains(a)),
                            "matched": matched,
                        }),
                    );
                }
                if matched || code != Some(0) {
                    break;
                }
            }
            if !matched {
                let refused = step
                    .get("refused")
                    .and_then(Value::as_str)
                    .unwrap_or("Permission denied.");
                let _ = writeln!(stderr, "{refused}");
                return ExitCode::from(255);
            }
        } else if let Some(text) = step.get("stderr").and_then(Value::as_str) {
            let _ = writeln!(stderr, "{text}");
        } else if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            thread::sleep(Duration::from_millis(ms));
        } else if let Some(ms) = step.get("child_sleep_ms").and_then(Value::as_u64) {
            let child = env::current_exe().ok().and_then(|exe| {
                Command::new(exe)
                    .args(["--sleep", &ms.to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()
            });
            if let (Some(log), Some(child)) = (log, child) {
                append(log, &json!({"child": child.id()}));
            }
        } else if let Some(code) = step.get("exit").and_then(Value::as_u64) {
            return ExitCode::from(u8::try_from(code).unwrap_or(255));
        } else if step.get("drain").is_some() {
            // Waits for standard input to end, as a session does.
            let mut sink = Vec::new();
            let _ = io::stdin().read_to_end(&mut sink);
        }
    }
    ExitCode::SUCCESS
}

/// `--relay PROGRAM ARGS...`: starts ssh with the plugin's own launcher,
/// as the plugin does, its askpass helper taken from this program's
/// `SSH_ASKPASS`, and ends with ssh's exit code, or 99 when it has not
/// ended in 15 seconds: a spike's stand-in for the plugin inside the
/// host's jobs.
fn relay(args: &[String]) -> ExitCode {
    use diro_sftp::ssh::{AskpassEnv, Launch};

    let Some((program, rest)) = args.split_first() else {
        return ExitCode::from(2);
    };
    let launch = Launch {
        program: PathBuf::from(program),
        args: rest.iter().map(Into::into).collect(),
        askpass: env::var_os("SSH_ASKPASS").map(|helper| AskpassEnv {
            helper: PathBuf::from(helper),
            endpoint: "tcp:127.0.0.1:9".to_owned(),
            token: "00".to_owned(),
        }),
    };
    let Ok(mut ssh) = launch.spawn() else {
        return ExitCode::from(3);
    };
    match ssh.wait_timeout(Duration::from_secs(15)) {
        Some(status) => {
            ExitCode::from(u8::try_from(status.code().unwrap_or(4) & 0xff).unwrap_or(4))
        }
        None => {
            ssh.kill();
            ExitCode::from(99)
        }
    }
}

fn main() -> ExitCode {
    if env::args().nth(1).as_deref() == Some("--relay") {
        let args: Vec<String> = env::args().skip(2).collect();
        return relay(&args);
    }
    if let Ok(file) = env::var("FAKE_ASKPASS_RECORD") {
        return record(&file);
    }
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--sleep") {
        let ms = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(0);
        thread::sleep(Duration::from_millis(ms));
        return ExitCode::SUCCESS;
    }
    let config = config();
    let log = config.get("log").and_then(Value::as_str);
    if let Some(log) = log {
        let exe = env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        append(
            log,
            &json!({"exe": exe, "args": args, "env": askpass_env()}),
        );
    }
    if args.first().map(String::as_str) == Some("-V") {
        let version = config
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("OpenSSH_9.9p1, fake");
        let _ = writeln!(io::stderr(), "{version}");
        return ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) == Some("-G") {
        let text = config.get("g_output").and_then(Value::as_str).unwrap_or("");
        let _ = io::stdout().write_all(text.as_bytes());
        return ExitCode::SUCCESS;
    }
    if let Some(script) = config.get("script").and_then(Value::as_array) {
        return run_script(script, log);
    }
    let _ = writeln!(io::stderr(), "diro-fake-ssh: no fake-ssh.json beside it");
    ExitCode::from(2)
}
