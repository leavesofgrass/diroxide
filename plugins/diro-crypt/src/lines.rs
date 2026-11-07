//! Every sentence the plugin says, its meaning in the first 40 cells and
//! words before any symbol (R29 3.2, R9's messages). The host adds the
//! plugin's name to titles and lines, so nothing here does. A file's name
//! is said with any hidden character spelled out (`[U+000A]`), so a name
//! built to look like another reads as what it is.

use diroxide_core::format::count;
use diroxide_core::names::spell_out;

/// A name for a sentence: hidden characters spelled out.
pub fn said(name: &str) -> String {
    spell_out(name).into_owned()
}

/// The plan line before an encryption for recipients: "Encrypt 3 files
/// for 1 recipient" (31).
pub fn plan_for_recipients(files: usize, recipients: usize) -> String {
    format!(
        "Encrypt {} for {}",
        count(files as u64, "file", "files"),
        count(recipients as u64, "recipient", "recipients")
    )
}

/// The plan line before an encryption with a passphrase: "Encrypt 3
/// files with a passphrase" (33).
pub fn plan_with_passphrase(files: usize) -> String {
    format!(
        "Encrypt {} with a passphrase",
        count(files as u64, "file", "files")
    )
}

/// The plan line before a decryption: "Decrypt 3 files" (15).
pub fn plan_decrypt(files: usize) -> String {
    format!("Decrypt {}", count(files as u64, "file", "files"))
}

/// The form's title for a passphrase.
pub const PASSPHRASE_TITLE: &str = "Encrypt with a passphrase";

/// The form's title for recipients.
pub const RECIPIENT_TITLE: &str = "Encrypt for a recipient";

/// The passphrase field's label.
pub const PASSPHRASE_LABEL: &str = "Passphrase";

/// The second passphrase field's label.
pub const REPEAT_LABEL: &str = "Repeat the passphrase";

/// The armor field's label: a checkbox, its row "ASCII armor: off".
pub const ARMOR_LABEL: &str = "ASCII armor";

/// The pasted key field's label.
pub const KEY_LABEL: &str = "Public key, starting age1";

/// The recipients file field's label.
pub const RECIPIENTS_FILE_LABEL: &str = "Or a recipients file";

/// The form's default button.
pub const ENCRYPT_BUTTON: &str = "Encrypt";

/// Said in the passphrase form after the two did not match.
pub const PASSPHRASES_DIFFER: &str = "The two passphrases differ. Nothing was changed. Try again.";

/// Said in the passphrase form after an empty one.
pub const PASSPHRASE_EMPTY: &str = "A passphrase cannot be empty.";

/// Read before the passphrase fields: that it cannot be got back.
pub const PASSPHRASE_WARNING: &str = "Typing is hidden. A lost passphrase cannot be got back, so note it where you keep such things.";

/// Read before the recipient fields.
pub const RECIPIENT_SUMMARY: &str = "Paste an age public key, or give a file of them, one a line.";

/// Said when neither a key nor a file was given.
pub const NO_RECIPIENT: &str = "No recipient: paste a key or give a file";

/// The question's title for a passphrase: "Passphrase for
/// report.pdf.age"; the host says it with "(hidden)" after it.
pub fn passphrase_title(name: &str) -> String {
    format!("Passphrase for {}", said(name))
}

/// The whole prompt as R29 words it: "Passphrase for report.pdf.age
/// (hidden):" (39).
pub fn passphrase_prompt(name: &str) -> String {
    format!("{} (hidden):", passphrase_title(name))
}

/// The question's title for an identity file: "Identity file for
/// report.pdf.age" (32).
pub fn identity_title(name: &str) -> String {
    format!("Identity file for {}", said(name))
}

/// A wrong passphrase, said in the next question: "Wrong passphrase for
/// report.pdf.age. Try again, 2 of 3."
pub fn try_again(name: &str, attempt: u32, allowed: u32) -> String {
    format!(
        "Wrong passphrase for {}. Try again, {attempt} of {allowed}.",
        said(name)
    )
}

/// After the last try: "Gave up on report.pdf.age: 3 wrong tries" (40).
pub fn gave_up(name: &str, tries: u32) -> String {
    format!(
        "Gave up on {}: {}",
        said(name),
        count(u64::from(tries), "wrong try", "wrong tries")
    )
}

