//! Memory secrets and the keyring, opt-in (R25 commit 74; answer 83),
//! through the whole askpass road: the plugin starts the fake ssh, which
//! runs the plugin's own program as its helper; the helper proves itself
//! and the plugin answers. The keyring is a store in memory: no test
//! touches the system's.

use std::path::PathBuf;
use std::sync::Arc;

use diro_sftp::plugin::Plugin;
use diro_sftp::provider::SshOpener;
use diro_sftp::sites::{KeyStore, MemoryKeyring};
use diroxide_core::Secret;
use serde_json::{Value, json};

use crate::connect::G_OUTPUT;
use crate::fakes::{fake_ssh_in, log_lines};
use crate::host::TestHost;
use crate::plugin_exe;
use crate::sites::fixture_places;

/// The password of the tests, which must never be seen anywhere but in
/// the answer that carries it.
const CANARY: &str = "canary-7f3a-not-a-password";

/// A site for the fixture's account, ada at build.example.
fn site(mode: &str) -> String {
    format!(
        "# A test site.\n[[site]]\nid = \"k3f9\"\nname = \"build\"\nhost = \"build.example\"\nsecret = \"{mode}\"\n"
    )
}

const ACCOUNT: &str = "site:k3f9:build.example";
const PROMPT: &str = "ada@build.example's password: ";
const PLACE: &str = "sftp://build.example/";

struct Setup {
    host: TestHost,
    keyring: Arc<MemoryKeyring>,
    log: PathBuf,
    root: tempfile::TempDir,
}

fn setup(sites: &str, g_output: &str, script: Value, answers: Vec<Value>) -> Setup {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("ssh.log");
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({"log": log, "g_output": g_output, "script": script}),
    );
    std::fs::create_dir_all(root.path().join("state")).unwrap();
    std::fs::write(root.path().join("state/sites.toml"), sites).unwrap();
    let keyring = MemoryKeyring::new();
    let plugin = Plugin::with_parts(
        Arc::new(SshOpener::with_helper(plugin_exe())),
        keyring.clone(),
    );
    plugin.sites().set_config_places(fixture_places());
    let host = TestHost::start(
        plugin,
        root.path(),
        json!({"ssh_program": ssh, "connect_timeout": 5}),
        answers,
    );
    Setup {
        host,
        keyring,
        log,
        root,
    }
}

fn connect(host: &TestHost) -> Result<Value, Value> {
    host.call(
        "provider/connect",
        json!({"place": PLACE, "interactive": true}),
    )
}

fn announced(host: &TestHost) -> Vec<String> {
    host.notices("ui/announce")
        .iter()
        .map(|n| n["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The canary appears nowhere: not in the sites file, the fake's log,
/// the notices, the questions, or the plugin's debug output.
fn no_canary(s: &Setup) {
    let file = std::fs::read_to_string(s.root.path().join("state/sites.toml")).unwrap();
    assert!(!file.contains(CANARY));
    let log = std::fs::read_to_string(&s.log).unwrap_or_default();
    assert!(!log.contains(CANARY));
    let notices = format!("{:?}", s.host.notices("ui/announce"));
    assert!(!notices.contains(CANARY));
    assert!(!format!("{:?}", s.host.asked()).contains(CANARY));
    assert!(!format!("{:?}", s.host.plugin().sites()).contains(CANARY));
}

fn script(tries: u64) -> Value {
    json!([
        {"ask": PROMPT, "accept": CANARY, "tries": tries},
        {"serve": {}}
    ])
}

#[test]
fn the_keyring_question_comes_only_after_a_successful_login() {
    let s = setup(
        &site("keyring"),
        G_OUTPUT,
        script(1),
        vec![json!({"text": CANARY}), json!({"answer": true})],
    );
    connect(&s.host).unwrap();
    let asked = s.host.asked();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0]["kind"], "secret");
    assert_eq!(asked[1]["kind"], "confirm");
    assert_eq!(asked[1]["title"], "Remember this password? y or n");
    assert_eq!(asked[1]["destructive"], true, "it starts on No");
    assert_eq!(s.keyring.accounts(), [ACCOUNT]);
    assert_eq!(s.keyring.get(ACCOUNT).unwrap().unwrap().expose(), CANARY);
    assert!(announced(&s.host).contains(&"Saved in the system keyring, not a file.".to_owned()));
    // The next connection is filled in from the keyring: nothing asked.
    s.host
        .command("disconnect", Some("site:k3f9"), "D:/work")
        .unwrap();
    connect(&s.host).unwrap();
    assert_eq!(s.host.asked().len(), 2);
    let matched = log_lines(&s.log)
        .iter()
        .filter(|l| l["matched"] == true)
        .count();
    assert_eq!(matched, 2);
    no_canary(&s);
}

#[test]
fn a_failed_login_never_asks_to_remember() {
    let s = setup(
        &site("keyring"),
        G_OUTPUT,
        script(2),
        vec![json!({"text": "wrong-1"}), json!({"text": "wrong-2"})],
    );
    assert!(connect(&s.host).is_err());
    let asked = s.host.asked();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked.iter().all(|q| q["kind"] == "secret"));
    // A wrong password is never replayed: the retry was asked, with its
    // count.
    assert_eq!(asked[1]["message"][0], "Try 2 of 3.");
    assert!(s.keyring.is_empty());
    assert!(s.host.plugin().sites().session().is_empty());
}

