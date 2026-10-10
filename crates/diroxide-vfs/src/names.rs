//! The name checks: names that are dangerous to type, flagged on creation
//! and rename, and shown on rows as "name needs care".
//!
//! Phase 0 covers the listed cases simply; Wave 1 (W1a) owns this module,
//! adds the property tests and the fuzz target, and settles which shell
//! characters to flag.

use diroxide_core::names::has_hidden;

/// Characters Windows does not allow in names, plus the separators.
pub const RESERVED_CHARACTERS: [char; 9] = [':', '*', '?', '"', '<', '>', '|', '\\', '/'];

/// Characters a shell expands, so a name with them changes when typed at a
/// prompt.
pub const SHELL_SPECIAL: [char; 2] = ['$', '`'];

/// Windows device names, reserved with or without an extension.
pub const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Something about a name that needs care.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameIssue {
    /// The name is empty.
    Empty,
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
}

impl NameIssue {
    /// The issue in words, meaning first: "ends with a dot or a space".
    pub fn describe(&self) -> String {
        match self {
            NameIssue::Empty => "is empty".to_owned(),
            NameIssue::HiddenCharacters => "has hidden characters".to_owned(),
            NameIssue::ReservedCharacter(c) => format!("has the reserved character {c}"),
            NameIssue::TrailingDotOrSpace => "ends with a dot or a space".to_owned(),
            NameIssue::LeadingSpace => "starts with a space".to_owned(),
            NameIssue::DeviceName => "is a Windows device name".to_owned(),
            NameIssue::ShellSpecial(c) => format!("has the shell character {c}"),
            NameIssue::CaseTwin(other) => format!("differs only by case from {other}"),
        }
    }
}

/// Every issue with a name, given the names already in its folder.
pub fn check_name(name: &str, siblings: &[&str]) -> Vec<NameIssue> {
    let mut issues = Vec::new();
    if name.is_empty() {
        issues.push(NameIssue::Empty);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_are_fine() {
        for name in [
            "report.pdf",
            "My Documents",
            "Report (1).pdf",
            ".profile",
            "a-b_c.txt",
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
        assert_eq!(check_name("$HOME", &[]), vec![NameIssue::ShellSpecial('$')]);
        assert_eq!(
            check_name("Readme", &["readme", "other"]),
            vec![NameIssue::CaseTwin("readme".into())]
        );
        assert_eq!(
            NameIssue::TrailingDotOrSpace.describe(),
            "ends with a dot or a space"
        );
    }
}
