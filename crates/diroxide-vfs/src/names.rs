//! The name checks: names that are dangerous to type, flagged on creation
//! and rename and shown on rows as "name needs care", and names a place
//! would change or cannot hold, which are refused (plan section 5.12;
//! research/file-operations.md and bulk-rename.md).
//!
//! Wave 1 (W1a) owns this module, adds the property tests and the fuzz
//! target, and settles which shell characters to flag.

use diroxide_core::names::{COMPOUND_EXTENSIONS, has_hidden, split_extension_with};

use crate::capabilities::Capabilities;

/// Characters Windows does not allow in names, plus the separators.
pub const RESERVED_CHARACTERS: [char; 9] = [':', '*', '?', '"', '<', '>', '|', '\\', '/'];

/// Characters a shell expands, so a name with them changes when typed at a
/// prompt.
pub const SHELL_SPECIAL: [char; 2] = ['$', '`'];

/// Windows device names, reserved with or without an extension, the
/// superscript digits included, as Microsoft's naming page lists them.
pub const DEVICE_NAMES: [&str; 28] = [
    "con",
    "prn",
    "aux",
    "nul",
    "com1",
    "com2",
    "com3",
    "com4",
    "com5",
    "com6",
    "com7",
    "com8",
    "com9",
    "com\u{B9}",
    "com\u{B2}",
    "com\u{B3}",
    "lpt1",
    "lpt2",
    "lpt3",
    "lpt4",
    "lpt5",
    "lpt6",
    "lpt7",
    "lpt8",
    "lpt9",
    "lpt\u{B9}",
    "lpt\u{B2}",
    "lpt\u{B3}",
];

/// The start of an unfinished file's name: a committing write's bytes, or
/// a file a hot folder must wait for. Shared so every part of the program
/// knows one.
pub const UNFINISHED_PREFIX: &str = ".diro-part-";

/// An unfinished file's name: the prefix and eight hexadecimal digits.
pub fn unfinished_name(token: u32) -> String {
    format!("{UNFINISHED_PREFIX}{token:08x}")
}

/// Whether a name is an unfinished file's: listings and watches drop it.
pub fn is_unfinished(name: &str) -> bool {
    name.strip_prefix(UNFINISHED_PREFIX)
        .is_some_and(|rest| rest.len() == 8 && rest.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The rules a place's names follow, from its capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameRules {
    /// Windows's rules apply: reserved characters, device names, and no
    /// trailing dot or space, which Windows would silently drop.
    pub windows: bool,
    /// The longest name.
    pub max_length: Option<usize>,
    /// Whether the length counts UTF-16 units (Windows) or bytes.
    pub length_in_utf16: bool,
}

impl NameRules {
    /// The rules of a place, from its capabilities.
    pub fn from_capabilities(capabilities: &Capabilities) -> Self {
        Self {
            windows: capabilities.reserved_names,
            max_length: capabilities.max_name_length,
            length_in_utf16: capabilities.reserved_names,
        }
    }

    /// The rules of this platform's own disks.
    pub const fn native() -> Self {
        Self {
            windows: cfg!(windows),
            max_length: Some(255),
            length_in_utf16: cfg!(windows),
        }
    }
}

/// Something about a name that needs care, or that the place refuses.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameIssue {
    /// The name is empty.
    Empty,
    /// The name is `.` or `..`.
    DotName,
    /// The name has control, format, direction or private-use characters.
    HiddenCharacters,
    /// The name has a character Windows reserves.
    ReservedCharacter(char),
    /// The name ends with a dot or a space, which Windows drops.
    TrailingDotOrSpace,
    /// The name starts with a space.
    LeadingSpace,
    /// The name is a Windows device name, such as `NUL` or `con.txt`.
    DeviceName,
    /// The name has a character a shell expands.
    ShellSpecial(char),
    /// The name differs only by case from a sibling's.
    CaseTwin(String),
    /// The name is longer than the place allows.
    TooLong {
        /// The longest the place allows.
        max: usize,
    },
}

impl NameIssue {
    /// The issue in words, meaning first and short: "ends in a dot or
    /// space".
    pub fn describe(&self) -> String {
        match self {
            NameIssue::Empty => "is empty".to_owned(),
            NameIssue::DotName => "is a dot name".to_owned(),
            NameIssue::HiddenCharacters => "has hidden characters".to_owned(),
            NameIssue::ReservedCharacter(c) => format!("reserved character {c}"),
            NameIssue::TrailingDotOrSpace => "ends in a dot or space".to_owned(),
            NameIssue::LeadingSpace => "starts with a space".to_owned(),
            NameIssue::DeviceName => "is a Windows device".to_owned(),
            NameIssue::ShellSpecial(c) => format!("has shell character {c}"),
            NameIssue::CaseTwin(other) => format!("differs only by case from {other}"),
            NameIssue::TooLong { max } => format!("is over {max} characters"),
        }
    }

