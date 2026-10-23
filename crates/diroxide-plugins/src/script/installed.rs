//! The plugins and scripts in the plugins folder, found and checked.
//!
//! - **Plugins** are folders with a `plugin.toml`. **Scripts** are single
//!   files in `plugins/scripts/` (`.sh`, `.bash`, `.py`, `.ps1`, `.cmd`,
//!   `.bat`, and on Linux and macOS any executable file). Scripts run only
//!   from there, never from the current folder or an archive.
//! - A script's id is its file's stem, which must be a plain id: lowercase
//!   letters, digits, `-` and `_`. A name with anything else is refused
//!   with the first bad character spelled out ("U+0020 is not allowed in
//!   a name"), because names that need escaping are how files go wrong.
//! - [`check`] is `diro plugin check`: the manifest's or the header's
//!   problems, the run keys of manifest-only commands, placeholders no
//!   command has, the `unattended` promise, folder trust, and whether a
//!   program runs the script here. Each problem names its field and the
//!   fix.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestReport, Os, Problem, Severity};
use crate::script::command::ScriptCommand;
use crate::script::consent::trust_problem;
use crate::script::header::{HeaderReport, parse_header};
use crate::script::interpreter::{Lookup, ScriptKind};
use crate::script::placeholders::{KNOWN, unknown_names};

/// The scripts folder's name, inside the plugins folder.
pub const SCRIPTS: &str = "scripts";

/// The extensions a script may have.
pub const SCRIPT_EXTENSIONS: [&str; 6] = ["sh", "bash", "py", "ps1", "cmd", "bat"];

/// The scripts folder of a plugins folder.
pub fn scripts_dir(plugins_dir: &Path) -> PathBuf {
    plugins_dir.join(SCRIPTS)
}

/// A plugin folder.
#[derive(Clone, Debug)]
pub struct PluginEntry {
    /// Its id: the manifest's, or the folder's name when the manifest
    /// cannot be read.
    pub id: String,
    /// Its folder.
    pub folder: PathBuf,
    /// The manifest read for this system, and every problem.
    pub report: ManifestReport,
}

/// A script file.
#[derive(Clone, Debug)]
pub struct ScriptEntry {
    /// Its id: the file's stem.
    pub id: String,
    /// The file.
    pub path: PathBuf,
    /// Its header, and every problem.
    pub header: HeaderReport,
}

/// A plugin or a script.
#[derive(Clone, Debug)]
pub enum Installed {
    /// A plugin folder.
    Plugin(Box<PluginEntry>),
    /// A script file.
    Script(Box<ScriptEntry>),
}

impl Installed {
    /// Its id.
    pub fn id(&self) -> &str {
        match self {
            Installed::Plugin(p) => &p.id,
            Installed::Script(s) => &s.id,
        }
    }

    /// Its name in words.
    pub fn name(&self) -> String {
        match self {
            Installed::Plugin(p) => p
                .report
                .manifest
                .as_ref()
                .map_or_else(|| p.id.clone(), |m| m.name.clone()),
            Installed::Script(s) => s.header.header.name_or(&s.id),
        }
    }

    /// "plugin 0.1.0" or "script".
    pub fn kind_words(&self) -> String {
        match self {
            Installed::Plugin(p) => match &p.report.manifest {
                Some(m) => format!("plugin {}", m.version),
                None => "plugin".to_owned(),
            },
            Installed::Script(_) => "script".to_owned(),
        }
    }

    /// The manifest, for a plugin that has a readable one.
    pub fn manifest(&self) -> Option<&Manifest> {
        match self {
            Installed::Plugin(p) => p.report.manifest.as_ref(),
            Installed::Script(_) => None,
        }
    }

    /// The script's command, for a script.
    pub fn script_command(&self) -> Option<ScriptCommand> {
        match self {
            Installed::Script(s) => Some(ScriptCommand::from_script(&s.path, &s.header.header)),
            Installed::Plugin(_) => None,
        }
    }
}

