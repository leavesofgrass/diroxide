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
        ids::GPG_ENCRYPT,
        ids::GPG_DECRYPT,
        ids::SIGN,
        ids::VERIFY_SIGNATURE,
    ] {
        assert_eq!(menus.holder_of(&id), Some("Encryption"), "{id}");
    }
}

/// A plain session on the fixture, its tools the test's own.
fn plain() -> (tempfile::TempDir, std::path::PathBuf, Session) {
    let dir = temp();
    let work = fixture(dir.path());
    let s = Session::open(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        Setup::default(),
    );
    (dir, work, s)
}

#[test]
fn encrypt_with_gpg_asks_the_keys_then_a_plan_starting_on_no() {
    let (_dir, work, mut s) = plain();
    s.tools.answer("gpg", b"[GNUPG:] END_ENCRYPTION\n");
    s.focus("notes.txt");
    s.command(ids::GPG_ENCRYPT);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Encrypt for gpg keys (empty: passphrase)")
    );
    s.type_text("4E7B");
    s.key("enter");
    let Some(Dialog::Confirm {
        question, listing, ..
    }) = s.dialog().cloned()
    else {
        panic!("no plan: {:?}", s.dialog());
    };
    assert_eq!(
        question.text,
        "Encrypt 1 file for 1 recipient? No is the default. y or n"
    );
    assert_eq!(listing, ["notes.txt to notes.txt.gpg"]);
    // Planning ran nothing.
    assert!(s.tools.ran().is_empty(), "{:?}", s.tools.ran());
    let said = s.key("n");
    assert!(said.contains(&"Nothing changed.".to_owned()), "{said:?}");
    // Yes runs gpg; this fake makes no file, so the file fails in words
    // and nothing is left beside it.
    s.command(ids::GPG_ENCRYPT);
    s.type_text("4E7B");
    s.key("enter");
    let said = s.key("y");
    let all = s.said.iter().map(|m| m.text.clone()).collect::<Vec<_>>();
    assert!(
        all.iter()
            .any(|l| l == "Encrypt started: 1 file for 1 recipient"),
        "{said:?}"
    );
    assert!(
        all.iter()
            .any(|l| l.contains("Could not encrypt notes.txt: gpg made no file")),
        "{all:?}"
    );
    let args: Vec<String> = s.tools.runs.lock().unwrap()[0]
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(args.contains(&"--batch".to_owned()), "{args:?}");
    assert!(!args.iter().any(|a| a.contains("passphrase ")), "{args:?}");
    let left: Vec<_> = fs::read_dir(&work)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".diro-part-"))
        .collect();
    assert!(left.is_empty());
}

#[test]
fn a_gpg_passphrase_is_typed_twice_hidden_and_never_reaches_an_argument() {
    let (_dir, _work, mut s) = plain();
    s.tools.answer("gpg", b"[GNUPG:] END_ENCRYPTION\n");
    s.focus("notes.txt");
    s.command(ids::GPG_ENCRYPT);
    s.key("enter");
    let Some(Dialog::Confirm { question, .. }) = s.dialog().cloned() else {
        panic!("no plan: {:?}", s.dialog());
    };
    assert!(
        question
            .text
            .starts_with("Encrypt 1 file with a passphrase"),
        "{}",
        question.text
    );
    s.key("y");
    // The masked prompt, twice; two that differ ask again.
    let canary = "canary-5e1d-not-a-passphrase";
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("New passphrase for notes.txt")
    );
    assert!(matches!(s.dialog(), Some(Dialog::Secret { .. })));
    s.type_text(canary);
    s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Same passphrase again for notes.txt")
    );
    s.type_text("something else");
    s.key("enter");
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Not the same. New passphrase for notes.txt")
    );
    s.type_text(canary);
    s.key("enter");
    s.type_text(canary);
    s.key("enter");
    let runs = s.tools.runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 1, "{:?}", s.said);
    for arg in &runs[0].args {
        assert!(!arg.to_string_lossy().contains(canary), "{arg:?}");
    }
    assert!(
        runs[0]
            .args
            .iter()
            .any(|a| a.to_string_lossy() == "--passphrase-fd"),
        "the passphrase goes on standard input"
    );
    for message in &s.said {
        assert!(!message.text.contains(canary), "{}", message.text);
    }
}

#[test]
fn without_gpg_the_plan_says_how_to_get_it() {
    let (_dir, _work, mut s) = plain();
    s.focus("notes.txt");
    let said = s.command(ids::GPG_DECRYPT);
    assert!(
        said.contains(&"gpg not found. Install GnuPG first.".to_owned()),
        "{said:?}"
    );
    assert!(s.dialog().is_none());
}

