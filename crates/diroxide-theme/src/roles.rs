// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The file manager's roles (plan section 5.10): the parts of diroxide's
//! screen a theme colors, each with a color pair and an attribute, so no
//! state is shown by color alone. The reader roles of textweaver's model
//! never covered them: the panel's page and text, the focused and marked
//! rows, folders, archives, links and broken links, hidden entries, the
//! active and inactive title lines, the status and jobs lines, the key
//! bar, dialogs and their default button, the viewer's and the quick
//! view's headings, code and links in documents, search matches, diff
//! additions and removals, progress, and the plan listing's "will be
//! removed" rows.
//!
//! Each role is drawn from the theme's own colors and styles, so it is
//! checked by the theme's own contrast checks; [`panel_checks`] checks
//! each pair again, which is what an override by name needs (W1c's
//! `[panel]` table re-checks a changed role and says the result in
//! words). The words are those of [`verdict`]: "meets AA", or "below AA: 2
//! checks fall short" (AAA for high-contrast themes).
//!
//! The words always come first: a row says "folder", "link", "broken
//! link", "hidden", "selected"; a diff line says "added" or "removed"; a
//! plan row says "will be removed". These roles are the extra, never the
//! only signal.

use std::fmt;

use crate::check::{Check, ContrastReport, Requirement, check, check_pair};
use crate::color::Rgb;
use crate::model::{Attrs, ColorRole, StyleRole, Theme, ThemeKind};

/// One part of the file manager's screen that a theme colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PanelRole {
    /// The panel's page: its background, with plain text on it.
    Page,
    /// A row's ordinary text.
    Text,
    /// The focused row (the cursor's row).
    FocusedRow,
    /// A marked row (it also starts with an asterisk and says "selected").
    MarkedRow,
    /// A folder's row.
    Folder,
    /// An archive's row.
    Archive,
    /// A link's row.
    Link,
    /// A broken link's row (it also says "broken link").
    BrokenLink,
    /// A hidden entry's row.
    Hidden,
    /// The active panel's title line.
    ActiveTitle,
    /// The inactive panel's title line.
    InactiveTitle,
    /// The status line.
    StatusLine,
    /// The jobs line.
    JobsLine,
    /// The key bar.
    KeyBar,
    /// A dialog's body.
    Dialog,
    /// A dialog's default button.
    DefaultButton,
    /// A heading in the viewer.
    ViewerHeading,
    /// A heading in the quick view.
    QuickViewHeading,
    /// Code in the viewer and the quick view.
    Code,
    /// A link in the viewer and the quick view.
    DocumentLink,
    /// A search match.
    SearchMatch,
    /// A diff's added line (it also says "added").
    DiffAdded,
    /// A diff's removed line (it also says "removed").
    DiffRemoved,
    /// Progress on the jobs line.
    Progress,
    /// A plan listing's row that will be removed (it also says so).
    WillBeRemoved,
}

impl PanelRole {
    /// Every role, in the order of the list in plan section 5.10.
    pub const ALL: [PanelRole; 25] = [
        PanelRole::Page,
        PanelRole::Text,
        PanelRole::FocusedRow,
        PanelRole::MarkedRow,
        PanelRole::Folder,
        PanelRole::Archive,
        PanelRole::Link,
        PanelRole::BrokenLink,
        PanelRole::Hidden,
        PanelRole::ActiveTitle,
        PanelRole::InactiveTitle,
        PanelRole::StatusLine,
        PanelRole::JobsLine,
        PanelRole::KeyBar,
        PanelRole::Dialog,
        PanelRole::DefaultButton,
        PanelRole::ViewerHeading,
        PanelRole::QuickViewHeading,
        PanelRole::Code,
        PanelRole::DocumentLink,
        PanelRole::SearchMatch,
        PanelRole::DiffAdded,
        PanelRole::DiffRemoved,
        PanelRole::Progress,
        PanelRole::WillBeRemoved,
    ];

