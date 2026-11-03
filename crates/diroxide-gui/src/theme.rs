//! The window's colors (R27's commit 27): the themes of
//! `crates/diroxide-theme` mapped to what the window draws, one
//! [`Look`] for every screen role ([`PanelRole`]), as the terminal maps
//! them in `crates/diroxide-tui/src/style.rs`.
//!
//! - Every role keeps its theme's colors and its attribute (bold,
//!   underline, italic), with the owner's `[colors]` overrides in place.
//! - A role whose text fails the theme's floor on its background (4.5 to
//!   1, or 7 to 1 in a high-contrast theme) is drawn in the text color
//!   instead (the dialogs lane's rule for the viewer's token classes, now
//!   for every role), or in black or white where even the text color
//!   fails; [`Palette::fell_back`] names those roles, so the window never
//!   draws a pair below the floor whatever the theme.
//! - The focus ring keeps 3 to 1 against the page and the dialog's card,
//!   and a line inside it keeps 3 to 1 against the focused row's band
//!   (textweaver's double ring), since no one color does against both a
//!   dark page and a light band.
//! - Words come first, color second: a row says "folder", "link", "broken
//!   link" and "hidden" in its name, a marked row starts with an asterisk
//!   and says "selected", the focused row has an outline as well as its
//!   band; every role's attribute is drawn too. Color is the extra.
//! - Windows High Contrast ("Contrast themes") is honored: while it is on
//!   the window draws only with the system's own colors
//!   ([`SystemColors`], read by the platform module), as textweaver does
//!   in `crates/textweaver-xilem/src/system_colors.rs`.
//! - Following the system's light or dark appearance goes through the
//!   theme crate's `os` module ([`drawn_theme`]).
//!
//! [`Palette::report`] runs the theme crate's checks on every pair the
//! window draws, so a test holds every built-in theme to the floor.
//!
//! Adapted from textweaver's `crates/textweaver-xilem/src/theme.rs` and
//! `system_colors.rs` (the owner's own project).

use std::collections::BTreeMap;

use diroxide_theme::check::{Requirement, check_pair, minimum};
use diroxide_theme::os::{OsScheme, follow_os};
use diroxide_theme::{
    Attrs, Check, ColorRole, ContrastReport, PanelRole, Registry, Rgb, Theme, ThemeKind,
    contrast_ratio, role_style,
};

/// How one part of the window is drawn: its text color, the fill behind
/// it, and the attribute that shows it without color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// The text color.
    pub fg: Rgb,
    /// The fill behind the text.
    pub bg: Rgb,
    /// Bold, italic, underline; reverse is already applied to the
    /// colors, and the window draws a reversed role underlined.
    pub attrs: Attrs,
}

/// The colors of the system's high contrast scheme (Windows' "Contrast
/// themes"): the person chose every one, so the window draws with them
/// and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemColors {
    /// The window's background.
    pub window: Rgb,
    /// Text on it.
    pub window_text: Rgb,
    /// Selected items and the focus.
    pub highlight: Rgb,
    /// Text on the highlight.
    pub highlight_text: Rgb,
    /// Links.
    pub hot_light: Rgb,
    /// Buttons' face.
    pub button_face: Rgb,
    /// Buttons' text.
    pub button_text: Rgb,
    /// Disabled text.
    pub gray_text: Rgb,
}

/// Windows' "Night sky" contrast theme, near enough: a black page, white
/// text, a yellow highlight, cyan links, green disabled text. For the
/// tests; the window reads the real colors.
pub const NIGHT_SKY: SystemColors = SystemColors {
    window: Rgb::new(0, 0, 0),
    window_text: Rgb::new(0xff, 0xff, 0xff),
    highlight: Rgb::new(0xff, 0xff, 0),
    highlight_text: Rgb::new(0, 0, 0),
    hot_light: Rgb::new(0, 0xff, 0xff),
    button_face: Rgb::new(0, 0, 0),
    button_text: Rgb::new(0xff, 0xff, 0xff),
    gray_text: Rgb::new(0x3f, 0xf2, 0x3f),
};

