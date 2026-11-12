//! Encrypt, Decrypt and Sign over gpg, as jobs (R30 commit 72): a plan
//! first, then each file in turn through the `ToolRunner` the engine
//! carries, gpg writing to a staging name in the target folder
//! (`.diro-part-1a2b3c4d.gpg`) that is renamed to the real name last,
//! with the rename that never replaces; a name taken meanwhile is
//! numbered as copies are. The original is always kept, and each file
//! made goes in the undo record.
//!
//! The options travel in [`GpgOptions`], a typed side struct, never in
//! `JobRequest`, which is serialized. A passphrase is asked through the
//! request's [`SecretSource`] (the app's masked prompt), held for this
//! job only (one per key, so ten files for one key ask once even when
//! gpg's agent forgets between runs, as it does on Windows, ES3), given
//! to gpg on standard input, and dropped with the job.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use diroxide_core::job::ErrorPolicy;
use diroxide_core::{
    Entry, EntryKind, JobId, JobKind, JobOrigin, JobState, Place, Secret, SecretKind, SecretRequest,
};
use diroxide_vfs::names::numbered_name;
use diroxide_vfs::{
    NoSecrets, OpContext, RunLimits, SecretSource, ToolInput, ToolOutput, ToolRequest,
    ToolRunError, VfsError,
};

use super::gpg::{self, GPG, Outcome, Pinentry};
use super::lines;
use crate::convert::staging::{staging_name, token};
use crate::engine::{Engine, JobControl, JobReport, JobSink};
use crate::progress::JobProgress;
use crate::undo::LeftAs;

/// How long one gpg run may take: a large file, a slow disk.
pub const RUN_LIMIT: Duration = Duration::from_secs(6 * 3600);

/// How much gpg may say on its two streams: its status lines only, since
/// the data goes to `--output`.
pub const OUTPUT_CAP: usize = 1024 * 1024;

/// How many passphrases are tried for one key or file.
pub const TRIES: u32 = SecretRequest::ATTEMPTS;

/// What a gpg job does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpgAction {
    /// Encrypt to these recipients: key ids, fingerprints or addresses
    /// the owner's keyring knows.
    Encrypt {
        /// The recipients.
        recipients: Vec<String>,
    },
    /// Encrypt with a passphrase (AES-256), asked twice.
    EncryptWithPassphrase,
    /// Decrypt.
    Decrypt,
    /// Make a detached signature with this key, or the default one.
    Sign {
        /// The key: an id, a fingerprint or an address; `None` for gpg's
        /// default key.
        key: Option<String>,
    },
}

/// A gpg job's options: the typed side struct beside the `JobRequest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpgOptions {
    /// What to do.
    pub action: GpgAction,
    /// ASCII armor: `.asc` instead of `.gpg` or `.sig`.
    pub armor: bool,
}

impl GpgOptions {
    /// The job's kind: Encrypt, Decrypt or Sign.
    pub fn kind(&self) -> JobKind {
        match self.action {
            GpgAction::Encrypt { .. } | GpgAction::EncryptWithPassphrase => JobKind::Encrypt,
            GpgAction::Decrypt => JobKind::Decrypt,
            GpgAction::Sign { .. } => JobKind::Sign,
        }
    }

    /// The name a source's output gets, before any number.
    pub fn output_name(&self, name: &str) -> String {
        match (&self.action, self.armor) {
            (GpgAction::Decrypt, _) => decrypted_name(name),
            (GpgAction::Sign { .. }, false) => format!("{name}.sig"),
            (_, true) => format!("{name}.asc"),
            (_, false) => format!("{name}.gpg"),
        }
    }
}

/// The name of a file decrypted from `name`: `.gpg`, `.pgp` or `.asc`
/// removed (any case), else `.decrypted` added.
pub fn decrypted_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for extension in [".gpg", ".pgp", ".asc"] {
        if lower.ends_with(extension) && name.len() > extension.len() {
            let stem = &name[..name.len() - extension.len()];
            if !stem.ends_with('.') {
                return stem.to_owned();
            }
        }
    }
    format!("{name}.decrypted")
}

/// A gpg job as the app or the command line asks for it.
#[derive(Clone)]
pub struct GpgRequest {
    /// The job's id.
    pub job: JobId,
    /// The files.
    pub sources: Vec<Place>,
    /// What to do, and how.
    pub options: GpgOptions,
    /// Who asked.
    pub origin: JobOrigin,
    /// What to do when a file fails: continue and list by default.
    pub errors: ErrorPolicy,
    /// Where a passphrase is asked: the app's masked prompt. None by
    /// default, which turns every question into "no passphrase was
    /// given".
    pub secrets: Arc<dyn SecretSource>,
}

