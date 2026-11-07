//! The theme in the terminal: each part of the screen ([`PanelRole`]) gets
//! the theme's colors and its attribute, at the terminal's color level.
//!
//! The rules (plan section 5.10, the owner's decision on themes): every
//! state also carries an attribute or a text marker, never color alone;
//! the owner's `[colors]` overrides replace a role's text color and keep
//! its attribute; `NO_COLOR` (or `TERM=dumb`, or `DIRO_COLOR=none`) leaves
//! attributes alone, with the focused row reversed and marked rows bold as
//! a terminal without color has always shown them. With 16 colors the
//! terminal's own palette is unknown, so the screen keeps the terminal's
//! colors and draws attributes only.
//!
//! A theme file's `[group-colors]` (`App::row_role`) draws a plain file's
//! or an archive's row in its group's color role, as the window does: the
//! row's words say its type and group either way, so the color is an
//! extra, held to the theme's text floor on the row's background, and
//! left out without color, at 16 colors and in a high contrast theme.

use std::collections::BTreeMap;
use std::path::Path;

use diroxide_theme::check::{Requirement, minimum};
use diroxide_theme::{
    Attrs, ColorRole, ColorSupport, PanelRole, Registry, Rgb, TermColor, ThemeKind, contrast_ratio,
    role_style, term_color,
};
use ratatui::style::{Color, Modifier, Style};

/// The roles a file group's color is drawn on: a plain file's row and an
/// archive's (the window's rule, `crates/diroxide-gui/src/theme.rs`).
const GROUP_ROLES: [PanelRole; 2] = [PanelRole::Text, PanelRole::Archive];

/// The styles of one theme at one color level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Styles {
    support: ColorSupport,
    theme: String,
    colors: BTreeMap<String, String>,
    roles: Vec<(PanelRole, Style)>,
    /// A file group's look on a plain file's or an archive's row, by the
    /// color role a theme file's `[group-colors]` maps the group to
    /// (`App::row_role`, TA's request 2): the row's own style with the
    /// role's color as its text, held to the theme's text floor on the
    /// row's background. Empty without color, at 16 colors and in a high
    /// contrast theme, where such rows stay plain.
    groups: Vec<(ColorRole, PanelRole, Style)>,
}

/// `fg` when it keeps `floor` on `bg`; else the row's own text color,
/// else black or white, whichever reads on `bg`.
fn fit(fg: Rgb, bg: Rgb, text: Rgb, floor: f64) -> Rgb {
    if contrast_ratio(fg, bg) >= floor {
        fg
    } else if contrast_ratio(text, bg) >= floor {
        text
    } else if contrast_ratio(Rgb::WHITE, bg) >= contrast_ratio(Rgb::BLACK, bg) {
        Rgb::WHITE
    } else {
        Rgb::BLACK
    }
}

fn color(rgb: Rgb, support: ColorSupport) -> Option<Color> {
    match term_color(rgb, support)? {
        TermColor::Rgb(c) => Some(Color::Rgb(c.r, c.g, c.b)),
        TermColor::Indexed(i) => Some(Color::Indexed(i)),
    }
}

fn modifiers(attrs: Attrs) -> Modifier {
    let mut m = Modifier::empty();
    if attrs.bold {
        m |= Modifier::BOLD;
    }
    if attrs.italic {
        m |= Modifier::ITALIC;
    }
    if attrs.underline {
        m |= Modifier::UNDERLINED;
    }
    if attrs.reverse {
        m |= Modifier::REVERSED;
    }
    m
}

/// The attribute a role keeps with no color at all: the focused row is
/// reversed, a marked row bold, the active title bold, the default button
/// reversed and bold; the rest keep the theme's own attribute.
fn no_color_modifier(role: PanelRole, theirs: Modifier) -> Modifier {
    match role {
        PanelRole::FocusedRow => Modifier::REVERSED,
        PanelRole::MarkedRow | PanelRole::ActiveTitle => Modifier::BOLD,
        PanelRole::DefaultButton => Modifier::REVERSED | Modifier::BOLD,
        _ => theirs,
    }
}

