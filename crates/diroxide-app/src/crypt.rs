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

use diroxide_core::{CommandId, Place};

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
