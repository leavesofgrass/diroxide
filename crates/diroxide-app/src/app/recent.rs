//! Recent files in the app (alpha 5, lane LA): recorded on Enter and F4,
//! listed in the general results lister (`App::open_list` with
//! `ListKind::Recent`), over `diroxide_config`'s recent files.
//!
//! Reserved by the pre-wiring step (lane P5); its commands are routed by
//! `collections.rs`. Each says "Not built yet: NAME." until lane LA fills
//! it.

use diroxide_core::Command;

use super::{App, Effect};

impl App {
    /// Recent files: the list, newest first.
    pub(super) fn recent_files_list(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }

    /// Clear recent files.
    pub(super) fn clear_recent_files(&mut self, command: &Command) -> Vec<Effect> {
        self.not_built(&command.id)
    }
}
