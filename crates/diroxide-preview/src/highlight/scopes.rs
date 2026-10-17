//! The scope mapper: Sublime Text's scope names (the naming guide at
//! sublimetext.com/docs/scope_naming.html) to the eighteen token classes,
//! by the longest rule that is a prefix of the scope, atom by atom.
//!
//! A token sits under a stack of scopes. The class comes from the stack,
//! innermost first, with a rank so the scope that matters wins: a problem
//! over everything, then a comment over what is inside it, then a key of
//! a mapping (a JSON key is also a string), then a string, then the
//! innermost scope that has a class at all.

use crate::classes::TokenClass;

/// A rule: a scope prefix, its class, and its rank.
type Rule = (&'static str, TokenClass, u8);

/// The rules, most specific first where two share a start.
const RULES: &[Rule] = &[
    ("invalid", TokenClass::Problem, 5),
    ("comment", TokenClass::Comment, 4),
    ("punctuation.definition.comment", TokenClass::Comment, 4),
    ("meta.mapping.key", TokenClass::Attribute, 3),
    ("entity.name.tag.yaml", TokenClass::Attribute, 3),
    ("entity.name.tag.toml", TokenClass::Attribute, 3),
    ("string.other.link", TokenClass::Link, 2),
    ("markup.underline.link", TokenClass::Link, 2),
    ("markup.heading", TokenClass::Heading, 2),
    ("string", TokenClass::String, 2),
    ("constant.numeric", TokenClass::Number, 1),
    ("constant.character.escape", TokenClass::Constant, 1),
    ("constant", TokenClass::Constant, 1),
    ("keyword.operator", TokenClass::Operator, 1),
    ("keyword", TokenClass::Keyword, 1),
    ("storage.type.primitive", TokenClass::TypeName, 1),
    ("storage.type.numeric", TokenClass::TypeName, 1),
    ("storage", TokenClass::Keyword, 1),
    ("entity.name.function", TokenClass::FunctionName, 1),
    ("entity.name.section", TokenClass::Heading, 1),
    ("entity.name.tag", TokenClass::Tag, 1),
    ("entity.name.type", TokenClass::TypeName, 1),
    ("entity.name.class", TokenClass::TypeName, 1),
    ("entity.name.struct", TokenClass::TypeName, 1),
    ("entity.name.enum", TokenClass::TypeName, 1),
    ("entity.name.union", TokenClass::TypeName, 1),
    ("entity.name.trait", TokenClass::TypeName, 1),
    ("entity.name.interface", TokenClass::TypeName, 1),
    ("entity.name.impl", TokenClass::TypeName, 1),
    ("entity.name.namespace", TokenClass::TypeName, 1),
    ("entity.name.module", TokenClass::TypeName, 1),
    ("entity.name.constant", TokenClass::Constant, 1),
    ("entity.name.table", TokenClass::Heading, 1),
    ("entity.name", TokenClass::Variable, 1),
    ("entity.other.attribute-name", TokenClass::Attribute, 1),
    ("entity.other.inherited-class", TokenClass::TypeName, 1),
    ("support.type.property-name", TokenClass::Attribute, 1),
    ("support.function", TokenClass::FunctionName, 1),
    ("support.type", TokenClass::TypeName, 1),
    ("support.class", TokenClass::TypeName, 1),
    ("support.constant", TokenClass::Constant, 1),
    ("variable.function", TokenClass::FunctionName, 1),
    ("variable", TokenClass::Variable, 1),
    ("markup.bold", TokenClass::Emphasis, 1),
    ("markup.italic", TokenClass::Emphasis, 1),
    ("markup.raw", TokenClass::Code, 1),
    ("punctuation", TokenClass::Punctuation, 1),
];

/// Whether `prefix` is a prefix of `scope` atom by atom: "string" is one
/// of "string.quoted.double.json", not of "strings".
fn atom_prefix(prefix: &str, scope: &str) -> bool {
    scope.starts_with(prefix)
        && (scope.len() == prefix.len() || scope.as_bytes().get(prefix.len()) == Some(&b'.'))
}

/// The class and rank of one scope, or `None` for a scope with no class
/// of its own (`source.rust`, `meta.function`).
pub fn rule_of(scope: &str) -> Option<(TokenClass, u8)> {
    RULES
        .iter()
        .filter(|(prefix, _, _)| atom_prefix(prefix, scope))
        .max_by_key(|(prefix, _, _)| prefix.len())
        .map(|(_, class, rank)| (*class, *rank))
}

/// The class of a token from the rules of its scopes, innermost first:
/// the highest rank wins, and among equal ranks the innermost.
pub fn class_of(rules: impl IntoIterator<Item = Option<(TokenClass, u8)>>) -> TokenClass {
    let mut best: Option<(TokenClass, u8)> = None;
    for (class, rank) in rules.into_iter().flatten() {
        if best.is_none_or(|(_, kept)| rank > kept) {
            best = Some((class, rank));
        }
    }
    best.map_or(TokenClass::Plain, |(class, _)| class)
}

/// The class of a token under scopes given by name, outermost first as a
/// scope stack lists them.
pub fn class_of_scopes(stack: &[&str]) -> TokenClass {
    class_of(stack.iter().rev().map(|scope| rule_of(scope)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_map_to_classes_by_their_longest_rule() {
        let table: [(&[&str], TokenClass); 14] = [
            (&["source.rust", "keyword.other.rust"], TokenClass::Keyword),
            (
                &[
                    "source.rust",
                    "meta.function.rust",
                    "entity.name.function.rust",
                ],
                TokenClass::FunctionName,
            ),
            (
                &["source.rust", "storage.type.function.rust"],
                TokenClass::Keyword,
            ),
            (
                &["source.rust", "keyword.operator.arithmetic.rust"],
                TokenClass::Operator,
            ),
            (
                &[
                    "source.rust",
                    "string.quoted.double.rust",
                    "constant.character.escape.rust",
                ],
                TokenClass::String,
            ),
            (
                &[
                    "source.rust",
                    "comment.line.double-slash.rust",
                    "punctuation.definition.comment.rust",
                ],
                TokenClass::Comment,
            ),
            (
                &[
                    "source.json",
                    "meta.mapping.key.json",
                    "string.quoted.double.json",
                ],
                TokenClass::Attribute,
            ),
            (
                &["source.json", "constant.numeric.integer.json"],
                TokenClass::Number,
            ),
            (
                &[
                    "text.html.markdown",
                    "markup.heading.2.markdown",
                    "punctuation.definition.heading.markdown",
                ],
                TokenClass::Heading,
            ),
            (
                &["source.css", "meta.selector.css", "entity.name.tag.css"],
                TokenClass::Tag,
            ),
            (
                &["source.python", "storage.type.primitive.python"],
                TokenClass::TypeName,
            ),
            (
                &["source.rust", "invalid.illegal.rust"],
                TokenClass::Problem,
            ),
            (&["source.rust", "meta.block.rust"], TokenClass::Plain),
            (
                &["source.shell", "punctuation.section.braces.begin.shell"],
                TokenClass::Punctuation,
            ),
        ];
        for (stack, class) in table {
            assert_eq!(class_of_scopes(stack), class, "{stack:?}");
        }
        assert!(atom_prefix("string", "string.quoted"));
        assert!(!atom_prefix("string", "strings.quoted"));
        assert_eq!(rule_of("source.rust"), None);
    }
}
