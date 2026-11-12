//! The rules as data: what each rule does, the counter, the passes, the
//! recipe and the presets file. Nothing here touches a name; the
//! compiler in [`super::compile`] turns a recipe into functions.
//!
//! A recipe is one or more passes. Each pass is a list of rules applied in
//! order to the name the pass was given, with its own counter; the next
//! pass starts from the names the last one made (Total Commander's "next
//! step"). Placeholders that name the old name (`{old-name}`) always mean
//! the name on disk.

use serde::{Deserialize, Serialize};

/// Which part of a name a rule edits. The stem is the default, so an
/// extension is never changed by accident.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Part {
    /// The name without its extension: `report` of `report.pdf`. For a
    /// folder, the whole name.
    #[default]
    Stem,
    /// The extension without its dot: `pdf` of `report.pdf`. A folder has
    /// none, so the rule leaves it alone.
    Extension,
    /// The whole name, extension and all.
    Name,
}

impl Part {
    /// The part in words, for a rule's summary: "the name without its
    /// extension".
    pub fn words(self) -> &'static str {
        match self {
            Part::Stem => "the name",
            Part::Extension => "the extension",
            Part::Name => "the whole name",
        }
    }
}

/// How the text to find is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindMode {
    /// The text itself, every character literal (the default).
    #[default]
    Text,
    /// A regular expression in the regex crate's syntax, with `$1`,
    /// `${1}` and `${name}` for its groups in the replacement.
    Pattern,
}

/// A case form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaseForm {
    /// every letter small.
    Lower,
    /// EVERY LETTER CAPITAL.
    Upper,
    /// The First Letter Of Each Word Capital, The Rest Small. Words split
    /// at spaces, `_`, `-` and `.`; an apostrophe does not split.
    Title,
    /// The first letter capital, the rest small.
    Sentence,
}

impl CaseForm {
    /// The form in words: "lowercase".
    pub fn words(self) -> &'static str {
        match self {
            CaseForm::Lower => "lowercase",
            CaseForm::Upper => "uppercase",
            CaseForm::Title => "title case",
            CaseForm::Sentence => "sentence case",
        }
    }
}

/// Which end a position counts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum End {
    /// From the start of the part.
    #[default]
    Start,
    /// From the end of the part.
    End,
}

/// A place between characters, counted in characters as a reader
/// perceives them (a letter with its accents is one), from either end.
/// Offset 0 from the start is before the first character; offset 0 from
/// the end is after the last. An offset past the other end stops there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Position {
    /// Which end the offset counts from.
    pub from: End,
    /// How many characters in from that end.
    pub offset: usize,
}

impl Position {
    /// Offset characters in from the start.
    pub const fn from_start(offset: usize) -> Self {
        Self {
            from: End::Start,
            offset,
        }
    }

    /// Offset characters in from the end.
    pub const fn from_end(offset: usize) -> Self {
        Self {
            from: End::End,
            offset,
        }
    }

    /// The position in words: "at the start", "after character 3", "at
    /// the end", "before the last 4 characters".
    pub fn words(self) -> String {
        match (self.from, self.offset) {
            (End::Start, 0) => "at the start".to_owned(),
            (End::End, 0) => "at the end".to_owned(),
            (End::Start, n) => format!("after character {n}"),
            (End::End, 1) => "before the last character".to_owned(),
            (End::End, n) => format!("before the last {n} characters"),
        }
    }
}

/// A class of characters a rule can remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CharacterClass {
    /// Digits of any script.
    Digits,
    /// Spaces, tabs and other white space.
    Spaces,
    /// Punctuation and symbols.
    Punctuation,
    /// Everything outside ASCII.
    NonAscii,
}

impl CharacterClass {
    /// The class in words: "digits".
    pub fn words(self) -> &'static str {
        match self {
            CharacterClass::Digits => "digits",
            CharacterClass::Spaces => "spaces",
            CharacterClass::Punctuation => "punctuation",
            CharacterClass::NonAscii => "characters outside ASCII",
        }
    }
}

