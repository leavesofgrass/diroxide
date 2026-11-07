//! The canary test (R9): passphrases and a secret key made for the test
//! go through every flow, right and wrong, and none of them is ever in a
//! line the plugin wrote to the host: no sentence, no progress, no form,
//! no question, no answer.

use age::secrecy::ExposeSecret;
use serde_json::json;

use crate::support::{FakeHost, canary, file, scratch};

#[test]
fn no_secret_ever_reaches_the_host() {
    let dir = scratch();
    file(dir.path(), "a.txt", 3000);
    file(dir.path(), "b.txt", 3000);
    let right = canary("right");
    let wrong = canary("wrong");
    let differ = canary("differ");
    let key = age::x25519::Identity::generate();
    let key_text = key.to_string().expose_secret().to_owned();
    let identity = dir.path().join("me.key");
    std::fs::write(&identity, format!("{key_text}\n")).unwrap();

    let mut host = FakeHost::start();
    // Encrypt a.txt: a try that differs, then the right one twice.
    host.answers
        .push_back(json!({"values": {"passphrase": right, "repeat": differ}}));
    host.passphrase_form(&right, false);
    host.command("encrypt_passphrase", dir.path(), &["a.txt"])
        .unwrap();
    // Encrypt b.txt for the key.
    host.answers.push_back(
        json!({"values": {"key": key.to_public().to_string(), "file": "", "armor": true}}),
    );
    host.command("encrypt_recipient", dir.path(), &["b.txt"])
        .unwrap();
    // Decrypt both: a wrong passphrase, the right one, the identity file.
    host.secret(&wrong);
    host.secret(&right);
    host.answers
        .push_back(json!({"text": identity.display().to_string()}));
    let answer = host
        .command("decrypt", dir.path(), &["a.txt.age", "b.txt.age"])
        .unwrap();
    assert_eq!(answer["done"], 2, "{}", host.seen.all_said());

    let secrets = [
        right.as_str(),
        wrong.as_str(),
        differ.as_str(),
        key_text.as_str(),
    ];
    assert!(!host.seen.wire.is_empty());
    for line in &host.seen.wire {
        for secret in secrets {
            assert!(!line.contains(secret), "a secret reached the host: {line}");
        }
        assert!(!line.contains("AGE-SECRET-KEY"), "{line}");
    }
    // Nor did one land in a file name or a file in clear.
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "me.key" {
            continue;
        }
        let bytes = std::fs::read(entry.path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for secret in secrets {
            assert!(!name.contains(secret) && !text.contains(secret), "{name}");
        }
    }
}
