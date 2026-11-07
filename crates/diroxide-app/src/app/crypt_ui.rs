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
//! - **Encrypt with gpg** asks for the recipients' keys (or none, for a
//!   passphrase typed twice in the masked prompt), **Decrypt with gpg**
//!   asks nothing when gpg's agent holds the key and the masked prompt
//!   otherwise ("Passphrase for key ending 4E7B (hidden):"), **Sign with
//!   gpg** makes a detached `NAME.sig` with a key or the default one. Each
//!   is planned on a worker through ops' gpg driver, asked about starting
//!   on No with the plan's lines, and run as a job Cancel stops; Undo
//!   puts what it made in the trash. Without gpg: "gpg not found. Install
//!   GnuPG first."
//! - **Verify signature** checks the focused file's signature beside it
//!   (`NAME.asc`, `NAME.sig`, `NAME.minisig`, or the checksum list's
//!   `SHA256SUMS.asc`), or the focused signature against its file: gpg
//!   for OpenPGP, minisign in process with a public key asked by path or
//!   pasted. The verdict comes first ("Signature good: key ending 4E7B"),
//!   then what good means. diroxide never fetches a key.
//!
//! The lists, fields and questions are hosted under the Organize
//! purpose, so the window's test of every purpose keeps its count (the
//! fold lane's CR5 gives them a purpose of their own).

use diroxide_core::{Command, CommandId, Effect, EntryKind, JobKind, JobOrigin, JobRequest};
use diroxide_core::{Message, Place, Redraw, Side, names::display_name};
use diroxide_ops::crypt::{
    GpgAction, GpgOptions, GpgPlan, GpgReport, SignatureCheck, SignatureVerdict, sidecars_present,
};
use diroxide_ops::undo::UndoRecord;

use super::organize::Wants;
use super::{App, Flow, Purpose, Then};
use crate::commands::ids;
use crate::crypt::{self, CryptAsk, CryptNote, GpgToRun, Made};
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::request::Request;

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
    /// Encrypt with gpg: the recipients typed, or none for a passphrase.
    Recipients {
        /// The panel.
        side: Side,
        /// The files.
        sources: Vec<Place>,
    },
    /// Sign with gpg: the key typed, or none for the default.
    Signer {
        /// The panel.
        side: Side,
        /// The files.
        sources: Vec<Place>,
    },
    /// A gpg plan's question: Yes runs it.
    Run {
        /// The panel it was asked from.
        side: Side,
        /// The plan.
        plan: GpgToRun,
    },
    /// Verify signature on a minisign signature: its public key typed.
    MinisignKey {
        /// The file the owner chose.
        file: Place,
        /// What to check, its key still to come.
        check: Box<SignatureCheck>,
    },
}

/// The submenu's gpg commands.
const GPG: [CommandId; 4] = [
    ids::GPG_ENCRYPT,
    ids::GPG_DECRYPT,
    ids::SIGN,
    ids::VERIFY_SIGNATURE,
];

