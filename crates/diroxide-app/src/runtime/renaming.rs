//! The runtime half of batch rename (alpha 5, lane RA): the folders'
//! facts, the editor buffer and the presets on workers, answered as
//! [`crate::rename_batch::RenameNote`].
//!
//! Reserved by the pre-wiring step (lane P5): `Request::RenameBatch`
//! reaches [`Runtime::rename_request`]; lane RA fills it as
//! [`RenameAsk`] gains variants.

use super::Runtime;
use crate::rename_batch::RenameAsk;

impl Runtime {
    /// Carries out one ask of lane RA.
    pub(super) fn rename_request(&mut self, ask: RenameAsk) {
        match ask {}
    }
}