/// One file encrypted: "Encrypted report.pdf to report.pdf.age" (38).
pub fn encrypted(from: &str, to: &str) -> String {
    format!("Encrypted {} to {}", said(from), said(to))
}

/// One file decrypted: "Decrypted report.pdf.age to report.pdf." (39).
pub fn decrypted(from: &str, to: &str) -> String {
    format!("Decrypted {} to {}.", said(from), said(to))
}

/// Said once after a decryption: age carries no signature.
pub const NOT_PROOF: &str = "This does not prove who made it.";

/// Said once after an encryption: the original is still there, and the
/// trash is no secure delete.
pub const ORIGINAL_STAYS: &str = "The original is still here. The trash does not erase it.";

/// A run of several files: "Encrypted 3 of 3 files", "Decrypted 2 of 3
/// files; 1 failed".
pub fn summary(verb: &str, done: usize, of: usize, failed: usize, skipped: usize) -> String {
    let mut line = format!("{verb} {done} of {}", count(of as u64, "file", "files"));
    if failed > 0 {
        line.push_str(&format!("; {failed} failed"));
    }
    if skipped > 0 {
        line.push_str(&format!("; {skipped} skipped"));
    }
    line
}

/// A file that could not be encrypted: "Could not encrypt notes.txt"; the
/// cause is said after it, a sentence of its own.
pub fn could_not_encrypt(name: &str) -> String {
    format!("Could not encrypt {}", said(name))
}

/// A file that could not be decrypted: "Could not decrypt report.pdf.age"
/// (32).
pub fn could_not_decrypt(name: &str) -> String {
    format!("Could not decrypt {}", said(name))
}

/// The cause for a file whose passphrase asks for more work than the cap.
pub const TOO_MUCH_WORK: &str = "it asks for more work than diroxide allows. It may be hostile.";

/// The cause for a wrong passphrase.
pub const WRONG_PASSPHRASE: &str = "wrong passphrase, or the file was changed.";

/// The cause when no key of the identity file opens the file.
pub const NO_MATCHING_KEY: &str = "none of the keys in that file can open it.";

/// The cause for a file that is not age's.
pub const NOT_AGE: &str = "it is not an age file.";

/// The cause for an age file from a newer version.
pub const NEWER_FORMAT: &str = "it is from a newer age; update the plugin.";

/// The cause for a file damaged or changed after it was made.
pub const DAMAGED: &str = "the file is damaged or was changed.";

/// The cause for a file encrypted to a hardware or plugin key.
pub const NEEDS_AGE_PLUGIN: &str = "it needs an age plugin key, which diroxide does not use.";

/// A skipped folder: "Skipped Photos: it is a folder" (30).
pub fn skipped_folder(name: &str) -> String {
    format!("Skipped {}: it is a folder", said(name))
}

/// The next step after a skipped folder.
pub const PACK_FIRST: &str = "Pack the folder first, then encrypt the archive.";

/// A skipped file whose passphrase or identity was not given.
pub fn skipped_unanswered(name: &str, what: &str) -> String {
    format!("Skipped {}: no {what} given", said(name))
}

/// A file that is not on this computer.
pub fn not_here(name: &str) -> String {
    format!("Skipped {}: not on this computer", said(name))
}

/// A file that is not there.
pub fn not_there(name: &str) -> String {
    format!("Skipped {}: it is not there", said(name))
}

/// Nothing to work on.
pub const NOTHING_MARKED: &str = "Nothing to do: mark the files first";

/// Progress on one file: "Encrypting 40 percent of backup.tar" (35).
pub fn progress(verb: &str, percent: u64, name: &str) -> String {
    format!("{verb} {percent} percent of {}", said(name))
}

/// A bad recipient key: "Not an age public key: line 2". The line is
/// counted from 1, the pasted text's line 1 when no file was given.
pub fn bad_recipient(line: usize) -> String {
    format!("Not an age public key: line {line}")
}

/// An SSH key among the recipients.
pub fn ssh_recipient(line: usize) -> String {
    format!("SSH keys are not used: line {line}")
}

/// A plugin recipient among the recipients.
pub fn plugin_recipient(line: usize) -> String {
    format!("Plugin keys are not used: line {line}")
}

