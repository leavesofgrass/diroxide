//! Encrypt with a passphrase and Encrypt for a recipient, through the
//! fake host.

use std::io::Read;

use serde_json::json;

use diro_crypt::lines;

use crate::support::{FakeHost, canary, file, names_in, scratch};

#[test]
fn a_passphrase_asked_twice_encrypts_each_file_beside_it() {
    let dir = scratch();
    file(dir.path(), "a.txt", 10);
    file(dir.path(), "b.txt", 2000);
    let pass = canary("enc");
    let mut host = FakeHost::start();
    host.passphrase_form(&pass, false);
    let answer = host
        .command("encrypt_passphrase", dir.path(), &["a.txt", "b.txt"])
        .unwrap();
    assert_eq!(answer["done"], 2, "{}", host.seen.all_said());
    assert_eq!(answer["failed"], json!([]));
    assert_eq!(
        answer["made"][0]["to"],
        dir.path().join("a.txt.age").display().to_string()
    );
    assert_eq!(
        answer["made"][0]["from"],
        dir.path().join("a.txt").display().to_string()
    );
    assert_eq!(
        names_in(dir.path()),
        ["a.txt", "a.txt.age", "b.txt", "b.txt.age"]
    );
    // Each output went through diroxide's stage and commit, so it joins
    // the command's Undo; no stage is left open.
    assert_eq!(
        host.seen.committed,
        [dir.path().join("a.txt.age"), dir.path().join("b.txt.age")]
    );
    assert!(host.seen.open_stages.is_empty());
    assert!(
        host.seen
            .wire
            .iter()
            .any(|line| line.contains("\"fs/stage\"")),
        "the plugin asked for a stage"
    );
    // The form: two hidden fields and the armor box, the plan first.
    let form = &host.seen.forms[0];
    assert_eq!(form["title"], lines::PASSPHRASE_TITLE);
    assert_eq!(form["fields"][0]["kind"], "secret");
    assert_eq!(form["fields"][1]["kind"], "secret");
    assert_eq!(form["fields"][2]["kind"], "checkbox");
    let said = host.seen.sentences();
    assert_eq!(
        said,
        [
            "Encrypt 2 files with a passphrase",
            "Encrypted a.txt to a.txt.age",
            "Encrypted b.txt to b.txt.age",
            "Encrypted 2 of 2 files",
            lines::ORIGINAL_STAYS,
        ]
    );
    // The file is age's, at the reference work factor.
    let made = std::fs::read(dir.path().join("b.txt.age")).unwrap();
    assert_eq!(
        diro_crypt::header::peek(&made).scrypt,
        Some(diro_crypt::header::WORK_FACTOR)
    );
    // And it opens with the passphrase, through age itself.
    let identity = age::scrypt::Identity::new(age::secrecy::SecretString::from(pass.clone()));
    let plain = age::decrypt(&identity, &made).unwrap();
    assert_eq!(plain, std::fs::read(dir.path().join("b.txt")).unwrap());
}

#[test]
fn two_passphrases_that_differ_are_asked_again_three_times() {
    let dir = scratch();
    file(dir.path(), "a.txt", 10);
    let mut host = FakeHost::start();
    for n in 0..3 {
        host.answers.push_back(json!({"values":
            {"passphrase": canary(&format!("one{n}")), "repeat": canary(&format!("two{n}"))}}));
    }
    let answer = host
        .command("encrypt_passphrase", dir.path(), &["a.txt"])
        .unwrap();
    assert_eq!(answer["done"], 0);
    assert_eq!(host.seen.forms.len(), 3);
    assert!(
        host.seen.forms[1]["summary"]
            .as_str()
            .unwrap()
            .starts_with(lines::PASSPHRASES_DIFFER)
    );
    assert!(host.seen.heard(lines::PASSPHRASES_DIFFER));
    assert_eq!(names_in(dir.path()), ["a.txt"]);
}

#[test]
fn an_empty_passphrase_and_escape_make_nothing() {
    let dir = scratch();
    file(dir.path(), "a.txt", 10);
    let mut host = FakeHost::start();
    host.answers
        .push_back(json!({"values": {"passphrase": "", "repeat": ""}}));
    host.escape();
    let answer = host
        .command("encrypt_passphrase", dir.path(), &["a.txt"])
        .unwrap();
    assert_eq!(answer["done"], 0);
    assert!(
        host.seen.forms[1]["summary"]
            .as_str()
            .unwrap()
            .starts_with(lines::PASSPHRASE_EMPTY)
    );
    assert_eq!(host.seen.sentences(), [lines::CANCELLED]);
    assert_eq!(names_in(dir.path()), ["a.txt"]);
}

