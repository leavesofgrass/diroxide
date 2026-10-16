//! The theme settings over the theme crate (plan section 5.10): the theme
//! chosen by name, the owner's color for any screen role, and the contrast
//! verdict in words, with every override checked again.
//!
//! An override names a role of [`PanelRole`] and a color: `folder =
//! "#ffaa00"` under `[colors]`. The role keeps its attribute (bold,
//! underline, italic, reverse), so nothing depends on the color alone;
//! the color is checked against the role's background at the theme's
//! floor (4.5 to 1, or 7 to 1 in a high-contrast theme) and the verdict
//! counts it: "meets AA", or "below AA: 2 checks fall short".

use std::collections::BTreeMap;
use std::path::Path;

use diroxide_theme::check::{Requirement, check_pair};
use diroxide_theme::{
    Check, DEFAULT_THEME, PanelRole, Registry, Rgb, Theme, ThemeKind, check, panel_checks,
    role_style,
};

use crate::store::ConfigProblem;

/// One override: a role and the owner's color for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorOverride {
    /// The screen role.
    pub role: PanelRole,
    /// The owner's color for its text.
    pub color: Rgb,
}

/// Reads the `[colors]` table: each known role with a color in `#rrggbb`
/// form becomes an override; anything else is a problem in words and is
/// left in the file as it is.
pub fn read_overrides(
    colors: &BTreeMap<String, String>,
    file: &str,
) -> (Vec<ColorOverride>, Vec<ConfigProblem>) {
    let mut overrides = Vec::new();
    let mut problems = Vec::new();
    for (key, value) in colors {
        let Some(role) = PanelRole::from_key(key) else {
            problems.push(ConfigProblem::new(
                file,
                format!("Unknown color role: {key}. Kept as it is."),
            ));
            continue;
        };
        match Rgb::parse(value) {
            Ok(color) => overrides.push(ColorOverride { role, color }),
            Err(_) => problems.push(ConfigProblem::new(
                file,
                format!("colors.{key} cannot be \"{value}\": write it as #rrggbb."),
            )),
        }
    }
    (overrides, problems)
}

/// The built-in themes and the owner's theme files in `themes_dir`, with
/// a problem in words for each file that could not be loaded.
pub fn registry(themes_dir: &Path, file: &str) -> (Registry, Vec<ConfigProblem>) {
    let mut registry = Registry::builtin();
    let report = registry.load_dir(themes_dir);
    let problems = report
        .errors
        .iter()
        .map(|(path, error)| {
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ConfigProblem::new(file, format!("Theme file {name} was not loaded: {error}."))
        })
        .collect();
    (registry, problems)
}

