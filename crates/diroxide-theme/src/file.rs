// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! The theme file format: TOML with `[theme]`, `[colors]`, `[styles.<role>]`
//! and `[[user_highlights]]`. Every key is optional in the file; the resolver
//! ([`crate::resolve`]) fills in what is missing. Keys this version does not
//! know are kept and written back unchanged.

use toml::{Table, Value};

use crate::color::Rgb;
use crate::error::ThemeError;
use crate::model::{Attrs, ColorRole, StyleRole, Theme, ThemeKind};

/// A style as written in a file; `None` means "not given".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleFile {
    /// `foreground = "#rrggbb"`.
    pub foreground: Option<Rgb>,
    /// `background = "#rrggbb"`, or `Some(None)` for `background = "none"`
    /// (no band).
    pub background: Option<Option<Rgb>>,
    /// `attributes = ["bold", "underline"]`.
    pub attributes: Option<Attrs>,
    /// Unknown keys.
    pub extra: Table,
}

impl StyleFile {
    /// True when nothing is given.
    pub fn is_empty(&self) -> bool {
        self.foreground.is_none()
            && self.background.is_none()
            && self.attributes.is_none()
            && self.extra.is_empty()
    }
}

/// One `[[user_highlights]]` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct UserHighlightFile {
    /// `name = "yellow"`.
    pub name: String,
    /// The rest of the entry.
    pub style: StyleFile,
}

/// A theme file as written: every value optional, unknown keys kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeFile {
    /// `theme.name`.
    pub name: Option<String>,
    /// `theme.display_name`.
    pub display_name: Option<String>,
    /// `theme.kind`.
    pub kind: Option<ThemeKind>,
    /// `theme.description`.
    pub description: Option<String>,
    /// `theme.author`.
    pub author: Option<String>,
    /// `theme.origin`.
    pub origin: Option<String>,
    /// `theme.counterpart`.
    pub counterpart: Option<String>,
    /// `theme.inherits`.
    pub inherits: Option<String>,
    /// Unknown keys in `[theme]`.
    pub meta_extra: Table,
    /// `[colors]`, indexed by [`ColorRole::index`].
    pub colors: [Option<Rgb>; ColorRole::COUNT],
    /// Unknown keys in `[colors]`.
    pub colors_extra: Table,
    /// `[styles.*]`, indexed by [`StyleRole::index`].
    pub styles: [StyleFile; StyleRole::COUNT],
    /// Unknown keys in `[styles]`.
    pub styles_extra: Table,
    /// `[[user_highlights]]`; `None` when absent.
    pub user_highlights: Option<Vec<UserHighlightFile>>,
    /// Unknown top-level keys.
    pub extra: Table,
}

/// Largest theme file accepted, in bytes.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;

fn line_of(src: &str, byte: usize) -> usize {
    src.get(..byte.min(src.len()))
        .map_or(1, |s| s.bytes().filter(|&b| b == b'\n').count() + 1)
}

fn string(v: Value, key: &str) -> Result<String, ThemeError> {
    match v {
        Value::String(s) => Ok(s),
        other => Err(ThemeError::invalid(
            key,
            format!("expected text in quotes, found {}", other.type_str()),
        )),
    }
}

fn color(v: Value, key: &str) -> Result<Rgb, ThemeError> {
    let s = string(v, key)?;
    Rgb::parse(&s).map_err(|e| ThemeError::invalid(key, e.to_string()))
}

/// Checks a theme name: 1 to 64 lowercase letters, digits, hyphens, or
/// underscores. Uppercase is folded to lowercase.
pub fn normalize_name(name: &str) -> Result<String, String> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() || n.len() > 64 {
        return Err("a theme name needs 1 to 64 characters".into());
    }
    if !n
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!(
            "{name:?} is not a usable theme name; use letters, digits, and hyphens"
        ));
    }
    Ok(n)
}

