//! `doctor` (R25 commit 75): the report through the plugin's command and
//! on the command line, with the fake ssh and the fixture configuration,
//! and the helper's real self-test against the plugin's own endpoint. No
//! network, never the owner's agent: the fake ssh has no `ssh-add` beside
//! it.

use std::process::Command;
use std::sync::Arc;

use diro_sftp::plugin::Plugin;
use diro_sftp::provider::SshOpener;
use diro_sftp::sites::MemoryKeyring;
use serde_json::{Value, json};

use crate::fakes::fake_ssh_in;
use crate::host::TestHost;
use crate::sites::{SITES, fixture_places};
use crate::{plain_ssh_config, plugin_exe};

fn lines_of(value: &Value) -> Vec<String> {
    value["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_doctor_command_reports_each_fact_first() {
    let root = tempfile::tempdir().unwrap();
    let ssh = fake_ssh_in(&root.path().join("bin"), &json!({}));
    std::fs::create_dir_all(root.path().join("state")).unwrap();
    std::fs::write(root.path().join("state/sites.toml"), SITES).unwrap();
    let plugin = Plugin::with_parts(
        Arc::new(SshOpener::with_helper(plugin_exe())),
        MemoryKeyring::new(),
    );
    plugin.sites().set_config_places(fixture_places());
    let host = TestHost::start(plugin, root.path(), json!({"ssh_program": ssh}), Vec::new());
    host.command("doctor", None, "D:/work").unwrap();
    let shown = host.requests("ui/show_text");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0]["title"], "SFTP doctor");
    let lines = lines_of(&shown[0]);
    assert_eq!(lines[0], "ssh: OpenSSH 9.9 from settings");
    assert_eq!(lines[1], "Passwords: asked through the helper.");
    assert_eq!(lines[2], "Agent: not checked, no ssh-add.");
    assert!(
        lines[3].starts_with("Config: ") && lines[3].ends_with(" hosts."),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"Sites: 2 saved, no secrets.".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"Key store: a test keyring in memory.".to_owned()));
    let endpoint = if cfg!(windows) {
        "Endpoint: owner-only pipe and proof."
    } else {
        "Endpoint: owner-only socket and proof."
    };
    assert!(lines.contains(&endpoint.to_owned()), "{lines:?}");
    assert!(lines.contains(&"Helper: works.".to_owned()), "{lines:?}");
    for line in &lines {
        assert!(line.chars().count() <= 40, "{line}");
    }
}

#[test]
fn doctor_on_the_command_line_prints_the_same_facts() {
    let root = tempfile::tempdir().unwrap();
    let ssh = fake_ssh_in(&root.path().join("bin"), &json!({}));
    let out = Command::new(plugin_exe())
        .arg("doctor")
        .arg("--ssh-program")
        .arg(&ssh)
        .arg("--ssh-config")
        .arg(plain_ssh_config())
        .env("DIRO_STATE_DIR", root.path())
        .env_remove("DIRO_ASKPASS_ENDPOINT")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "ssh: OpenSSH 9.9 from settings");
    assert!(lines.contains(&"Config: 2 hosts."), "{lines:?}");
    assert!(lines.contains(&"Sites: none saved."), "{lines:?}");
    assert!(lines.contains(&"Helper: works."), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("Plugin: SFTP places ")));
    assert!(out.stderr.is_empty());
}
