//! The recipients of an encryption: a pasted age public key, or a
//! recipients file by its path, read the way age reads one (a key a line,
//! `#` for a comment, blank lines passed over).
//!
//! Only age's own X25519 keys (`age1` and 58 more characters) are taken.
//! SSH keys are refused (the `ssh` feature is off: the `rsa` crate's open
//! timing advisory), plugin keys too (no plugin is ever started), and the
//! hardware tag keys (`age1tag1`, `age1tagpq1`) as well, so the P-256 code
//! under age's two advisories is compiled in and never reached
//! (ADR-0015). A passphrase is never a recipient here: it has its own
//! command, as age's rule wants a passphrase to be a file's only
//! recipient.

use std::str::FromStr;

use crate::lines;

/// The most a recipients file may hold, in bytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The most recipients one encryption takes.
pub const MAX_RECIPIENTS: usize = 1000;

/// Why recipients could not be read: a sentence, its meaning first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub String);

/// Reads recipients from text: a pasted key, or a file's contents. Lines
/// are counted from 1 for the words.
pub fn parse(text: &str) -> Result<Vec<age::x25519::Recipient>, Refused> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        out.push(one(line, number)?);
        if out.len() > MAX_RECIPIENTS {
            return Err(Refused(format!(
                "More than {MAX_RECIPIENTS} recipients: line {number}"
            )));
        }
    }
    Ok(out)
}

/// One key on line `number`.
fn one(line: &str, number: usize) -> Result<age::x25519::Recipient, Refused> {
    if let Ok(recipient) = age::x25519::Recipient::from_str(line) {
        return Ok(recipient);
    }
    let lower = line.to_ascii_lowercase();
    if ["ssh-", "ecdsa-", "sk-ssh-", "sk-ecdsa-"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return Err(Refused(lines::ssh_recipient(number)));
    }
    if lower.starts_with("age1tag1") || lower.starts_with("age1tagpq1") {
        return Err(Refused(format!(
            "Hardware keys are not used: line {number}"
        )));
    }
    // A plugin's key: `age1` then the plugin's name, a `1` and its data.
    if let Some(rest) = lower.strip_prefix("age1")
        && let Some((name, _)) = rest.split_once('1')
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name.len() < rest.len()
    {
        return Err(Refused(lines::plugin_recipient(number)));
    }
    Err(Refused(lines::bad_recipient(number)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    fn key() -> String {
        age::x25519::Identity::generate().to_public().to_string()
    }

    #[test]
    fn keys_comments_and_blank_lines_read_as_age_reads_them() {
        let (a, b) = (key(), key());
        let text = format!("# the team\n{a}\n\n   {b}  \r\n# end\n");
        let read = parse(&text).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].to_string(), a);
        assert!(parse("").unwrap().is_empty());
        assert!(parse("# only a comment\n").unwrap().is_empty());
    }

    #[test]
    fn what_is_refused_says_which_line_first() {
        let a = key();
        let bad = format!("{a}\nage1notakey\n");
        assert_eq!(parse(&bad), Err(Refused(lines::bad_recipient(2))));
        assert_eq!(
            parse("ssh-ed25519 AAAAC3Nza ada@example.invalid"),
            Err(Refused(lines::ssh_recipient(1)))
        );
        assert_eq!(
            parse("age1yubikey1qwerty"),
            Err(Refused(lines::plugin_recipient(1)))
        );
        assert_eq!(
            parse("age1tag1qwerty"),
            Err(Refused("Hardware keys are not used: line 1".to_owned()))
        );
        // A secret key pasted by mistake is not a recipient, and is never
        // said back.
        let secret = age::x25519::Identity::generate();
        let pasted = secret.to_string();
        let refused = parse(pasted.expose_secret()).unwrap_err();
        assert!(!refused.0.contains("AGE-SECRET-KEY"));
        assert_eq!(refused, Refused(lines::bad_recipient(1)));
    }

    #[test]
    fn too_many_recipients_are_refused() {
        let a = key();
        let text = std::iter::repeat_n(a.as_str(), MAX_RECIPIENTS + 1)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse(&text).unwrap_err().0.starts_with("More than 1000"));
    }
}
