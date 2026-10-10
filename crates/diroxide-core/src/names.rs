//! Characters that cannot be seen or heard in a name, and the two safe ways
//! to show them: replaced by a question mark for display, or spelled out as
//! `U+XXXX` in the listing before a delete, move or overwrite.
//!
//! The real name is always kept for operations; these functions only make
//! text for people.

use std::borrow::Cow;

/// What kind of character this is, for display purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharClass {
    /// An ordinary character that shows and reads as itself.
    Normal,
    /// A control character (C0, DEL, C1) or a line or paragraph separator.
    Control,
    /// An invisible format character, such as a zero-width space or a
    /// byte-order mark.
    Format,
    /// A character that changes the direction of text, such as the
    /// right-to-left override.
    Direction,
    /// A private-use character, which shows as nothing or as a font's icon.
    PrivateUse,
}

impl CharClass {
    /// Whether characters of this class are hidden from sight or speech.
    pub const fn is_hidden(self) -> bool {
        !matches!(self, CharClass::Normal)
    }
}

/// The class of a character.
pub fn classify(c: char) -> CharClass {
    let code = u32::from(c);
    if c.is_control() || code == 0x2028 || code == 0x2029 {
        return CharClass::Control;
    }
    if is_direction(code) {
        return CharClass::Direction;
    }
    if is_format(code) {
        return CharClass::Format;
    }
    if (0xE000..=0xF8FF).contains(&code)
        || (0xF_0000..=0xF_FFFD).contains(&code)
        || (0x10_0000..=0x10_FFFD).contains(&code)
    {
        return CharClass::PrivateUse;
    }
    CharClass::Normal
}

fn is_direction(code: u32) -> bool {
    matches!(code, 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

fn is_format(code: u32) -> bool {
    matches!(
        code,
        0x00AD
            | 0x0600..=0x0605
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200D
            | 0x2060..=0x2064
            | 0x206A..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// Whether a name contains any hidden character.
pub fn has_hidden(name: &str) -> bool {
    name.chars().any(|c| classify(c).is_hidden())
}

/// The name as displayed: every hidden character replaced by a question
/// mark. Borrowed when nothing needs replacing.
pub fn display_name(name: &str) -> Cow<'_, str> {
    if !has_hidden(name) {
        return Cow::Borrowed(name);
    }
    Cow::Owned(
        name.chars()
            .map(|c| if classify(c).is_hidden() { '?' } else { c })
            .collect(),
    )
}

/// The name with every hidden character spelled out as `[U+XXXX]`, for the
/// listing a confirmation shows before anything is touched.
pub fn spell_out(name: &str) -> Cow<'_, str> {
    if !has_hidden(name) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len() + 8);
    for c in name.chars() {
        if classify(c).is_hidden() {
            out.push_str(&format!("[U+{:04X}]", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_are_borrowed() {
        assert!(matches!(display_name("report.pdf"), Cow::Borrowed(_)));
        assert!(matches!(spell_out("report.pdf"), Cow::Borrowed(_)));
    }

    #[test]
    fn hidden_characters_are_replaced_or_spelled_out() {
        // The September 26 name: "C" and a private-use character.
        let name = "C\u{F03A}";
        assert_eq!(display_name(name), "C?");
        assert_eq!(spell_out(name), "C[U+F03A]");
        assert_eq!(display_name("a\u{202E}fdp.exe"), "a?fdp.exe");
        assert_eq!(display_name("tab\there"), "tab?here");
        assert_eq!(display_name("zero\u{200B}width"), "zero?width");
        assert_eq!(spell_out("bom\u{FEFF}"), "bom[U+FEFF]");
    }

    #[test]
    fn classes_are_told_apart() {
        assert_eq!(classify('a'), CharClass::Normal);
        assert_eq!(classify('\u{e9}'), CharClass::Normal);
        assert_eq!(classify('\u{7}'), CharClass::Control);
        assert_eq!(classify('\u{2029}'), CharClass::Control);
        assert_eq!(classify('\u{200F}'), CharClass::Direction);
        assert_eq!(classify('\u{AD}'), CharClass::Format);
        assert_eq!(classify('\u{E000}'), CharClass::PrivateUse);
        assert_eq!(classify('\u{10FFFD}'), CharClass::PrivateUse);
    }
}
