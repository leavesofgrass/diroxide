//! The plugin's answers to the host: `initialize`, the settings, and the
//! provider methods of the `sftp` scheme.
//!
//! This lane (S-A, R25 commits 46 to 56) builds what a connection stands
//! on: finding ssh, starting it, the askpass helper and its dialogs, the
//! SFTP codec and the multiplexer. The provider methods themselves are
//! the next lane's (S-B, commits 57 to 64): until then a provider call
//! that needs a connection answers `auth_required` ("Not connected"), the
//! answer the host meets by connecting, and `provider/connect` answers
//! that connections are not built yet.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_core::Place;
use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities, ProviderCapability,
};
use serde_json::Value;

use crate::lines;
use crate::rpc::{Cancel, Handler, Peer, WORKERS};
use crate::settings::Settings;

/// The scheme this plugin serves.
pub const SCHEME: &str = "sftp";

/// The plugin's name, as its manifest says and every line it causes ends
/// with (answer 90).
pub const NAME: &str = "SFTP places";

/// The provider methods this build answers, without the `provider/`
/// prefix; the host calls nothing else.
pub const PROVIDER_METHODS: &[&str] = &["connect", "list", "stat"];

/// What the plugin knows from the host.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The owner's settings.
    pub settings: Settings,
    /// The plugin's private folder for its state (the sites file, the
    /// askpass endpoints, ssh's debug log).
    pub state_dir: Option<PathBuf>,
}

/// The plugin.
#[derive(Debug, Default)]
pub struct Plugin {
    state: Mutex<State>,
}

impl Plugin {
    /// A plugin that has not been initialized.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A copy of what it knows now.
    pub fn state(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The authority of an `sftp` place named in a request, in words for a
/// message ("build.example").
fn host_of(params: &Params) -> Result<String, RpcError> {
    let text = params
        .get("place")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "a place is missing"))?;
    let place = Place::parse(text)
        .map_err(|e| RpcError::new(ErrorKind::InvalidParams, format!("the place: {e}")))?;
    place
        .remote()
        .filter(|r| r.scheme() == SCHEME)
        .map(|r| r.host().to_owned())
        .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "not an sftp place"))
}

impl Handler for Plugin {
    fn initialize(&self, params: &InitializeParams) -> InitializeResult {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.settings = Settings::from_values(&params.settings);
            state.state_dir = Some(params.dirs.state.as_str())
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from);
        }
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
        match method {
            methods::PROVIDER_CONNECT => Err(RpcError::with_hint(
                ErrorKind::Unavailable,
                lines::not_built_yet(&host_of(&params)?),
                lines::NOT_BUILT_HINT,
            )),
            methods::PROVIDER_LIST | methods::PROVIDER_STAT => Err(RpcError::with_hint(
                ErrorKind::AuthRequired,
                lines::not_connected(&host_of(&params)?),
                lines::OPEN_TO_CONNECT,
            )),
            other if other.starts_with("provider/") => Err(RpcError::new(
                ErrorKind::Unsupported,
                format!("{other} is not served yet"),
            )),
            other => Err(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {other}"),
            )),
        }
    }

    fn notification(&self, _peer: &Arc<Peer>, method: &str, params: Params) {
        if method == methods::SETTINGS_CHANGED
            && let Some(Value::Object(values)) = params.get("settings")
        {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .settings = Settings::from_values(values);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn params(value: Value) -> Params {
        match value {
            Value::Object(map) => map,
            _ => Params::new(),
        }
    }

    #[test]
    fn initialize_reads_the_settings_and_names_the_provider() {
        let plugin = Plugin::new();
        let init: InitializeParams = serde_json::from_value(json!({
            "protocol": 1,
            "settings": {"requests": 8},
            "dirs": {"state": "D:/state/sftp"}
        }))
        .unwrap();
        let answer = plugin.initialize(&init);
        assert_eq!(answer.plugin.name, "SFTP places");
        assert_eq!(answer.plugin_capabilities.providers[0].scheme, "sftp");
        assert_eq!(plugin.state().settings.requests, 8);
        assert_eq!(
            plugin.state().state_dir,
            Some(PathBuf::from("D:/state/sftp"))
        );
        let peer = Arc::new(Peer::new(Box::new(std::io::sink())));
        plugin.notification(
            &peer,
            methods::SETTINGS_CHANGED,
            params(json!({"settings": {"requests": 4}})),
        );
        assert_eq!(plugin.state().settings.requests, 4);
    }

    #[test]
    fn provider_calls_say_what_is_not_there_yet() {
        let plugin = Plugin::new();
        let peer = Arc::new(Peer::new(Box::new(std::io::sink())));
        let cancel = Cancel::new();
        let list = plugin
            .request(
                &peer,
                methods::PROVIDER_LIST,
                params(json!({"place": "sftp://build.example/srv"})),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(list.kind(), Some(ErrorKind::AuthRequired));
        assert_eq!(list.message, "Not connected to build.example.");
        let connect = plugin
            .request(
                &peer,
                methods::PROVIDER_CONNECT,
                params(json!({"place": "sftp://build.example/"})),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(connect.kind(), Some(ErrorKind::Unavailable));
        let bad = plugin
            .request(
                &peer,
                methods::PROVIDER_STAT,
                params(json!({"place": "memfs://box/x"})),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(bad.code, -32602);
        let unknown = plugin
            .request(&peer, "nothing/here", Params::new(), &cancel)
            .unwrap_err();
        assert_eq!(unknown.code, -32601);
    }
}