#[test]
fn verify_signature_says_the_verdict_first_and_what_good_means() {
    let (_dir, work, mut s) = plain();
    fs::write(
        work.join("notes.txt.asc"),
        "-----BEGIN PGP SIGNATURE-----\n",
    )
    .unwrap();
    s.command(ids::RELOAD);
    s.tools.answer(
        "gpg",
        b"[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG 168B6F650E9C8469 Ada Example <ada@example.invalid>\n[GNUPG:] VALIDSIG x\n[GNUPG:] TRUST_UNDEFINED 0 pgp\n",
    );
    s.focus("notes.txt");
    s.command(ids::VERIFY_SIGNATURE);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Signature good: key ending 8469")
    );
    match s.dialog() {
        Some(Dialog::Read { lines, .. }) => {
            assert_eq!(
                lines.last().map(String::as_str),
                Some("Good means the key fits, not who holds it.")
            );
        }
        other => panic!("{other:?}"),
    }
    // The focused signature checks its own file the same way.
    s.key("escape");
    s.focus("notes.txt.asc");
    s.command(ids::VERIFY_SIGNATURE);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Signature good: key ending 8469")
    );
    // A file with no signature beside it says so.
    s.key("escape");
    s.focus("report.pdf");
    let said = s.command(ids::VERIFY_SIGNATURE);
    assert!(
        said.contains(&"No signature beside report.pdf".to_owned()),
        "{said:?}"
    );
}

#[test]
fn a_minisign_signature_asks_its_public_key() {
    let (_dir, work, mut s) = plain();
    fs::write(work.join("notes.txt.minisig"), "untrusted comment: x\n").unwrap();
    s.command(ids::RELOAD);
    s.focus("notes.txt");
    s.command(ids::VERIFY_SIGNATURE);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("minisign public key: a file or pasted")
    );
    let said = s.key("enter");
    assert!(
        said.contains(&"No key given; nothing was checked.".to_owned()),
        "{said:?}"
    );
    // A key file that is not a key: the verdict says it, in words.
    fs::write(work.join("minisign.pub"), "not a key\n").unwrap();
    s.command(ids::VERIFY_SIGNATURE);
    s.type_text("minisign.pub");
    let said = s.key("enter");
    let all = s.said.iter().map(|m| m.text.clone()).collect::<Vec<_>>();
    assert!(
        all.iter().any(|l| l.starts_with("Cannot check")),
        "{said:?} {all:?}"
    );
    assert!(s.tools.ran().is_empty(), "minisign never runs gpg");
}

/// The SHA-256 of "abc".
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// A folder with `example-tool.zip` ("abc"), its line in `SHA256SUMS`,
/// and `SHA256SUMS.asc` when asked; Verify checksum run on the zip, its
/// rows returned.
fn chain_rows(signed: bool, gpg: Option<&[u8]>) -> Vec<String> {
    let (_dir, work, mut s) = plain();
    fs::write(work.join("example-tool.zip"), b"abc").unwrap();
    fs::write(
        work.join("SHA256SUMS"),
        format!("{ABC}  example-tool.zip\n"),
    )
    .unwrap();
    if signed {
        fs::write(
            work.join("SHA256SUMS.asc"),
            "-----BEGIN PGP SIGNATURE-----\n",
        )
        .unwrap();
    }
    if let Some(status) = gpg {
        s.tools.answer("gpg", status);
    }
    s.command(ids::RELOAD);
    s.focus("example-tool.zip");
    s.command(ids::VERIFY_CHECKSUM);
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => {
            let mut rows = vec![list.title().to_owned()];
            rows.extend(list.rows().iter().map(|r| r.label.clone()));
            rows
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn verify_checksum_says_the_chain_over_the_lists_signature() {
    let good = b"[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG 168B6F650E9C8469 Ada Example <ada@example.invalid>\n[GNUPG:] VALIDSIG x\n[GNUPG:] TRUST_ULTIMATE 0 pgp\n";
    assert_eq!(
        chain_rows(true, Some(good)),
        [
            "1 file matches SHA256SUMS",
            "List is signed: 1 of 1 files match",
            "Signature good: key ending 8469",
            "Key ending 8469 is named Ada Example <ada@example.invalid>",
            "Good means the key fits, not who holds it.",
            "Show the 1 match",
        ]
    );
    let bad = b"[GNUPG:] NEWSIG\n[GNUPG:] BADSIG 168B6F650E9C8469 Ada Example\n";
    assert_eq!(
        chain_rows(true, Some(bad)),
        [
            "1 file matches SHA256SUMS",
            "Signature bad: the file was changed",
            "List not verified: 1 of 1 match",
            "Show the 1 match",
        ]
    );
    // A signature with no gpg to check it is not verified, in words.
    let rows = chain_rows(true, None);
    assert_eq!(rows[1], "gpg not found. Install GnuPG first.");
    assert_eq!(rows[2], "List not verified: 1 of 1 match");
    // No signature: what a match shows, and no more.
    assert_eq!(
        chain_rows(false, None)[1..3],
        [
            "List not signed: 1 of 1 match",
            "This shows the files are not damaged. It does not show who made them.",
        ]
    );
    for row in chain_rows(true, Some(good)) {
        assert!(!row.contains("safe") && !row.contains("genuine"), "{row}");
    }
}
