//! Reconnect and keepalive (R25 commit 64): ServerAliveInterval 15 and
//! count 3 only when the owner's value is 0; one automatic reconnect,
//! asking nothing, only for a connection that needed no typed secret; a
//! typed-secret connection waits for the owner; "Cancel connection" kills
//! ssh and says so; the reason a lost link gives.

use std::time::{Duration, Instant};

use diro_sftp::sftp::fake::Faults;
use diro_sftp::ssh::Target;
use serde_json::{Value, json};

use crate::connect::{connect, fake_host, ssh_host};
use crate::fakes::log_lines;
use crate::host::{TestHost, answer_of, kind};

fn target() -> Target {
    Target::new("build.example", None, None).unwrap()
}

fn sftp_runs(log: &std::path::Path) -> Vec<Value> {
    log_lines(log)
        .into_iter()
        .filter(|l| {
            l["args"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "sftp"))
        })
        .collect()
}

fn args_of(run: &Value) -> Vec<String> {
    run["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect()
}

fn reconnected(host: &TestHost) -> bool {
    host.notices("ui/announce")
        .iter()
        .any(|n| n["text"] == "Reconnected to build.example.")
}

#[test]
fn a_dropped_link_is_reopened_once_asking_nothing() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener
        .fs()
        .lock()
        .unwrap()
        .add_file(b"/home/ada/a.txt", b"a");
    connect(&host, "sftp://build.example/").unwrap();
    opener.cut_all();
    assert!(host.wait_for(reconnected));
    let openings = opener.openings();
    assert_eq!(openings.len(), 2);
    assert!(openings[0].interactive);
    assert!(!openings[1].interactive, "a reconnect asks nothing");
    assert!(host.notices("provider/lost").is_empty());
    let said = host.notices("ui/announce");
    assert_eq!(said[0]["importance"], "status", "never takes focus");
    let listed = host
        .call(
            "provider/list",
            json!({"place": "sftp://build.example/home/ada"}),
        )
        .unwrap();
    assert_eq!(listed["entries"][0]["name"], "a.txt");
    // A second loss within the minute is said, not tried again.
    opener.cut_all();
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(opener.openings().len(), 2);
    assert_eq!(host.notices("provider/lost").len(), 1);
}

#[test]
fn a_typed_secret_connection_waits_for_the_owner() {
    let (host, opener, _root) = fake_host(Faults::default());
    opener.as_if_typed(true);
    connect(&host, "sftp://build.example/").unwrap();
    opener.cut_all();
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(opener.openings().len(), 1, "nothing was reopened");
    assert!(!reconnected(&host));
}

#[test]
fn a_reconnect_that_fails_is_said_once() {
    let (host, opener, _root) = fake_host(Faults::default());
    connect(&host, "sftp://build.example/").unwrap();
    opener.refuse_next(1);
    opener.cut_all();
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(opener.openings().len(), 2);
    let lost = host.notices("provider/lost");
    assert_eq!(lost.len(), 1);
    assert_eq!(lost[0]["reason"], "Connection lost: build.example.");
    let after = host
        .call("provider/stat", json!({"place": "sftp://build.example/x"}))
        .unwrap_err();
    assert_eq!(kind(&after), "auth_required");
}

#[test]
fn the_reconnect_runs_ssh_in_batch_mode_with_no_helper() {
    let (host, log, _root) = ssh_host(json!([{"serve": {"for_ms": 1200}}]), Vec::new());
    connect(&host, "sftp://build.example/").unwrap();
    assert!(host.wait_for(reconnected));
    let runs = sftp_runs(&log);
    assert_eq!(runs.len(), 2);
    assert!(args_of(&runs[0]).contains(&"BatchMode=no".to_owned()));
    assert!(args_of(&runs[1]).contains(&"BatchMode=yes".to_owned()));
    assert_eq!(runs[1]["env"]["SSH_ASKPASS"], Value::Null);
    assert_eq!(runs[1]["env"]["DIRO_ASKPASS_TOKEN"], false);
    // It dies again within the minute: said once, not tried a third time.
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sftp_runs(&log).len(), 2);
}

