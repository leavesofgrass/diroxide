//! `provider/list`: one folder of an image. At the root of a disk with a
//! table, one folder per partition, its words in the row ("Partition 2,
//! home, ext4, 40.0 GB"); elsewhere the file system's folder under the
//! tree's rules, with the names and loops it left out said once after
//! the listing ("3 names skipped", "Skipped a loop in the image").

use serde_json::{Value, json};

use super::{Call, folder_entry, inner_of, wire_entry};
use crate::error::DiskError;
use crate::host;
use crate::image::Layout;
use crate::lines;

/// Answers `provider/list`.
pub fn list(call: &Call<'_>) -> Result<Value, DiskError> {
    let at = call.at("place")?;
    let open = call.images.get(call.peer, &at.container)?;
    let inner = inner_of(&open, &at)?;
    if let (Layout::Table { partitions, .. }, None) = (&open.image.layout, inner.partition) {
        let entries: Vec<Value> = partitions
            .iter()
            .map(|p| {
                let entry = folder_entry(&p.folder(), Some(p.volume.backing.len()), Some(p.row()));
                serde_json::to_value(entry).unwrap_or(Value::Null)
            })
            .collect();
        return Ok(json!({"entries": entries, "complete": true}));
    }
    let volume = open.image.volume(inner.partition)?;
    let listing = volume.with_tree(|tree| tree.list(&inner.names))?;
    if let Some(note) = lines::listing_notes(listing.loops_skipped, listing.names_skipped) {
        host::announce(call.peer, &note, "status");
    }
    let entries: Vec<Value> = listing
        .rows
        .iter()
        .map(|row| serde_json::to_value(wire_entry(row)).unwrap_or(Value::Null))
        .collect();
    Ok(json!({"entries": entries, "complete": true}))
}