/// The extensions a signature file has, and whether minisign checks it.
const SIGNATURES: [(&str, bool); 3] = [(".asc", false), (".sig", false), (".minisig", true)];

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
            .map(|(_, local)| *local);
        if local.is_none() && !GPG.contains(&command.id) {
            return None;
        }
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        Some(match local {
            Some(local) => self.age_command(local),
            None if command.id == ids::VERIFY_SIGNATURE => self.verify_signature(),
            None => self.gpg_command(&command.id),
        })
    }

    /// The marked files, or the focused one, folders left out; or the
    /// line said instead ("Encrypt with gpg works on files; no file is
    /// chosen").
    fn crypt_files(&self, side: Side, what: &str) -> Result<Vec<Place>, String> {
        let Some((places, _)) = self.frozen(side) else {
            return Err(self.panel(side).list().empty_text().to_owned());
        };
        let panel = self.panel(side);
        let files: Vec<Place> = places
            .into_iter()
            .filter(|place| {
                place
                    .name()
                    .and_then(|name| panel.entry(name))
                    .is_some_and(|e| matches!(e.kind, EntryKind::File | EntryKind::Archive))
            })
            .collect();
        if files.is_empty() {
            return Err(format!("{what} works on files; no file is chosen"));
        }
        Ok(files)
    }

    /// Encrypt, Decrypt or Sign with gpg: its field first, or the plan.
    fn gpg_command(&mut self, id: &CommandId) -> Vec<Effect> {
        let side = self.active;
        let what = crate::commands::name_of(id);
        let sources = match self.crypt_files(side, &what) {
            Ok(sources) => sources,
            Err(line) => return vec![self.say(Message::status(line))],
        };
        if *id == ids::GPG_DECRYPT {
            let options = GpgOptions {
                action: GpgAction::Decrypt,
                armor: false,
            };
            return self.plan_gpg(side, sources, options);
        }
        let (label, wants) = if *id == ids::GPG_ENCRYPT {
            (
                crypt::RECIPIENTS_LABEL,
                CryptWants::Recipients { side, sources },
            )
        } else {
            (crypt::SIGNER_LABEL, CryptWants::Signer { side, sources })
        };
        let field = EntryField::new(label, CompletionSource::None);
        self.open_organize(Dialog::edit(field), Wants::Crypt(wants))
    }

    /// Asks the runtime for a gpg plan; the question follows.
    fn plan_gpg(&mut self, side: Side, sources: Vec<Place>, options: GpgOptions) -> Vec<Effect> {
        let job = self.next_job_id();
        self.flows.insert(
            job,
            Flow {
                side,
                note: None,
                then: Then::Report,
                confirm: true,
                kind: options.kind(),
            },
        );
        self.request(Request::Crypt(Box::new(CryptAsk::Plan {
            job,
            sources,
            options,
        })));
        vec![self.say(Message::status("Planning with gpg"))]
    }

    /// What the runtime answered the gpg commands or Verify signature.
    pub(super) fn crypt_note(&mut self, note: CryptNote) -> Vec<Effect> {
        match note {
            CryptNote::Planned { job, result } => {
                let side = self.flows.remove(&job).map_or(self.active, |f| f.side);
                match result {
                    Ok(plan) => self.gpg_question(side, plan),
                    Err(line) => vec![self.say(Message::error(line))],
                }
            }
            CryptNote::Finished(report) => self.gpg_finished(*report),
            CryptNote::Verified {
                file,
                check,
                verdict,
            } => self.signature_verified(&file, &check, &verdict),
        }
    }

    /// A gpg plan's question, starting on No, with its lines; or, when no
    /// file can be worked on, the plan's summary and why.
    fn gpg_question(&mut self, side: Side, plan: Box<GpgPlan>) -> Vec<Effect> {
        let lines = plan.lines();
        if plan.items.is_empty() {
            let mut text = lines.join(". ");
            text.push('.');
            return vec![self.say(Message::info(text))];
        }
        let question = with_default_no(Message::question(&plan.summary()));
        let listing = lines.into_iter().skip(1).collect();
        self.open_organize(
            Dialog::Confirm {
                question,
                listing,
                on_yes: Command::new(ids::YES),
            },
            Wants::Crypt(CryptWants::Run {
                side,
                plan: GpgToRun(plan),
            }),
        )
    }

    /// A gpg job ended: its words (the file made, or the summary and the
    /// first failure), its undo record through the job's own end, and
    /// after an encryption the trash offered for the originals.
    fn gpg_finished(&mut self, report: GpgReport) -> Vec<Effect> {
        let lines = report.lines();
        let one_made = report.made.len() == 1 && report.job.failed.is_empty();
        let said: Vec<String> = if one_made {
            lines.iter().skip(1).cloned().collect()
        } else {
            let mut said = vec![lines.first().cloned().unwrap_or_default()];
            said.extend(lines.iter().find(|l| l.starts_with("Could not ")).cloned());
            said
        };
        self.finish_lines = Some(said);
        let mut effects = self.job_finished(report.job.clone());
        self.finish_lines = None;
        let encrypted = matches!(
            report.action,
            GpgAction::Encrypt { .. } | GpgAction::EncryptWithPassphrase
        );
        if encrypted && !report.made.is_empty() {
            let made = Made {
                encrypted: true,
                pairs: report.made,
            };
            // The job's own end recorded its undo.
            effects.extend(self.crypt_made(Some(made), true));
        }
        effects
    }

    /// Verify signature on the focused file: its signature beside it, or
    /// itself against its file.
    fn verify_signature(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let Some(entry) = panel.focused_entry() else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if !matches!(entry.kind, EntryKind::File | EntryKind::Archive) {
            let line = format!(
                "Verify signature works on files: {} is a folder",
                entry.display_name()
            );
            return vec![self.say(Message::status(line))];
        }
        let name = entry.name.clone();
        let folder = panel.place().clone();
        let siblings: Vec<&str> = panel
            .all_entries()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        let found = |wanted: &str| {
            siblings
                .iter()
                .find(|s| s.eq_ignore_ascii_case(wanted))
                .map(|s| (*s).to_owned())
        };
        let lower = name.to_ascii_lowercase();
        let own = SIGNATURES
            .iter()
            .find(|(ext, _)| lower.ends_with(ext) && lower.len() > ext.len());
        let (signature, data) = match own {
            // The focused file is a signature: its file is the name
            // without the extension; a gpg one may be signed inline.
            Some((ext, minisign)) => {
                let data = found(&name[..name.len() - ext.len()]);
                if data.is_none() && *minisign {
                    let line = format!("{} signs no file here", display_name(&name));
                    return vec![self.say(Message::info(line))];
                }
                (name.clone(), data)
            }
            None => match sidecars_present(&name, &siblings).into_iter().next() {
                Some(sidecar) => (sidecar.name, found(&sidecar.signs)),
                None => {
                    let line = crypt::no_signature(&display_name(&name));
                    return vec![self.say(Message::info(line))];
                }
            },
        };
        let (Ok(file), Ok(signature)) = (folder.join(&name), folder.join(&signature)) else {
            return Vec::new();
        };
        let check = SignatureCheck {
            signature,
            data: data.and_then(|d| folder.join(&d).ok()),
            minisign_key: None,
        };
        if check.is_minisign() {
            let field = EntryField::new(crypt::MINISIGN_KEY_LABEL, CompletionSource::Paths);
            return self.open_organize(
                Dialog::edit(field),
                Wants::Crypt(CryptWants::MinisignKey {
                    file,
                    check: Box::new(check),
                }),
            );
        }
        self.request_verify(file, check)
    }

    /// Asks the runtime to check a signature.
    fn request_verify(&mut self, file: Place, check: SignatureCheck) -> Vec<Effect> {
        self.request(Request::Crypt(Box::new(CryptAsk::Verify {
            file,
            check: Box::new(check),
        })));
        vec![self.say(Message::status("Checking the signature"))]
    }

    /// A signature's verdict: its line said first, then its other lines
    /// to read; the list's signature says whose it is.
    fn signature_verified(
        &mut self,
        file: &Place,
        check: &SignatureCheck,
        verdict: &SignatureVerdict,
    ) -> Vec<Effect> {
        let mut lines = verdict.lines();
        let first = lines.remove(0);
        let name = file.name().unwrap_or_default();
        let signs_list = check
            .data
            .as_ref()
            .and_then(|d| d.name())
            .is_some_and(|d| d != name && check.signature.name() != Some(name));
        if signs_list && let Some(list) = check.data.as_ref().and_then(|d| d.name()) {
            lines.push(crypt::list_signature(list));
        }
        let message = if verdict.is_good() {
            Message::done(first.clone())
        } else {
            Message::warning(first.clone())
        };
        if lines.is_empty() || !self.dialogs.is_empty() {
            return vec![self.say(message)];
        }
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        effects.extend(self.open_dialog(
            Dialog::Read {
                title: first,
                lines,
            },
            Purpose::Read,
        ));
        effects
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

    /// Enter in a dialog of the submenu: a field's text taken; a question
    /// answers on Yes.
    pub(super) fn crypt_accept(&mut self, wants: CryptWants, text: &str) -> Vec<Effect> {
        match wants {
            CryptWants::Recipients { side, sources } => {
                let mut effects = self.close_top();
                let recipients = crypt::recipients_of(text);
                let action = if recipients.is_empty() {
                    GpgAction::EncryptWithPassphrase
                } else {
                    GpgAction::Encrypt { recipients }
                };
                let options = GpgOptions {
                    action,
                    armor: false,
                };
                effects.extend(self.plan_gpg(side, sources, options));
                effects
            }
            CryptWants::Signer { side, sources } => {
                let mut effects = self.close_top();
                let key = Some(text.trim().to_owned()).filter(|k| !k.is_empty());
                let options = GpgOptions {
                    action: GpgAction::Sign { key },
                    armor: false,
                };
                effects.extend(self.plan_gpg(side, sources, options));
                effects
            }
            CryptWants::MinisignKey { file, check } => {
                let mut effects = self.close_top();
                if text.trim().is_empty() {
                    effects
                        .push(self.say(Message::info("No public key given; nothing was checked.")));
                    return effects;
                }
                let folder = check.signature.parent();
                let folder = folder.as_ref().and_then(|f| f.local_path());
                let key = crypt::minisign_key_of(text, folder.map(std::path::Path::new));
                effects.extend(self.request_verify(file, check.with_minisign_key(key)));
                effects
            }
            CryptWants::TrashOriginals { .. } | CryptWants::Run { .. } => Vec::new(),
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
            CryptWants::Run { side, plan } => {
                let job = plan.0.job;
                let kind = plan.0.kind();
                let line = start_line(&plan.0.summary());
                let now = self.now();
                self.jobs.begin(job, kind, line.clone(), now);
                self.flows.insert(
                    job,
                    Flow {
                        side,
                        note: None,
                        then: Then::Report,
                        confirm: false,
                        kind,
                    },
                );
                self.request(Request::Crypt(Box::new(CryptAsk::Run(plan))));
                vec![
                    Effect::Redraw(Redraw::JobsLine),
                    self.say(Message::info(line)),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// No or Escape on a dialog of the submenu.
    pub(super) fn crypt_escape(&mut self, wants: CryptWants) -> Vec<Effect> {
        match wants {
            CryptWants::TrashOriginals { .. } => {
                vec![self.say(Message::info(crypt::ORIGINALS_STAY))]
            }
            CryptWants::Run { .. } => vec![self.say(Message::info("Nothing changed."))],
            _ => Vec::new(),
        }
    }
}

/// The line said when a gpg job starts, the verb first: "Encrypt
/// started: 3 files for 1 recipient".
fn start_line(summary: &str) -> String {
    match summary.split_once(' ') {
        Some((verb, rest)) => format!("{verb} started: {rest}"),
        None => format!("{summary} started"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_line_puts_the_verb_first_in_a_braille_line() {
        let line = start_line("Encrypt 3 files for 1 recipient");
        assert_eq!(line, "Encrypt started: 3 files for 1 recipient");
        assert!(line.chars().count() <= 40);
        assert_eq!(start_line("Decrypt 1 file"), "Decrypt started: 1 file");
    }
}