impl fmt::Debug for GpgRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpgRequest")
            .field("job", &self.job)
            .field("sources", &self.sources)
            .field("options", &self.options)
            .field("origin", &self.origin)
            .field("errors", &self.errors)
            .finish_non_exhaustive()
    }
}

impl GpgRequest {
    /// A request with the defaults: from the user, continue and list, no
    /// secrets.
    pub fn new(job: JobId, sources: Vec<Place>, options: GpgOptions) -> Self {
        Self {
            job,
            sources,
            options,
            origin: JobOrigin::User,
            errors: ErrorPolicy::ContinueAndList,
            secrets: Arc::new(NoSecrets),
        }
    }

    /// The same request asking passphrases here.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretSource>) -> Self {
        self.secrets = secrets;
        self
    }

    /// The same request from this origin.
    pub fn from(mut self, origin: JobOrigin) -> Self {
        self.origin = origin;
        self
    }

    /// The same request with this error policy.
    pub fn on_errors(mut self, errors: ErrorPolicy) -> Self {
        self.errors = errors;
        self
    }
}

/// Why a gpg job could not be planned. Each reads after "Could not
/// VERB: ", except [`GpgError::Missing`], which is a sentence of its own.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GpgError {
    /// gpg is not installed.
    #[error("{}", lines::GPG_NOT_FOUND)]
    Missing,
    /// Nothing was selected.
    #[error("nothing is selected")]
    NothingSelected,
    /// No recipient was given.
    #[error("no recipient was given")]
    NoRecipient,
    /// A recipient's text cannot be given to gpg.
    #[error("{0} is not a key gpg can look up")]
    BadRecipient(String),
    /// The plan was cancelled.
    #[error("it was cancelled")]
    Cancelled,
}

/// One file the plan works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpgItem {
    /// The file.
    pub source: Place,
    /// What it makes, with the name the plan chose.
    pub output: Place,
    /// Its size, for progress.
    pub size: u64,
}

/// A gpg job's plan.
#[derive(Clone)]
pub struct GpgPlan {
    /// The job's id.
    pub job: JobId,
    /// What it does.
    pub options: GpgOptions,
    /// The files and their outputs.
    pub items: Vec<GpgItem>,
    /// Files refused, each with its reason.
    pub refused: Vec<(Place, String)>,
    /// Who asked.
    pub origin: JobOrigin,
    /// The error policy.
    pub errors: ErrorPolicy,
    /// Where a passphrase is asked.
    pub secrets: Arc<dyn SecretSource>,
}

impl fmt::Debug for GpgPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpgPlan")
            .field("job", &self.job)
            .field("options", &self.options)
            .field("items", &self.items)
            .field("refused", &self.refused)
            .finish_non_exhaustive()
    }
}

/// A place's name as said.
fn name_of(place: &Place) -> String {
    place.name().unwrap_or_default().to_owned()
}

/// A key or address's ending for a line: "4E7B" for a hexadecimal id,
/// else the text itself, cleaned.
fn key_words(key: &str) -> String {
    super::status::key_ending(key).unwrap_or_else(|| lines::clean_text(key))
}

impl GpgPlan {
    /// The job's kind.
    pub fn kind(&self) -> JobKind {
        self.options.kind()
    }

    /// The plan's line: "Encrypt 3 files for 1 recipient".
    pub fn summary(&self) -> String {
        let n = self.items.len();
        match &self.options.action {
            GpgAction::Encrypt { recipients } => lines::plan_for_recipients(n, recipients.len()),
            GpgAction::EncryptWithPassphrase => lines::plan_with_passphrase(n),
            GpgAction::Decrypt => lines::plan_decrypt(n),
            GpgAction::Sign { key } => {
                lines::plan_sign(n, key.as_deref().map(key_words).as_deref())
            }
        }
    }

    /// The question before it starts, which starts on No: the summary and
    /// "y or n".
    pub fn question(&self) -> String {
        format!("{}? y or n", self.summary())
    }

    /// The plan in lines: the summary, each refusal, then each file and
    /// what it makes ("report.pdf to report.pdf.gpg").
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![self.summary()];
        for (place, why) in &self.refused {
            out.push(format!("Refused: {}, {why}", lines::said(&name_of(place))));
        }
        for item in &self.items {
            out.push(format!(
                "{} to {}",
                lines::said(&name_of(&item.source)),
                lines::said(&name_of(&item.output))
            ));
        }
        out
    }
}

