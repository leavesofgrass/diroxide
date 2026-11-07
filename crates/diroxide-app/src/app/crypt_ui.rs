//! The Encryption submenu in the app (alpha 4, lane AE; R29 3.2 to 3.5,
//! R30 commits 71 to 73's app half; the words and requests are in
//! [`crate::crypt`]).
//!
//! - **Encrypt with a passphrase**, **Encrypt for a recipient** and
//!   **Decrypt** run the encryption plugin's commands (age, in the
//!   `diro-crypt` plugin) with the marked files or the focused one; the
//!   plugin asks every secret in hidden fields, an identity file by its
//!   path, and keeps nothing. Without the plugin each offers its
//!   component, starting on No ("Encrypting with age needs a plugin.").
//! - What an encryption or a decryption made is recorded for Undo, which
//!   puts the new files in the trash; after an encryption the app asks
//!   "Move the original to the trash?", starting on No, and says the
//!   trash is not secure. The originals are never removed otherwise.
//!
//! The question is hosted under the Organize purpose, so the window's
//! test of every purpose keeps its count (the fold lane's CR5 gives it a
//! purpose of its own).

use diroxide_core::{Command, CommandId, Effect, JobKind, JobOrigin, JobRequest, Message, Place};
use diroxide_core::{Side, names::display_name};
use diroxide_ops::undo::UndoRecord;

use super::organize::Wants;
use super::{App, Then};
use crate::commands::ids;
use crate::crypt::{self, Made};
use crate::dialog::{Dialog, with_default_no};

/// What a dialog of the Encryption submenu is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CryptWants {
    /// After an encryption: the originals to the trash, on Yes.
    TrashOriginals {
        /// The panel they are in.
        side: Side,
        /// The originals.
        places: Vec<Place>,
    },
}

/// The submenu's age commands and the plugin command each runs.
const AGE: [(CommandId, &str); 3] = [
    (ids::ENCRYPT_PASSPHRASE, "encrypt_passphrase"),
    (ids::ENCRYPT_RECIPIENT, "encrypt_recipient"),
    (ids::DECRYPT, "decrypt"),
];

impl App {
    /// The Encryption submenu's commands; `None` for any other.
    pub(super) fn crypt_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let local = AGE
            .iter()
            .find(|(id, _)| *id == command.id)
            .map(|(_, local)| *local)?;
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(self.age_command(local))
    }

    /// An age command: the plugin's, run as any plugin command is (its
    /// consent, its conditions, a job in the jobs line); without the
    /// plugin, its component offered.
    fn age_command(&mut self, local: &str) -> Vec<Effect> {
        let Some(id) = crypt::plugin_command(local) else {
            return Vec::new();
        };
        if self.plugins.catalog.any_command(&id).is_none() {
            return self
                .ask_for_feature(crypt::FEATURE)
                .unwrap_or_else(|| vec![self.say(Message::info(crypt::needs_plugin()))]);
        }
        self.plugins_dispatch(&Command::new(id)).unwrap_or_default()
    }

    /// A plugin command ended with `made` (its answer read by
    /// [`crypt::made_by`]): the new files recorded for Undo unless the
    /// plugin committed them itself (`committed`), then, after an
    /// encryption, the trash offered for the originals.
    pub(super) fn crypt_made(&mut self, made: Option<Made>, committed: bool) -> Vec<Effect> {
        let Some(made) = made else {
            return Vec::new();
        };
        if !committed {
            let job = self.next_job_id();
            let kind = if made.encrypted {
                JobKind::Encrypt
            } else {
                JobKind::Decrypt
            };
            let mut record = UndoRecord::for_job(job, kind);
            record.origin = JobOrigin::Plugin(self.plugin_name(crypt::PLUGIN));
            record.created = made.pairs.iter().map(|(_, to)| to.clone()).collect();
            self.undo.record(record);
        }
        if !made.encrypted {
            return Vec::new();
        }
        let places: Vec<Place> = made.pairs.into_iter().map(|(from, _)| from).collect();
        if !self.dialogs.is_empty() {
            // Another dialog is in front: the originals simply stay.
            return vec![self.say(Message::info(crypt::ORIGINALS_STAY))];
        }
        let question = with_default_no(Message::question(&crypt::trash_question(places.len())));
        let mut listing: Vec<String> = places
            .iter()
            .map(|p| display_name(p.name().unwrap_or_default()).into_owned())
            .collect();
        listing.push(crypt::trash_not_secure(places.len()));
        let side = self.active;
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Wants::Crypt(CryptWants::TrashOriginals { side, places }),
        )
    }

    /// The plugin's name as the catalog has it, for an undo record's
    /// origin; its id when it is not found.
    fn plugin_name(&self, plugin: &str) -> String {
        self.plugins
            .catalog
            .items
            .iter()
            .find(|item| item.id == plugin)
            .map_or_else(|| plugin.to_owned(), |item| item.name.clone())
    }

    /// Enter in a dialog of the submenu: a question answers on Yes.
    pub(super) fn crypt_accept(&mut self, wants: CryptWants, _text: &str) -> Vec<Effect> {
        match wants {
            CryptWants::TrashOriginals { .. } => Vec::new(),
        }
    }

    /// Yes to a question of the submenu.
    pub(super) fn crypt_yes(&mut self, wants: CryptWants) -> Vec<Effect> {
        match wants {
            CryptWants::TrashOriginals { side, places } => {
                // The question was the confirmation: the trash job plans
                // and runs as any, its own words at its end.
                let job = self.next_job_id();
                let request = JobRequest::new(job, JobKind::Trash, places);
                self.plan_job(side, request, None, Then::Report, false)
            }
        }
    }

    /// No or Escape on a question of the submenu.
    pub(super) fn crypt_escape(&mut self, wants: CryptWants) -> Vec<Effect> {
        match wants {
            CryptWants::TrashOriginals { .. } => {
                vec![self.say(Message::info(crypt::ORIGINALS_STAY))]
            }
        }
    }
}