/// Why text is not a plain name, with the first bad character spelled
/// out: "U+0020 is not allowed in a name". `None` when it is plain:
/// lowercase letters, digits, `-` and `_`, starting and ending with a
/// letter or digit, 2 to 40 characters.
pub fn plain_name_problem(name: &str) -> Option<String> {
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
    {
        let code = format!("U+{:04X}", u32::from(bad));
        return Some(if bad.is_ascii_uppercase() {
            format!("{code} is an uppercase letter; use lowercase")
        } else {
            format!("{code} is not allowed in a name")
        });
    }
    let count = name.chars().count();
    if count < 2 {
        return Some("is shorter than 2 characters".to_owned());
    }
    if count > 40 {
        return Some(format!("is {count} characters, over 40"));
    }
    if name.starts_with(['-', '_']) || name.ends_with(['-', '_']) {
        return Some("starts or ends with - or _".to_owned());
    }
    None
}

/// Whether a file in the scripts folder is a script.
fn is_script_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.starts_with('.') || !path.is_file() {
        return false;
    }
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
    {
        Some(ext) if SCRIPT_EXTENSIONS.contains(&ext.as_str()) => true,
        Some(_) => false,
        None => executable(path),
    }
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_path: &Path) -> bool {
    false
}

/// Reads a plugin folder.
pub fn read_plugin(folder: &Path, os: Os) -> Option<PluginEntry> {
    let text = fs::read_to_string(folder.join(Manifest::FILE)).ok()?;
    let report = Manifest::check_for(&text, os);
    let id = report.manifest.as_ref().map_or_else(
        || {
            folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        },
        |m| m.id.clone(),
    );
    Some(PluginEntry {
        id,
        folder: folder.to_path_buf(),
        report,
    })
}

/// Reads a script file.
pub fn read_script(path: &Path) -> Option<ScriptEntry> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let id = path.file_stem()?.to_string_lossy().into_owned();
    Some(ScriptEntry {
        id,
        path: path.to_path_buf(),
        header: parse_header(&text),
    })
}

/// Reads a plugin folder or a script file anywhere (`diro plugin check
/// PATH`, `diro plugin install PATH`).
pub fn read_path(path: &Path, os: Os) -> Option<Installed> {
    if path.is_dir() {
        read_plugin(path, os).map(|p| Installed::Plugin(Box::new(p)))
    } else {
        read_script(path).map(|s| Installed::Script(Box::new(s)))
    }
}

/// Everything in the plugins folder, plugins and scripts, by id.
pub fn scan(plugins_dir: &Path, os: Os) -> Vec<Installed> {
    let mut found = Vec::new();
    if let Ok(items) = fs::read_dir(plugins_dir) {
        for path in items.flatten().map(|i| i.path()).filter(|p| p.is_dir()) {
            if let Some(plugin) = read_plugin(&path, os) {
                found.push(Installed::Plugin(Box::new(plugin)));
            }
        }
    }
    if let Ok(items) = fs::read_dir(scripts_dir(plugins_dir)) {
        for path in items.flatten().map(|i| i.path()) {
            if is_script_file(&path)
                && let Some(script) = read_script(&path)
            {
                found.push(Installed::Script(Box::new(script)));
            }
        }
    }
    found.sort_by(|a, b| a.id().cmp(b.id()));
    found
}

/// A plugin or script by id, or by its name in any case.
pub fn find(plugins_dir: &Path, name: &str, os: Os) -> Option<Installed> {
    let all = scan(plugins_dir, os);
    if let Some(index) = all.iter().position(|i| i.id() == name) {
        return all.into_iter().nth(index);
    }
    let lower = name.to_lowercase();
    all.into_iter().find(|i| i.name().to_lowercase() == lower)
}

/// Every problem `diro plugin check` finds in a plugin or a script, errors
/// first.
pub fn check(installed: &Installed, lookup: &Lookup) -> Vec<Problem> {
    let mut problems = match installed {
        Installed::Plugin(plugin) => check_plugin(plugin),
        Installed::Script(script) => check_script(script, lookup),
    };
    problems.sort_by_key(|p| p.severity);
    problems
}

fn problem(severity: Severity, field: &str, message: &str, fix: &str) -> Problem {
    Problem {
        severity,
        field: field.to_owned(),
        message: message.to_owned(),
        fix: fix.to_owned(),
    }
}

