//! The name rule of a folder, probed without writing anything (research
//! bulk-rename.md, "Names and equivalence"): find a name in the folder
//! with a letter whose case can change, look the other case up, and
//! compare identities. The same entry means the folder ignores case; no
//! entry, or another entry, means it does not. Unicode form is probed the
//! same way with a precomposed accented letter and its decomposed form.
//!
//! When nothing in the folder can answer, or an identity cannot be read,
//! the stricter rule is assumed: case ignored and form ignored. Its cost is
//! a false "same name" on a case-sensitive folder, which the row explains;
//! the other mistake loses a file.

use std::fs;
use std::path::Path;

use crate::sys;

/// How many names the probe reads before it gives up looking.
const NAMES_TO_READ: usize = 200;

/// A folder's rule for names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NameRule {
    /// Names that differ only by case are different entries.
    pub(crate) case_sensitive: bool,
    /// Names that differ only by Unicode form are the same entry.
    pub(crate) normalization_insensitive: bool,
}

impl NameRule {
    /// The stricter rule, assumed when the folder cannot answer.
    pub(crate) const STRICT: NameRule = NameRule {
        case_sensitive: false,
        normalization_insensitive: true,
    };
}

/// Precomposed lowercase letters and their decomposed form: the base
/// letter and the combining mark. Uppercase forms are found from these.
const COMPOSED: [(char, char, char); 26] = [
    ('\u{E0}', 'a', '\u{300}'),
    ('\u{E1}', 'a', '\u{301}'),
    ('\u{E2}', 'a', '\u{302}'),
    ('\u{E3}', 'a', '\u{303}'),
    ('\u{E4}', 'a', '\u{308}'),
    ('\u{E5}', 'a', '\u{30A}'),
    ('\u{E7}', 'c', '\u{327}'),
    ('\u{E8}', 'e', '\u{300}'),
    ('\u{E9}', 'e', '\u{301}'),
    ('\u{EA}', 'e', '\u{302}'),
    ('\u{EB}', 'e', '\u{308}'),
    ('\u{EC}', 'i', '\u{300}'),
    ('\u{ED}', 'i', '\u{301}'),
    ('\u{EE}', 'i', '\u{302}'),
    ('\u{EF}', 'i', '\u{308}'),
    ('\u{F1}', 'n', '\u{303}'),
    ('\u{F2}', 'o', '\u{300}'),
    ('\u{F3}', 'o', '\u{301}'),
    ('\u{F4}', 'o', '\u{302}'),
    ('\u{F5}', 'o', '\u{303}'),
    ('\u{F6}', 'o', '\u{308}'),
    ('\u{F9}', 'u', '\u{300}'),
    ('\u{FA}', 'u', '\u{301}'),
    ('\u{FB}', 'u', '\u{302}'),
    ('\u{FC}', 'u', '\u{308}'),
    ('\u{FD}', 'y', '\u{301}'),
];

/// The name with every ASCII letter's case swapped, or `None` when it has
/// no ASCII letter.
pub(crate) fn case_swapped(name: &str) -> Option<String> {
    name.bytes().any(|b| b.is_ascii_alphabetic()).then(|| {
        name.chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect()
    })
}

/// The decomposed form of a precomposed letter, uppercase included.
fn decompose(c: char) -> Option<(char, char)> {
    COMPOSED.iter().find_map(|&(composed, base, mark)| {
        let upper = char::from_u32(u32::from(composed) - 0x20)?;
        if c == composed {
            Some((base, mark))
        } else if c == upper {
            Some((base.to_ascii_uppercase(), mark))
        } else {
            None
        }
    })
}

/// The precomposed letter for a base and a mark, uppercase included.
fn compose(base: char, mark: char) -> Option<char> {
    COMPOSED.iter().find_map(|&(composed, b, m)| {
        if m != mark {
            None
        } else if base == b {
            Some(composed)
        } else if base == b.to_ascii_uppercase() {
            char::from_u32(u32::from(composed) - 0x20)
        } else {
            None
        }
    })
}