/// What a gpg job did, with the lines to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpgReport {
    /// The engine's report: done (the files made), skipped, failed, the
    /// undo record naming every file made.
    pub job: JobReport,
    /// Each source and what it made.
    pub made: Vec<(Place, Place)>,
    /// What the job does.
    pub action: GpgAction,
}

impl GpgReport {
    /// The report in lines: the summary first, each failure with its
    /// cause, each file made, and after a decryption that it proves
    /// nothing of who made the file.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![self.job.summary()];
        let verb = match self.action {
            GpgAction::Decrypt => "decrypt",
            GpgAction::Sign { .. } => "sign",
            _ => "encrypt",
        };
        for (place, cause) in &self.job.failed {
            out.push(format!(
                "Could not {verb} {}: {cause}",
                lines::said(&name_of(place))
            ));
        }
        for (place, why) in &self.job.skipped {
            out.push(format!("Skipped {}: {why}", lines::said(&name_of(place))));
        }
        for (from, to) in &self.made {
            let (from, to) = (name_of(from), name_of(to));
            out.push(match self.action {
                GpgAction::Decrypt => lines::decrypted(&from, &to),
                GpgAction::Sign { .. } => lines::signed(&from, &to),
                _ => lines::encrypted(&from, &to),
            });
        }
        if matches!(self.action, GpgAction::Decrypt) && !self.made.is_empty() {
            out.push(lines::NOT_PROOF.to_owned());
        }
        out
    }
}

/// How one file ended.
enum One {
    Made(Place, Box<Entry>),
    Failed(String),
    /// Every file after this one fails the same way: a recipient gpg will
    /// not use.
    Halt(String),
    Skipped(String),
    Cancelled,
}

/// The passphrases the job holds, by key ending or by file, for this job
/// only.
#[derive(Default)]
struct Held {
    by_key: HashMap<String, Secret>,
    new_secret: Option<Secret>,
}

impl Engine {
    /// Plans a gpg job: gpg found, the recipients usable, each source a
    /// file on this computer, each output a free name beside it.
    pub fn plan_gpg(
        &self,
        request: &GpgRequest,
        control: &JobControl,
    ) -> Result<GpgPlan, GpgError> {
        if request.sources.is_empty() {
            return Err(GpgError::NothingSelected);
        }
        if !self.tools().has(GPG) {
            return Err(GpgError::Missing);
        }
        if let GpgAction::Encrypt { recipients } = &request.options.action {
            if recipients.is_empty() {
                return Err(GpgError::NoRecipient);
            }
            if let Some(bad) = recipients.iter().find(|r| !gpg::recipient_ok(r)) {
                return Err(GpgError::BadRecipient(lines::clean_text(bad)));
            }
        }
        if let GpgAction::Sign { key: Some(key) } = &request.options.action
            && !gpg::recipient_ok(key)
        {
            return Err(GpgError::BadRecipient(lines::clean_text(key)));
        }
        let ctx = self.context(&control.cancel);
        let mut items = Vec::new();
        let mut refused = Vec::new();
        let mut planned: BTreeSet<String> = BTreeSet::new();
        for source in &request.sources {
            if control.cancel.is_cancelled() {
                return Err(GpgError::Cancelled);
            }
            match self.gpg_plan_one(&request.options, source, &ctx, &mut planned) {
                Ok(item) => items.push(item),
                Err(why) => refused.push((source.clone(), why)),
            }
        }
        Ok(GpgPlan {
            job: request.job,
            options: request.options.clone(),
            items,
            refused,
            origin: request.origin.clone(),
            errors: request.errors,
            secrets: Arc::clone(&request.secrets),
        })
    }

