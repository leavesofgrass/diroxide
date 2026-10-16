//! The app core of diroxide: it holds every panel, job and dialog, turns
//! keys into commands, dispatches commands, and returns [`Effect`]s for a
//! frontend and the [`Runtime`] to carry out. Frontends draw and announce;
//! a feature exists when the core has it. Tests drive the core without a
//! terminal or a window.
//!
//! Module owners after Phase 0: W1c owns `commands`, `keymap`, `menu`,
//! `palette`, `settings`, `entry` and `messages`; W1d owns the rest
//! (`app`, `panel`, `dialog`, `jobs`, `runtime`); later waves add their
//! own files.
//!
//! [`Effect`]: diroxide_core::Effect

pub mod app;
pub mod commands;
pub mod dialog;
pub mod entry;
pub mod jobs;
pub mod keymap;
pub mod menu;
pub mod messages;
pub mod palette;
pub mod panel;
pub mod runtime;
pub mod settings;

pub use app::{App, AppEvent, AppOptions};
pub use commands::{CATALOG, CommandInfo, Menu};
pub use dialog::{Answer, Dialog};
pub use jobs::JobTable;
pub use keymap::{Binding, Keymap, Lookup};
pub use panel::{Column, Panel};
pub use runtime::Runtime;
