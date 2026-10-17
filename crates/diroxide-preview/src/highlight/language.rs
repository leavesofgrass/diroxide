//! The languages highlighted by default: the nine of R17's allow-list,
//! found by the file's name and its first line, on the bytes already
//! read, never by opening the file again (so a member of an archive or a
//! remote file is found the same way).

use serde::{Deserialize, Serialize};

/// A language on the allow-list. Others join only after they pass the
/// hang test (R17); until then they show plain.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    /// Markdown.
    Markdown,
    /// JSON.
    Json,
    /// TOML.
    Toml,
    /// YAML.
    Yaml,
    /// The shell: sh, bash and zsh.
    Shell,
    /// Rust.
    Rust,
    /// Python.
    Python,
    /// HTML.
    Html,
    /// CSS.
    Css,
}

impl Language {
    /// Every language on the allow-list.
    pub const ALL: [Language; 9] = [
        Language::Markdown,
        Language::Json,
        Language::Toml,
        Language::Yaml,
        Language::Shell,
        Language::Rust,
        Language::Python,
        Language::Html,
        Language::Css,
    ];

    /// The language's name as said: "Rust", "JSON".
    pub const fn name(self) -> &'static str {
        match self {
            Language::Markdown => "Markdown",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Yaml => "YAML",
            Language::Shell => "Shell",
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::Html => "HTML",
            Language::Css => "CSS",
        }
    }

    /// What a file in the language is, for the title and the details line:
    /// "Rust source", "Markdown text", "JSON data".
    pub const fn kind_words(self) -> &'static str {
        match self {
            Language::Markdown => "Markdown text",
            Language::Json => "JSON data",
            Language::Toml => "TOML data",
            Language::Yaml => "YAML data",
            Language::Shell => "Shell script",
            Language::Rust => "Rust source",
            Language::Python => "Python source",
            Language::Html => "HTML page",
            Language::Css => "CSS style sheet",
        }
    }

    /// The name of the language's grammar in the syntax set.
    pub const fn syntax_name(self) -> &'static str {
        match self {
            Language::Markdown => "Markdown",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Yaml => "YAML",
            Language::Shell => "Bourne Again Shell (bash)",
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::Html => "HTML",
            Language::Css => "CSS",
        }
    }

    /// The language of a file by its name, then its first line (a `#!`
    /// line, a doctype); `None` for anything off the allow-list.
    pub fn detect(file_name: &str, first_line: &str) -> Option<Self> {
        Self::by_name(file_name).or_else(|| Self::by_first_line(first_line))
    }

    /// The language a file name says.
    pub fn by_name(file_name: &str) -> Option<Self> {
        let lower = file_name.to_ascii_lowercase();
        match lower.as_str() {
            "cargo.lock" | "pipfile" | "poetry.lock" => return Some(Language::Toml),
            ".bashrc" | ".bash_profile" | ".bash_aliases" | ".profile" | ".zshrc" | ".zprofile"
            | ".zshenv" | "pkgbuild" => return Some(Language::Shell),
            _ => {}
        }
        let (_, extension) = lower.rsplit_once('.')?;
        Some(match extension {
            "md" | "markdown" | "mdown" | "mkd" | "mkdn" => Language::Markdown,
            "json" | "jsonc" | "geojson" | "webmanifest" => Language::Json,
            "toml" => Language::Toml,
            "yaml" | "yml" => Language::Yaml,
            "sh" | "bash" | "zsh" | "ksh" => Language::Shell,
            "rs" => Language::Rust,
            "py" | "pyw" | "pyi" => Language::Python,
            "html" | "htm" | "xhtml" => Language::Html,
            "css" => Language::Css,
            _ => return None,
        })
    }

    /// The language a first line says: `#!/bin/sh`, `#!/usr/bin/env
    /// python3`, `<!DOCTYPE html>`.
    pub fn by_first_line(line: &str) -> Option<Self> {
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(rest) = line.strip_prefix("#!") {
            let mut words = rest.split_whitespace();
            let program = words.next()?;
            let program = match program.rsplit('/').next() {
                Some("env") => words.find(|w| !w.starts_with('-'))?,
                Some(name) => name,
                None => program,
            };
            let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
            return match program {
                "sh" | "bash" | "zsh" | "ksh" | "dash" | "ash" => Some(Language::Shell),
                "python" | "pypy" => Some(Language::Python),
                _ => None,
            };
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
            return Some(Language::Html);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_found_by_name_then_first_line() {
        let table = [
            ("main.rs", "", Some(Language::Rust)),
            ("README.md", "", Some(Language::Markdown)),
            ("package.JSON", "", Some(Language::Json)),
            ("Cargo.toml", "", Some(Language::Toml)),
            ("Cargo.lock", "", Some(Language::Toml)),
            ("ci.yml", "", Some(Language::Yaml)),
            ("build.sh", "", Some(Language::Shell)),
            (".bashrc", "", Some(Language::Shell)),
            ("tool.py", "", Some(Language::Python)),
            ("index.htm", "", Some(Language::Html)),
            ("site.css", "", Some(Language::Css)),
            ("deploy", "#!/bin/bash -e", Some(Language::Shell)),
            ("run", "#!/usr/bin/env python3", Some(Language::Python)),
            (
                "run",
                "#!/usr/bin/env -S python3.12 -u",
                Some(Language::Python),
            ),
            ("page", "<!DOCTYPE html>", Some(Language::Html)),
            ("notes.txt", "plain text", None),
            ("script.ps1", "", None),
            ("app.ts", "", None),
            ("noextension", "", None),
        ];
        for (name, first, expected) in table {
            assert_eq!(Language::detect(name, first), expected, "{name} {first}");
        }
        for language in Language::ALL {
            assert!(!language.name().is_empty());
            assert!(language.kind_words().starts_with(language.name()));
        }
    }
}
