//! Decrypt, through the fake host: the header decides the question, a
//! wrong passphrase is asked again three times at most, a hostile work
//! factor is refused before anything is asked, and an identity file is
//! read for the command only.

use std::path::Path;

use age::secrecy::{ExposeSecret, SecretString};
use serde_json::json;

use diro_crypt::lines;

use crate::support::{FakeHost, canary, file, names_in, scratch};

/// A file encrypted with a passphrase at a low work factor, as a test
/// needs it quickly.
fn sealed(folder: &Path, name: &str, body: &[u8], pass: &str, factor: u8) {
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(pass.to_owned()));
    recipient.set_work_factor(factor);
    let made = age::encrypt(&recipient, body).unwrap();
    std::fs::write(folder.join(name), made).unwrap();
}

#[test]
fn the_passphrase_question_names_the_file_and_one_answer_opens_two() {
    let dir = scratch();
    let pass = canary("dec");
    sealed(dir.path(), "report.pdf.age", b"report", &pass, 12);
    sealed(dir.path(), "notes.txt.age", b"notes", &pass, 12);
    let mut host = FakeHost::start();
    host.secret(&pass);
    let answer = host
        .command("decrypt", dir.path(), &["report.pdf.age", "notes.txt.age"])
        .unwrap();
    assert_eq!(answer["done"], 2, "{}", host.seen.all_said());
    assert_eq!(
        host.seen.asked.len(),
        1,
        "the second file is opened by the first answer"
    );
    let asked = &host.seen.asked[0];
    assert_eq!(asked["kind"], "secret");
    assert_eq!(asked["title"], "Passphrase for report.pdf.age");
    assert_eq!(
        host.seen.sentences(),
        [
            "Decrypted report.pdf.age to report.pdf.",
            "Decrypted notes.txt.age to notes.txt.",
            "Decrypted 2 of 2 files",
            lines::NOT_PROOF,
        ]
    );
    assert_eq!(
        std::fs::read(dir.path().join("report.pdf")).unwrap(),
        b"report"
    );
    assert_eq!(
        names_in(dir.path()),
        ["notes.txt", "notes.txt.age", "report.pdf", "report.pdf.age"]
    );
}

#[test]
fn a_wrong_passphrase_is_asked_again_with_the_try_said() {
    let dir = scratch();
    let pass = canary("right");
    sealed(dir.path(), "a.age", b"plain", &pass, 12);
    let mut host = FakeHost::start();
    host.secret(&canary("wrong1"));
    host.secret(&pass);
    let answer = host.command("decrypt", dir.path(), &["a.age"]).unwrap();
    assert_eq!(answer["done"], 1);
    assert_eq!(host.seen.asked.len(), 2);
    assert_eq!(
        host.seen.asked[1]["message"],
        json!(["Wrong passphrase for a.age. Try again, 2 of 3."])
    );
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"plain");

    let mut host = FakeHost::start();
    for n in 0..3 {
        host.secret(&canary(&format!("wrong{n}")));
    }
    let answer = host.command("decrypt", dir.path(), &["a.age"]).unwrap();
    assert_eq!(answer["done"], 0);
    assert_eq!(answer["failed"], json!(["a.age"]));
    assert!(host.seen.heard("Gave up on a.age: 3 wrong tries"));
    assert_eq!(host.seen.asked.len(), 3);
}

#[test]
fn a_hostile_work_factor_is_refused_before_anything_is_asked() {
    let dir = scratch();
    sealed(dir.path(), "plain.age", b"x", &canary("hostile"), 12);
    // The same header, asking for factor 30: hours and gigabytes.
    let mut bytes = std::fs::read(dir.path().join("plain.age")).unwrap();
    let text = String::from_utf8_lossy(&bytes[..100]).into_owned();
    let at = text.find(" 12\n").unwrap();
    bytes[at + 1..at + 3].copy_from_slice(b"30");
    std::fs::write(dir.path().join("hostile.age"), &bytes).unwrap();
    let mut host = FakeHost::start();
    let start = std::time::Instant::now();
    let answer = host
        .command("decrypt", dir.path(), &["hostile.age"])
        .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(host.seen.asked.is_empty(), "nothing is asked");
    assert_eq!(answer["failed"], json!(["hostile.age"]));
    assert_eq!(
        host.seen.sentences()[..2],
        [
            "Could not decrypt hostile.age",
            "It asks for more work than diroxide allows. It may be hostile."
        ]
    );
}