fn style(v: Value, key: &str) -> Result<StyleFile, ThemeError> {
    let Value::Table(t) = v else {
        return Err(ThemeError::invalid(
            key,
            "expected a table with foreground, background, and attributes",
        ));
    };
    let mut s = StyleFile::default();
    for (k, v) in t {
        let path = format!("{key}.{k}");
        match k.as_str() {
            "foreground" => s.foreground = Some(color(v, &path)?),
            "background" => {
                let text = string(v, &path)?;
                s.background = Some(if text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(Rgb::parse(&text).map_err(|e| {
                        ThemeError::invalid(&path, format!("{e}, or \"none\" for no band"))
                    })?)
                });
            }
            "attributes" => {
                let items = match v {
                    Value::Array(a) => a,
                    Value::String(one) => vec![Value::String(one)],
                    other => {
                        return Err(ThemeError::invalid(
                            &path,
                            format!(
                                "expected a list such as [\"bold\"], found {}",
                                other.type_str()
                            ),
                        ));
                    }
                };
                let mut a = Attrs::NONE;
                for item in items {
                    let name = string(item, &path)?;
                    a = a.with(Attrs::from_name(&name).ok_or_else(|| {
                        ThemeError::invalid(
                            &path,
                            format!(
                                "{name:?} is not an attribute; use bold, italic, underline, or reverse"
                            ),
                        )
                    })?);
                }
                s.attributes = Some(a);
            }
            _ => {
                s.extra.insert(k, v);
            }
        }
    }
    Ok(s)
}

impl ThemeFile {
    /// Reads a theme file. Unknown keys are kept; values of the wrong kind
    /// are errors naming the key.
    pub fn parse(src: &str) -> Result<Self, ThemeError> {
        let table: Table = toml::from_str(src).map_err(|e| ThemeError::Syntax {
            line: e.span().map(|s| line_of(src, s.start)),
            message: e.message().trim().to_owned(),
        })?;
        let mut f = ThemeFile::default();
        for (key, value) in table {
            match key.as_str() {
                "theme" => {
                    let Value::Table(t) = value else {
                        return Err(ThemeError::invalid("theme", "expected a [theme] table"));
                    };
                    for (k, v) in t {
                        let path = format!("theme.{k}");
                        match k.as_str() {
                            "name" => {
                                let s = string(v, &path)?;
                                f.name = Some(
                                    normalize_name(&s)
                                        .map_err(|m| ThemeError::invalid(&path, m))?,
                                );
                            }
                            "display_name" => f.display_name = Some(string(v, &path)?),
                            "kind" => {
                                let s = string(v, &path)?;
                                f.kind = Some(ThemeKind::from_key(&s).ok_or_else(|| {
                                    ThemeError::invalid(
                                        &path,
                                        format!(
                                            "{s:?} is not a kind; use light, dark, or high-contrast"
                                        ),
                                    )
                                })?);
                            }
                            "description" => f.description = Some(string(v, &path)?),
                            "author" => f.author = Some(string(v, &path)?),
                            "origin" => f.origin = Some(string(v, &path)?),
                            "counterpart" => f.counterpart = Some(string(v, &path)?),
                            "inherits" => f.inherits = Some(string(v, &path)?),
                            _ => {
                                f.meta_extra.insert(k, v);
                            }
                        }
                    }
                }
                "colors" => {
                    let Value::Table(t) = value else {
                        return Err(ThemeError::invalid("colors", "expected a [colors] table"));
                    };
                    for (k, v) in t {
                        match ColorRole::from_key(&k) {
                            Some(role) => {
                                f.colors[role.index()] = Some(color(v, &format!("colors.{k}"))?)
                            }
                            None => {
                                f.colors_extra.insert(k, v);
                            }
                        }
                    }
                }
                "styles" => {
                    let Value::Table(t) = value else {
                        return Err(ThemeError::invalid("styles", "expected a [styles] table"));
                    };
                    for (k, v) in t {
                        match StyleRole::from_key(&k) {
                            Some(role) => {
                                f.styles[role.index()] = style(v, &format!("styles.{k}"))?
                            }
                            None => {
                                f.styles_extra.insert(k, v);
                            }
                        }
                    }
                }
                "user_highlights" => {
                    let Value::Array(items) = value else {
                        return Err(ThemeError::invalid(
                            "user_highlights",
                            "expected [[user_highlights]] entries",
                        ));
                    };
                    let mut list = Vec::with_capacity(items.len());
                    for (i, item) in items.into_iter().enumerate() {
                        let path = format!("user_highlights entry {}", i + 1);
                        let Value::Table(mut t) = item else {
                            return Err(ThemeError::invalid(&path, "expected a table"));
                        };
                        let name = match t.remove("name") {
                            Some(v) => string(v, &format!("{path}.name"))?,
                            None => return Err(ThemeError::Missing(format!("{path}.name"))),
                        };
                        let name = name.trim().to_owned();
                        if name.is_empty() {
                            return Err(ThemeError::invalid(
                                format!("{path}.name"),
                                "a highlight color needs a name",
                            ));
                        }
                        let style = style(Value::Table(t), &path)?;
                        list.push(UserHighlightFile { name, style });
                    }
                    f.user_highlights = Some(list);
                }
                _ => {
                    f.extra.insert(key, value);
                }
            }
        }
        Ok(f)
    }