/// Windows' "Desert" contrast theme, near enough: a light page with dark
/// text, the other way round from [`NIGHT_SKY`]. For the tests.
pub const DESERT: SystemColors = SystemColors {
    window: Rgb::new(0xff, 0xfa, 0xef),
    window_text: Rgb::new(0x3d, 0x3d, 0x3d),
    highlight: Rgb::new(0x90, 0x39, 0x09),
    highlight_text: Rgb::new(0xff, 0xf5, 0xe3),
    hot_light: Rgb::new(0x1c, 0x5e, 0x75),
    button_face: Rgb::new(0xff, 0xfa, 0xef),
    button_text: Rgb::new(0x20, 0x20, 0x20),
    gray_text: Rgb::new(0x67, 0x67, 0x67),
};

/// What the window draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The theme's name: the setting's, or the one following the system
    /// gave ([`drawn_theme`]).
    pub theme: String,
    /// The owner's colors by role key, `#rrggbb` (the `[colors]` table).
    pub colors: BTreeMap<String, String>,
    /// The system's high contrast colors, while that mode is on: they
    /// win over every theme.
    pub system: Option<SystemColors>,
}

/// The colors the window draws with, one [`Look`] for every role.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The theme's key ("galaxy"), or "system" for the system's high
    /// contrast colors.
    pub key: String,
    /// The theme's name as said ("Galaxy"), or "System high contrast".
    pub name: String,
    /// Dark, light or high contrast: high contrast holds text to 7 to 1.
    pub kind: ThemeKind,
    /// The page: the window's background and the panels'.
    pub page: Rgb,
    /// Plain text on the page.
    pub text: Rgb,
    /// The focus ring and the focused row's outline: 3 to 1 against the
    /// page and the dialog's card.
    pub ring: Rgb,
    /// The line just inside the focused row's outline and a focused
    /// button's ring: 3 to 1 against the focused row's band.
    pub ring_inner: Rgb,
    /// An unavailable row's text on the page; the word "unavailable"
    /// says it too.
    pub dim: Rgb,
    /// Every role's look, in the order of [`PanelRole::ALL`].
    looks: Vec<Look>,
    /// The roles drawn in the text color (or black or white) because
    /// their own color fell short of the floor.
    fell_back: Vec<PanelRole>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::from_theme(diroxide_theme::builtin::default_theme(), &BTreeMap::new())
    }
}

/// `fg` when it meets `floor` on `bg`; else the text color, else black or
/// white, whichever reads better. The flag says whether it fell back.
fn fit(fg: Rgb, bg: Rgb, text: Rgb, floor: f64) -> (Rgb, bool) {
    if contrast_ratio(fg, bg) >= floor {
        return (fg, false);
    }
    if contrast_ratio(text, bg) >= floor {
        return (text, true);
    }
    (black_or_white(bg), true)
}

/// Black or white, whichever has the more contrast on `bg`.
fn black_or_white(bg: Rgb) -> Rgb {
    if contrast_ratio(Rgb::WHITE, bg) >= contrast_ratio(Rgb::BLACK, bg) {
        Rgb::WHITE
    } else {
        Rgb::BLACK
    }
}

/// The first of `candidates` that keeps 3 to 1 against every color in
/// `against`; else the candidate whose worst contrast is the best.
fn indicator(candidates: &[Rgb], against: &[Rgb]) -> Rgb {
    let worst = |c: Rgb| {
        against
            .iter()
            .map(|b| contrast_ratio(c, *b))
            .fold(f64::INFINITY, f64::min)
    };
    candidates
        .iter()
        .copied()
        .find(|c| worst(*c) >= 3.0)
        .unwrap_or_else(|| {
            candidates
                .iter()
                .copied()
                .max_by(|a, b| worst(*a).total_cmp(&worst(*b)))
                .unwrap_or(Rgb::WHITE)
        })
}

impl Palette {
    /// The palette for a choice: the system's high contrast colors while
    /// that mode is on, else the theme with that name (the default when
    /// no theme has it) and the owner's colors.
    pub fn of(choice: &Choice) -> Self {
        if let Some(system) = &choice.system {
            return Self::from_system(system);
        }
        let registry = Registry::builtin();
        let (theme, _) = registry.resolve(&choice.theme);
        Self::from_theme(theme, &choice.colors)
    }