    /// The row's or the message's words, meaning first and inside 40 cells
    /// for every issue but a long sibling name: "Name needs care: ends in
    /// a dot or space".
    pub fn line(&self) -> String {
        format!("Name needs care: {}", self.describe())
    }

    /// Whether a place with these rules refuses the name, rather than
    /// only flagging it: empty and dot names everywhere, names too long,
    /// and on a Windows-rules place the names Windows would change or
    /// reject (reserved characters, a trailing dot or space, device
    /// names). Hidden characters, shell characters, a leading space and a
    /// case twin are flagged, never refused.
    pub fn refused(&self, rules: &NameRules) -> bool {
        match self {
            NameIssue::Empty | NameIssue::DotName | NameIssue::TooLong { .. } => true,
            NameIssue::ReservedCharacter(c) => rules.windows || *c == '/',
            NameIssue::TrailingDotOrSpace | NameIssue::DeviceName => rules.windows,
            NameIssue::HiddenCharacters
            | NameIssue::LeadingSpace
            | NameIssue::ShellSpecial(_)
            | NameIssue::CaseTwin(_) => false,
        }
    }
}

/// Every issue with a name, given the names already in its folder, with
/// the platform's own rules.
pub fn check_name(name: &str, siblings: &[&str]) -> Vec<NameIssue> {
    check_name_with(name, siblings, &NameRules::native())
}