/// A tidying rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "tidy", rename_all = "kebab-case")]
pub enum Tidy {
    /// White space removed from both ends.
    Trim,
    /// Runs of white space made one space.
    CollapseSpaces,
    /// Every space replaced by this text (an underscore or a hyphen,
    /// usually).
    SpacesTo {
        /// The text each space becomes.
        with: String,
    },
    /// This text (an underscore, say) replaced by a space.
    ToSpaces {
        /// The text that becomes a space.
        from: String,
    },
    /// Every character of a class removed.
    Remove {
        /// The class.
        class: CharacterClass,
    },
}

/// One rule. Each edits one [`Part`] of the working name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Rule {
    /// Find and replace: plain text by default, a regular expression in
    /// [`FindMode::Pattern`].
    Replace {
        /// The text or pattern to find.
        find: String,
        /// What it becomes; in pattern mode `$1`, `${1}` and `${name}`
        /// name groups and `$$` is a dollar sign. In text mode every
        /// character is literal.
        #[serde(default)]
        with: String,
        /// Text or pattern.
        #[serde(default)]
        mode: FindMode,
        /// Every match, not only the first.
        #[serde(default)]
        all: bool,
        /// Case ignored when finding.
        #[serde(default)]
        ignore_case: bool,
        /// The part edited.
        #[serde(default)]
        part: Part,
    },
    /// Text put in at a position.
    Insert {
        /// The text, which may hold placeholders such as `{counter}`.
        text: String,
        /// Where it goes.
        #[serde(default)]
        at: Position,
        /// The part edited.
        #[serde(default)]
        part: Part,
    },
    /// Characters taken out from a position.
    Remove {
        /// Where the removal starts, counted inward from its end: from
        /// the start, the characters after it; from the end, the ones
        /// before it.
        #[serde(default)]
        at: Position,
        /// How many characters; 0 means all the rest in that direction.
        #[serde(default)]
        count: usize,
        /// The part edited.
        #[serde(default)]
        part: Part,
    },
    /// White space and characters tidied.
    Tidy {
        /// What is tidied.
        #[serde(flatten)]
        tidy: Tidy,
        /// The part edited.
        #[serde(default)]
        part: Part,
    },
    /// A case form.
    Case {
        /// The form.
        form: CaseForm,
        /// The part edited.
        #[serde(default)]
        part: Part,
    },
    /// The part made anew from a template with placeholders in braces:
    /// `{stem}-{counter}`, `{modified:YYYY-MM-DD} {old-stem}`. The list
    /// is [`super::PLACEHOLDERS`]; `{{` and `}}` are braces themselves.
    Template {
        /// The template.
        text: String,
        /// The part made.
        #[serde(default)]
        part: Part,
    },
}

impl Rule {
    /// A plain-text replacement of the first match in the stem.
    pub fn replace(find: impl Into<String>, with: impl Into<String>) -> Self {
        Rule::Replace {
            find: find.into(),
            with: with.into(),
            mode: FindMode::Text,
            all: false,
            ignore_case: false,
            part: Part::Stem,
        }
    }

    /// A pattern replacement of the first match in the stem.
    pub fn pattern(find: impl Into<String>, with: impl Into<String>) -> Self {
        Rule::Replace {
            find: find.into(),
            with: with.into(),
            mode: FindMode::Pattern,
            all: false,
            ignore_case: false,
            part: Part::Stem,
        }
    }

    /// A template for the stem.
    pub fn template(text: impl Into<String>) -> Self {
        Rule::Template {
            text: text.into(),
            part: Part::Stem,
        }
    }

    /// A case form for the stem.
    pub fn case(form: CaseForm) -> Self {
        Rule::Case {
            form,
            part: Part::Stem,
        }
    }

    /// The same rule, editing another part.
    #[must_use]
    pub fn on(mut self, new_part: Part) -> Self {
        match &mut self {
            Rule::Replace { part, .. }
            | Rule::Insert { part, .. }
            | Rule::Remove { part, .. }
            | Rule::Tidy { part, .. }
            | Rule::Case { part, .. }
            | Rule::Template { part, .. } => *part = new_part,
        }
        self
    }