    /// The palette for `theme` with the owner's `[colors]` overrides: a
    /// role's override replaces its text color and keeps its attribute,
    /// as in the terminal.
    pub fn from_theme(theme: &Theme, colors: &BTreeMap<String, String>) -> Self {
        let kind = theme.kind();
        let floor = minimum(Requirement::Text, kind);
        let page = theme.color(ColorRole::Background);
        let mut fell_back = Vec::new();
        let (text, text_fell) = fit(theme.color(ColorRole::Text), page, Rgb::WHITE, floor);
        let text = if text_fell {
            black_or_white(page)
        } else {
            text
        };
        if text_fell {
            fell_back.push(PanelRole::Text);
        }
        let mut looks = Vec::with_capacity(PanelRole::ALL.len());
        for role in PanelRole::ALL {
            let style = role_style(theme, role);
            let attrs = style.attributes;
            if role == PanelRole::Page {
                looks.push(Look {
                    fg: text,
                    bg: page,
                    attrs,
                });
                continue;
            }
            let fg = colors
                .get(role.key())
                .and_then(|c| Rgb::parse(c).ok())
                .unwrap_or(style.foreground);
            let (mut fg, mut fell) = fit(fg, style.background, text, floor);
            let mut bg = style.background;
            if contrast_ratio(fg, bg) < floor {
                // Not even black or white reach the floor on this band (a
                // mid gray in a high-contrast theme): plain text on the
                // page, the attribute still marking the role.
                fg = text;
                bg = page;
                fell = true;
            }
            if fell && role != PanelRole::Text {
                fell_back.push(role);
            }
            looks.push(Look { fg, bg, attrs });
        }
        let mut palette = Self {
            key: theme.name().to_owned(),
            name: theme.meta.display_name.clone(),
            kind,
            page,
            text,
            ring: text,
            ring_inner: page,
            dim: text,
            looks,
            fell_back,
        };
        palette.dim = palette.look(PanelRole::Hidden).fg;
        palette.ring = palette.ring_for(&[text, Rgb::WHITE, Rgb::BLACK]);
        palette.ring_inner = palette.inner_for(&[page, Rgb::BLACK, Rgb::WHITE, text]);
        palette
    }

    /// The palette drawn only with the system's high contrast colors:
    /// the page and its text, the highlight for the focused and marked
    /// rows, the system's link color, its button colors for the key bar
    /// and the dialogs; every other role is plain text, its attribute
    /// marking it.
    pub fn from_system(c: &SystemColors) -> Self {
        let plain = |attrs: Attrs| Look {
            fg: c.window_text,
            bg: c.window,
            attrs,
        };
        let band = |attrs: Attrs| Look {
            fg: c.highlight_text,
            bg: c.highlight,
            attrs,
        };
        let theme = diroxide_theme::builtin::default_theme();
        let looks = PanelRole::ALL
            .into_iter()
            .map(|role| {
                let attrs = role_style(theme, role).attributes;
                match role {
                    PanelRole::FocusedRow
                    | PanelRole::MarkedRow
                    | PanelRole::DefaultButton
                    | PanelRole::SearchMatch
                    | PanelRole::ActiveTitle => band(attrs),
                    PanelRole::Link | PanelRole::DocumentLink | PanelRole::TokenLink => Look {
                        fg: c.hot_light,
                        bg: c.window,
                        attrs,
                    },
                    PanelRole::KeyBar | PanelRole::Dialog => Look {
                        fg: c.button_text,
                        bg: c.button_face,
                        attrs,
                    },
                    PanelRole::Hidden => Look {
                        fg: c.gray_text,
                        bg: c.window,
                        attrs,
                    },
                    _ => plain(attrs),
                }
            })
            .collect();
        let mut palette = Self {
            key: "system".to_owned(),
            name: "System high contrast".to_owned(),
            kind: ThemeKind::HighContrast,
            page: c.window,
            text: c.window_text,
            ring: c.window_text,
            ring_inner: c.window,
            dim: c.gray_text,
            looks,
            fell_back: Vec::new(),
        };
        palette.ring = palette.ring_for(&[c.highlight, c.window_text, c.button_text]);
        palette.ring_inner = palette.inner_for(&[c.highlight_text, c.window, c.window_text]);
        palette
    }

    /// The focus ring: the first candidate that keeps 3 to 1 against the
    /// page and the dialog's card.
    fn ring_for(&self, candidates: &[Rgb]) -> Rgb {
        indicator(candidates, &[self.page, self.look(PanelRole::Dialog).bg])
    }

    /// The line inside the ring: the first candidate that keeps 3 to 1
    /// against the focused row's band.
    fn inner_for(&self, candidates: &[Rgb]) -> Rgb {
        indicator(candidates, &[self.look(PanelRole::FocusedRow).bg])
    }

