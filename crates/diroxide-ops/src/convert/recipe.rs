//! The recipe, one shape for a user's `[[recipe]]` in `conversions.toml`,
//! a built-in recipe and a plugin's `[[converter]]` (R28 5.3, question
//! 5.1): what a recipe takes, what it makes, which tool runs it with which
//! arguments, and its limits.
//!
//! A recipe is checked once, when its file loads, and every problem is a
//! sentence that names the recipe first: "Recipe png-to-jpeg: {size} is
//! not a placeholder." A recipe with a problem is left out; the others
//! load.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::placeholders;

/// How long one file may take when a recipe does not say: ten minutes.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// The longest a recipe may give one file: a day.
pub const LONGEST_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// The keys of a recipe table, in the order the guide lists them.
pub const KEYS: [&str; 13] = [
    "id",
    "name",
    "about",
    "from",
    "from-group",
    "from-type",
    "to",
    "tool",
    "args",
    "argv",
    "timeout",
    "max-input",
    "needs-encoder",
];

/// A recipe as a file writes it: the keys of `[[recipe]]`, every value as
/// written. The same keys as the config crate's `RecipeConfig`, so a
/// recipe the config folder's `conversions.toml` gave passes through
/// [`Recipe::from_serialized`] unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RecipeShape {
    /// A stable id, plain: letters, digits, `-` and `_`.
    pub id: String,
    /// The name, as listed: "Image to JPEG".
    pub name: String,
    /// What it does in one sentence, meaning first.
    pub about: String,
    /// The extensions it takes, without the dot.
    pub from: Vec<String>,
    /// A group whose files it takes: `images`.
    pub from_group: Option<String>,
    /// File type ids it takes: `png`.
    pub from_type: Vec<String>,
    /// The extension it makes, without the dot.
    pub to: String,
    /// The tool's id in the tools registry; `carta` for the built-in
    /// converter.
    pub tool: String,
    /// The arguments after the program, each one whole. A plugin's
    /// converter writes the same list as `argv`.
    #[serde(alias = "argv")]
    pub args: Vec<String>,
    /// How long one file may take: `10m`, `90s`, `2h`.
    pub timeout: Option<String>,
    /// The largest input it takes: `500 MB`.
    pub max_input: Option<String>,
    /// An FFmpeg encoder the recipe needs: offered only when the
    /// installed FFmpeg lists it (`ffmpeg -encoders`).
    pub needs_encoder: Option<String>,
}

/// Where a recipe came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeSource {
    /// Shipped with diroxide, in its built-in `conversions.toml`.
    BuiltIn,
    /// The owner's own `conversions.toml`; a recipe there replaces a
    /// built-in one with the same id.
    User,
    /// A plugin's `[[converter]]`, by the plugin's id.
    Plugin(String),
}

impl RecipeSource {
    /// Where it came from, in words: "built in", "conversions.toml",
    /// "plugin pdf-tools".
    pub fn words(&self) -> String {
        match self {
            RecipeSource::BuiltIn => "built in".to_owned(),
            RecipeSource::User => "conversions.toml".to_owned(),
            RecipeSource::Plugin(id) => format!("plugin {id}"),
        }
    }
}

/// A recipe that could not be used, and why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Recipe {recipe}: {problem}.")]
pub struct RecipeError {
    /// The recipe's id, or its place in the file ("3") when it has none.
    pub recipe: String,
    /// What is wrong, in words, without the full stop.
    pub problem: String,
}

impl RecipeError {
    fn new(recipe: &str, problem: impl Into<String>) -> Self {
        Self {
            recipe: recipe.to_owned(),
            problem: problem.into(),
        }
    }
}

/// One conversion recipe, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// Its id, plain.
    pub id: String,
    /// Its name, as listed: "Image to JPEG".
    pub name: String,
    /// What it does in one sentence.
    pub about: String,
    /// The extensions it takes, lowercase, without the dot.
    pub from: Vec<String>,
    /// A group whose files it takes.
    pub from_group: Option<String>,
    /// File type ids it takes.
    pub from_type: Vec<String>,
    /// The extension it makes, lowercase, without the dot.
    pub to: String,
    /// The tool's registry id.
    pub tool: String,
    /// The arguments, placeholders unfilled.
    pub args: Vec<String>,
    /// How long one file may take.
    pub timeout: Duration,
    /// The largest input, in bytes, when it has a limit.
    pub max_input: Option<u64>,
    /// An FFmpeg encoder it needs.
    pub needs_encoder: Option<String>,
    /// Where it came from.
    pub source: RecipeSource,
}

