//! Words for numbers and sizes, and the 40-cell Braille line.
//!
//! Every row, label and message carries its meaning in its first
//! [`BRAILLE_CELLS`] characters, because the owner's display shows one line
//! of 40 cells at a time. [`within_cells`] is the test for that rule.

/// The cells on the owner's Braille display: one line at a time.
pub const BRAILLE_CELLS: usize = 40;

/// A count with its noun: "1 item", "40 items", "no items".
pub fn count(n: u64, singular: &str, plural: &str) -> String {
    match n {
        0 => format!("no {plural}"),
        1 => format!("1 {singular}"),
        _ => format!("{n} {plural}"),
    }
}

/// A size in words, with decimal units: "1 byte", "512 bytes", "2.1 MB",
/// "153 MB". One decimal below 100 of a unit, none above.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return count(bytes, "byte", "bytes").replace("no bytes", "0 bytes");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value < 99.95 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// The first `cells` characters of a line, as one line of a Braille display
/// shows it.
pub fn first_cells(text: &str, cells: usize) -> &str {
    match text.char_indices().nth(cells) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// Whether `needle` appears whole within the first [`BRAILLE_CELLS`]
/// characters of `text`: the test that a row or message says its meaning
/// first.
pub fn within_cells(text: &str, needle: &str) -> bool {
    first_cells(text, BRAILLE_CELLS).contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_naturally() {
        assert_eq!(count(0, "item", "items"), "no items");
        assert_eq!(count(1, "item", "items"), "1 item");
        assert_eq!(count(40, "item", "items"), "40 items");
    }

    #[test]
    fn sizes_use_words_and_decimal_units() {
        assert_eq!(human_size(0), "0 bytes");
        assert_eq!(human_size(1), "1 byte");
        assert_eq!(human_size(999), "999 bytes");
        assert_eq!(human_size(1000), "1.0 KB");
        assert_eq!(human_size(2_100_000), "2.1 MB");
        assert_eq!(human_size(153_000_000), "153 MB");
        assert_eq!(human_size(999_960), "1.0 MB");
        assert_eq!(human_size(u64::MAX), "18.4 EB");
    }

    #[test]
    fn the_first_cells_count_characters_not_bytes() {
        let text = "\u{e9}".repeat(50);
        assert_eq!(first_cells(&text, BRAILLE_CELLS).chars().count(), 40);
        assert!(within_cells("report.pdf, PDF, 2.1 MB", "report.pdf"));
        let late = format!("{}needle", "x".repeat(40));
        assert!(!within_cells(&late, "needle"));
    }
}