    /// A role's look.
    pub fn look(&self, role: PanelRole) -> Look {
        PanelRole::ALL
            .iter()
            .position(|r| *r == role)
            .and_then(|i| self.looks.get(i).copied())
            .unwrap_or(Look {
                fg: self.text,
                bg: self.page,
                attrs: Attrs::NONE,
            })
    }

    /// The roles drawn in the text color (or black or white) because
    /// their own color fell short of the floor on their background.
    pub fn fell_back(&self) -> &[PanelRole] {
        &self.fell_back
    }

    /// Every pair the window draws, measured by the theme crate's own
    /// checks: each role's text on its background at the theme's floor;
    /// the page's text, an unavailable row's text; the focus ring at 3 to
    /// 1 against the page and the dialog's card, and its inner line
    /// against the focused row's band.
    pub fn checks(&self) -> Vec<Check> {
        let mut checks = Vec::new();
        let mut pair = |subject: String, key: String, fg: Rgb, bg: Rgb, req: Requirement| {
            let mut c = check_pair(fg, bg, req, self.kind);
            c.subject = subject;
            c.key = key;
            checks.push(c);
        };
        pair(
            "text on the page".into(),
            "window.text".into(),
            self.text,
            self.page,
            Requirement::Text,
        );
        pair(
            "unavailable row on the page".into(),
            "window.dim".into(),
            self.dim,
            self.page,
            Requirement::Text,
        );
        for role in PanelRole::ALL {
            if role == PanelRole::Page {
                continue;
            }
            let look = self.look(role);
            pair(
                format!("{} in the window", role.label()),
                format!("window.{}", role.key()),
                look.fg,
                look.bg,
                Requirement::Text,
            );
        }
        for (what, against) in [
            ("the page", self.page),
            ("a dialog", self.look(PanelRole::Dialog).bg),
        ] {
            pair(
                format!("focus ring against {what}"),
                "window.ring".into(),
                self.ring,
                against,
                Requirement::NonText,
            );
        }
        pair(
            "inner focus line against the focused row".into(),
            "window.ring_inner".into(),
            self.ring_inner,
            self.look(PanelRole::FocusedRow).bg,
            Requirement::NonText,
        );
        checks
    }

    /// The theme crate's report over [`Self::checks`].
    pub fn report(&self) -> ContrastReport {
        ContrastReport {
            theme: self.key.clone(),
            display_name: self.name.clone(),
            checks: self.checks(),
        }
    }

    /// What the window says of its colors when a theme is chosen, after
    /// the theme's own verdict: nothing when every role drew in its own
    /// color, else how many roles use the text color. "In the window, 3
    /// roles use the text color."
    pub fn fallback_words(&self) -> Option<String> {
        match self.fell_back.len() {
            0 => None,
            1 => Some("In the window, 1 role uses the text color.".to_owned()),
            n => Some(format!("In the window, {n} roles use the text color.")),
        }
    }
}