/// A recipients file that is too large to be one.
pub const RECIPIENTS_TOO_LARGE: &str = "The recipients file is over 1 MB";

/// A recipients file that could not be read: "Recipients file unreadable:
/// CAUSE".
pub fn recipients_unreadable(cause: &str) -> String {
    format!("Recipients file unreadable: {cause}")
}

/// An identity file that could not be read or holds no key.
pub fn identity_unusable(cause: &str) -> String {
    format!("Identity file unusable: {cause}")
}

/// The answer was cancelled, so the command stops.
pub const CANCELLED: &str = "Cancelled; nothing more was done";

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
    fn the_lines_of_r29_as_built_and_within_40_cells() {
        let cases = [
            (
                plan_for_recipients(3, 1),
                "Encrypt 3 files for 1 recipient",
                31,
            ),
            (
                encrypted("report.pdf", "report.pdf.age"),
                "Encrypted report.pdf to report.pdf.age",
                38,
            ),
            (
                decrypted("report.pdf.age", "report.pdf"),
                "Decrypted report.pdf.age to report.pdf.",
                39,
            ),
            (
                passphrase_prompt("report.pdf.age"),
                "Passphrase for report.pdf.age (hidden):",
                39,
            ),
            (
                could_not_decrypt("report.pdf.age"),
                "Could not decrypt report.pdf.age",
                32,
            ),
            (
                progress("Encrypting", 40, "backup.tar"),
                "Encrypting 40 percent of backup.tar",
                35,
            ),
            (
                identity_title("report.pdf.age"),
                "Identity file for report.pdf.age",
                32,
            ),
            (
                plan_with_passphrase(3),
                "Encrypt 3 files with a passphrase",
                33,
            ),
            (
                skipped_folder("Photos"),
                "Skipped Photos: it is a folder",
                30,
            ),
            (
                summary("Encrypted", 3, 3, 0, 0),
                "Encrypted 3 of 3 files",
                22,
            ),
            (
                summary("Decrypted", 2, 3, 1, 0),
                "Decrypted 2 of 3 files; 1 failed",
                32,
            ),
            (bad_recipient(2), "Not an age public key: line 2", 29),
        ];
        for (line, want, cells) in cases {
            assert_eq!(line, want);
            assert_eq!(line.chars().count(), cells, "{line}");
            fits(&line);
        }
        for line in [
            NOT_PROOF,
            NOTHING_MARKED,
            NO_RECIPIENT,
            KEY_LABEL,
            RECIPIENTS_FILE_LABEL,
            REPEAT_LABEL,
            PASSPHRASE_EMPTY,
            RECIPIENTS_TOO_LARGE,
            CANCELLED,
            &plan_decrypt(12),
            &gave_up("report.pdf.age", 3),
            &ssh_recipient(12),
            &plugin_recipient(3),
            &not_here("a.txt"),
            &not_there("a.txt"),
            &skipped_unanswered("a.age", "passphrase"),
        ] {
            fits(line);
        }
        assert_eq!(
            format!("{} {TOO_MUCH_WORK}", could_not_decrypt("report.pdf.age")),
            "Could not decrypt report.pdf.age it asks for more work than diroxide allows. It may be hostile."
        );
        assert_eq!(
            try_again("report.pdf.age", 2, 3),
            "Wrong passphrase for report.pdf.age. Try again, 2 of 3."
        );
        assert!(try_again("report.pdf.age", 2, 3).starts_with("Wrong passphrase"));
    }

    #[test]
    fn a_hidden_character_in_a_name_is_spelled_out() {
        let line = encrypted("two\nlines.txt", "two\nlines.txt.age");
        assert!(!line.contains('\n'));
        assert!(line.starts_with("Encrypted two[U+000A]lines.txt"));
        assert_eq!(said("plain.txt"), "plain.txt");
    }

    #[test]
    fn the_words_never_promise_what_age_cannot() {
        for line in [NOT_PROOF, ORIGINAL_STAYS, PASSPHRASE_WARNING] {
            let lower = line.to_lowercase();
            for word in ["safe", "genuine", "authored", "secure delete", "erased"] {
                assert!(
                    !lower.contains(word) || lower.contains("does not erase"),
                    "{line}"
                );
            }
        }
    }
}
