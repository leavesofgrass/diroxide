//! The Exif facts the words use, through `kamadak-exif`: the orientation,
//! the author's description, the camera, and the two private facts, a
//! location and a camera's serial number. Every string is cleaned like a
//! file name before it is kept ([`clean_text`]).

use ::exif::{Exif, In, Tag, Value};

use super::ImageFacts;
use crate::words::clean_text;

/// Reads raw Exif (a TIFF-structured block) into the facts; a block that
/// does not parse adds nothing.
pub(super) fn apply(raw: Vec<u8>, facts: &mut ImageFacts) {
    if let Ok(exif) = ::exif::Reader::new().read_raw(raw) {
        apply_parsed(&exif, facts);
    }
}

/// Puts what a parsed Exif block says into the facts.
pub(super) fn apply_parsed(exif: &Exif, facts: &mut ImageFacts) {
    let field = |tag: Tag| exif.get_field(tag, In::PRIMARY).map(|f| &f.value);
    if let Some(value) = field(Tag::Orientation).and_then(|v| v.get_uint(0))
        && (1..=8).contains(&value)
    {
        facts.orientation = value as u8;
    }
    if let Some(text) = field(Tag::ImageDescription).and_then(ascii) {
        facts.description = Some(text);
    }
    let make = field(Tag::Make).and_then(ascii);
    let model = field(Tag::Model).and_then(ascii);
    facts.camera = match (make, model) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
            Some(model)
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (one, other) => one.or(other),
    };
    facts.camera_serial = field(Tag::BodySerialNumber).and_then(ascii);
    let latitude = field(Tag::GPSLatitude).and_then(degrees);
    let longitude = field(Tag::GPSLongitude).and_then(degrees);
    if let (Some(lat), Some(lon)) = (latitude, longitude) {
        let north = field(Tag::GPSLatitudeRef)
            .and_then(ascii)
            .is_none_or(|r| !r.eq_ignore_ascii_case("S"));
        let east = field(Tag::GPSLongitudeRef)
            .and_then(ascii)
            .is_none_or(|r| !r.eq_ignore_ascii_case("W"));
        facts.location = Some(format!(
            "{lat:.2} {}, {lon:.2} {}",
            if north { "north" } else { "south" },
            if east { "east" } else { "west" }
        ));
    }
    if facts.width == 0 || facts.height == 0 {
        let w = field(Tag::PixelXDimension).and_then(|v| v.get_uint(0));
        let h = field(Tag::PixelYDimension).and_then(|v| v.get_uint(0));
        if let (Some(w), Some(h)) = (w, h) {
            facts.width = w;
            facts.height = h;
        }
    }
}

/// An ASCII value as cleaned text, `None` when it is blank.
fn ascii(value: &Value) -> Option<String> {
    let Value::Ascii(parts) = value else {
        return None;
    };
    let joined: Vec<u8> = parts.iter().flatten().copied().collect();
    let text = clean_text(&String::from_utf8_lossy(&joined), super::MAX_TEXT);
    (!text.is_empty()).then_some(text)
}

/// Degrees, minutes and seconds as decimal degrees.
fn degrees(value: &Value) -> Option<f64> {
    let Value::Rational(parts) = value else {
        return None;
    };
    let part = |n: usize| {
        parts
            .get(n)
            .filter(|r| r.denom != 0)
            .map_or(0.0, |r| r.to_f64())
    };
    if parts.is_empty() {
        return None;
    }
    let total = part(0) + part(1) / 60.0 + part(2) / 3600.0;
    total.is_finite().then_some(total)
}