/// Whether a text is a plain id: letters, digits, `-` and `_`, at most
/// 64, starting with a letter or digit.
pub fn is_plain_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && text.as_bytes()[0].is_ascii_alphanumeric()
}

/// An extension as a recipe keeps it: lowercase, no leading dot.
fn extension(text: &str) -> String {
    text.trim().trim_start_matches('.').to_ascii_lowercase()
}

/// Whether an extension is plain: letters and digits, at most 16.
fn is_plain_extension(text: &str) -> bool {
    !text.is_empty() && text.len() <= 16 && text.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Reads a time limit: a whole number and `s`, `m` or `h` (`90s`, `10m`,
/// `2h`), at most a day.
pub fn parse_timeout(text: &str) -> Result<Duration, &'static str> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().map_err(|_| "is not a time limit")?;
    let seconds = match unit.trim() {
        "s" => Some(number),
        "m" => number.checked_mul(60),
        "h" => number.checked_mul(3600),
        _ => return Err("is not a time limit"),
    }
    .ok_or("is longer than a day")?;
    if seconds == 0 {
        return Err("is zero");
    }
    let limit = Duration::from_secs(seconds);
    if limit > LONGEST_TIMEOUT {
        return Err("is longer than a day");
    }
    Ok(limit)
}

/// Reads a size: a number and `B`, `KB`, `MB`, `GB` or `TB` (powers of
/// 1000), or `KiB`, `MiB`, `GiB`, `TiB` (powers of 1024), a space before
/// the unit allowed, case not minded: `500 MB`, `2GiB`.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let multiplier: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let number: f64 = number.parse().ok()?;
    let bytes = (number * multiplier).round();
    (1.0..1e18).contains(&bytes).then_some(bytes as u64)
}

