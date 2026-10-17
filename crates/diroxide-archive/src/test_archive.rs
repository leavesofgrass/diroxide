//! Test archive (plan commit 31, research/archives.md 8.3): every member
//! read to its end through the limiter chain, every checksum and size it
//! claims held to, nothing written; the verdict by name: "Archive is
//! sound: 10 members." or "2 members damaged: first is week1/a.txt". The
//! same reading checks a new archive after packing ([`check_written`]):
//! "Checked 300 of 300 entries, all good."

use std::collections::HashSet;
use std::io::{self, Read};

use diroxide_core::format::count;
use diroxide_core::{JobKind, Message, Place, Secret};
use diroxide_ops::JobReport;
use diroxide_vfs::{OpContext, VfsError};

use crate::error::ArchiveError;
use crate::guard::{Allowances, Budget, LimitHit, Question, from_io, limit_of};
use crate::index::{Index, RecordKind};
use crate::job::{Declined, JobEnv, Progress};
use crate::provider::{ArchiveProvider, Opened};
use crate::secrets::JobSecrets;

/// What Test archive found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestOutcome {
    /// The archive's name.
    pub archive: String,
    /// The members in the archive, folders counted.
    pub members: u64,
    /// The members read to their end and found sound.
    pub sound: u64,
    /// The members found damaged, each with its path and the cause, in
    /// archive order.
    pub damaged: Vec<(String, String)>,
    /// The members not checked, with the reason: locked with no right
    /// password, or a limit's question answered No.
    pub unchecked: Vec<(String, String)>,
    /// The archive could not be opened or read at all: the cause.
    pub failed: Option<String>,
    /// Whether the test was cancelled.
    pub cancelled: bool,
    /// The engine's report: what was checked, skipped and failed.
    pub report: JobReport,
}

impl TestOutcome {
    /// Whether every member was read and found sound.
    pub fn is_sound(&self) -> bool {
        self.damaged.is_empty()
            && self.unchecked.is_empty()
            && self.failed.is_none()
            && !self.cancelled
    }

    /// The verdict, the meaning first: "Archive is sound: 10 members.",
    /// "2 members damaged: first is week1/a.txt", "Test cancelled: 4 of
    /// 10 members checked.", "Could not test a.7z: a password is needed."
    pub fn sentence(&self) -> Message {
        if let Some(cause) = &self.failed {
            return Message::error(format!("Could not test {}: {cause}.", self.archive));
        }
        if !self.damaged.is_empty() {
            let first = diroxide_core::names::display_name(&self.damaged[0].0).into_owned();
            return Message::error(format!(
                "{} damaged: first is {first}",
                count(self.damaged.len() as u64, "member", "members")
            ));
        }
        if self.cancelled {
            return Message::warning(format!(
                "Test cancelled: {} of {} checked.",
                self.sound,
                count(self.members, "member", "members")
            ));
        }
        if !self.unchecked.is_empty() {
            return Message::warning(format!(
                "Not checked: {} of {}.",
                self.unchecked.len(),
                count(self.members, "member", "members")
            ));
        }
        Message::done(format!(
            "Archive is sound: {}.",
            count(self.members, "member", "members")
        ))
    }

    /// The verdict, then a line for each damaged member and each one not
    /// checked: "week1/a.txt: it is damaged: the checksum of a.txt
    /// failed".
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.sentence().text];
        for (path, cause) in self.damaged.iter().chain(&self.unchecked) {
            lines.push(format!(
                "{}: {cause}",
                diroxide_core::names::display_name(path)
            ));
        }
        lines
    }
}

/// Why a pass stopped.
enum Stop {
    Ask(Question),
    WrongPassword,
}

