//! The plugin's answers to the host: `initialize` and the three commands
//! (`command/run`). It keeps nothing between commands: no passphrase, no
//! identity, no key.

use std::sync::Arc;

use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities,
};
use serde_json::Value;

use crate::commands;
use crate::host::Host;
use crate::rpc::{Cancel, Handler, Peer};

/// The plugin's name, as its manifest says; the host adds it last to
/// every line the plugin causes.
pub const NAME: &str = "Encryption";

/// How many commands may run at once, as the manifest says.
pub const CONCURRENCY: u32 = 4;

/// The plugin. It holds no state: every secret lives only as long as the
/// command that asked for it.
#[derive(Debug, Default)]
pub struct Plugin;

impl Plugin {
    /// A plugin that has not been initialized.
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl Handler for Plugin {
    fn initialize(&self, _params: &InitializeParams) -> InitializeResult {
        InitializeResult {
            protocol: PROTOCOL,
            plugin: Who {
                name: NAME.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            plugin_capabilities: PluginCapabilities {
                commands: commands::IDS.iter().map(|c| (*c).to_owned()).collect(),
                concurrency: Some(CONCURRENCY),
                ..PluginCapabilities::default()
            },
            warnings: Vec::new(),
        }
    }

    fn request(
        &self,
        peer: &Arc<Peer>,
        method: &str,
        params: Params,
        cancel: &Cancel,
    ) -> Result<Value, RpcError> {
        cancel.check()?;
        let host = Host { peer, cancel };
        match method {
            methods::COMMAND_RUN => commands::run(host, &params).map_err(|e| e.to_rpc()),
            _ => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {method}"),
            )),
        }
    }
}
