//! The join runner (plan section 5.4; research/alpha2-plan.md commit 25).
//!
//! - The parts are read through the parts reader over the plan's snapshot,
//!   so a part that changed since the plan, or while it is read, stops the
//!   join with its number ("Part 004 changed while it was joined"), and
//!   never puts other bytes in the whole.
//! - The whole is written through the provider's committing writer under
//!   its unfinished name (journaled), so a cancelled or failed join leaves
//!   nothing; a name taken is the engine's conflict question.
//! - The CRC-32 streams through the whole as it is written, and the size
//!   is read back after the commit; the verdict against the sidecar is the
//!   report's [`JoinOutcome`](crate::join::JoinOutcome), a mismatch a
//!   warning as well. The parts are never touched.

use std::io::{self, Read, Write};

use diroxide_core::catalog;
use diroxide_core::{Entry, EntryKind, Place};
use diroxide_vfs::{FileWriter, OpContext, TrashCheck, VfsError};

use super::{Outcome, Run, vfs_of};
use crate::join::{JoinFacts, JoinOutcome, Verdict};
use crate::parts::{PartInfo, PartsError, PartsReader};

/// A join while its parts are written into the whole.
pub(super) struct JoinState {
    /// The whole as planned.
    planned: Place,
    /// The whole as settled: the planned place, or a free name.
    output: Place,
    writer: Option<Box<dyn FileWriter>>,
    reader: Option<PartsReader>,
    crc: crc32fast::Hasher,
    done: u64,
    /// Whether the whole replaces a file there, as the user answered.
    replace: bool,
    /// Why the rest of the join's steps are skipped.
    broken: Option<String>,
}