/// Tests an archive: see the module documentation. `password` is the
/// first try when one is known; otherwise the first locked member asks
/// through the environment's secrets, three tries at most.
pub fn test_archive(archive: &Place, password: Option<Secret>, env: &JobEnv<'_>) -> TestOutcome {
    let secrets = JobSecrets::knowing(env.secrets.clone(), password);
    let ctx = env.context(secrets.clone());
    let name = archive.name().unwrap_or("archive").to_owned();
    let mut outcome = TestOutcome {
        archive: name,
        members: 0,
        sound: 0,
        damaged: Vec::new(),
        unchecked: Vec::new(),
        failed: None,
        cancelled: false,
        report: JobReport::new(env.job, JobKind::TestArchive),
    };
    let mut allow = Allowances {
        member_size: true,
        ..Allowances::default()
    };
    let opened = loop {
        match secrets.with_tries(&ctx, |ctx| env.archives.open_archive(archive, ctx, allow)) {
            Ok(opened) => break opened,
            Err(ArchiveError::Ask(question)) if env.questions.ask(&question) => {
                allow = allow.with(question.kind);
            }
            Err(ArchiveError::Vfs(VfsError::Cancelled)) => {
                outcome.cancelled = true;
                outcome.report.cancelled = true;
                return outcome;
            }
            Err(error) => {
                let cause = match &error {
                    ArchiveError::Ask(question) => format!("you said No to: {}", question.text),
                    other => other.to_string(),
                };
                outcome.report.failed.push((archive.clone(), cause.clone()));
                outcome.failed = Some(cause);
                return outcome;
            }
        }
    };
    let mut progress = Progress::start(
        env,
        JobKind::TestArchive,
        opened.index().len() as u64,
        opened.index().summary().total_size,
    );
    run_checks(
        env.archives,
        &opened,
        &ctx,
        &secrets,
        allow,
        env,
        &mut progress,
        &mut outcome,
    );
    progress.finish();
    outcome.cancelled |= env.control.cancel.is_cancelled();
    outcome.report.cancelled = outcome.cancelled;
    outcome
}

