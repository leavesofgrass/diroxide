//! The program's two modes, chosen by the environment (answer 82).

use std::io::Write;
use std::process::{Command, Stdio};

use crate::plugin_exe;

/// With `DIRO_ASKPASS_ENDPOINT` set the program is the helper: it does
/// not speak JSON-RPC, and when it cannot reach a plugin it writes
/// nothing and exits non-zero, which ssh takes as no answer.
#[test]
fn the_endpoint_variable_makes_the_program_the_helper() {
    let mut child = Command::new(plugin_exe())
        .arg("user@build.example's password: ")
        .env("DIRO_ASKPASS_ENDPOINT", "tcp:127.0.0.1:1")
        .env("DIRO_ASKPASS_TOKEN", "0000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocol\":1}}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    assert!(out.stderr.is_empty(), "{:?}", out.stderr);
}

/// Without it the program is the plugin: it answers `initialize`.
#[test]
fn without_it_the_program_is_the_plugin() {
    let mut child = Command::new(plugin_exe())
        .env_remove("DIRO_ASKPASS_ENDPOINT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocol\":1}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"shutdown\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}\n",
            )
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["result"]["plugin"]["name"], "SFTP places");
}
