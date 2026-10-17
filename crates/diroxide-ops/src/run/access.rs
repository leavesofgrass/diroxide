//! Permissions, attributes and times in the run (research/permissions.md
//! 6.6; research/alpha2-plan.md commit 27): each change applied only when
//! the entry is still what the plan saw, read back, and its inverse kept
//! for Undo.

use diroxide_core::Place;
use diroxide_vfs::{AttributeChange, OpContext, VfsError};

use super::{Outcome, Run};

impl Run<'_> {
    /// Applies one change. An entry that changed since the plan is skipped,
    /// never overwritten; a change the disk ignored is said; the inverse,
    /// which the provider returns, goes into the undo record with the new
    /// value as its own check, so Undo skips an entry changed since.
    pub(super) fn set_attributes(&mut self, place: &Place, change: &AttributeChange) -> Outcome {
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        let checked = change.expect.is_some();
        let applied = self.attempt(place, |_| {
            match provider.set_attributes(place, change, &ctx) {
                Ok(inverse) => Ok(Some(inverse)),
                Err(VfsError::Changed | VfsError::Unsafe { .. }) if checked => Ok(None),
                Err(error) => Err(error),
            }
        });
        let mut inverse = match applied {
            Ok(Some(inverse)) => inverse,
            Ok(None) => return Outcome::Skipped("it changed after the plan".to_owned()),
            Err(stop) => return stop.into(),
        };
        if let Ok(now) = provider.stat(place, &OpContext::new()) {
            let tolerance = self.engine.options().time_tolerance_seconds;
            let near = |a: diroxide_core::Timestamp, b: diroxide_core::Timestamp| {
                a <= b.plus_seconds(tolerance) && b <= a.plus_seconds(tolerance)
            };
            if change
                .attributes
                .is_some_and(|wanted| now.attributes != Some(wanted))
            {
                self.warn(
                    place,
                    "Permissions did not change: the disk ignored the request.",
                );
            }
            if change
                .modified
                .is_some_and(|wanted| now.modified.is_none_or(|found| !near(found, wanted)))
            {
                self.warn(
                    place,
                    "The modified time did not change: the disk ignored it.",
                );
            }
        }
        if !inverse.is_empty() {
            if inverse.attributes.is_some() {
                inverse.expect = change.attributes;
            }
            self.report.undo.attributes.push((place.clone(), inverse));
        }
        Outcome::Done
    }
}
