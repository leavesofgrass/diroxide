//! A recipe compiled: every pattern built once, every replacement and
//! template checked, and each rule turned into a function over a name.
//! A recipe that cannot compile is refused as a whole, in words, with the
//! number of the rule; a value missing for one entry is that row's
//! problem instead.

use std::fmt;

use diroxide_core::format::human_size;
use diroxide_core::names::{COMPOUND_EXTENSIONS, split_extension, split_extension_with};
use diroxide_core::pattern::{PatternError, compile_regex};
use diroxide_core::{NameCase, Timestamp, UtcOffset};

use super::rule::{FindMode, MAX_COUNTER_WIDTH, Part, Recipe, Rule, Tidy};
use super::text;
use super::{FieldLookup, RenameSource};

/// The placeholders a template may hold, each with what it gives, in the
/// order the chooser lists them. Dates take a format after a colon:
/// `{modified:YYYY-MM-DD}` (the default format), with `YYYY`, `YY`, `MM`,
/// `DD`, `HH`, `mm` and `ss`; no default holds a colon, which Windows
/// refuses in a name.
pub const PLACEHOLDERS: [(&str, &str); 14] = [
    ("{stem}", "the name without its extension, so far"),
    ("{ext}", "the extension without its dot, so far"),
    ("{name}", "the whole name, so far"),
    ("{old-stem}", "the name without its extension on disk"),
    ("{old-ext}", "the extension on disk"),
    ("{old-name}", "the whole name on disk"),
    ("{parent}", "the name of the folder it is in"),
    ("{counter}", "the counter: start, step and width"),
    ("{modified}", "the date it was changed"),
    ("{created}", "the date it was made"),
    ("{accessed}", "the date it was last read"),
    ("{size}", "the size in bytes"),
    ("{size:human}", "the size in words, such as 2.1 MB"),
    ("{meta:FIELD}", "a details field, as in {meta:exif.date}"),
];

/// The default date format: no colon, so Windows takes it.
const DEFAULT_DATE_FORMAT: &str = "YYYY-MM-DD";

/// What is wrong with one rule, so the recipe cannot run.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuleError {
    /// The text to find is empty.
    #[error("nothing to find")]
    EmptyFind,
    /// The pattern could not be read.
    #[error("{}", .0.line())]
    Pattern(PatternError),
    /// The replacement names a group the pattern does not have.
    #[error("no group {reference}; the pattern has {groups}")]
    MissingGroup {
        /// The group as written: `2`, or `year`.
        reference: String,
        /// How many numbered groups the pattern has, the whole match not
        /// counted.
        groups: usize,
    },
    /// The replacement says `$1_`, which the regex crate reads as a
    /// group named `1_`.
    #[error("${reference} is a group named {reference}")]
    AmbiguousGroup {
        /// The name as the crate would read it: `1_`.
        reference: String,
        /// The digits that start it: `1`.
        digits: String,
    },
    /// A template's brace is not closed, or a `}` stands alone.
    #[error("a brace is not closed")]
    UnclosedBrace,
    /// A template names a placeholder diroxide does not have.
    #[error("no placeholder {0}")]
    UnknownPlaceholder(String),
    /// A date format holds none of the date's letters.
    #[error("date format {0} has no YYYY, MM or DD")]
    BadDateFormat(String),
    /// The counter is padded wider than [`MAX_COUNTER_WIDTH`].
    #[error("counter wider than 20 digits")]
    CounterTooWide,
}