    fn gpg_plan_one(
        &self,
        options: &GpgOptions,
        source: &Place,
        ctx: &OpContext,
        planned: &mut BTreeSet<String>,
    ) -> Result<GpgItem, String> {
        if source.local_path().is_none() {
            return Err("it is not on this computer".to_owned());
        }
        let provider = self.registry().resolve(source).map_err(|e| e.to_string())?;
        let entry = provider.stat(source, ctx).map_err(|e| e.to_string())?;
        if entry.kind != EntryKind::File {
            return Err("it is not a file".to_owned());
        }
        let folder = source
            .parent()
            .ok_or_else(|| "it has no folder".to_owned())?;
        let wanted = options.output_name(&name_of(source));
        let taken = |name: &str| {
            planned.contains(&name.to_lowercase())
                || folder
                    .join(name)
                    .ok()
                    .is_none_or(|place| provider.stat(&place, ctx).is_ok())
        };
        let name = free_name(&wanted, taken).ok_or_else(|| "no free name is left".to_owned())?;
        planned.insert(name.to_lowercase());
        let output = folder.join(&name).map_err(|e| e.to_string())?;
        Ok(GpgItem {
            source: source.clone(),
            output,
            size: entry.size.unwrap_or(0),
        })
    }

    /// Runs a gpg plan, one file at a time, until its end or the cancel.
    /// Blocking: the app runs it on a worker thread.
    pub fn run_gpg(&self, plan: &GpgPlan, control: &JobControl, sink: &dyn JobSink) -> GpgReport {
        let kind = plan.kind();
        let mut report = JobReport::new(plan.job, kind);
        report.undo.origin = plan.origin.clone();
        report.skipped = plan
            .refused
            .iter()
            .map(|(p, why)| (p.clone(), why.clone()))
            .collect();
        let bytes: u64 = plan.items.iter().map(|i| i.size).sum();
        let mut progress = JobProgress::start(kind, plan.items.len() as u64, Some(bytes));
        progress.state = JobState::Running;
        sink.progress(&progress);
        let mut made = Vec::new();
        let mut held = Held::default();
        let mut halted: Option<String> = None;
        for item in &plan.items {
            if control.cancel.is_cancelled() {
                report.cancelled = true;
                break;
            }
            while control.is_paused() && !control.cancel.is_cancelled() {
                thread::sleep(Duration::from_millis(50));
            }
            if let Some(why) = &halted {
                report.skipped.push((item.source.clone(), why.clone()));
                continue;
            }
            progress.current = Some(name_of(&item.source));
            sink.progress(&progress);
            match self.gpg_one(plan, item, control, sink, &mut progress, &mut held) {
                One::Made(output, entry) => {
                    report.done.push(output.clone());
                    report.undo.created.push(output.clone());
                    report.undo.left.push(LeftAs::of(&output, &entry));
                    made.push((item.source.clone(), output));
                }
                One::Failed(cause) => {
                    report.failed.push((item.source.clone(), cause));
                    if plan.errors == ErrorPolicy::StopAtFirst {
                        report.stopped = true;
                        halted = Some("the job stopped at the first failure".to_owned());
                    }
                }
                One::Halt(cause) => {
                    report.failed.push((item.source.clone(), cause.clone()));
                    halted = Some(cause);
                }
                One::Skipped(why) => report.skipped.push((item.source.clone(), why)),
                One::Cancelled => {
                    report.cancelled = true;
                    break;
                }
            }
            progress.items_done += 1;
            progress.bytes_done = progress.bytes_done.saturating_add(item.size);
            progress.state = JobState::Running;
            sink.progress(&progress);
        }
        progress.state = JobState::Finished;
        progress.current = None;
        sink.progress(&progress);
        GpgReport {
            job: report,
            made,
            action: plan.options.action.clone(),
        }
    }

    /// Asks a passphrase through the plan's source, saying the job waits.
    fn gpg_ask(
        &self,
        plan: &GpgPlan,
        request: &SecretRequest,
        control: &JobControl,
        sink: &dyn JobSink,
        progress: &mut JobProgress,
    ) -> Option<Secret> {
        progress.state = JobState::WaitingForAnswer;
        sink.progress(progress);
        let answer = plan.secrets.ask(request, &control.cancel);
        progress.state = JobState::Running;
        sink.progress(progress);
        answer.filter(|s| !s.is_empty())
    }

    /// One run of gpg with these arguments and this input.
    fn gpg_run_once(
        &self,
        args: Vec<std::ffi::OsString>,
        input: ToolInput,
        folder: &Path,
        control: &JobControl,
    ) -> Result<ToolOutput, ToolRunError> {
        let request = ToolRequest::new(GPG, args)
            .with_limits(RunLimits::new(RUN_LIMIT, OUTPUT_CAP))
            .with_input(input)
            .in_folder(folder)
            .with_env(gpg::GPG_LOCALE.0, gpg::GPG_LOCALE.1);
        self.tools().run(&request, &control.cancel)
    }

