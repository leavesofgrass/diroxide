//! The flat view in the app (alpha 5, lane LA): every file below a
//! folder in one list, streamed into the general results lister
//! (`App::open_list` with `ListKind::Flat`), over
//! `diroxide_search::flat_view`.
//!
//! Reserved by the pre-wiring step (lane P5); its command is routed by
//! `collections.rs`. Says "Not built yet: Flat view." until lane LA
//! fills it.

use diroxide_core::Command;

use super::{App, Effect};

impl App {
    /// Flat view: on for this panel's folder, or off.
    pub(super) fn flat_view_toggle(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }
}
