//! Decrypt (R29 3.2): each file's own header decides the question before
//! anything is typed. A passphrase file asks "Passphrase for
//! report.pdf.age" in a hidden field, three tries at most; a key file asks
//! for an identity file by its path, read for this command only. A header
//! that asks for more work than [`WORK_CAP`](crate::header::WORK_CAP) is
//! refused before any question.
//!
//! Within one command a passphrase or an identity that opened one file is
//! tried first on the next, so ten files for one key ask once; nothing
//! outlives the command, and nothing is ever written down.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use age::secrecy::SecretString;
use serde_json::Value;
use zeroize::Zeroizing;

use super::{Ctx, Tally, Target, Verb};
use crate::crypt::{self, End, Failure, Unlock, Watch};
use crate::error::CryptError;
use crate::header::{self, WORK_CAP};
use crate::lines;

/// How many passphrases are tried on one file.
pub const TRIES: u32 = 3;

/// The most an identity file may hold, in bytes.
pub const MAX_IDENTITY_BYTES: u64 = 64 * 1024;

/// An identity file's keys, kept for one command.
type Keys = Vec<Box<dyn age::Identity + Send + Sync>>;

/// What one command remembers between its files, in memory only.
#[derive(Default)]
struct Held {
    passphrase: Option<SecretString>,
    keys: Option<Keys>,
}

/// Decrypt.
pub fn decrypt(ctx: &Ctx<'_>) -> Result<Value, CryptError> {
    let mut tally = Tally::default();
    let targets = ctx.targets(&mut tally);
    if targets.is_empty() {
        return Ok(ctx.nothing(&tally));
    }
    let plan = lines::plan_decrypt(targets.len());
    ctx.progress.begin(&plan);
    let mut held = Held::default();
    for target in &targets {
        if ctx.cancelled() {
            break;
        }
        one(ctx, target, &mut held, &mut tally)?;
    }
    Ok(ctx.finish(&tally, targets.len(), Verb::Decrypt))
}

/// Runs the decryption of one file with one way to open it.
fn attempt(ctx: &Ctx<'_>, target: &Target, unlock: &Unlock<'_>) -> End {
    let cancelled = || ctx.cancelled();
    let mut progress = ctx.reporter("Decrypting", &target.name);
    let mut watch = Watch {
        cancelled: &cancelled,
        progress: &mut progress,
    };
    crypt::decrypt_file(&target.path, unlock, &mut watch)
}

/// One file: its header looked at, the right question asked, the result
/// said.
fn one(
    ctx: &Ctx<'_>,
    target: &Target,
    held: &mut Held,
    tally: &mut Tally,
) -> Result<(), CryptError> {
    let peek = match File::open(&target.path).and_then(header::peek_reader) {
        Ok(peek) => peek,
        Err(error) => {
            let failure = Failure::Words(error.to_string().to_lowercase());
            ctx.failed(tally, target, &failure, Verb::Decrypt);
            return Ok(());
        }
    };
    if !peek.age {
        ctx.failed(tally, target, &Failure::NotAge, Verb::Decrypt);
        return Ok(());
    }
    if peek.too_much_work() {
        ctx.failed(tally, target, &Failure::TooMuchWork, Verb::Decrypt);
        return Ok(());
    }
    if peek.scrypt.is_some() {
        with_passphrase(ctx, target, held, tally)
    } else {
        with_keys(ctx, target, held, tally)
    }
}

fn with_passphrase(
    ctx: &Ctx<'_>,
    target: &Target,
    held: &mut Held,
    tally: &mut Tally,
) -> Result<(), CryptError> {
    if let Some(passphrase) = &held.passphrase {
        match attempt(ctx, target, &Unlock::Passphrase(passphrase)) {
            End::Failed(Failure::WrongPassphrase) => {}
            end => {
                ctx.record(tally, target, end, Verb::Decrypt);
                return Ok(());
            }
        }
    }
    let mut note: Vec<String> = Vec::new();
    for tries in 1..=TRIES {
        let Some(passphrase) = ctx
            .host
            .ask_secret(&lines::passphrase_title(&target.name), &note)?
        else {
            ctx.host.announce(
                &lines::skipped_unanswered(&target.name, "passphrase"),
                "warning",
            );
            tally.skipped += 1;
            return Ok(());
        };
        match attempt(ctx, target, &Unlock::Passphrase(&passphrase)) {
            End::Failed(Failure::WrongPassphrase) if tries < TRIES => {
                note = vec![lines::try_again(&target.name, tries + 1, TRIES)];
            }
            End::Failed(Failure::WrongPassphrase) => {
                ctx.host
                    .announce(&lines::gave_up(&target.name, TRIES), "warning");
                tally.failed.push(target.name.clone());
                return Ok(());
            }
            end => {
                if matches!(end, End::Made(_)) {
                    held.passphrase = Some(passphrase);
                }
                ctx.record(tally, target, end, Verb::Decrypt);
                return Ok(());
            }
        }
    }
    Ok(())
}

fn with_keys(
    ctx: &Ctx<'_>,
    target: &Target,
    held: &mut Held,
    tally: &mut Tally,
) -> Result<(), CryptError> {
    if let Some(keys) = &held.keys {
        match attempt(ctx, target, &Unlock::Identities(keys)) {
            End::Failed(Failure::NoMatchingKey) => {}
            end => {
                ctx.record(tally, target, end, Verb::Decrypt);
                return Ok(());
            }
        }
    }
    let Some(path) = ctx
        .host
        .ask_file(&lines::identity_title(&target.name), &[])?
    else {
        ctx.host.announce(
            &lines::skipped_unanswered(&target.name, "identity file"),
            "warning",
        );
        tally.skipped += 1;
        return Ok(());
    };
    let keys = match load_identities(ctx, Path::new(&path))? {
        Ok(keys) => keys,
        Err(cause) => {
            ctx.host
                .announce(&lines::identity_unusable(&cause), "warning");
            tally.failed.push(target.name.clone());
            return Ok(());
        }
    };
    let end = attempt(ctx, target, &Unlock::Identities(&keys));
    if matches!(end, End::Made(_)) {
        held.keys = Some(keys);
    }
    ctx.record(tally, target, end, Verb::Decrypt);
    Ok(())
}

