//! Token classes: what a run of text is, as a word, never as a color
//! (research/syntax-highlighting.md, R17). The highlighter (W2d's
//! `highlight` module) maps syntect's scopes to these classes; each
//! frontend's theme turns a class into a style with an attribute, and the
//! words-first commands say the class ("Function name: parse_args, line
//! 40"). The viewer and the quick view keep the same runs, so saying a
//! token's kind is a lookup, not a second parse.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// The kind of a run of text. More may join (a plugin's classes, a
/// language's own), so callers match with a fallback arm.
#[non_exhaustive]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum TokenClass {
    /// Text with no class: the default, and what a gap between runs is.
    #[default]
    Plain,
    /// A keyword of the language (`fn`, `if`, `return`).
    Keyword,
    /// A string or character literal.
    String,
    /// A comment.
    Comment,
    /// A number.
    Number,
    /// A named constant or a literal such as `true`.
    Constant,
    /// The name of a function or method where it is defined or called.
    FunctionName,
    /// The name of a type, class, struct, enum, trait or interface.
    TypeName,
    /// A variable or parameter.
    Variable,
    /// An operator.
    Operator,
    /// Punctuation: brackets, commas, semicolons.
    Punctuation,
    /// A heading in markup (Markdown, reStructuredText, Org).
    Heading,
    /// A link or its address.
    Link,
    /// Emphasized or strong text in markup.
    Emphasis,
    /// Code inside markup: a code span or a fenced block's text.
    Code,
    /// A tag in HTML or XML.
    Tag,
    /// An attribute of a tag, or a key in structured text.
    Attribute,
    /// Something the grammar marks as invalid.
    Problem,
}

impl TokenClass {
    /// Every class, in the order of R17's table.
    pub const ALL: [TokenClass; 18] = [
        TokenClass::Plain,
        TokenClass::Keyword,
        TokenClass::String,
        TokenClass::Comment,
        TokenClass::Number,
        TokenClass::Constant,
        TokenClass::FunctionName,
        TokenClass::TypeName,
        TokenClass::Variable,
        TokenClass::Operator,
        TokenClass::Punctuation,
        TokenClass::Heading,
        TokenClass::Link,
        TokenClass::Emphasis,
        TokenClass::Code,
        TokenClass::Tag,
        TokenClass::Attribute,
        TokenClass::Problem,
    ];

    /// The class as a word, lowercase, for "Line 40: keyword, function
    /// name, string": "function name".
    pub const fn word(self) -> &'static str {
        match self {
            TokenClass::Plain => "plain",
            TokenClass::Keyword => "keyword",
            TokenClass::String => "string",
            TokenClass::Comment => "comment",
            TokenClass::Number => "number",
            TokenClass::Constant => "constant",
            TokenClass::FunctionName => "function name",
            TokenClass::TypeName => "type name",
            TokenClass::Variable => "variable",
            TokenClass::Operator => "operator",
            TokenClass::Punctuation => "punctuation",
            TokenClass::Heading => "heading",
            TokenClass::Link => "link",
            TokenClass::Emphasis => "emphasis",
            TokenClass::Code => "code",
            TokenClass::Tag => "tag",
            TokenClass::Attribute => "attribute",
            TokenClass::Problem => "problem",
        }
    }

    /// The class as a label at the start of a line, capitalized: "Function
    /// name: parse_args, line 40".
    pub fn label(self) -> String {
        let word = self.word();
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }
}

/// A run of one class in one line: byte offsets into the line, `start`
/// inclusive and `end` exclusive, on character boundaries. The runs of a
/// line are in order and never overlap; a gap between them is plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClassRun {
    /// The first byte of the run.
    pub start: u32,
    /// The byte after the run.
    pub end: u32,
    /// What the run is.
    pub class: TokenClass,
}

impl ClassRun {
    /// A run from `start` to `end` of a class.
    pub const fn new(start: u32, end: u32, class: TokenClass) -> Self {
        Self { start, end, class }
    }

    /// The run's bytes as a range of the line.
    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The run's text in its line, or `None` when the offsets do not fall
    /// on the line's character boundaries.
    pub fn text<'a>(&self, line: &'a str) -> Option<&'a str> {
        line.get(self.range())
    }
}

/// The class at a byte of a line, from its runs: plain where no run
/// covers it.
pub fn class_at(runs: &[ClassRun], byte: usize) -> TokenClass {
    runs.iter()
        .find(|run| run.range().contains(&byte))
        .map_or(TokenClass::Plain, |run| run.class)
}

/// Whether a line's runs are well formed: in order, not overlapping, not
/// empty, and on the line's character boundaries.
pub fn runs_fit(line: &str, runs: &[ClassRun]) -> bool {
    let mut last_end = 0u32;
    runs.iter().all(|run| {
        let fits = run.start >= last_end && run.start < run.end && run.text(line).is_some();
        last_end = run.end;
        fits
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_class_is_a_word_and_reads_back() {
        let mut words = HashSet::new();
        for class in TokenClass::ALL {
            let word = class.word();
            assert!(words.insert(word), "{word} twice");
            assert_eq!(word, word.to_lowercase());
            assert!(class.label().starts_with(|c: char| c.is_ascii_uppercase()));
            let json = serde_json::to_string(&class).unwrap();
            assert_eq!(serde_json::from_str::<TokenClass>(&json).unwrap(), class);
        }
        assert_eq!(TokenClass::FunctionName.label(), "Function name");
        assert_eq!(
            serde_json::to_string(&TokenClass::TypeName).unwrap(),
            "\"type-name\""
        );
        assert_eq!(TokenClass::default(), TokenClass::Plain);
    }

    #[test]
    fn runs_say_the_class_at_a_byte_and_check_their_line() {
        let line = "fn parse_args() {";
        let runs = [
            ClassRun::new(0, 2, TokenClass::Keyword),
            ClassRun::new(3, 13, TokenClass::FunctionName),
            ClassRun::new(13, 15, TokenClass::Punctuation),
        ];
        assert!(runs_fit(line, &runs));
        assert_eq!(runs[1].text(line), Some("parse_args"));
        assert_eq!(class_at(&runs, 4), TokenClass::FunctionName);
        assert_eq!(class_at(&runs, 2), TokenClass::Plain, "a gap is plain");
        assert_eq!(class_at(&runs, 99), TokenClass::Plain);
        let overlapping = [
            ClassRun::new(0, 4, TokenClass::Keyword),
            ClassRun::new(3, 6, TokenClass::String),
        ];
        assert!(!runs_fit(line, &overlapping));
        let accent = "caf\u{e9}";
        assert!(!runs_fit(
            accent,
            &[ClassRun::new(0, 4, TokenClass::String)]
        ));
        assert!(runs_fit(accent, &[ClassRun::new(0, 5, TokenClass::String)]));
        assert!(!runs_fit(line, &[ClassRun::new(2, 2, TokenClass::Code)]));
    }
}
