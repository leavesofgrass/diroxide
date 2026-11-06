//! Values in words, shared by the readers: lengths of time, rates,
//! channels, dates and places, each as the details view says it.

use std::time::Duration;

pub use diroxide_core::format::{count, grouped, human_size};

/// A length of time in words, largest unit first, zero parts left out:
/// "3 min 25 sec", "1 hr 2 min", "under 1 sec".
pub fn duration_words(length: Duration) -> String {
    let total = length.as_secs();
    if total == 0 {
        return "under 1 sec".to_owned();
    }
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours} hr"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes} min"));
    }
    if seconds > 0 {
        parts.push(format!("{seconds} sec"));
    }
    parts.join(" ")
}

/// A bit rate in kilobits a second: "128 kbps".
pub fn bitrate_words(kbps: u32) -> String {
    format!("{kbps} kbps")
}

/// A sample rate: "44.1 kHz", "48 kHz", "8 kHz".
pub fn sample_rate_words(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        format!("{} kHz", hz / 1000)
    } else {
        format!("{:.1} kHz", f64::from(hz) / 1000.0)
    }
}

/// Channels in words: "mono", "stereo", "6 channels".
pub fn channels_words(channels: u8) -> String {
    match channels {
        1 => "mono".to_owned(),
        2 => "stereo".to_owned(),
        n => format!("{n} channels"),
    }
}

/// A date and time as the readers say it, sortable as text: the parts
/// given, `2026-11-06 08:00` or `2026-11-06`. Each part is checked; a
/// part out of range makes the whole `None`.
pub fn date_words(year: u32, month: u32, day: u32, time: Option<(u32, u32)>) -> Option<String> {
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    match time {
        Some((hour, minute)) if hour < 24 && minute < 60 => Some(format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}"
        )),
        Some(_) => None,
        None => Some(format!("{year:04}-{month:02}-{day:02}")),
    }
}

/// The digits of `text` from `at`, `len` of them, as a number.
fn digits(text: &str, at: usize, len: usize) -> Option<u32> {
    let part = text.get(at..at + len)?;
    if part.bytes().all(|b| b.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

/// A PDF date (`D:YYYYMMDDHHmmSSOHH'mm'`, every part after the year
/// optional) in words: "2026-11-06 08:00". The zone is left as written
/// in the file and not said.
pub fn pdf_date_words(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix("D:").unwrap_or(text);
    let year = digits(text, 0, 4)?;
    let month = digits(text, 4, 2).unwrap_or(1);
    let day = digits(text, 6, 2).unwrap_or(1);
    let time = match (digits(text, 8, 2), digits(text, 10, 2)) {
        (Some(hour), Some(minute)) => Some((hour, minute)),
        (Some(hour), None) => Some((hour, 0)),
        _ => None,
    };
    date_words(year, month, day, time)
}

/// An ISO 8601 date or date and time (`2026-11-06T08:00:00Z`, as Office
/// and EPUB write them), or Exif's `2026:11:06 08:00:00`, in words.
pub fn iso_date_words(text: &str) -> Option<String> {
    let text = text.trim();
    let year = digits(text, 0, 4)?;
    let sep = text.as_bytes().get(4).copied();
    if !matches!(sep, Some(b'-' | b':')) {
        return if text.len() == 4 {
            date_words(year, 1, 1, None).map(|_| format!("{year:04}"))
        } else {
            None
        };
    }
    let month = digits(text, 5, 2)?;
    let day = digits(text, 8, 2)?;
    let time = match text.as_bytes().get(10) {
        Some(b'T' | b' ') => Some((digits(text, 11, 2)?, digits(text, 14, 2)?)),
        _ => None,
    };
    date_words(year, month, day, time)
}

/// A place on the earth in words, `decimals` after the point (two by
/// default, about a kilometer: R28's location risk), with north, south,
/// east and west: "45.50 north, 122.69 west".
pub fn coordinate_words(latitude: f64, longitude: f64, decimals: usize) -> Option<String> {
    if !latitude.is_finite()
        || !longitude.is_finite()
        || latitude.abs() > 90.0
        || longitude.abs() > 180.0
    {
        return None;
    }
    let ns = if latitude < 0.0 { "south" } else { "north" };
    let ew = if longitude < 0.0 { "west" } else { "east" };
    Some(format!(
        "{:.decimals$} {ns}, {:.decimals$} {ew}",
        latitude.abs(),
        longitude.abs()
    ))
}

/// Yes or no, as a value.
pub fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_rates_and_channels() {
        assert_eq!(duration_words(Duration::from_secs(205)), "3 min 25 sec");
        assert_eq!(duration_words(Duration::from_secs(3720)), "1 hr 2 min");
        assert_eq!(
            duration_words(Duration::from_secs(18750)),
            "5 hr 12 min 30 sec"
        );
        assert_eq!(duration_words(Duration::from_millis(400)), "under 1 sec");
        assert_eq!(bitrate_words(128), "128 kbps");
        assert_eq!(sample_rate_words(44_100), "44.1 kHz");
        assert_eq!(sample_rate_words(48_000), "48 kHz");
        assert_eq!(channels_words(1), "mono");
        assert_eq!(channels_words(2), "stereo");
        assert_eq!(channels_words(6), "6 channels");
    }

    #[test]
    fn dates_in_every_written_form() {
        assert_eq!(
            pdf_date_words("D:20261106080000-08'00'").as_deref(),
            Some("2026-11-06 08:00")
        );
        assert_eq!(pdf_date_words("D:2026").as_deref(), Some("2026-01-01"));
        assert_eq!(pdf_date_words("D:20261399").as_deref(), None);
        assert_eq!(pdf_date_words("yesterday"), None);
        assert_eq!(
            iso_date_words("2026-11-06T08:00:00Z").as_deref(),
            Some("2026-11-06 08:00")
        );
        assert_eq!(
            iso_date_words("2001:01:01 00:00:00").as_deref(),
            Some("2001-01-01 00:00")
        );
        assert_eq!(iso_date_words("2026-11-06").as_deref(), Some("2026-11-06"));
        assert_eq!(iso_date_words("2026").as_deref(), Some("2026"));
        assert_eq!(iso_date_words("0000:00:00 00:00:00"), None);
        assert_eq!(iso_date_words("2026-1"), None);
    }

    #[test]
    fn places_two_decimals_and_compass_words() {
        let line = format!(
            "Location: {}",
            coordinate_words(45.4986, -122.6853, 2).unwrap()
        );
        assert_eq!(line, "Location: 45.50 north, 122.69 west");
        assert!(line.chars().count() <= 40);
        assert_eq!(
            coordinate_words(-33.8688, 151.2093, 4).as_deref(),
            Some("33.8688 south, 151.2093 east")
        );
        assert_eq!(coordinate_words(91.0, 0.0, 2), None);
        assert_eq!(coordinate_words(f64::NAN, 0.0, 2), None);
    }
}