impl RuleError {
    /// What to do about it, in a sentence.
    pub fn next_step(&self) -> String {
        match self {
            RuleError::EmptyFind => "Type the text to find.".to_owned(),
            RuleError::Pattern(error) => diroxide_core::error::Explain::next_step(error),
            RuleError::MissingGroup { groups, .. } => match groups {
                0 => "Put a group in the pattern, such as (\\d+).".to_owned(),
                1 => "Use $1, the pattern's one group.".to_owned(),
                n => format!("Use a group from $1 to ${n}."),
            },
            RuleError::AmbiguousGroup { digits, reference } => {
                let rest = &reference[digits.len()..];
                format!("Write ${{{digits}}}{rest} for group {digits} followed by {rest}.")
            }
            RuleError::UnclosedBrace => {
                "Close the brace, or write {{ or }} for a brace itself.".to_owned()
            }
            RuleError::UnknownPlaceholder(_) => {
                "Choose one from the list of placeholders.".to_owned()
            }
            RuleError::BadDateFormat(_) => {
                "Use capital YYYY, MM and DD, and HH, mm, ss for the time.".to_owned()
            }
            RuleError::CounterTooWide => "Use a width of 20 digits or fewer.".to_owned(),
        }
    }
}

/// Why a recipe cannot run: which rule, and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeError {
    /// The pass, from 1; only said when the recipe has more than one.
    pub pass: usize,
    /// The rule in its pass, from 1; 0 for the pass's counter.
    pub rule: usize,
    /// Whether the recipe has several passes, so the line names the pass.
    pub several_passes: bool,
    /// What is wrong.
    pub error: RuleError,
}

impl RecipeError {
    /// The message, the rule first and the cause after it: "Rule 2:
    /// pattern needs care: look-around"; "Pass 2, rule 1: nothing to
    /// find"; "Counter: counter wider than 20 digits".
    pub fn line(&self) -> String {
        let place = match (self.rule, self.several_passes) {
            (0, false) => "Counter".to_owned(),
            (0, true) => format!("Pass {}, counter", self.pass),
            (rule, false) => format!("Rule {rule}"),
            (rule, true) => format!("Pass {}, rule {rule}", self.pass),
        };
        let cause = self.error.to_string();
        let mut chars = cause.chars();
        let cause = match chars.next() {
            Some(first) => first.to_lowercase().chain(chars).collect(),
            None => cause,
        };
        format!("{place}: {cause}")
    }

    /// What to do about it, in a sentence.
    pub fn next_step(&self) -> String {
        self.error.next_step()
    }
}

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line())
    }
}

impl std::error::Error for RecipeError {}

/// A date a template reads from the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DateField {
    Modified,
    Created,
    Accessed,
}

/// One placeholder, read.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Placeholder {
    Stem,
    Ext,
    Name,
    OldStem,
    OldExt,
    OldName,
    Parent,
    Counter,
    Date(DateField, String),
    Size,
    SizeHuman,
    Meta(String),
}

/// A piece of a template.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    Value(Placeholder, String),
}

/// Reads a template into pieces, refusing unknown placeholders and
/// unclosed braces.
fn read_template(source: &str) -> Result<Vec<Piece>, RuleError> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '}' => return Err(RuleError::UnclosedBrace),
            '{' => {
                let mut inside = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    if c == '{' {
                        return Err(RuleError::UnclosedBrace);
                    }
                    inside.push(c);
                }
                if !closed {
                    return Err(RuleError::UnclosedBrace);
                }
                if !literal.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut literal)));
                }
                let written = format!("{{{inside}}}");
                pieces.push(Piece::Value(read_placeholder(&inside)?, written));
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    Ok(pieces)
}

