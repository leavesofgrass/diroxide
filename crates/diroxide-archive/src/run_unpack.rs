//! The unpack runner (plan commit 30): one forward pass over the archive
//! in the order its data lies (a solid 7z block by block, a compressed
//! tar from its start to its end, a zip in its own order), each member
//! through the limiter chain and the provider's committing writer, so a
//! cancelled unpack never leaves a half-written file.
//!
//! What it holds to, whatever the archive says:
//!
//! - **Nothing outside the destination.** Every target was made from safe
//!   names under the destination by the plan; before a file is written,
//!   each folder on its way is checked, and a link there (one the
//!   destination already had) stops it: "Skipped x/evil.txt: a folder on
//!   the way is a link." A link in the archive followed by a member
//!   written through it cannot happen: the index makes such a name a
//!   folder, and links are made, when asked, only last and only when they
//!   stay inside.
//! - **Conflicts through the engine's question**, asked once with
//!   apply-to-rest; a replace sends the old file to the trash only after
//!   the new one is whole.
//! - **Mode bits made safe** (no set-id or sticky bit, no group or other
//!   write, execute only where the owner has it), owners never applied,
//!   devices and other special entries never made, times kept, a
//!   folder's after its contents.
//! - **Questions and passwords asked on the job's thread**: a limit's
//!   question through [`crate::ArchiveQuestions`], once per kind; a wrong password
//!   tried again up to three times; members already done are never read
//!   again.
//!
//! The report says the problems first ("Unpack finished: 118 done, 2
//! skipped.") with the lines after it: "Skipped ../x: name leaves the
//! archive.", "Renamed 3 names: unsafe characters."

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;

use diroxide_core::format::count;
use diroxide_core::{Attributes, EntryKind, JobKind, Place, Secret, Timestamp};
use diroxide_ops::{JobReport, Reconciliation};
use diroxide_vfs::{AttributeChange, LinkKind, LinkRequest, OpContext, Provider, VfsError};

use crate::error::ArchiveError;
use crate::guard::{Allowances, Budget, LimitHit, Question, limit_of};
use crate::index::{Index, RecordKind};
use crate::job::{
    Declined, JobEnv, Landing, Progress, Settler, WriteFailure, declined_reason, io_to_vfs,
    lines_of, write_file,
};
use crate::secrets::JobSecrets;
use crate::unpack::{UnpackPlan, renamed_line};

/// What an unpack did: the engine's report (with its undo record of every
/// file and folder made), and the lines to say after its summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpackOutcome {
    /// The report.
    pub report: JobReport,
    /// The summary first, then the lines: "Unpack finished: 118 done, 2
    /// skipped.", "Skipped ../x: name leaves the archive.", "Renamed 3
    /// names: unsafe characters."
    pub lines: Vec<String>,
}

/// Mode bits made safe for a file unpacked (Python's `data` filter's
/// rule): no set-id or sticky bit, no write for the group or others, and
/// execute only where the owner has it.
pub fn safe_mode(mode: u32) -> u32 {
    let mut safe = mode & 0o755;
    if mode & 0o100 == 0 {
        safe &= !0o111;
    }
    safe
}

/// Why the pass stopped before its end.
enum Stop {
    /// A limit asks, at a member.
    Ask(Question, usize),
    /// The password proved wrong, at a member.
    WrongPassword,
}

/// What the pass changes as it goes.
struct State<'a> {
    report: JobReport,
    progress: Progress<'a>,
    settler: Settler,
    done: Vec<bool>,
    skipped: HashSet<usize>,
    folders: Vec<(Place, Option<i64>)>,
    known_folders: HashSet<Place>,
    stop: Option<Stop>,
    links_left_out: u64,
}

