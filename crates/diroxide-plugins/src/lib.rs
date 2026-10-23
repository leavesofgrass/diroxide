//! The plugin system: a protocol, not a language.
//!
//! - **Process plugins:** a folder with a `plugin.toml` [`Manifest`] and a
//!   program in any language, spoken to in JSON-RPC 2.0 over standard input
//!   and output, one JSON object per line ([`protocol`]), with shared JSON
//!   shapes for places, entries and the command context ([`shapes`]), through a
//!   [`PluginTransport`] (standard input and output today; WebAssembly can
//!   be a second transport later without changing the API).
//! - **Script commands:** one-file scripts with a header comment, run with
//!   the context in environment variables and `diro:` directives on their
//!   output ([`script`]).
//! - **No in-process tier.** The plugin boundary is the language boundary
//!   (plan section 5.8, item 3): Lua scripts run through the Lua runner
//!   plugin, a process like any other, and the column formatters and row
//!   filters the old Lua tier was meant for use the batched column request.
//!
//! A plugin runs with the user's rights, so nothing is enabled by default
//! and the manifest's capabilities are shown on first enable. A crash or a
//! timeout is reported and the app goes on.
//!
//! How the host is built, from the wire up:
//!
//! - [`framing`] reads lines with a bound and writes them from a queue;
//!   [`transport`] carries them over a process's pipes or, in tests, over
//!   channels; the process itself starts through `diroxide-tools`' process
//!   helper (no shell, no console window, a tree killed as a whole), with
//!   its standard error in the plugin's [`log`].
//! - [`protocol`] holds the envelopes, the error kinds and the method list
//!   ([`protocol::methods`]), from which `docs/dev/plugin-protocol.md` is
//!   generated; [`manifest`] reads `plugin.toml`.
//! - [`session`] is the handshake and the polite ending in both
//!   directions; [`router`] matches answers to requests and serves the
//!   plugin's own requests; [`supervisor`] keeps the inactivity timers,
//!   the leases, the idle timer and the crash and flood policies on a
//!   clock tests move by hand; [`live`] runs one plugin; [`host`] starts
//!   plugins on demand and turns what happens into events.
//! - [`services`] are the host services a plugin calls: announcements
//!   (cleaned, attributed with the plugin's name last, rate-limited), its
//!   log, its settings and its progress; every sentence is in [`lines`].
//! - [`testing`] has the in-process fake plugin and the dialogue driver;
//!   [`fuzzing`] the bodies of the fuzz targets.
//!
//! W4a-1 built the host up to here (alpha 2, commits 1 to 8 of R22's
//! order); W4a-2 adds the rest of the host services, the plugin provider,
//! listers, hooks, the fake plugin process and the conformance runner;
//! W4b owns [`script`].

pub mod ask;
pub mod base64;
mod calls;
pub mod conformance;
pub mod error;
pub mod files;
pub mod framing;
pub mod fuzzing;
pub mod hooks;
pub mod host;
pub mod lines;
pub mod listers;
pub mod live;
pub mod log;
pub mod manifest;
pub mod panels;
pub mod places;
pub mod protocol;
pub mod provider;
pub mod router;
pub mod script;
pub mod services;
pub mod session;
pub mod shapes;
pub mod spool;
pub mod supervisor;
pub mod testing;
pub mod tools;
pub mod transport;

pub use ask::{Answer, Question, QuestionKind, TextDocument};
pub use error::PluginError;
pub use host::{Host, HostEvent, HostServices, Launcher, PluginInfo};
pub use manifest::{CommandSpec, Manifest, ManifestReport, Problem, Severity};
pub use panels::{PanelChange, PanelChoice, PanelSnapshot};
pub use protocol::{ErrorKind, Id, Incoming, Notification, Request, Response, RpcError};
pub use shapes::{PanelRef, WireContext, WireEntry, WireImportance, WirePlace};
pub use transport::{MemoryPeer, MemoryTransport, PluginTransport, StdioTransport};
