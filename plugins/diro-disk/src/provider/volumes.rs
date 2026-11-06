//! `provider/volumes`: an image's partitions, each a volume with its size
//! (and what its file system says it holds free, where it says), so the
//! details can say them and a copy plan's room check works. With a place,
//! the one volume that holds it.

use serde_json::{Value, json};

use super::{At, Call, inner_of};
use crate::error::DiskError;
use crate::image::{Image, Layout, Volume};

fn row(name: String, place: String, volume: &Volume) -> Value {
    let mut row = json!({"name": name, "place": place, "total": volume.backing.len()});
    if let Ok(Some(free)) = volume.with_tree(|tree| Ok(tree.facts().free)) {
        row["free"] = json!(free);
    }
    row
}

fn rows(image: &Image, container: &str, only: Option<u32>) -> Vec<Value> {
    match &image.layout {
        Layout::Single(volume) => {
            let name = format!("{}, {}", image.name, image.layout_words());
            vec![row(name, format!("{container}!"), volume)]
        }
        Layout::Table { partitions, .. } => partitions
            .iter()
            .filter(|p| only.is_none_or(|n| n == p.number))
            .map(|p| {
                row(
                    p.volume_name(),
                    format!("{container}!{}", p.folder()),
                    &p.volume,
                )
            })
            .collect(),
    }
}

/// Answers `provider/volumes`.
pub fn volumes(call: &Call<'_>) -> Result<Value, DiskError> {
    if let Some(text) = call.text("place") {
        let at = At::parse(text)?;
        let open = call.images.get(call.peer, &at.container)?;
        let inner = inner_of(&open, &at)?;
        let container = at.container.to_string();
        let only = match (&open.image.layout, inner.partition) {
            (Layout::Table { .. }, None) => return Ok(json!({"volumes": []})),
            (_, number) => number,
        };
        return Ok(json!({"volumes": rows(&open.image, &container, only)}));
    }
    let authority = call.text("authority").unwrap_or_default();
    let Ok(container) = diroxide_core::Place::parse(authority) else {
        return Err(DiskError::NotFound);
    };
    if container.kind() == diroxide_core::PlaceKind::Remote {
        return Err(DiskError::NotFound);
    }
    let open = call.images.get(call.peer, &container)?;
    Ok(json!({"volumes": rows(&open.image, authority, None)}))
}