/// One placeholder's words, inside its braces.
fn read_placeholder(inside: &str) -> Result<Placeholder, RuleError> {
    let unknown = || RuleError::UnknownPlaceholder(format!("{{{inside}}}"));
    let (word, argument) = match inside.split_once(':') {
        Some((word, argument)) => (word.trim(), Some(argument)),
        None => (inside.trim(), None),
    };
    let date = |field| {
        let format = argument.unwrap_or(DEFAULT_DATE_FORMAT);
        if text::date_format_is_valid(format) {
            Ok(Placeholder::Date(field, format.to_owned()))
        } else {
            Err(RuleError::BadDateFormat(format.to_owned()))
        }
    };
    match (word, argument) {
        ("stem", None) => Ok(Placeholder::Stem),
        ("ext", None) => Ok(Placeholder::Ext),
        ("name", None) => Ok(Placeholder::Name),
        ("old-stem", None) => Ok(Placeholder::OldStem),
        ("old-ext", None) => Ok(Placeholder::OldExt),
        ("old-name", None) => Ok(Placeholder::OldName),
        ("parent", None) => Ok(Placeholder::Parent),
        ("counter", None) => Ok(Placeholder::Counter),
        ("modified", _) => date(DateField::Modified),
        ("created", _) => date(DateField::Created),
        ("accessed", _) => date(DateField::Accessed),
        ("size", None) => Ok(Placeholder::Size),
        ("size", Some("human")) => Ok(Placeholder::SizeHuman),
        ("meta", Some(key)) if !key.trim().is_empty() => {
            Ok(Placeholder::Meta(key.trim().to_owned()))
        }
        _ => Err(unknown()),
    }
}

/// One reference in a replacement: a group number or name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Reference {
    Number(usize),
    Name(String),
}

/// The group references in a replacement, read by the regex crate's own
/// rules: `$$` is a dollar sign, `${...}` is exact, and `$` takes the
/// longest run of letters, digits and underscores after it. A run that
/// starts with digits and goes on with other characters is the `$1_`
/// trap, refused with advice.
fn references(replacement: &str) -> Result<Vec<Reference>, RuleError> {
    let bytes = replacement.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'$') => i += 2,
            Some(b'{') => match replacement[i + 2..].find('}') {
                Some(end) => {
                    let name = &replacement[i + 2..i + 2 + end];
                    found.push(reference(name));
                    i += end + 3;
                }
                // The crate writes an unclosed `${` as it is.
                None => i += 2,
            },
            _ => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if end > start {
                    let name = &replacement[start..end];
                    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
                    if !digits.is_empty() && digits.len() < name.len() {
                        return Err(RuleError::AmbiguousGroup {
                            reference: name.to_owned(),
                            digits,
                        });
                    }
                    found.push(reference(name));
                }
                i = end.max(i + 1);
            }
        }
    }
    Ok(found)
}

fn reference(name: &str) -> Reference {
    match name.parse::<usize>() {
        Ok(n) if name.bytes().all(|b| b.is_ascii_digit()) => Reference::Number(n),
        _ => Reference::Name(name.to_owned()),
    }
}

/// A replacement, ready to run over a part.
type Replacer = Box<dyn Fn(&str) -> String + Send + Sync>;

/// A rule compiled.
enum Compiled {
    Replace(Replacer),
    Insert(Vec<Piece>, super::rule::Position),
    Remove(super::rule::Position, usize),
    Tidy(Tidy),
    Case(super::rule::CaseForm),
    Template(Vec<Piece>),
}

/// A pass compiled.
pub(crate) struct CompiledPass {
    rules: Vec<(Compiled, Part)>,
    pub(crate) counter: super::rule::Counter,
}

/// A recipe compiled.
pub(crate) struct CompiledRecipe {
    pub(crate) passes: Vec<CompiledPass>,
    compound: bool,
}

/// Compiles a replace rule.
fn compile_replace(
    find: &str,
    with: &str,
    mode: FindMode,
    all: bool,
    ignore_case: bool,
) -> Result<Replacer, RuleError> {
    if find.is_empty() {
        return Err(RuleError::EmptyFind);
    }
    let case = if ignore_case {
        NameCase::Ignore
    } else {
        NameCase::Respect
    };
    let (source, replacement) = match mode {
        FindMode::Text => (text::escape_pattern(find), text::literal_replacement(with)),
        FindMode::Pattern => (find.to_owned(), with.to_owned()),
    };
    let regex = compile_regex(&source, case).map_err(RuleError::Pattern)?;
    if mode == FindMode::Pattern {
        let groups = regex.captures_len().saturating_sub(1);
        for found in references(&replacement)? {
            let known = match &found {
                Reference::Number(n) => *n <= groups,
                Reference::Name(name) => regex.capture_names().flatten().any(|g| g == name),
            };
            if !known {
                let reference = match found {
                    Reference::Number(n) => n.to_string(),
                    Reference::Name(name) => name,
                };
                return Err(RuleError::MissingGroup { reference, groups });
            }
        }
    }
    let limit = if all { 0 } else { 1 };
    Ok(Box::new(move |part: &str| {
        regex
            .replacen(part, limit, replacement.as_str())
            .into_owned()
    }))
}

