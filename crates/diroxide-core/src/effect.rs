//! Effects: what the app core asks the outside world to do after a command.
//!
//! The core holds all state and never does I/O. Dispatching a command
//! changes that state and returns effects. A frontend handles the effects
//! that are about the screen and speech (redraw, announce, dialogs, quit);
//! the app's runtime handles the ones that are I/O (listing, jobs, plugins)
//! and sends their results back as events.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::ids::{DialogId, JobId, RequestId, Side};
use crate::job::JobRequest;
use crate::message::Message;
use crate::place::Place;

/// What part of the screen changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Redraw {
    /// Everything.
    All,
    /// One panel.
    Panel(Side),
    /// The title line, which says the active place.
    TitleLine,
    /// The status line, which says the key fact.
    StatusLine,
    /// The jobs line.
    JobsLine,
}

/// A request to run a plugin's command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCall {
    /// The plugin's name, from its manifest.
    pub plugin: String,
    /// The plugin command and its arguments.
    pub command: Command,
}

/// Something a frontend or the runtime must do. New kinds may be added, so
/// a frontend matches with a fallback arm.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    /// Draw part of the screen again from the app's state.
    Redraw(Redraw),
    /// Say a message through the screen reader (and show it).
    Announce(Message),
    /// Show a dialog the app has opened; its content is in the app's state.
    OpenDialog(DialogId),
    /// Close a dialog and return the focus to where it was.
    CloseDialog(DialogId),
    /// List a place in the background; the answer comes back as an event
    /// tagged with the request id.
    List {
        /// Tags the answer, so a late answer to an old request is dropped.
        request: RequestId,
        /// What to list.
        place: Place,
    },
    /// Start a job in the job engine.
    StartJob(JobRequest),
    /// Cancel a running job.
    CancelJob(JobId),
    /// Run a plugin's command.
    RunPlugin(PluginCall),
    /// Open a place with the system's default program.
    OpenExternally(Place),
    /// Open a web or mail address (`https://`, `http://` or `mailto:`)
    /// with the system's browser or mail program, after the user said yes:
    /// a link followed from the quick view (W2e's request 1). Never a file
    /// and never through a shell; the runtime checks the address again and
    /// refuses anything else.
    OpenAddress(String),
    /// Put places on the clipboard, for other file managers.
    SetClipboard(Vec<Place>),
    /// Put text on the system clipboard: a path or a name from the
    /// viewer's Copy Path and Copy Name (W2d's request 3, P2b). A frontend
    /// effect, since each frontend owns its clipboard: the terminal can set
    /// it with its own escape sequence, the GUI through the platform.
    SetClipboardText(String),
    /// Leave the program.
    Quit,
    /// The app has queued requests of its own for its runtime: work whose
    /// types live in the app's crates (plan a job and wait for its
    /// confirmation, run a confirmed plan, answer a job's question, watch a
    /// folder, write the session, a setting or a prompt's history), so the
    /// core names only that they are waiting. The runtime takes them here,
    /// in the order they were queued (`App::take_requests` through
    /// `Runtime::carry_out`); a frontend never sees it.
    AppRequests,
}

impl Effect {
    /// Whether this effect is for the frontend (screen and speech) rather
    /// than for the runtime (I/O).
    pub fn is_for_frontend(&self) -> bool {
        matches!(
            self,
            Effect::Redraw(_)
                | Effect::Announce(_)
                | Effect::OpenDialog(_)
                | Effect::CloseDialog(_)
                | Effect::SetClipboardText(_)
                | Effect::Quit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_split_between_frontend_and_runtime() {
        assert!(Effect::Redraw(Redraw::All).is_for_frontend());
        assert!(Effect::Announce(Message::info("Copied.")).is_for_frontend());
        assert!(!Effect::CancelJob(JobId::new(1)).is_for_frontend());
        assert!(!Effect::AppRequests.is_for_frontend());
        let text = Effect::SetClipboardText("report.txt".into());
        assert!(text.is_for_frontend(), "each frontend owns its clipboard");
        assert!(!Effect::SetClipboard(Vec::new()).is_for_frontend());
        let json = serde_json::to_string(&text).unwrap();
        assert_eq!(json, r#"{"set-clipboard-text":"report.txt"}"#);
        assert_eq!(serde_json::from_str::<Effect>(&json).unwrap(), text);
        let json = serde_json::to_string(&Effect::AppRequests).unwrap();
        assert_eq!(json, "\"app-requests\"");
        assert_eq!(
            serde_json::from_str::<Effect>(&json).unwrap(),
            Effect::AppRequests
        );
    }
}