#[test]
fn folders_and_missing_files_are_said_before_anything_is_asked() {
    let dir = scratch();
    std::fs::create_dir(dir.path().join("Photos")).unwrap();
    let mut host = FakeHost::start();
    let answer = host
        .command("encrypt_passphrase", dir.path(), &["Photos/", "gone.txt"])
        .unwrap();
    assert_eq!(answer["skipped"], 2);
    assert!(host.seen.forms.is_empty(), "nothing is asked");
    assert_eq!(
        host.seen.sentences(),
        [
            "Skipped Photos: it is a folder",
            lines::PACK_FIRST,
            "Skipped gone.txt: it is not there",
        ]
    );
    let mut host = FakeHost::start();
    let answer = host.command("encrypt_recipient", dir.path(), &[]).unwrap();
    assert_eq!(answer["done"], 0);
    assert_eq!(host.seen.sentences(), [lines::NOTHING_MARKED]);
}

#[test]
fn a_pasted_key_and_a_recipients_file_encrypt_for_both() {
    let dir = scratch();
    file(dir.path(), "report.pdf", 5000);
    let one = age::x25519::Identity::generate();
    let two = age::x25519::Identity::generate();
    let list = dir.path().join("team.txt");
    std::fs::write(&list, format!("# the team\n{}\n", two.to_public())).unwrap();
    let mut host = FakeHost::start();
    // First a key that is not one: the form comes back with the reason.
    host.answers
        .push_back(json!({"values": {"key": "age1notakey", "file": ""}}));
    host.answers.push_back(json!({"values": {
        "key": one.to_public().to_string(),
        "file": list.display().to_string(),
        "armor": true,
    }}));
    let answer = host
        .command("encrypt_recipient", dir.path(), &["report.pdf"])
        .unwrap();
    assert_eq!(answer["done"], 1, "{}", host.seen.all_said());
    assert!(
        host.seen.forms[1]["summary"]
            .as_str()
            .unwrap()
            .starts_with("Not an age public key: line 1 in the pasted text.")
    );
    assert_eq!(
        host.seen.sentences()[..2],
        [
            "Encrypt 1 file for 2 recipients",
            "Encrypted report.pdf to report.pdf.age"
        ]
    );
    let made = std::fs::read(dir.path().join("report.pdf.age")).unwrap();
    assert!(made.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));
    for key in [one, two] {
        let mut reader = age::Decryptor::new(age::armor::ArmoredReader::new(made.as_slice()))
            .unwrap()
            .decrypt(std::iter::once(&key as &dyn age::Identity))
            .unwrap();
        let mut plain = Vec::new();
        reader.read_to_end(&mut plain).unwrap();
        assert_eq!(plain.len(), 5000);
    }
}

#[test]
fn a_taken_name_is_numbered_and_nothing_is_replaced() {
    let dir = scratch();
    file(dir.path(), "a.txt", 10);
    std::fs::write(dir.path().join("a.txt.age"), b"someone else's").unwrap();
    let mut host = FakeHost::start();
    host.passphrase_form(&canary("taken"), false);
    host.command("encrypt_passphrase", dir.path(), &["a.txt"])
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("a.txt.age")).unwrap(),
        b"someone else's"
    );
    assert_eq!(
        names_in(dir.path()),
        ["a.txt", "a.txt (2).age", "a.txt.age"]
    );
    assert!(host.seen.heard("Encrypted a.txt to a.txt (2).age"));
}

#[test]
fn a_large_file_reports_its_progress_in_words() {
    let dir = scratch();
    file(dir.path(), "backup.tar", 3 * 1024 * 1024);
    let mut host = FakeHost::start();
    host.passphrase_form(&canary("big"), false);
    host.command("encrypt_passphrase", dir.path(), &["backup.tar"])
        .unwrap();
    let reports: Vec<&str> = host
        .seen
        .progress
        .iter()
        .filter(|v| v["kind"] == "report")
        .filter_map(|v| v["message"].as_str())
        .collect();
    assert!(
        reports.contains(&"Encrypting 40 percent of backup.tar"),
        "{reports:?}"
    );
    assert_eq!(host.seen.progress[0]["kind"], "begin");
    assert_eq!(host.seen.progress.last().unwrap()["kind"], "end");
}
