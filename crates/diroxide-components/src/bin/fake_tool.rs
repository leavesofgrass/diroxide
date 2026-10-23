//! A stand-in for curl and wget in the tests (the `fake-tools` feature):
//! copied into a test folder as `curl` or `wget` and put on a test PATH,
//! it records its arguments and standard input beside itself, prints the
//! bytes it was given, and exits with the code it was given. It never
//! opens a connection.
//!
//! Beside the program, for a program named NAME: `NAME-args.txt` gets one
//! argument a line, `NAME-stdin.txt` what standard input held; it prints
//! `NAME-stdout.bin` and `NAME-stderr.txt` when they are there and exits
//! with the number in `NAME-exit.txt`, 0 when there is none.

use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        return ExitCode::from(90);
    };
    let (Some(folder), Some(name)) = (exe.parent(), exe.file_stem()) else {
        return ExitCode::from(91);
    };
    let name = name.to_string_lossy().into_owned();
    let file = |suffix: &str| folder.join(format!("{name}-{suffix}"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = fs::write(file("args.txt"), args.join("\n"));
    let mut stdin = Vec::new();
    let _ = io::stdin().read_to_end(&mut stdin);
    let _ = fs::write(file("stdin.txt"), &stdin);
    if let Ok(bytes) = fs::read(file("stdout.bin")) {
        let _ = io::stdout().write_all(&bytes);
    }
    if let Ok(text) = fs::read_to_string(file("stderr.txt")) {
        let _ = io::stderr().write_all(text.as_bytes());
    }
    let code = fs::read_to_string(file("exit.txt"))
        .ok()
        .and_then(|t| t.trim().parse::<u8>().ok())
        .unwrap_or(0);
    ExitCode::from(code)
}