    /// The key for a setting that overrides the role, `snake_case`.
    pub const fn key(self) -> &'static str {
        match self {
            PanelRole::Page => "page",
            PanelRole::Text => "text",
            PanelRole::FocusedRow => "focused_row",
            PanelRole::MarkedRow => "marked_row",
            PanelRole::Folder => "folder",
            PanelRole::Archive => "archive",
            PanelRole::Link => "link",
            PanelRole::BrokenLink => "broken_link",
            PanelRole::Hidden => "hidden",
            PanelRole::ActiveTitle => "active_title",
            PanelRole::InactiveTitle => "inactive_title",
            PanelRole::StatusLine => "status_line",
            PanelRole::JobsLine => "jobs_line",
            PanelRole::KeyBar => "key_bar",
            PanelRole::Dialog => "dialog",
            PanelRole::DefaultButton => "default_button",
            PanelRole::ViewerHeading => "viewer_heading",
            PanelRole::QuickViewHeading => "quick_view_heading",
            PanelRole::Code => "code",
            PanelRole::DocumentLink => "document_link",
            PanelRole::SearchMatch => "search_match",
            PanelRole::DiffAdded => "diff_added",
            PanelRole::DiffRemoved => "diff_removed",
            PanelRole::Progress => "progress",
            PanelRole::WillBeRemoved => "will_be_removed",
        }
    }

    /// The role in words, as Settings and the contrast check say it.
    pub const fn label(self) -> &'static str {
        match self {
            PanelRole::Page => "panel page",
            PanelRole::Text => "row text",
            PanelRole::FocusedRow => "focused row",
            PanelRole::MarkedRow => "marked row",
            PanelRole::Folder => "folder",
            PanelRole::Archive => "archive",
            PanelRole::Link => "link",
            PanelRole::BrokenLink => "broken link",
            PanelRole::Hidden => "hidden entry",
            PanelRole::ActiveTitle => "active title line",
            PanelRole::InactiveTitle => "inactive title line",
            PanelRole::StatusLine => "status line",
            PanelRole::JobsLine => "jobs line",
            PanelRole::KeyBar => "key bar",
            PanelRole::Dialog => "dialog",
            PanelRole::DefaultButton => "default button",
            PanelRole::ViewerHeading => "viewer heading",
            PanelRole::QuickViewHeading => "quick view heading",
            PanelRole::Code => "code",
            PanelRole::DocumentLink => "document link",
            PanelRole::SearchMatch => "search match",
            PanelRole::DiffAdded => "added line",
            PanelRole::DiffRemoved => "removed line",
            PanelRole::Progress => "progress",
            PanelRole::WillBeRemoved => "will be removed",
        }
    }

    /// Reads a key.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.key() == key)
    }
}

impl fmt::Display for PanelRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A role's colors and attribute in a theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoleStyle {
    /// The text color.
    pub foreground: Rgb,
    /// The background color.
    pub background: Rgb,
    /// The attribute that marks the role without color.
    pub attributes: Attrs,
}

fn colored(theme: &Theme, fg: ColorRole, bg: ColorRole, attributes: Attrs) -> RoleStyle {
    RoleStyle {
        foreground: theme.color(fg),
        background: theme.color(bg),
        attributes,
    }
}

fn styled(theme: &Theme, role: StyleRole, add: Attrs) -> RoleStyle {
    let resolved = theme.resolve_style(role);
    let reverse = theme.style(role).attributes.reverse;
    let mut attributes = resolved.attributes.with(add);
    if attributes.is_empty() && !reverse {
        attributes = role.no_color_attributes();
    }
    RoleStyle {
        foreground: resolved.foreground,
        background: resolved.background,
        attributes,
    }
}

/// A role's colors and attribute in a theme, drawn from the theme's own
/// colors and styles.
pub fn role_style(theme: &Theme, role: PanelRole) -> RoleStyle {
    use ColorRole as C;
    use PanelRole as P;
    let on_page = |fg, attributes| colored(theme, fg, C::Background, attributes);
    match role {
        P::Page | P::Text => on_page(C::Text, Attrs::NONE),
        P::FocusedRow => styled(theme, StyleRole::Focus, Attrs::NONE),
        P::MarkedRow => styled(theme, StyleRole::Selection, Attrs::BOLD),
        P::Folder => on_page(C::Heading2, Attrs::BOLD),
        P::Archive => on_page(C::Heading3, Attrs::ITALIC),
        P::Link | P::DocumentLink => on_page(C::Link, Attrs::UNDERLINE),
        P::BrokenLink => on_page(C::Error, Attrs::UNDERLINE.with(Attrs::ITALIC)),
        P::Hidden => on_page(C::DimText, Attrs::ITALIC),
        P::ActiveTitle => styled(theme, StyleRole::StatusBar, Attrs::BOLD),
        P::InactiveTitle => on_page(C::DimText, Attrs::NONE),
        P::StatusLine | P::JobsLine => styled(theme, StyleRole::StatusBar, Attrs::NONE),
        P::KeyBar => colored(theme, C::PanelDimText, C::Surface, Attrs::NONE),
        P::Dialog => colored(theme, C::Text, C::Surface, Attrs::NONE),
        P::DefaultButton => colored(theme, C::OnAccent, C::Accent, Attrs::BOLD),
        P::ViewerHeading => on_page(C::Heading1, Attrs::BOLD),
        P::QuickViewHeading => on_page(C::Heading2, Attrs::BOLD),
        P::Code => colored(theme, C::Code, C::CodeBackground, Attrs::NONE),
        P::SearchMatch => styled(theme, StyleRole::FindHit, Attrs::NONE),
        // Blue against red, never green against red: the pair stays apart
        // with red-green color blindness, and the words say which is
        // which.
        P::DiffAdded => on_page(C::Link, Attrs::BOLD),
        P::DiffRemoved => on_page(C::Error, Attrs::ITALIC),
        P::Progress => on_page(C::Text, Attrs::BOLD),
        P::WillBeRemoved => on_page(C::Error, Attrs::BOLD),
    }
}

