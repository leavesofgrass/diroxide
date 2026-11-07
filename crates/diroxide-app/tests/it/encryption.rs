//! The Encryption submenu (alpha 4, lane AE; R29 3.2): its three age
//! commands run the encryption plugin's commands, here a test plugin over
//! a memory transport (never the real `diro-crypt`, never a real age);
//! what an encryption made is recorded for Undo and the originals are
//! offered to the trash, starting on No, the trash a test folder; without
//! the plugin the commands offer its component or say what they need.
//! Every file is under the test's own folder; nothing of the owner's is
//! encrypted or decrypted.

use std::fs;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_app::menu::Menus;
use diroxide_config::Paths;
use diroxide_core::{JobKind, JobOrigin};
use diroxide_plugins::manifest::Manifest;
use diroxide_plugins::script::consent::{ConsentStore, Kind, plugin_fingerprint};
use serde_json::json;

use crate::plugins::{Act, TestLauncher, World};
use crate::support::{
    Session, Setup, TestTrash, fixture, place, registry_with_trash, settings, temp,
};

/// The test plugin's manifest under the encryption plugin's id.
const CRYPT: &str = r#"
[plugin]
id = "crypt"
name = "Encryption"
version = "0.1.0"
protocol = 1

[run]
command = ["test-plugin"]
startup_timeout = "2s"

[[command]]
id = "encrypt_passphrase"
name = "Encrypt with a passphrase"
description = "Encrypts the marked files with a passphrase."
selection = "any"
places = "local"

[[command]]
id = "encrypt_recipient"
name = "Encrypt for a recipient"
description = "Encrypts the marked files for a recipient."
selection = "any"
places = "local"

[[command]]
id = "decrypt"
name = "Decrypt"
description = "Decrypts the marked age files."
selection = "any"
places = "local"
"#;

/// A world with the test plugin installed as `crypt`, enabled when asked.
fn world(enabled: bool) -> World {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let folder = paths.plugins_dir().join("crypt");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(Manifest::FILE), CRYPT).unwrap();
    if enabled {
        let manifest = Manifest::from_toml(CRYPT).unwrap();
        let fingerprint =
            plugin_fingerprint(&manifest, &folder, diroxide_components::sha256_hex).unwrap();
        ConsentStore::open(&paths.state_dir.join("plugins"))
            .enable("crypt", Kind::Plugin, fingerprint)
            .unwrap();
    }
    let work = fixture(dir.path());
    World {
        _dir: dir,
        paths,
        work,
    }
}

/// A session over the world whose trash is a test folder.
fn session(
    w: &World,
    launcher: std::sync::Arc<TestLauncher>,
) -> (Session, std::sync::Arc<TestTrash>) {
    let (registry, trash) = registry_with_trash(&w.work.parent().unwrap().join("bin"));
    let here = place(&w.work);
    let s = Session::open(
        here.clone(),
        here,
        registry,
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    );
    (s, trash)
}

/// The plugin's answer: what it made from `from`.
fn made(w: &World, from: &str, to: &str) -> serde_json::Value {
    json!({"done": 1, "skipped": 0, "failed": [], "made": [{
        "from": w.work.join(from).to_str().unwrap(),
        "to": w.work.join(to).to_str().unwrap(),
    }]})
}

#[test]
fn encrypting_records_undo_and_offers_the_trash_starting_on_no() {
    let w = world(true);
    let answer = made(&w, "notes.txt", "notes.txt.age");
    let launcher = TestLauncher::with(&[("encrypt_passphrase", Act::Answer(answer))]);
    let (mut s, trash) = session(&w, launcher);
    s.focus("notes.txt");
    let said = s.command(ids::ENCRYPT_PASSPHRASE);
    let record = s.app.undo_log().last().cloned().expect("an undo record");
    assert_eq!(record.kind, Some(JobKind::Encrypt), "{said:?}");
    assert_eq!(record.created, [place(&w.work.join("notes.txt.age"))]);
    assert_eq!(record.origin, JobOrigin::Plugin("Encryption".to_owned()));
    let Some(Dialog::Confirm {
        question, listing, ..
    }) = s.dialog().cloned()
    else {
        panic!("no question: {said:?} {:?}", s.dialog());
    };
    assert_eq!(
        question.text,
        "Move the original to the trash? No is the default. y or n"
    );
    assert_eq!(listing[0], "notes.txt");
    assert!(
        listing[1].starts_with("The trash is not secure"),
        "{listing:?}"
    );
    // No keeps it, and says so.
    let said = s.key("n");
    assert!(
        said.contains(&"The originals stay where they are.".to_owned()),
        "{said:?}"
    );
    assert!(w.work.join("notes.txt").exists());
    assert_eq!(trash.count(), 0);
    // Again, and Yes moves it to the (test) trash as a job.
    s.command(ids::ENCRYPT_PASSPHRASE);
    assert!(s.dialog().is_some());
    s.key("y");
    assert!(!w.work.join("notes.txt").exists());
    assert_eq!(trash.count(), 1);
}

#[test]
fn decrypting_records_undo_and_offers_nothing() {
    let w = world(true);
    let answer = made(&w, "report.pdf.age", "report.pdf (2)");
    let launcher = TestLauncher::with(&[("decrypt", Act::Answer(answer))]);
    let (mut s, _trash) = session(&w, launcher);
    s.focus("report.pdf");
    s.command(ids::DECRYPT);
    let record = s.app.undo_log().last().cloned().expect("an undo record");
    assert_eq!(record.kind, Some(JobKind::Decrypt));
    assert!(s.dialog().is_none());
}

#[test]
fn without_the_plugin_the_commands_say_what_they_need() {
    let w = world(false);
    let dir = temp();
    let work = fixture(dir.path());
    let mut s = Session::open(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        Setup::default(),
    );
    s.focus("notes.txt");
    let said = s.command(ids::ENCRYPT_RECIPIENT);
    let title = s.dialog_title().unwrap_or_default();
    assert!(
        said.iter()
            .chain(std::iter::once(&title))
            .any(|l| l.starts_with("Encrypting with age needs a plugin.")),
        "{said:?} {title}"
    );
    // Installed but not enabled: Manage plugins enables it.
    let (mut s, _trash) = session(&w, TestLauncher::with(&[]));
    s.focus("notes.txt");
    let said = s.command(ids::DECRYPT);
    assert!(
        said.contains(&"Encryption is not enabled. Enable it in Manage plugins.".to_owned()),
        "{said:?}"
    );
}

#[test]
fn the_submenu_is_on_the_file_menu() {
    let menus = Menus::builtin();
    let file = menus.menu("file").unwrap();
    assert!(file.items.iter().any(|i| i.row_id() == "menu:encryption"));
    for id in [
        ids::ENCRYPT_PASSPHRASE,
        ids::ENCRYPT_RECIPIENT,
        ids::DECRYPT,
    ] {
        assert_eq!(menus.holder_of(&id), Some("Encryption"), "{id}");
    }
}
