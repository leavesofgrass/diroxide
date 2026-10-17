//! Member names: the rules of research/archives.md group D, run for every
//! member in one place, so a name such as `../../x` or `\\?\C:\x` can never
//! leave the archive, and every name that needs care says why in words.
//!
//! Two doors. [`normalize_member_name`] is the strict one of the contract:
//! it refuses a name with a control character outright. [`member_path`] is
//! the one the index uses: it lists such a name under a safe stand-in (each
//! control character becomes U+FFFD) and flags it, so nothing in an archive
//! is silently lost, and it applies the length limits. Both give a path
//! with forward slashes, no leading slash, no drive or device prefix, no
//! `.`, no `..` and no empty part, and both are stable: a result given to
//! them again comes back unchanged.

use std::fmt;

use diroxide_core::names::classify;

use crate::limits::Limits;

/// What the name rules found about a member's name. Each flag has its
/// words ([`NameFlags::words`]), which a row says after "name needs care".
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NameFlags(u16);

impl NameFlags {
    /// No flags.
    pub const NONE: Self = Self(0);
    /// A leading slash was removed: "leading slash removed".
    pub const LEADING_SLASH: Self = Self(1);
    /// A drive letter was removed, at the start or in the middle: "drive
    /// removed".
    pub const DRIVE: Self = Self(1 << 1);
    /// A long-path or device prefix (`\\?\`, `\\.\`, `\??\`) was removed:
    /// "long-path prefix removed".
    pub const DEVICE_PREFIX: Self = Self(1 << 2);
    /// Backslashes were read as folder separators: "backslashes read as
    /// folders".
    pub const BACKSLASH: Self = Self(1 << 3);
    /// Control characters were replaced by U+FFFD: "contains a control
    /// character".
    pub const CONTROL: Self = Self(1 << 4);
    /// Format, direction or private-use characters: "has hidden
    /// characters".
    pub const HIDDEN: Self = Self(1 << 5);
    /// A `!` after an archive extension, which would read back as a nested
    /// archive: "contains an archive marker".
    pub const ARCHIVE_MARKER: Self = Self(1 << 6);
    /// The name was not valid text and is shown with stand-ins: "name is
    /// not valid text".
    pub const NOT_UNICODE: Self = Self(1 << 7);
    /// A part looks like a Windows short name (`PROGRA~1`), which can
    /// alias another folder: "looks like a short name".
    pub const SHORT_NAME: Self = Self(1 << 8);

