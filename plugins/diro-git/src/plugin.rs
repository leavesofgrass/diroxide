//! The plugin's answers to the host: `initialize`, the title line
//! (`place/describe`), the Git column (`column/format`), the commands
//! (`command/run`) and the lists' rows (`lister/rows`), with what it keeps
//! between them: git's version, the status and scan caches, the trust
//! answers and the rows of the lists it opened.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities,
};
use serde_json::Value;

use crate::cache::{ScanCache, StatusCache};
use crate::commands::{self, Listers};
use crate::error::GitError;
use crate::folder;
use crate::git::version::Version;
use crate::host::Host;
use crate::repo::{self, Repo};
use crate::rpc::{Cancel, Handler, Peer, WORKERS};
use crate::store::TrustStore;

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
    /// Folder statuses for the title line and the column.
    pub statuses: StatusCache,
    /// Trust scans by repository.
    pub scans: ScanCache,
    /// The owner's trust answers.
    pub trust: TrustStore,
    /// Repositories already asked about this session, so the title line
    /// asks once.
    pub asked: Mutex<HashSet<PathBuf>>,
    /// The rows of the lists the plugin opened.
    pub listers: Listers,
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

    /// Marks a repository as asked about this session; false when it was
    /// already.
    pub fn first_ask(&self, root: &std::path::Path) -> bool {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(root.to_path_buf())
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
            methods::COMMAND_RUN => commands::run(self, host, params),
            methods::LISTER_ROWS => commands::rows(self, host, params),
            _ => Err(GitError::Host(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {method}"),
            ))),
        }
    }
}

impl Handler for Plugin {
    fn initialize(&self, params: &InitializeParams) -> InitializeResult {
        if !params.dirs.state.is_empty() {
            self.trust.set_folder(PathBuf::from(&params.dirs.state));
        }
        InitializeResult {
            protocol: PROTOCOL,
            plugin: Who {
                name: NAME.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            plugin_capabilities: PluginCapabilities {
                commands: commands::IDS.iter().map(|c| (*c).to_owned()).collect(),
                listers: commands::LISTERS.iter().map(|l| (*l).to_owned()).collect(),
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
        match method {
            methods::TOOLS_OUTPUT => {
                peer.deliver_output(&params);
            }
            methods::LISTER_CLOSED => {
                if let Some(lister) = params.get("lister").and_then(Value::as_str) {
                    self.listers.closed(lister);
                }
            }
            _ => {}
        }
    }
}