/// Reads every member of an opened archive, keeping the verdicts.
#[allow(clippy::too_many_arguments)]
fn run_checks(
    archives: &ArchiveProvider,
    opened: &Opened,
    ctx: &OpContext,
    secrets: &std::sync::Arc<JobSecrets>,
    mut allow: Allowances,
    env: &JobEnv<'_>,
    progress: &mut Progress<'_>,
    outcome: &mut TestOutcome,
) {
    let index = opened.index();
    outcome.members = index.len() as u64;
    let root = opened.container().clone().into_archive();
    let mut settled: HashSet<u32> = HashSet::new();
    let mut declined = Declined::default();
    let budget = Budget::new(&archives.limits());
    for _ in 0..index.len() + 8 {
        let pending: HashSet<u32> = (0..index.len() as u32)
            .filter(|id| !settled.contains(id))
            .collect();
        if pending.is_empty() {
            break;
        }
        let mut stop = None;
        let mut current: Option<u32> = None;
        let result = archives.visit_some(
            opened,
            ctx,
            allow,
            Some(budget.clone()),
            &|id| pending.contains(&id),
            &mut |index, id, reader| {
                current = Some(id);
                if !progress.hold(env.control) {
                    return Err(VfsError::Cancelled);
                }
                progress.current(index.name(id));
                match check_one(index, id, reader, progress) {
                    Ok(()) => {
                        settled.insert(id);
                        outcome.sound += 1;
                        progress.item_done();
                        Ok(true)
                    }
                    Err(error) => {
                        if let Some(LimitHit::Ask(question)) = limit_of(&error) {
                            stop = Some(Stop::Ask(question.clone()));
                            return Ok(false);
                        }
                        match from_io(error) {
                            VfsError::Cancelled => Err(VfsError::Cancelled),
                            VfsError::WrongPassword => {
                                stop = Some(Stop::WrongPassword);
                                Ok(false)
                            }
                            other => {
                                settled.insert(id);
                                outcome.damaged.push((index.path(id), other.to_string()));
                                progress.item_done();
                                Ok(true)
                            }
                        }
                    }
                }
            },
        );
        match (result, stop) {
            (Ok(()), None) => break,
            (Ok(()), Some(Stop::Ask(question))) => {
                if !declined.contains(question.kind) && env.questions.ask(&question) {
                    allow = allow.with(question.kind);
                } else {
                    declined.insert(question.kind);
                    if let Some(id) = current {
                        settled.insert(id);
                        outcome
                            .unchecked
                            .push((index.path(id), format!("you said No to: {}", question.text)));
                    }
                }
            }
            (Ok(()), Some(Stop::WrongPassword))
            | (Err(ArchiveError::Vfs(VfsError::WrongPassword)), _) => {
                if !secrets.reject() {
                    leave_locked(index, &mut settled, outcome, secrets);
                }
            }
            (Err(ArchiveError::Vfs(VfsError::PasswordRequired)), _) => {
                leave_locked(index, &mut settled, outcome, secrets);
                if opened.format() == crate::ArchiveFormat::SevenZ {
                    break;
                }
            }
            (Err(ArchiveError::Vfs(VfsError::Cancelled)), _) => {
                outcome.cancelled = true;
                break;
            }
            (Err(ArchiveError::Ask(question)), _) => {
                if env.questions.ask(&question) {
                    allow = allow.with(question.kind);
                } else {
                    outcome.failed = Some(format!("you said No to: {}", question.text));
                    break;
                }
            }
            (Err(error), _) => {
                // The archive cannot be read past here: the member at work
                // and what follows are damaged.
                let cause = error.to_string();
                for id in 0..index.len() as u32 {
                    if !settled.contains(&id)
                        && index.record(id).is_some_and(|r| r.kind == RecordKind::File)
                    {
                        outcome.damaged.push((index.path(id), cause.clone()));
                    }
                }
                break;
            }
        }
    }
    for (path, cause) in &outcome.damaged {
        outcome
            .report
            .failed
            .push((member_place(&root, path), cause.clone()));
    }
    for (path, cause) in &outcome.unchecked {
        outcome
            .report
            .skipped
            .push((member_place(&root, path), cause.clone()));
    }
    for id in 0..index.len() as u32 {
        if settled.contains(&id)
            && !outcome.damaged.iter().any(|(p, _)| *p == index.path(id))
            && !outcome.unchecked.iter().any(|(p, _)| *p == index.path(id))
        {
            outcome
                .report
                .done
                .push(member_place(&root, &index.path(id)));
        }
    }
}

/// Every locked member not yet settled is left unchecked.
fn leave_locked(
    index: &Index,
    settled: &mut HashSet<u32>,
    outcome: &mut TestOutcome,
    secrets: &JobSecrets,
) {
    let mut locked = Vec::new();
    for id in 0..index.len() as u32 {
        if !settled.contains(&id)
            && index
                .record(id)
                .is_some_and(|r| r.flags.contains(crate::index::RecordFlags::ENCRYPTED))
        {
            settled.insert(id);
            locked.push(id);
        }
    }
    let reason = secrets
        .gave_up_line(&outcome.archive, locked.len() as u64)
        .text;
    for id in locked {
        outcome.unchecked.push((index.path(id), reason.clone()));
    }
}

/// A member's place for the report; the archive's root when a place
/// cannot hold its name.
fn member_place(root: &Place, path: &str) -> Place {
    let mut place = root.clone();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        match place.join(part) {
            Ok(next) => place = next,
            Err(_) => return root.clone(),
        }
    }
    place
}

/// Reads one member to its end; the limiter chain says if its size or
/// checksum is wrong.
fn check_one(
    index: &Index,
    id: u32,
    reader: &mut dyn Read,
    progress: &mut Progress<'_>,
) -> io::Result<()> {
    if index.record(id).is_none_or(|r| r.kind != RecordKind::File) {
        return Ok(());
    }
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => progress.bytes(n as u64),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// What a packer wrote for one member, for the check after packing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The member's path in the archive.
    pub path: String,
    /// Its size.
    pub size: u64,
    /// The CRC-32 of its bytes.
    pub crc32: u32,
}

