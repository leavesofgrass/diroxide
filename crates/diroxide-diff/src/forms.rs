//! Unicode forms: whether two texts are the same letters written another
//! way, a composed é (U+00E9) against e and a combining acute accent
//! (U+0065 U+0301), which look and sound the same.
//!
//! No normalization crate is in the workspace, so this is canonical
//! decomposition (NFD) over a generated table for Latin, Greek and
//! Cyrillic, Hangul syllables by their formula, and the canonical order of
//! combining marks. Other scripts' composed forms are not recognized; a
//! line that differs only there reads as an ordinary change.

mod table;

use table::{COMBINING_CLASSES, DECOMPOSITIONS};

/// The canonical combining class of a mark; 0 for every other character.
pub(crate) fn combining_class(c: char) -> u8 {
    let code = u32::from(c);
    COMBINING_CLASSES
        .binary_search_by_key(&code, |&(mark, _)| mark)
        .map_or(0, |at| COMBINING_CLASSES[at].1)
}

/// Whether a character is a combining mark, which belongs with the
/// character before it.
pub(crate) fn is_mark(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036F | 0x0483..=0x0489 | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = 19 * N_COUNT;

fn push_decomposed(c: char, out: &mut Vec<char>) {
    let code = u32::from(c);
    if (S_BASE..S_BASE + S_COUNT).contains(&code) {
        let index = code - S_BASE;
        let parts = [
            Some(L_BASE + index / N_COUNT),
            Some(V_BASE + (index % N_COUNT) / T_COUNT),
            (!index.is_multiple_of(T_COUNT)).then_some(T_BASE + index % T_COUNT),
        ];
        out.extend(parts.into_iter().flatten().filter_map(char::from_u32));
        return;
    }
    match DECOMPOSITIONS.binary_search_by_key(&code, |&(composed, _)| composed) {
        Ok(at) => out.extend(
            DECOMPOSITIONS[at]
                .1
                .iter()
                .filter_map(|&p| char::from_u32(p)),
        ),
        Err(_) => out.push(c),
    }
}

/// The text in canonical decomposed form, as far as the table reaches:
/// composed letters taken apart, and each run of combining marks in
/// canonical order.
pub(crate) fn decomposed(text: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        push_decomposed(c, &mut out);
    }
    // Canonical order: a stable sort of each run of marks by class.
    let mut at = 0;
    while at < out.len() {
        if combining_class(out[at]) == 0 {
            at += 1;
            continue;
        }
        let start = at;
        while at < out.len() && combining_class(out[at]) != 0 {
            at += 1;
        }
        out[start..at].sort_by_key(|&c| combining_class(c));
    }
    out
}

/// Whether two texts that differ are the same text in another Unicode
/// form.
pub(crate) fn same_in_another_form(before: &str, after: &str) -> bool {
    before != after && decomposed(before) == decomposed(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_sorted_for_searching() {
        assert!(DECOMPOSITIONS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(COMBINING_CLASSES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn composed_and_decomposed_letters_are_the_same_text() {
        assert!(same_in_another_form("caf\u{e9}", "cafe\u{301}"));
        assert!(same_in_another_form(
            "\u{c5}ngstr\u{f6}m",
            "A\u{30a}ngstro\u{308}m"
        ));
        // The Angstrom sign is the letter Å.
        assert!(same_in_another_form("\u{212b}", "\u{c5}"));
        // Vietnamese: two marks, in either order of classes.
        assert!(same_in_another_form("\u{1ec7}", "e\u{302}\u{323}"));
        // Cyrillic short i.
        assert!(same_in_another_form("\u{439}", "\u{438}\u{306}"));
        // Hangul: a syllable and its letters.
        assert!(same_in_another_form("\u{d55c}", "\u{1112}\u{1161}\u{11ab}"));
    }

    #[test]
    fn different_letters_are_not_another_form() {
        assert!(!same_in_another_form("caf\u{e9}", "cafe\u{300}"));
        assert!(!same_in_another_form("cafe", "caf\u{e9}"));
        assert!(!same_in_another_form("same", "same"));
    }

    #[test]
    fn marks_are_known() {
        assert!(is_mark('\u{301}'));
        assert!(!is_mark('e'));
        assert_eq!(combining_class('\u{301}'), 230);
        assert_eq!(combining_class('\u{323}'), 220);
        assert_eq!(combining_class('a'), 0);
    }
}
