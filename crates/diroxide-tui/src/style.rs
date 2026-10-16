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

use std::collections::BTreeMap;
use std::path::Path;

use diroxide_theme::{
    Attrs, ColorSupport, PanelRole, Registry, Rgb, TermColor, role_style, term_color,
};
use ratatui::style::{Color, Modifier, Style};

/// The styles of one theme at one color level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Styles {
    support: ColorSupport,
    theme: String,
    colors: BTreeMap<String, String>,
    roles: Vec<(PanelRole, Style)>,
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
        Self {
            support,
            theme: theme.to_owned(),
            colors: colors.clone(),
            roles,
        }
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
