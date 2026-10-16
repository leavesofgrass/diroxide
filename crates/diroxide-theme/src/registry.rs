// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The themes available to the reader: the built-ins plus the user's
//! `themes/` folder, looked up by name and cycled in order.

use std::path::{Path, PathBuf};

use crate::builtin;
use crate::check::{ContrastReport, check};
use crate::error::ThemeError;
use crate::file::{MAX_FILE_BYTES, ThemeFile};
use crate::model::Theme;
use crate::os::ThemeFacts;
use crate::resolve::{Repair, resolve};

/// What happened when a themes folder was loaded.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// Names of the themes loaded, in file-name order.
    pub loaded: Vec<String>,
    /// Built-in themes a user theme of the same name replaced.
    pub replaced: Vec<String>,
    /// Files that could not be loaded, and why.
    pub errors: Vec<(PathBuf, ThemeError)>,
    /// Contrast reports for loaded themes that fail a check. The themes are
    /// still usable: diroxide warns, never blocks.
    pub warnings: Vec<ContrastReport>,
}

impl LoadReport {
    /// True when every file loaded and passed every check.
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty()
    }

    /// Sentences for the status line and screen readers, for example
    /// `Loaded 2 user themes. ocean.toml was not loaded: colors.text: …`
    pub fn summary(&self) -> String {
        let n = self.loaded.len();
        let mut s = match n {
            0 => "No user themes loaded.".to_owned(),
            1 => "Loaded 1 user theme.".to_owned(),
            _ => format!("Loaded {n} user themes."),
        };
        for name in &self.replaced {
            s.push_str(&format!(" Your {name} replaces the built-in one."));
        }
        for (path, e) in &self.errors {
            let file = path.file_name().map_or_else(
                || path.display().to_string(),
                |f| f.to_string_lossy().into_owned(),
            );
            s.push_str(&format!(" {file} was not loaded: {e}."));
        }
        for w in &self.warnings {
            s.push(' ');
            s.push_str(&w.summary());
        }
        s
    }
}

/// All themes: built-ins first in star's order, then user themes by file
/// name. A user theme with a built-in's name takes its place.
#[derive(Clone, Debug)]
pub struct Registry {
    themes: Vec<Theme>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Registry {
    /// The built-in themes only.
    pub fn builtin() -> Self {
        Registry {
            themes: builtin::all().to_vec(),
        }
    }

    /// Every theme, in cycle order.
    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    /// Every theme name, in cycle order.
    pub fn names(&self) -> Vec<&str> {
        self.themes.iter().map(|t| t.meta.name.as_str()).collect()
    }

    fn position(&self, name: &str) -> Option<usize> {
        let n = name.trim().to_ascii_lowercase();
        let n = builtin::canonical_name(&n).map_or(n, str::to_owned);
        self.themes.iter().position(|t| t.meta.name == n)
    }

    /// A theme by name: any case, and star's old names accepted.
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.position(name).map(|i| &self.themes[i])
    }

    /// The theme for `name`, or the default when there is none; the flag is
    /// true when it fell back, so the caller can say so (star fell back
    /// silently).
    pub fn resolve(&self, name: &str) -> (&Theme, bool) {
        match self.get(name) {
            Some(t) => (t, false),
            None => (
                self.get(crate::DEFAULT_THEME)
                    .unwrap_or_else(|| builtin::default_theme()),
                true,
            ),
        }
    }

    /// The theme after `current` in cycle order. An unknown `current` gives
    /// the first theme (star skipped to the second).
    pub fn next(&self, current: &str) -> &Theme {
        self.step(current, 1)
    }

    /// The theme before `current` in cycle order.
    pub fn previous(&self, current: &str) -> &Theme {
        self.step(current, -1)
    }

    fn step(&self, current: &str, by: isize) -> &Theme {
        let len = self.themes.len();
        if len == 0 {
            return builtin::default_theme();
        }
        let i = match self.position(current) {
            Some(i) => (i as isize + by).rem_euclid(len as isize) as usize,
            None => 0,
        };
        &self.themes[i]
    }

    /// Kind and partner of a theme, for [`crate::os::follow_os`].
    pub fn facts(&self, name: &str) -> Option<ThemeFacts<'_>> {
        self.get(name).map(|t| ThemeFacts {
            kind: t.meta.kind,
            counterpart: t.meta.counterpart.as_deref(),
        })
    }

    /// Adds a theme, replacing any theme of the same name in place. Returns
    /// the replaced theme.
    pub fn add(&mut self, theme: Theme) -> Option<Theme> {
        match self
            .themes
            .iter()
            .position(|t| t.meta.name == theme.meta.name)
        {
            Some(i) => Some(std::mem::replace(&mut self.themes[i], theme)),
            None => {
                self.themes.push(theme);
                None
            }
        }
    }

    /// Loads one theme file. `inherits` may name any built-in theme.
    pub fn load_file(path: &Path) -> Result<Theme, ThemeError> {
        let io = |source| ThemeError::Io {
            path: path.to_owned(),
            source,
        };
        let len = std::fs::metadata(path).map_err(io)?.len();
        if len > MAX_FILE_BYTES {
            return Err(ThemeError::TooLarge(path.to_owned()));
        }
        let text = std::fs::read_to_string(path).map_err(io)?;
        let file = ThemeFile::parse(&text)?;
        let base = match &file.inherits {
            Some(b) => Some(builtin::get(b).ok_or_else(|| ThemeError::UnknownBase(b.clone()))?),
            None => None,
        };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        let (theme, _) = resolve(&file, base, stem.as_deref(), Repair::DerivedOnly)?;
        Ok(theme)
    }

    /// Loads every `*.toml` file in `dir` (not subfolders), in file-name
    /// order. A missing folder loads nothing. Each file stands alone: one
    /// bad file is reported and the rest still load.
    pub fn load_dir(&mut self, dir: &Path) -> LoadReport {
        let mut report = LoadReport::default();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return report;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
            })
            .collect();
        paths.sort();
        let mut seen: Vec<(String, PathBuf)> = Vec::new();
        for path in paths {
            let theme = match Self::load_file(&path) {
                Ok(t) => t,
                Err(e) => {
                    report.errors.push((path, e));
                    continue;
                }
            };
            let name = theme.meta.name.clone();
            if let Some((_, first)) = seen.iter().find(|(n, _)| *n == name) {
                let first = first
                    .file_name()
                    .map_or_else(String::new, |f| f.to_string_lossy().into_owned());
                report.errors.push((
                    path,
                    ThemeError::invalid(
                        "theme.name",
                        format!("{name} is already used by {first}; give this theme another name"),
                    ),
                ));
                continue;
            }
            let r = check(&theme);
            if !r.passed() {
                report.warnings.push(r);
            }
            if self.add(theme).is_some() {
                report.replaced.push(name.clone());
            }
            seen.push((name.clone(), path));
            report.loaded.push(name);
        }
        report
    }
}
