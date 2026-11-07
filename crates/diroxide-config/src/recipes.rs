//! Reading the owner's `conversions.toml` recipe by recipe (alpha.7, R28
//! 5.3): one recipe that cannot be used is left out with a sentence that
//! names it first ("Recipe png-to-jpeg: {size} is not a placeholder."),
//! and the others load. The job engine checks each recipe again, in full,
//! when the app builds its recipe book; this reader catches what the
//! owner must hear about the moment the file loads.

use crate::files::{ConversionsConfig, RecipeConfig};
use crate::paths::Paths;
use crate::store::ConfigProblem;

/// The placeholders a recipe may use, without the braces: the job
/// engine's list (`diroxide_ops::convert::placeholders::PLACEHOLDERS`),
/// repeated here because this crate does not depend on the engine.
pub const RECIPE_PLACEHOLDERS: [&str; 9] = [
    "input",
    "typed_input",
    "output",
    "stem",
    "name",
    "file",
    "dir",
    "dest",
    "other",
];

/// The placeholders an argument names, without the braces: `{` and a run
/// of lowercase letters, digits and underscores closed by `}`; `{{` is a
/// written brace.
fn placeholders_in(argument: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = argument;
    while let Some(at) = rest.find('{') {
        let after = &rest[at + 1..];
        if let Some(written) = after.strip_prefix('{') {
            rest = written;
            continue;
        }
        match after.find('}') {
            Some(end)
                if end > 0
                    && after[..end]
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') =>
            {
                out.push(&after[..end]);
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    out
}

/// Whether an id is plain: letters, digits, `-` and `_`.
fn is_plain(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The problem with one recipe the moment its file loads, if any, in
/// words that name the recipe: its id, or its place in the file.
pub fn recipe_problem(recipe: &RecipeConfig, place: usize) -> Option<String> {
    let id = recipe.id.trim();
    let who = if id.is_empty() {
        place.to_string()
    } else {
        id.to_owned()
    };
    let problem = if id.is_empty() {
        "it has no id".to_owned()
    } else if !is_plain(id) {
        "its id may hold only letters, digits, - and _".to_owned()
    } else if let Some(unknown) = recipe
        .args
        .iter()
        .flat_map(|argument| placeholders_in(argument))
        .find(|name| !RECIPE_PLACEHOLDERS.contains(name))
    {
        format!("{{{unknown}}} is not a placeholder")
    } else {
        return None;
    };
    Some(format!("Recipe {who}: {problem}."))
}

/// Reads `conversions.toml` text one `[[recipe]]` at a time: a recipe
/// whose keys cannot be read, or that has a problem
/// ([`recipe_problem`]), or whose id an earlier recipe took, is left out
/// and said in `problems`. The text is already known to be TOML.
pub fn read_conversions(text: &str, problems: &mut Vec<ConfigProblem>) -> ConversionsConfig {
    let say = |problems: &mut Vec<ConfigProblem>, line: String| {
        problems.push(ConfigProblem::new(Paths::CONVERSIONS, line));
    };
    let mut kept: Vec<RecipeConfig> = Vec::new();
    let Ok(table) = toml::from_str::<toml::Table>(text) else {
        return ConversionsConfig::default();
    };
    let list = match table.get("recipe") {
        None => return ConversionsConfig::default(),
        Some(toml::Value::Array(list)) => list,
        Some(_) => {
            say(problems, "Recipes must be [[recipe]] tables.".to_owned());
            return ConversionsConfig::default();
        }
    };
    for (i, value) in list.iter().enumerate() {
        let place = i + 1;
        let recipe: RecipeConfig = match value.clone().try_into() {
            Ok(recipe) => recipe,
            Err(error) => {
                let who = value
                    .get("id")
                    .and_then(toml::Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .map_or_else(|| place.to_string(), str::to_owned);
                let cause = error.message().trim_end_matches('.').to_owned();
                say(problems, format!("Recipe {who}: {cause}."));
                continue;
            }
        };
        if let Some(line) = recipe_problem(&recipe, place) {
            say(problems, line);
            continue;
        }
        if kept.iter().any(|earlier| earlier.id == recipe.id) {
            say(
                problems,
                format!(
                    "Recipe {}: its id is used twice, so the second is left out.",
                    recipe.id
                ),
            );
            continue;
        }
        kept.push(recipe);
    }
    ConversionsConfig { recipes: kept }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"
[[recipe]]
id = "image-to-jpeg"
name = "Image to JPEG"
from-group = "images"
to = "jpg"
tool = "magick"
args = ["{typed_input}", "-resize", "1600x1600>", "{output}", "{{literal}}"]

[[recipe]]
id = "png-to-jpeg"
name = "PNG to JPEG"
from = ["png"]
to = "jpg"
tool = "magick"
args = ["{typed_input}", "-resize", "{size}", "{output}"]

[[recipe]]
id = "listless"
name = "Args as text"
from = ["md"]
to = "html"
tool = "pandoc"
args = "{input} {output}"

[[recipe]]
name = "No id"
from = ["md"]
to = "html"
tool = "pandoc"
args = ["{input}", "{output}"]

[[recipe]]
id = "image-to-jpeg"
name = "Image to JPEG again"
from = ["png"]
to = "jpg"
tool = "magick"
argv = ["{typed_input}", "{output}"]
"#;

    /// The owner hears each bad recipe once, its id first, when the file
    /// loads; the good one is kept.
    #[test]
    fn each_bad_recipe_is_said_by_its_id_and_the_rest_load() {
        let mut problems = Vec::new();
        let read = read_conversions(TEXT, &mut problems);
        assert_eq!(read.recipes.len(), 1);
        assert_eq!(read.recipes[0].id, "image-to-jpeg");
        let lines: Vec<&str> = problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            lines,
            [
                "Recipe png-to-jpeg: {size} is not a placeholder.",
                "Recipe listless: invalid type: string \"{input} {output}\", expected a sequence.",
                "Recipe 4: it has no id.",
                "Recipe image-to-jpeg: its id is used twice, so the second is left out.",
            ]
        );
        assert!(problems.iter().all(|p| p.file == Paths::CONVERSIONS));
        assert!(lines[0].chars().count() <= 48);
    }

    /// Through the whole load: the startup line counts the recipe's
    /// problem, and the good recipe is there.
    #[test]
    fn the_config_folders_load_says_a_bad_recipe() {
        let folder = tempfile::tempdir().unwrap();
        let paths = Paths::new(folder.path().join("config"), folder.path().join("state"));
        std::fs::create_dir_all(folder.path().join("config")).unwrap();
        std::fs::write(paths.config_file(Paths::CONVERSIONS), TEXT).unwrap();
        let config = crate::load(&paths);
        assert_eq!(config.conversions.recipes.len(), 1);
        let said: Vec<&str> = config
            .problems
            .iter()
            .filter(|p| p.file == Paths::CONVERSIONS)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(said.len(), 4);
        assert_eq!(said[0], "Recipe png-to-jpeg: {size} is not a placeholder.");
    }

    #[test]
    fn placeholders_are_read_the_engines_way() {
        assert_eq!(placeholders_in("file:{input}"), ["input"]);
        assert_eq!(placeholders_in("{stem}-{dir}"), ["stem", "dir"]);
        assert!(placeholders_in("{{x}}").is_empty());
        assert!(placeholders_in("{ x }").is_empty());
        assert!(placeholders_in("1600x1600>").is_empty());
        let mut problems = Vec::new();
        assert!(read_conversions("", &mut problems).recipes.is_empty());
        assert!(
            read_conversions("recipe = 1\n", &mut problems)
                .recipes
                .is_empty()
        );
        assert_eq!(problems[0].text, "Recipes must be [[recipe]] tables.");
    }
}
