//! The fake ssh of the test plan (R25 commit 55): a program the plugin's
//! tests start in place of OpenSSH's `ssh`. Built only with the
//! `fake-ssh` feature.
//!
//! As an askpass recorder (spike SF1): when `FAKE_ASKPASS_RECORD` names a
//! file, the program is an `SSH_ASKPASS` stand-in that appends one JSON
//! line describing how it was started (its arguments, the askpass
//! variables, whether it has a console) to that file, waits
//! `FAKE_ASKPASS_PAUSE_MS`, prints `FAKE_ASKPASS_ANSWER` (default "no")
//! and exits 0. It never reads a secret and never writes one.

use std::env;
use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

/// Whether this process is attached to a console: Windows opens the
/// console's output only for a process that has one.
fn has_console() -> bool {
    if cfg!(windows) {
        OpenOptions::new().write(true).open("CONOUT$").is_ok()
    } else {
        OpenOptions::new().read(true).open("/dev/tty").is_ok()
    }
}

fn record(file: &str) -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let var = |name: &str| env::var(name).ok();
    let line = serde_json::json!({
        "pid": std::process::id(),
        "args": args,
        "ssh_askpass_prompt": var("SSH_ASKPASS_PROMPT"),
        "ssh_askpass_require": var("SSH_ASKPASS_REQUIRE"),
        "display": var("DISPLAY"),
        "has_console": has_console(),
        "stdin_is_terminal": io::stdin().is_terminal(),
        "stdout_is_terminal": io::stdout().is_terminal(),
        "stderr_is_terminal": io::stderr().is_terminal(),
    });
    if let Ok(mut out) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(out, "{line}");
    }
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

fn main() -> ExitCode {
    if let Ok(file) = env::var("FAKE_ASKPASS_RECORD") {
        return record(&file);
    }
    let mut err = io::stderr();
    let _ = writeln!(err, "diro-fake-ssh: no mode was chosen");
    ExitCode::from(2)
}