    #[allow(clippy::too_many_lines)]
    fn gpg_one(
        &self,
        plan: &GpgPlan,
        item: &GpgItem,
        control: &JobControl,
        sink: &dyn JobSink,
        progress: &mut JobProgress,
        held: &mut Held,
    ) -> One {
        let (Some(input), Some(output_path)) = (
            item.source.local_path().map(PathBuf::from),
            item.output.local_path().map(PathBuf::from),
        ) else {
            return One::Failed("it is not on this computer".to_owned());
        };
        let Some(folder) = output_path.parent().map(Path::to_path_buf) else {
            return One::Failed("its output has no folder".to_owned());
        };
        let extension = output_path
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .unwrap_or_else(|| "out".to_owned());
        let staging = folder.join(staging_name(token(), &extension));
        let quiet = OpContext::new();
        let discard = || {
            if staging.exists()
                && let Ok(place) = Place::parse(&staging.to_string_lossy())
                && let Ok(provider) = self.registry().resolve(&place)
            {
                let _ = provider.remove(&place, &quiet);
            }
        };
        let name = name_of(&item.source);
        let armor = plan.options.armor;
        let read = |action: &GpgAction, code, out: &[u8], err: &[u8]| match action {
            GpgAction::Decrypt => gpg::read_decrypt(code, out, err),
            GpgAction::Sign { .. } => gpg::read_sign(code, out, err),
            _ => gpg::read_encrypt(code, out, err),
        };
        let build = |mode: Pinentry| match &plan.options.action {
            GpgAction::Encrypt { recipients } => {
                gpg::encrypt_args(recipients, armor, &staging, &input)
            }
            GpgAction::EncryptWithPassphrase => gpg::symmetric_args(armor, &staging, &input),
            GpgAction::Decrypt => gpg::decrypt_args(mode, &staging, &input),
            GpgAction::Sign { key } => {
                gpg::sign_args(mode, key.as_deref(), armor, &staging, &input)
            }
        };
        // A passphrase encryption asks its new passphrase once a job, typed
        // twice by the masked prompt.
        let mut first_input = ToolInput::Closed;
        if plan.options.action == GpgAction::EncryptWithPassphrase {
            if held.new_secret.is_none() {
                let request = SecretRequest {
                    new_secret: true,
                    ..SecretRequest::new(SecretKind::FilePassphrase, name.clone())
                };
                held.new_secret = self.gpg_ask(plan, &request, control, sink, progress);
            }
            match &held.new_secret {
                Some(secret) => first_input = gpg::passphrase_input(secret),
                None => return One::Skipped(lines::NO_PASSPHRASE.to_owned()),
            }
        }
        let first_mode = match plan.options.action {
            GpgAction::EncryptWithPassphrase => Pinentry::Loopback,
            _ => Pinentry::Error,
        };
        let mut outcome = match self.gpg_run_once(build(first_mode), first_input, &folder, control)
        {
            Ok(o) => read(&plan.options.action, o.status, &o.stdout, &o.stderr),
            Err(ToolRunError::Cancelled) => {
                discard();
                return One::Cancelled;
            }
            Err(ToolRunError::NotInstalled { .. }) => {
                return One::Halt(lines::GPG_NOT_FOUND.to_owned());
            }
            Err(error) => {
                discard();
                return One::Failed(error.to_string());
            }
        };
        // The second step: a passphrase through the masked prompt, then
        // loopback; the one held for the key first.
        if let Outcome::NeedsPassphrase { key, symmetric } = outcome.clone() {
            discard();
            let (slot, base) = match (&key, symmetric) {
                (_, true) => (
                    "a file's passphrase".to_owned(),
                    SecretRequest::new(SecretKind::FilePassphrase, name.clone()),
                ),
                // A key's passphrase is a file passphrase whose subject
                // is the key: the core's title keeps "key ending 4E7B"
                // whole in 40 cells (lane AE's request 2), and reads
                // "Passphrase for key ending 4E7B", as `lines` says it.
                (Some(key), false) => (
                    key.clone(),
                    SecretRequest::new(SecretKind::FilePassphrase, lines::key_subject(Some(key))),
                ),
                (None, false) => (
                    "the default key".to_owned(),
                    SecretRequest::new(SecretKind::FilePassphrase, lines::key_subject(None)),
                ),
            };
            let mut try_held = held.by_key.get(&slot).cloned();
            let mut tries = 0u32;
            outcome = Outcome::WrongPassphrase;
            loop {
                let secret = match try_held.take() {
                    Some(secret) => secret,
                    None => {
                        if tries == TRIES {
                            break;
                        }
                        tries += 1;
                        let request = SecretRequest {
                            attempt: tries,
                            ..base.clone()
                        };
                        match self.gpg_ask(plan, &request, control, sink, progress) {
                            Some(secret) => secret,
                            None if control.cancel.is_cancelled() => return One::Cancelled,
                            None => return One::Skipped(lines::NO_PASSPHRASE.to_owned()),
                        }
                    }
                };
                let ran = self.gpg_run_once(
                    build(Pinentry::Loopback),
                    gpg::passphrase_input(&secret),
                    &folder,
                    control,
                );
                outcome = match ran {
                    Ok(o) => read(&plan.options.action, o.status, &o.stdout, &o.stderr),
                    Err(ToolRunError::Cancelled) => {
                        discard();
                        return One::Cancelled;
                    }
                    Err(error) => {
                        discard();
                        return One::Failed(error.to_string());
                    }
                };
                match &outcome {
                    Outcome::WrongPassphrase => {
                        discard();
                        held.by_key.remove(&slot);
                    }
                    Outcome::Done => {
                        held.by_key.insert(slot.clone(), secret);
                        break;
                    }
                    _ => break,
                }
            }
            if outcome == Outcome::WrongPassphrase {
                discard();
                return One::Failed(format!("{} {TRIES} times", lines::WRONG_PASSPHRASE));
            }
        }
        match outcome {
            Outcome::Done => self.gpg_commit(&staging, item, &folder, &discard),
            Outcome::NotCertified => {
                discard();
                One::Halt(lines::NOT_CERTIFIED.to_owned())
            }
            Outcome::RecipientNotFound(who) => {
                discard();
                One::Halt(lines::recipient_not_found(&who))
            }
            Outcome::NoSecretKey { key } => {
                discard();
                One::Failed(lines::no_secret_key(key.as_deref()))
            }
            Outcome::NotOpenPgp => {
                discard();
                One::Failed(lines::NOT_OPENPGP.to_owned())
            }
            Outcome::WrongPassphrase => {
                discard();
                One::Failed(lines::WRONG_PASSPHRASE.to_owned())
            }
            Outcome::NeedsPassphrase { .. } => {
                discard();
                One::Skipped(lines::NO_PASSPHRASE.to_owned())
            }
            Outcome::Failed(words) => {
                discard();
                One::Failed(words)
            }
        }
    }

