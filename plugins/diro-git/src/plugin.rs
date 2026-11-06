//! The plugin's answers to the host: `initialize`, the title line
//! (`place/describe`) and the Git column (`column/format`), with what
//! it keeps between them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities,
};
use serde_json::Value;

use crate::error::GitError;
use crate::folder;
use crate::git::version::Version;
use crate::host::Host;
use crate::repo::{self, Repo};
use crate::rpc::{Cancel, Handler, Peer, WORKERS};

/// The plugin's name, as its manifest says; the host adds it last to
/// every line the plugin causes.
pub const NAME: &str = "Git";

/// The column it declares.
pub const COLUMN: &str = "git";

/// The plugin and what it keeps for the session.
#[derive(Debug, Default)]
pub struct Plugin {
    /// git's version, once read.
    pub version: Mutex<Option<Version>>,
    /// Folders the repository walk never climbs into, as git's own
    /// `GIT_CEILING_DIRECTORIES` says.
    pub ceilings: Vec<PathBuf>,
}

impl Plugin {
    /// A plugin that has not been initialized, with the ceiling folders
    /// of its environment.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ceilings: repo::ceilings_from_environment(),
            ..Self::default()
        })
    }

    /// A plugin whose repository walk stops below these folders (tests,
    /// whose repositories live inside the project's own checkout).
    pub fn with_ceilings(ceilings: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            ceilings,
            ..Self::default()
        })
    }

    /// The repository a local folder is in.
    pub fn find_repo(&self, folder: &std::path::Path) -> Option<Repo> {
        repo::find_below(folder, &self.ceilings)
    }

    fn serve(
        &self,
        peer: &Arc<Peer>,
        method: &str,
        params: &Params,
        cancel: &Cancel,
    ) -> Result<Value, GitError> {
        let host = Host { peer, cancel };
        match method {
            methods::PLACE_DESCRIBE => folder::describe(self, host, params),
            methods::COLUMN_FORMAT => folder::column(self, host, params),
            _ => Err(GitError::Host(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {method}"),
            ))),
        }
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
        peer: &Arc<Peer>,
        method: &str,
        params: Params,
        cancel: &Cancel,
    ) -> Result<Value, RpcError> {
        cancel.check()?;
        self.serve(peer, method, &params, cancel)
            .map_err(|e| e.to_rpc())
    }

    fn notification(&self, peer: &Arc<Peer>, method: &str, params: Params) {
        if method == methods::TOOLS_OUTPUT {
            peer.deliver_output(&params);
        }
    }
}
