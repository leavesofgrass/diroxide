//! The symbols of a file, for the outline and next and previous symbol:
//! from the grammar's names for code (Rust, Python, the shell), from
//! `pulldown-cmark`'s offsets for Markdown headings, and from small
//! direct scans for the structure of JSON, TOML, YAML, HTML and CSS, which
//! need no grammar and so work over the caps and without the syntax set.
//! A row says its kind first, so a long name cannot push the kind past
//! cell 40: "function main, line 40", "heading 2, Installation, line 10".

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

use super::language::Language;

/// What a symbol is.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolKind {
    /// A function or method.
    Function,
    /// A type: a struct, an enum, a class, a trait.
    Type,
    /// An impl block.
    Impl,
    /// A module.
    Module,
    /// A heading, with its level, 1 to 6.
    Heading(u8),
    /// A top-level key of JSON, TOML or YAML.
    Key,
    /// A TOML table.
    Table,
    /// A CSS rule's selector.
    Selector,
}

impl SymbolKind {
    /// The kind in words: "function", "heading 2".
    pub fn word(&self) -> String {
        match self {
            SymbolKind::Function => "function".to_owned(),
            SymbolKind::Type => "type".to_owned(),
            SymbolKind::Impl => "impl".to_owned(),
            SymbolKind::Module => "module".to_owned(),
            SymbolKind::Heading(level) => format!("heading {level}"),
            SymbolKind::Key => "key".to_owned(),
            SymbolKind::Table => "table".to_owned(),
            SymbolKind::Selector => "selector".to_owned(),
        }
    }
}

/// A symbol: its kind, its name and its line, 0-based.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol {
    /// What it is.
    pub kind: SymbolKind,
    /// Its name as written.
    pub name: String,
    /// Its line, 0-based.
    pub line: usize,
}

impl Symbol {
    /// A symbol.
    pub fn new(kind: SymbolKind, name: impl Into<String>, line: usize) -> Self {
        Self {
            kind,
            name: name.into(),
            line,
        }
    }

    /// The outline's row, kind first, the line 1-based: "function main,
    /// line 40", "heading 2, Installation, line 10".
    pub fn row(&self) -> String {
        let line = diroxide_core::format::grouped(self.line as u64 + 1);
        match self.kind {
            SymbolKind::Heading(_) => format!("{}, {}, line {line}", self.kind.word(), self.name),
            _ => format!("{} {}, line {line}", self.kind.word(), self.name),
        }
    }
}

/// The symbols that need no grammar: Markdown headings, the structure of
/// JSON, TOML, YAML, HTML and CSS. `None` for the code languages, whose
/// symbols come from the parse.
pub fn structure_symbols(language: Language, lines: &[String]) -> Option<Vec<Symbol>> {
    Some(match language {
        Language::Markdown => markdown_headings(&lines.join("\n")),
        Language::Json => json_keys(lines),
        Language::Toml => toml_tables(lines),
        Language::Yaml => yaml_keys(lines),
        Language::Html => html_headings(lines),
        Language::Css => css_selectors(lines),
        _ => return None,
    })
}

