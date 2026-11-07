//! What a task asks of the app or the command line (R28 6.1 and 6.3;
//! R30's decision 4): the parts this crate cannot reach, behind one
//! trait. The recipe book and the type registry for a conversion, the
//! filter language, the version a tool reports, and the plugin host that
//! runs a script command or a plugin's command unattended.
//!
//! The runner reaches tools only through the engine's `ToolRunner` and
//! conversions only through the Convert engine
//! ([`Engine::plan_convert`](crate::Engine::plan_convert)), so what
//! comes through here is never a shell string: a recipe, a type answer, a
//! yes or no, and the outcome of a command the host ran with its own
//! limits.

use std::time::Duration;

use diroxide_core::{Entry, Place};
use diroxide_vfs::CancelToken;

use super::approval::Resolved;
use crate::convert::recipe::Recipe;
use crate::convert::typing::{ByExtension, Typing};

/// One file for a script command or a plugin's command, run unattended.
/// The host sets `DIRO_UNATTENDED=1`, `DIRO_TRIGGER=hot-folder` and
/// `DIRO_HOT_RULE` to the task's name, and answers any question the
/// command would ask with "no user present".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRun {
    /// The task's name: `DIRO_HOT_RULE`.
    pub task: String,
    /// The command's id.
    pub command: String,
    /// The file: the selection, and the active panel's folder its folder.
    pub input: Place,
    /// The task's `out` folder: the other panel's place.
    pub out: Place,
    /// The task's time limit for one file, when it has one; the command's
    /// own applies too.
    pub timeout: Option<Duration>,
}

/// What a command did with one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionResult {
    /// It finished, and made these files (they are committed, under their
    /// final names).
    Made(Vec<Place>),
    /// It failed on this file. `retry` when the cause may pass (in use,
    /// timed out, stopped by a signal).
    Failed {
        /// The cause in words, after "Could not run ... on NAME: ".
        cause: String,
        /// Whether trying again may help.
        retry: bool,
    },
    /// It cannot run at all here (the command is gone, the host is not
    /// available): the task pauses and every file stays in `in`.
    Pause(String),
    /// The run was cancelled.
    Cancelled,
}

/// What the app and the command line give a task's runner. Every method
/// has a default that says the part is not here, so a frontend implements
/// only what it has; a task that needs a missing part pauses with the
/// reason.
pub trait TaskActions: Send + Sync {
    /// The recipe a convert task names, from the app's recipe book: the
    /// built-in recipes, the owner's `conversions.toml` and plugins'
    /// converters.
    fn recipe(&self, id: &str) -> Option<Recipe> {
        let _ = id;
        None
    }

    /// The type registry's door, for `{typed_input}` and the plan's
    /// refusals; by extension when the frontend has no registry.
    fn typing(&self) -> &dyn Typing {
        &ByExtension
    }

    /// The version a tool reports, for the approval key, when known.
    fn tool_version(&self, tool: &str) -> Option<String> {
        let _ = tool;
        None
    }

    /// Whether a file passes the task's filter expression (the filter
    /// language: `group images and size over 100 KB`); an error says why
    /// the expression cannot be read.
    fn passes(&self, filter: &str, place: &Place, entry: &Entry) -> Result<bool, String> {
        let _ = (filter, place, entry);
        Err("the filter language is not available here".to_owned())
    }

    /// What a script command resolves to, for the approval key: its id
    /// and the SHA-256 of its file. An error says why it cannot be found.
    fn resolve_script(&self, id: &str) -> Result<Resolved, String> {
        Err(format!("no script command is named {id}"))
    }

    /// What a plugin's command resolves to, for the approval key: its id
    /// and the plugin's version.
    fn resolve_plugin(&self, id: &str) -> Result<Resolved, String> {
        Err(format!("no plugin command is named {id}"))
    }

    /// Runs a script command that declares `unattended = "ok"` on one
    /// file. Blocking; stopped when `cancel` is set.
    fn run_script(&self, run: &ActionRun, cancel: &CancelToken) -> ActionResult {
        let _ = (run, cancel);
        ActionResult::Pause("script commands are not available here".to_owned())
    }

    /// Runs a plugin's command on one file. Blocking; stopped when
    /// `cancel` is set.
    fn run_plugin(&self, run: &ActionRun, cancel: &CancelToken) -> ActionResult {
        let _ = (run, cancel);
        ActionResult::Pause("plugin commands are not available here".to_owned())
    }
}

/// A frontend with none of the parts: move, copy and tell tasks run;
/// the others pause with their reason.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoActions;

impl TaskActions for NoActions {}
