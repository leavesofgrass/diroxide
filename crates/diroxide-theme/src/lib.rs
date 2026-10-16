// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Themes (ADR-0020): one palette per theme, checked for WCAG contrast, and
//! rendered for the terminal UI, the GUI, and HTML output.
//!
//! - [`Theme`] names a color for every semantic role ([`ColorRole`]: page,
//!   text, dim text, six heading levels, links, code, quotes, errors, panels)
//!   and a foreground, background, and attributes for every highlight and
//!   chrome style ([`StyleRole`]: selection, spoken word and sentence, find
//!   matches, bookmarks, notes, status bar, focus), plus the reader's
//!   highlight colors. Every highlight carries a text attribute, so no state
//!   is shown by color alone. Fifteen interface roles (raised fills,
//!   borders, the accent, the caret, the reading ruler, document marks) are
//!   derived from those colors at load unless the file gives them; each
//!   role has a [`RoleClass`] that decides its check.
//! - Themes are TOML files ([`ThemeFile`]). star's 23 palettes are built in
//!   ([`builtin`], ported by [`star`]); user themes load from the config
//!   folder's `themes/` directory ([`Registry::load_dir`]), with errors
//!   reported per file and unknown keys kept.
//! - [`check()`] measures every pair against WCAG 2.x (4.5:1 text, 7:1 in
//!   high-contrast themes, 3:1 for indicators) and reports APCA for
//!   information. Built-in themes pass; user themes are warned about, never
//!   blocked or silently changed.
//! - Renderers: [`terminal`] (truecolor, 256, 16, and no-color levels),
//!   [`css`] (custom properties with `prefers-color-scheme` pairs), and
//!   [`Theme::rgb_table`] for the GUI.
//! - [`os`] follows the system's light, dark, or high-contrast setting.
//! - [`roles`] adds diroxide's own roles, the file manager's screen
//!   ([`PanelRole`]), each a color pair and an attribute drawn from the
//!   theme, and the verdict in words: "meets AA" or "below AA: 2 checks
//!   fall short".
//! - [`reading`] lays the reader's `[highlight]` colors over a theme's
//!   spoken-word and spoken-sentence styles, with a contrast warning.

pub mod builtin;
pub mod check;
pub mod color;
pub mod css;
mod error;
pub mod file;
mod model;
pub mod os;
pub mod reading;
pub mod registry;
pub mod resolve;
pub mod roles;
pub mod star;
pub mod terminal;

pub use check::{Check, ContrastReport, Requirement, check};
pub use color::{Rgb, apca_lc, contrast_ratio};
pub use error::ThemeError;
pub use file::ThemeFile;
pub use model::{
    Attrs, ColorRole, Meta, Resolved, RoleClass, Style, StyleRole, Theme, ThemeKind,
    USER_HIGHLIGHT_NO_COLOR, UserHighlight,
};
pub use os::OsScheme;
pub use registry::{LoadReport, Registry};
pub use resolve::{Adjustment, Repair};
pub use roles::{PanelRole, RoleStyle, panel_checks, role_style, verdict};
pub use terminal::{ColorSupport, TermColor, TermStyle, TerminalTheme, term_color};

/// The theme used when none is chosen or the chosen one is missing.
pub const DEFAULT_THEME: &str = "galaxy";
