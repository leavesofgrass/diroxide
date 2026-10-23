//! Sites, the Sites list, the site commands and `provider/places` (R25
//! commits 72 and 73), through the plugin's own loop and the fake server
//! in memory. The configuration's hosts come from the fixture home, never
//! the owner's.

use std::path::Path;
use std::sync::{Arc, Mutex};

use diro_sftp::plugin::Plugin;
use diro_sftp::provider::fake::FakeOpener;
use diro_sftp::sftp::fake::{FakeFs, Faults};
use diro_sftp::ssh::hosts::ConfigPlaces;
use serde_json::{Value, json};

use crate::host::{TestHost, kind};

/// The owner's sites in the tests: placeholder hosts and users only.
pub const SITES: &str = "\
# Test sites. Comments stay.
[[site]]
id = \"k3f9\"
name = \"build\"
group = \"Work\"
host = \"build.example\"
user = \"ada\"
start_remote = \"/srv\"
start_local = \"D:/projects\"
secret = \"never\"

[[site]]
id = \"w3b1\"
name = \"web\"
host = \"web.example\"
port = 2222
secret = \"session\"
";

/// The fixture home of S-A's configuration tests: hosts in Include files.
pub fn fixture_places() -> ConfigPlaces {
    let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sftp/ssh-config/home");
    ConfigPlaces {
        file: home.join(".ssh/config"),
        ssh_dir: home.join(".ssh"),
        home,
    }
}

/// A host whose plugin opens sessions on the fake server, with `sites`
/// as its sites file and the owner's answers in order.
pub fn sites_host(
    sites: &str,
    answers: Vec<Value>,
) -> (TestHost, Arc<FakeOpener>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("state")).unwrap();
    std::fs::write(root.path().join("state/sites.toml"), sites).unwrap();
    let opener = FakeOpener::new(Arc::new(Mutex::new(FakeFs::default())), Faults::default());
    let plugin = Plugin::with_opener(opener.clone());
    plugin.sites().set_config_places(fixture_places());
    let host = TestHost::start(plugin, root.path(), json!({}), answers);
    (host, opener, root)
}

fn sites_file(root: &tempfile::TempDir) -> String {
    std::fs::read_to_string(root.path().join("state/sites.toml")).unwrap()
}

