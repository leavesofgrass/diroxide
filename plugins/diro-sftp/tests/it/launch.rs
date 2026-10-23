//! The option builder and the launcher (R25 commit 50): a fake ssh that
//! echoes its arguments and environment, a bounded standard error, and a
//! kill that leaves no process behind.

use std::process::Command;
use std::time::{Duration, Instant};

use diro_sftp::settings::Settings;
use diro_sftp::ssh::options::{Remote, SessionOptions};
use diro_sftp::ssh::{AskpassEnv, Launch, Resolved, SshVersion, Target};
use serde_json::{Value, json};

use crate::fakes::{fake_ssh_in, log_lines};
use crate::plugin_exe;

const WINDOWS_G: &str = include_str!("../../../../fixtures/sftp/ssh-g/windows-9.5p2-plain.txt");

/// Whether a process is still running, asked of the system with no
/// console window.
fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success())
    }
}

fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn arguments_and_environment_reach_ssh_as_given() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("ssh.log");
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"log": log, "script": [{"exit": 0}]}),
    );
    let target = Target::new("build.example", Some("ada"), Some(2222)).unwrap();
    let resolved = Resolved::parse(WINDOWS_G);
    let version = SshVersion::parse("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2").unwrap();
    let settings = Settings::default();
    let args = SessionOptions {
        target: &target,
        resolved: &resolved,
        version: &version,
        settings: &settings,
        interactive: true,
        windows: cfg!(windows),
        key_file: None,
        add_keys_to_agent: false,
        debug_log: None,
        remote: Remote::Subsystem,
    }
    .arguments();
    let token = "5ec2e7c0ffee";
    let mut process = Launch {
        program: ssh,
        args: args.clone(),
        askpass: Some(AskpassEnv {
            helper: plugin_exe(),
            endpoint: "tcp:127.0.0.1:9".to_owned(),
            token: token.to_owned(),
        }),
    }
    .spawn()
    .unwrap();
    assert!(process.wait_timeout(Duration::from_secs(10)).is_some());
    let runs = log_lines(&log);
    let given: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(runs[0]["args"], json!(given));
    let n = given.len();
    assert_eq!(&given[n - 3..], ["--", "build.example", "sftp"]);
    assert_eq!(
        given.contains(&"ControlMaster=no".to_owned()),
        cfg!(windows)
    );
    let env = &runs[0]["env"];
    assert_eq!(env["SSH_ASKPASS_REQUIRE"], "force");
    assert_eq!(
        env["SSH_ASKPASS"],
        Value::String(plugin_exe().display().to_string())
    );
    assert_eq!(env["DIRO_ASKPASS_TOKEN"], true);
    assert!(
        !std::fs::read_to_string(&log).unwrap().contains(token),
        "the token reached a log"
    );
}

#[test]
fn standard_error_is_kept_in_a_bounded_ring() {
    let root = tempfile::tempdir().unwrap();
    let noise = "x".repeat(1000);
    let mut script: Vec<Value> = (0..100).map(|_| json!({"stderr": noise})).collect();
    script
        .push(json!({"stderr": "ssh: connect to host build.example port 22: Connection refused"}));
    script.push(json!({"exit": 255}));
    let ssh = fake_ssh_in(&root.path().join("bin"), &json!({"script": script}));
    let mut process = Launch {
        program: ssh,
        args: vec!["--".into(), "build.example".into()],
        askpass: None,
    }
    .spawn()
    .unwrap();
    let status = process.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(status.code(), Some(255));
    assert!(wait_for(|| process.stderr_ended()));
    let text = process.stderr_text();
    assert!(text.len() <= diro_sftp::ssh::launch::STDERR_RING);
    assert!(
        text.trim_end().ends_with("Connection refused"),
        "{}",
        &text[text.len() - 80..]
    );
}

/// The kill ends ssh and everything it started, on every system: the
/// helper it started must not outlive it.
#[test]
fn the_kill_leaves_no_process() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("ssh.log");
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"log": log, "script": [{"child_sleep_ms": 60000}, {"sleep_ms": 60000}]}),
    );
    let mut process = Launch {
        program: ssh,
        args: vec!["--".into(), "build.example".into(), "sftp".into()],
        askpass: None,
    }
    .spawn()
    .unwrap();
    let pid = process.id();
    let mut child = 0;
    assert!(wait_for(|| {
        child = log_lines(&log)
            .iter()
            .find_map(|l| l["child"].as_u64())
            .unwrap_or(0) as u32;
        child != 0
    }));
    assert!(alive(pid) && alive(child));
    process.kill();
    assert!(wait_for(|| !alive(pid)), "ssh outlived the kill");
    assert!(wait_for(|| !alive(child)), "ssh's child outlived the kill");
}
