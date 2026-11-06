//! `provider/stat`: one entry's details. The image's root is a folder
//! named as the image file; a partition's root is its folder with its
//! words; anything else is the entry its folder lists, found by the name
//! its row showed. A link is never followed.

use serde_json::{Value, json};

use super::{Call, folder_entry, inner_of, wire_entry};
use crate::error::DiskError;
use crate::image::tree::Row;

/// Answers `provider/stat`.
pub fn stat(call: &Call<'_>) -> Result<Value, DiskError> {
    let at = call.at("place")?;
    let open = call.images.get(call.peer, &at.container)?;
    let inner = inner_of(&open, &at)?;
    let entry = match (inner.partition, inner.names.as_slice()) {
        (None, []) => folder_entry(&at.image_name(), None, Some(open.image.details_line())),
        (Some(number), []) => {
            let partition = open.image.partition(number).ok_or(DiskError::NotFound)?;
            folder_entry(
                &partition.folder(),
                Some(partition.volume.backing.len()),
                Some(partition.row()),
            )
        }
        (partition, names) => {
            let found = open
                .image
                .volume(partition)?
                .with_tree(|tree| tree.stat(names))?;
            let shown = names.last().map(|n| (*n).to_owned()).unwrap_or_default();
            wire_entry(&Row {
                needs_care: String::from_utf8_lossy(&found.node.name) != shown,
                shown,
                node: found.node,
            })
        }
    };
    Ok(json!({"entry": serde_json::to_value(entry).unwrap_or(Value::Null)}))
}