/// The check after packing: the new archive opened from its bytes, every
/// member read to its end, and the count, the sizes and the checksums held
/// to what was written. Gives the line "Checked 300 of 300 entries, all
/// good." or the first problem in words.
pub fn check_written(
    archives: &ArchiveProvider,
    opened: &Opened,
    written: &[Written],
    ctx: &OpContext,
) -> Result<String, String> {
    let index = opened.index();
    let mut files: Vec<(String, u64, u32)> = Vec::new();
    let result = archives.visit_opened(
        opened,
        ctx,
        Allowances::all(),
        None,
        &mut |index, id, reader| {
            if index.record(id).is_none_or(|r| r.kind != RecordKind::File) {
                return Ok(true);
            }
            let mut hasher = crc32fast::Hasher::new();
            let mut size = 0u64;
            let mut chunk = vec![0u8; 64 << 10];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        hasher.update(&chunk[..n]);
                        size += n as u64;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(from_io(e)),
                }
            }
            files.push((index.path(id), size, hasher.finalize()));
            Ok(true)
        },
    );
    if let Err(error) = result {
        return Err(format!("the new archive could not be read back: {error}"));
    }
    let expected: Vec<&Written> = written.iter().collect();
    if files.len() != expected.len() {
        return Err(format!(
            "the new archive holds {} where {} were written",
            count(files.len() as u64, "file", "files"),
            expected.len()
        ));
    }
    let mut found: std::collections::HashMap<&str, (u64, u32)> = std::collections::HashMap::new();
    for (path, size, crc) in &files {
        found.insert(path.as_str(), (*size, *crc));
    }
    for item in &expected {
        match found.get(item.path.as_str()) {
            Some(&(size, crc)) if size == item.size && crc == item.crc32 => {}
            Some(_) => {
                return Err(format!(
                    "{} reads back differently in the new archive",
                    diroxide_core::names::display_name(&item.path)
                ));
            }
            None => {
                return Err(format!(
                    "{} is missing from the new archive",
                    diroxide_core::names::display_name(&item.path)
                ));
            }
        }
    }
    let entries = index.len() as u64;
    Ok(format!("Checked {entries} of {entries} entries, all good."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome() -> TestOutcome {
        TestOutcome {
            archive: "course.zip".to_owned(),
            members: 10,
            sound: 10,
            damaged: Vec::new(),
            unchecked: Vec::new(),
            failed: None,
            cancelled: false,
            report: JobReport::new(diroxide_core::JobId::new(1), JobKind::TestArchive),
        }
    }

    #[test]
    fn the_verdicts_say_the_meaning_first() {
        let sound = outcome();
        assert_eq!(sound.sentence().text, "Archive is sound: 10 members.");
        assert!(sound.is_sound());
        let damaged = TestOutcome {
            damaged: vec![
                ("week1/a.txt".to_owned(), "it is damaged".to_owned()),
                ("week2/b.txt".to_owned(), "it is damaged".to_owned()),
            ],
            ..outcome()
        };
        assert_eq!(
            damaged.sentence().text,
            "2 members damaged: first is week1/a.txt"
        );
        assert_eq!(damaged.lines()[1], "week1/a.txt: it is damaged");
        let failed = TestOutcome {
            failed: Some("a password is needed".to_owned()),
            ..outcome()
        };
        assert_eq!(
            failed.sentence().text,
            "Could not test course.zip: a password is needed."
        );
        let cancelled = TestOutcome {
            cancelled: true,
            sound: 4,
            ..outcome()
        };
        assert_eq!(
            cancelled.sentence().text,
            "Test cancelled: 4 of 10 members checked."
        );
    }
}
