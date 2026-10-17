//! Whether structured text holds together, said before it is read: JSON
//! and TOML through the parsers the workspace already has. "JSON invalid
//! at line 12, column 5", "JSON, 3.1 KB, valid". YAML has no parser here
//! and says nothing.

use diroxide_core::format::{grouped, human_size};

use super::language::Language;

/// Whether a file parses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Validity {
    /// It parses.
    Valid,
    /// It does not: where the parser stopped, 1-based.
    Invalid {
        /// The line.
        line: usize,
        /// The column, in characters.
        column: usize,
    },
}

/// Whether JSON or TOML text parses; `None` for the other languages.
pub fn validity(language: Language, text: &str) -> Option<Validity> {
    match language {
        Language::Json => Some(match serde_json::from_str::<serde::de::IgnoredAny>(text) {
            Ok(_) => Validity::Valid,
            Err(error) => Validity::Invalid {
                line: error.line().max(1),
                column: error.column().max(1),
            },
        }),
        Language::Toml => Some(match toml::from_str::<toml::Table>(text) {
            Ok(_) => Validity::Valid,
            Err(error) => {
                let at = error.span().map_or(0, |span| span.start).min(text.len());
                let (line, column) = line_and_column(text, at);
                Validity::Invalid { line, column }
            }
        }),
        _ => None,
    }
}

/// The 1-based line and column of a byte, the column in characters.
fn line_and_column(text: &str, at: usize) -> (usize, usize) {
    let mut at = at;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let before = &text[..at];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

impl Validity {
    /// The line said when the file opens, meaning first: "JSON invalid at
    /// line 12, column 5", "JSON, 3.1 KB, valid".
    pub fn line(&self, language: Language, bytes: u64) -> String {
        let name = language.name();
        match self {
            Validity::Valid => format!("{name}, {}, valid", human_size(bytes)),
            Validity::Invalid { line, column } => format!(
                "{name} invalid at line {}, column {}",
                grouped(*line as u64),
                grouped(*column as u64)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample_lines;
    use super::*;

    #[test]
    fn json_and_toml_say_whether_they_parse() {
        let good = sample_lines("data.json").join("\n");
        let valid = validity(Language::Json, &good).unwrap();
        assert_eq!(valid, Validity::Valid);
        assert_eq!(valid.line(Language::Json, 3_100), "JSON, 3.1 KB, valid");
        let bad = sample_lines("bad.json").join("\n");
        let invalid = validity(Language::Json, &bad).unwrap();
        assert_eq!(invalid, Validity::Invalid { line: 4, column: 1 });
        let said = invalid.line(Language::Json, 40);
        assert_eq!(said, "JSON invalid at line 4, column 1");
        assert!(said.chars().count() <= 40);
        let toml = sample_lines("config.toml").join("\n");
        assert_eq!(validity(Language::Toml, &toml), Some(Validity::Valid));
        let broken = "title = \"x\"\ncount = = 3\n";
        assert_eq!(
            validity(Language::Toml, broken),
            Some(Validity::Invalid { line: 2, column: 9 })
        );
        assert_eq!(validity(Language::Yaml, "a: b"), None);
    }
}
