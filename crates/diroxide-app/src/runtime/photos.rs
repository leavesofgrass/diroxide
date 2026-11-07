//! The photo helpers' reader (alpha 4, lane AP, R30 commits 83 to 85):
//! each picture's date taken and camera, from its Exif block, read once
//! for a panel's pictures when a helper first needs them.

use std::sync::Arc;

use diroxide_vfs::{OpContext, Registry};

use super::Shared;
use crate::app::AppEvent;
use crate::photos::{PhotoAsk, PhotoFact, PhotoNote, read_fact};

/// Carries out one ask on the worker it runs on.
pub(super) fn run(shared: &Arc<Shared>, registry: &Registry, ask: PhotoAsk) {
    let note = match ask {
        PhotoAsk::Facts {
            side,
            listing,
            places,
        } => {
            let ctx = OpContext::new();
            let facts = places
                .iter()
                .map(|place| {
                    let name = place.name().unwrap_or_default().to_owned();
                    let fact = registry
                        .resolve(place)
                        .map(|provider| read_fact(provider.as_ref(), place, &ctx))
                        .unwrap_or_else(|_| PhotoFact::default());
                    (name, fact)
                })
                .collect();
            PhotoNote::Facts {
                side,
                listing,
                facts,
            }
        }
    };
    shared.send(AppEvent::Photos(Box::new(note)));
}
