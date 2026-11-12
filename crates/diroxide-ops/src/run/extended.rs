//! Extended attributes and streams in the run (lane MA's request 5): one
//! item written or removed, what it held before read first and kept in
//! the undo record, so Undo writes it back through the engine.

use diroxide_core::Place;
use diroxide_vfs::VfsError;

use super::{Outcome, Run};
use crate::undo::ExtendedBefore;

/// The most bytes of an item read to keep for Undo: the internet mark
/// and tags are a few hundred bytes; a larger item is refused, never
/// cut short, so Undo never writes back half of one.
pub(crate) const MOST_EXTENDED_BYTES: usize = 1 << 20;

impl Run<'_> {
    /// Sets or removes one item. A place whose provider keeps no
    /// extended items fails in words; removing an item that is not there
    /// is done with nothing to undo.
    pub(super) fn set_extended(
        &mut self,
        place: &Place,
        name: &str,
        value: Option<&[u8]>,
    ) -> Outcome {
        let provider = match self.provider(place) {
            Ok(provider) => provider,
            Err(stop) => return stop.into(),
        };
        let ctx = self.ctx.clone();
        let applied = self.attempt(place, |_| {
            let Some(extended) = provider.as_extended() else {
                return Err(VfsError::unsupported(
                    provider.scheme(),
                    "change extended items",
                ));
            };
            let there = extended
                .list_extended(place, &ctx)?
                .iter()
                .any(|item| item.name == name);
            let before = if there {
                Some(extended.read_extended(place, name, MOST_EXTENDED_BYTES, &ctx)?)
            } else {
                None
            };
            match value {
                Some(bytes) if before.as_deref() == Some(bytes) => return Ok(None),
                Some(bytes) => extended.write_extended(place, name, bytes, &ctx)?,
                None if before.is_none() => return Ok(None),
                None => extended.remove_extended(place, name, &ctx)?,
            }
            Ok(Some(before))
        });
        match applied {
            Ok(Some(before)) => {
                self.report.undo.extended.push(ExtendedBefore {
                    place: place.clone(),
                    name: name.to_owned(),
                    before,
                });
                Outcome::Done
            }
            Ok(None) => Outcome::Done,
            Err(stop) => stop.into(),
        }
    }
}