fn announced(host: &TestHost) -> Vec<String> {
    host.notices("ui/announce")
        .iter()
        .map(|n| n["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn the_sites_list_opens_with_golden_rows_and_counts() {
    let (host, _, _root) = sites_host(SITES, Vec::new());
    let answer = host.command("sites", None, "D:/work").unwrap();
    let opened = host.requests("lister/open");
    assert_eq!(opened.len(), 1);
    let lister = &opened[0];
    assert_eq!(lister["lister"], "sites");
    let names: Vec<&str> = lister["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names[0], "web, saved, web.example:2222");
    assert_eq!(names[1], "build, saved, ada@build.example");
    assert!(names[2..].iter().all(|n| n.ends_with(", from ssh config")));
    let configs = names.len() - 2;
    assert_eq!(
        lister["title"],
        format!("Sites, 2 saved, {configs} from ssh config")
    );
    assert_eq!(answer["done"], names.len());
    for name in names {
        assert!(name.chars().count() <= 40, "{name}");
    }
}

#[test]
fn places_answer_a_state_word_and_follow_the_connection() {
    let (host, opener, _root) = sites_host(SITES, Vec::new());
    // A connection that needed a typed secret is not reopened unasked, so
    // the cut is a loss.
    opener.as_if_typed(true);
    let before = host.call("provider/places", json!({})).unwrap();
    assert_eq!(before["places"][1]["name"], "build");
    assert_eq!(before["places"][1]["state"], "saved");
    assert_eq!(before["places"][1]["place"], "sftp://ada@build.example/srv");
    host.call(
        "provider/connect",
        json!({"place": "sftp://ada@build.example/srv", "interactive": true}),
    )
    .unwrap();
    let after = host.call("provider/places", json!({})).unwrap();
    assert_eq!(after["places"][1]["state"], "connected");
    opener.cut_all();
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    let lost = host.call("provider/places", json!({})).unwrap();
    assert_eq!(lost["places"][1]["state"], "lost");
}

#[test]
fn connect_signs_in_then_opens_both_start_folders() {
    let (host, opener, _root) = sites_host(SITES, Vec::new());
    host.command("connect", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert_eq!(opener.openings().len(), 1);
    let said = announced(&host);
    assert!(
        said.contains(&"Connecting to build.example.".to_owned()),
        "{said:?}"
    );
    assert!(
        said.contains(&"Connected to build.example.".to_owned()),
        "{said:?}"
    );
    let moves = host.requests("panel/set");
    assert_eq!(
        moves,
        [
            json!({"panel": "active", "place": "sftp://ada@build.example/srv"}),
            json!({"panel": "other", "place": "D:/projects"}),
        ]
    );
    // A host from the configuration connects to its name; its home opens.
    host.command("connect", Some("config:nas"), "D:/work")
        .unwrap();
    let moves = host.requests("panel/set");
    assert_eq!(moves[2]["place"], "sftp://nas/home/ada");
}

#[test]
fn disconnect_and_cancel_act_on_the_row_or_the_panel() {
    let (host, _, _root) = sites_host(SITES, Vec::new());
    host.command("connect", Some("site:k3f9"), "D:/work")
        .unwrap();
    host.command("disconnect", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert!(announced(&host).contains(&"Disconnected from build.".to_owned()));
    assert!(host.plugin().provider().connected().is_empty());
    let again = host
        .command("disconnect", Some("site:k3f9"), "D:/work")
        .unwrap_err();
    assert_eq!(again["message"], "Not connected to build.");
    // From the panel: the place's own connection; Cancel is said as lost.
    host.command("connect", Some("site:k3f9"), "D:/work")
        .unwrap();
    host.command("cancel_connection", None, "sftp://ada@build.example/srv")
        .unwrap();
    assert!(host.wait_for(|h| !h.notices("provider/lost").is_empty()));
    let none = host.command("disconnect", None, "D:/work").unwrap_err();
    assert_eq!(none["message"], "No site on this row.");
}

#[test]
fn delete_asks_first_and_starts_on_no() {
    let (host, _, root) = sites_host(
        SITES,
        vec![json!({"answer": false}), json!({"answer": true})],
    );
    host.command("delete_site", Some("site:w3b1"), "D:/work")
        .unwrap();
    let asked = host.asked();
    assert_eq!(asked[0]["kind"], "confirm");
    assert_eq!(asked[0]["destructive"], true);
    assert_eq!(asked[0]["title"], "Delete the site web?");
    assert!(announced(&host).contains(&"Nothing was changed.".to_owned()));
    assert!(sites_file(&root).contains("web.example"));
    host.command("delete_site", Some("site:w3b1"), "D:/work")
        .unwrap();
    let text = sites_file(&root);
    assert!(!text.contains("web.example"));
    assert!(text.contains("# Test sites. Comments stay."));
    assert!(announced(&host).contains(&"Site deleted: web.".to_owned()));
}

#[test]
fn new_sites_are_saved_with_no_secret_and_comments_kept() {
    let (host, _, root) = sites_host(
        SITES,
        vec![
            json!({"text": "sftp://ada@files.example:2200/data"}),
            json!({"text": "files"}),
            json!({"ids": ["keyring"]}),
        ],
    );
    host.command("new_site", None, "D:/work").unwrap();
    let text = sites_file(&root);
    assert!(text.contains("# Test sites. Comments stay."));
    assert!(text.contains("name = \"files\""));
    assert!(text.contains("host = \"files.example\""));
    assert!(text.contains("port = 2200"));
    assert!(text.contains("start_remote = \"/data\""));
    assert!(text.contains("secret = \"keyring\""));
    assert!(!text.to_lowercase().contains("password"));
    assert_eq!(host.asked()[2]["kind"], "choose");
    // A host from the configuration is saved as an alias site, unasked.
    host.command("new_site", Some("config:nas"), "D:/work")
        .unwrap();
    assert!(sites_file(&root).contains("alias = \"nas\""));
    // A hostile address is refused in words and nothing is saved.
    let (host, _, root) = sites_host(SITES, Vec::new());
    let refused = host
        .call(
            "command/run",
            json!({"command": "sftp.new_site",
                   "args": {"address": "-oProxyCommand=x", "name": "evil"}}),
        )
        .unwrap_err();
    assert_eq!(refused["message"], "Not saved: the address was not valid.");
    assert_eq!(sites_file(&root), SITES);
}

#[test]
fn edit_keeps_comments_and_escape_changes_nothing() {
    let (host, _, root) = sites_host(
        SITES,
        vec![
            json!({"text": "build server"}),
            json!({"text": "ada@build.example"}),
            json!({"text": "Work/Servers"}),
            json!({"text": "/srv/www"}),
            json!({"ids": ["session"]}),
        ],
    );
    host.command("edit_site", Some("site:k3f9"), "D:/work")
        .unwrap();
    let text = sites_file(&root);
    assert!(text.contains("name = \"build server\""));
    assert!(text.contains("group = \"Work/Servers\""));
    assert!(text.contains("start_remote = \"/srv/www\""));
    assert!(text.contains("# Test sites. Comments stay."));
    let (host, _, root) = sites_host(SITES, vec![json!({"cancelled": true})]);
    host.command("edit_site", Some("site:k3f9"), "D:/work")
        .unwrap();
    assert_eq!(sites_file(&root), SITES);
    assert!(announced(&host).contains(&"Nothing was changed.".to_owned()));
}

#[test]
fn a_planted_password_is_refused_and_said() {
    let planted = SITES.replace("secret = \"never\"", "password = \"hunter2\"");
    let (host, _, root) = sites_host(&planted, Vec::new());
    let refused = host.command("sites", None, "D:/work").unwrap_err();
    assert_eq!(
        refused["message"],
        "Sites file refused: it holds a password."
    );
    assert_eq!(kind(&refused), "failed");
    let hint = refused["data"]["hint"].as_str().unwrap();
    assert!(hint.starts_with("Remove password from build."), "{hint}");
    // Places leave the saved sites out and keep the configuration's.
    let places = host.call("provider/places", json!({})).unwrap();
    let rows = places["places"].as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r["state"] == "from ssh config"));
    // Nothing writes over a refused file.
    let edit = host.command("new_site", Some("config:nas"), "D:/work");
    assert!(edit.is_err());
    assert_eq!(sites_file(&root), planted);
}
