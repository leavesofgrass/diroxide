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
//! Phase 0: the protocol types and parser, the manifest, the script header
//! and directive parsers, and the transport and host shapes. Wave 3 (W3a
//! and W3b) owns this crate.

pub mod error;
pub mod framing;
pub mod fuzzing;
pub mod host;
pub mod lines;
pub mod log;
pub mod manifest;
pub mod protocol;
pub mod router;
pub mod script;
pub mod session;
pub mod shapes;
pub mod testing;
pub mod transport;

pub use error::PluginError;
pub use host::{Host, HostServices, PanelSnapshot, PluginInfo};
pub use manifest::{CommandSpec, Manifest, ManifestReport, Problem, Severity};
pub use protocol::{ErrorKind, Id, Incoming, Notification, Request, Response, RpcError};
pub use shapes::{PanelRef, WireContext, WireEntry, WireImportance, WirePlace};
pub use transport::{MemoryPeer, MemoryTransport, PluginTransport, StdioTransport};