    /// The same replacement, of every match. Other rules are unchanged.
    #[must_use]
    pub fn every_match(mut self) -> Self {
        if let Rule::Replace { all, .. } = &mut self {
            *all = true;
        }
        self
    }

    /// The same replacement, ignoring case. Other rules are unchanged.
    #[must_use]
    pub fn ignoring_case(mut self) -> Self {
        if let Rule::Replace { ignore_case, .. } = &mut self {
            *ignore_case = true;
        }
        self
    }

    /// The part the rule edits.
    pub fn part(&self) -> Part {
        match self {
            Rule::Replace { part, .. }
            | Rule::Insert { part, .. }
            | Rule::Remove { part, .. }
            | Rule::Tidy { part, .. }
            | Rule::Case { part, .. }
            | Rule::Template { part, .. } => *part,
        }
    }

    /// The rule as one sentence, the action first, for the rules list:
    /// "Replace IMG_ with trip-, in the name"; "Change case to lowercase,
    /// in the extension".
    pub fn describe(&self) -> String {
        let part = self.part().words();
        match self {
            Rule::Replace {
                find,
                with,
                mode,
                all,
                ignore_case,
                ..
            } => {
                let verb = match (mode, all) {
                    (FindMode::Text, false) => "Replace",
                    (FindMode::Text, true) => "Replace every",
                    (FindMode::Pattern, false) => "Replace pattern",
                    (FindMode::Pattern, true) => "Replace every pattern",
                };
                let with = if with.is_empty() {
                    "nothing".to_owned()
                } else {
                    with.clone()
                };
                let case = if *ignore_case { ", any case" } else { "" };
                format!("{verb} {find} with {with}, in {part}{case}")
            }
            Rule::Insert { text, at, .. } => {
                format!("Insert {text} {}, in {part}", at.words())
            }
            Rule::Remove { at, count, .. } => {
                let what = match count {
                    0 => "all characters".to_owned(),
                    1 => "1 character".to_owned(),
                    n => format!("{n} characters"),
                };
                let from = match (at.from, at.offset) {
                    (End::Start, 0) => "from the start".to_owned(),
                    (End::End, 0) => "from the end".to_owned(),
                    _ => at.words(),
                };
                format!("Remove {what} {from}, in {part}")
            }
            Rule::Tidy { tidy, .. } => match tidy {
                Tidy::Trim => format!("Trim spaces at both ends of {part}"),
                Tidy::CollapseSpaces => format!("Collapse runs of spaces in {part}"),
                Tidy::SpacesTo { with } => format!("Turn spaces into {with}, in {part}"),
                Tidy::ToSpaces { from } => format!("Turn {from} into spaces, in {part}"),
                Tidy::Remove { class } => format!("Remove {}, in {part}", class.words()),
            },
            Rule::Case { form, .. } => format!("Change case to {}, in {part}", form.words()),
            Rule::Template { text, .. } => format!("Make {part} from {text}"),
        }
    }
}

/// The counter a pass's `{counter}` placeholder reads: start, step and
/// width, counted in the order the entries were given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Counter {
    /// The first number.
    pub start: i64,
    /// What each next entry adds; may be negative.
    pub step: i64,
    /// The fewest digits, zero padded; wider numbers simply grow.
    pub width: u8,
    /// Whether the counter starts again in each folder (for flat views
    /// and collections, whose entries span folders).
    pub per_folder: bool,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            start: 1,
            step: 1,
            width: 1,
            per_folder: false,
        }
    }
}

/// The widest a counter may be padded.
pub const MAX_COUNTER_WIDTH: u8 = 20;

/// One pass: rules in order, and the counter they read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Pass {
    /// The rules, applied in order.
    #[serde(rename = "rule")]
    pub rules: Vec<Rule>,
    /// The counter.
    pub counter: Counter,
}

