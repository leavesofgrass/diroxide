//! The runtime half of the flat view, collections and recent files
//! (alpha 5, lane LA): walks, reads and writes on workers, answered as
//! [`crate::lists::ListsNote`].
//!
//! Reserved by the pre-wiring step (lane P5): `Request::Lists` reaches
//! [`Runtime::lists_request`]; lane LA fills it as [`ListsAsk`] gains
//! variants.

use super::Runtime;
use crate::lists::ListsAsk;

impl Runtime {
    /// Carries out one ask of lane LA.
    pub(super) fn lists_request(&mut self, ask: ListsAsk) {
        match ask {}
    }
}