    /// Writes the file as TOML in a fixed, readable order: `[theme]`,
    /// `[colors]`, `[styles.*]`, `[[user_highlights]]`, then unknown tables.
    pub fn to_toml_string(&self) -> String {
        let mut out = String::new();
        // Top-level plain values must come before the first table header.
        let (top_tables, top_values): (Vec<_>, Vec<_>) = self
            .extra
            .iter()
            .partition(|(_, v)| matches!(v, Value::Table(_)) || is_array_of_tables(v));
        for (k, v) in &top_values {
            line(&mut out, k, v);
        }
        if !top_values.is_empty() {
            out.push('\n');
        }

        out.push_str("[theme]\n");
        for (k, v) in [("name", &self.name), ("display_name", &self.display_name)] {
            if let Some(v) = v {
                line(&mut out, k, &Value::String(v.clone()));
            }
        }
        if let Some(kind) = self.kind {
            line(&mut out, "kind", &Value::String(kind.key().into()));
        }
        for (k, v) in [
            ("description", &self.description),
            ("author", &self.author),
            ("origin", &self.origin),
            ("counterpart", &self.counterpart),
            ("inherits", &self.inherits),
        ] {
            if let Some(v) = v {
                line(&mut out, k, &Value::String(v.clone()));
            }
        }
        for (k, v) in &self.meta_extra {
            line(&mut out, k, v);
        }

        out.push_str("\n[colors]\n");
        for &role in ColorRole::ALL {
            if let Some(c) = self.colors[role.index()] {
                line(&mut out, role.key(), &Value::String(c.hex()));
            }
        }
        for (k, v) in &self.colors_extra {
            line(&mut out, k, v);
        }

        if !self.styles_extra.is_empty() {
            out.push_str("\n[styles]\n");
            for (k, v) in &self.styles_extra {
                line(&mut out, k, v);
            }
        }
        for &role in StyleRole::ALL {
            let s = &self.styles[role.index()];
            if s.is_empty() {
                continue;
            }
            out.push_str(&format!("\n[styles.{}]\n", role.key()));
            style_lines(&mut out, s);
        }

        if let Some(list) = &self.user_highlights {
            for h in list {
                out.push_str("\n[[user_highlights]]\n");
                line(&mut out, "name", &Value::String(h.name.clone()));
                style_lines(&mut out, &h.style);
            }
        }

        if !top_tables.is_empty() {
            let mut rest = Table::new();
            for (k, v) in top_tables {
                rest.insert(k.clone(), v.clone());
            }
            if let Ok(s) = toml::to_string(&rest) {
                out.push('\n');
                out.push_str(&s);
            }
        }
        out
    }
}

fn is_array_of_tables(v: &Value) -> bool {
    matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(|x| matches!(x, Value::Table(_))))
}

fn style_lines(out: &mut String, s: &StyleFile) {
    if let Some(c) = s.foreground {
        line(out, "foreground", &Value::String(c.hex()));
    }
    match s.background {
        Some(Some(c)) => line(out, "background", &Value::String(c.hex())),
        Some(None) => line(out, "background", &Value::String("none".into())),
        None => {}
    }
    if let Some(a) = s.attributes {
        let names = a
            .names()
            .into_iter()
            .map(|n| Value::String(n.into()))
            .collect();
        line(out, "attributes", &Value::Array(names));
    }
    for (k, v) in &s.extra {
        line(out, k, v);
    }
}

fn key(k: &str) -> String {
    if !k.is_empty()
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        k.to_owned()
    } else {
        Value::String(k.to_owned()).to_string()
    }
}

fn line(out: &mut String, k: &str, v: &Value) {
    out.push_str(&key(k));
    out.push_str(" = ");
    out.push_str(&v.to_string());
    out.push('\n');
}

