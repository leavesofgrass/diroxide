//! The runtime half of duplicates (alpha 5, lane DA): the scan as a job
//! on a worker, answered as [`crate::dups_ui::DupsNote`].
//!
//! Reserved by the pre-wiring step (lane P5): `Request::Duplicates`
//! reaches [`Runtime::dups_request`]; lane DA fills it as
//! [`DupsAsk`] gains variants.

use super::Runtime;
use crate::dups_ui::DupsAsk;

impl Runtime {
    /// Carries out one ask of lane DA.
    pub(super) fn dups_request(&mut self, ask: DupsAsk) {
        match ask {}
    }
}
