//! minisign signatures, checked in process by `minisign-verify` (R29 3.4,
//! ADR-0015): a `.minisig` file against its data, with a public key from a
//! file (`minisign.pub`, two lines) or a pasted string (one line, `RW`
//! and 54 more). A pre-hashed signature (the default since minisign 0.8)
//! is checked as the file streams by, so a large file is never held in
//! memory; an old, legacy one only for a file up to [`LEGACY_MAX`].
//!
//! The key's id is read from the key's own bytes, as minisign prints it
//! ("E7620F1842B4E81F"), so the verdict says "key ending E81F".

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use diroxide_vfs::CancelToken;
use minisign_verify::{Error, PublicKey, Signature};

use super::verify::{SignatureVerdict, Signer};

/// The most a signature or a public key file may hold, in bytes.
pub const SMALL_MAX: u64 = 64 * 1024;

/// The most a file with a legacy (not pre-hashed) signature may be: it
/// must be read whole.
pub const LEGACY_MAX: u64 = 256 * 1024 * 1024;

/// How much of the data is read at a time.
const CHUNK: usize = 64 * 1024;

/// A minisign public key, as the owner gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MinisignKey {
    /// A pasted key: the one base64 line, or a `minisign.pub` file's two
    /// lines.
    Text(String),
    /// A public key file by its path.
    File(std::path::PathBuf),
}

/// A small file's text.
fn small_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    file.take(SMALL_MAX + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= SMALL_MAX).then_some(text)
}

/// The key's text: one base64 line, or the last non-empty line of a
/// `minisign.pub`.
fn key_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
}

/// The public key and its id's last four digits as minisign prints them.
pub fn read_key(key: &MinisignKey) -> Option<(PublicKey, String)> {
    let text = match key {
        MinisignKey::Text(text) => text.clone(),
        MinisignKey::File(path) => small_text(path)?,
    };
    let line = key_line(&text)?;
    let public = PublicKey::from_base64(line).ok()?;
    let bytes = decode_base64(line)?;
    // Two bytes of algorithm, then the 8-byte id, printed as a
    // little-endian number: its last four digits are bytes 1 and 0.
    let ending = format!("{:02X}{:02X}", bytes.get(3)?, bytes.get(2)?);
    Some((public, ending))
}

/// Checks a `.minisig` against its data. Blocking; the cancel is looked
/// at between pieces.
pub fn verify(
    signature: &Path,
    data: &Path,
    key: &MinisignKey,
    cancel: &CancelToken,
) -> SignatureVerdict {
    let Some((public, ending)) = read_key(key) else {
        return SignatureVerdict::KeyUnreadable;
    };
    let Some(signature) = small_text(signature).and_then(|t| Signature::decode(&t).ok()) else {
        return SignatureVerdict::Unreadable;
    };
    let good = || SignatureVerdict::Good {
        signer: Signer::Minisign,
        key: ending.clone(),
        name: Some(signature.trusted_comment().to_owned()),
        certified: true,
    };
    let Ok(file) = File::open(data) else {
        return SignatureVerdict::DataUnreadable;
    };
    let checked = match public.verify_stream(&signature) {
        Ok(mut stream) => {
            let mut reader = BufReader::with_capacity(CHUNK, file);
            let mut buffer = vec![0u8; CHUNK];
            loop {
                if cancel.is_cancelled() {
                    return SignatureVerdict::Cancelled;
                }
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => stream.update(&buffer[..n]),
                    Err(_) => return SignatureVerdict::DataUnreadable,
                }
            }
            stream.finalize()
        }
        Err(Error::UnsupportedLegacyMode) => {
            let size = file.metadata().map_or(u64::MAX, |m| m.len());
            if size > LEGACY_MAX {
                return SignatureVerdict::Unreadable;
            }
            let mut bytes = Vec::new();
            if file.take(LEGACY_MAX).read_to_end(&mut bytes).is_err() {
                return SignatureVerdict::DataUnreadable;
            }
            public.verify(&bytes, &signature, true)
        }
        Err(error) => Err(error),
    };
    match checked {
        Ok(()) => good(),
        Err(Error::InvalidSignature) => SignatureVerdict::Bad { key: Some(ending) },
        Err(Error::UnexpectedKeyId) => SignatureVerdict::OtherKey,
        Err(_) => SignatureVerdict::Unreadable,
    }
}

/// Standard base64 (with or without padding), for the key's id; `None`
/// for anything else. Never allocates more than the text's length.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for byte in text.trim().bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The published test vector of the minisign-verify crate: a public
    // key and a pre-hashed signature over the four bytes "test". Public
    // data, no secret.
    const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const SIGNATURE: &str = "untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1556193335\tfile:test
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==
";

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("crypt-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("minisign-")
            .tempdir_in(&base)
            .unwrap()
    }

    #[test]
    fn the_key_id_reads_as_minisign_prints_it() {
        let (_, ending) = read_key(&MinisignKey::Text(KEY.to_owned())).unwrap();
        // minisign prints this key as E7620F1842B4E81F.
        assert_eq!(ending, "E81F");
        let two_lines = format!("untrusted comment: minisign public key E7620F1842B4E81F\n{KEY}\n");
        assert!(read_key(&MinisignKey::Text(two_lines)).is_some());
        assert!(read_key(&MinisignKey::Text("not a key".to_owned())).is_none());
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert!(decode_base64("a*b").is_none());
    }

    #[test]
    fn a_good_a_changed_and_another_keys_file() {
        let dir = scratch();
        let data = dir.path().join("test");
        std::fs::write(&data, b"test").unwrap();
        let sig = dir.path().join("test.minisig");
        std::fs::write(&sig, SIGNATURE).unwrap();
        let key = MinisignKey::Text(KEY.to_owned());
        let cancel = CancelToken::new();
        match verify(&sig, &data, &key, &cancel) {
            SignatureVerdict::Good { key, name, .. } => {
                assert_eq!(key, "E81F");
                assert_eq!(name.as_deref(), Some("timestamp:1556193335\tfile:test"));
            }
            other => panic!("{other:?}"),
        }
        std::fs::write(&data, b"Test").unwrap();
        assert_eq!(
            verify(&sig, &data, &key, &cancel),
            SignatureVerdict::Bad {
                key: Some("E81F".to_owned())
            }
        );
        // A key file of another key (its id bytes changed).
        let other = KEY.replacen("RWQf", "RWQg", 1);
        let key_file = dir.path().join("minisign.pub");
        std::fs::write(&key_file, format!("untrusted comment: a key\n{other}\n")).unwrap();
        assert_eq!(
            verify(&sig, &data, &MinisignKey::File(key_file), &cancel),
            SignatureVerdict::OtherKey
        );
        std::fs::write(&sig, "not a signature").unwrap();
        assert_eq!(
            verify(&sig, &data, &key, &cancel),
            SignatureVerdict::Unreadable
        );
        assert_eq!(
            verify(&sig, &data, &MinisignKey::Text("x".to_owned()), &cancel),
            SignatureVerdict::KeyUnreadable
        );
    }
}