fn check_plugin(plugin: &PluginEntry) -> Vec<Problem> {
    let mut problems = plugin.report.problems.clone();
    let Some(manifest) = &plugin.report.manifest else {
        return problems;
    };
    let settings: BTreeSet<String> = manifest.settings.iter().map(|s| s.key.clone()).collect();
    for (index, spec) in manifest.commands.iter().enumerate() {
        if let Some((_, recipe_problems)) =
            ScriptCommand::from_manifest(manifest, index, spec, &plugin.folder)
        {
            problems.extend(recipe_problems);
            for name in unknown_names(&spec.argv, &settings) {
                problems.push(problem(
                    Severity::Error,
                    &format!("command[{index}].argv"),
                    &format!("{{{name}}} is not a placeholder"),
                    &format!("Use one of: {}, or setting.KEY.", KNOWN.join(", ")),
                ));
            }
        }
    }
    for (index, previewer) in manifest.previewers.iter().enumerate() {
        for name in unknown_names(&previewer.argv, &settings) {
            if name != "input" {
                problems.push(problem(
                    Severity::Error,
                    &format!("previewer[{index}].argv"),
                    &format!("{{{name}}} is not a placeholder"),
                    "A previewer's argv uses {input}.",
                ));
            }
        }
    }
    for (index, converter) in manifest.converters.iter().enumerate() {
        for name in unknown_names(&converter.argv, &settings) {
            problems.push(problem(
                Severity::Error,
                &format!("converter[{index}].argv"),
                &format!("{{{name}}} is not a placeholder"),
                "A converter's argv uses {input} and {output}.",
            ));
        }
    }
    if manifest.id != plugin.id
        || plugin
            .folder
            .file_name()
            .is_some_and(|n| n.to_string_lossy() != manifest.id)
    {
        problems.push(problem(
            Severity::Warning,
            "plugin.id",
            &format!("\"{}\" is not the folder's name", manifest.id),
            "Name the folder after the id.",
        ));
    }
    problems
}

fn check_script(script: &ScriptEntry, lookup: &Lookup) -> Vec<Problem> {
    let mut problems = script.header.problems.clone();
    if let Some(why) = plain_name_problem(&script.id) {
        problems.push(problem(
            Severity::Error,
            "file name",
            &why,
            "Rename it with lowercase letters, digits, - and _.",
        ));
    }
    if let Some(why) = trust_problem(&script.path) {
        problems.push(problem(
            Severity::Error,
            "file",
            &why,
            "Make the file and its folder writable by you alone.",
        ));
    }
    let header = &script.header.header;
    if header.exists_on(lookup.os)
        && ScriptKind::of(&script.path) != ScriptKind::Direct
        && let Err(missing) = lookup.choose(&script.path, &header.recipe.interpreter)
    {
        problems.push(problem(
            Severity::Warning,
            "file",
            "no program runs it on this computer",
            &missing.sentence(),
        ));
    }
    problems
}