/// The name in its other Unicode form, as far as the table knows:
/// precomposed letters decomposed, or, for a name with none, decomposed
/// pairs composed. `None` when the name has neither.
pub(crate) fn other_form(name: &str) -> Option<String> {
    if name.chars().any(|c| decompose(c).is_some()) {
        let mut out = String::with_capacity(name.len() + 4);
        for c in name.chars() {
            match decompose(c) {
                Some((base, mark)) => {
                    out.push(base);
                    out.push(mark);
                }
                None => out.push(c),
            }
        }
        return Some(out);
    }
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len());
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        if let Some(composed) = chars.get(i + 1).and_then(|&mark| compose(chars[i], mark)) {
            out.push(composed);
            changed = true;
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    changed.then_some(out)
}

/// Whether `other` names the same entry as `name` in `folder`: `Some(true)`
/// when it is found with the same identity, `Some(false)` when it is
/// missing or another entry, `None` when an identity cannot be read.
fn same_entry(folder: &Path, name: &str, other: &str) -> Option<bool> {
    let first = sys::identity(&folder.join(name)).ok()?;
    match sys::identity(&folder.join(other)) {
        Ok(second) => Some(first == second),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// Probes a folder's name rule.
pub(crate) fn name_rule(folder: &Path) -> NameRule {
    let Ok(reader) = fs::read_dir(folder) else {
        return NameRule::STRICT;
    };
    let names: Vec<String> = reader
        .take(NAMES_TO_READ)
        .filter_map(|item| item.ok()?.file_name().into_string().ok())
        .collect();
    let mut rule = NameRule::STRICT;
    // Two names in one listing that differ only by case are two entries.
    let twins = names.iter().any(|name| {
        case_swapped(name).is_some_and(|swapped| swapped != *name && names.contains(&swapped))
    });
    if twins {
        rule.case_sensitive = true;
    } else if let Some(answer) = names.iter().find_map(|name| {
        let swapped = case_swapped(name)?;
        same_entry(folder, name, &swapped)
    }) {
        rule.case_sensitive = !answer;
    }
    if let Some(answer) = names.iter().find_map(|name| {
        let other = other_form(name)?;
        same_entry(folder, name, &other)
    }) {
        rule.normalization_insensitive = answer;
    }
    rule
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_swaps_touch_only_ascii_letters() {
        assert_eq!(case_swapped("Report1.PDF").as_deref(), Some("rEPORT1.pdf"));
        assert_eq!(case_swapped("\u{65E5}\u{672C}"), None);
        assert_eq!(case_swapped("123"), None);
    }

    #[test]
    fn forms_go_both_ways() {
        assert_eq!(other_form("caf\u{E9}").as_deref(), Some("cafe\u{301}"));
        assert_eq!(other_form("CAF\u{C9}").as_deref(), Some("CAFE\u{301}"));
        assert_eq!(other_form("cafe\u{301}").as_deref(), Some("caf\u{E9}"));
        assert_eq!(other_form("Ni\u{F1}o").as_deref(), Some("Nin\u{303}o"));
        assert_eq!(other_form("plain"), None);
        assert_eq!(other_form("\u{301}lone"), None);
    }

    #[test]
    fn a_missing_folder_gets_the_strict_rule() {
        let rule = name_rule(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("no-such")
                .as_path(),
        );
        assert_eq!(rule, NameRule::STRICT);
    }

    #[test]
    fn this_crate_folder_answers_by_its_file_system() {
        // The crate folder holds names with letters; the probe's answer
        // must match a direct look at one of them, whatever file system
        // the checkout is on (NTFS, ext4, or a bind mount of either).
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
        let rule = name_rule(folder);
        let swapped_exists = folder.join("CARGO.TOML").exists();
        assert_eq!(rule.case_sensitive, !swapped_exists);
    }
}
