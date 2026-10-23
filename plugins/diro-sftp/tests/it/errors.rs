//! Error mapping and the standard-error classifier (R25 commit 63): each
//! standard-error text gets one sentence with the cause first; a changed
//! key, a refusal, a timeout, a missing subsystem and a shell that prints
//! before the protocol through the fake ssh; and the operation table on
//! the fake server, which reads a plain FAILURE for what it means.

use std::time::{Duration, Instant};

use diro_sftp::sftp::fake::Faults;
use diro_sftp::ssh::stderr::{Cause, classify, refusal};
use diroxide_plugins::base64;
use serde_json::{Value, json};

use crate::connect::{connect, fake_host, ssh_host};
use crate::host::kind;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!("../../../../fixtures/sftp/stderr/", $name))
    };
}

/// The texts, the cause each tells, and the sentence it gets.
const TEXTS: &[(&str, Cause, &str, &str)] = &[
    (
        fixture!("changed-host-key-windows-9.5p2.txt"),
        Cause::ChangedKey,
        "untrusted",
        "Host key changed. Not connected.",
    ),
    (
        fixture!("changed-host-key-msys-10.3p1.txt"),
        Cause::ChangedKey,
        "untrusted",
        "Host key changed. Not connected.",
    ),
    (
        fixture!("host-key-verification-failed.txt"),
        Cause::KeyNotTrusted,
        "untrusted",
        "Host key not trusted. Not connected.",
    ),
    (
        fixture!("permission-denied.txt"),
        Cause::SignInRefused,
        "auth_required",
        "Sign-in refused by build.example.",
    ),
    (
        fixture!("too-many-authentication-failures.txt"),
        Cause::SignInRefused,
        "auth_required",
        "Sign-in refused by build.example.",
    ),
    (
        fixture!("no-subsystem.txt"),
        Cause::NoSubsystem,
        "unsupported",
        "No SFTP service on build.example.",
    ),
    (
        fixture!("connect-timed-out.txt"),
        Cause::TimedOut,
        "timed_out",
        "No answer from build.example.",
    ),
    (
        fixture!("banner-timed-out.txt"),
        Cause::TimedOut,
        "timed_out",
        "No answer from build.example.",
    ),
    (
        fixture!("connection-refused.txt"),
        Cause::ConnectionRefused,
        "unavailable",
        "Connection refused by build.example.",
    ),
    (
        fixture!("unknown-host-windows.txt"),
        Cause::UnknownHost,
        "unavailable",
        "Unknown host: build.example.",
    ),
    (
        fixture!("unknown-host-linux.txt"),
        Cause::UnknownHost,
        "unavailable",
        "Unknown host: build.example.",
    ),
    (
        fixture!("network-unreachable.txt"),
        Cause::NoRoute,
        "unavailable",
        "No route to build.example.",
    ),
    (
        fixture!("bad-config.txt"),
        Cause::BadConfig,
        "unavailable",
        "Your ssh config has an error.",
    ),
    (
        fixture!("closed-by-server.txt"),
        Cause::ClosedByServer,
        "unavailable",
        "Connection closed by build.example.",
    ),
];

#[test]
fn every_captured_text_gets_one_sentence_cause_first() {
    for (text, cause, kind_name, sentence) in TEXTS {
        assert_eq!(classify(text), *cause, "{text}");
        let error = refusal(text, "build.example", Some(22));
        assert_eq!(error.message, *sentence, "{text}");
        assert_eq!(error.kind().map(|k| k.name()), Some(*kind_name), "{text}");
        assert!(error.message.chars().count() <= 40);
        // The server's and ssh's words are never the sentence; when they
        // come, they come last, cleaned.
        let hint = error.hint().unwrap_or_default();
        for line in hint.lines() {
            assert!(!line.contains('\u{1B}'));
        }
    }
}

fn connect_error(script: Value) -> Value {
    let (host, _log, _root) = ssh_host(script, Vec::new());
    connect(&host, "sftp://build.example/").unwrap_err()
}

#[test]
fn a_refused_sign_in_through_the_fake_ssh() {
    let refused = connect_error(json!([
        {"stderr": fixture!("permission-denied.txt").trim_end()},
        {"exit": 255}
    ]));
    assert_eq!(kind(&refused), "auth_required");
    assert_eq!(refused["message"], "Sign-in refused by build.example.");
    let hint = refused["data"]["hint"].as_str().unwrap();
    let lines: Vec<&str> = hint.lines().collect();
    assert_eq!(lines[0], "Check the user name, key or password.");
    assert!(lines[1].starts_with("ssh said: "), "{hint}");
}

