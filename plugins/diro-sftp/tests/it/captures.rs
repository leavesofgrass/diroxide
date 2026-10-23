//! ssh's real words, captured from OpenSSH 10.0p2 against the container's
//! server (R25 commit 76; `DIRO_SFTP_CAPTURE=1 docker/sftp/run-tests.sh`),
//! read on every system: each refusal classifies as the table says, and
//! each prompt as the askpass classifier says.

use diro_sftp::askpass::Prompt;
use diro_sftp::askpass::classify::{PromptKind, classify as prompt_kind};
use diro_sftp::ssh::stderr::{Cause, classify};

macro_rules! capture {
    ($folder:literal, $name:literal) => {
        include_str!(concat!(
            "../../../../fixtures/sftp/",
            $folder,
            "/",
            $name,
            "-linux-10.0p2.txt"
        ))
    };
}

#[test]
fn the_real_refusals_classify_as_the_table_says() {
    for (text, cause) in [
        (
            capture!("stderr", "connection-refused"),
            Cause::ConnectionRefused,
        ),
        (
            capture!("stderr", "host-key-verification-failed"),
            Cause::KeyNotTrusted,
        ),
        (capture!("stderr", "changed-host-key"), Cause::ChangedKey),
        (
            capture!("stderr", "permission-denied"),
            Cause::SignInRefused,
        ),
        (
            capture!("stderr", "too-many-authentication-failures"),
            Cause::SignInRefused,
        ),
        (capture!("stderr", "no-subsystem"), Cause::NoSubsystem),
        (capture!("stderr", "bad-config"), Cause::BadConfig),
        (capture!("stderr", "banner-timed-out"), Cause::TimedOut),
        (
            capture!("stderr", "closed-by-server"),
            Cause::ClosedByServer,
        ),
        // Said as "Connection lost", which is what it is.
        (capture!("stderr", "keepalive-timed-out"), Cause::Unknown),
    ] {
        assert!(!text.trim().is_empty());
        assert_eq!(classify(text), cause, "{text}");
    }
}

fn prompt(text: &str) -> PromptKind {
    prompt_kind(&Prompt {
        raw: text.trim_end_matches('\n').as_bytes().to_vec(),
        kind: None,
    })
}

#[test]
fn the_real_prompts_classify_as_ssh_means_them() {
    assert_eq!(
        prompt(capture!("prompts", "password")),
        PromptKind::HostPassword {
            user: "builder".into(),
            host: "127.0.0.1".into()
        }
    );
    assert!(matches!(
        prompt(capture!("prompts", "key-passphrase")),
        PromptKind::KeyPassphrase { key } if key == "id_pass"
    ));
    // A keyboard-interactive question names the account in brackets first,
    // so it is never taken for ssh's own password prompt.
    assert!(matches!(
        prompt(capture!("prompts", "keyboard-interactive")),
        PromptKind::Server { .. }
    ));
    assert!(matches!(
        prompt(capture!("prompts", "new-host-key")),
        PromptKind::NewHostKey { key_type, .. } if key_type == "ED25519"
    ));
}