#[test]
fn keepalive_is_added_only_when_the_owner_set_none() {
    let root = tempfile::tempdir().unwrap();
    for (case, (interval, settings, wanted)) in [
        ("0", json!({}), Some("ServerAliveInterval=15")),
        (
            "0",
            json!({"keepalive_seconds": 10}),
            Some("ServerAliveInterval=10"),
        ),
        ("30", json!({}), None),
    ]
    .into_iter()
    .enumerate()
    {
        let folder = root.path().join(format!("case{case}"));
        let log = folder.join("ssh.log");
        let g_output = crate::connect::G_OUTPUT.replace(
            "serveraliveinterval 0",
            &format!("serveraliveinterval {interval}"),
        );
        let ssh = crate::fakes::fake_ssh_in(
            &folder.join("bin"),
            &json!({"log": log, "g_output": g_output, "script": [{"serve": {}}]}),
        );
        let mut settings = settings;
        settings["ssh_program"] = json!(ssh);
        let host = TestHost::start(
            diro_sftp::plugin::Plugin::with_opener(std::sync::Arc::new(
                diro_sftp::provider::SshOpener::with_helper(crate::plugin_exe()),
            )),
            &folder,
            settings,
            Vec::new(),
        );
        connect(&host, "sftp://build.example/").unwrap();
        let args = args_of(&sftp_runs(&log)[0]);
        match wanted {
            Some(interval) => {
                assert!(args.contains(&interval.to_owned()), "{args:?}");
                assert!(args.contains(&"ServerAliveCountMax=3".to_owned()));
            }
            None => {
                assert!(
                    !args.iter().any(|a| a.starts_with("ServerAlive")),
                    "the owner's value stands: {args:?}"
                );
            }
        }
        host.plugin().provider().close_all();
    }
}

#[test]
fn cancel_connection_kills_a_stuck_ssh_and_says_so() {
    // The server stops answering after the handshake: a stuck link.
    let (host, _log, _root) = ssh_host(
        json!([
            {"ask": "ada@build.example's password: ", "accept": "s3cret-test"},
            {"serve": {"faults": {"stall_after": 2}}}
        ]),
        vec![json!({"text": "s3cret-test"})],
    );
    connect(&host, "sftp://build.example/").unwrap();
    let (_, stuck) = host.send_request(
        "provider/list",
        json!({"place": "sftp://build.example/home/ada"}),
    );
    std::thread::sleep(Duration::from_millis(500));
    let began = Instant::now();
    assert!(host.plugin().provider().cancel_connection(&target()));
    let answer = answer_of(&stuck).unwrap_err();
    assert_eq!(kind(&answer), "unavailable");
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "the list ended at once"
    );
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    let lost = host.notices("provider/lost");
    assert_eq!(lost.len(), 1);
    assert_eq!(lost[0]["reason"], "Connection cancelled: build.example.");
    assert!(!host.plugin().provider().cancel_connection(&target()));
}

#[test]
fn a_lost_link_says_who_closed_it_when_ssh_tells() {
    for (words, reason) in [
        (
            include_str!("../../../../fixtures/sftp/stderr/keepalive-timed-out.txt"),
            "Connection lost: build.example.",
        ),
        (
            include_str!("../../../../fixtures/sftp/stderr/closed-by-server.txt"),
            "Connection closed by build.example.",
        ),
    ] {
        let (host, _log, _root) = ssh_host(
            json!([
                {"ask": "ada@build.example's password: ", "accept": "s3cret-test"},
                {"serve": {"for_ms": 600}},
                {"stderr": words.trim_end()},
                {"exit": 255}
            ]),
            vec![json!({"text": "s3cret-test"})],
        );
        connect(&host, "sftp://build.example/").unwrap();
        assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
        assert_eq!(host.notices("provider/lost")[0]["reason"], reason);
    }
}
