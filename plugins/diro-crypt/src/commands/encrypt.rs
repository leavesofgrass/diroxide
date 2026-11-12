//! Encrypt with a passphrase, and Encrypt for a recipient (R29 3.2).
//!
//! The passphrase is asked twice in one form's hidden fields and compared
//! here; three tries that do not fit end the command with nothing done.
//! The recipients are a pasted key, a recipients file by its path, or
//! both; a form that names neither, or a key that is not one, is shown
//! again with the reason first, three times at most. Each file becomes
//! `NAME.age` beside it, the original left where it was.

use std::path::Path;

use serde_json::Value;

use super::forms::{self, Passphrase};
use super::{Ctx, Tally, Verb};
use crate::crypt::{self, Lock, Watch};
use crate::error::CryptError;
use crate::lines;
use crate::recipients;

/// How many times a form is shown before the command gives up.
pub const TRIES: usize = 3;

/// Encrypt with a passphrase.
pub fn with_passphrase(ctx: &Ctx<'_>) -> Result<Value, CryptError> {
    let mut tally = Tally::default();
    let targets = ctx.targets(&mut tally);
    if targets.is_empty() {
        return Ok(ctx.nothing(&tally));
    }
    let mut note = None;
    let mut chosen = None;
    for _ in 0..TRIES {
        let Some(values) = ctx.host.form(forms::passphrase_form(targets.len(), note))? else {
            return Ok(ctx.escaped(&tally));
        };
        match forms::read_passphrase(values) {
            Passphrase::Fits(passphrase, armor) => {
                chosen = Some((passphrase, armor));
                break;
            }
            Passphrase::Again(why) => note = Some(why),
        }
    }
    let Some((passphrase, armor)) = chosen else {
        if let Some(why) = note {
            ctx.host.announce(why, "warning");
        }
        return Ok(ctx.escaped(&tally));
    };
    let plan = lines::plan_with_passphrase(targets.len());
    ctx.host.announce(&plan, "status");
    ctx.progress.begin(&plan);
    let lock = Lock::Passphrase(&passphrase);
    run(ctx, &targets, &lock, armor, &mut tally);
    Ok(ctx.finish(&tally, targets.len(), Verb::Encrypt))
}

/// Encrypt for a recipient.
pub fn for_recipients(ctx: &Ctx<'_>) -> Result<Value, CryptError> {
    let mut tally = Tally::default();
    let targets = ctx.targets(&mut tally);
    if targets.is_empty() {
        return Ok(ctx.nothing(&tally));
    }
    let mut note: Option<String> = None;
    let mut chosen = None;
    for _ in 0..TRIES {
        let Some(values) = ctx
            .host
            .form(forms::recipient_form(targets.len(), note.as_deref()))?
        else {
            return Ok(ctx.escaped(&tally));
        };
        let answer = forms::read_recipient(&values);
        match read_all(&answer) {
            Ok(keys) => {
                chosen = Some((keys, answer.armor));
                break;
            }
            Err(why) => note = Some(why),
        }
    }
    let Some((keys, armor)) = chosen else {
        if let Some(why) = note {
            ctx.host.announce(&why, "warning");
        }
        return Ok(ctx.escaped(&tally));
    };
    let plan = lines::plan_for_recipients(targets.len(), keys.len());
    ctx.host.announce(&plan, "status");
    ctx.progress.begin(&plan);
    let lock = Lock::Recipients(&keys);
    run(ctx, &targets, &lock, armor, &mut tally);
    Ok(ctx.finish(&tally, targets.len(), Verb::Encrypt))
}

/// The keys of an answer: the pasted ones, then the file's; a sentence
/// when none is given or one is not a key.
pub fn read_all(answer: &forms::RecipientAnswer) -> Result<Vec<age::x25519::Recipient>, String> {
    let mut keys =
        recipients::parse(&answer.key).map_err(|r| format!("{} in the pasted text.", r.0))?;
    if !answer.file.is_empty() {
        let text = read_recipients_file(Path::new(&answer.file))?;
        let more =
            recipients::parse(&text).map_err(|r| format!("{} of the recipients file.", r.0))?;
        keys.extend(more);
    }
    if keys.is_empty() {
        return Err(format!("{}.", lines::NO_RECIPIENT));
    }
    if keys.len() > recipients::MAX_RECIPIENTS {
        return Err(format!(
            "More than {} recipients.",
            recipients::MAX_RECIPIENTS
        ));
    }
    Ok(keys)
}

/// A recipients file's text: at most 1 MB, UTF-8.
fn read_recipients_file(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}.", lines::recipients_unreadable(&lower(&e.to_string()))))?
        .len();
    if size > recipients::MAX_FILE_BYTES {
        return Err(format!("{}.", lines::RECIPIENTS_TOO_LARGE));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| format!("{}.", lines::recipients_unreadable(&lower(&e.to_string()))))?;
    String::from_utf8(bytes)
        .map_err(|_| format!("{}.", lines::recipients_unreadable("it is not text")))
}

fn lower(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Encrypts each file in turn until the request is cancelled.
fn run(ctx: &Ctx<'_>, targets: &[super::Target], lock: &Lock<'_>, armor: bool, tally: &mut Tally) {
    for target in targets {
        if ctx.cancelled() {
            break;
        }
        let cancelled = || ctx.cancelled();
        let mut progress = ctx.reporter("Encrypting", &target.name);
        let mut watch = Watch {
            cancelled: &cancelled,
            progress: &mut progress,
        };
        let end = crypt::encrypt_file(&target.path, lock, armor, &ctx.host, &mut watch);
        ctx.record(tally, target, end, Verb::Encrypt);
    }
}
