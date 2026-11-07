//! The recipes in use: the built-in ones, then the owner's
//! `conversions.toml` (a recipe there with a built-in id replaces it),
//! then the plugins' converters. Reading never stops: a recipe with a
//! problem is left out and its sentence kept in [`RecipeBook::problems`],
//! the recipe's id first.

use serde::Serialize;

use super::recipe::{KEYS, Recipe, RecipeError, RecipeShape, RecipeSource};

/// The recipes in use, in the order they are offered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipeBook {
    recipes: Vec<Recipe>,
    problems: Vec<String>,
}

/// Reads the `[[recipe]]` tables of a `conversions.toml` text, each on its
/// own, so one recipe with a problem leaves the others; unknown keys are
/// problems too, since a misspelled key would silently do nothing.
pub fn read_toml(text: &str, source: &RecipeSource) -> (Vec<Recipe>, Vec<String>) {
    let mut recipes = Vec::new();
    let mut problems = Vec::new();
    let table: toml::Table = match toml::from_str(text) {
        Ok(table) => table,
        Err(error) => {
            let message = error.message().trim_end_matches('.').to_owned();
            problems.push(format!(
                "Recipes from {} could not be read: {message}.",
                source.words()
            ));
            return (recipes, problems);
        }
    };
    let Some(list) = table.get("recipe") else {
        return (recipes, problems);
    };
    let Some(list) = list.as_array() else {
        problems.push(format!(
            "Recipes from {}: recipe must be [[recipe]] tables.",
            source.words()
        ));
        return (recipes, problems);
    };
    for (i, value) in list.iter().enumerate() {
        let label = (i + 1).to_string();
        let id = value
            .get("id")
            .and_then(toml::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .unwrap_or(&label)
            .to_owned();
        if let Some(table) = value.as_table()
            && let Some(key) = table.keys().find(|key| !KEYS.contains(&key.as_str()))
        {
            problems.push(
                RecipeError {
                    recipe: id,
                    problem: format!("{key} is not a recipe key"),
                }
                .to_string(),
            );
            continue;
        }
        let shape: Result<RecipeShape, _> = value.clone().try_into();
        match shape {
            Ok(shape) => match Recipe::from_shape(&shape, source.clone(), &label) {
                Ok(recipe) => recipes.push(recipe),
                Err(error) => problems.push(error.to_string()),
            },
            Err(error) => problems.push(
                RecipeError {
                    recipe: id,
                    problem: error.message().trim_end_matches('.').to_owned(),
                }
                .to_string(),
            ),
        }
    }
    (recipes, problems)
}

impl RecipeBook {
    /// A book with no recipes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a recipe. One with an id already in the book replaces it in
    /// its place, so the owner's recipe of a built-in id keeps the
    /// built-in's position in lists.
    pub fn add(&mut self, recipe: Recipe) {
        match self.recipes.iter_mut().find(|r| r.id == recipe.id) {
            Some(old) => *old = recipe,
            None => self.recipes.push(recipe),
        }
    }

    /// Adds the recipes of a `conversions.toml` text, keeping its
    /// problems.
    pub fn add_toml(&mut self, text: &str, source: &RecipeSource) {
        let (recipes, problems) = read_toml(text, source);
        for recipe in recipes {
            self.add(recipe);
        }
        self.problems.extend(problems);
    }

    /// Adds recipes from values with a recipe's keys: the config crate's
    /// `RecipeConfig` list, already read from the owner's file.
    pub fn add_serialized<T: Serialize>(&mut self, recipes: &[T], source: &RecipeSource) {
        for (i, value) in recipes.iter().enumerate() {
            match Recipe::from_serialized(value, source.clone(), &(i + 1).to_string()) {
                Ok(recipe) => self.add(recipe),
                Err(error) => self.problems.push(error.to_string()),
            }
        }
    }

    /// Adds a plugin's converter as recipes, one per output.
    pub fn add_converter(&mut self, converter: &super::recipe::ConverterParts) {
        for made in Recipe::from_converter(converter) {
            match made {
                Ok(recipe) => self.add(recipe),
                Err(error) => self.problems.push(error.to_string()),
            }
        }
    }

    /// The recipes, in the order they are offered.
    pub fn recipes(&self) -> &[Recipe] {
        &self.recipes
    }

    /// The recipe with this id.
    pub fn get(&self, id: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.id == id)
    }

    /// Every problem met while reading, one sentence each, the recipe's id
    /// first.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The recipes that take a file with this extension, type and groups,
    /// in the book's order.
    pub fn taking(
        &self,
        extension: Option<&str>,
        type_id: Option<&str>,
        groups: &[String],
    ) -> Vec<&Recipe> {
        self.recipes
            .iter()
            .filter(|r| r.takes(extension, type_id, groups))
            .collect()
    }

    /// The recipe `diro convert --to FORMAT` picks for a file: the first
    /// that takes it and makes `to` (an extension, the dot optional, or
    /// the name's last words, "Word"), carta's first, then the book's
    /// order.
    pub fn pick_for(
        &self,
        to: &str,
        extension: Option<&str>,
        type_id: Option<&str>,
        groups: &[String],
    ) -> Option<&Recipe> {
        let wanted = to.trim().trim_start_matches('.').to_ascii_lowercase();
        let makes = |r: &&Recipe| r.to == wanted || r.makes_words().to_ascii_lowercase() == wanted;
        let mut taking: Vec<&Recipe> = self
            .taking(extension, type_id, groups)
            .into_iter()
            .filter(makes)
            .collect();
        taking.sort_by_key(|r| r.tool != "carta");
        taking.into_iter().next()
    }
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
args = ["{typed_input}", "{output}"]

[[recipe]]
id = "png-to-jpeg"
name = "PNG to JPEG"
from = ["png"]
to = "jpg"
tool = "magick"
args = ["{typed_input}", "-resize", "{size}", "{output}"]

[[recipe]]
id = "spelled"
name = "Spelled wrong"
from = ["md"]
to = "html"
tool = "pandoc"
argz = ["{input}", "{output}"]

[[recipe]]
id = "listless"
name = "Args as text"
from = ["md"]
to = "html"
tool = "pandoc"
args = "{input} {output}"
"#;

    #[test]
    fn one_bad_recipe_leaves_the_others_and_is_said() {
        let (recipes, problems) = read_toml(TEXT, &RecipeSource::User);
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].id, "image-to-jpeg");
        assert_eq!(
            problems,
            [
                "Recipe png-to-jpeg: {size} is not a placeholder.",
                "Recipe spelled: argz is not a recipe key.",
                "Recipe listless: invalid type: string \"{input} {output}\", expected a sequence."
            ]
        );
        let (none, said) = read_toml("[[recipe]\n", &RecipeSource::User);
        assert!(none.is_empty());
        assert!(said[0].starts_with("Recipes from conversions.toml could not be read: "));
    }

    #[test]
    fn the_owners_recipe_replaces_a_built_in_one_in_its_place() {
        let mut book = RecipeBook::new();
        book.add_toml(TEXT, &RecipeSource::BuiltIn);
        book.add_toml(
            r#"
[[recipe]]
id = "md-to-html"
name = "Markdown to a web page"
from = ["md"]
to = "html"
tool = "carta"
args = ["--from", "markdown", "--to", "html", "{input}", "{output}"]

[[recipe]]
id = "md-to-html-pandoc"
name = "Markdown to HTML"
from = ["md"]
to = "html"
tool = "pandoc"
args = ["{input}", "-o", "{output}"]

[[recipe]]
id = "image-to-jpeg"
name = "Image to JPEG, quality 80"
from-group = "images"
to = "jpg"
tool = "magick"
args = ["{typed_input}", "-quality", "80", "{output}"]
"#,
            &RecipeSource::User,
        );
        let ids: Vec<&str> = book.recipes().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["image-to-jpeg", "md-to-html", "md-to-html-pandoc"]);
        assert_eq!(book.recipes()[0].source, RecipeSource::User);
        assert_eq!(book.problems().len(), 3);
        let images = ["images".to_owned()];
        assert_eq!(
            book.pick_for("JPG", Some("png"), Some("png"), &images)
                .map(|r| r.id.as_str()),
            Some("image-to-jpeg")
        );
        // carta first, whatever the book's order.
        assert_eq!(
            book.pick_for(".html", Some("md"), None, &[])
                .map(|r| r.tool.as_str()),
            Some("carta")
        );
        assert_eq!(
            book.pick_for("a web page", Some("md"), None, &[])
                .map(|r| r.id.as_str()),
            Some("md-to-html")
        );
        assert!(book.pick_for("mp3", Some("md"), None, &[]).is_none());
        assert_eq!(
            book.get("md-to-html-pandoc").map(|r| r.to.as_str()),
            Some("html")
        );
    }
}
