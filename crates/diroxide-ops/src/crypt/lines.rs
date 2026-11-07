//! The words of gpg's jobs, the signature verdicts and the chain (R29 3.3
//! to 3.5), each meaning first and inside 40 cells where a name allows.
//! A name is said with its hidden characters spelled out; a key's owner's
//! own text is cleaned of hidden characters and said only after "key
//! ending 4E7B". Nothing here calls a file safe, genuine or authored.

use diroxide_core::format::count;
use diroxide_core::names::spell_out;

/// A name for a sentence: hidden characters spelled out.
pub fn said(name: &str) -> String {
    spell_out(name).into_owned()
}

/// gpg is not installed: "gpg not found. Install GnuPG first." (35).
pub const GPG_NOT_FOUND: &str = "gpg not found. Install GnuPG first.";

/// A recipient key gpg will not use in batch mode: the owner has not
/// certified it. diroxide never adds `--trust-model always`.
pub const NOT_CERTIFIED: &str = "Key not certified: sign it in your key tool, or choose another";

/// A recipient gpg cannot find: "Key not found: ada@example.invalid".
pub fn recipient_not_found(recipient: &str) -> String {
    format!("Key not found: {}", clean_text(recipient))
}

/// No secret key opens the file: "No secret key for it: key ending 4E7B".
pub fn no_secret_key(key: Option<&str>) -> String {
    match key {
        Some(key) => format!("No secret key for it: key ending {key}"),
        None => "No secret key for it".to_owned(),
    }
}

/// The plan line for an encryption to keys: "Encrypt 3 files for 1
/// recipient" (31).
pub fn plan_for_recipients(files: usize, recipients: usize) -> String {
    format!(
        "Encrypt {} for {}",
        count(files as u64, "file", "files"),
        count(recipients as u64, "recipient", "recipients")
    )
}

/// The plan line for a passphrase encryption.
pub fn plan_with_passphrase(files: usize) -> String {
    format!(
        "Encrypt {} with a passphrase",
        count(files as u64, "file", "files")
    )
}

/// The plan line for a decryption.
pub fn plan_decrypt(files: usize) -> String {
    format!("Decrypt {}", count(files as u64, "file", "files"))
}

/// The plan line for signatures: "Sign 3 files with key ending 4E7B".
pub fn plan_sign(files: usize, key: Option<&str>) -> String {
    let files = count(files as u64, "file", "files");
    match key {
        Some(key) => format!("Sign {files} with key ending {key}"),
        None => format!("Sign {files} with your default key"),
    }
}

/// One file encrypted: "Encrypted report.pdf to report.pdf.gpg".
pub fn encrypted(from: &str, to: &str) -> String {
    format!("Encrypted {} to {}", said(from), said(to))
}

/// One file decrypted: "Decrypted report.pdf.gpg to report.pdf.".
pub fn decrypted(from: &str, to: &str) -> String {
    format!("Decrypted {} to {}.", said(from), said(to))
}

/// One file signed: "Signed report.pdf in report.pdf.sig".
pub fn signed(from: &str, to: &str) -> String {
    format!("Signed {} in {}", said(from), said(to))
}

/// Said once after a decryption: decrypting proves nothing of who made a
/// file.
pub const NOT_PROOF: &str = "This does not prove who made it.";

/// The masked prompt's words for a key: "Passphrase for key ending 4E7B"
/// (30); with " (hidden):" the line is 40 cells.
pub fn key_passphrase(key: Option<&str>) -> String {
    match key {
        Some(key) => format!("Passphrase for key ending {key}"),
        None => "Passphrase for your signing key".to_owned(),
    }
}

/// A wrong passphrase, said in the next question.
pub fn try_again(what: &str, attempt: u32, allowed: u32) -> String {
    format!("Wrong passphrase for {what}. Try again, {attempt} of {allowed}.")
}

/// A wrong passphrase three times: "Gave up on report.pdf.gpg: 3 wrong
/// tries".
pub fn gave_up(name: &str, tries: u32) -> String {
    format!(
        "Gave up on {}: {}",
        said(name),
        count(u64::from(tries), "wrong try", "wrong tries")
    )
}