/// The roles whose text is drawn on a background and must meet the text
/// floor there.
fn is_text_role(role: PanelRole) -> bool {
    !matches!(role, PanelRole::Page)
}

/// One contrast check per role: its text on its background, at the
/// theme's floor (4.5 to 1, or 7 to 1 in a high-contrast theme). An
/// override of a role's color is checked the same way.
pub fn panel_checks(theme: &Theme) -> Vec<Check> {
    PanelRole::ALL
        .into_iter()
        .filter(|role| is_text_role(*role))
        .map(|role| {
            let style = role_style(theme, role);
            let mut c = check_pair(
                style.foreground,
                style.background,
                Requirement::Text,
                theme.kind(),
            );
            c.subject = role.label().to_owned();
            c.key = format!("panel.{}", role.key());
            c
        })
        .collect()
}

/// The verdict in words, for Settings and the status line when a theme is
/// chosen: "meets AA", or "below AA: 2 checks fall short" (AAA in a
/// high-contrast theme). Counts the theme's own failing checks, and the
/// panel roles' failing checks whose color pair the theme's checks have
/// not already counted, so one weak pair counts once.
pub fn verdict(theme: &Theme) -> String {
    let report: ContrastReport = check(theme);
    let counted: Vec<(Option<Rgb>, Option<Rgb>)> = report
        .failures()
        .map(|c| (c.foreground, c.background))
        .collect();
    let falling_short = report.failures().count()
        + panel_checks(theme)
            .iter()
            .filter(|c| !c.passed && !counted.contains(&(c.foreground, c.background)))
            .count();
    let level = if theme.kind() == ThemeKind::HighContrast {
        "AAA"
    } else {
        "AA"
    };
    match falling_short {
        0 => format!("meets {level}"),
        1 => format!("below {level}: 1 check falls short"),
        n => format!("below {level}: {n} checks fall short"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;

    #[test]
    fn every_role_has_a_key_a_label_and_reads_back() {
        for role in PanelRole::ALL {
            assert_eq!(PanelRole::from_key(role.key()), Some(role));
            assert!(!role.label().is_empty());
        }
        assert_eq!(PanelRole::from_key("nothing"), None);
    }

    #[test]
    fn state_roles_carry_an_attribute_besides_color() {
        let theme = builtin::default_theme();
        for role in [
            PanelRole::FocusedRow,
            PanelRole::MarkedRow,
            PanelRole::Folder,
            PanelRole::Archive,
            PanelRole::Link,
            PanelRole::BrokenLink,
            PanelRole::Hidden,
            PanelRole::ActiveTitle,
            PanelRole::DefaultButton,
            PanelRole::SearchMatch,
            PanelRole::DiffAdded,
            PanelRole::DiffRemoved,
            PanelRole::WillBeRemoved,
        ] {
            let style = role_style(theme, role);
            let reversed = matches!(role, PanelRole::FocusedRow)
                && theme.style(StyleRole::Focus).attributes.reverse;
            assert!(
                !style.attributes.is_empty() || reversed,
                "{role} has no attribute"
            );
        }
        let added = role_style(theme, PanelRole::DiffAdded);
        let removed = role_style(theme, PanelRole::DiffRemoved);
        assert_ne!(added.attributes, removed.attributes);
    }

    #[test]
    fn the_default_theme_meets_aa_and_says_so() {
        let theme = builtin::default_theme();
        assert_eq!(verdict(theme), "meets AA");
        assert!(panel_checks(theme).iter().all(|c| c.passed));
    }

    #[test]
    fn every_built_in_theme_has_a_verdict_in_words() {
        for theme in builtin::all() {
            let said = verdict(theme);
            assert!(
                said.starts_with("meets AA") || said.starts_with("below AA"),
                "{}: {said}",
                theme.name()
            );
        }
    }

    #[test]
    fn a_low_contrast_override_is_below_aa() {
        let mut theme = builtin::default_theme().clone();
        let page = theme.color(ColorRole::Background);
        theme.set_color(ColorRole::Heading2, page);
        let said = verdict(&theme);
        assert!(said.starts_with("below AA: "), "{said}");
        assert!(said.ends_with("fall short") || said.ends_with("falls short"));
    }
}
