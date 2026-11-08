//! gpg's argument lists and the reading of what it said, by R29 3.3's
//! rules and spike ES3's findings (ADR-0015):
//!
//! - The common prefix, every run: `--batch --no-tty --status-fd 1
//!   --no-greeting --disable-dirmngr --no-auto-key-locate`, so gpg never
//!   asks on a terminal and never looks a key up on a server (a lookup
//!   sends an address away).
//! - A passphrase only on standard input (`--pinentry-mode loopback
//!   --passphrase-fd 0`), never in an argument or the environment.
//! - The input always a file, after `--`, so a name starting with a dash
//!   is never an option; the output always a fresh name with `--output`,
//!   never `--yes`, so gpg refuses to write over anything.
//! - Only `[GNUPG:]` lines are read ([`super::status`]); `--trust-model
//!   always` is never added.
//! - A decryption and a signature go in two steps: `--pinentry-mode
//!   error` first, so a key the owner's own agent holds never asks; when
//!   that fails with GnuPG's "no pinentry" (code 85), the masked prompt
//!   asks and the run is made again in loopback. gpg 2.4 prints no
//!   `NEED_PASSPHRASE` for a key (ES3), so the key's id comes from
//!   `ENC_TO` or `KEY_CONSIDERED`.

use std::ffi::OsString;
use std::path::Path;

use diroxide_core::Secret;
use diroxide_vfs::ToolInput;

use super::status::{self, BAD_PASSPHRASE, NO_PINENTRY, NO_SECRET_KEY, Status};

/// The tools registry's id for gpg.
pub const GPG: &str = "gpg";

/// The arguments every run starts with.
pub const PREFIX: [&str; 7] = [
    "--batch",
    "--no-tty",
    "--status-fd",
    "1",
    "--no-greeting",
    "--disable-dirmngr",
    "--no-auto-key-locate",
];

/// How a step gets a passphrase: not at all (a cached key or none
/// needed), or from standard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pinentry {
    /// `--pinentry-mode error`: the agent's cache or nothing.
    Error,
    /// `--pinentry-mode loopback --passphrase-fd 0`: the line on
    /// standard input.
    Loopback,
}

fn start() -> Vec<OsString> {
    PREFIX.iter().map(OsString::from).collect()
}

fn pinentry(args: &mut Vec<OsString>, mode: Pinentry) {
    match mode {
        Pinentry::Error => args.extend(["--pinentry-mode", "error"].map(OsString::from)),
        Pinentry::Loopback => {
            args.extend(["--pinentry-mode", "loopback", "--passphrase-fd", "0"].map(OsString::from))
        }
    }
}

fn ends(args: &mut Vec<OsString>, output: &Path, input: &Path, armor: bool) {
    if armor {
        args.push("--armor".into());
    }
    args.push("--output".into());
    args.push(output.as_os_str().to_owned());
    args.push("--".into());
    args.push(input.as_os_str().to_owned());
}

/// Encrypts to recipients: their key ids, fingerprints or addresses as
/// the owner's keyring knows them. No passphrase is needed.
pub fn encrypt_args(
    recipients: &[String],
    armor: bool,
    output: &Path,
    input: &Path,
) -> Vec<OsString> {
    let mut args = start();
    args.push("--encrypt".into());
    for recipient in recipients {
        args.push("--recipient".into());
        args.push(recipient.into());
    }
    ends(&mut args, output, input, armor);
    args
}

/// Encrypts with a passphrase, AES-256, the passphrase on standard input.
pub fn symmetric_args(armor: bool, output: &Path, input: &Path) -> Vec<OsString> {
    let mut args = start();
    pinentry(&mut args, Pinentry::Loopback);
    args.extend(["--symmetric", "--cipher-algo", "AES256"].map(OsString::from));
    ends(&mut args, output, input, armor);
    args
}

/// Decrypts, in the given step.
pub fn decrypt_args(mode: Pinentry, output: &Path, input: &Path) -> Vec<OsString> {
    let mut args = start();
    pinentry(&mut args, mode);
    args.push("--decrypt".into());
    ends(&mut args, output, input, false);
    args
}

