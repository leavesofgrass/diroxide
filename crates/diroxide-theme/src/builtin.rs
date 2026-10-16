// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The built-in themes: star's 23 palettes, ported and embedded as TOML
//! files (`crates/diroxide-theme/themes/*.toml`), plus textweaver's own
//! (Lamplight). star's files are generated from [`crate::star`] and a test
//! keeps them in step; textweaver's own files are hand-written.
//!
//! [`CYCLE`] is the order the Next theme command steps through: the themes that meet WCAG AA
//! first, light and dark partners side by side, then the ten below AA.

use std::sync::OnceLock;

use crate::file::ThemeFile;
use crate::model::Theme;
use crate::resolve::{Repair, resolve};

macro_rules! sources {
    ($($name:literal),+ $(,)?) => {
        /// star's built-in theme names, in star's order (galaxy first).
        pub const NAMES: [&str; 23] = [$($name),+];
        const STAR_SOURCES: [(&str, &str); 23] = [
            $(($name, include_str!(concat!("../themes/", $name, ".toml")))),+
        ];
    };
}

sources!(
    "galaxy",
    "galaxy-light",
    "one-dark",
    "one-light",
    "dark",
    "light",
    "contrast",
    "high-contrast",
    "phosphor",
    "dracula",
    "nord",
    "solarized-dark",
    "solarized-light",
    "gruvbox-dark",
    "tokyo-night",
    "catppuccin-mocha",
    "monokai",
    "sepia",
    "amber",
    "everforest-dark",
    "rose-pine",
    "kanagawa",
    "gruvbox-light",
);

/// Built-in themes that are textweaver's own, not ported from star.
pub const OWN: [&str; 1] = ["lamplight"];

const OWN_SOURCES: [(&str, &str); 1] = [("lamplight", include_str!("../themes/lamplight.toml"))];

/// Every built-in theme in cycle order: the 14 that meet WCAG AA first, light
/// and dark partners side by side, then the 10 below AA, partners side by
/// side.
pub const CYCLE: [&str; 24] = [
    "galaxy",
    "galaxy-light",
    "high-contrast",
    "contrast",
    "lamplight",
    "sepia",
    "gruvbox-dark",
    "gruvbox-light",
    "tokyo-night",
    "catppuccin-mocha",
    "dracula",
    "rose-pine",
    "kanagawa",
    "amber",
    "one-dark",
    "one-light",
    "dark",
    "light",
    "solarized-dark",
    "solarized-light",
    "nord",
    "everforest-dark",
    "monokai",
    "phosphor",
];

fn sources() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    STAR_SOURCES.iter().chain(OWN_SOURCES.iter())
}

/// The TOML text of a built-in theme, for exporting or copying.
pub fn source(name: &str) -> Option<&'static str> {
    let name = canonical_name(name)?;
    sources().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// The built-in name for `name`: case-insensitive, with star's old names
/// (`obsidian` → `galaxy`) and alternate spellings (`high_contrast`).
pub fn canonical_name(name: &str) -> Option<&'static str> {
    let n = name.trim().to_ascii_lowercase();
    let n = crate::star::ALIASES
        .iter()
        .find(|(old, _)| *old == n)
        .map_or(n.as_str(), |(_, new)| *new);
    CYCLE.iter().copied().find(|b| *b == n)
}

/// Every built-in theme, parsed once, in cycle order ([`CYCLE`]).
pub fn all() -> &'static [Theme] {
    static ALL: OnceLock<Vec<Theme>> = OnceLock::new();
    ALL.get_or_init(|| {
        CYCLE
            .iter()
            .filter_map(|name| sources().find(|(n, _)| n == name))
            .filter_map(|(name, src)| {
                let file = ThemeFile::parse(src).ok()?;
                resolve(&file, None, Some(name), Repair::DerivedOnly)
                    .ok()
                    .map(|(t, _)| t)
            })
            .collect()
    })
}

/// A built-in theme by name (aliases and any case accepted).
pub fn get(name: &str) -> Option<&'static Theme> {
    let name = canonical_name(name)?;
    all().iter().find(|t| t.meta.name == name)
}

/// The default theme, `galaxy`.
#[allow(
    clippy::expect_used,
    reason = "the themes are constants compiled in and parsed by this crate's tests, never user input"
)]
pub fn default_theme() -> &'static Theme {
    // The embedded files are constants checked by this crate's tests.
    get(crate::DEFAULT_THEME)
        .or_else(|| all().first())
        .expect("built-in themes are embedded constants and always parse")
}