#[test]
fn a_file_that_is_not_age_and_escape_are_said() {
    let dir = scratch();
    file(dir.path(), "photo.jpg", 100);
    sealed(dir.path(), "b.age", b"b", &canary("esc"), 12);
    let mut host = FakeHost::start();
    host.escape();
    let answer = host
        .command("decrypt", dir.path(), &["photo.jpg", "b.age"])
        .unwrap();
    assert_eq!(answer["done"], 0);
    assert_eq!(answer["skipped"], 1);
    assert_eq!(
        host.seen.sentences()[..3],
        [
            "Could not decrypt photo.jpg",
            "It is not an age file.",
            "Skipped b.age: no passphrase given",
        ]
    );
    assert_eq!(names_in(dir.path()), ["b.age", "photo.jpg"]);
}

#[test]
fn an_identity_file_is_asked_by_path_and_one_under_a_passphrase_too() {
    let dir = scratch();
    let key = age::x25519::Identity::generate();
    let made = age::encrypt(&key.to_public(), b"for the key").unwrap();
    std::fs::write(dir.path().join("k1.age"), &made).unwrap();
    std::fs::write(dir.path().join("k2.age"), &made).unwrap();
    let keys = dir.path().join("keys");
    std::fs::create_dir(&keys).unwrap();
    let plain_identity = keys.join("key.txt");
    std::fs::write(
        &plain_identity,
        format!("# a test key\n{}\n", key.to_string().expose_secret()),
    )
    .unwrap();
    let mut host = FakeHost::start();
    host.answers
        .push_back(json!({"text": plain_identity.display().to_string()}));
    let answer = host
        .command("decrypt", dir.path(), &["k1.age", "k2.age"])
        .unwrap();
    assert_eq!(answer["done"], 2, "{}", host.seen.all_said());
    assert_eq!(host.seen.asked.len(), 1);
    assert_eq!(host.seen.asked[0]["kind"], "path");
    assert_eq!(host.seen.asked[0]["title"], "Identity file for k1.age");
    assert_eq!(
        std::fs::read(dir.path().join("k1")).unwrap(),
        b"for the key"
    );

    // The identity file itself under a passphrase, armored as age writes
    // one.
    let pass = canary("idpass");
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(pass.clone()));
    recipient.set_work_factor(12);
    let identity_text = format!("{}\n", key.to_string().expose_secret());
    let locked = age::encrypt_and_armor(&recipient, identity_text.as_bytes()).unwrap();
    let locked_identity = keys.join("key.age");
    std::fs::write(&locked_identity, locked).unwrap();
    let mut host = FakeHost::start();
    host.answers
        .push_back(json!({"text": locked_identity.display().to_string()}));
    host.secret(&pass);
    let answer = host.command("decrypt", dir.path(), &["k1.age"]).unwrap();
    assert_eq!(answer["done"], 1, "{}", host.seen.all_said());
    assert_eq!(host.seen.asked[1]["title"], "Passphrase for key.age");
    assert!(host.seen.heard("Decrypted k1.age to k1 (2)."));

    // A file that holds no key.
    let mut host = FakeHost::start();
    host.answers
        .push_back(json!({"text": dir.path().join("k1").display().to_string()}));
    let answer = host.command("decrypt", dir.path(), &["k2.age"]).unwrap();
    assert_eq!(answer["failed"], json!(["k2.age"]));
    assert!(
        host.seen
            .heard("Identity file unusable: it holds no age key diroxide can use"),
        "{}",
        host.seen.all_said()
    );
}