impl From<&Theme> for ThemeFile {
    /// A fully explicit file for a resolved theme: every key written except
    /// the colors derived at load.
    fn from(t: &Theme) -> Self {
        let to_style = |s: &crate::model::Style| StyleFile {
            foreground: Some(s.foreground),
            background: Some(s.background),
            attributes: Some(s.attributes),
            extra: s.extra.clone(),
        };
        let mut colors = [None; ColorRole::COUNT];
        // Derived roles are left out, so a copied file derives them again
        // from its own colors instead of freezing these.
        for &r in ColorRole::ALL {
            if !t.is_derived(r) {
                colors[r.index()] = Some(t.color(r));
            }
        }
        ThemeFile {
            name: Some(t.meta.name.clone()),
            display_name: Some(t.meta.display_name.clone()),
            kind: Some(t.meta.kind),
            description: Some(t.meta.description.clone()),
            author: t.meta.author.clone(),
            origin: Some(t.meta.origin.clone()),
            counterpart: t.meta.counterpart.clone(),
            inherits: t.meta.inherits.clone(),
            meta_extra: t.meta.extra.clone(),
            colors,
            colors_extra: t.colors_extra.clone(),
            styles: std::array::from_fn(|i| to_style(&t.styles[i])),
            styles_extra: t.styles_extra.clone(),
            user_highlights: Some(
                t.user_highlights
                    .iter()
                    .map(|h| UserHighlightFile {
                        name: h.name.clone(),
                        style: to_style(&h.style),
                    })
                    .collect(),
            ),
            extra: t.extra.clone(),
        }
    }
}

impl Theme {
    /// The theme as a fully explicit TOML file, unknown keys included.
    pub fn to_toml_string(&self) -> String {
        ThemeFile::from(self).to_toml_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
future_top = 1

[theme]
name = "Ocean"
kind = "dark"
mood = "calm"

[colors]
background = "#001122"
text = "#EEEEEE"
sparkle = "#ff00ff"

[styles.find_hit]
background = "none"
attributes = ["underline", "bold"]
glow = true

[styles.future_role]
background = "#123456"

[[user_highlights]]
name = "sea"
background = "#004466"

[plugin.settings]
level = 3
"##;

    #[test]
    fn parse_keeps_unknown_keys() {
        let f = ThemeFile::parse(SAMPLE).unwrap();
        assert_eq!(f.name.as_deref(), Some("ocean"));
        assert_eq!(f.kind, Some(ThemeKind::Dark));
        assert_eq!(f.meta_extra["mood"].as_str(), Some("calm"));
        assert_eq!(
            f.colors[ColorRole::Text.index()],
            Some(Rgb::from_u32(0xeeeeee))
        );
        assert!(f.colors_extra.contains_key("sparkle"));
        let fh = &f.styles[StyleRole::FindHit.index()];
        assert_eq!(fh.background, Some(None));
        assert_eq!(fh.attributes, Some(Attrs::BOLD.with(Attrs::UNDERLINE)));
        assert_eq!(fh.extra["glow"].as_bool(), Some(true));
        assert!(f.styles_extra.contains_key("future_role"));
        assert_eq!(f.user_highlights.as_ref().map(Vec::len), Some(1));
        assert!(f.extra.contains_key("plugin"));
        assert!(f.extra.contains_key("future_top"));
    }

    #[test]
    fn write_then_read_is_identity() {
        let f = ThemeFile::parse(SAMPLE).unwrap();
        let text = f.to_toml_string();
        let again = ThemeFile::parse(&text).unwrap();
        assert_eq!(f, again, "{text}");
    }

    #[test]
    fn errors_name_the_key() {
        let e = ThemeFile::parse("[colors]\ntext = \"teal\"\n").unwrap_err();
        assert!(e.to_string().starts_with("colors.text: "), "{e}");
        let e = ThemeFile::parse("[styles.bookmark]\nattributes = [\"blink\"]\n").unwrap_err();
        assert!(e.to_string().contains("styles.bookmark.attributes"), "{e}");
        let e = ThemeFile::parse("[theme]\nkind = \"purple\"\n").unwrap_err();
        assert!(
            e.to_string().contains("light, dark, or high-contrast"),
            "{e}"
        );
        let e = ThemeFile::parse("[theme]\nname = \"a b\"\n").unwrap_err();
        assert!(e.to_string().contains("theme.name"), "{e}");
        let e = ThemeFile::parse("[colors]\ntext = \n").unwrap_err();
        assert!(
            matches!(e, ThemeError::Syntax { line: Some(2), .. }),
            "{e:?}"
        );
        assert!(e.to_string().contains("line 2"), "{e}");
        let e = ThemeFile::parse("[[user_highlights]]\nbackground = \"#ffff00\"\n").unwrap_err();
        assert!(matches!(e, ThemeError::Missing(_)), "{e:?}");
    }

    #[test]
    fn quoted_keys_survive() {
        let f = ThemeFile::parse("[colors]\n\"odd key\" = 1\n").unwrap();
        let again = ThemeFile::parse(&f.to_toml_string()).unwrap();
        assert_eq!(f, again);
    }
}