#[test]
fn a_missing_subsystem_and_a_timeout_through_the_fake_ssh() {
    let missing = connect_error(json!([
        {"stderr": "subsystem request failed on channel 0"},
        {"exit": 255}
    ]));
    assert_eq!(kind(&missing), "unsupported");
    assert_eq!(missing["message"], "No SFTP service on build.example.");
    let late = connect_error(json!([
        {"stderr": "Connection timed out during banner exchange"},
        {"exit": 255}
    ]));
    assert_eq!(kind(&late), "timed_out");
    assert_eq!(late["message"], "No answer from build.example.");
}

#[test]
fn a_shell_that_prints_first_is_told_at_the_first_four_bytes() {
    let began = Instant::now();
    let chatter = connect_error(json!([
        {"stdout": "Welcome to build.example!\nHave a nice day.\n"},
        {"drain": true}
    ]));
    assert_eq!(kind(&chatter), "unavailable");
    assert_eq!(chatter["message"], "The server's shell printed text first.");
    assert_eq!(
        chatter["data"]["hint"],
        "Make its startup file quiet for sftp."
    );
    // Told from the first four bytes; nothing more is waited for.
    assert!(began.elapsed() < Duration::from_secs(10));
}

#[test]
fn an_ssh_too_old_to_ask_says_so() {
    let root = tempfile::tempdir().unwrap();
    let ssh = crate::fakes::fake_ssh_in(
        &root.path().join("bin"),
        &json!({
            "version": "OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2",
            "g_output": crate::connect::G_OUTPUT,
            "script": [{"stderr": "ada@build.example: Permission denied (password)."}, {"exit": 255}]
        }),
    );
    let host = crate::host::TestHost::start(
        diro_sftp::plugin::Plugin::with_opener(std::sync::Arc::new(
            diro_sftp::provider::SshOpener::with_helper(crate::plugin_exe()),
        )),
        root.path(),
        json!({"ssh_program": ssh}),
        Vec::new(),
    );
    let refused = connect(&host, "sftp://build.example/").unwrap_err();
    assert_eq!(kind(&refused), "auth_required");
    assert_eq!(refused["message"], "This ssh is too old for passwords.");
}

#[test]
fn a_plain_failure_is_read_by_the_operation_table() {
    let (host, opener, _root) = fake_host(Faults {
        fail_write_at: Some(2),
        ..Faults::default()
    });
    {
        let mut fs = opener.fs().lock().unwrap();
        fs.add_dir(b"/srv/made");
        fs.add_file(b"/srv/full/a.txt", b"a");
        fs.add_file(b"/srv/file.txt", b"x");
    }
    connect(&host, "sftp://build.example/").unwrap();
    let call = |method: &str, params: Value| host.call(method, params).unwrap_err();
    let made = call(
        "provider/create_dir",
        json!({"place": "sftp://build.example/srv/made"}),
    );
    assert_eq!(kind(&made), "already_exists");
    let full = call(
        "provider/remove",
        json!({"place": "sftp://build.example/srv/full"}),
    );
    assert_eq!(kind(&full), "not_empty");
    assert_eq!(full["message"], "The folder is not empty.");
    let not_folder = call(
        "provider/list",
        json!({"place": "sftp://build.example/srv/file.txt"}),
    );
    assert_eq!(kind(&not_folder), "not_a_directory");
    let folder = call(
        "provider/read",
        json!({"place": "sftp://build.example/srv/made", "offset": 0, "length": 10}),
    );
    assert_eq!(kind(&folder), "is_a_directory");
    let begun = host
        .call(
            "provider/write_begin",
            json!({"place": "sftp://build.example/srv/new.bin", "conflict": "fail"}),
        )
        .unwrap();
    let data: Vec<u8> = vec![7; 4 * 32 * 1024];
    let no_space = call(
        "provider/write_chunk",
        json!({"write_id": begun["write_id"], "offset": 0, "data": base64::encode(&data)}),
    );
    assert_eq!(kind(&no_space), "no_space");
    assert_eq!(no_space["message"], "The server's disk or quota is full.");
    assert_eq!(
        no_space["data"]["hint"],
        "Server said: No space left on device"
    );
}

#[test]
fn a_quota_at_close_is_no_space() {
    let (host, opener, _root) = fake_host(Faults {
        fail_close: true,
        ..Faults::default()
    });
    opener.fs().lock().unwrap().add_dir(b"/srv");
    connect(&host, "sftp://build.example/").unwrap();
    let begun = host
        .call(
            "provider/write_begin",
            json!({"place": "sftp://build.example/srv/new.bin", "conflict": "fail"}),
        )
        .unwrap();
    host.call(
        "provider/write_chunk",
        json!({"write_id": begun["write_id"], "offset": 0, "data": base64::encode(b"bytes")}),
    )
    .unwrap();
    let refused = host
        .call(
            "provider/write_end",
            json!({"write_id": begun["write_id"], "commit": true}),
        )
        .unwrap_err();
    assert_eq!(kind(&refused), "no_space");
}
