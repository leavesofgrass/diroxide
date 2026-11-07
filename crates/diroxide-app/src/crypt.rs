//! The Encryption submenu's words, and what its commands ask of the
//! runtime (alpha 4, lane AE; R29 3.2 to 3.5, ADR-0015). The app's side
//! is `app::crypt_ui`, the runtime's `runtime::crypt`.
//!
//! - age runs in the `diro-crypt` plugin (the Encryption component):
//!   "Encrypt with a passphrase", "Encrypt for a recipient" and "Decrypt"
//!   run its three commands, which ask every secret in hidden fields and
//!   keep none; without the plugin they offer the component, "Encrypting
//!   with age needs a plugin." After an encryption the app offers to move
//!   the originals to the trash, starting on No, and says the trash is
//!   not secure.
//! - gpg runs as the program the tools registry finds, through ops' jobs:
//!   "Encrypt with gpg" (for keys, or with a passphrase typed twice),
//!   "Decrypt with gpg" (a key the agent holds asks nothing), "Sign with
//!   gpg" (a detached signature) and "Verify signature" (gpg or minisign,
//!   the verdict first). A passphrase goes through the masked prompt,
//!   never a command line.

use diroxide_core::{CommandId, JobId, Place};
use diroxide_ops::crypt::{GpgOptions, GpgPlan, GpgReport, SignatureCheck, SignatureVerdict};

/// What the gpg commands and Verify signature ask of the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum CryptAsk {
    /// Plan a gpg job on a worker; answered with [`CryptNote::Planned`].
    Plan {
        /// The job.
        job: JobId,
        /// The files.
        sources: Vec<Place>,
        /// What to do.
        options: GpgOptions,
    },
    /// Run a plan the owner confirmed; answered with
    /// [`CryptNote::Finished`].
    Run(GpgToRun),
    /// Check one signature; answered with [`CryptNote::Verified`].
    Verify {
        /// The file the owner chose, for the words.
        file: Place,
        /// What to check.
        check: SignatureCheck,
    },
}

/// A gpg plan the owner confirmed; two are equal when they are the same
/// job's.
#[derive(Clone, Debug)]
pub struct GpgToRun(pub Box<GpgPlan>);

impl PartialEq for GpgToRun {
    fn eq(&self, other: &Self) -> bool {
        self.0.job == other.0.job
    }
}

impl Eq for GpgToRun {}

/// What the runtime answers the gpg commands and Verify signature.
#[derive(Debug)]
pub enum CryptNote {
    /// A plan, or the sentence why there is none.
    Planned {
        /// The job.
        job: JobId,
        /// The plan, or why not.
        result: Result<Box<GpgPlan>, String>,
    },
    /// A gpg job ended.
    Finished(Box<GpgReport>),
    /// A signature's verdict.
    Verified {
        /// The file the owner chose.
        file: Place,
        /// What was checked.
        check: Box<SignatureCheck>,
        /// The verdict.
        verdict: Box<SignatureVerdict>,
    },
}

/// The field for Encrypt with gpg: the recipients, or empty for a
/// passphrase (40 cells).
pub const RECIPIENTS_LABEL: &str = "Encrypt for gpg keys (empty: passphrase)";

/// The field for Sign with gpg (39 cells).
pub const SIGNER_LABEL: &str = "Sign with gpg key (empty: your default)";

/// The field for a minisign public key (37 cells).
pub const MINISIGN_KEY_LABEL: &str = "minisign public key: a file or pasted";

