//! The plugin's answers to the host: `initialize`, the Git column and the
//! title line.
//!
//! A skeleton from alpha 4's contract step (P4): `column/format` answers
//! no value, so every cell says "pending", and `place/describe` answers
//! no text, so the title line keeps its own words, until the git lane
//! (commits 98 to 107) runs git after the trust scan.

use std::sync::Arc;

use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities,
};
use serde_json::{Value, json};

use crate::rpc::{Cancel, Handler, Peer, WORKERS};

/// The plugin's name, as its manifest says and every line it causes ends
/// with.
pub const NAME: &str = "Git";

/// The column it declares.
pub const COLUMN: &str = "git";

/// The plugin.
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
                columns: vec![COLUMN.to_owned()],
                place_info: true,
                concurrency: u32::try_from(WORKERS).ok(),
                ..PluginCapabilities::default()
            },
            warnings: Vec::new(),
        }
    }

    fn request(
        &self,
        _peer: &Arc<Peer>,
        method: &str,
        params: Params,
        cancel: &Cancel,
    ) -> Result<Value, RpcError> {
        cancel.check()?;
        let _ = params;
        match method {
            methods::COLUMN_FORMAT => Ok(json!({"values": {}})),
            methods::PLACE_DESCRIBE => Ok(json!({})),
            _ => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {method}"),
            )),
        }
    }
}
