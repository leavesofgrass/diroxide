//! Provider connect, disconnect and lost (R25 commit 57): the capability
//! table against the fake server's extension lists, one connection per
//! target, a lost link said once, a quiet disconnect; and through the
//! fake ssh, a changed key refused, a cancel that ends ssh, and an ssh
//! that dies.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diro_sftp::plugin::Plugin;
use diro_sftp::provider::SshOpener;
use diro_sftp::provider::fake::FakeOpener;
use diro_sftp::sftp::fake::{FakeFs, Faults};
use serde_json::{Value, json};

use crate::fakes::{fake_ssh_in, log_lines};
use crate::host::{TestHost, answer_of, kind};
use crate::plugin_exe;

pub const G_OUTPUT: &str = include_str!("../../../../fixtures/sftp/ssh-g/windows-9.5p2-plain.txt");
const CHANGED: &str =
    include_str!("../../../../fixtures/sftp/stderr/changed-host-key-windows-9.5p2.txt");

/// A host whose plugin opens sessions on the fake server in memory.
pub fn fake_host(faults: Faults) -> (TestHost, Arc<FakeOpener>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let opener = FakeOpener::new(Arc::new(Mutex::new(FakeFs::default())), faults);
    let host = TestHost::start(
        Plugin::with_opener(opener.clone()),
        root.path(),
        json!({}),
        Vec::new(),
    );
    (host, opener, root)
}

/// A host whose plugin starts the fake ssh with `script`, and the fake's
/// log.
pub fn ssh_host(
    script: Value,
    answers: Vec<Value>,
) -> (TestHost, std::path::PathBuf, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("ssh.log");
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"log": log, "g_output": G_OUTPUT, "script": script}),
    );
    let host = TestHost::start(
        Plugin::with_opener(Arc::new(SshOpener::with_helper(plugin_exe()))),
        root.path(),
        json!({"ssh_program": ssh, "connect_timeout": 5}),
        answers,
    );
    (host, log, root)
}

pub fn connect(host: &TestHost, place: &str) -> Result<Value, Value> {
    host.call(
        "provider/connect",
        json!({"place": place, "interactive": true}),
    )
}

#[test]
fn capabilities_follow_the_servers_extensions() {
    for (faults, copy, rename, space) in [
        (Faults::default(), false, true, true),
        (
            Faults {
                copy_data: true,
                ..Faults::default()
            },
            true,
            true,
            true,
        ),
        (
            Faults {
                no_posix_rename: true,
                ..Faults::default()
            },
            false,
            false,
            true,
        ),
        (
            Faults {
                no_statvfs: true,
                ..Faults::default()
            },
            false,
            true,
            false,
        ),
    ] {
        let (host, _, _root) = fake_host(faults.clone());
        let answer = connect(&host, "sftp://build.example/").unwrap();
        let caps = &answer["capabilities"];
        assert_eq!(caps["server_side_copy"], copy, "{faults:?}");
        assert_eq!(caps["atomic_rename"], rename, "{faults:?}");
        assert_eq!(caps["free_space"], space, "{faults:?}");
        assert_eq!(caps["resume"], true);
        assert_eq!(caps["trash"], false);
        assert_eq!(answer["root"], "sftp://build.example/home/ada");
        assert_eq!(answer["display_name"], "build.example");
    }
}

#[test]
fn one_connection_a_target_and_none_without_connect() {
    let (host, opener, _root) = fake_host(Faults::default());
    let before = host
        .call("provider/stat", json!({"place": "sftp://build.example/x"}))
        .unwrap_err();
    assert_eq!(kind(&before), "auth_required");
    assert_eq!(before["message"], "Not connected to build.example.");
    connect(&host, "sftp://build.example/").unwrap();
    connect(&host, "sftp://build.example/srv").unwrap();
    assert_eq!(opener.openings().len(), 1);
    // Another account on the same host is another connection.
    connect(&host, "sftp://ada@build.example/").unwrap();
    assert_eq!(opener.openings().len(), 2);
    assert_eq!(host.plugin().provider().connected().len(), 2);
}

#[test]
fn a_refused_open_says_it_cannot_connect() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.refuse_next(1);
    let refused = connect(&host, "sftp://build.example/").unwrap_err();
    assert_eq!(kind(&refused), "unavailable");
    assert_eq!(refused["message"], "Cannot connect to build.example.");
    assert!(host.plugin().provider().connected().is_empty());
    connect(&host, "sftp://build.example/").unwrap();
}

