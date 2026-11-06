//! Photo facts from a JPEG, PNG or WebP file's Exif block and its other
//! metadata parts, background cost (a walk of the head and the few
//! segments that hold metadata): when it was taken and with what camera,
//! for the photo columns (R30's commit 83), and the private details
//! Check before sharing names: the place, the camera's serial number,
//! the owner, the author, comments, a stored thumbnail, and the XMP or
//! IPTC data that can hold more. The preview crate's image reader says
//! the picture's own facts (its size, color and orientation in words);
//! this one never decodes a pixel.
//!
//! A place is said with two decimals, about a kilometer (R28's location
//! risk); the full precision is [`crate::exif::ExifFacts::gps`], one
//! command away.

use diroxide_core::details::DetailGroup;
use diroxide_vfs::ReadSeek;

use crate::exif::{self, ExifFacts};
use crate::field::Field;
use crate::image::{ImageParts, read_parts};
use crate::reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
use crate::words::{coordinate_words, iso_date_words};

/// The photo facts reader.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhotoFacts;

impl Reader for PhotoFacts {
    fn id(&self) -> &'static str {
        "photo"
    }

    fn cost(&self) -> Cost {
        Cost::Background
    }

    fn keys(&self) -> &'static [&'static str] {
        &[
            "image.taken",
            "image.camera",
            "image.lens",
            "image.software",
            "image.gps",
            "image.serial",
            "image.lens_serial",
            "image.owner",
            "image.artist",
            "image.copyright",
            "image.comment",
            "image.thumbnail",
            "image.xmp",
            "image.iptc",
            "image.more_pictures",
        ]
    }

    fn accepts(&self, request: &ReadRequest<'_>) -> bool {
        super::takes(
            request,
            &["jpeg", "png", "webp"],
            &[".jpg", ".jpeg", ".jpe", ".png", ".webp"],
        )
    }

    fn read(
        &self,
        source: &mut dyn ReadSeek,
        _request: &ReadRequest<'_>,
        _limits: &ReadLimits,
    ) -> Result<Vec<Field>, MetaError> {
        let parts = read_parts(source)?;
        Ok(photo_fields(&parts))
    }
}

/// The camera in words: the maker and the model, the maker once when
/// the model already names it.
fn camera(facts: &ExifFacts) -> Option<String> {
    match (facts.make.as_deref(), facts.model.as_deref()) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
            Some(model.to_owned())
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (Some(one), None) | (None, Some(one)) => Some(one.to_owned()),
        (None, None) => None,
    }
}

/// The fields of a picture's metadata parts.
pub(crate) fn photo_fields(parts: &ImageParts) -> Vec<Field> {
    let facts = parts
        .exif
        .as_deref()
        .and_then(exif::parse)
        .unwrap_or_default();
    let mut fields = Vec::new();
    let mut add = |key: &str, label: &str, value: Option<String>, group, private: bool| {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            let field = Field::new(key, label, value, group);
            fields.push(if private { field.private() } else { field });
        }
    };
    add(
        "image.taken",
        "Taken",
        facts.taken.as_deref().and_then(iso_date_words),
        DetailGroup::Dates,
        false,
    );
    add(
        "image.camera",
        "Camera",
        camera(&facts),
        DetailGroup::Source,
        false,
    );
    add(
        "image.lens",
        "Lens",
        facts.lens_model.clone(),
        DetailGroup::Source,
        false,
    );
    add(
        "image.software",
        "Software",
        facts.software.clone(),
        DetailGroup::Source,
        false,
    );
    add(
        "image.gps",
        "Location",
        facts
            .gps
            .and_then(|(lat, lon)| coordinate_words(lat, lon, 2)),
        DetailGroup::Source,
        true,
    );
    if facts.gps.is_none() && facts.gps_folder {
        add(
            "image.gps",
            "Location data",
            Some("yes, without a place".to_owned()),
            DetailGroup::Source,
            true,
        );
    }
    add(
        "image.serial",
        "Camera serial",
        facts.body_serial.clone(),
        DetailGroup::Source,
        true,
    );
    add(
        "image.lens_serial",
        "Lens serial",
        facts.lens_serial.clone(),
        DetailGroup::Source,
        true,
    );
    add(
        "image.owner",
        "Camera owner",
        facts.owner.clone(),
        DetailGroup::Source,
        true,
    );
    add(
        "image.artist",
        "Author",
        facts.artist.clone(),
        DetailGroup::Source,
        true,
    );
    add(
        "image.copyright",
        "Copyright",
        facts.copyright.clone(),
        DetailGroup::Source,
        false,
    );
    let comment = facts
        .comment
        .clone()
        .or_else(|| parts.comments.first().cloned());
    add(
        "image.comment",
        "Comment",
        comment,
        DetailGroup::Content,
        true,
    );
    if facts.thumbnail {
        add(
            "image.thumbnail",
            "Stored thumbnail",
            Some("yes, maybe uncropped".to_owned()),
            DetailGroup::Technical,
            true,
        );
    }
    if parts.xmp {
        add(
            "image.xmp",
            "XMP data",
            Some("yes, may hold names, places".to_owned()),
            DetailGroup::Technical,
            true,
        );
    }
    if parts.iptc {
        add(
            "image.iptc",
            "IPTC data",
            Some("yes, may hold names, places".to_owned()),
            DetailGroup::Technical,
            true,
        );
    }
    if parts.more_pictures {
        add(
            "image.more_pictures",
            "More pictures inside",
            Some("yes".to_owned()),
            DetailGroup::Technical,
            true,
        );
    }
    fields
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use diroxide_core::{Entry, EntryKind, Place};

    use super::*;
    use crate::image::tests::jpeg;

    #[test]
    fn a_photo_says_when_with_what_and_what_is_private() {
        let mut block = exif::EXIF_PREFIX.to_vec();
        block.extend_from_slice(&exif::tests::private_block(false));
        let bytes = jpeg(&[(0xE1, block), (0xFE, b"from the beach".to_vec())]);
        let place = Place::parse(if cfg!(windows) { "C:\\p.jpg" } else { "/p.jpg" }).unwrap();
        let entry = Entry::new("IMG_2041.jpg", EntryKind::File);
        let request = ReadRequest {
            place: &place,
            entry: &entry,
            type_id: Some("jpeg"),
            group: Some("images"),
        };
        assert!(PhotoFacts.accepts(&request));
        let fields = PhotoFacts
            .read(&mut Cursor::new(bytes), &request, &ReadLimits::default())
            .unwrap();
        let lines: Vec<String> = fields
            .iter()
            .map(|f| format!("{}{}", f.line(), if f.private { " (private)" } else { "" }))
            .collect();
        assert_eq!(
            lines,
            [
                "Taken: 2001-01-01 08:30",
                "Camera: SpikeCam Spike 1",
                "Location: 45.50 north, 122.69 west (private)",
                "Camera serial: SN12345 (private)",
                "Author: Ada Example (private)",
                "Comment: a private note (private)",
                "Stored thumbnail: yes, maybe uncropped (private)",
            ]
        );
        for field in &fields {
            assert!(field.line().chars().count() <= 40, "{}", field.line());
        }
    }

    #[test]
    fn a_camera_that_names_its_maker_once() {
        let facts = ExifFacts {
            make: Some("Canon".to_owned()),
            model: Some("Canon EOS R6".to_owned()),
            ..ExifFacts::default()
        };
        assert_eq!(camera(&facts).as_deref(), Some("Canon EOS R6"));
    }
}
