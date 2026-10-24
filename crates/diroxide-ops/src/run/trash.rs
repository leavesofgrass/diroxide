//! Restoring from the trash and deleting from it for good, in the run
//! (R25 3.3, commit 85). A restore checks the name is free just before it
//! puts the item back, and a name taken meanwhile is skipped and counted,
//! never written over; a purge is permanent, and the plan listed it and
//! asked first.

use diroxide_core::EntryKind;
use diroxide_vfs::{OpContext, PurgeCheck, TrashedItem, VfsError};

use super::{Outcome, Run};
use crate::trash::NAME_TAKEN;

impl Run<'_> {
    /// Puts one item back where it was. A taken name, found before the
    /// call or by the trash itself, skips it with [`NAME_TAKEN`]; what is
    /// put back goes on the undo record's made list, so Undo trashes it
    /// again.
    pub(super) fn restore_item(&mut self, item: &TrashedItem) -> Outcome {
        let bin = match self.engine.trash_bin() {
            Ok(bin) => std::sync::Arc::clone(bin),
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let place = &item.original;
        if self.engine.is_taken(place, &OpContext::new()) {
            return Outcome::Skipped(NAME_TAKEN.to_owned());
        }
        let ctx = self.ctx.clone();
        let restored = self.attempt(place, |_| match bin.restore(item, &ctx) {
            Ok(()) => Ok(true),
            Err(VfsError::AlreadyExists) => Ok(false),
            Err(error) => Err(error),
        });
        match restored {
            Ok(true) => {
                let kind = self
                    .engine
                    .registry()
                    .resolve(place)
                    .and_then(|provider| provider.stat(place, &OpContext::new()))
                    .map_or(EntryKind::File, |entry| entry.kind);
                self.report.undo.created.push(place.clone());
                self.made(place, kind, None);
                Outcome::Done
            }
            Ok(false) => Outcome::Skipped(NAME_TAKEN.to_owned()),
            Err(stop) => stop.into(),
        }
    }

    /// Deletes one item in the trash for good. A trash that cannot say
    /// whether it is gone is a warning, never a silent yes.
    pub(super) fn purge_item(&mut self, item: &TrashedItem) -> Outcome {
        let bin = match self.engine.trash_bin() {
            Ok(bin) => std::sync::Arc::clone(bin),
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let ctx = self.ctx.clone();
        match self.attempt(&item.original, |_| bin.purge(item, &ctx)) {
            Ok(PurgeCheck::Gone) => Outcome::Done,
            Ok(PurgeCheck::NotChecked) => {
                self.warn(
                    &item.original,
                    "the trash could not be checked: it may still hold it",
                );
                Outcome::Done
            }
            Err(stop) => stop.into(),
        }
    }
}
