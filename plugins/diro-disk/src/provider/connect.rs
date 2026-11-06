//! `provider/connect`: the host opens an image once per container
//! (`D:\dl\fedora.iso!`) before its first call. The image is opened and
//! its table read; the answer says it is read only, and the opened line
//! is shown ("Opened disk.vhdx: 3 partitions, 120.0 GB", said on Say
//! Status), with the sparse guard's line when the container claims more
//! than its file holds.

use serde_json::{Value, json};

use super::{At, Call};
use crate::error::DiskError;
use crate::host;

/// Answers `provider/connect`.
pub fn connect(call: &Call<'_>) -> Result<Value, DiskError> {
    let at = call.at("place")?;
    let open = call.images.get(call.peer, &at.container)?;
    announce_opened(call, &at, &open.image);
    Ok(json!({
        "display_name": at.image_name(),
        "capabilities": {
            "writable": false,
            "links": true,
            "attributes": true,
            "case_sensitive": true,
        }
    }))
}

fn announce_opened(call: &Call<'_>, _at: &At, image: &crate::image::Image) {
    host::announce(call.peer, &image.opened_line(), "status");
    if let Some(sparse) = image.sparse_line() {
        host::announce(call.peer, &sparse, "status");
    }
}