/// Reads an identity file's keys: a plain file of `AGE-SECRET-KEY-1`
/// lines, or one encrypted with a passphrase, asked for here. Its bytes
/// are wiped when dropped. The cause, when it cannot be used, never
/// repeats the file's text.
fn load_identities(ctx: &Ctx<'_>, path: &Path) -> Result<Result<Keys, String>, CryptError> {
    let bytes = match read_small(path) {
        Ok(bytes) => bytes,
        Err(cause) => return Ok(Err(cause)),
    };
    let peek = header::peek_reader(bytes.as_slice()).unwrap_or_default();
    if !peek.age {
        return Ok(parse_identities(&bytes));
    }
    if peek.scrypt.is_none() {
        return Ok(Err("it is encrypted to a key, not a passphrase".to_owned()));
    }
    if peek.too_much_work() {
        return Ok(Err(lines::TOO_MUCH_WORK.trim_end_matches('.').to_owned()));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut note = Vec::new();
    for tries in 1..=TRIES {
        let Some(passphrase) = ctx
            .host
            .ask_secret(&lines::passphrase_title(&name), &note)?
        else {
            return Ok(Err("no passphrase was given for it".to_owned()));
        };
        match open_identity(&bytes, &passphrase) {
            Ok(plain) => return Ok(parse_identities(&plain)),
            Err(Failure::WrongPassphrase) if tries < TRIES => {
                note = vec![lines::try_again(&name, tries + 1, TRIES)];
            }
            Err(failure) => return Ok(Err(failure.cause().trim_end_matches('.').to_owned())),
        }
    }
    Ok(Err("the passphrase was wrong 3 times".to_owned()))
}

/// A small file's bytes, wiped when dropped.
fn read_small(path: &Path) -> Result<Zeroizing<Vec<u8>>, String> {
    let file = File::open(path).map_err(|e| e.to_string().to_lowercase())?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if size > MAX_IDENTITY_BYTES {
        return Err("it is over 64 KB, too large for an identity file".to_owned());
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(size as usize));
    file.take(MAX_IDENTITY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string().to_lowercase())?;
    if bytes.len() as u64 > MAX_IDENTITY_BYTES {
        return Err("it is over 64 KB, too large for an identity file".to_owned());
    }
    Ok(bytes)
}

/// Decrypts an identity file encrypted with a passphrase, in memory.
fn open_identity(bytes: &[u8], passphrase: &SecretString) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let armored = age::armor::ArmoredReader::new(BufReader::new(bytes));
    let decryptor = age::Decryptor::new(armored).map_err(|e| crypt::decrypt_words(&e))?;
    let mut identity = age::scrypt::Identity::new(passphrase.clone());
    identity.set_max_work_factor(WORK_CAP);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| crypt::decrypt_words(&e))?;
    let mut plain = Zeroizing::new(Vec::new());
    reader
        .by_ref()
        .take(MAX_IDENTITY_BYTES + 1)
        .read_to_end(&mut plain)
        .map_err(|_| Failure::Words(lines::DAMAGED.to_owned()))?;
    Ok(plain)
}

/// The keys of an identity file's text.
fn parse_identities(bytes: &[u8]) -> Result<Keys, String> {
    let unusable = || "it holds no age key diroxide can use".to_owned();
    let file = age::IdentityFile::from_buffer(BufReader::new(bytes)).map_err(|_| unusable())?;
    let keys = file.into_identities().map_err(|_| unusable())?;
    if keys.is_empty() {
        return Err(unusable());
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    #[test]
    fn an_identity_file_reads_its_keys_and_never_says_them() {
        let key = age::x25519::Identity::generate();
        let text = format!(
            "# created: a test\n# public key: {}\n{}\n",
            key.to_public(),
            key.to_string().expose_secret()
        );
        assert_eq!(parse_identities(text.as_bytes()).unwrap().len(), 1);
        let broken = text.replace("AGE-SECRET-KEY-1", "AGE-SECRET-KEY-9");
        let cause = parse_identities(broken.as_bytes()).err().unwrap();
        assert!(!cause.to_uppercase().contains("AGE-SECRET-KEY"), "{cause}");
        assert!(parse_identities(b"").is_err());
    }

    #[test]
    fn an_identity_file_under_a_passphrase_opens_in_memory() {
        let key = age::x25519::Identity::generate();
        let text = format!("{}\n", key.to_string().expose_secret());
        let pass = SecretString::from("thrown-away-identity-pass".to_owned());
        let mut recipient = age::scrypt::Recipient::new(pass.clone());
        recipient.set_work_factor(10);
        let sealed = age::encrypt(&recipient, text.as_bytes()).unwrap();
        let plain = open_identity(&sealed, &pass).unwrap();
        assert_eq!(parse_identities(&plain).unwrap().len(), 1);
        let wrong = SecretString::from("thrown-away-wrong".to_owned());
        assert_eq!(
            open_identity(&sealed, &wrong).err(),
            Some(Failure::WrongPassphrase)
        );
    }
}
