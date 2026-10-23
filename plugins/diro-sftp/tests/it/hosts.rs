//! The Host lister and the `-G` resolver (R25 commit 49): fixture
//! configurations, never the owner's real file, and a fake ssh that
//! prints `-G` output.

use std::path::{Path, PathBuf};

use diro_sftp::ssh::{ConfigPlaces, Target, config_hosts, resolve};
use serde_json::json;

use crate::fakes::{fake_ssh_in, log_lines};

fn fixture_home() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/sftp/ssh-config/home")
        .canonicalize()
        .unwrap()
}

fn places() -> ConfigPlaces {
    let home = fixture_home();
    ConfigPlaces {
        file: home.join(".ssh").join("config"),
        ssh_dir: home.join(".ssh"),
        home,
    }
}

#[test]
fn the_user_file_and_its_includes_give_the_host_names() {
    let list = config_hosts(&places());
    let names: Vec<&str> = list.hosts.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(
        names,
        ["build", "nas", "quoted", "lab", "lab-gpu", "[::1]", "last"]
    );
    assert_eq!(
        list.hosts[3].file.file_name().and_then(|n| n.to_str()),
        Some("a.conf")
    );
    let rows: Vec<String> = list.hosts.iter().take(2).map(|h| h.row()).collect();
    assert_eq!(rows, ["build, from ssh config", "nas, from ssh config"]);
    assert_eq!(
        list.skipped,
        [
            (
                "two words".to_owned(),
                "The host name holds a character ssh cannot take.".to_owned()
            ),
            (
                "-oProxyCommand=calc".to_owned(),
                "The host name starts with a dash.".to_owned()
            ),
        ]
    );
    assert_eq!(list.unreadable.len(), 1);
    assert!(list.unreadable[0].ends_with("missing/never.conf"));
}

#[test]
fn a_missing_file_lists_nothing() {
    let mut places = places();
    places.file = places.ssh_dir.join("no-such-config");
    assert_eq!(config_hosts(&places).hosts.len(), 0);
}

/// Answer 94: the list never starts ssh; `-G` runs once, when a host is
/// opened, with the host after `--`.
#[test]
fn ssh_g_runs_only_when_a_host_is_opened() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("ssh.log");
    let ssh = fake_ssh_in(
        &root.path().join("bin"),
        &json!({
            "log": log,
            "g_output": include_str!("../../../../fixtures/sftp/ssh-g/jump-host.txt"),
        }),
    );
    let list = config_hosts(&places());
    assert!(!list.hosts.is_empty());
    assert!(log_lines(&log).is_empty(), "the list started ssh");
    let resolved = resolve(&ssh, &Target::new("lab", None, None).unwrap(), None).unwrap();
    assert_eq!(resolved.hostname(), Some("10.0.0.7"));
    assert_eq!(resolved.proxy_jump(), Some("bastion.example"));
    let runs = log_lines(&log);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["args"], json!(["-G", "--", "lab"]));
}