#[test]
fn a_refused_saved_password_is_said_and_the_owner_asked() {
    let s = setup(
        &site("keyring"),
        G_OUTPUT,
        script(2),
        vec![json!({"text": CANARY}), json!({"answer": false})],
    );
    s.keyring
        .set(ACCOUNT, &Secret::new("stale-password".to_owned()))
        .unwrap();
    connect(&s.host).unwrap();
    let asked = s.host.asked();
    // The first prompt was filled from the keyring; the second asked.
    assert_eq!(asked[0]["kind"], "secret");
    assert_eq!(asked[1]["title"], "Remember this password? y or n");
    assert!(announced(&s.host).contains(&"Saved password refused; asking you.".to_owned()));
    let lines = log_lines(&s.log);
    let attempts: Vec<&Value> = lines
        .iter()
        .filter(|l| l.get("attempt").is_some())
        .collect();
    assert_eq!(attempts[0]["matched"], false);
    assert_eq!(attempts[1]["matched"], true);
    no_canary(&s);
}

#[test]
fn a_jump_host_prompt_that_reads_like_ours_is_asked_never_filled() {
    let jump = include_str!("../../../../fixtures/sftp/ssh-g/jump-host.txt");
    let sites = "[[site]]\nid = \"l4b0\"\nname = \"lab\"\nhost = \"lab\"\nsecret = \"keyring\"\n";
    let s = setup(
        sites,
        jump,
        json!([
            // The very words of ssh's own prompt for the target's account,
            // as an older jump host could type them.
            {"ask": "ada@10.0.0.7's password: ", "accept": "typed-by-the-owner"},
            {"serve": {}}
        ]),
        vec![json!({"text": "typed-by-the-owner"})],
    );
    s.keyring
        .set("site:l4b0:lab", &Secret::new(CANARY.to_owned()))
        .unwrap();
    s.host
        .call(
            "provider/connect",
            json!({"place": "sftp://lab/", "interactive": true}),
        )
        .unwrap();
    let asked = s.host.asked();
    assert_eq!(asked.len(), 1, "asked, and no remember question: {asked:?}");
    assert_eq!(asked[0]["kind"], "secret");
    let connection = s
        .host
        .plugin()
        .provider()
        .connection(&diro_sftp::ssh::Target::new("lab", None, None).unwrap())
        .unwrap();
    // Nothing from a jump host's road is remembered for a second session.
    assert!(!connection.memory().has_password());
    assert_eq!(
        s.keyring.get("site:l4b0:lab").unwrap().unwrap().expose(),
        CANARY
    );
    no_canary(&s);
}

#[test]
fn this_session_keeps_it_quietly_until_the_owner_disconnects() {
    let s = setup(
        &site("session"),
        G_OUTPUT,
        script(1),
        vec![json!({"text": CANARY}), json!({"text": CANARY})],
    );
    connect(&s.host).unwrap();
    assert_eq!(s.host.asked().len(), 1, "no remember question in this mode");
    assert!(s.keyring.is_empty());
    // The last lease ends: the connection closes; the password stays.
    s.host
        .notify("provider/disconnect", json!({"place": PLACE}));
    assert!(
        s.host
            .wait_for(|h| h.plugin().provider().connected().is_empty())
    );
    connect(&s.host).unwrap();
    assert_eq!(s.host.asked().len(), 1);
    // The owner disconnects the last connection: this session's go.
    s.host
        .command("disconnect", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert!(s.host.plugin().sites().session().is_empty());
    connect(&s.host).unwrap();
    assert_eq!(s.host.asked().len(), 2);
    no_canary(&s);
}

#[test]
fn a_site_that_never_remembers_keeps_nothing() {
    let s = setup(
        &site("never"),
        G_OUTPUT,
        script(1),
        vec![json!({"text": CANARY}), json!({"text": CANARY})],
    );
    connect(&s.host).unwrap();
    s.host
        .command("disconnect", Some("site:k3f9"), "D:/work")
        .unwrap();
    connect(&s.host).unwrap();
    assert_eq!(s.host.asked().len(), 2);
    assert!(s.keyring.is_empty());
    assert!(s.host.plugin().sites().session().is_empty());
}

#[test]
fn forget_password_asks_first_and_starts_on_no() {
    let s = setup(
        &site("keyring"),
        G_OUTPUT,
        script(1),
        vec![json!({"answer": false}), json!({"answer": true})],
    );
    s.keyring
        .set(ACCOUNT, &Secret::new(CANARY.to_owned()))
        .unwrap();
    s.host
        .command("forget_password", Some("site:k3f9"), "D:/work")
        .unwrap();
    let asked = s.host.asked();
    assert_eq!(asked[0]["title"], "Forget the saved password for build?");
    assert_eq!(asked[0]["destructive"], true);
    assert_eq!(s.keyring.len(), 1);
    s.host
        .command("forget_password", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert!(s.keyring.is_empty());
    assert!(announced(&s.host).contains(&"Password forgotten for build.".to_owned()));
    s.host
        .command("forget_password", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert!(announced(&s.host).contains(&"No password saved for build.".to_owned()));
    assert_eq!(s.host.asked().len(), 2);
}

#[test]
fn deleting_a_site_forgets_its_password() {
    let s = setup(
        &site("keyring"),
        G_OUTPUT,
        script(1),
        vec![json!({"answer": true})],
    );
    s.keyring
        .set(ACCOUNT, &Secret::new(CANARY.to_owned()))
        .unwrap();
    s.host
        .command("delete_site", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert!(s.keyring.is_empty());
}