impl Run<'_> {
    /// One part of a join: its bytes, read through the parts reader, added
    /// to the whole; after the last part, the commit and the verdict.
    pub(super) fn join_part(&mut self, from: &Place, to: &Place) -> Outcome {
        let plan = self.plan;
        let Some(facts) = plan.facts.join.as_ref() else {
            return Outcome::Failed("the join has lost its parts".to_owned());
        };
        let label = |place: &Place| {
            facts
                .set
                .parts
                .iter()
                .find(|part| part.place == *place)
                .map_or_else(
                    || super::name_of(place),
                    |part| facts.set.label(part.number),
                )
        };
        if self.join.as_ref().is_none_or(|state| state.planned != *to) {
            if facts
                .set
                .parts
                .first()
                .is_none_or(|part| part.place != *from)
            {
                return Outcome::Skipped("the join did not start at its first part".to_owned());
            }
            if let Err(outcome) = self.start_join(facts, from, to) {
                return outcome;
            }
        }
        if let Some(reason) = self.join.as_ref().and_then(|state| state.broken.clone()) {
            return Outcome::Skipped(reason);
        }
        let length = plan
            .expected
            .get(from)
            .and_then(|entry| entry.size)
            .unwrap_or(0);
        if let Err(stop) = self.stream_part(length) {
            return match stop {
                Streamed::Cancelled => Outcome::Cancelled,
                Streamed::Changed => {
                    let reason = format!("{} changed while it was joined", label(from));
                    self.stop_join(&reason);
                    Outcome::Failed(reason)
                }
                Streamed::Failed(cause) => {
                    self.stop_join(&format!("{} could not be read", label(from)));
                    Outcome::Failed(cause)
                }
            };
        }
        let last = facts
            .set
            .parts
            .last()
            .is_some_and(|part| part.place == *from);
        if last {
            return self.end_join(facts);
        }
        Outcome::Done
    }

    /// Settles the whole's name, opens the parts and the writer.
    fn start_join(&mut self, facts: &JoinFacts, from: &Place, to: &Place) -> Result<(), Outcome> {
        let total = facts.set.total();
        let whole = Entry::new(to.name().unwrap_or_default(), EntryKind::File).with_size(total);
        let (output, mode, replace) = self.settle(from, &whole, to)?;
        let destination = self.provider(&output)?;
        self.check_target_name(&*destination, &output)?;
        let parts_provider = self.provider(from)?;
        let infos: Vec<PartInfo> = facts
            .set
            .parts
            .iter()
            .map(|part| match self.plan.expected.get(&part.place) {
                Some(entry) => PartInfo::of(&part.place, entry),
                None => PartInfo::new(part.place.clone(), part.size),
            })
            .collect();
        let reader = PartsReader::with_parts(parts_provider, infos, &self.ctx)
            .map_err(|error| Outcome::Failed(error.to_string()))?;
        let ctx = self.ctx.clone();
        let writer = self.attempt(&output, |_| destination.write(&output, mode, &ctx))?;
        if let Some(unfinished) = writer.unfinished_place()
            && let Some(journal) = &mut self.journal
        {
            let _ = journal.writing(&output, &unfinished);
        }
        self.join = Some(JoinState {
            planned: to.clone(),
            output,
            writer: Some(writer),
            reader: Some(reader),
            crc: crc32fast::Hasher::new(),
            done: 0,
            replace,
            broken: None,
        });
        Ok(())
    }

    /// Streams `length` bytes from the parts into the whole, with pause,
    /// cancel and progress between chunks.
    fn stream_part(&mut self, length: u64) -> Result<(), Streamed> {
        let mut buffer = vec![0u8; self.engine.options().chunk_bytes.max(1)];
        let mut moved: u64 = 0;
        while moved < length {
            if !self.hold() {
                return Err(Streamed::Cancelled);
            }
            let Some(state) = self.join.as_mut() else {
                return Err(Streamed::Failed("the join has lost its writer".to_owned()));
            };
            let (Some(reader), Some(writer)) = (state.reader.as_mut(), state.writer.as_mut())
            else {
                return Err(Streamed::Failed("the join has lost its writer".to_owned()));
            };
            let want = usize::try_from(length - moved)
                .unwrap_or(usize::MAX)
                .min(buffer.len());
            let read = match reader.read(&mut buffer[..want]) {
                Ok(0) => return Err(Streamed::Changed),
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Streamed::from_io(error)),
            };
            if let Err(error) = writer.write_all(&buffer[..read]) {
                return Err(Streamed::Failed(vfs_of(error).to_string()));
            }
            state.crc.update(&buffer[..read]);
            state.done += read as u64;
            moved += read as u64;
            self.advance(read as u64);
        }
        Ok(())
    }

    /// After the last part: the old file to the trash when it is replaced,
    /// the commit, the size read back, the verdict.
    fn end_join(&mut self, facts: &JoinFacts) -> Outcome {
        let Some(mut state) = self.join.take() else {
            return Outcome::Failed("the join has lost its writer".to_owned());
        };
        let Some(mut writer) = state.writer.take() else {
            return Outcome::Failed("the join has lost its writer".to_owned());
        };
        drop(state.reader.take());
        let output = state.output.clone();
        let destination = match self.provider(&output) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        if let Err(error) = writer.flush() {
            return Outcome::Failed(vfs_of(error).to_string());
        }
        let ctx = self.ctx.clone();
        let mut old_in_trash = false;
        if state.replace && self.keeps_old(&*destination, &output) {
            let check = match destination.trash(&output, &ctx) {
                Ok(check) => check,
                Err(error) => return Outcome::Failed(error.to_string()),
            };
            match self.post_check(&*destination, &output, check) {
                TrashCheck::InTrash | TrashCheck::NotChecked => old_in_trash = true,
                TrashCheck::GoneNotInTrash => self.warn(&output, catalog::TRASH_NOT_CONFIRMED),
                TrashCheck::StillThere => {
                    return Outcome::Failed(
                        "the old file is still there after the trash".to_owned(),
                    );
                }
            }
        } else if state.replace {
            self.warn(&output, "replaced; this cannot be undone");
        }
        if let Err(error) = writer.commit(false) {
            if old_in_trash && destination.restore(&output, &OpContext::new()).is_err() {
                self.warn(&output, "the old file is in the trash");
            }
            return Outcome::Failed(error.to_string());
        }
        if old_in_trash {
            self.report.undo.trashed.push(output.clone());
        }
        match destination.stat(&output, &OpContext::new()) {
            Ok(found) if found.size.is_none_or(|size| size == state.done) => {}
            Ok(_) => return Outcome::Failed("the whole is not the size of its parts".to_owned()),
            Err(error) => return Outcome::Failed(error.to_string()),
        }
        self.report.undo.created.push(output.clone());
        self.made(&output, EntryKind::File, Some(state.done));
        let crc32 = state.crc.finalize();
        let verdict = match facts.sidecar.as_ref().map(|(_, read)| read) {
            None => Verdict::NoSidecar,
            Some(read) => {
                let size_ok = read.size.is_none_or(|size| size == state.done);
                match read.crc32 {
                    _ if !size_ok => Verdict::Differs,
                    Some(crc) if crc == crc32 => Verdict::Matches,
                    Some(_) => Verdict::Differs,
                    None => Verdict::SizeOnly,
                }
            }
        };
        let outcome = JoinOutcome {
            output: output.clone(),
            parts: facts.set.places(),
            verdict,
            size: state.done,
            crc32,
        };
        if verdict == Verdict::Differs {
            self.warn(&output, outcome.sentence());
        }
        self.report.joined = Some(outcome);
        Outcome::Done
    }

    /// Stops a join: the writer dropped, so nothing of the whole is left;
    /// the rest of its steps skip.
    fn stop_join(&mut self, reason: &str) {
        if let Some(state) = self.join.as_mut() {
            state.writer = None;
            state.reader = None;
            state.broken = Some(format!("the join stopped: {reason}"));
        }
    }

    /// At the end of a run: a join that did not finish leaves nothing.
    pub(super) fn finish_join(&mut self) {
        if let Some(state) = self.join.take()
            && state.writer.is_some()
        {
            let name = super::name_of(&state.output);
            let planned = state.planned.clone();
            drop(state);
            self.warn(
                &planned,
                format!("Join of {name} stopped: nothing was written."),
            );
        }
    }
}

/// Why streaming a part stopped.
enum Streamed {
    Cancelled,
    /// A part ended early or changed since the plan.
    Changed,
    Failed(String),
}

impl Streamed {
    fn from_io(error: io::Error) -> Self {
        if let Some(parts) = PartsError::of(&error) {
            return match parts {
                PartsError::Changed { .. } => Streamed::Changed,
                other => Streamed::Failed(other.to_string()),
            };
        }
        match vfs_of(error) {
            VfsError::Cancelled => Streamed::Cancelled,
            other => Streamed::Failed(other.to_string()),
        }
    }
}