    const ALL: [(Self, &'static str); 9] = [
        (Self::CONTROL, "contains a control character"),
        (Self::NOT_UNICODE, "name is not valid text"),
        (Self::HIDDEN, "has hidden characters"),
        (Self::ARCHIVE_MARKER, "contains an archive marker"),
        (Self::DRIVE, "drive removed"),
        (Self::DEVICE_PREFIX, "long-path prefix removed"),
        (Self::LEADING_SLASH, "leading slash removed"),
        (Self::BACKSLASH, "backslashes read as folders"),
        (Self::SHORT_NAME, "looks like a short name"),
    ];

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Both sets of flags.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Sets the flags of `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// The flags in words, the most serious first: "contains a control
    /// character", "drive removed".
    pub fn words(self) -> Vec<&'static str> {
        // A long-path prefix or a drive says the rest: the slash before it
        // and the backslashes in it go without saying.
        let mut flags = self;
        if flags.contains(Self::DEVICE_PREFIX) || flags.contains(Self::DRIVE) {
            flags.0 &= !(Self::LEADING_SLASH.0 | Self::BACKSLASH.0);
        }
        Self::ALL
            .iter()
            .filter(|(flag, _)| flags.contains(*flag))
            .map(|(_, words)| *words)
            .collect()
    }

    /// The raw bits, for the index's compact records.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Flags from raw bits.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }
}

impl fmt::Debug for NameFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.words()).finish()
    }
}

/// Why a member's name is refused: it is left out of the listing, counted,
/// and said in the result ("Skipped ../x: name leaves the archive.").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameRefusal {
    /// `..` climbs out of the archive.
    LeavesArchive,
    /// Nothing is left once the prefixes, `.` and empty parts are removed.
    Empty,
    /// A part is made only of dots and spaces, which Windows reads as `.`
    /// or `..`.
    DotsOnly,
    /// The strict door's refusal: a control character.
    Control,
    /// The whole name is longer than the limit, in bytes.
    TooLong {
        /// The limit.
        max: usize,
    },
    /// One part is longer than the limit, in bytes.
    PartTooLong {
        /// The limit.
        max: usize,
    },
}

impl NameRefusal {
    /// The reason in words, for "Skipped NAME: REASON.": "name leaves the
    /// archive", "name part over 255 bytes".
    pub fn reason(self) -> String {
        match self {
            NameRefusal::LeavesArchive => "name leaves the archive".to_owned(),
            NameRefusal::Empty => "name is empty once made safe".to_owned(),
            NameRefusal::DotsOnly => "name part is only dots and spaces".to_owned(),
            NameRefusal::Control => "name contains a control character".to_owned(),
            NameRefusal::TooLong { max } => {
                format!(
                    "name over {} bytes",
                    diroxide_core::format::grouped(max as u64)
                )
            }
            NameRefusal::PartTooLong { max } => {
                format!(
                    "name part over {} bytes",
                    diroxide_core::format::grouped(max as u64)
                )
            }
        }
    }

    /// The skip line, the name shown with hidden characters as question
    /// marks: "Skipped ../x: name leaves the archive."
    pub fn line(self, raw: &str) -> String {
        let name = diroxide_core::names::display_name(raw);
        if name.chars().count() > 20 {
            // A long name would push the reason past the first 40 cells.
            format!("Skipped, {}: {name}.", self.reason())
        } else {
            format!("Skipped {name}: {}.", self.reason())
        }
    }
}

/// A member's path made safe, with what was done to it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MemberPath {
    /// The path: forward slashes, no leading slash, no drive, no `.` or
    /// `..`, no empty part, no control character.
    pub path: String,
    /// What the rules found.
    pub flags: NameFlags,
}

/// The lenient door: a member's name made safe for the index, with flags,
/// or the reason it is refused. Control characters become U+FFFD and are
/// flagged rather than refused; `limits` bounds the name and its parts.
pub fn member_path(name: &str, limits: &Limits) -> Result<MemberPath, NameRefusal> {
    let mut flags = NameFlags::NONE;
    let mut text = String::with_capacity(name.len());
    for c in name.chars() {
        if classify(c) == diroxide_core::names::CharClass::Control {
            flags.insert(NameFlags::CONTROL);
            text.push('\u{FFFD}');
        } else {
            if c == '\u{FFFD}' {
                flags.insert(NameFlags::NOT_UNICODE);
            }
            text.push(c);
        }
    }
    if text.contains('\\') {
        flags.insert(NameFlags::BACKSLASH);
    }
    let unified = text.replace('\\', "/");
    let path = clean_path(&unified, &mut flags)?;
    if path.len() > limits.max_name_bytes {
        return Err(NameRefusal::TooLong {
            max: limits.max_name_bytes,
        });
    }
    if path
        .split('/')
        .any(|part| part.len() > limits.max_component_bytes)
    {
        return Err(NameRefusal::PartTooLong {
            max: limits.max_component_bytes,
        });
    }
    for part in path.split('/') {
        if has_archive_marker(part) {
            flags.insert(NameFlags::ARCHIVE_MARKER);
        }
        if looks_like_short_name(part) {
            flags.insert(NameFlags::SHORT_NAME);
        }
    }
    if path.chars().any(|c| classify(c).is_hidden()) {
        flags.insert(NameFlags::HIDDEN);
    }
    Ok(MemberPath { path, flags })
}

/// A member name made safe, by research/archives.md group D: refused
/// (`None`) when it holds NUL or another control character, climbs out of
/// the archive with `..`, has a part made only of dots and spaces, or
/// normalizes to nothing; otherwise with forward slashes, no long-path or
/// device prefix (`\\?\`, `\\.\`, `\??\`, however many), no leading
/// slash, no drive letter anywhere in the path, and no `.` or empty parts.
/// Normalizing a result again gives it back unchanged. The destination's
/// own name check comes after, at unpack.
pub fn normalize_member_name(name: &str) -> Option<String> {
    if name
        .chars()
        .any(|c| classify(c) == diroxide_core::names::CharClass::Control)
    {
        return None;
    }
    let unified = name.replace('\\', "/");
    let mut flags = NameFlags::NONE;
    clean_path(&unified, &mut flags).ok()
}

/// The shared heart: prefixes, drives, `.`, `..` and dot-only parts.
fn clean_path(unified: &str, flags: &mut NameFlags) -> Result<String, NameRefusal> {
    let mut parts: Vec<&str> = Vec::new();
    // Leading separators and the device markers they introduce: `//?/`,
    // `//./`, `/??/`, with `UNC` after the marker. Repeated as often as
    // they appear, so a doubled prefix is no way round.
    let mut rest = unified;
    loop {
        let trimmed = rest.trim_start_matches('/');
        if trimmed.len() != rest.len() {
            flags.insert(NameFlags::LEADING_SLASH);
        }
        let slashes = rest.len() - trimmed.len();
        let mut marker = None;
        for (prefix, needs) in [("??/", 1), ("?/", 2), ("./", 2)] {
            if slashes >= needs
                && let Some(after) = trimmed.strip_prefix(prefix)
            {
                marker = Some(after);
                break;
            }
        }
        match marker {
            Some(after) => {
                flags.insert(NameFlags::DEVICE_PREFIX);
                rest = after;
                for unc in ["UNC/", "unc/", "Unc/"] {
                    if let Some(after) = rest.strip_prefix(unc) {
                        rest = after;
                        break;
                    }
                }
            }
            None => {
                rest = trimmed;
                break;
            }
        }
    }
    for part in rest.split('/') {
        let mut part = part;
        while let [letter, b':', ..] = part.as_bytes() {
            if !letter.is_ascii_alphabetic() {
                break;
            }
            flags.insert(NameFlags::DRIVE);
            part = &part[2..];
        }
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(NameRefusal::LeavesArchive);
                }
            }
            other if other.chars().all(|c| c == '.' || c == ' ') => {
                return Err(NameRefusal::DotsOnly);
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(NameRefusal::Empty);
    }
    Ok(parts.join("/"))
}

