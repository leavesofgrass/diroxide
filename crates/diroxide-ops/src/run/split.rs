//! The split runner (plan section 5.4; research/alpha2-plan.md commit 24).
//!
//! - Each part is written through the provider's committing writer under
//!   its unfinished name, which the journal records first, so an
//!   interrupted split says what it left; a part is a file only once its
//!   last byte is written and committed.
//! - The CRC-32 is streamed part by part and folded in only when a part
//!   commits, so a part written again after Retry is counted once.
//! - The sidecar is written last, after the file split is read again and
//!   found unchanged; a file that changed while it was split fails with
//!   "report.pdf changed while it was split".
//! - A file's split is whole or nothing: when a part fails, or the job is
//!   cancelled, the parts already written for that file are removed (they
//!   are the job's own, and the file is still where it was), and nothing
//!   is sent to the trash. A file whose split finished keeps its parts and
//!   its sidecar, recorded as made so Undo sends them to the trash.

use std::io::{self, Read, Seek, SeekFrom, Write};

use diroxide_core::{EntryKind, JobState, Place};
use diroxide_vfs::{OpContext, Provider, VfsError, WriteMode};

use super::{Check, Outcome, Run, Stop, name_of, vfs_of};
use crate::sidecar::Sidecar;

/// The split of one file, while its parts are written.
pub(super) struct SplitState {
    /// The file split.
    source: Place,
    /// The CRC-32 of the parts committed so far.
    crc: crc32fast::Hasher,
    /// The bytes of the parts committed so far: the next part's offset.
    bytes: u64,
    /// The parts committed so far.
    written: Vec<Place>,
    /// Why the rest of this file's steps are skipped.
    broken: Option<String>,
}

impl SplitState {
    fn new(source: &Place) -> Self {
        Self {
            source: source.clone(),
            crc: crc32fast::Hasher::new(),
            bytes: 0,
            written: Vec::new(),
            broken: None,
        }
    }
}

