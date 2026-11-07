//! A look at an age file's header before any secret is asked: whether it
//! is age's at all, whether it wants a passphrase, and how much work its
//! passphrase asks for, so a hostile file is refused before the owner
//! types anything and before scrypt runs (spike ES2; the cap is
//! [`WORK_CAP`]).
//!
//! The header is text: `age-encryption.org/v1`, then a stanza a recipient
//! (`-> scrypt SALT 18` for a passphrase), then `--- MAC`. An armored file
//! is read through age's own armor reader first.

use std::io::Read;

/// The highest scrypt work factor (log2 of N) a file may ask before it is
/// refused: 20 opens in about two seconds on the owner's computer, a file
/// made at the reference program's default of 18 in about half a second,
/// and 21 would take over four (spike ES2, Friday, November 6, 2026).
pub const WORK_CAP: u8 = 20;

/// The work factor this plugin encrypts with: the reference program's
/// default, so the file opens in about half a second here and on slower
/// computers, under every cap at or above it.
pub const WORK_FACTOR: u8 = 18;

/// How much of a file's start is looked at for its header.
pub const PEEK_BYTES: usize = 16 * 1024;

/// The first line of every age file.
pub const MAGIC: &str = "age-encryption.org/v1";

/// What a header says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peek {
    /// The file starts as an age file of version 1.
    pub age: bool,
    /// A stanza asks for a passphrase, with this work factor.
    pub scrypt: Option<u8>,
    /// How many recipient stanzas the header holds, up to the first
    /// `---`.
    pub stanzas: usize,
}

impl Peek {
    /// Whether the passphrase's work is over the cap.
    pub fn too_much_work(&self) -> bool {
        self.scrypt.is_some_and(|n| n > WORK_CAP)
    }
}

/// Reads a header from the start of a file's (dearmored) bytes. Never
/// panics and never allocates by a number it read.
pub fn peek(bytes: &[u8]) -> Peek {
    let mut out = Peek::default();
    let mut lines = bytes.split(|b| *b == b'\n');
    if lines.next() != Some(MAGIC.as_bytes()) {
        return out;
    }
    out.age = true;
    for line in lines {
        if line.starts_with(b"---") {
            break;
        }
        let Some(stanza) = line.strip_prefix(b"-> ") else {
            continue;
        };
        out.stanzas += 1;
        let mut args = stanza.split(|b| *b == b' ');
        if args.next() != Some(b"scrypt".as_slice()) {
            continue;
        }
        let _salt = args.next();
        let factor = args.next().and_then(|n| {
            (!n.is_empty() && n.len() <= 2 && n.iter().all(u8::is_ascii_digit))
                .then(|| n.iter().fold(0u8, |acc, d| acc * 10 + (d - b'0')))
        });
        // A factor that is not two digits is a header age refuses; it
        // counts as the most work, so it is refused here first.
        let factor = factor.unwrap_or(u8::MAX);
        out.scrypt = Some(out.scrypt.map_or(factor, |old| old.max(factor)));
    }
    out
}

/// Reads the start of a reader, through age's armor reader, and peeks at
/// it.
pub fn peek_reader(reader: impl Read) -> std::io::Result<Peek> {
    let mut armored = age::armor::ArmoredReader::new(std::io::BufReader::new(reader));
    let mut head = Vec::with_capacity(PEEK_BYTES);
    let mut chunk = [0u8; 4096];
    while head.len() < PEEK_BYTES {
        let n = match armored.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            // The armor reader refuses bytes that are not a header it
            // knows; whatever was read is looked at.
            Err(_) if !head.is_empty() => break,
            Err(e) => return Err(e),
        };
        head.extend_from_slice(&chunk[..n.min(PEEK_BYTES - head.len())]);
    }
    Ok(peek(&head))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passphrase_header_says_its_work() {
        let text = format!("{MAGIC}\n-> scrypt c2FsdA 18\nYm9keQ\n--- bWFj\n");
        let seen = peek(text.as_bytes());
        assert_eq!(
            seen,
            Peek {
                age: true,
                scrypt: Some(18),
                stanzas: 1
            }
        );
        assert!(!seen.too_much_work());
        let hostile = format!("{MAGIC}\n-> scrypt c2FsdA 40\nYm9keQ\n--- bWFj\n");
        assert!(peek(hostile.as_bytes()).too_much_work());
        let odd = format!("{MAGIC}\n-> scrypt c2FsdA 018\n--- x\n");
        assert!(peek(odd.as_bytes()).too_much_work());
    }

    #[test]
    fn a_key_header_and_other_files() {
        let text = format!("{MAGIC}\n-> X25519 a2V5\nYm9keQ\n--- bWFj\n-> scrypt s 99\n");
        let seen = peek(text.as_bytes());
        assert!(seen.age);
        assert_eq!(seen.scrypt, None);
        assert_eq!(seen.stanzas, 1);
        assert_eq!(peek(b"%PDF-1.7\n"), Peek::default());
        assert_eq!(peek(b""), Peek::default());
    }

    #[test]
    fn a_real_file_and_its_armored_form_peek_the_same() {
        let pass = age::secrecy::SecretString::from("thrown-away-test-pass".to_owned());
        let mut recipient = age::scrypt::Recipient::new(pass);
        recipient.set_work_factor(10);
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .unwrap();
        let mut armored = Vec::new();
        let writer =
            age::armor::ArmoredWriter::wrap_output(&mut armored, age::armor::Format::AsciiArmor)
                .unwrap();
        let mut stream = encryptor.wrap_output(writer).unwrap();
        std::io::Write::write_all(&mut stream, b"hello").unwrap();
        stream.finish().unwrap().finish().unwrap();
        assert!(armored.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));
        let seen = peek_reader(armored.as_slice()).unwrap();
        assert_eq!(seen.scrypt, Some(10));
        assert!(seen.age);
    }
}