/// Runs an unpack plan: see the module documentation. `password` is a
/// password already typed (the first try); otherwise the first locked
/// member asks through the environment's secrets. `allow` carries the
/// answers to the plan's questions. A locked 7z's names were asked for
/// when the plan was made: give the run the same [`JobSecrets`] as the
/// environment's secrets, and the job asks once.
pub fn run_unpack(
    plan: &UnpackPlan,
    password: Option<Secret>,
    allow: Allowances,
    env: &JobEnv<'_>,
) -> UnpackOutcome {
    let secrets = JobSecrets::knowing(env.secrets.clone(), password);
    let ctx = env.context(secrets.clone());
    let items = plan.members.len() as u64;
    let mut state = State {
        report: JobReport::new(env.job, JobKind::Unpack),
        progress: Progress::start(env, JobKind::Unpack, items, Some(plan.facts.bytes)),
        settler: Settler::new(plan.conflicts, env.engine.options().time_tolerance_seconds),
        done: vec![false; plan.members.len()],
        skipped: HashSet::new(),
        folders: Vec::new(),
        known_folders: HashSet::new(),
        stop: None,
        links_left_out: 0,
    };
    for left in &plan.left_out {
        state.links_left_out += u64::from(left.reason == "link not followed");
        let place = member_place(&plan.archive, &left.path);
        state.report.skipped.push((place, left.reason.clone()));
    }
    for line in &plan.refused_lines {
        state
            .report
            .skipped
            .push((plan.archive.clone().into_archive(), line.clone()));
    }
    let mut extra = Vec::new();
    if let Some(refusal) = plan.refusal() {
        state
            .report
            .failed
            .push((plan.destination.clone(), refusal.text.clone()));
        state.progress.finish();
        extra.push(refusal.text);
        return UnpackOutcome {
            lines: lines_of(&state.report, extra),
            report: state.report,
        };
    }
    let provider = match env.provider(&plan.chosen) {
        Ok(provider) => provider,
        Err(error) => {
            state
                .report
                .failed
                .push((plan.chosen.clone(), error.to_string()));
            return UnpackOutcome {
                lines: lines_of(&state.report, extra),
                report: state.report,
            };
        }
    };
    let runner = Runner {
        plan,
        env,
        ctx: &ctx,
        provider: provider.as_ref(),
        by_id: plan
            .members
            .iter()
            .enumerate()
            .map(|(n, member)| (member.id, n))
            .collect(),
    };
    if plan.made_folder.is_some()
        && let Err(reason) = runner.ensure_folder(&mut state, &plan.destination)
    {
        state.report.failed.push((plan.destination.clone(), reason));
    } else {
        runner.passes(&mut state, &secrets, allow);
    }
    // A cancel at a prompt or a question ends the job as a cancel, even
    // when what stopped it was the missing answer.
    if env.control.cancel.is_cancelled() {
        state.report.cancelled = true;
    }
    runner.finish(&mut state);
    if plan.renamed > 0 {
        extra.push(renamed_line(plan.renamed));
    }
    if state.links_left_out > 0 {
        extra.push(format!(
            "{} skipped: links are not unpacked.",
            count(state.links_left_out, "link", "links")
        ));
    }
    for left in plan
        .left_out
        .iter()
        .filter(|l| l.reason != "link not followed")
    {
        extra.push(left.line());
    }
    extra.extend(plan.refused_lines.iter().cloned());
    if secrets.gave_up() {
        let locked = plan.members.iter().filter(|m| m.encrypted).count() as u64;
        extra.push(secrets.gave_up_line(&plan.archive_name, locked).text);
    }
    let lines = lines_of(&state.report, extra);
    UnpackOutcome {
        report: state.report,
        lines,
    }
}

/// The place of a member in the archive, for the report; the archive's
/// root when a place cannot hold its name.
fn member_place(archive: &Place, path: &str) -> Place {
    let root = archive.clone().into_archive();
    let mut place = root.clone();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        match place.join(part) {
            Ok(next) => place = next,
            Err(_) => return root,
        }
    }
    place
}

/// The parts of the run that do not change.
struct Runner<'r> {
    plan: &'r UnpackPlan,
    env: &'r JobEnv<'r>,
    ctx: &'r OpContext,
    provider: &'r dyn Provider,
    by_id: HashMap<u32, usize>,
}