    /// Renames the staging file to the output's name, never over a file:
    /// a name taken since the plan gets the next number.
    fn gpg_commit(&self, staging: &Path, item: &GpgItem, folder: &Path, discard: &dyn Fn()) -> One {
        let size = std::fs::metadata(staging).map_or(0, |m| m.len());
        if size == 0 {
            discard();
            return One::Failed(lines::MADE_NOTHING.to_owned());
        }
        let ctx = OpContext::new();
        let Ok(from) = Place::parse(&staging.to_string_lossy()) else {
            discard();
            return One::Failed("its staging name cannot be read".to_owned());
        };
        let provider = match self.registry().resolve(&from) {
            Ok(provider) => provider,
            Err(error) => {
                discard();
                return One::Failed(error.to_string());
            }
        };
        let wanted = name_of(&item.output);
        let Ok(folder_place) = Place::parse(&folder.to_string_lossy()) else {
            discard();
            return One::Failed("its folder cannot be read".to_owned());
        };
        let mut target = item.output.clone();
        for n in 1..1000u32 {
            match provider.rename(&from, &target, &ctx) {
                Ok(()) => {
                    let entry = provider
                        .stat(&target, &ctx)
                        .unwrap_or_else(|_| Entry::new(name_of(&target), EntryKind::File));
                    return One::Made(target, Box::new(entry));
                }
                Err(VfsError::AlreadyExists) => {
                    let Ok(next) = folder_place.join(&numbered_name(&wanted, n + 1)) else {
                        break;
                    };
                    target = next;
                }
                Err(error) => {
                    discard();
                    return One::Failed(error.to_string());
                }
            }
        }
        discard();
        One::Failed("no free name is left".to_owned())
    }
}

/// The first free name: `name` itself, then `name (2)` and on, the
/// extension kept after the number.
fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    if !taken(name) {
        return Some(name.to_owned());
    }
    (2..1000)
        .map(|n| numbered_name(name, n))
        .find(|candidate| !taken(candidate))
}