impl Run<'_> {
    /// One part: `length` bytes of `from` from `offset`, as the new file
    /// `to`.
    pub(super) fn split_part(
        &mut self,
        from: &Place,
        to: &Place,
        offset: u64,
        length: u64,
    ) -> Outcome {
        if self
            .split
            .as_ref()
            .is_none_or(|state| state.source != *from)
        {
            self.split = Some(SplitState::new(from));
            if offset == 0
                && let Err(outcome) = self.recheck(from, Check::Exact)
            {
                self.break_split("its first part was not written");
                return outcome;
            }
        }
        if let Some(reason) = self.split.as_ref().and_then(|state| state.broken.clone()) {
            return Outcome::Skipped(reason);
        }
        if self
            .split
            .as_ref()
            .is_some_and(|state| state.bytes != offset)
        {
            self.break_split("an earlier part was not written");
            return Outcome::Failed("the parts are out of order".to_owned());
        }
        let source = match self.provider(from) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let destination = match self.provider(to) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let written = self.attempt(from, |run| {
            run.write_part(&*source, from, &*destination, to, offset, length)
        });
        match written {
            Ok(hasher) => {
                if let Some(state) = self.split.as_mut() {
                    state.crc.combine(&hasher);
                    state.bytes += length;
                    state.written.push(to.clone());
                }
                self.report.undo.created.push(to.clone());
                self.made(to, EntryKind::File, Some(length));
                Outcome::Done
            }
            Err(Stop::Cancelled) => Outcome::Cancelled,
            Err(Stop::Failed(cause)) => {
                self.undo_split(from);
                self.break_split("an earlier part could not be written");
                Outcome::Failed(cause)
            }
        }
    }

    /// Writes one part, once: the source read from `offset`, `length`
    /// bytes through a committing writer, pause and cancel between chunks.
    /// Returns the part's own CRC-32.
    fn write_part(
        &mut self,
        source: &dyn Provider,
        from: &Place,
        destination: &dyn Provider,
        to: &Place,
        offset: u64,
        length: u64,
    ) -> Result<crc32fast::Hasher, VfsError> {
        let ctx = self.ctx.clone();
        let mut reader: Box<dyn Read + Send> = match source.open_seekable(from, &ctx) {
            Ok(mut seekable) => {
                seekable.seek(SeekFrom::Start(offset)).map_err(vfs_of)?;
                Box::new(seekable)
            }
            Err(VfsError::Unsupported { .. }) => {
                let mut forward = source.read(from, &ctx)?;
                let skipped =
                    io::copy(&mut (&mut forward).take(offset), &mut io::sink()).map_err(vfs_of)?;
                if skipped != offset {
                    return Err(VfsError::Changed);
                }
                forward
            }
            Err(error) => return Err(error),
        };
        let mut writer = destination.write(to, WriteMode::CreateNew, &ctx)?;
        if let Some(unfinished) = writer.unfinished_place()
            && let Some(journal) = &mut self.journal
        {
            let _ = journal.writing(to, &unfinished);
        }
        let mut hasher = crc32fast::Hasher::new();
        let mut buffer = vec![0u8; self.engine.options().chunk_bytes.max(1)];
        let mut done: u64 = 0;
        let streamed = (|| -> Result<(), VfsError> {
            while done < length {
                if !self.hold() {
                    return Err(VfsError::Cancelled);
                }
                let want = usize::try_from(length - done)
                    .unwrap_or(usize::MAX)
                    .min(buffer.len());
                let read = match reader.read(&mut buffer[..want]) {
                    Ok(0) => return Err(VfsError::Changed),
                    Ok(read) => read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(vfs_of(error)),
                };
                writer.write_all(&buffer[..read]).map_err(vfs_of)?;
                hasher.update(&buffer[..read]);
                done += read as u64;
                self.advance(read as u64);
            }
            writer.flush().map_err(vfs_of)
        })();
        if let Err(error) = streamed {
            drop(writer);
            self.rewind(done);
            return Err(error);
        }
        if let Err(error) = writer.commit(false) {
            self.rewind(done);
            return Err(error);
        }
        Ok(hasher)
    }

    /// The sidecar, last: the file split is read again, and written only
    /// when it is as the plan saw it.
    pub(super) fn write_checksum(&mut self, of: &Place, place: &Place) -> Outcome {
        let Some(state) = self.split.as_ref().filter(|state| state.source == *of) else {
            return Outcome::Skipped("its parts were not written".to_owned());
        };
        if let Some(reason) = state.broken.clone() {
            return Outcome::Skipped(reason);
        }
        let crc32 = state.crc.clone().finalize();
        let bytes = state.bytes;
        let file = self
            .plan
            .facts
            .split
            .as_ref()
            .and_then(|facts| facts.files.iter().find(|file| file.sidecar == *place))
            .cloned();
        let (size, base) = match &file {
            Some(file) => (file.size, file.base.clone()),
            None => (
                bytes,
                place
                    .name()
                    .and_then(|name| name.strip_suffix(".crc"))
                    .unwrap_or_default()
                    .to_owned(),
            ),
        };
        let changed = format!("{} changed while it was split", name_of(of));
        let seen = self.plan.expected.get(of).cloned();
        let now = self
            .engine
            .registry()
            .resolve(of)
            .and_then(|provider| provider.stat(of, &OpContext::new()));
        let unchanged = match (&now, &seen) {
            (Ok(now), Some(seen)) => now.size == seen.size && now.modified == seen.modified,
            (Ok(now), None) => now.size == Some(size),
            (Err(_), _) => false,
        };
        if !unchanged || bytes != size {
            self.undo_split(of);
            self.split = None;
            return Outcome::Failed(changed);
        }
        let destination = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let text = Sidecar::new(base, size, crc32).to_bytes();
        self.emit(JobState::Verifying);
        let ctx = self.ctx.clone();
        let written = self.attempt(place, |_| {
            let mut writer = destination.write(place, WriteMode::CreateNew, &ctx)?;
            writer.write_all(&text).map_err(vfs_of)?;
            writer.flush().map_err(vfs_of)?;
            writer.commit(true)
        });
        self.emit(JobState::Running);
        match written {
            Ok(()) => {
                self.report.undo.created.push(place.clone());
                self.made(place, EntryKind::File, Some(text.len() as u64));
                self.split = None;
                Outcome::Done
            }
            Err(Stop::Cancelled) => Outcome::Cancelled,
            Err(Stop::Failed(cause)) => {
                self.undo_split(of);
                self.split = None;
                Outcome::Failed(format!("the checksum file could not be written: {cause}"))
            }
        }
    }

    /// Marks the current file's split as stopped: its later steps skip.
    fn break_split(&mut self, reason: &str) {
        if let Some(state) = self.split.as_mut() {
            state.broken = Some(reason.to_owned());
        }
    }

    /// Removes the parts written for a file whose split did not finish:
    /// the job's own new files, the original untouched. Nothing goes to
    /// the trash; what cannot be removed is said.
    fn undo_split(&mut self, source: &Place) {
        let Some(state) = self.split.as_mut().filter(|state| state.source == *source) else {
            return;
        };
        let written = std::mem::take(&mut state.written);
        let mut kept = 0u64;
        for part in written.iter().rev() {
            let removed = self
                .engine
                .registry()
                .resolve(part)
                .and_then(|provider| provider.remove(part, &OpContext::new()));
            match removed {
                Ok(()) => {
                    self.unmake(part);
                    self.gone(part);
                }
                Err(_) => kept += 1,
            }
        }
        if !written.is_empty() {
            let line = if kept == 0 {
                format!(
                    "Split of {} stopped: its parts were removed.",
                    name_of(source)
                )
            } else {
                format!(
                    "Split of {} stopped: {} could not be removed.",
                    name_of(source),
                    diroxide_core::format::count(kept, "part", "parts")
                )
            };
            self.warn(source, line);
        }
    }

    /// At the end of a run: a split that did not finish (cancelled between
    /// its parts) leaves no part.
    pub(super) fn finish_split(&mut self) {
        if let Some(source) = self.split.as_ref().map(|state| state.source.clone()) {
            self.undo_split(&source);
            self.split = None;
        }
    }
}