#[test]
fn a_cut_link_is_said_once_and_the_connection_forgotten() {
    let (host, opener, _root) = fake_host(Faults::default());
    // A connection that needed a typed secret is not reopened unasked.
    opener.as_if_typed(true);
    connect(&host, "sftp://build.example/").unwrap();
    opener.cut_all();
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(Duration::from_millis(400));
    let lost = host.notices("provider/lost");
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0]["authority"], "sftp://build.example");
    assert_eq!(lost[0]["reason"], "Connection lost: build.example.");
    let after = host
        .call("provider/stat", json!({"place": "sftp://build.example/x"}))
        .unwrap_err();
    assert_eq!(kind(&after), "auth_required");
    assert!(host.plugin().provider().connected().is_empty());
}

#[test]
fn disconnect_closes_quietly() {
    let (host, _, _root) = fake_host(Faults::default());
    connect(&host, "sftp://build.example/").unwrap();
    host.notify(
        "provider/disconnect",
        json!({"place": "sftp://build.example/"}),
    );
    assert!(host.wait_for(|h| h.plugin().provider().connected().is_empty()));
    std::thread::sleep(Duration::from_millis(300));
    assert!(host.notices("provider/lost").is_empty());
}

#[test]
fn ssh_starts_with_the_subsystem_and_dies_said_once() {
    let (host, log, _root) = ssh_host(json!([{"serve": {"for_ms": 1500}}]), Vec::new());
    let answer = connect(&host, "sftp://build.example/").unwrap();
    assert_eq!(answer["capabilities"]["atomic_rename"], true);
    assert_eq!(answer["ssh"], "OpenSSH 9.9");
    let run = log_lines(&log)
        .into_iter()
        .find(|l| {
            l["args"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "sftp"))
        })
        .unwrap();
    let args: Vec<&str> = run["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(&args[args.len() - 3..], ["--", "build.example", "sftp"]);
    assert!(args.contains(&"BatchMode=no"));
    assert_eq!(run["env"]["SSH_ASKPASS_REQUIRE"], "force");
    // The fake ssh ends after a while, as a dying ssh does.
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(Duration::from_millis(400));
    assert!(host.notices("provider/lost").len() <= 1);
}

#[test]
fn a_changed_host_key_is_refused_with_the_line_to_forget_it() {
    let (host, _log, _root) = ssh_host(json!([{"stderr": CHANGED}, {"exit": 255}]), Vec::new());
    let refused = connect(&host, "sftp://build.example/").unwrap_err();
    assert_eq!(kind(&refused), "untrusted");
    assert_eq!(refused["message"], "Host key changed. Not connected.");
    let hint = refused["data"]["hint"].as_str().unwrap();
    assert!(hint.ends_with("ssh-keygen -R build.example"), "{hint}");
    assert!(host.plugin().provider().connected().is_empty());
}

#[test]
fn a_cancelled_connect_ends_ssh_at_once() {
    let (host, _log, _root) = ssh_host(json!([{"sleep_ms": 30_000}]), Vec::new());
    let (id, rx) = host.send_request(
        "provider/connect",
        json!({"place": "sftp://build.example/", "interactive": true}),
    );
    std::thread::sleep(Duration::from_millis(700));
    let began = Instant::now();
    host.cancel(id);
    let answer = answer_of(&rx).unwrap_err();
    assert_eq!(kind(&answer), "request_cancelled");
    assert!(began.elapsed() < Duration::from_secs(10));
}

#[test]
fn ssh_s_password_prompt_is_asked_with_the_account_and_host() {
    let (host, log, _root) = ssh_host(
        json!([
            {"ask": "ada@build.example's password: ", "accept": "s3cret-test"},
            {"serve": {}}
        ]),
        vec![json!({"text": "s3cret-test"})],
    );
    connect(&host, "sftp://build.example/").unwrap();
    let asked = host.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["kind"], "secret");
    assert_eq!(asked[0]["title"], "Password for ada at build.example");
    // The fake ssh logs whether the answer matched, never the answer.
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(!text.contains("s3cret-test"));
    assert!(log_lines(&log).iter().any(|l| l["matched"] == true));
    let connection = host
        .plugin()
        .provider()
        .connection(&diro_sftp::ssh::Target::new("build.example", None, None).unwrap())
        .unwrap();
    assert!(connection.memory().typed());
    assert!(connection.memory().has_password());
}
