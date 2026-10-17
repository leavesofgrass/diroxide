// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Carried whole from textweaver's formats crate under the owner's
//! relicensing (answer 41); the doc references below are textweaver's.
//!
//! The callout rules shared by the reader and the renderer (ADR-0044):
//! Obsidian callouts (`> [!tip] Title`, any type, with `+` or `-` for a
//! foldable one) and GitHub's five alerts (`> [!NOTE]`, nothing after the
//! marker).
//!
//! The Markdown loader reads a callout as a labeled block quote ("Tip:
//! Remember." then the body); `textweaver-render` turns one into a
//! `<div>` or `<details>` element. Both parse the head line here, so the
//! two always agree on what is a callout, its type, its title, and its
//! fold state.

/// Obsidian's built-in callout types and their aliases: the written type
/// (lowercase) and the type it styles as.
pub const TYPES: &[(&str, &str)] = &[
    ("note", "note"),
    ("abstract", "abstract"),
    ("summary", "abstract"),
    ("tldr", "abstract"),
    ("info", "info"),
    ("todo", "todo"),
    ("tip", "tip"),
    ("hint", "tip"),
    ("important", "tip"),
    ("success", "success"),
    ("check", "success"),
    ("done", "success"),
    ("question", "question"),
    ("help", "question"),
    ("faq", "question"),
    ("warning", "warning"),
    ("caution", "warning"),
    ("attention", "warning"),
    ("failure", "failure"),
    ("fail", "failure"),
    ("missing", "failure"),
    ("danger", "danger"),
    ("error", "danger"),
    ("bug", "bug"),
    ("example", "example"),
    ("quote", "quote"),
    ("cite", "quote"),
];

/// GitHub's alert types.
pub const ALERTS: [&str; 5] = ["note", "tip", "important", "warning", "caution"];

/// Longest callout type read; a longer `[!...]` is not a callout.
const MAX_TYPE: usize = 64;

/// A callout's fold state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    /// `+`: foldable, open at first.
    Expanded,
    /// `-`: foldable, closed at first.
    Collapsed,
}

/// The head line of a callout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalloutHead {
    /// Spaces before the `>`.
    pub indent: usize,
    /// The type as written, in lowercase (`hint`).
    pub kind: String,
    /// Whether it folds, and how it starts.
    pub fold: Option<Fold>,
    /// The custom title after the marker, trimmed; empty when there is
    /// none.
    pub custom_title: String,
}

impl CalloutHead {
    /// The type it styles as: a built-in type for an alias (`hint` is
    /// `tip`), else the type as written (Obsidian keeps custom types).
    pub fn canonical(&self) -> &str {
        TYPES
            .iter()
            .find(|(t, _)| *t == self.kind)
            .map_or(self.kind.as_str(), |(_, c)| c)
    }

    /// The type as a word to show and say: `Tip`, `To do`, `FAQ`; a custom
    /// type capitalized, its hyphens and underscores as spaces.
    pub fn type_word(&self) -> String {
        match self.kind.as_str() {
            "todo" => "To do".to_owned(),
            "faq" => "FAQ".to_owned(),
            "tldr" => "TLDR".to_owned(),
            k => capitalize(&k.replace(['-', '_'], " ")),
        }
    }

    /// The title shown: the custom title, else the type word.
    pub fn title(&self) -> String {
        if self.custom_title.is_empty() {
            self.type_word()
        } else {
            self.custom_title.clone()
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Reads `> [!type]± Title` (after any indent, which the caller judges:
/// a callout inside a list item is indented). With `obsidian` false only
/// GitHub's alerts count: the five types, with nothing after the marker.
pub fn head(line: &str, obsidian: bool) -> Option<CalloutHead> {
    let line = line.trim_end_matches(['\n', '\r']);
    let t = line.trim_start();
    let indent = line.len() - t.len();
    let rest = t.strip_prefix('>')?.trim_start().strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = rest[..close].trim();
    if kind.is_empty()
        || kind.len() > MAX_TYPE
        || !kind
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let kind = kind.to_lowercase();
    let mut after = &rest[close + 1..];
    let mut fold = None;
    if obsidian {
        if let Some(a) = after.strip_prefix('+') {
            fold = Some(Fold::Expanded);
            after = a;
        } else if let Some(a) = after.strip_prefix('-') {
            fold = Some(Fold::Collapsed);
            after = a;
        }
    }
    let custom_title = after.trim().to_owned();
    if !obsidian && (!ALERTS.contains(&kind.as_str()) || !custom_title.is_empty()) {
        return None;
    }
    Some(CalloutHead {
        indent,
        kind,
        fold,
        custom_title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_type_with_title_and_fold() {
        let h = head("> [!hint]- Remember this\n", true).unwrap();
        assert_eq!(h.kind, "hint");
        assert_eq!(h.canonical(), "tip");
        assert_eq!(h.fold, Some(Fold::Collapsed));
        assert_eq!(h.title(), "Remember this");
        assert_eq!(h.type_word(), "Hint");
        let custom = head(">[!my-box]", true).unwrap();
        assert_eq!(custom.canonical(), "my-box");
        assert_eq!(custom.title(), "My box");
        assert_eq!(head("> [!todo]+", true).unwrap().title(), "To do");
    }

    #[test]
    fn github_alerts_only_without_obsidian() {
        assert!(head("> [!NOTE]", false).is_some());
        assert!(head("> [!tip] Title", false).is_none());
        assert!(head("> [!custom]", false).is_none());
        assert_eq!(head("    > [!note]", true).unwrap().indent, 4);
        assert!(head("> [!] x", true).is_none());
        assert!(head("> [!a b]", true).is_none());
        assert!(head("> text", true).is_none());
    }
}
