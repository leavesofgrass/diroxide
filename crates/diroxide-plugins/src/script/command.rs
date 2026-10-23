//! A script command, whichever way it was written: a one-file script with
//! a header, or a manifest-only `[[command]]` with an `argv` and no
//! `[run]` program. Both run through the same runner with the same
//! context, sentences and safety rules.

use std::path::{Path, PathBuf};

use crate::manifest::{CommandSpec, Manifest, Os, Places, Problem, Selection};
use crate::script::header::ScriptHeader;
use crate::script::recipe::{RECIPE_KEYS, Recipe, RunMode};

/// The prefix of a script's command id: `script.total-size`.
pub const SCRIPT_ID_PREFIX: &str = "script";

/// What runs: a script file through its interpreter, or a manifest's
/// argument list with placeholders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    /// A script file, run by the program [`crate::script::interpreter`]
    /// chooses.
    Script(PathBuf),
    /// A program and its arguments, placeholders not yet filled.
    Argv(Vec<String>),
}

/// A command the script runner can run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptCommand {
    /// The full command id: `script.total-size` or
    /// `system-index.everything`.
    pub id: String,
    /// Its name in menus, the palette and its sentences.
    pub title: String,
    /// Who made it, last in every line its text causes: the script's name,
    /// or the plugin's.
    pub source: String,
    /// The plugin's id, for a manifest's command; the script's id (its
    /// file stem) for a script.
    pub owner_id: String,
    /// One sentence for F1 help.
    pub description: String,
    /// How many entries it needs.
    pub selection: Selection,
    /// Which places it works on.
    pub places: Places,
    /// The tools it needs, by name.
    pub needs: Vec<String>,
    /// The systems it exists on; empty means every system.
    pub os: Vec<Os>,
    /// What runs.
    pub program: Program,
    /// The folder of the script or the plugin.
    pub folder: PathBuf,
    /// How it runs.
    pub recipe: Recipe,
}

impl ScriptCommand {
    /// The command of a script file and its header. The id is the file's
    /// stem, which the caller has checked is a plain id.
    pub fn from_script(path: &Path, header: &ScriptHeader) -> Self {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = header.name_or(&stem);
        Self {
            id: format!("{SCRIPT_ID_PREFIX}.{stem}"),
            title: title.clone(),
            source: title,
            owner_id: stem,
            description: header.description.clone(),
            selection: header.selection,
            places: header.places,
            needs: header.needs.clone(),
            os: header.os.clone(),
            program: Program::Script(path.to_path_buf()),
            folder: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            recipe: header.recipe.clone(),
        }
    }

    /// The command of a manifest-only `[[command]]`, with the problems of
    /// its run keys (named `command[i].key`). `None` when the command has
    /// no `argv` (it is answered by the plugin's program instead).
    pub fn from_manifest(
        manifest: &Manifest,
        index: usize,
        spec: &CommandSpec,
        folder: &Path,
    ) -> Option<(Self, Vec<Problem>)> {
        if spec.argv.is_empty() {
            return None;
        }
        let mut table = spec.extra.clone();
        table.retain(|key, _| {
            let key: &str = key;
            RECIPE_KEYS.contains(&key)
        });
        if let Some(mode) = &spec.mode {
            table.insert("mode".to_owned(), toml::Value::String(mode.clone()));
        }
        let label = |key: &str| format!("command[{index}].{key}");
        let (recipe, problems) = Recipe::read(&table, &label);
        Some((
            Self {
                id: spec.full_id(&manifest.id),
                title: spec.name.clone(),
                source: manifest.name.clone(),
                owner_id: manifest.id.clone(),
                description: spec.description.clone(),
                selection: spec.selection,
                places: spec.places,
                needs: spec.needs.clone(),
                os: spec.os.clone(),
                program: Program::Argv(spec.argv.clone()),
                folder: folder.to_path_buf(),
                recipe,
            },
            problems,
        ))
    }

    /// Whether it exists on this system.
    pub fn exists_on(&self, os: Os) -> bool {
        self.os.is_empty() || self.os.contains(&os)
    }

    /// Whether it is a script file.
    pub fn is_script(&self) -> bool {
        matches!(self.program, Program::Script(_))
    }

    /// Whether its output is opened as a lister of places.
    pub fn is_lister(&self) -> bool {
        self.recipe.mode == RunMode::Lister
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::header::parse_header;

    #[test]
    fn a_script_is_named_by_its_header_or_its_file() {
        let header = parse_header("# diro: name = \"Total size of selection\"\n").header;
        let command = ScriptCommand::from_script(Path::new("/s/total-size.sh"), &header);
        assert_eq!(command.id, "script.total-size");
        assert_eq!(command.title, "Total size of selection");
        assert_eq!(command.source, command.title);
        assert!(command.is_script());
        let plain =
            ScriptCommand::from_script(Path::new("/s/make_thumbs.py"), &parse_header("").header);
        assert_eq!(plain.title, "make thumbs");
    }

    #[test]
    fn a_manifest_only_command_reads_its_run_keys() {
        let text = r#"
[plugin]
id = "system-index"
name = "Search the whole computer"
version = "0.1.0"
protocol = 1

[[command]]
id = "plocate"
name = "Search with plocate"
selection = "none"
mode = "lister"
list = "nul"
max_rows = 5000
timeout = "60s"
prompt = "Search the whole computer for:"
argv = ["plocate", "-0", "-i", "-e", "-l", "{max_rows}", "--", "{input}"]
"#;
        let manifest = Manifest::check_for(text, Os::Linux).manifest.unwrap();
        let (command, problems) =
            ScriptCommand::from_manifest(&manifest, 0, &manifest.commands[0], Path::new("/p"))
                .unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(command.id, "system-index.plocate");
        assert_eq!(command.source, "Search the whole computer");
        assert!(command.is_lister());
        assert_eq!(
            command.recipe.prompt.as_deref(),
            Some("Search the whole computer for:")
        );
    }
}