/// The keys typed in the recipients field: split at commas, semicolons
/// and spaces, empty parts dropped.
pub fn recipients_of(text: &str) -> Vec<String> {
    text.split([',', ';', ' ', '\t'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A minisign key as typed: a pasted key (minisign's base64 starts
/// "RW") or a file's path, relative to `folder` when it is not whole.
pub fn minisign_key_of(
    text: &str,
    folder: Option<&std::path::Path>,
) -> diroxide_ops::crypt::MinisignKey {
    use diroxide_ops::crypt::MinisignKey;
    let text = text.trim();
    if text.starts_with("RW") && !text.contains(['/', '\\']) {
        return MinisignKey::Text(text.to_owned());
    }
    let path = std::path::PathBuf::from(text);
    match folder {
        Some(folder) if path.is_relative() => MinisignKey::File(folder.join(path)),
        _ => MinisignKey::File(path),
    }
}

/// Said when a file has no signature beside it: "No signature beside
/// report.pdf".
pub fn no_signature(name: &str) -> String {
    format!("No signature beside {name}")
}

/// Said after the verdict when the signature checked is the checksum
/// list's, not the file's own.
pub fn list_signature(list: &str) -> String {
    format!("That signature is on {list}; Verify checksum uses it.")
}

/// The line when a gpg plan cannot be made: gpg missing is a sentence of
/// its own ("gpg not found. Install GnuPG first."), the rest "Could not
/// encrypt: no recipient was given."
pub fn plan_refused(verb: &str, error: &diroxide_ops::crypt::GpgError) -> String {
    match error {
        diroxide_ops::crypt::GpgError::Missing => error.to_string(),
        other => format!("Could not {verb}: {other}."),
    }
}

/// The encryption plugin's id: its commands are `crypt.encrypt_passphrase`,
/// `crypt.encrypt_recipient` and `crypt.decrypt`.
pub const PLUGIN: &str = "crypt";

/// The components' feature the plugin serves.
pub const FEATURE: &str = "encryption";

/// The plugin's command a submenu item runs: `crypt.decrypt`.
pub fn plugin_command(local: &str) -> Option<CommandId> {
    CommandId::new(format!("{PLUGIN}.{local}")).ok()
}

/// Said when the plugin is not here and no component list can offer it:
/// "Encrypting with age needs a plugin. Manage optional components gets
/// it."
pub fn needs_plugin() -> String {
    format!(
        "{} needs a plugin. Manage optional components gets it.",
        diroxide_components::messages::feature_words(FEATURE)
    )
}

/// The question after an encryption, which starts on No: "Move the
/// original to the trash", "Move the 3 originals to the trash" (the
/// dialog adds "? No is the default. y or n").
pub fn trash_question(originals: usize) -> String {
    if originals == 1 {
        "Move the original to the trash".to_owned()
    } else {
        format!("Move the {originals} originals to the trash")
    }
}

/// The line under it: the trash keeps what it holds.
pub fn trash_not_secure(originals: usize) -> String {
    if originals == 1 {
        "The trash is not secure: the original can be restored from it.".to_owned()
    } else {
        "The trash is not secure: the originals can be restored from it.".to_owned()
    }
}

/// Said on No: nothing moved.
pub const ORIGINALS_STAY: &str = "The originals stay where they are.";

/// What an encryption command of the plugin made: each source and its
/// output, from the command's answer (`made: [{from, to}]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    /// Whether the files were encrypted (so the originals may go to the
    /// trash) rather than decrypted.
    pub encrypted: bool,
    /// Each source and what it made.
    pub pairs: Vec<(Place, Place)>,
}

/// What the plugin's answer says it made, for a command of the
/// encryption plugin: `None` for any other command or an answer with
/// nothing made.
pub fn made_by(command: &CommandId, answer: &serde_json::Value) -> Option<Made> {
    let (plugin, local) = command.as_str().split_once('.')?;
    if plugin != PLUGIN {
        return None;
    }
    let encrypted = match local {
        "encrypt_passphrase" | "encrypt_recipient" => true,
        "decrypt" => false,
        _ => return None,
    };
    let pairs: Vec<(Place, Place)> = answer
        .get("made")?
        .as_array()?
        .iter()
        .filter_map(|pair| {
            let from = Place::parse(pair.get("from")?.as_str()?).ok()?;
            let to = Place::parse(pair.get("to")?.as_str()?).ok()?;
            // A file made beside its source, never elsewhere.
            (to.parent() == from.parent()).then_some((from, to))
        })
        .collect();
    (!pairs.is_empty()).then_some(Made { encrypted, pairs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;
    use serde_json::json;

    fn local(name: &str) -> String {
        if cfg!(windows) {
            format!("D:\\work\\{name}")
        } else {
            format!("/work/{name}")
        }
    }

    #[test]
    fn the_words_say_their_meaning_first() {
        assert_eq!(
            needs_plugin(),
            "Encrypting with age needs a plugin. Manage optional components gets it."
        );
        assert!(needs_plugin().split('.').next().unwrap().chars().count() <= BRAILLE_CELLS);
        for line in [
            trash_question(1),
            trash_question(3),
            ORIGINALS_STAY.to_owned(),
        ] {
            assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
        }
        assert!(trash_not_secure(1).starts_with("The trash is not secure"));
        assert_eq!(
            plugin_command("decrypt").map(|c| c.to_string()),
            Some("crypt.decrypt".to_owned())
        );
        for line in [RECIPIENTS_LABEL, SIGNER_LABEL, MINISIGN_KEY_LABEL] {
            assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
        }
        assert_eq!(no_signature("report.pdf"), "No signature beside report.pdf");
        assert_eq!(
            plan_refused("encrypt", &diroxide_ops::crypt::GpgError::Missing),
            "gpg not found. Install GnuPG first."
        );
        assert_eq!(
            plan_refused("encrypt", &diroxide_ops::crypt::GpgError::NoRecipient),
            "Could not encrypt: no recipient was given."
        );
    }

    #[test]
    fn recipients_and_keys_are_read_as_typed() {
        assert_eq!(
            recipients_of(" 4E7B, ada@example.invalid;B0B "),
            ["4E7B", "ada@example.invalid", "B0B"]
        );
        assert!(recipients_of("  ").is_empty());
        use diroxide_ops::crypt::MinisignKey;
        assert_eq!(
            minisign_key_of(
                "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
                None
            ),
            MinisignKey::Text(
                "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3".to_owned()
            )
        );
        let folder = std::path::Path::new("work");
        assert_eq!(
            minisign_key_of("minisign.pub", Some(folder)),
            MinisignKey::File(folder.join("minisign.pub"))
        );
    }

    #[test]
    fn what_the_plugin_made_is_read_from_its_answer_and_nothing_else() {
        let answer = json!({"done": 1, "skipped": 0, "failed": [],
            "made": [{"from": local("report.pdf"), "to": local("report.pdf.age")},
                     {"from": local("a.txt"), "to": "/elsewhere/a.txt.age"}]});
        let made = made_by(&plugin_command("encrypt_passphrase").unwrap(), &answer).unwrap();
        assert!(made.encrypted);
        assert_eq!(made.pairs.len(), 1, "a file made elsewhere is ignored");
        assert_eq!(made.pairs[0].1.name(), Some("report.pdf.age"));
        let decrypt = made_by(&plugin_command("decrypt").unwrap(), &answer).unwrap();
        assert!(!decrypt.encrypted);
        let other = CommandId::new("line-counter.count").unwrap();
        assert!(made_by(&other, &answer).is_none());
        let nothing = json!({"done": 0, "made": []});
        assert!(made_by(&plugin_command("decrypt").unwrap(), &nothing).is_none());
    }
}