/// Every issue with a name under a place's rules. The device-name and
/// trailing checks run everywhere, since a name made on Linux may travel
/// to Windows; whether each is refused or only flagged is
/// [`NameIssue::refused`]'s answer.
pub fn check_name_with(name: &str, siblings: &[&str], rules: &NameRules) -> Vec<NameIssue> {
    let mut issues = Vec::new();
    if name.is_empty() {
        issues.push(NameIssue::Empty);
        return issues;
    }
    if name == "." || name == ".." {
        issues.push(NameIssue::DotName);
        return issues;
    }
    if has_hidden(name) {
        issues.push(NameIssue::HiddenCharacters);
    }
    if let Some(c) = name.chars().find(|c| RESERVED_CHARACTERS.contains(c)) {
        issues.push(NameIssue::ReservedCharacter(c));
    }
    if name.ends_with(['.', ' ']) {
        issues.push(NameIssue::TrailingDotOrSpace);
    }
    if name.starts_with(' ') {
        issues.push(NameIssue::LeadingSpace);
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_lowercase();
    if DEVICE_NAMES.contains(&stem.as_str()) {
        issues.push(NameIssue::DeviceName);
    }
    if let Some(c) = name.chars().find(|c| SHELL_SPECIAL.contains(c)) {
        issues.push(NameIssue::ShellSpecial(c));
    }
    if let Some(max) = rules.max_length {
        let length = if rules.length_in_utf16 {
            name.encode_utf16().count()
        } else {
            name.len()
        };
        if length > max {
            issues.push(NameIssue::TooLong { max });
        }
    }
    let lower = name.to_lowercase();
    if let Some(twin) = siblings
        .iter()
        .find(|s| **s != name && s.to_lowercase() == lower)
    {
        issues.push(NameIssue::CaseTwin((*twin).to_owned()));
    }
    issues
}

/// Whether a name, on its own, needs care.
pub fn needs_care(name: &str) -> bool {
    !check_name(name, &[]).is_empty()
}

/// A numbered name for keeping both: "report (2).pdf", "Makefile (2)",
/// ".profile (2)", and "archive (2).tar.gz" with the compound extensions
/// kept together.
pub fn numbered_name(name: &str, n: u32) -> String {
    let (stem, extension) = split_extension_with(name, &COMPOUND_EXTENSIONS);
    format!("{stem} ({n}){extension}")
}

/// The first numbered name, from 2, that no sibling has, comparing without
/// regard to case (so a case-insensitive place never gets a twin); `None`
/// after 999 tries.
pub fn unused_name(name: &str, siblings: &[&str]) -> Option<String> {
    let taken: Vec<String> = siblings.iter().map(|s| s.to_lowercase()).collect();
    (2..1000)
        .map(|n| numbered_name(name, n))
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    const WINDOWS: NameRules = NameRules {
        windows: true,
        max_length: Some(255),
        length_in_utf16: true,
    };
    const UNIX: NameRules = NameRules {
        windows: false,
        max_length: Some(255),
        length_in_utf16: false,
    };

    #[test]
    fn ordinary_names_are_fine() {
        for name in [
            "report.pdf",
            "My Documents",
            "Report (1).pdf",
            ".profile",
            "a-b_c.txt",
            "\u{1F980} crab.txt",
            "\u{65E5}\u{672C}.txt",
        ] {
            assert!(check_name(name, &[]).is_empty(), "{name}");
        }
    }

    #[test]
    fn dangerous_names_are_flagged() {
        assert_eq!(check_name("", &[]), vec![NameIssue::Empty]);
        assert_eq!(
            check_name("C\u{F03A}", &[]),
            vec![NameIssue::HiddenCharacters]
        );
        assert_eq!(
            check_name("a:b", &[]),
            vec![NameIssue::ReservedCharacter(':')]
        );
        assert_eq!(
            check_name("notes.", &[]),
            vec![NameIssue::TrailingDotOrSpace]
        );
        assert_eq!(check_name(" x", &[]), vec![NameIssue::LeadingSpace]);
        assert_eq!(check_name("con.txt", &[]), vec![NameIssue::DeviceName]);
        assert_eq!(check_name("NUL", &[]), vec![NameIssue::DeviceName]);
        assert_eq!(
            check_name("COM\u{B9}.log", &[]),
            vec![NameIssue::DeviceName]
        );
        assert_eq!(check_name("$HOME", &[]), vec![NameIssue::ShellSpecial('$')]);
        assert_eq!(check_name("..", &[]), vec![NameIssue::DotName]);
        assert_eq!(
            check_name("Readme", &["readme", "other"]),
            vec![NameIssue::CaseTwin("readme".into())]
        );
        assert_eq!(
            NameIssue::TrailingDotOrSpace.describe(),
            "ends in a dot or space"
        );
    }

    #[test]
    fn lengths_count_by_the_place_rules() {
        let long = "\u{e9}".repeat(200);
        assert!(check_name_with(&long, &[], &WINDOWS).is_empty());
        assert_eq!(
            check_name_with(&long, &[], &UNIX),
            vec![NameIssue::TooLong { max: 255 }]
        );
    }

    #[test]
    fn windows_places_refuse_what_windows_would_change() {
        let trailing = NameIssue::TrailingDotOrSpace;
        assert!(trailing.refused(&WINDOWS));
        assert!(!trailing.refused(&UNIX));
        assert!(NameIssue::DeviceName.refused(&WINDOWS));
        assert!(NameIssue::ReservedCharacter(':').refused(&WINDOWS));
        assert!(!NameIssue::ReservedCharacter(':').refused(&UNIX));
        assert!(NameIssue::DotName.refused(&UNIX));
        assert!(!NameIssue::HiddenCharacters.refused(&WINDOWS));
    }

    #[test]
    fn every_line_says_its_meaning_in_40_cells() {
        let issues = [
            NameIssue::Empty,
            NameIssue::DotName,
            NameIssue::HiddenCharacters,
            NameIssue::ReservedCharacter('|'),
            NameIssue::TrailingDotOrSpace,
            NameIssue::LeadingSpace,
            NameIssue::DeviceName,
            NameIssue::ShellSpecial('$'),
            NameIssue::TooLong { max: 255 },
        ];
        for issue in issues {
            assert!(
                issue.line().chars().count() <= BRAILLE_CELLS,
                "{}",
                issue.line()
            );
        }
    }

    #[test]
    fn keeping_both_numbers_the_name() {
        assert_eq!(numbered_name("report.pdf", 2), "report (2).pdf");
        assert_eq!(numbered_name("Makefile", 3), "Makefile (3)");
        assert_eq!(numbered_name(".profile", 2), ".profile (2)");
        assert_eq!(numbered_name("archive.tar.gz", 2), "archive (2).tar.gz");
        assert_eq!(
            unused_name("report.pdf", &["REPORT (2).PDF", "report.pdf"]).as_deref(),
            Some("report (3).pdf")
        );
    }

    #[test]
    fn unfinished_names_are_known() {
        let name = unfinished_name(0xDEAD_BEEF);
        assert_eq!(name, ".diro-part-deadbeef");
        assert!(is_unfinished(&name));
        assert!(!is_unfinished(".diro-part-xyz"));
        assert!(!is_unfinished("report.pdf"));
    }
}