impl Recipe {
    /// Checks a recipe as written and makes it one. `label` names it in a
    /// problem when it has no id ("3", its place in the file).
    pub fn from_shape(
        shape: &RecipeShape,
        source: RecipeSource,
        label: &str,
    ) -> Result<Self, RecipeError> {
        let id = shape.id.trim();
        let who = if id.is_empty() { label } else { id };
        if id.is_empty() {
            return Err(RecipeError::new(who, "it has no id"));
        }
        if !is_plain_id(id) {
            return Err(RecipeError::new(
                who,
                "its id may hold only letters, digits, - and _",
            ));
        }
        let name = shape.name.trim();
        if name.is_empty() {
            return Err(RecipeError::new(who, "it has no name"));
        }
        if name.chars().count() > 40 {
            return Err(RecipeError::new(who, "its name is longer than 40 letters"));
        }
        let from: Vec<String> = shape.from.iter().map(|e| extension(e)).collect();
        if let Some(bad) = from.iter().find(|e| !is_plain_extension(e)) {
            return Err(RecipeError::new(
                who,
                format!("from has {bad:?}, which is not an extension"),
            ));
        }
        let from_type: Vec<String> = shape
            .from_type
            .iter()
            .map(|t| t.trim().to_owned())
            .collect();
        let from_group = shape
            .from_group
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_owned);
        if from.is_empty() && from_type.is_empty() && from_group.is_none() {
            return Err(RecipeError::new(
                who,
                "it takes nothing: add from, from-group or from-type",
            ));
        }
        let to = extension(&shape.to);
        if to.is_empty() {
            return Err(RecipeError::new(
                who,
                "it makes nothing: add to, the extension it makes",
            ));
        }
        if !is_plain_extension(&to) {
            return Err(RecipeError::new(
                who,
                format!("to is {to:?}, which is not an extension"),
            ));
        }
        let tool = shape.tool.trim();
        if tool.is_empty() {
            return Err(RecipeError::new(who, "it has no tool"));
        }
        if !is_plain_id(tool) {
            return Err(RecipeError::new(
                who,
                format!("its tool {tool:?} is not a tool id"),
            ));
        }
        if let Some(unknown) = placeholders::first_unknown(&shape.args) {
            return Err(RecipeError::new(
                who,
                format!("{unknown} is not a placeholder"),
            ));
        }
        let (input, output) = placeholders::has_input_and_output(&shape.args);
        if !input {
            return Err(RecipeError::new(
                who,
                "its arguments never name the input: add {input} or {typed_input}",
            ));
        }
        if !output {
            return Err(RecipeError::new(
                who,
                "its arguments never name the output: add {output}",
            ));
        }
        let timeout = match shape.timeout.as_deref() {
            None => DEFAULT_TIMEOUT,
            Some(text) => parse_timeout(text)
                .map_err(|problem| RecipeError::new(who, format!("timeout {text:?} {problem}")))?,
        };
        let max_input = match shape.max_input.as_deref() {
            None => None,
            Some(text) => Some(parse_size(text).ok_or_else(|| {
                RecipeError::new(who, format!("max-input {text:?} is not a size"))
            })?),
        };
        let needs_encoder = match shape.needs_encoder.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(encoder) if is_plain_id(encoder) => Some(encoder.to_owned()),
            Some(encoder) => {
                return Err(RecipeError::new(
                    who,
                    format!("needs-encoder {encoder:?} is not an encoder's name"),
                ));
            }
        };
        Ok(Self {
            id: id.to_owned(),
            name: name.to_owned(),
            about: shape.about.trim().to_owned(),
            from,
            from_group,
            from_type,
            to,
            tool: tool.to_owned(),
            args: shape.args.clone(),
            timeout,
            max_input,
            needs_encoder,
            source,
        })
    }

    /// A recipe from any value with a recipe's keys, such as the config
    /// crate's `RecipeConfig`: it passes through TOML's value form into a
    /// [`RecipeShape`], then [`Recipe::from_shape`]. The app calls this for
    /// each recipe of the owner's `conversions.toml`.
    pub fn from_serialized<T: Serialize>(
        value: &T,
        source: RecipeSource,
        label: &str,
    ) -> Result<Self, RecipeError> {
        let unreadable = || RecipeError::new(label, "its keys could not be read");
        let shape: RecipeShape = toml::Value::try_from(value)
            .map_err(|_| unreadable())?
            .try_into()
            .map_err(|_| unreadable())?;
        Self::from_shape(&shape, source, label)
    }

    /// The recipes of one plugin's `[[converter]]`, one per output it
    /// makes: the one recipe shape (R28 5.3). The converter's `argv` names
    /// its program first; that program must be a tool it `needs`, found
    /// through the tools registry like any recipe's tool. A converter that
    /// runs through the plugin's own process is not a recipe and gives
    /// none here.
    pub fn from_converter(converter: &ConverterParts) -> Vec<Result<Self, RecipeError>> {
        let label = format!("{}-{}", converter.plugin, converter.id);
        let Some((program, args)) = converter.argv.split_first() else {
            return Vec::new();
        };
        if !converter.needs.iter().any(|need| need == program) {
            return vec![Err(RecipeError::new(
                &label,
                format!("its program {program:?} is not a tool it needs"),
            ))];
        }
        converter
            .outputs
            .iter()
            .map(|output| {
                let shape = RecipeShape {
                    id: format!("{}-{}-{}", converter.plugin, converter.id, output.id),
                    name: output_name(&converter.name, &output.name),
                    about: String::new(),
                    from: converter.extensions.clone(),
                    from_group: None,
                    from_type: converter.types.clone(),
                    to: output.extension.clone(),
                    tool: program.clone(),
                    args: args.to_vec(),
                    timeout: converter.timeout.clone(),
                    max_input: None,
                    needs_encoder: None,
                };
                Self::from_shape(
                    &shape,
                    RecipeSource::Plugin(converter.plugin.clone()),
                    &label,
                )
            })
            .collect()
    }

    /// Whether the recipe takes a file by its extension, its type id or
    /// its type's groups.
    pub fn takes(&self, extension: Option<&str>, type_id: Option<&str>, groups: &[String]) -> bool {
        let by_extension = extension
            .map(str::to_ascii_lowercase)
            .is_some_and(|e| self.from.contains(&e));
        let by_type = type_id.is_some_and(|t| self.from_type.iter().any(|f| f == t));
        let by_group = self
            .from_group
            .as_ref()
            .is_some_and(|g| groups.iter().any(|have| have == g));
        by_extension || by_type || by_group
    }

    /// What the recipe makes, in words, for "Convert 40 files to JPEG? y
    /// or n": the part of its name after its last " to ", or else the
    /// extension in capitals.
    pub fn makes_words(&self) -> String {
        match self.name.rsplit_once(" to ") {
            Some((_, made)) if !made.trim().is_empty() => made.trim().to_owned(),
            _ => self.to.to_ascii_uppercase(),
        }
    }
}

