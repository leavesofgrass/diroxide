//! The access dialogs' workers (alpha 4, lane AP, R30 commits 78 and 79):
//! the times a time job left, read back; and the hand-off of a file to
//! Windows' own Security dialog, through a seam a test replaces so no
//! test ever opens a system dialog.

use std::path::Path;
use std::sync::Arc;

use diroxide_vfs::Registry;

use super::Shared;
use crate::access::{AccessAsk, AccessNote, read_back};
use crate::app::AppEvent;

/// Opens the system's Security dialog for a file (Windows, the
/// Properties dialog's Security page), never waited for: the system's in
/// the program, a recorder in tests, which never opens one.
pub trait SecurityDialog: Send + Sync {
    /// Opens the dialog for the file; the cause in words when it cannot.
    fn open(&self, file: &Path) -> Result<(), String>;
}

/// The system's Security dialog. The file system crate has no call that
/// opens it yet (lane AP's contract request: `ShellExecuteExW` with the
/// "properties" verb and the "Security" page, in vfs's `sys` module), so
/// it says it could not, and the app says what to do in File Explorer.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSecurityDialog;

impl SecurityDialog for SystemSecurityDialog {
    fn open(&self, _file: &Path) -> Result<(), String> {
        Err("this build has no call that opens it".to_owned())
    }
}

/// Carries out one ask on the worker it runs on.
pub(super) fn run(
    shared: &Arc<Shared>,
    registry: &Registry,
    security: &Arc<dyn SecurityDialog>,
    ask: AccessAsk,
) {
    let note = match ask {
        AccessAsk::ReadBack { job, places, asked } => AccessNote::ReadBack {
            job,
            lines: read_back(registry, &places, &asked),
        },
        AccessAsk::SecurityDialog { place } => {
            let result = match place.local_path() {
                Some(path) => security.open(Path::new(path)),
                None => Err("it is not on this computer".to_owned()),
            };
            AccessNote::SecurityDialog { place, result }
        }
    };
    shared.send(AppEvent::Access(Box::new(note)));
}
