// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! WCAG contrast checks for a theme: the same code gates the built-in themes
//! in tests and warns about user themes at load time (warn, never block).
//!
//! Thresholds (WCAG 2.x): text 4.5:1 (1.4.3 AA), 7:1 in high-contrast
//! themes (1.4.6 AAA); large text 3:1, 4.5:1 in high contrast; non-text
//! indicators 3:1 (1.4.11). Headings are held to the text floor, not the
//! large-text one, because the same palette drives the terminal, where every
//! cell is one size. APCA Lc is computed for every pair and reported for
//! information only.
//!
//! Beyond ratios, every highlight must carry a text attribute (1.4.1, use of
//! color), and highlights that appear together must differ by attribute:
//! the spoken word from its sentence, the current match from the others.

use crate::color::{Rgb, apca_lc, contrast_ratio, spoken_ratio};
use crate::model::{ColorRole, RoleClass, StyleRole, Theme, ThemeKind};

/// What a check measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// Body-size text on its background.
    Text,
    /// Large text (18 pt, or 14 pt bold). No built-in check uses it, because
    /// terminals have no large text; available to callers.
    LargeText,
    /// A non-text indicator (a focus ring or band) against what is next to it.
    NonText,
    /// A highlight carries at least one text attribute, so it is not shown by
    /// color alone.
    Cue,
    /// Two highlights that appear together differ by attribute.
    Distinct,
}

/// The contrast floor for a requirement in a theme of this kind; 0 for the
/// requirements that are not ratios.
pub fn minimum(req: Requirement, kind: ThemeKind) -> f64 {
    let hc = kind == ThemeKind::HighContrast;
    match req {
        Requirement::Text if hc => 7.0,
        Requirement::Text => 4.5,
        Requirement::LargeText if hc => 4.5,
        Requirement::LargeText => 3.0,
        Requirement::NonText => 3.0,
        Requirement::Cue | Requirement::Distinct => 0.0,
    }
}

/// The contrast floor a role is held to by its class, against what it is
/// drawn on ([`fill`]): text and the text on surfaces and bands at the
/// text floor, disabled text at the 3 to 1 house floor, indicators at 3 to
/// 1, and decorative lines at none.
pub fn role_minimum(role: ColorRole, kind: ThemeKind) -> f64 {
    match role.class() {
        _ if role == ColorRole::Disabled => 3.0,
        RoleClass::Text | RoleClass::Surface | RoleClass::Band => minimum(Requirement::Text, kind),
        RoleClass::Indicator => minimum(Requirement::NonText, kind),
        RoleClass::Decorative => 0.0,
    }
}

/// How a role is drawn on its own: the color in front and the fill behind
/// it (`None` for the page). Surfaces and bands put body text on themselves;
/// the derived text roles sit on the panel, the accent, or a raised control.
pub fn fill(role: ColorRole) -> (ColorRole, Option<ColorRole>) {
    use ColorRole as C;
    match role {
        C::Background | C::Surface | C::Raised | C::Ruler | C::RulerBand => (C::Text, Some(role)),
        C::CodeBackground => (C::Code, Some(role)),
        C::PanelDimText => (role, Some(C::Surface)),
        C::OnAccent => (role, Some(C::Accent)),
        C::Disabled => (role, Some(C::Raised)),
        _ => (role, None),
    }
}

/// One measured pair or rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// What was measured, as read aloud: `heading 2 on background`.
    pub subject: String,
    /// The theme-file key at fault, for example `colors.heading2`.
    pub key: String,
    /// The rule.
    pub requirement: Requirement,
    /// Foreground (text or indicator), for ratio checks.
    pub foreground: Option<Rgb>,
    /// Background, for ratio checks.
    pub background: Option<Rgb>,
    /// WCAG 2.x contrast ratio, for ratio checks.
    pub ratio: Option<f64>,
    /// APCA Lc of the foreground on the background, for information.
    pub apca: Option<f64>,
    /// The floor, for ratio checks.
    pub minimum: Option<f64>,
    /// Whether the rule is met.
    pub passed: bool,
}

impl Check {
    fn pair(
        subject: String,
        key: String,
        req: Requirement,
        fg: Rgb,
        bg: Rgb,
        kind: ThemeKind,
    ) -> Self {
        let ratio = contrast_ratio(fg, bg);
        let min = minimum(req, kind);
        Check {
            subject,
            key,
            requirement: req,
            foreground: Some(fg),
            background: Some(bg),
            ratio: Some(ratio),
            apca: Some(apca_lc(fg, bg)),
            minimum: Some(min),
            passed: ratio >= min,
        }
    }

    fn rule(subject: String, key: String, req: Requirement, passed: bool) -> Self {
        Check {
            subject,
            key,
            requirement: req,
            foreground: None,
            background: None,
            ratio: None,
            apca: None,
            minimum: None,
            passed,
        }
    }

