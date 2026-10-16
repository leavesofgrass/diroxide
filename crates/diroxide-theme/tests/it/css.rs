// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! CSS output: a snapshot of the default stylesheet (Galaxy, Galaxy Light,
//! High Contrast), and the properties every template can rely on.

use diroxide_theme::css::{
    SchemeSet, default_stylesheet, single_stylesheet, stylesheet, variables,
};
use diroxide_theme::{ColorRole, StyleRole, builtin};

#[test]
fn default_stylesheet_snapshot() {
    insta::assert_snapshot!("galaxy_default", default_stylesheet());
}

#[test]
fn sepia_variables_snapshot() {
    insta::assert_snapshot!("sepia_variables", variables(builtin::get("sepia").unwrap()));
}

#[test]
fn galaxy_is_the_default_for_html() {
    let css = default_stylesheet();
    let root = css.split("@media").next().unwrap();
    assert!(root.contains("--tw-background: #1e1e1e;"), "{root}");
    assert!(root.contains("color-scheme: dark;"));
    assert!(css.contains("@media (prefers-color-scheme: light)"));
    assert!(!css.contains("@media (prefers-color-scheme: dark)"));
    // A light-first set flips the blocks.
    let light_first = stylesheet(SchemeSet {
        prefer_dark: false,
        ..SchemeSet::galaxy()
    });
    let root = light_first.split("@media").next().unwrap();
    assert!(root.contains("--tw-background: #ffffff;"), "{root}");
    assert!(light_first.contains("@media (prefers-color-scheme: dark)"));
}

#[test]
fn every_role_has_a_property_in_every_block() {
    let css = default_stylesheet();
    for &r in ColorRole::ALL {
        let p = format!("--tw-{}:", r.key().replace('_', "-"));
        // preferred, other scheme, more contrast, forced colors
        assert_eq!(css.matches(&p).count(), 4, "{p}");
    }
    for &r in StyleRole::ALL {
        let p = format!("--tw-{}-fg:", r.key().replace('_', "-"));
        assert_eq!(css.matches(&p).count(), 4, "{p}");
    }
    assert!(css.contains("@media (prefers-contrast: more)"));
    assert!(css.contains("@media (forced-colors: active)"));
    assert!(css.contains("a { color: var(--tw-link); text-decoration: underline; }"));
    assert!(css.contains(".tw-highlight-yellow {"));
    assert!(css.contains(":focus-visible"));
    assert!(!css.contains("outline: none"));
}

/// Every `var(--tw-…)` a rule reads is a property the stylesheet defines,
/// and each role's value is its color in hex.
#[test]
fn every_variable_used_is_defined() {
    let css = default_stylesheet();
    for used in css.split("var(").skip(1) {
        let name = used.split([')', ',']).next().unwrap().trim();
        assert!(css.contains(&format!("{name}:")), "{name} is never defined");
    }
    let g = builtin::get("galaxy").unwrap();
    let root = css.split("@media").next().unwrap();
    for &r in ColorRole::ALL {
        let line = format!("--tw-{}: {};", r.key().replace('_', "-"), g.color(r));
        assert!(root.contains(&line), "{line}");
    }
}

#[test]
fn a_chosen_theme_does_not_follow_the_system() {
    let css = single_stylesheet(builtin::get("nord").unwrap());
    assert!(!css.contains("prefers-color-scheme"));
    assert!(css.contains("--tw-background: #2e3440;"));
    assert!(css.contains("@media (forced-colors: active)"));
}
