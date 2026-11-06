//! The plugin's answers to the host: `initialize`, and the provider
//! methods of the `disk` scheme.
//!
//! A skeleton from alpha 4's contract step (P4): an image connects and
//! lists as empty, and a name in it is not found, so the shapes are fixed
//! and the conformance suite passes until the disk lane (commits 86 to
//! 97) reads the images.

use std::sync::Arc;

use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities, ProviderCapability,
};
use serde_json::{Value, json};

use crate::rpc::{Cancel, Handler, Peer, WORKERS};

/// The scheme this plugin's provider is known by.
pub const SCHEME: &str = "disk";

/// The plugin's name, as its manifest says and every line it causes ends
/// with.
pub const NAME: &str = "Disk images";

/// The provider methods this build answers, without the `provider/`
/// prefix; the host calls nothing else.
pub const PROVIDER_METHODS: &[&str] = &["connect", "list", "stat", "read", "volumes"];

/// The plugin.
#[derive(Debug, Default)]
pub struct Plugin;

impl Plugin {
    /// A plugin that has not been initialized.
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

/// The answer to a name the skeleton cannot read yet.
fn not_read() -> RpcError {
    RpcError::new(
        ErrorKind::NotFound,
        "this disk image reader is not built yet",
    )
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
                providers: vec![ProviderCapability {
                    scheme: SCHEME.to_owned(),
                    methods: PROVIDER_METHODS.iter().map(|m| (*m).to_owned()).collect(),
                }],
                concurrency: u32::try_from(WORKERS).ok(),
                transfer: vec!["inline".to_owned()],
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
            methods::PROVIDER_CONNECT => Ok(json!({
                "display_name": NAME,
                "capabilities": {"writable": false, "links": true, "attributes": true}
            })),
            methods::PROVIDER_LIST => Ok(json!({"entries": [], "complete": true})),
            methods::PROVIDER_STAT | methods::PROVIDER_READ => Err(not_read()),
            methods::PROVIDER_VOLUMES => Ok(json!({"volumes": []})),
            _ => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {method}"),
            )),
        }
    }
}