/// Makes a detached signature, with the key given or the default one.
pub fn sign_args(
    mode: Pinentry,
    key: Option<&str>,
    armor: bool,
    output: &Path,
    input: &Path,
) -> Vec<OsString> {
    let mut args = start();
    pinentry(&mut args, mode);
    args.push("--detach-sign".into());
    if let Some(key) = key {
        args.push("--local-user".into());
        args.push(key.into());
    }
    ends(&mut args, output, input, armor);
    args
}

/// Verifies a detached signature against its file, or an inline signed
/// file when `data` is `None`.
pub fn verify_args(signature: &Path, data: Option<&Path>) -> Vec<OsString> {
    let mut args = start();
    args.push("--verify".into());
    args.push("--".into());
    args.push(signature.as_os_str().to_owned());
    if let Some(data) = data {
        args.push(data.as_os_str().to_owned());
    }
    args
}

/// A passphrase as gpg's standard input: its bytes and a line feed, as a
/// secret input, hidden from Debug and wiped when dropped (E's request 5).
pub fn passphrase_input(secret: &Secret) -> ToolInput {
    ToolInput::secret_line(secret)
}

/// The locale every gpg run gets, so its status words and messages come
/// out the same whatever the computer's language (E's request 6).
pub const GPG_LOCALE: (&str, &str) = ("LC_ALL", "C");

/// Whether a recipient's text can be given to gpg: not empty, no control
/// characters, not over 256 characters, and not starting with a dash.
pub fn recipient_ok(recipient: &str) -> bool {
    let recipient = recipient.trim();
    !recipient.is_empty()
        && recipient.chars().count() <= 256
        && !recipient.starts_with('-')
        && !recipient.chars().any(char::is_control)
}

/// What a run of gpg came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Done: the output is written.
    Done,
    /// A passphrase is needed: for a key (its ending, when gpg said it),
    /// or for a passphrase-encrypted file.
    NeedsPassphrase {
        /// The key's last four hexadecimal digits.
        key: Option<String>,
        /// Whether the file was encrypted with a passphrase, not a key.
        symmetric: bool,
    },
    /// The passphrase given was wrong.
    WrongPassphrase,
    /// No secret key for it.
    NoSecretKey {
        /// The key's ending, when gpg said it.
        key: Option<String>,
    },
    /// A recipient gpg will not encrypt to: not certified by the owner.
    NotCertified,
    /// A recipient gpg did not find.
    RecipientNotFound(String),
    /// Not OpenPGP data.
    NotOpenPgp,
    /// Anything else, in gpg's own first words or the exit code.
    Failed(String),
}

/// The key a decryption was for: `ENC_TO`'s key id.
fn enc_to(lines: &[Status]) -> Option<String> {
    status::find(lines, "ENC_TO").and_then(|s| status::key_ending(s.arg(0)))
}

/// The key a signature was made with: `KEY_CONSIDERED`'s fingerprint.
fn considered(lines: &[Status]) -> Option<String> {
    status::find(lines, "KEY_CONSIDERED").and_then(|s| status::key_ending(s.arg(0)))
}

/// The first line of gpg's standard error, cut, for a failure gpg alone
/// can explain; its words are its own, so they are cleaned.
pub fn first_words(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.strip_prefix("gpg: ").unwrap_or(line);
    Some(super::lines::clean_text(line))
}

fn failed(code: Option<i32>, stderr: &[u8]) -> Outcome {
    Outcome::Failed(first_words(stderr).unwrap_or_else(|| match code {
        Some(code) => format!("gpg ended with code {code}"),
        None => "gpg was stopped".to_owned(),
    }))
}

