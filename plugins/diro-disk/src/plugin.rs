//! The plugin's answers to the host: `initialize`, the provider methods
//! of the `disk` scheme, `place/describe` for the title line, and
//! `provider/disconnect`, which closes an image when its last lease ends.

use std::path::PathBuf;
use std::sync::{Arc, PoisonError};

use diroxide_plugins::protocol::{ErrorKind, PROTOCOL, Params, RpcError, methods};
use diroxide_plugins::session::{
    InitializeParams, InitializeResult, Peer as Who, PluginCapabilities, ProviderCapability,
};
use serde_json::{Value, json};

use crate::error::DiskError;
use crate::host::Serving;
use crate::image::Layout;
use crate::lines;
use crate::provider::{self, At, Call, Images};
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
#[derive(Default)]
pub struct Plugin {
    images: Images,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("open_images", &self.images.len())
            .finish()
    }
}

impl Plugin {
    /// A plugin that has not been initialized.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The open images, for the tests.
    pub fn images(&self) -> &Images {
        &self.images
    }

    fn provider(
        &self,
        peer: &Arc<Peer>,
        method: &str,
        params: &Params,
        cancel: &Cancel,
    ) -> Result<Value, DiskError> {
        let call = Call {
            peer,
            params,
            cancel,
            images: &self.images,
        };
        match method {
            methods::PROVIDER_CONNECT => provider::connect::connect(&call),
            methods::PROVIDER_LIST => provider::list::list(&call),
            methods::PROVIDER_STAT => provider::stat::stat(&call),
            methods::PROVIDER_READ => provider::read::read(&call),
            methods::PROVIDER_VOLUMES => provider::volumes::volumes(&call),
            methods::PLACE_DESCRIBE => describe(&call),
            _ => Err(DiskError::Host(RpcError::new(
                ErrorKind::MethodNotFound,
                format!("no method {method}"),
            ))),
        }
    }
}

/// `place/describe`: the title line's words for a place in an image:
/// "Partition 2, in disk.img", "EFI, in fedora.iso", "home, in partition
/// 2 of disk.img".
fn describe(call: &Call<'_>) -> Result<Value, DiskError> {
    let at = call.at("place")?;
    let image = at.image_name();
    let open = call.images.get(call.peer, &at.container)?;
    let inner = open.image.inner(&at.inner)?;
    let text = match (&open.image.layout, inner.partition, inner.names.last()) {
        (_, None, None) => open.image.details_line(),
        (Layout::Table { .. }, Some(number), None) => lines::partition_title(number, &image),
        (Layout::Table { .. }, Some(number), Some(name)) => {
            lines::folder_title(name, &format!("partition {number} of {image}"))
        }
        (_, _, Some(name)) => lines::folder_title(name, &image),
        (Layout::Single(_), Some(_), None) => return Err(DiskError::NotFound),
    };
    Ok(json!({"text": text}))
}

impl Handler for Plugin {
    fn initialize(&self, params: &InitializeParams) -> InitializeResult {
        if !params.dirs.spool.is_empty() {
            *self
                .images
                .spool
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(PathBuf::from(&params.dirs.spool));
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
                place_info: true,
                concurrency: u32::try_from(WORKERS).ok(),
                transfer: vec!["inline".to_owned(), "file".to_owned()],
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
        let _serving = Serving::start(cancel);
        self.provider(peer, method, &params, cancel)
            .map_err(|e| e.to_rpc())
    }

    fn notification(&self, _peer: &Arc<Peer>, method: &str, params: Params) {
        if method == methods::PROVIDER_DISCONNECT {
            let place = params
                .get("authority")
                .or_else(|| params.get("place"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            match At::parse(place) {
                Ok(at) => self.images.close(&at.container.to_string()),
                Err(_) => self.images.close(place),
            }
        }
    }

    fn shutdown(&self) {
        self.images.close_all();
    }
}