/// Whether a name holds a `!` after an archive extension, which a place
/// would read back as a nested archive: core's own rule,
/// [`diroxide_core::place::holds_archive_marker`], so the provider flags
/// exactly the names [`Place::join`](diroxide_core::Place::join) refuses.
pub fn has_archive_marker(name: &str) -> bool {
    diroxide_core::place::holds_archive_marker(name)
}

/// Whether a part looks like a Windows short name: up to six characters, a
/// tilde, digits, and an extension of up to three characters
/// (`PROGRA~1`, `LONGNA~2.TXT`).
pub fn looks_like_short_name(part: &str) -> bool {
    let (stem, extension) = match part.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (part, None),
    };
    let Some((base, digits)) = stem.rsplit_once('~') else {
        return false;
    };
    !base.is_empty()
        && base.chars().count() <= 6
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && extension.is_none_or(|e| e.chars().count() <= 3)
}

/// Whether a member is junk that other systems leave in archives and that
/// is hidden from listings: `__MACOSX` and everything in it, `.DS_Store`,
/// `Thumbs.db`, `desktop.ini`. Hidden, never dropped: the hidden-files
/// toggle shows it.
pub fn is_junk(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    let last = normalized.rsplit('/').find(|p| !p.is_empty()).unwrap_or("");
    normalized.split('/').any(|part| part == "__MACOSX")
        || matches!(
            last.to_lowercase().as_str(),
            ".ds_store" | "thumbs.db" | "desktop.ini"
        )
}

/// The key two names clash under on some drives: lowercase, with the
/// common precomposed Latin letters taken apart and combining accents
/// dropped, so `README` meets `readme` and a composed `café` meets a
/// decomposed one. An approximation of Unicode case folding and
/// normalization that needs no tables crate; it errs toward flagging.
pub fn clash_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        if ('\u{300}'..='\u{36F}').contains(&c) {
            continue;
        }
        key.push(base_letter(c));
    }
    key
}