/// The names of the built-in themes, in cycle order.
pub fn builtin_names() -> Vec<String> {
    Registry::builtin()
        .names()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// The checks of the overridden roles: each override's color on the
/// role's background, at the theme's text floor.
pub fn override_checks(theme: &Theme, overrides: &[ColorOverride]) -> Vec<Check> {
    overrides
        .iter()
        .filter(|o| o.role != PanelRole::Page)
        .map(|o| {
            let style = role_style(theme, o.role);
            let mut c = check_pair(o.color, style.background, Requirement::Text, theme.kind());
            c.subject = o.role.label().to_owned();
            c.key = format!("colors.{}", o.role.key());
            c
        })
        .collect()
}

/// The verdict in words with the overrides applied: "meets AA", or "below
/// AA: 2 checks fall short" (AAA in a high-contrast theme). A role the
/// owner colored is checked with the owner's color in place of the
/// theme's; one weak color pair counts once.
pub fn verdict(theme: &Theme, overrides: &[ColorOverride]) -> String {
    let report = check(theme);
    let mut counted: Vec<(Option<Rgb>, Option<Rgb>)> = report
        .failures()
        .map(|c| (c.foreground, c.background))
        .collect();
    let mut falling_short = counted.len();
    let overridden: Vec<PanelRole> = overrides.iter().map(|o| o.role).collect();
    let theirs = panel_checks(theme).into_iter().filter(|c| {
        !overridden
            .iter()
            .any(|role| c.key == format!("panel.{}", role.key()))
    });
    for c in theirs.chain(override_checks(theme, overrides)) {
        let pair = (c.foreground, c.background);
        if !c.passed && !counted.contains(&pair) {
            counted.push(pair);
            falling_short += 1;
        }
    }
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

/// What the Settings list and the status line say when a theme is chosen,
/// its name first: "Theme Nord, meets AA", "Theme Mine, below AA: 2
/// checks fall short".
pub fn chosen_line(theme: &Theme, overrides: &[ColorOverride]) -> String {
    format!(
        "Theme {}, {}",
        theme.meta.display_name,
        verdict(theme, overrides)
    )
}

/// The theme a name gives, or the default with a problem in words when
/// no theme has that name.
pub fn resolve<'a>(
    registry: &'a Registry,
    name: &str,
    file: &str,
) -> (&'a Theme, Option<ConfigProblem>) {
    let (theme, fell_back) = registry.resolve(name);
    let problem = fell_back.then(|| {
        ConfigProblem::new(
            file,
            format!(
                "Unknown theme: {name}. {} is in use.",
                capitalized(DEFAULT_THEME)
            ),
        )
    });
    (theme, problem)
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_theme::builtin;

    fn colors(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn overrides_are_read_and_mistakes_said() {
        let (overrides, problems) = read_overrides(
            &colors(&[
                ("folder", "#ffaa00"),
                ("fodler", "#ffaa00"),
                ("link", "orange"),
            ]),
            "config.toml",
        );
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].role, PanelRole::Folder);
        let said: Vec<&str> = problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "Unknown color role: fodler. Kept as it is.",
                "colors.link cannot be \"orange\": write it as #rrggbb."
            ]
        );
        for p in &problems {
            assert!(within_cells(&p.text, "color") || within_cells(&p.text, "colors."));
        }
    }

    #[test]
    fn a_low_contrast_override_is_below_aa_and_says_how_many() {
        let theme = builtin::default_theme();
        assert_eq!(verdict(theme, &[]), "meets AA");
        let page = theme.color(diroxide_theme::ColorRole::Background);
        let weak = [ColorOverride {
            role: PanelRole::Folder,
            color: page,
        }];
        assert_eq!(verdict(theme, &weak), "below AA: 1 check falls short");
        let two = [
            weak[0],
            ColorOverride {
                role: PanelRole::Link,
                color: Rgb::new(page.r.saturating_add(1), page.g, page.b),
            },
        ];
        assert_eq!(verdict(theme, &two), "below AA: 2 checks fall short");
        let line = chosen_line(theme, &weak);
        assert!(line.starts_with("Theme Galaxy, below AA"), "{line}");
        let checks = override_checks(theme, &weak);
        assert!(
            checks[0]
                .describe()
                .starts_with("Folder: 1.0 to 1, needs 4.5 to 1")
        );
    }

    #[test]
    fn a_strong_override_keeps_the_verdict() {
        let theme = builtin::default_theme();
        let text = theme.color(diroxide_theme::ColorRole::Text);
        let strong = [ColorOverride {
            role: PanelRole::Folder,
            color: text,
        }];
        assert_eq!(verdict(theme, &strong), "meets AA");
    }

    #[test]
    fn an_unknown_theme_falls_back_and_says_so() {
        let registry = Registry::builtin();
        let (theme, problem) = resolve(&registry, "nord", "config.toml");
        assert_eq!(theme.name(), "nord");
        assert!(problem.is_none());
        let (theme, problem) = resolve(&registry, "no-such-theme", "config.toml");
        assert_eq!(theme.name(), DEFAULT_THEME);
        assert_eq!(
            problem.unwrap().text,
            "Unknown theme: no-such-theme. Galaxy is in use."
        );
        assert!(builtin_names().len() >= 24);
    }
}
