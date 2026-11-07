//! The app core of diroxide: it holds every panel, job and dialog, turns
//! keys into commands, dispatches commands, and returns [`Effect`]s for a
//! frontend and the [`Runtime`] to carry out. Frontends draw and announce;
//! a feature exists when the core has it. Tests drive the core without a
//! terminal or the GUI.
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
// The custom columns: fields' keys, regex and plugin columns (alpha 4,
// lane MA).
pub mod columns;
pub mod commands;
// Optional components in the app (W4d).
pub mod components;
// The details view's reading and words (alpha 4, lane VA).
pub mod details;
pub mod dialog;
pub mod entry;
pub mod jobs;
pub mod keymap;
pub mod labels;
pub mod menu;
// The metadata commands' requests, answers and words (alpha 4, lane MA).
pub mod messages;
pub mod metadata;
pub mod palette;
pub mod panel;
// Read text in picture, through Tesseract (alpha 4, lane VA).
pub mod picture_text;
// Plugins and script commands in the app (W4d).
pub mod plugins;
pub mod quickview;
pub mod request;
pub mod runtime;
pub mod settings;
pub mod startup;
pub mod state;
// The Settings Tools page's rows (alpha 4, lane VA).
pub mod tools_page;
// Watch this folder (W3t).
pub mod tracker;
// File types in the app: the background check (alpha 4, lane TA).
pub mod typing;
pub mod viewer;
pub mod views;

/// The configuration types a frontend reads through the app: the terminal
/// crate may depend only on core, app and theme (`cargo xtask deps`), and
/// draws from [`App::settings`]. The GUI reads its own three settings
/// here too: which frontend starts, how it draws, and whether its menus
/// are native or lists (lane A4).
pub mod config {
    pub use diroxide_config::{
        Frontend, Graphics, KeymapConfig, Layout, Menus, Paths, ScreenReaderMode, SecretEcho,
        Settings, SingleLetters,
    };
}

pub use app::{App, AppEvent, AppOptions, Clipboard};
pub use commands::{CATALOG, CommandInfo, Menu};
pub use dialog::{Answer, Dialog};
pub use jobs::JobTable;
pub use keymap::{Binding, Keymap, Lookup};
pub use panel::{Column, Panel};
pub use request::{AskId, Question, Reply, Request, Spawn, SpawnKind};
pub use runtime::{Handover, Launcher, OpenWithPrograms, Opener, Program, Runtime, SystemOpenWith};
pub use startup::Startup;