/// The base letter of a precomposed Latin letter (lowercase), or the
/// character itself.
fn base_letter(c: char) -> char {
    match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

/// How many archives deep a place is: 0 for a plain place, 1 inside
/// `a.zip!`, 2 inside `a.zip!b.zip!`.
pub fn nesting_depth(place: &diroxide_core::Place) -> usize {
    let mut depth = 0;
    let mut current = place;
    while let Some((container, _)) = current.archive() {
        depth += 1;
        current = container;
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn lenient(name: &str) -> Result<MemberPath, NameRefusal> {
        member_path(name, &Limits::default())
    }

    #[test]
    fn the_lenient_door_lists_and_flags() {
        let path = lenient("a\u{0}b.txt").unwrap();
        assert_eq!(path.path, "a\u{FFFD}b.txt");
        assert!(path.flags.contains(NameFlags::CONTROL));
        let path = lenient(r"\\?\C:\x").unwrap();
        assert_eq!(path.path, "x");
        assert!(path.flags.contains(NameFlags::DRIVE));
        assert!(path.flags.contains(NameFlags::DEVICE_PREFIX));
        assert_eq!(
            path.flags.words(),
            ["drive removed", "long-path prefix removed"]
        );
        assert_eq!(
            lenient("/abs.txt").unwrap().flags.words(),
            ["leading slash removed"]
        );
        assert!(
            lenient("inner.zip!x.txt")
                .unwrap()
                .flags
                .contains(NameFlags::ARCHIVE_MARKER)
        );
        assert!(
            lenient("photo\u{202e}gpj.exe")
                .unwrap()
                .flags
                .contains(NameFlags::HIDDEN)
        );
        assert!(
            lenient("PROGRA~1/x")
                .unwrap()
                .flags
                .contains(NameFlags::SHORT_NAME)
        );
        assert!(lenient("ok.txt").unwrap().flags.is_empty());
    }

    #[test]
    fn refusals_say_why() {
        assert_eq!(lenient("../x"), Err(NameRefusal::LeavesArchive));
        assert_eq!(
            NameRefusal::LeavesArchive.line("../x"),
            "Skipped ../x: name leaves the archive."
        );
        let long = format!("{}.txt", "n".repeat(300));
        let refused = lenient(&long).unwrap_err();
        assert_eq!(refused.reason(), "name part over 255 bytes");
        assert!(
            refused
                .line(&long)
                .starts_with("Skipped, name part over 255 bytes: nnn")
        );
        let limits = Limits {
            max_name_bytes: 10,
            ..Limits::default()
        };
        assert_eq!(
            member_path("a/b/c/d/e", &limits).map(|p| p.path),
            Ok("a/b/c/d/e".to_owned())
        );
        assert_eq!(
            member_path("abcdef/ghijkl", &limits).unwrap_err().reason(),
            "name over 10 bytes"
        );
        assert_eq!(lenient(". ."), Err(NameRefusal::DotsOnly));
        assert_eq!(lenient("C:"), Err(NameRefusal::Empty));
    }

    #[test]
    fn short_names_and_clash_keys() {
        assert!(looks_like_short_name("PROGRA~1"));
        assert!(looks_like_short_name("LONGNA~2.TXT"));
        assert!(!looks_like_short_name("notes~draft.txt"));
        assert!(!looks_like_short_name("~1"));
        assert_eq!(clash_key("README"), clash_key("readme"));
        assert_eq!(clash_key("caf\u{e9}.txt"), clash_key("cafe\u{301}.txt"));
        assert_ne!(clash_key("cafe.txt"), clash_key("cafes.txt"));
    }

    #[test]
    fn junk_includes_the_macos_folder_itself() {
        assert!(is_junk("__MACOSX"));
        assert!(is_junk("photos/__MACOSX/._a"));
        assert!(!is_junk("macosx.txt"));
    }

    fn hostile_text() -> impl Strategy<Value = String> {
        let pieces = prop_oneof![
            Just("a".to_owned()),
            Just("b".to_owned()),
            Just(".".to_owned()),
            Just("..".to_owned()),
            Just("/".to_owned()),
            Just("\\".to_owned()),
            Just(":".to_owned()),
            Just("C:".to_owned()),
            Just("?".to_owned()),
            Just("??".to_owned()),
            Just("UNC".to_owned()),
            Just(" ".to_owned()),
            Just("\u{0}".to_owned()),
            Just("\t".to_owned()),
            Just("\u{85}".to_owned()),
            Just("\u{202e}".to_owned()),
            Just("\u{e9}".to_owned()),
            Just(".zip!".to_owned()),
            Just("~1".to_owned()),
            any::<char>().prop_map(String::from),
        ];
        proptest::collection::vec(pieces, 0..24).prop_map(|v| v.concat())
    }

    /// The properties of a safe path: no `..`, no drive, no NUL or control
    /// character, no leading slash, no backslash, no empty or dot part.
    fn assert_safe(path: &str) {
        assert!(!path.starts_with('/'), "{path:?}");
        assert!(!path.contains('\\'), "{path:?}");
        assert!(!path.chars().any(char::is_control), "{path:?}");
        for part in path.split('/') {
            assert!(!part.is_empty() && part != "." && part != "..", "{path:?}");
            assert!(!part.chars().all(|c| c == '.' || c == ' '), "{path:?}");
            let bytes = part.as_bytes();
            assert!(
                !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'),
                "a drive in {path:?}"
            );
        }
    }

    proptest! {
        #[test]
        fn normalizing_twice_is_stable_and_safe(name in hostile_text()) {
            if let Some(once) = normalize_member_name(&name) {
                assert_safe(&once);
                prop_assert_eq!(normalize_member_name(&once), Some(once.clone()));
            }
            if let Ok(once) = member_path(&name, &Limits::default()) {
                assert_safe(&once.path);
                let twice = member_path(&once.path, &Limits::default()).unwrap();
                prop_assert_eq!(&twice.path, &once.path);
            }
        }
    }
}
