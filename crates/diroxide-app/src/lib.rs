//! The app core of diroxide: it holds every panel, job and dialog, turns
//! keys into commands, dispatches commands, and returns [`Effect`]s for a
//! frontend and the [`Runtime`] to carry out. Frontends draw and announce;
//! a feature exists when the core has it. Tests drive the core without a
//! terminal or a window.
//!
//! A frontend's loop is three steps: hand each key to [`App::handle`],
//! give the effects to [`Runtime::carry_out`] (which does the I/O and
//! returns what to draw and say), and feed [`Runtime::poll`]'s events back
//! to [`App::handle`]. [`startup::load`] reads what the app starts with,
//! and [`startup::local_clock`] gives dates in local time.
//!
//! Module owners after Phase 0: W1c owns `commands`, `keymap`, `menu`,
//! `palette`, `settings`, `entry` and `messages`; W1d owns the rest
//! (`app`, `panel`, `dialog`, `jobs`, `runtime`, `request`, `startup`,
//! `state`); later waves add their own files: W2d owns `viewer`, W2e owns
//! `quickview`; W2f wires the archives (`archives`), the viewer, the quick
//! view and the alpha.2 dialogs; W3l owns `labels`, `bookmarks` and
//! `views` (the label, bookmark and folder-view stores and the History
//! list) and their wiring in `app`.
//!
//! [`Effect`]: diroxide_core::Effect

pub mod app;
pub mod archives;
pub mod bookmarks;
pub mod commands;
pub mod dialog;
pub mod entry;
pub mod jobs;
pub mod keymap;
pub mod labels;
pub mod menu;
pub mod messages;
pub mod palette;
pub mod panel;
pub mod quickview;
pub mod request;
pub mod runtime;
pub mod settings;
pub mod startup;
pub mod state;
pub mod viewer;
pub mod views;

/// The configuration types a frontend reads through the app: the terminal
/// crate may depend only on core, app and theme (`cargo xtask deps`), and
/// draws from [`App::settings`].
pub mod config {
    pub use diroxide_config::{Layout, ScreenReaderMode, SecretEcho, Settings};
}

pub use app::{App, AppEvent, AppOptions, Clipboard};
pub use commands::{CATALOG, CommandInfo, Menu};
pub use dialog::{Answer, Dialog};
pub use jobs::JobTable;
pub use keymap::{Binding, Keymap, Lookup};
pub use panel::{Column, Panel};
pub use request::{AskId, Question, Reply, Request, Spawn, SpawnKind};
pub use runtime::{Handover, Launcher, Opener, Program, Runtime};
pub use startup::Startup;