impl Pass {
    /// A pass of these rules with the default counter.
    pub fn of(rules: Vec<Rule>) -> Self {
        Self {
            rules,
            counter: Counter::default(),
        }
    }
}

/// A recipe: passes run one after another, each over the last one's
/// names.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Recipe {
    /// The passes, in order.
    #[serde(rename = "pass")]
    pub passes: Vec<Pass>,
    /// Whether `tar.gz`, `tar.xz`, `tar.bz2` and `tar.zst` count as one
    /// extension (on by default).
    pub compound_extensions: bool,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            passes: Vec::new(),
            compound_extensions: true,
        }
    }
}

impl Recipe {
    /// A recipe of one pass of these rules.
    pub fn of(rules: Vec<Rule>) -> Self {
        Self {
            passes: vec![Pass::of(rules)],
            ..Self::default()
        }
    }

    /// The same recipe with the first pass's counter set.
    #[must_use]
    pub fn counting(mut self, counter: Counter) -> Self {
        if self.passes.is_empty() {
            self.passes.push(Pass::default());
        }
        self.passes[0].counter = counter;
        self
    }

    /// The same recipe with another pass after the others.
    #[must_use]
    pub fn then(mut self, pass: Pass) -> Self {
        self.passes.push(pass);
        self
    }

    /// How many rules, over every pass.
    pub fn rule_count(&self) -> usize {
        self.passes.iter().map(|pass| pass.rules.len()).sum()
    }
}

/// A recipe saved by name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Preset {
    /// The name the owner gave it.
    pub name: String,
    /// The recipe.
    #[serde(flatten)]
    pub recipe: Recipe,
}

/// The presets file, `rename-presets.toml` in the config folder: the
/// same shape for the dialog, `diro rename --preset` and a recipe file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Presets {
    /// The presets, in the order kept.
    #[serde(rename = "preset")]
    pub presets: Vec<Preset>,
}

/// Why a presets file or a recipe file could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PresetError {
    /// The text is not TOML in the presets' shape. The detail is the TOML
    /// reader's own words.
    #[error("Rename presets not read: {0}")]
    NotRead(String),
    /// The recipe could not be written as TOML.
    #[error("Rename presets not written: {0}")]
    NotWritten(String),
}

impl Presets {
    /// Reads a presets file's text.
    pub fn from_toml(text: &str) -> Result<Self, PresetError> {
        toml::from_str(text).map_err(|error| PresetError::NotRead(first_line(&error.to_string())))
    }

    /// The presets as TOML text.
    pub fn to_toml(&self) -> Result<String, PresetError> {
        toml::to_string(self).map_err(|error| PresetError::NotWritten(error.to_string()))
    }

    /// The preset of this name, case ignored.
    pub fn get(&self, name: &str) -> Option<&Preset> {
        let wanted = name.to_lowercase();
        self.presets
            .iter()
            .find(|preset| preset.name.to_lowercase() == wanted)
    }

    /// Keeps a recipe under a name, replacing a preset of the same name
    /// (case ignored) in its place, or adding it at the end.
    pub fn save(&mut self, name: &str, recipe: Recipe) {
        let wanted = name.to_lowercase();
        let preset = Preset {
            name: name.to_owned(),
            recipe,
        };
        match self
            .presets
            .iter_mut()
            .find(|preset| preset.name.to_lowercase() == wanted)
        {
            Some(slot) => *slot = preset,
            None => self.presets.push(preset),
        }
    }
}

impl Recipe {
    /// Reads a recipe file's text: one recipe, the shape of a preset
    /// without its name.
    pub fn from_toml(text: &str) -> Result<Self, PresetError> {
        toml::from_str(text).map_err(|error| PresetError::NotRead(first_line(&error.to_string())))
    }

    /// The recipe as TOML text.
    pub fn to_toml(&self) -> Result<String, PresetError> {
        toml::to_string(self).map_err(|error| PresetError::NotWritten(error.to_string()))
    }
}

/// The first line of the TOML reader's message, which puts the place and
/// the cause there; the lines after it point at the text.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("the text could not be read")
        .to_owned()
}