/// No passphrase given: the files are skipped.
pub const NO_PASSPHRASE: &str = "no passphrase was given";

/// The cause for a wrong passphrase after the last try.
pub const WRONG_PASSPHRASE: &str = "wrong passphrase";

/// The cause for a file gpg reads as no OpenPGP data.
pub const NOT_OPENPGP: &str = "it is not an OpenPGP file";

/// The cause when gpg made no file.
pub const MADE_NOTHING: &str = "gpg made no file";

/// The verdicts of a signature (R29 3.3).
pub mod verdict {
    /// A good signature: "Signature good: key ending 4E7B" (31).
    pub fn good(key: &str) -> String {
        format!("Signature good: key ending {key}")
    }

    /// Said after every good signature.
    pub const GOOD_MEANS: &str = "Good means the key fits, not who holds it.";

    /// Said after a good signature from a key the owner has not certified.
    pub const NOT_CERTIFIED: &str = "You have not certified this key";

    /// A bad signature: "Signature bad: the file was changed" (35).
    pub const BAD: &str = "Signature bad: the file was changed";

    /// The public key is missing: "Cannot check: public key is missing"
    /// (35).
    pub const NO_PUBLIC_KEY: &str = "Cannot check: public key is missing";

    /// Said after a missing key: diroxide never fetches one.
    pub const NO_FETCH: &str = "diroxide does not fetch keys.";

    /// A good signature from an expired key: "Good signature, but the key
    /// expired" (35).
    pub const KEY_EXPIRED: &str = "Good signature, but the key expired";

    /// A good signature that has itself expired.
    pub const SIGNATURE_EXPIRED: &str = "Good signature, but it has expired";

    /// A revoked key: "Key revoked. Do not trust this file." (36; R29
    /// counted 37).
    pub const REVOKED: &str = "Key revoked. Do not trust this file.";

    /// A signature made by another key than the one given (minisign).
    pub const OTHER_KEY: &str = "Cannot check: signed by another key";

    /// A signature file that cannot be read.
    pub const UNREADABLE: &str = "Cannot check: signature unreadable";

    /// A public key that cannot be read (minisign).
    pub const BAD_PUBLIC_KEY: &str = "Cannot check: public key unreadable";

    /// A signature gpg could not check for another reason.
    pub const CANNOT_CHECK: &str = "Cannot check this signature";

    /// The key's own text, after its ending: "Key ending 4E7B is named Ada
    /// Example <ada@example.invalid>". The text is the key owner's own
    /// choice: cleaned of hidden characters and cut at 120 characters.
    pub fn named(key: &str, name: &str) -> String {
        format!("Key ending {key} is named {}", super::clean_text(name))
    }

    /// minisign's trusted comment, cleaned: "Trusted comment: timestamp
    /// 1555779966, file test".
    pub fn comment(text: &str) -> String {
        format!("Trusted comment: {}", super::clean_text(text))
    }
}

/// The chain's sentences (R29 3.5, step 6), the same words as the
/// checksum list's own `signature_line`.
pub mod chain {
    /// A good signature on the list: "List is signed: 12 of 12 files
    /// match" (36).
    pub fn signed(matched: usize, files: usize) -> String {
        format!("List is signed: {matched} of {files} files match")
    }

    /// No signature beside the list: "List not signed: 12 of 12 match"
    /// (31).
    pub fn not_signed(matched: usize, files: usize) -> String {
        format!("List not signed: {matched} of {files} match")
    }

    /// A signature that is there and not good, or cannot be checked:
    /// "List not verified: 12 of 12 match" (33), after the verdict.
    pub fn not_good(matched: usize, files: usize) -> String {
        format!("List not verified: {matched} of {files} match")
    }

    /// What a match without a good signature shows, and what it does not.
    pub const ONLY_DAMAGE: &str =
        "This shows the files are not damaged. It does not show who made them.";
}