/// The summary line of `diro plugin check`: "Checked 1 plugin: no
/// problems." or "Checked 3 plugins: 2 errors, 1 warning."
pub fn summary(checked: usize, problems: &[Problem]) -> String {
    let errors = problems
        .iter()
        .filter(|p| p.severity == Severity::Error)
        .count();
    let warnings = problems.len() - errors;
    let things = if checked == 1 {
        "1 plugin".to_owned()
    } else {
        format!("{checked} plugins")
    };
    let count = |n: usize, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    match (errors, warnings) {
        (0, 0) => format!("Checked {things}: no problems."),
        (0, w) => format!("Checked {things}: {}.", count(w, "warning")),
        (e, 0) => format!("Checked {things}: {}.", count(e, "error")),
        (e, w) => format!(
            "Checked {things}: {}, {}.",
            count(e, "error"),
            count(w, "warning")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct NoProbe;
    impl crate::script::interpreter::Probe for NoProbe {
        fn version(&self, _: &Path, _: &[&str]) -> crate::script::interpreter::Probed {
            crate::script::interpreter::Probed {
                code: None,
                text: String::new(),
            }
        }
    }

    fn empty_lookup() -> Lookup {
        Lookup::new(Os::Linux, None, Vec::new(), Arc::new(NoProbe))
    }

    #[test]
    fn plain_names_and_their_refusals() {
        assert_eq!(plain_name_problem("total-size"), None);
        assert_eq!(plain_name_problem("line_counter2"), None);
        assert_eq!(
            plain_name_problem("total size").as_deref(),
            Some("U+0020 is not allowed in a name")
        );
        assert_eq!(
            plain_name_problem("Total").as_deref(),
            Some("U+0054 is an uppercase letter; use lowercase")
        );
        assert_eq!(
            plain_name_problem("a\u{f03a}b").as_deref(),
            Some("U+F03A is not allowed in a name")
        );
        assert_eq!(
            plain_name_problem("a.b").as_deref(),
            Some("U+002E is not allowed in a name")
        );
        assert!(plain_name_problem("x").is_some());
        assert!(plain_name_problem("-x").is_some());
    }

    #[test]
    fn plugins_and_scripts_are_found_by_id_or_name() {
        let root = tempfile::tempdir().unwrap();
        let plugins = root.path();
        fs::create_dir_all(plugins.join("line-counter")).unwrap();
        fs::write(
            plugins.join("line-counter").join("plugin.toml"),
            "[plugin]\nid = \"line-counter\"\nname = \"Line counter\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[run]\ncommand = [\"python3\", \"main.py\"]\n",
        )
        .unwrap();
        fs::create_dir_all(scripts_dir(plugins)).unwrap();
        fs::write(
            scripts_dir(plugins).join("total-size.sh"),
            "# diro: name = \"Total size of selection\"\n",
        )
        .unwrap();
        fs::write(scripts_dir(plugins).join("notes.txt"), "not a script").unwrap();
        let all = scan(plugins, Os::Linux);
        let ids: Vec<&str> = all.iter().map(Installed::id).collect();
        assert_eq!(ids, ["line-counter", "total-size"]);
        assert_eq!(all[0].kind_words(), "plugin 0.1.0");
        assert_eq!(all[1].kind_words(), "script");
        assert_eq!(
            find(plugins, "Total size of selection", Os::Linux)
                .unwrap()
                .id(),
            "total-size"
        );
        assert_eq!(
            find(plugins, "line-counter", Os::Linux).unwrap().name(),
            "Line counter"
        );
        assert!(find(plugins, "nothing", Os::Linux).is_none());
    }

    #[test]
    fn check_names_each_problem_and_its_fix() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("odd");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("plugin.toml"),
            "[plugin]\nid = \"odd-one\"\nname = \"Odd\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[[command]]\nid = \"go\"\nname = \"Go\"\nargv = [\"tool\", \"{wat}\"]\nunattended = \"ok\"\n",
        )
        .unwrap();
        let installed = read_path(&folder, Os::Linux).unwrap();
        let problems = check(&installed, &empty_lookup());
        let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
        assert!(
            text.iter()
                .any(|t| t.starts_with("Error, command[0].timeout: is not set")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.starts_with("Error, command[0].argv: {wat} is not a placeholder")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t.starts_with("Warning, plugin.id")),
            "{text:?}"
        );
        assert_eq!(
            summary(1, &problems),
            "Checked 1 plugin: 2 errors, 1 warning."
        );
        assert_eq!(summary(1, &[]), "Checked 1 plugin: no problems.");
    }

    #[test]
    fn a_script_with_an_odd_file_name_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("TotalSize.sh");
        fs::write(&path, "# diro: name = \"T\"\n").unwrap();
        let installed = read_path(&path, Os::Linux).unwrap();
        let problems = check(&installed, &empty_lookup());
        assert_eq!(
            problems[0].to_string(),
            "Error, file name: U+0054 is an uppercase letter; use lowercase. Rename it with lowercase letters, digits, - and _."
        );
        let warning = problems
            .iter()
            .find(|p| p.severity == Severity::Warning)
            .unwrap();
        assert_eq!(warning.message, "no program runs it on this computer");
    }
}