    /// A sentence that reads well aloud, for example
    /// `Heading 2 on background: 3.6 to 1, needs 4.5 to 1.`
    pub fn describe(&self) -> String {
        let mut s = capitalize(&self.subject);
        match (self.ratio, self.minimum) {
            (Some(r), Some(m)) => {
                s.push_str(&format!(": {}", spoken_ratio(r)));
                if !self.passed {
                    s.push_str(&format!(", needs {}", spoken_ratio(m)));
                }
            }
            _ => s.push_str(if self.passed { ": yes" } else { ": no" }),
        }
        s.push('.');
        s
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Every check for one theme.
#[derive(Clone, Debug, PartialEq)]
pub struct ContrastReport {
    /// Theme name.
    pub theme: String,
    /// Display name, for messages.
    pub display_name: String,
    /// All checks, passed and failed.
    pub checks: Vec<Check>,
}

impl ContrastReport {
    /// True when every check passes.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// The checks that fail.
    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| !c.passed)
    }

    /// One short paragraph for the status line and screen readers:
    /// `Theme Nord: all 61 checks pass.` or
    /// `Theme Mine: 2 of 61 checks fail. Dim text on background: 4.0 to 1,
    /// needs 4.5 to 1. …`
    pub fn summary(&self) -> String {
        let total = self.checks.len();
        let failed: Vec<_> = self.failures().collect();
        if failed.is_empty() {
            return format!("Theme {}: all {total} checks pass.", self.display_name);
        }
        let mut s = format!(
            "Theme {}: {} of {total} checks fail.",
            self.display_name,
            failed.len()
        );
        for c in failed {
            s.push(' ');
            s.push_str(&c.describe());
        }
        s
    }
}

/// Runs every check on a theme.
pub fn check(theme: &Theme) -> ContrastReport {
    use ColorRole as C;
    let kind = theme.kind();
    let bg = theme.color(C::Background);
    let mut v = Vec::new();
    let text_pair = |v: &mut Vec<Check>, fg: ColorRole, on: ColorRole| {
        v.push(Check::pair(
            format!("{} on {}", fg.label(), on.label()),
            format!("colors.{}", fg.key()),
            Requirement::Text,
            theme.color(fg),
            theme.color(on),
            kind,
        ));
    };
    for &r in ColorRole::ALL.iter().filter(|r| r.is_text()) {
        text_pair(&mut v, r, C::Background);
    }
    text_pair(&mut v, C::Code, C::CodeBackground);
    text_pair(&mut v, C::Text, C::CodeBackground);
    text_pair(&mut v, C::Text, C::Surface);

    for &r in StyleRole::ALL {
        let s = theme.resolve_style(r);
        let key = format!("styles.{}", r.key());
        v.push(Check::pair(
            format!("{} text", r.label()),
            key.clone(),
            Requirement::Text,
            s.foreground,
            s.background,
            kind,
        ));
        if r.is_highlight() {
            v.push(Check::rule(
                format!("{} marked by more than color", r.label()),
                format!("{key}.attributes"),
                Requirement::Cue,
                !theme.style(r).attributes.is_empty(),
            ));
        }
    }
    let focus = theme.resolve_style(StyleRole::Focus);
    v.push(Check::pair(
        "focus against background".into(),
        "styles.focus.background".into(),
        Requirement::NonText,
        focus.background,
        bg,
        kind,
    ));
    // The spoken word's band is a step up from the page and from the
    // sentence band around it. The sentence band itself needs no ratio: it
    // is a soft tint, and the sentence's underline is its cue.
    let word = theme.resolve_style(StyleRole::SpokenWord);
    let sentence = theme.resolve_style(StyleRole::SpokenSentence);
    for (what, against) in [
        ("background", bg),
        ("spoken sentence band", sentence.background),
    ] {
        v.push(Check::pair(
            format!("spoken word band against {what}"),
            "styles.spoken_word.background".into(),
            Requirement::NonText,
            word.background,
            against,
            kind,
        ));
    }
    for (a, b) in [
        (StyleRole::SpokenSentence, StyleRole::SpokenWord),
        (StyleRole::FindHit, StyleRole::CurrentFindHit),
    ] {
        v.push(Check::rule(
            format!(
                "{} and {} look different without color",
                a.label(),
                b.label()
            ),
            format!("styles.{}.attributes", b.key()),
            Requirement::Distinct,
            theme.style(a).attributes != theme.style(b).attributes,
        ));
    }
    interface_checks(theme, &mut v);
    for (i, h) in theme.user_highlights.iter().enumerate() {
        let s = theme.resolve(&h.style);
        let key = format!("user_highlights entry {} ({})", i + 1, h.name);
        v.push(Check::pair(
            format!("{} highlight text", h.name),
            key.clone(),
            Requirement::Text,
            s.foreground,
            s.background,
            kind,
        ));
        v.push(Check::rule(
            format!("{} highlight marked by more than color", h.name),
            format!("{key}.attributes"),
            Requirement::Cue,
            !h.style.attributes.is_empty(),
        ));
    }
    ContrastReport {
        theme: theme.meta.name.clone(),
        display_name: theme.meta.display_name.clone(),
        checks: v,
    }
}