/// The theme the window draws: the setting's, or, while following the
/// system's appearance, the one the system's light, dark or high
/// contrast setting gives (the theme crate's [`follow_os`]: the saved
/// theme when it already matches, else its light or dark partner, else
/// Galaxy, Galaxy Light or High contrast).
pub fn drawn_theme(saved: &str, follow: bool, scheme: OsScheme) -> String {
    if !follow {
        return saved.to_owned();
    }
    let registry = Registry::builtin();
    follow_os(registry.facts(saved), scheme).unwrap_or_else(|| saved.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test the brief names: every built-in theme, drawn by the
    /// window, passes the theme crate's checks on every pair the window
    /// draws; so do the system's high contrast colors, dark and light.
    #[test]
    fn every_theme_passes_the_checks_on_the_windows_roles() {
        let none = BTreeMap::new();
        for theme in diroxide_theme::builtin::all() {
            let palette = Palette::from_theme(theme, &none);
            // The verdicts for the lane's report (shown with
            // --no-capture): the theme crate's, and the window's roles
            // in the text color.
            let fell: Vec<&str> = palette.fell_back().iter().map(|r| r.key()).collect();
            eprintln!(
                "{}: {}; in the text color: {}",
                theme.name(),
                diroxide_theme::verdict(theme),
                if fell.is_empty() {
                    "none".to_owned()
                } else {
                    fell.join(", ")
                }
            );
            let report = palette.report();
            assert!(report.passed(), "{}", report.summary());
            assert_eq!(
                report.checks.len(),
                PanelRole::ALL.len() - 1 + 2 + 3,
                "every role, the text, the dim text and the ring"
            );
        }
        let report = Palette::from_system(&NIGHT_SKY).report();
        assert!(report.passed(), "{}", report.summary());
        // A contrast theme the person chose is honored as it is, even
        // where a pair of its own falls under 7 to 1 (Desert's highlight
        // is 6.9 to 1): the window draws only its colors.
        let desert = Palette::from_system(&DESERT);
        let own = [
            DESERT.window,
            DESERT.window_text,
            DESERT.highlight,
            DESERT.highlight_text,
            DESERT.hot_light,
            DESERT.button_face,
            DESERT.button_text,
            DESERT.gray_text,
        ];
        for role in PanelRole::ALL {
            let look = desert.look(role);
            assert!(own.contains(&look.fg) && own.contains(&look.bg), "{role}");
        }
        assert!(own.contains(&desert.ring) && own.contains(&desert.ring_inner));
    }

    #[test]
    fn an_override_keeps_its_attribute_and_a_weak_one_falls_back() {
        let theme = diroxide_theme::builtin::default_theme();
        let page = theme.color(ColorRole::Background);
        let mut colors = BTreeMap::new();
        colors.insert("folder".to_owned(), "#3b82f6".to_owned());
        let mine = Palette::from_theme(theme, &colors);
        let folder = mine.look(PanelRole::Folder);
        assert!(folder.attrs.bold, "the folder stays bold");
        if contrast_ratio(Rgb::from_u32(0x3b82f6), page) >= 4.5 {
            assert_eq!(folder.fg, Rgb::from_u32(0x3b82f6));
        }
        // The page's own color for a folder: drawn in the text color.
        colors.insert("folder".to_owned(), page.hex());
        let weak = Palette::from_theme(theme, &colors);
        assert_eq!(weak.look(PanelRole::Folder).fg, weak.text);
        assert!(weak.fell_back().contains(&PanelRole::Folder));
        assert_eq!(
            weak.fallback_words().as_deref(),
            Some("In the window, 1 role uses the text color.")
        );
        assert!(weak.report().passed());
    }

    #[test]
    fn the_system_colors_win_and_keep_their_shapes() {
        let choice = Choice {
            theme: "galaxy-light".into(),
            colors: BTreeMap::new(),
            system: Some(NIGHT_SKY),
        };
        let p = Palette::of(&choice);
        assert_eq!(p.key, "system");
        assert_eq!(p.page, NIGHT_SKY.window);
        assert_eq!(p.text, NIGHT_SKY.window_text);
        assert_eq!(p.look(PanelRole::FocusedRow).bg, NIGHT_SKY.highlight);
        assert_eq!(p.look(PanelRole::Link).fg, NIGHT_SKY.hot_light);
        assert!(p.look(PanelRole::Folder).attrs.bold, "a folder stays bold");
        assert!(p.look(PanelRole::Link).attrs.underline);
        // The ring is the highlight where it shows on the page and the
        // card; the line inside it shows on the highlight.
        assert_eq!(p.ring, NIGHT_SKY.highlight);
        assert!(contrast_ratio(p.ring_inner, NIGHT_SKY.highlight) >= 3.0);
    }

    #[test]
    fn following_the_system_stays_in_the_pair_or_takes_galaxy() {
        assert_eq!(drawn_theme("nord", false, OsScheme::Light), "nord");
        assert_eq!(drawn_theme("galaxy", true, OsScheme::Light), "galaxy-light");
        assert_eq!(
            drawn_theme("solarized-dark", true, OsScheme::Light),
            "solarized-light"
        );
        assert_eq!(drawn_theme("galaxy", true, OsScheme::Dark), "galaxy");
        assert_eq!(
            drawn_theme("galaxy", true, OsScheme::HighContrast),
            "high-contrast"
        );
        assert_eq!(drawn_theme("nord", true, OsScheme::Unknown), "nord");
    }

    #[test]
    fn the_default_palette_is_galaxy() {
        let p = Palette::default();
        assert_eq!(p.key, "galaxy");
        assert_eq!(p.name, "Galaxy");
        assert!(p.fell_back().is_empty(), "{:?}", p.fell_back());
    }
}