impl Styles {
    /// The styles of the theme named `theme` (a built-in one, or a file in
    /// `themes_dir`), with the owner's `[colors]` overrides, at a color
    /// level. An unknown name gives the default theme, as the startup
    /// problems already said.
    pub fn new(
        theme: &str,
        colors: &BTreeMap<String, String>,
        themes_dir: Option<&Path>,
        support: ColorSupport,
    ) -> Self {
        let mut registry = Registry::builtin();
        if let Some(dir) = themes_dir {
            let _ = registry.load_dir(dir);
        }
        let (chosen, _) = registry.resolve(theme);
        let mut roles = Vec::new();
        for role in PanelRole::ALL {
            let mut rs = role_style(chosen, role);
            if let Some(over) = colors.get(role.key()).and_then(|c| Rgb::parse(c).ok()) {
                rs.foreground = over;
            }
            let mut modifier = modifiers(rs.attributes);
            if role == PanelRole::FocusedRow && modifier.is_empty() {
                // The theme marks focus by its colors alone; bold keeps it
                // visible where the colors are not told apart.
                modifier = Modifier::BOLD;
            }
            let style = match support {
                ColorSupport::NoColor => {
                    Style::default().add_modifier(no_color_modifier(role, modifier))
                }
                ColorSupport::Ansi16 => {
                    let modifier = match role {
                        PanelRole::FocusedRow => modifier | Modifier::REVERSED,
                        _ => modifier,
                    };
                    Style::default().add_modifier(modifier)
                }
                ColorSupport::Ansi256 | ColorSupport::TrueColor => {
                    let mut style = Style::default().add_modifier(modifier);
                    if let Some(fg) = color(rs.foreground, support) {
                        style = style.fg(fg);
                    }
                    if let Some(bg) = color(rs.background, support) {
                        style = style.bg(bg);
                    }
                    style
                }
            };
            roles.push((role, style));
        }
        // A group's color is an extra on a row that already says its
        // type and group in words: drawn only with real colors, never in
        // a high contrast theme, and held to the text floor.
        let mut groups = Vec::new();
        let colored = matches!(support, ColorSupport::Ansi256 | ColorSupport::TrueColor);
        if colored && chosen.kind() != ThemeKind::HighContrast {
            let floor = minimum(Requirement::Text, chosen.kind());
            for role in GROUP_ROLES {
                let rs = role_style(chosen, role);
                let base = roles
                    .iter()
                    .find(|(r, _)| *r == role)
                    .map_or_else(Style::default, |(_, s)| *s);
                for group in ColorRole::ALL {
                    let fg = fit(chosen.color(*group), rs.background, rs.foreground, floor);
                    if let Some(fg) = color(fg, support) {
                        groups.push((*group, role, base.fg(fg)));
                    }
                }
            }
        }
        Self {
            support,
            theme: theme.to_owned(),
            colors: colors.clone(),
            roles,
            groups,
        }
    }

    /// The look of a row in `role` whose file group the theme maps to
    /// `group`: `None` for any other role, and wherever groups stay plain
    /// (no color, 16 colors, a high contrast theme), so the row keeps its
    /// role's own style.
    pub fn group(&self, group: ColorRole, role: PanelRole) -> Option<Style> {
        self.groups
            .iter()
            .find(|(g, r, _)| *g == group && *r == role)
            .map(|(_, _, s)| *s)
    }

    /// Whether these styles were made for this theme, these colors and
    /// this level, so a changed setting makes new ones.
    pub fn matches(&self, theme: &str, colors: &BTreeMap<String, String>) -> bool {
        self.theme == theme && self.colors == *colors
    }

    /// The color level.
    pub fn support(&self) -> ColorSupport {
        self.support
    }

    /// A role's style.
    pub fn get(&self, role: PanelRole) -> Style {
        self.roles
            .iter()
            .find(|(r, _)| *r == role)
            .map_or_else(Style::default, |(_, s)| *s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_color_every_highlight_keeps_an_attribute() {
        let styles = Styles::new("galaxy", &BTreeMap::new(), None, ColorSupport::NoColor);
        for role in [
            PanelRole::FocusedRow,
            PanelRole::MarkedRow,
            PanelRole::ActiveTitle,
            PanelRole::DefaultButton,
            PanelRole::Folder,
            PanelRole::Link,
            PanelRole::BrokenLink,
            PanelRole::Hidden,
        ] {
            let style = styles.get(role);
            assert!(!style.add_modifier.is_empty(), "{role} has no attribute");
            assert_eq!(style.fg, None, "{role} has a color");
            assert_eq!(style.bg, None, "{role} has a color");
        }
        assert!(
            styles
                .get(PanelRole::FocusedRow)
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn a_theme_gives_colors_and_an_override_changes_one() {
        let plain = Styles::new("nord", &BTreeMap::new(), None, ColorSupport::TrueColor);
        assert!(plain.get(PanelRole::Folder).fg.is_some());
        let mut colors = BTreeMap::new();
        colors.insert("folder".to_owned(), "#ffaa00".to_owned());
        let mine = Styles::new("nord", &colors, None, ColorSupport::TrueColor);
        assert_eq!(
            mine.get(PanelRole::Folder).fg,
            Some(Color::Rgb(0xff, 0xaa, 0x00))
        );
        assert_eq!(
            mine.get(PanelRole::Folder).add_modifier,
            plain.get(PanelRole::Folder).add_modifier
        );
        assert!(mine.matches("nord", &colors));
        assert!(!mine.matches("galaxy", &colors));
        let indexed = Styles::new("nord", &BTreeMap::new(), None, ColorSupport::Ansi256);
        assert!(matches!(
            indexed.get(PanelRole::Text).fg,
            Some(Color::Indexed(_))
        ));
    }
}