/// Reads an encryption's run.
pub fn read_encrypt(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Outcome {
    let lines = status::parse(stdout);
    if let Some(invalid) = status::find(&lines, "INV_RECP") {
        // Reason 10: "key not trusted", which in batch mode means the
        // owner has not certified it; 1: "not found".
        return match invalid.arg(0) {
            "10" => Outcome::NotCertified,
            _ => Outcome::RecipientNotFound(invalid.rest(1)),
        };
    }
    if code == Some(0) && status::has(&lines, "END_ENCRYPTION") {
        return Outcome::Done;
    }
    if status::has_code(&lines, BAD_PASSPHRASE) {
        return Outcome::WrongPassphrase;
    }
    failed(code, stderr)
}

/// Reads a decryption's run.
pub fn read_decrypt(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Outcome {
    let lines = status::parse(stdout);
    if code == Some(0) && status::has(&lines, "DECRYPTION_OKAY") {
        return Outcome::Done;
    }
    let symmetric = status::has(&lines, "NEED_PASSPHRASE_SYM");
    if status::has_code(&lines, NO_PINENTRY) {
        return Outcome::NeedsPassphrase {
            key: if symmetric { None } else { enc_to(&lines) },
            symmetric,
        };
    }
    if status::has_code(&lines, BAD_PASSPHRASE) {
        return Outcome::WrongPassphrase;
    }
    if status::has(&lines, "NO_SECKEY") || status::has_code(&lines, NO_SECRET_KEY) {
        return Outcome::NoSecretKey {
            key: status::find(&lines, "NO_SECKEY")
                .and_then(|s| status::key_ending(s.arg(0)))
                .or_else(|| enc_to(&lines)),
        };
    }
    if status::has(&lines, "NODATA") {
        return Outcome::NotOpenPgp;
    }
    failed(code, stderr)
}

/// Reads a detached signature's run.
pub fn read_sign(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Outcome {
    let lines = status::parse(stdout);
    if code == Some(0) && status::has(&lines, "SIG_CREATED") {
        return Outcome::Done;
    }
    if status::has_code(&lines, NO_PINENTRY) {
        return Outcome::NeedsPassphrase {
            key: considered(&lines),
            symmetric: false,
        };
    }
    if status::has_code(&lines, BAD_PASSPHRASE) {
        return Outcome::WrongPassphrase;
    }
    if status::has(&lines, "INV_SGNR") || status::has_code(&lines, NO_SECRET_KEY) {
        return Outcome::NoSecretKey {
            key: status::find(&lines, "INV_SGNR").and_then(|s| status::key_ending(s.arg(1))),
        };
    }
    failed(code, stderr)
}

/// The trust gpg gave a good signature's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The owner's own key, or one the owner fully trusts.
    Certified,
    /// A key the owner has not certified (`TRUST_UNDEFINED`,
    /// `TRUST_NEVER`, `TRUST_MARGINAL`), or no trust line.
    NotCertified,
}

/// What gpg's `--verify` said, as one verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verified {
    /// GOODSIG: the signature fits the key.
    Good {
        /// The key's ending.
        key: String,
        /// The key's own text, as its owner chose it.
        name: String,
        /// Whether the owner certified the key.
        trust: Trust,
    },
    /// BADSIG.
    Bad {
        /// The key's ending.
        key: Option<String>,
    },
    /// ERRSIG with NO_PUBKEY: the public key is not in the keyring.
    MissingKey {
        /// The key's ending.
        key: Option<String>,
    },
    /// EXPKEYSIG: good, but the key expired.
    KeyExpired {
        /// The key's ending.
        key: Option<String>,
    },
    /// EXPSIG: good, but the signature expired.
    SignatureExpired {
        /// The key's ending.
        key: Option<String>,
    },
    /// REVKEYSIG: good, but the key is revoked.
    Revoked {
        /// The key's ending.
        key: Option<String>,
    },
    /// No signature there (NODATA), or one gpg cannot check (ERRSIG for
    /// another reason).
    Unreadable,
}

/// Reads a `--verify` run's status words into one verdict. The worst
/// word wins: a revoked key or a bad signature over a good one.
pub fn read_verify(stdout: &[u8]) -> Verified {
    let lines = status::parse(stdout);
    let key_of = |word: &str| status::find(&lines, word).and_then(|s| status::key_ending(s.arg(0)));
    if status::has(&lines, "REVKEYSIG") || status::has(&lines, "KEYREVOKED") {
        return Verified::Revoked {
            key: key_of("REVKEYSIG").or_else(|| key_of("VALIDSIG")),
        };
    }
    if status::has(&lines, "BADSIG") {
        return Verified::Bad {
            key: key_of("BADSIG"),
        };
    }
    if status::has(&lines, "NO_PUBKEY") {
        return Verified::MissingKey {
            key: key_of("NO_PUBKEY").or_else(|| key_of("ERRSIG")),
        };
    }
    if status::has(&lines, "ERRSIG") {
        return Verified::Unreadable;
    }
    if status::has(&lines, "EXPKEYSIG") {
        return Verified::KeyExpired {
            key: key_of("EXPKEYSIG"),
        };
    }
    if status::has(&lines, "EXPSIG") {
        return Verified::SignatureExpired {
            key: key_of("EXPSIG"),
        };
    }
    if let Some(good) = status::find(&lines, "GOODSIG")
        && status::has(&lines, "VALIDSIG")
    {
        let trust = if status::has(&lines, "TRUST_ULTIMATE") || status::has(&lines, "TRUST_FULLY") {
            Trust::Certified
        } else {
            Trust::NotCertified
        };
        return Verified::Good {
            key: status::key_ending(good.arg(0)).unwrap_or_else(|| "????".to_owned()),
            name: good.rest(1),
            trust,
        };
    }
    Verified::Unreadable
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn s(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn every_argument_list_keeps_the_rules() {
        let input = PathBuf::from("-rf.txt");
        let output = PathBuf::from(".diro-part-1a2b3c4d.gpg");
        let lists = [
            encrypt_args(&["4E7B".to_owned()], false, &output, &input),
            symmetric_args(true, &output, &input),
            decrypt_args(Pinentry::Error, &output, &input),
            decrypt_args(Pinentry::Loopback, &output, &input),
            sign_args(Pinentry::Loopback, Some("4E7B"), true, &output, &input),
        ];
        for list in &lists {
            let list = s(list);
            assert_eq!(list[..7], PREFIX);
            assert!(
                !list
                    .iter()
                    .any(|a| a == "--yes" || a.starts_with("--trust-model"))
            );
            assert!(!list.iter().any(|a| a == "--passphrase"));
            let dash = list.iter().position(|a| a == "--").unwrap();
            assert_eq!(list[dash + 1], "-rf.txt", "the input is after --");
            assert_eq!(dash + 2, list.len());
            let at = list.iter().position(|a| a == "--output").unwrap();
            assert_eq!(list[at + 1], ".diro-part-1a2b3c4d.gpg");
        }
        assert!(
            s(&lists[1])
                .windows(2)
                .any(|w| w == ["--cipher-algo", "AES256"])
        );
        assert!(
            s(&lists[2])
                .windows(2)
                .any(|w| w == ["--pinentry-mode", "error"])
        );
        assert!(
            s(&lists[3])
                .windows(2)
                .any(|w| w == ["--passphrase-fd", "0"])
        );
        let verify = s(&verify_args(Path::new("a.sig"), Some(Path::new("-a"))));
        assert_eq!(verify[7..], ["--verify", "--", "a.sig", "-a"]);
        assert!(recipient_ok("ada@example.invalid"));
        assert!(!recipient_ok("--trust-model=always"));
        assert!(!recipient_ok("a\nb"));
        assert!(!recipient_ok(" "));
    }

    #[test]
    fn a_passphrase_goes_to_standard_input_only() {
        let secret = Secret::new("canary-7f3a-not-a-password".to_owned());
        let input = passphrase_input(&secret);
        assert!(matches!(input, ToolInput::Secret(_)), "{input:?}");
        assert_eq!(input.bytes(), Some(&b"canary-7f3a-not-a-password\n"[..]));
        assert!(!format!("{input:?}").contains("canary"), "{input:?}");
    }

    // The status words below are those spike ES3 saw from gpg 2.4.9
    // (Git for Windows) and 2.4.7 (Debian), with test keys' ids.
    #[test]
    fn decryption_in_two_steps_reads_as_es3_saw_it() {
        let step1 = b"[GNUPG:] ENC_TO 9D5EB0D1926687A4 18 0\n[GNUPG:] KEY_CONSIDERED 78B5 0\n\
[GNUPG:] NO_SECKEY 9D5EB0D1926687A4\n[GNUPG:] ERROR pkdecrypt_failed 67108949\n\
[GNUPG:] BEGIN_DECRYPTION\n[GNUPG:] DECRYPTION_FAILED\n[GNUPG:] END_DECRYPTION\n\
[GNUPG:] FAILURE gpg-exit 33554433\n";
        assert_eq!(
            read_decrypt(Some(2), step1, b""),
            Outcome::NeedsPassphrase {
                key: Some("87A4".to_owned()),
                symmetric: false
            }
        );
        let wrong = b"[GNUPG:] ENC_TO 9D5EB0D1926687A4 18 0\n[GNUPG:] NO_SECKEY 9D5EB0D1926687A4\n\
[GNUPG:] ERROR pkdecrypt_failed 67108875\n[GNUPG:] DECRYPTION_FAILED\n";
        assert_eq!(read_decrypt(Some(2), wrong, b""), Outcome::WrongPassphrase);
        let done = b"[GNUPG:] ENC_TO 9D5EB0D1926687A4 18 0\n[GNUPG:] DECRYPTION_KEY X Y u\n\
[GNUPG:] BEGIN_DECRYPTION\n[GNUPG:] DECRYPTION_OKAY\n[GNUPG:] GOODMDC\n[GNUPG:] END_DECRYPTION\n";
        assert_eq!(read_decrypt(Some(0), done, b""), Outcome::Done);
        let sym = b"[GNUPG:] NEED_PASSPHRASE_SYM 9 3 2\n[GNUPG:] ERROR get_passphrase 85\n\
[GNUPG:] CANCELED_BY_USER\n[GNUPG:] ERROR symkey_decrypt.maybe_error 11_BAD_PASSPHRASE\n\
[GNUPG:] DECRYPTION_FAILED\n";
        assert_eq!(
            read_decrypt(Some(2), sym, b""),
            Outcome::NeedsPassphrase {
                key: None,
                symmetric: true
            }
        );
        let not_mine =
            b"[GNUPG:] ENC_TO 1122334455667788 18 0\n[GNUPG:] NO_SECKEY 1122334455667788\n\
[GNUPG:] BEGIN_DECRYPTION\n[GNUPG:] DECRYPTION_FAILED\n";
        assert_eq!(
            read_decrypt(Some(2), not_mine, b""),
            Outcome::NoSecretKey {
                key: Some("7788".to_owned())
            }
        );
        let not_pgp =
            b"[GNUPG:] NODATA 1\n[GNUPG:] NODATA 2\n[GNUPG:] FAILURE decrypt 4294967295\n";
        assert_eq!(read_decrypt(Some(2), not_pgp, b""), Outcome::NotOpenPgp);
    }

    #[test]
    fn encryption_and_signatures_read_as_es3_saw_them() {
        let uncertified = b"[GNUPG:] KEY_CONSIDERED B5FD 0\n\
[GNUPG:] INV_RECP 10 B5FD3E56AC68360280778E9B4B3EF074F910E7AA\n[GNUPG:] FAILURE encrypt 53\n";
        assert_eq!(
            read_encrypt(Some(2), uncertified, b""),
            Outcome::NotCertified
        );
        let missing = b"[GNUPG:] INV_RECP 1 bob@example.invalid\n[GNUPG:] FAILURE encrypt 9\n";
        assert_eq!(
            read_encrypt(Some(2), missing, b""),
            Outcome::RecipientNotFound("bob@example.invalid".to_owned())
        );
        let done = b"[GNUPG:] KEY_CONSIDERED A 0\n[GNUPG:] BEGIN_ENCRYPTION 2 9\n[GNUPG:] END_ENCRYPTION\n";
        assert_eq!(read_encrypt(Some(0), done, b""), Outcome::Done);
        let exists = b"[GNUPG:] KEY_CONSIDERED A 0\n[GNUPG:] FAILURE encrypt 33587235\n";
        assert_eq!(
            read_encrypt(
                Some(2),
                exists,
                b"gpg: D:\\x\\report.txt: encryption failed: File exists\n"
            ),
            Outcome::Failed("D:\\x\\report.txt: encryption failed: File exists".to_owned())
        );
        let step1 = b"[GNUPG:] KEY_CONSIDERED 0B8D814646FF8E39A6B422008C65CD3C79AB3972 0\n\
[GNUPG:] BEGIN_SIGNING H10\n[GNUPG:] FAILURE sign 67108949\n";
        assert_eq!(
            read_sign(Some(2), step1, b""),
            Outcome::NeedsPassphrase {
                key: Some("3972".to_owned()),
                symmetric: false
            }
        );
        let wrong = b"[GNUPG:] BEGIN_SIGNING H10\n[GNUPG:] FAILURE sign 67108875\n";
        assert_eq!(read_sign(Some(2), wrong, b""), Outcome::WrongPassphrase);
        let signed =
            b"[GNUPG:] BEGIN_SIGNING H10\n[GNUPG:] SIG_CREATED D 22 10 00 1794075089 0B8D\n";
        assert_eq!(read_sign(Some(0), signed, b""), Outcome::Done);
        let no_key = b"[GNUPG:] INV_SGNR 9 0123456789ABCDEF\n[GNUPG:] FAILURE sign 17\n";
        assert_eq!(
            read_sign(Some(2), no_key, b""),
            Outcome::NoSecretKey {
                key: Some("CDEF".to_owned())
            }
        );
    }

    #[test]
    fn verification_reads_every_word_es3_saw() {
        let good =
            b"[GNUPG:] NEWSIG\n[GNUPG:] KEY_CONSIDERED 78B5 0\n[GNUPG:] SIG_ID x 2026-11-07 1\n\
[GNUPG:] GOODSIG 168B6F650E9C8469 Ada Example <ada@example.invalid>\n\
[GNUPG:] VALIDSIG 78B5 2026-11-07 1 0 4 0 22 10 00 78B5\n[GNUPG:] TRUST_ULTIMATE 0 pgp\n";
        assert_eq!(
            read_verify(good),
            Verified::Good {
                key: "8469".to_owned(),
                name: "Ada Example <ada@example.invalid>".to_owned(),
                trust: Trust::Certified
            }
        );
        let undefined = String::from_utf8_lossy(good).replace("TRUST_ULTIMATE", "TRUST_UNDEFINED");
        assert!(matches!(
            read_verify(undefined.as_bytes()),
            Verified::Good {
                trust: Trust::NotCertified,
                ..
            }
        ));
        let bad = b"[GNUPG:] NEWSIG\n[GNUPG:] BADSIG 168B6F650E9C8469 Ada Example\n";
        assert_eq!(
            read_verify(bad),
            Verified::Bad {
                key: Some("8469".to_owned())
            }
        );
        let missing = b"[GNUPG:] NEWSIG\n[GNUPG:] ERRSIG 168B6F650E9C8469 22 10 00 1 9 78B5\n\
[GNUPG:] NO_PUBKEY 168B6F650E9C8469\n";
        assert_eq!(
            read_verify(missing),
            Verified::MissingKey {
                key: Some("8469".to_owned())
            }
        );
        let expired =
            b"[GNUPG:] NEWSIG\n[GNUPG:] KEYEXPIRED 1\n[GNUPG:] EXPKEYSIG 168B6F650E9C8469 Ada\n\
[GNUPG:] VALIDSIG 78B5 x\n";
        assert_eq!(
            read_verify(expired),
            Verified::KeyExpired {
                key: Some("8469".to_owned())
            }
        );
        let revoked =
            b"[GNUPG:] NEWSIG\n[GNUPG:] REVKEYSIG 168B6F650E9C8469 Ada\n[GNUPG:] VALIDSIG 78B5 x\n\
[GNUPG:] KEYREVOKED\n[GNUPG:] TRUST_ULTIMATE 0 pgp\n";
        assert_eq!(
            read_verify(revoked),
            Verified::Revoked {
                key: Some("8469".to_owned())
            }
        );
        let none = b"[GNUPG:] NODATA 1\n[GNUPG:] NODATA 2\n[GNUPG:] FAILURE gpg-exit 33554433\n";
        assert_eq!(read_verify(none), Verified::Unreadable);
        // A good line with no VALIDSIG is not good.
        assert_eq!(
            read_verify(b"[GNUPG:] GOODSIG 168B6F650E9C8469 Ada\n"),
            Verified::Unreadable
        );
    }
}