impl Runner<'_> {
    /// The passes: one, unless a question or a wrong password stops it;
    /// then the next goes on with what is left.
    fn passes(&self, state: &mut State<'_>, secrets: &Arc<JobSecrets>, mut allow: Allowances) {
        let budget = Budget::new(&self.env.archives.limits());
        // The whole job's total was weighed by the plan; a member's size
        // alone never stops an unpack.
        allow.member_size = true;
        let mut declined = Declined::default();
        let mut tries = 0usize;
        loop {
            tries += 1;
            if tries > self.plan.members.len() + 8 {
                break;
            }
            state.stop = None;
            let opened = match self
                .env
                .archives
                .open_archive(&self.plan.archive, self.ctx, allow)
            {
                Ok(opened) => opened,
                Err(error) => {
                    if self.settle_error(state, secrets, &mut allow, &mut declined, error) {
                        continue;
                    }
                    break;
                }
            };
            let wanted = |id: u32| {
                self.by_id
                    .get(&id)
                    .is_some_and(|&n| !state_done(&state.done, &state.skipped, n))
            };
            // The closure below takes the state; `wanted` reads a copy of
            // the marks taken now.
            let pending: HashSet<u32> = (0..opened.index().len() as u32)
                .filter(|&id| wanted(id))
                .collect();
            if pending.is_empty() {
                break;
            }
            let result = self.env.archives.visit_some(
                &opened,
                self.ctx,
                allow,
                Some(budget.clone()),
                &|id| pending.contains(&id),
                &mut |index, id, reader| self.member(state, index, id, reader),
            );
            match (result, state.stop.take()) {
                (Ok(()), None) => break,
                (Ok(()), Some(Stop::Ask(question, at))) => {
                    if declined.contains(question.kind) {
                        self.skip(state, at, declined_reason(&question));
                    } else if self.env.questions.ask(&question) {
                        allow = allow.with(question.kind);
                    } else {
                        declined.insert(question.kind);
                        self.skip(state, at, declined_reason(&question));
                    }
                }
                (Ok(()), Some(Stop::WrongPassword)) => {
                    if !secrets.reject() {
                        self.skip_locked(state);
                    }
                }
                (Err(error), _) => {
                    if !self.settle_error(state, secrets, &mut allow, &mut declined, error) {
                        break;
                    }
                }
            }
        }
    }

    /// Settles an error that stopped a pass; true when another pass should
    /// go on with what is left.
    fn settle_error(
        &self,
        state: &mut State<'_>,
        secrets: &Arc<JobSecrets>,
        allow: &mut Allowances,
        declined: &mut Declined,
        error: ArchiveError,
    ) -> bool {
        match error {
            ArchiveError::Vfs(VfsError::Cancelled) => {
                state.report.cancelled = true;
                false
            }
            ArchiveError::Vfs(VfsError::WrongPassword) => {
                if !secrets.reject() {
                    self.skip_locked(state);
                }
                self.any_left(state)
            }
            ArchiveError::Vfs(VfsError::PasswordRequired) => {
                self.skip_locked(state);
                if self.any_left(state) && self.plan.format != crate::ArchiveFormat::SevenZ {
                    return true;
                }
                // A 7z that cannot be opened or decoded without the
                // password: what is left cannot be read.
                self.fail_left(state, "it needs the password");
                false
            }
            ArchiveError::Ask(question) => {
                if !declined.contains(question.kind) && self.env.questions.ask(&question) {
                    *allow = allow.with(question.kind);
                    true
                } else {
                    declined.insert(question.kind);
                    let reason = declined_reason(&question);
                    for n in 0..self.plan.members.len() {
                        if !state_done(&state.done, &state.skipped, n) {
                            self.skip(state, n, reason.clone());
                        }
                    }
                    false
                }
            }
            ArchiveError::Vfs(error) => {
                self.fail_left(state, &error.to_string());
                false
            }
            other => {
                self.fail_left(state, &other.to_string());
                false
            }
        }
    }

    /// Whether any member is neither done nor skipped.
    fn any_left(&self, state: &State<'_>) -> bool {
        (0..self.plan.members.len()).any(|n| !state_done(&state.done, &state.skipped, n))
    }

    /// Every member not yet done fails with this cause.
    fn fail_left(&self, state: &mut State<'_>, cause: &str) {
        for (n, member) in self.plan.members.iter().enumerate() {
            if !state_done(&state.done, &state.skipped, n) {
                state.skipped.insert(n);
                state
                    .report
                    .failed
                    .push((member.target.clone(), cause.to_owned()));
            }
        }
    }

    /// Every locked member not yet done is skipped: the job gave up on the
    /// password.
    fn skip_locked(&self, state: &mut State<'_>) {
        for (n, member) in self.plan.members.iter().enumerate() {
            if member.encrypted && !state_done(&state.done, &state.skipped, n) {
                self.skip(state, n, "no right password was given".to_owned());
            }
        }
    }

    fn skip(&self, state: &mut State<'_>, n: usize, reason: String) {
        if let Some(member) = self.plan.members.get(n)
            && state.skipped.insert(n)
        {
            state.report.skipped.push((member.target.clone(), reason));
            state.progress.item_done();
        }
    }

    /// One member, as the pass reaches it.
    fn member(
        &self,
        state: &mut State<'_>,
        index: &Index,
        id: u32,
        reader: &mut dyn Read,
    ) -> Result<bool, VfsError> {
        let Some(&n) = self.by_id.get(&id) else {
            return Ok(true);
        };
        if state_done(&state.done, &state.skipped, n) {
            return Ok(true);
        }
        if !state.progress.hold(self.env.control) {
            return Err(VfsError::Cancelled);
        }
        let member = &self.plan.members[n];
        state.progress.current(&member.path);
        match member.kind {
            RecordKind::Folder => {
                match self.ensure_folder(state, &member.target) {
                    Ok(()) => {
                        state.folders.push((member.target.clone(), member.modified));
                        self.done(state, n);
                    }
                    Err(reason) => self.skip(state, n, reason),
                }
                Ok(true)
            }
            RecordKind::Link => {
                self.link(state, n);
                Ok(true)
            }
            RecordKind::Special => Ok(true),
            RecordKind::File => self.file(state, index, id, n, reader),
        }
    }

    fn done(&self, state: &mut State<'_>, n: usize) {
        state.done[n] = true;
        state.report.done.push(self.plan.members[n].target.clone());
        state.progress.item_done();
    }

    /// Writes one file member.
    fn file(
        &self,
        state: &mut State<'_>,
        index: &Index,
        id: u32,
        n: usize,
        reader: &mut dyn Read,
    ) -> Result<bool, VfsError> {
        let member = &self.plan.members[n];
        let Some(folder) = member.target.parent() else {
            self.skip(state, n, "it has no folder".to_owned());
            return Ok(true);
        };
        if let Err(reason) = self.ensure_folder(state, &folder) {
            self.skip(state, n, reason);
            return Ok(true);
        }
        let mut entry = index
            .entry(id)
            .unwrap_or_else(|| diroxide_core::Entry::new(member.path.clone(), EntryKind::File));
        entry.kind = EntryKind::File;
        let source = member_place(&self.plan.archive, &member.path);
        let landing = match state.settler.settle(
            self.provider,
            self.ctx,
            &source,
            &entry,
            &member.target,
            self.env.sink,
        ) {
            Ok(landing) => landing,
            Err(VfsError::Cancelled) => return Err(VfsError::Cancelled),
            Err(error) => {
                state.skipped.insert(n);
                state
                    .report
                    .failed
                    .push((member.target.clone(), error.to_string()));
                return Ok(true);
            }
        };
        let (target, replace) = match landing {
            Landing::Free(target) => (target, false),
            Landing::Replace(target) => (target, true),
            Landing::Skip(reason) => {
                self.skip(state, n, reason);
                return Ok(true);
            }
            Landing::Cancel => {
                state.report.cancelled = true;
                return Err(VfsError::Cancelled);
            }
        };
        let times = self.times_for(&target, member.modified, member.mode);
        let options = self.env.engine.options();
        match write_file(
            self.provider,
            &target,
            replace,
            options.replace_keeps_old_in_trash,
            reader,
            &times,
            self.ctx,
            &mut state.progress,
            self.env.control,
            options.chunk_bytes,
        ) {
            Ok(written) => {
                if written.old_in_trash {
                    state.report.undo.trashed.push(target.clone());
                }
                if let Some(warning) = written.warning {
                    state.report.warnings.push((target.clone(), warning));
                }
                state.report.undo.created.push(target.clone());
                state.done[n] = true;
                state.report.done.push(target);
                state.progress.item_done();
                Ok(true)
            }
            Err(WriteFailure::Read(error)) => {
                if let Some(LimitHit::Ask(question)) = limit_of(&error) {
                    state.stop = Some(Stop::Ask(question.clone(), n));
                    return Ok(false);
                }
                match io_to_vfs(error) {
                    VfsError::Cancelled => Err(VfsError::Cancelled),
                    VfsError::WrongPassword => {
                        state.stop = Some(Stop::WrongPassword);
                        Ok(false)
                    }
                    other => {
                        state.skipped.insert(n);
                        state.report.failed.push((target, other.to_string()));
                        state.progress.item_done();
                        Ok(true)
                    }
                }
            }
            Err(WriteFailure::Write(VfsError::Cancelled)) => Err(VfsError::Cancelled),
            Err(WriteFailure::Write(error)) => {
                state.skipped.insert(n);
                state.report.failed.push((target, error.to_string()));
                state.progress.item_done();
                Ok(true)
            }
        }
    }

    /// The times and safe mode a file lands with, as the place can keep
    /// them.
    fn times_for(
        &self,
        target: &Place,
        modified: Option<i64>,
        mode: Option<u32>,
    ) -> AttributeChange {
        let mut change = AttributeChange::default();
        if self.env.engine.options().keep_dates {
            change.modified = modified.map(|secs| Timestamp::from_unix(secs, 0));
        }
        if let Some(mode) = mode
            && self.provider.capabilities(target).access.mode.is_possible()
        {
            change.attributes = Some(Attributes::Unix(safe_mode(mode)));
        }
        change
    }

    /// Makes a link member: a relative link the plan found to stay inside.
    fn link(&self, state: &mut State<'_>, n: usize) {
        let member = &self.plan.members[n];
        let Some(target) = member.link_target.clone() else {
            self.skip(state, n, "link not followed".to_owned());
            return;
        };
        let Some(folder) = member.target.parent() else {
            return;
        };
        if let Err(reason) = self.ensure_folder(state, &folder) {
            self.skip(state, n, reason);
            return;
        }
        if self.provider.stat(&member.target, self.ctx).is_ok() {
            self.skip(state, n, "the name is taken".to_owned());
            return;
        }
        let request = LinkRequest {
            kind: LinkKind::Symbolic,
            target,
            target_is_folder: member.link_to_folder,
        };
        match self
            .provider
            .create_link(&member.target, &request, self.ctx)
        {
            Ok(()) => {
                state.report.undo.created.push(member.target.clone());
                self.done(state, n);
            }
            Err(error) => {
                state.skipped.insert(n);
                state
                    .report
                    .failed
                    .push((member.target.clone(), error.to_string()));
            }
        }
    }

    /// Makes sure a folder under the destination exists, making what is
    /// missing on the way, and that nothing on the way is a link or a
    /// file. The chosen folder itself is the user's and is not checked.
    fn ensure_folder(&self, state: &mut State<'_>, folder: &Place) -> Result<(), String> {
        if state.known_folders.contains(folder) || *folder == self.plan.chosen {
            return Ok(());
        }
        if !folder.is_within(&self.plan.chosen) {
            return Err("it would land outside the destination".to_owned());
        }
        let mut chain = Vec::new();
        let mut at = folder.clone();
        while at != self.plan.chosen && !state.known_folders.contains(&at) {
            chain.push(at.clone());
            match at.parent() {
                Some(up) => at = up,
                None => return Err("it would land outside the destination".to_owned()),
            }
        }
        for place in chain.into_iter().rev() {
            match self.provider.stat(&place, self.ctx) {
                Ok(entry) if entry.kind == EntryKind::Folder && entry.link.is_none() => {}
                Ok(entry) if entry.kind == EntryKind::Link || entry.link.is_some() => {
                    return Err("a folder on the way is a link".to_owned());
                }
                Ok(_) => return Err("a file is in the way".to_owned()),
                Err(VfsError::NotFound) => match self.provider.create_dir(&place, self.ctx) {
                    Ok(()) => state.report.undo.created.push(place.clone()),
                    Err(error) => return Err(format!("could not make its folder: {error}")),
                },
                Err(error) => return Err(error.to_string()),
            }
            state.known_folders.insert(place);
        }
        Ok(())
    }

    /// The folders' times, newest first so a folder's own change does not
    /// undo its time; the check against a fresh read; the last report.
    fn finish(&self, state: &mut State<'_>) {
        if self.env.engine.options().keep_dates && !state.report.cancelled {
            for (place, modified) in state.folders.iter().rev() {
                if let Some(secs) = modified {
                    let change = AttributeChange {
                        modified: Some(Timestamp::from_unix(*secs, 0)),
                        ..AttributeChange::default()
                    };
                    let _ = self
                        .provider
                        .set_attributes(place, &change, &OpContext::new());
                }
            }
        }
        let mut check = Reconciliation::of(state.report.done.clone());
        for place in &state.report.done {
            if self.provider.stat(place, &OpContext::new()).is_ok() {
                check.present.push(place.clone());
            } else {
                check.missing.push(place.clone());
            }
        }
        state.report.reconciliation = Some(check);
        state.progress.finish();
    }
}

fn state_done(done: &[bool], skipped: &HashSet<usize>, n: usize) -> bool {
    done.get(n).copied().unwrap_or(true) || skipped.contains(&n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::QuestionKind;

    #[test]
    fn modes_are_made_safe() {
        assert_eq!(safe_mode(0o4755), 0o755);
        assert_eq!(safe_mode(0o666), 0o644);
        assert_eq!(safe_mode(0o777), 0o755);
        assert_eq!(safe_mode(0o611), 0o600);
        assert_eq!(safe_mode(0o1777), 0o755);
        assert_eq!(safe_mode(0o700), 0o700);
    }

    #[test]
    fn a_question_kind_is_kept() {
        let question = Question {
            kind: QuestionKind::Ratio,
            text: "Expands to 40 GB from 40 KB. Read zeros.bin".to_owned(),
        };
        assert!(declined_reason(&question).contains("Expands to 40 GB"));
    }
}