impl CompiledRecipe {
    /// Compiles every pass of a recipe; the first fault refuses it.
    pub(crate) fn new(recipe: &Recipe) -> Result<Self, RecipeError> {
        let several_passes = recipe.passes.len() > 1;
        let mut passes = Vec::with_capacity(recipe.passes.len());
        for (p, pass) in recipe.passes.iter().enumerate() {
            let fault = |rule, error| RecipeError {
                pass: p + 1,
                rule,
                several_passes,
                error,
            };
            if pass.counter.width > MAX_COUNTER_WIDTH {
                return Err(fault(0, RuleError::CounterTooWide));
            }
            let mut rules = Vec::with_capacity(pass.rules.len());
            for (r, rule) in pass.rules.iter().enumerate() {
                let compiled = match rule {
                    Rule::Replace {
                        find,
                        with,
                        mode,
                        all,
                        ignore_case,
                        ..
                    } => compile_replace(find, with, *mode, *all, *ignore_case)
                        .map(Compiled::Replace),
                    Rule::Insert { text, at, .. } => {
                        read_template(text).map(|pieces| Compiled::Insert(pieces, *at))
                    }
                    Rule::Remove { at, count, .. } => Ok(Compiled::Remove(*at, *count)),
                    Rule::Tidy { tidy, .. } => Ok(Compiled::Tidy(tidy.clone())),
                    Rule::Case { form, .. } => Ok(Compiled::Case(*form)),
                    Rule::Template { text, .. } => read_template(text).map(Compiled::Template),
                }
                .map_err(|error| fault(r + 1, error))?;
                rules.push((compiled, rule.part()));
            }
            passes.push(CompiledPass {
                rules,
                counter: pass.counter,
            });
        }
        Ok(Self {
            passes,
            compound: recipe.compound_extensions,
        })
    }

    /// A name's stem and extension (with its dot) by the shared rule; a
    /// folder's whole name is its stem.
    pub(crate) fn split<'a>(&self, name: &'a str, is_folder: bool) -> (&'a str, &'a str) {
        if is_folder {
            (name, "")
        } else if self.compound {
            split_extension_with(name, &COMPOUND_EXTENSIONS)
        } else {
            split_extension(name)
        }
    }
}

/// Why one entry's new name could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ValueProblem {
    /// A placeholder had no value for this entry.
    Missing(String),
    /// The counter went below zero.
    CounterBelowZero,
}

/// What a row's rules read beside its name.
pub(crate) struct Subject<'a> {
    pub source: &'a RenameSource,
    pub parent: &'a str,
    pub counter: i64,
    pub offset: UtcOffset,
    pub lookup: Option<&'a dyn FieldLookup>,
}