/// The checks for the fifteen derived roles, by class: text on each fill it
/// is drawn on, indicators at 3 to 1 against what is beside them, bands with
/// text on them at the floor, and the three rules (accent against surface,
/// focus against its inner line, disabled text at the 3 to 1 house floor).
fn interface_checks(theme: &Theme, v: &mut Vec<Check>) {
    use ColorRole as C;
    let kind = theme.kind();
    let col = |r: ColorRole| theme.color(r);
    let key = |r: ColorRole| format!("colors.{}", r.key());
    let mut pair = |fg: ColorRole, on: ColorRole, req: Requirement, fg_key: ColorRole| {
        v.push(Check::pair(
            format!("{} on {}", fg.label(), on.label()),
            key(fg_key),
            req,
            col(fg),
            col(on),
            kind,
        ));
    };
    use Requirement::{NonText, Text};
    // Text on fills.
    pair(C::Text, C::Raised, Text, C::Raised);
    pair(C::PanelDimText, C::Surface, Text, C::PanelDimText);
    pair(C::OnAccent, C::Accent, Text, C::OnAccent);
    pair(C::Text, C::Ruler, Text, C::Ruler);
    pair(C::Text, C::RulerBand, Text, C::RulerBand);
    // Indicators.
    pair(C::ControlBorder, C::Surface, NonText, C::ControlBorder);
    pair(C::ControlBorder, C::Background, NonText, C::ControlBorder);
    pair(C::Accent, C::Surface, NonText, C::Accent);
    for r in [
        C::Caret,
        C::DifficultWord,
        C::SyllableMark,
        C::Misspelling,
        C::Lint,
    ] {
        pair(r, C::Background, NonText, r);
    }
    let focus = theme.resolve_style(StyleRole::Focus).background;
    v.push(Check::pair(
        "focus against inner focus line".into(),
        key(C::FocusInner),
        NonText,
        focus,
        col(C::FocusInner),
        kind,
    ));
    // Disabled text: a 3 to 1 house floor on raised controls.
    let mut disabled = Check::pair(
        format!("{} on {}", C::Disabled.label(), C::Raised.label()),
        key(C::Disabled),
        NonText,
        col(C::Disabled),
        col(C::Raised),
        kind,
    );
    disabled.minimum = Some(3.0);
    disabled.passed = disabled.ratio.is_some_and(|r| r >= 3.0);
    v.push(disabled);
    // Decorative: the border only has to differ from the page.
    v.push(Check::rule(
        "border differs from background".into(),
        key(C::Border),
        Requirement::Distinct,
        col(C::Border) != col(C::Background),
    ));
}

/// Checks one pair against a requirement in a theme of this kind; for
/// color pickers and previews.
pub fn check_pair(fg: Rgb, bg: Rgb, req: Requirement, kind: ThemeKind) -> Check {
    Check::pair("color".into(), String::new(), req, fg, bg, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors() {
        assert_eq!(minimum(Requirement::Text, ThemeKind::Dark), 4.5);
        assert_eq!(minimum(Requirement::Text, ThemeKind::HighContrast), 7.0);
        assert_eq!(minimum(Requirement::LargeText, ThemeKind::Light), 3.0);
        assert_eq!(
            minimum(Requirement::LargeText, ThemeKind::HighContrast),
            4.5
        );
        assert_eq!(minimum(Requirement::NonText, ThemeKind::HighContrast), 3.0);
    }

    #[test]
    fn describe_reads_aloud() {
        let c = check_pair(
            Rgb::from_u32(0x777777),
            Rgb::WHITE,
            Requirement::Text,
            ThemeKind::Light,
        );
        assert!(!c.passed);
        assert_eq!(c.describe(), "Color: 4.4 to 1, needs 4.5 to 1.");
        let c = check_pair(Rgb::BLACK, Rgb::WHITE, Requirement::Text, ThemeKind::Light);
        assert_eq!(c.describe(), "Color: 21.0 to 1.");
        assert!(c.apca.is_some_and(|l| l > 100.0));
        let c = check_pair(
            Rgb::from_u32(0x949494),
            Rgb::WHITE,
            Requirement::LargeText,
            ThemeKind::Light,
        );
        assert!(c.passed);
    }
}