/// One output of a plugin's converter: its `[[converter.to]]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConverterOutput {
    /// Its id.
    pub id: String,
    /// Its name in words: "Word document".
    pub name: String,
    /// The extension it makes.
    pub extension: String,
}

/// A plugin's `[[converter]]` as the recipe model needs it, filled by the
/// plugin host from its manifest's `ConverterSpec` (the shape lives in the
/// plugins crate, which this crate does not see).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConverterParts {
    /// The plugin's id.
    pub plugin: String,
    /// The converter's id.
    pub id: String,
    /// The converter's name.
    pub name: String,
    /// `from.extensions`.
    pub extensions: Vec<String>,
    /// `from.types`.
    pub types: Vec<String>,
    /// The outputs.
    pub outputs: Vec<ConverterOutput>,
    /// The tools it needs.
    pub needs: Vec<String>,
    /// The program and its arguments.
    pub argv: Vec<String>,
    /// The manifest's convert time limit, as written.
    pub timeout: Option<String>,
}

/// A converter's name for one output: its own name when that already
/// says what it makes ("Markdown to Word"), else "NAME to OUTPUT".
fn output_name(converter: &str, output: &str) -> String {
    if output.trim().is_empty() || converter.contains(" to ") {
        converter.to_owned()
    } else {
        format!("{converter} to {output}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> RecipeShape {
        RecipeShape {
            id: "image-to-jpeg".into(),
            name: "Image to JPEG".into(),
            about: "Makes a JPEG copy of each picture, quality 90.".into(),
            from: vec![".PNG".into(), "webp".into()],
            from_group: Some("images".into()),
            from_type: Vec::new(),
            to: "jpg".into(),
            tool: "magick".into(),
            args: vec![
                "{typed_input}".into(),
                "-quality".into(),
                "90".into(),
                "{output}".into(),
            ],
            timeout: Some("10m".into()),
            max_input: Some("500 MB".into()),
            needs_encoder: None,
        }
    }

    fn problem(shape: &RecipeShape) -> String {
        Recipe::from_shape(shape, RecipeSource::User, "1")
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn a_good_recipe_is_read_with_its_limits() {
        let recipe = Recipe::from_shape(&shape(), RecipeSource::BuiltIn, "1").unwrap();
        assert_eq!(recipe.from, ["png", "webp"]);
        assert_eq!(recipe.timeout, Duration::from_secs(600));
        assert_eq!(recipe.max_input, Some(500_000_000));
        assert!(recipe.takes(Some("PNG"), None, &[]));
        assert!(recipe.takes(Some("tif"), Some("tiff"), &["images".to_owned()]));
        assert!(!recipe.takes(Some("txt"), Some("text"), &["text".to_owned()]));
        assert_eq!(recipe.makes_words(), "JPEG");
        assert_eq!(RecipeSource::Plugin("pdf".into()).words(), "plugin pdf");
    }

    /// Each problem names the recipe first, then what is wrong, inside a
    /// Braille line where the recipe's id is short.
    #[test]
    fn each_problem_names_the_recipe_first() {
        let mut bad = shape();
        bad.id = "png-to-jpeg".into();
        bad.args.push("{size}".into());
        assert_eq!(
            problem(&bad),
            "Recipe png-to-jpeg: {size} is not a placeholder."
        );
        assert!(problem(&bad).chars().count() <= 48);
        let mut files = shape();
        files.args = vec!["{files}".into(), "{output}".into()];
        assert_eq!(
            problem(&files),
            "Recipe image-to-jpeg: {files} is not a placeholder."
        );
        let mut no_id = shape();
        no_id.id = String::new();
        assert_eq!(problem(&no_id), "Recipe 1: it has no id.");
        let mut odd = shape();
        odd.id = "a b".into();
        assert!(problem(&odd).starts_with("Recipe a b: its id may hold only"));
        let mut nothing = shape();
        nothing.from.clear();
        nothing.from_group = None;
        assert!(problem(&nothing).contains("it takes nothing"));
        let mut no_output = shape();
        no_output.args = vec!["{input}".into()];
        assert!(problem(&no_output).contains("never name the output"));
        let mut no_input = shape();
        no_input.args = vec!["{output}".into()];
        assert!(problem(&no_input).contains("never name the input"));
        let mut slow = shape();
        slow.timeout = Some("3d".into());
        assert_eq!(
            problem(&slow),
            "Recipe image-to-jpeg: timeout \"3d\" is not a time limit."
        );
        let mut huge = shape();
        huge.max_input = Some("lots".into());
        assert!(problem(&huge).contains("max-input \"lots\" is not a size"));
        let mut long = shape();
        long.name = "A recipe whose name runs well past a Braille line".into();
        assert!(problem(&long).contains("longer than 40"));
        let mut odd_to = shape();
        odd_to.to = "tar.gz".into();
        assert!(problem(&odd_to).contains("not an extension"));
    }

    #[test]
    fn limits_read_in_words_and_numbers() {
        assert_eq!(parse_timeout("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(parse_timeout("2h"), Ok(Duration::from_secs(7200)));
        assert_eq!(parse_timeout("0s"), Err("is zero"));
        assert_eq!(parse_timeout("25h"), Err("is longer than a day"));
        assert_eq!(parse_size("500 MB"), Some(500_000_000));
        assert_eq!(parse_size("2GiB"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1.5 kb"), Some(1500));
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("ten"), None);
    }

    /// The config crate's recipe passes through TOML's value form: a
    /// stand-in with the same serde shape proves the bridge.
    #[test]
    fn a_recipe_from_another_crates_shape_passes_through() {
        #[derive(Serialize)]
        #[serde(rename_all = "kebab-case")]
        struct Other {
            id: String,
            name: String,
            from: Vec<String>,
            from_group: Option<String>,
            to: String,
            tool: String,
            args: Vec<String>,
            timeout: Option<String>,
            needs_encoder: Option<String>,
        }
        let other = Other {
            id: "audio-to-mp3".into(),
            name: "Audio to MP3".into(),
            from: vec!["flac".into()],
            from_group: None,
            to: "mp3".into(),
            tool: "ffmpeg".into(),
            args: vec!["-i".into(), "file:{input}".into(), "{output}".into()],
            timeout: None,
            needs_encoder: Some("libmp3lame".into()),
        };
        let recipe = Recipe::from_serialized(&other, RecipeSource::User, "1").unwrap();
        assert_eq!(recipe.needs_encoder.as_deref(), Some("libmp3lame"));
        assert_eq!(recipe.source, RecipeSource::User);
        assert_eq!(recipe.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn a_plugins_converter_gives_one_recipe_per_output() {
        let converter = ConverterParts {
            plugin: "docs".into(),
            id: "md_to_docx".into(),
            name: "Markdown to Word".into(),
            extensions: vec!["md".into()],
            types: Vec::new(),
            outputs: vec![ConverterOutput {
                id: "docx".into(),
                name: "Word document".into(),
                extension: "docx".into(),
            }],
            needs: vec!["pandoc".into()],
            argv: vec![
                "pandoc".into(),
                "--sandbox".into(),
                "--output".into(),
                "{output}".into(),
                "{input}".into(),
            ],
            timeout: Some("2m".into()),
        };
        let recipes = Recipe::from_converter(&converter);
        assert_eq!(recipes.len(), 1);
        let recipe = recipes[0].as_ref().unwrap();
        assert_eq!(recipe.id, "docs-md_to_docx-docx");
        assert_eq!(recipe.tool, "pandoc");
        assert_eq!(recipe.args[0], "--sandbox");
        assert_eq!(recipe.name, "Markdown to Word");
        assert_eq!(recipe.source, RecipeSource::Plugin("docs".into()));
        let mut stray = converter.clone();
        stray.argv[0] = "/usr/bin/pandoc".into();
        assert!(
            Recipe::from_converter(&stray)[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("not a tool it needs")
        );
        let mut process = converter;
        process.argv.clear();
        assert!(Recipe::from_converter(&process).is_empty());
    }
}