impl CompiledPass {
    /// Runs the pass's rules over a name.
    pub(crate) fn apply(
        &self,
        recipe: &CompiledRecipe,
        name: &str,
        subject: &Subject<'_>,
    ) -> Result<String, ValueProblem> {
        let is_folder = subject.source.is_folder();
        let mut name = name.to_owned();
        for (rule, part) in &self.rules {
            let (stem, ext) = recipe.split(&name, is_folder);
            let ext_bare = ext.strip_prefix('.').unwrap_or(ext);
            let target = match part {
                Part::Stem => stem,
                Part::Extension if is_folder => continue,
                Part::Extension => ext_bare,
                Part::Name => name.as_str(),
            };
            let made = match rule {
                Compiled::Replace(replace) => replace(target),
                Compiled::Insert(pieces, at) => {
                    let text = fill(pieces, &name, stem, ext_bare, recipe, self, subject)?;
                    let at = text::offset_of(target, *at);
                    format!("{}{text}{}", &target[..at], &target[at..])
                }
                Compiled::Remove(at, count) => text::remove_at(target, *at, *count),
                Compiled::Tidy(tidy) => tidy_text(target, tidy),
                Compiled::Case(form) => text::change_case(target, *form),
                Compiled::Template(pieces) => {
                    fill(pieces, &name, stem, ext_bare, recipe, self, subject)?
                }
            };
            name = match part {
                Part::Stem => format!("{made}{ext}"),
                Part::Extension if made.is_empty() => stem.to_owned(),
                Part::Extension => format!("{stem}.{made}"),
                Part::Name => made,
            };
        }
        Ok(name)
    }
}

/// A tidy rule over a part.
fn tidy_text(target: &str, tidy: &Tidy) -> String {
    match tidy {
        Tidy::Trim => target.trim().to_owned(),
        Tidy::CollapseSpaces => {
            let mut out = String::with_capacity(target.len());
            let mut last_space = false;
            for c in target.chars() {
                if c.is_whitespace() {
                    if !last_space {
                        out.push(' ');
                    }
                    last_space = true;
                } else {
                    out.push(c);
                    last_space = false;
                }
            }
            out
        }
        Tidy::SpacesTo { with } => target.replace(' ', with),
        Tidy::ToSpaces { from } if from.is_empty() => target.to_owned(),
        Tidy::ToSpaces { from } => target.replace(from.as_str(), " "),
        Tidy::Remove { class } => target
            .chars()
            .filter(|c| !text::in_class(*c, *class))
            .collect(),
    }
}