/// Text from a key or a signature, which its maker chose: hidden and
/// control characters made `?`, runs of spaces kept to one, cut at 120
/// characters with "..." after.
pub fn clean_text(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let shown = diroxide_core::names::display_name(&spaced);
    let mut out = String::new();
    let mut space = false;
    for c in shown.chars() {
        let c = if c.is_control() { '?' } else { c };
        if c.is_whitespace() {
            if !space && !out.is_empty() {
                out.push(' ');
            }
            space = true;
            continue;
        }
        space = false;
        out.push(c);
    }
    let out = out.trim_end().to_owned();
    if out.chars().count() > 120 {
        let cut: String = out.chars().take(117).collect();
        format!("{cut}...")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    fn fits(line: &str) {
        assert!(
            line.chars().count() <= BRAILLE_CELLS,
            "{line} is {} cells",
            line.chars().count()
        );
    }

    #[test]
    fn the_lines_of_r29_as_built() {
        let cases = [
            (GPG_NOT_FOUND.to_owned(), 35),
            (verdict::good("4E7B"), 31),
            (verdict::BAD.to_owned(), 35),
            (verdict::NO_PUBLIC_KEY.to_owned(), 35),
            (verdict::KEY_EXPIRED.to_owned(), 35),
            (verdict::REVOKED.to_owned(), 36),
            (chain::signed(12, 12), 36),
            (chain::not_signed(12, 12), 31),
            (chain::not_good(12, 12), 33),
            (plan_for_recipients(3, 1), 31),
        ];
        for (line, cells) in cases {
            assert_eq!(line.chars().count(), cells, "{line}");
        }
        assert_eq!(verdict::good("4E7B"), "Signature good: key ending 4E7B");
        assert_eq!(
            chain::signed(12, 12),
            "List is signed: 12 of 12 files match"
        );
        assert_eq!(chain::not_signed(12, 12), "List not signed: 12 of 12 match");
        // R29's sentence, 42 cells, kept whole: its point is in the first
        // 40 ("Good means the key fits, not who holds").
        assert_eq!(verdict::GOOD_MEANS.chars().count(), 42);
        for line in [
            verdict::NOT_CERTIFIED,
            verdict::SIGNATURE_EXPIRED,
            verdict::OTHER_KEY,
            verdict::UNREADABLE,
            verdict::CANNOT_CHECK,
            NOT_PROOF,
            &plan_sign(3, Some("4E7B")),
            &plan_with_passphrase(3),
            &plan_decrypt(12),
            &no_secret_key(Some("4E7B")),
            &gave_up("report.pdf.gpg", 3),
            &encrypted("report.pdf", "report.pdf.gpg"),
            &decrypted("report.pdf.gpg", "report.pdf"),
            &signed("report.pdf", "report.pdf.sig"),
        ] {
            fits(line);
        }
        // Over 40, the meaning first.
        assert!(NOT_CERTIFIED.starts_with("Key not certified"));
        assert!(verdict::BAD_PUBLIC_KEY.starts_with("Cannot check"));
    }

    #[test]
    fn a_key_owners_text_is_cleaned_and_cut() {
        assert_eq!(
            verdict::named("4E7B", "Ada  Example\u{202E}<ada@example.invalid>\n"),
            "Key ending 4E7B is named Ada Example?<ada@example.invalid>"
        );
        let long = "x".repeat(300);
        assert_eq!(clean_text(&long).chars().count(), 120);
        assert_eq!(recipient_not_found("bob\u{7}"), "Key not found: bob?");
    }

    #[test]
    fn no_line_overclaims() {
        let all = [
            verdict::GOOD_MEANS,
            verdict::NOT_CERTIFIED,
            chain::ONLY_DAMAGE,
            NOT_PROOF,
            &chain::signed(1, 1),
        ];
        for line in all {
            let lower = line.to_lowercase();
            for word in ["safe", "genuine", "authored", "authentic", "trusted"] {
                assert!(!lower.contains(word), "{line}");
            }
        }
    }
}
