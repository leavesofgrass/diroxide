//! The runtime half of compare folders and synchronize (alpha 5, lane
//! SA): the compare and the run as jobs on workers, answered as
//! [`crate::sync_ui::SyncNote`].
//!
//! Reserved by the pre-wiring step (lane P5): `Request::Sync` reaches
//! [`Runtime::sync_request`]; lane SA fills it as [`SyncAsk`] gains
//! variants.

use super::Runtime;
use crate::sync_ui::SyncAsk;

impl Runtime {
    /// Carries out one ask of lane SA.
    pub(super) fn sync_request(&mut self, ask: SyncAsk) {
        match ask {}
    }
}
