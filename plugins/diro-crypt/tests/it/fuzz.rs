//! The fuzz targets' bodies replayed in the normal tests: seeds made here
//! at test time (no fixture files), every cut of each, copies with bytes
//! bent, and random bytes. None may panic.

use proptest::prelude::*;

use diro_crypt::fuzzing;

type Body = fn(&[u8]);

const TARGETS: [(&str, Body); 3] = [
    ("recipients", fuzzing::recipients),
    ("form_answer", fuzzing::form_answer),
    ("header", fuzzing::header),
];

fn seeds(target: &str) -> Vec<Vec<u8>> {
    let key = age::x25519::Identity::generate().to_public().to_string();
    match target {
        "recipients" => vec![
            format!("# a team\n{key}\n\n{key}\n").into_bytes(),
            b"ssh-ed25519 AAAAC3Nza ada@example.invalid\n".to_vec(),
            b"age1yubikey1qqqq\nage1tag1qqqq\n".to_vec(),
            "age1\u{200B}x\r\n#\u{FEFF}\n".as_bytes().to_vec(),
        ],
        "form_answer" => vec![
            br#"{"passphrase": "a", "repeat": "a", "armor": true}"#.to_vec(),
            br#"{"passphrase": "a", "repeat": "b"}"#.to_vec(),
            br#"{"passphrase": 7, "repeat": null, "armor": "yes"}"#.to_vec(),
            format!(r#"{{"key": "{key}", "file": "", "armor": false}}"#).into_bytes(),
            br#"{"key": ["x"], "file": {"a": 1}}"#.to_vec(),
        ],
        _ => {
            let pass = age::secrecy::SecretString::from("thrown-away-fuzz-seed".to_owned());
            let mut recipient = age::scrypt::Recipient::new(pass);
            recipient.set_work_factor(2);
            let binary = age::encrypt(&recipient, b"seed").unwrap();
            let armored = age::encrypt_and_armor(&recipient, b"seed")
                .unwrap()
                .into_bytes();
            let x25519 =
                age::encrypt(&age::x25519::Identity::generate().to_public(), b"seed").unwrap();
            vec![
                binary,
                armored,
                x25519,
                b"age-encryption.org/v1\n-> scrypt c2FsdA 99\n--- x\n".to_vec(),
                b"-----BEGIN AGE ENCRYPTED FILE-----\n!!!!\n".to_vec(),
                b"report.pdf.age".to_vec(),
            ]
        }
    }
}

#[test]
fn every_seed_cut_and_bent_reads_without_a_panic() {
    for (target, body) in TARGETS {
        let seeds = seeds(target);
        assert!(!seeds.is_empty(), "{target} has no seeds");
        for seed in &seeds {
            body(seed);
            for cut in 0..seed.len().min(400) {
                body(&seed[..cut]);
            }
            for at in (0..seed.len()).step_by(7) {
                let mut bent = seed.clone();
                bent[at] ^= 0xA5;
                body(&bent);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes_read_without_a_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        for (_, body) in TARGETS {
            body(&bytes);
        }
    }
}