/// Markdown headings with their levels, from `pulldown-cmark`'s offsets,
/// so a `#` inside a fenced block is not one. The quick view's heading
/// moves use the same index.
pub fn markdown_headings(text: &str) -> Vec<Symbol> {
    let mut found = Vec::new();
    let mut open: Option<(u8, usize, String)> = None;
    for (event, range) in Parser::new_ext(text, Options::empty()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let line = text[..range.start].matches('\n').count();
                open = Some((heading_level(level), line, String::new()));
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, _, name)) = open.as_mut() {
                    name.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, line, name)) = open.take() {
                    found.push(Symbol::new(SymbolKind::Heading(level), name.trim(), line));
                }
            }
            _ => {}
        }
    }
    found
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// The keys of JSON's top-level object, by a scan that knows strings and
/// nesting.
fn json_keys(lines: &[String]) -> Vec<Symbol> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut expecting_key = false;
    for (n, line) in lines.iter().enumerate() {
        let mut chars = line.char_indices().peekable();
        while let Some((_, c)) = chars.next() {
            match c {
                '{' | '[' => {
                    depth += 1;
                    expecting_key = c == '{' && depth == 1;
                }
                '}' | ']' => depth = depth.saturating_sub(1),
                ',' if depth == 1 => expecting_key = true,
                '"' => {
                    let mut text = String::new();
                    let mut escaped = false;
                    for (_, c) in chars.by_ref() {
                        if escaped {
                            text.push(c);
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '"' {
                            break;
                        } else {
                            text.push(c);
                        }
                    }
                    if depth == 1 && expecting_key {
                        found.push(Symbol::new(SymbolKind::Key, text, n));
                        expecting_key = false;
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// TOML's top-level keys before the first table, then its tables.
fn toml_tables(lines: &[String]) -> Vec<Symbol> {
    let mut found = Vec::new();
    let mut in_table = false;
    for (n, line) in lines.iter().enumerate() {
        let text = line.trim();
        if text.starts_with('#') || text.is_empty() {
            continue;
        }
        if let Some(inner) = text.strip_prefix("[[").and_then(|t| t.split_once("]]")) {
            found.push(Symbol::new(SymbolKind::Table, inner.0.trim(), n));
            in_table = true;
        } else if let Some(inner) = text.strip_prefix('[').and_then(|t| t.split_once(']')) {
            found.push(Symbol::new(SymbolKind::Table, inner.0.trim(), n));
            in_table = true;
        } else if !in_table && let Some((key, _)) = text.split_once('=') {
            found.push(Symbol::new(SymbolKind::Key, key.trim(), n));
        }
    }
    found
}

/// YAML's top-level keys: lines with no indent that name a key.
fn yaml_keys(lines: &[String]) -> Vec<Symbol> {
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if line.starts_with([' ', '\t', '#', '-']) || line.starts_with("---") {
            continue;
        }
        if let Some((key, _)) = line.split_once(':') {
            let key = key.trim().trim_matches(['"', '\'']);
            if !key.is_empty() {
                found.push(Symbol::new(SymbolKind::Key, key, n));
            }
        }
    }
    found
}

/// HTML's headings, `<h1>` to `<h6>`, with their text on the line.
fn html_headings(lines: &[String]) -> Vec<Symbol> {
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();
        let mut at = 0;
        while let Some(i) = lower[at..].find("<h") {
            let start = at + i;
            let level = lower.as_bytes().get(start + 2).copied();
            let after = lower.as_bytes().get(start + 3).copied();
            at = start + 2;
            let (Some(level @ b'1'..=b'6'), Some(b'>' | b' ' | b'\t')) = (level, after) else {
                continue;
            };
            let Some(open_end) = lower[start..].find('>').map(|e| start + e + 1) else {
                continue;
            };
            let close = lower[open_end..]
                .find("</h")
                .map_or(line.len(), |e| open_end + e);
            let name = strip_tags(&line[open_end..close]);
            found.push(Symbol::new(
                SymbolKind::Heading(level - b'0'),
                name.trim(),
                n,
            ));
            at = close;
        }
    }
    found
}

fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// CSS rules' selectors: the text before each `{` at the top level,
/// comments left out.
fn css_selectors(lines: &[String]) -> Vec<Symbol> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut in_comment = false;
    let mut pending = String::new();
    let mut pending_line = 0usize;
    for (n, line) in lines.iter().enumerate() {
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if in_comment {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_comment = false;
                }
                continue;
            }
            match c {
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    in_comment = true;
                }
                '{' => {
                    if depth == 0 && !pending.trim().is_empty() {
                        found.push(Symbol::new(
                            SymbolKind::Selector,
                            pending.trim(),
                            pending_line,
                        ));
                    }
                    pending.clear();
                    depth += 1;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    pending.clear();
                }
                ';' if depth == 0 => pending.clear(),
                c if depth == 0 => {
                    if pending.trim().is_empty() && !c.is_whitespace() {
                        pending_line = n;
                    }
                    pending.push(c);
                }
                _ => {}
            }
        }
        if depth == 0 && !pending.is_empty() {
            pending.push(' ');
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample_lines;
    use super::*;

    fn rows(symbols: &[Symbol]) -> Vec<String> {
        symbols.iter().map(Symbol::row).collect()
    }

    #[test]
    fn markdown_headings_come_from_the_parser_not_the_hash_signs() {
        let lines = sample_lines("report.md");
        let found = structure_symbols(Language::Markdown, &lines).unwrap();
        assert_eq!(
            rows(&found),
            [
                "heading 1, The report, line 1",
                "heading 2, Installation, line 5",
                "heading 2, Usage, line 14",
                "heading 3, Options, line 18",
                "heading 2, Known limits, line 22",
            ]
        );
    }

    #[test]
    fn structured_files_list_their_keys_tables_headings_and_selectors() {
        let json = structure_symbols(Language::Json, &sample_lines("data.json")).unwrap();
        assert_eq!(
            rows(&json),
            [
                "key name, line 2",
                "key version, line 3",
                "key tags, line 4",
                "key owner, line 5",
                "key limits, line 9",
            ]
        );
        let toml = structure_symbols(Language::Toml, &sample_lines("config.toml")).unwrap();
        assert_eq!(
            rows(&toml),
            [
                "key title, line 2",
                "key count, line 3",
                "table viewer, line 5",
                "table presets, line 9",
            ]
        );
        let yaml = structure_symbols(Language::Yaml, &sample_lines("ci.yaml")).unwrap();
        assert_eq!(
            rows(&yaml),
            ["key name, line 2", "key on, line 3", "key jobs, line 6"]
        );
        let html = structure_symbols(Language::Html, &sample_lines("page.html")).unwrap();
        assert_eq!(
            rows(&html),
            [
                "heading 1, Sample page, line 8",
                "heading 2, Details, line 10"
            ]
        );
        let css = structure_symbols(Language::Css, &sample_lines("site.css")).unwrap();
        assert_eq!(
            rows(&css),
            [
                "selector body, line 2",
                "selector .intro, line 7",
                "selector a:hover, line 11"
            ]
        );
        assert!(structure_symbols(Language::Rust, &[]).is_none());
    }
}
