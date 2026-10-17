//! Making links and changing where one points, in the run
//! (research/alpha2-plan.md commit 26; research/permissions.md 10).

use diroxide_core::catalog;
use diroxide_core::{EntryKind, Place};
use diroxide_vfs::{LinkRequest, OpContext, TrashCheck, VfsError};

use super::{Check, Outcome, Run};
use crate::links::NEEDS_DEVELOPER_MODE;

impl Run<'_> {
    /// Makes one link, with the usual recovery; a missing privilege is
    /// said as [`NEEDS_DEVELOPER_MODE`] at once, since retrying cannot help.
    pub(super) fn make_link(
        &mut self,
        place: &Place,
        request: &LinkRequest,
    ) -> Result<(), Outcome> {
        let provider = self.provider(place)?;
        let ctx = self.ctx.clone();
        let made = self.attempt(place, |_| {
            match provider.create_link(place, request, &ctx) {
                Ok(()) => Ok(true),
                Err(VfsError::NeedsPrivilege { .. }) => Ok(false),
                Err(error) => Err(error),
            }
        });
        match made {
            Ok(true) => Ok(()),
            Ok(false) => Err(Outcome::Failed(NEEDS_DEVELOPER_MODE.to_owned())),
            Err(stop) => Err(stop.into()),
        }
    }

    /// Changes where a link points: the new link beside it under a
    /// temporary name, the old one to the trash (checked), the new one
    /// given the name; any failure puts the old link back.
    pub(super) fn relink(&mut self, place: &Place, request: &LinkRequest) -> Outcome {
        let entry = match self.recheck(place, Check::Removal) {
            Ok(entry) => entry,
            Err(outcome) => return outcome,
        };
        if entry.kind != EntryKind::Link {
            return Outcome::Skipped("it is not a link now".to_owned());
        }
        let Some(folder) = place.parent() else {
            return Outcome::Failed("the link has no folder".to_owned());
        };
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let quiet = OpContext::new();
        let job = self.plan.job.get();
        let Some(temporary) = (1..=100)
            .filter_map(|n| folder.join(&format!("diro-link-{job}-{n}")).ok())
            .find(|candidate| provider.stat(candidate, &quiet) == Err(VfsError::NotFound))
        else {
            return Outcome::Failed("no free temporary name beside the link".to_owned());
        };
        if let Err(outcome) = self.make_link(&temporary, request) {
            return outcome;
        }
        let ctx = self.ctx.clone();
        let check = match provider.trash(place, &ctx) {
            Ok(check) => check,
            Err(error) => {
                let _ = provider.remove(&temporary, &quiet);
                return Outcome::Failed(format!("the old link could not go to the trash: {error}"));
            }
        };
        let in_trash = match self.post_check(&*provider, place, check) {
            TrashCheck::InTrash | TrashCheck::NotChecked => true,
            TrashCheck::GoneNotInTrash => {
                self.warn(place, catalog::TRASH_NOT_CONFIRMED);
                false
            }
            TrashCheck::StillThere => {
                let _ = provider.remove(&temporary, &quiet);
                return Outcome::Failed("the old link is still there after the trash".to_owned());
            }
        };
        if let Err(error) = provider.rename(&temporary, place, &quiet) {
            if in_trash && provider.restore(place, &quiet).is_err() {
                self.warn(place, "the old link is in the trash");
            }
            let _ = provider.remove(&temporary, &quiet);
            return Outcome::Failed(error.to_string());
        }
        if in_trash {
            self.report.undo.trashed.push(place.clone());
        }
        self.report.undo.created.push(place.clone());
        self.made(place, EntryKind::Link, None);
        Outcome::Done
    }
}