/// A template's pieces filled for one entry.
fn fill(
    pieces: &[Piece],
    name: &str,
    stem: &str,
    ext: &str,
    recipe: &CompiledRecipe,
    pass: &CompiledPass,
    subject: &Subject<'_>,
) -> Result<String, ValueProblem> {
    let entry = &subject.source.entry;
    let old_name = entry.name.as_str();
    let (old_stem, old_ext) = recipe.split(old_name, subject.source.is_folder());
    let old_ext = old_ext.strip_prefix('.').unwrap_or(old_ext);
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Value(placeholder, written) => {
                let missing = || ValueProblem::Missing(written.clone());
                let date = |time: Option<Timestamp>, format: &str| {
                    time.map(|time| text::format_date(&time.to_civil(subject.offset), format))
                        .ok_or_else(missing)
                };
                let value = match placeholder {
                    Placeholder::Stem => stem.to_owned(),
                    Placeholder::Ext => ext.to_owned(),
                    Placeholder::Name => name.to_owned(),
                    Placeholder::OldStem => old_stem.to_owned(),
                    Placeholder::OldExt => old_ext.to_owned(),
                    Placeholder::OldName => old_name.to_owned(),
                    Placeholder::Parent => subject.parent.to_owned(),
                    Placeholder::Counter => {
                        if subject.counter < 0 {
                            return Err(ValueProblem::CounterBelowZero);
                        }
                        format!(
                            "{:0width$}",
                            subject.counter,
                            width = usize::from(pass.counter.width)
                        )
                    }
                    Placeholder::Date(DateField::Modified, format) => date(entry.modified, format)?,
                    Placeholder::Date(DateField::Created, format) => date(entry.created, format)?,
                    Placeholder::Date(DateField::Accessed, format) => date(entry.accessed, format)?,
                    Placeholder::Size => entry.size.map(|s| s.to_string()).ok_or_else(missing)?,
                    Placeholder::SizeHuman => entry.size.map(human_size).ok_or_else(missing)?,
                    Placeholder::Meta(key) => subject
                        .lookup
                        .and_then(|lookup| lookup.field(subject.source, key))
                        .filter(|value| !value.is_empty())
                        .ok_or_else(missing)?,
                };
                out.push_str(&value);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_read_their_placeholders() {
        assert!(read_template("{stem}-{counter}").is_ok());
        assert_eq!(
            read_template("{{x}}").unwrap(),
            vec![Piece::Text("{x}".into())]
        );
        assert_eq!(read_template("{stem"), Err(RuleError::UnclosedBrace));
        assert_eq!(read_template("a}b"), Err(RuleError::UnclosedBrace));
        assert_eq!(
            read_template("{nam}"),
            Err(RuleError::UnknownPlaceholder("{nam}".into()))
        );
        assert_eq!(
            read_template("{modified:yyyy}"),
            Err(RuleError::BadDateFormat("yyyy".into()))
        );
        assert!(read_template("{meta:exif.date}").is_ok());
        assert!(read_template("{meta:}").is_err());
    }

    #[test]
    fn references_follow_the_crates_rules() {
        assert_eq!(
            references("$1-${2}x$$${year}").unwrap(),
            vec![
                Reference::Number(1),
                Reference::Number(2),
                Reference::Name("year".into())
            ]
        );
        assert_eq!(
            references("$1_$2"),
            Err(RuleError::AmbiguousGroup {
                reference: "1_".into(),
                digits: "1".into()
            })
        );
        assert_eq!(references("cost $").unwrap(), vec![]);
        assert_eq!(references("${open").unwrap(), vec![]);
    }

    #[test]
    fn replacements_name_groups_that_exist() {
        let missing = compile_replace("(a)", "$2", FindMode::Pattern, false, false);
        assert_eq!(
            missing.err(),
            Some(RuleError::MissingGroup {
                reference: "2".into(),
                groups: 1
            })
        );
        let named = compile_replace("(?<y>\\d{4})", "${y}", FindMode::Pattern, false, false);
        assert!(named.is_ok());
        let wrong = compile_replace("(?<y>\\d{4})", "${m}", FindMode::Pattern, false, false);
        assert!(matches!(wrong, Err(RuleError::MissingGroup { .. })));
        let text = compile_replace("a.b", "$1", FindMode::Text, true, false).unwrap();
        assert_eq!(text("a.b axb a.b"), "$1 axb $1");
        let swap = compile_replace(
            "(\\d{2})-(\\d{2})-(\\d{4})",
            "$3-$2-$1",
            FindMode::Pattern,
            false,
            false,
        )
        .unwrap();
        assert_eq!(swap("29-03-2020"), "2020-03-29");
    }

    #[test]
    fn errors_say_the_rule_first_inside_40_cells() {
        let error = RecipeError {
            pass: 1,
            rule: 2,
            several_passes: false,
            error: RuleError::Pattern(PatternError::LookAround),
        };
        assert_eq!(error.line(), "Rule 2: pattern needs care: look-around");
        assert!(error.line().chars().count() <= 40);
        let trap = RecipeError {
            pass: 2,
            rule: 1,
            several_passes: true,
            error: RuleError::AmbiguousGroup {
                reference: "1_".into(),
                digits: "1".into(),
            },
        };
        assert_eq!(trap.line(), "Pass 2, rule 1: $1_ is a group named 1_");
        assert!(trap.line().chars().count() <= 40);
        assert_eq!(trap.next_step(), "Write ${1}_ for group 1 followed by _.");
        let missing = RecipeError {
            pass: 1,
            rule: 3,
            several_passes: false,
            error: RuleError::MissingGroup {
                reference: "2".into(),
                groups: 1,
            },
        };
        assert_eq!(missing.line(), "Rule 3: no group 2; the pattern has 1");
        assert!(missing.line().chars().count() <= 40);
    }
}
