//! `cargo xtask deps --check`: the dependency rules of plan section 5.1.
//!
//! 1. Every member's dependencies (normal, dev, build, and per target) use
//!    `workspace = true`, so each crate is declared once, in the root.
//! 2. Every internal dependency points the allowed way, by [`ALLOWED`].
//! 3. Every member is in [`ALLOWED`]: a new crate is added only when a
//!    wave's brief says so, here and in docs/dev/architecture.md.
//! 4. Nothing depends on `xtask` or on the binary crate `diro`.
//! 5. The vendored GUI stack (docs/dev/vendoring.md) stays apart: every
//!    path in the root's `[workspace.dependencies]` and `[patch.crates-io]`
//!    is a member's folder or lies in a [`VENDORED`] folder the root's
//!    `exclude` lists, and only the crates in [`VENDORED_USERS`] depend on a
//!    vendored crate.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use toml::{Table, Value};

/// Each crate and the workspace crates it may depend on. Keep in step with
/// docs/dev/architecture.md.
pub const ALLOWED: [(&str, &[&str]); 19] = [
    ("diroxide-core", &[]),
    ("diroxide-theme", &[]),
    ("diroxide-vfs", &["diroxide-core"]),
    ("diroxide-config", &["diroxide-core", "diroxide-theme"]),
    ("diroxide-ops", &["diroxide-core", "diroxide-vfs"]),
    (
        "diroxide-archive",
        &["diroxide-core", "diroxide-vfs", "diroxide-ops"],
    ),
    ("diroxide-search", &["diroxide-core", "diroxide-vfs"]),
    (
        "diroxide-tools",
        &["diroxide-core", "diroxide-vfs", "diroxide-config"],
    ),
    (
        "diroxide-filetype",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-config",
            "diroxide-tools",
        ],
    ),
    (
        "diroxide-preview",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-tools",
            "diroxide-filetype",
        ],
    ),
    (
        "diroxide-diff",
        &["diroxide-core", "diroxide-vfs", "diroxide-preview"],
    ),
    // Optional components (W4c, plan section 5.14): the archive crate
    // unpacks a component, the tools registry finds curl and wget.
    (
        "diroxide-components",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-config",
            "diroxide-tools",
            "diroxide-archive",
        ],
    ),
    (
        "diroxide-plugins",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-config",
            "diroxide-tools",
            "diroxide-ops",
        ],
    ),
    (
        "diroxide-app",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-archive",
            "diroxide-ops",
            "diroxide-search",
            "diroxide-filetype",
            "diroxide-config",
            "diroxide-plugins",
            "diroxide-tools",
            "diroxide-preview",
            "diroxide-theme",
            "diroxide-diff",
            "diroxide-components",
        ],
    ),
    (
        "diroxide-tui",
        &["diroxide-core", "diroxide-app", "diroxide-theme"],
    ),
    (
        "diroxide-gui",
        &["diroxide-core", "diroxide-app", "diroxide-theme"],
    ),
    (
        "diroxide-cli",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-archive",
            "diroxide-ops",
            "diroxide-search",
            "diroxide-filetype",
            "diroxide-config",
            "diroxide-plugins",
            "diroxide-tools",
            "diroxide-preview",
            "diroxide-app",
            "diroxide-tui",
            "diroxide-gui",
            "diroxide-theme",
            "diroxide-diff",
            "diroxide-components",
        ],
    ),
    ("diro", &["diroxide-cli"]),
    (
        "xtask",
        &[
            "diroxide-core",
            "diroxide-vfs",
            "diroxide-ops",
            "diroxide-archive",
            "diroxide-search",
            "diroxide-config",
            "diroxide-plugins",
            "diroxide-app",
            "diroxide-theme",
            "diroxide-components",
        ],
    ),
];

/// The folders of vendored code, each its own workspace, copied whole from
/// textweaver (docs/dev/vendoring.md). The fonts folder holds no crate.
pub const VENDORED: [&str; 2] = ["third_party/xilem", "third_party/accesskit_windows"];

/// The members that may depend on a vendored crate: the GUI frontend alone,
/// behind its `gui` feature.
pub const VENDORED_USERS: [&str; 1] = ["diroxide-gui"];

const SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// The problems with the vendored tree in the root manifest, in words: a
/// path outside the members' folders and the vendored ones, or a vendored
/// folder the workspace does not exclude. Returns them with the names of
/// the root's dependencies that are vendored crates.
pub fn vendored_problems(root: &Table) -> (Vec<String>, Vec<String>) {
    let mut problems = Vec::new();
    let excluded: Vec<&str> = root
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for folder in VENDORED {
        if !excluded.contains(&folder) {
            problems.push(format!(
                "the root Cargo.toml does not exclude {folder}: add it to [workspace] exclude"
            ));
        }
    }
    let in_vendored = |path: &str| {
        let path = path.replace('\\', "/");
        VENDORED
            .iter()
            .any(|folder| path == *folder || path.starts_with(&format!("{folder}/")))
    };
    let mut vendored = Vec::new();
    let declared = root
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(Value::as_table);
    for (name, spec) in declared.into_iter().flatten() {
        let Some(path) = spec.get("path").and_then(Value::as_str) else {
            continue;
        };
        if in_vendored(path) {
            vendored.push(name.clone());
        } else if !path.starts_with("crates/") {
            problems.push(format!(
                "{name} is declared with the path {path}, outside crates/ and the vendored folders"
            ));
        }
    }
    let patches = root
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(Value::as_table);
    for (name, spec) in patches.into_iter().flatten() {
        match spec.get("path").and_then(Value::as_str) {
            Some(path) if in_vendored(path) => {}
            Some(path) => problems.push(format!(
                "the patch for {name} points at {path}, outside the vendored folders"
            )),
            None => problems.push(format!(
                "the patch for {name} has no path: patches come from the vendored folders"
            )),
        }
    }
    (problems, vendored)
}

fn read_table(path: &Path) -> Result<Table> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("{} could not be read", path.display()))?;
    text.parse::<Table>()
        .with_context(|| format!("{} is not valid TOML", path.display()))
}

/// The members' manifest folders, from the root's `members` (a trailing
/// `/*` lists the folders under it).
fn member_folders(root: &Path, manifest: &Table) -> Result<Vec<std::path::PathBuf>> {
    let members = manifest
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(Value::as_array)
        .context("the root Cargo.toml has no workspace members")?;
    let mut folders = Vec::new();
    for member in members.iter().filter_map(Value::as_str) {
        if let Some(parent) = member.strip_suffix("/*") {
            let mut found: Vec<_> = fs::read_dir(root.join(parent))
                .with_context(|| format!("{parent} could not be listed"))?
                .filter_map(|item| item.ok().map(|i| i.path()))
                .filter(|path| path.join("Cargo.toml").is_file())
                .collect();
            found.sort();
            folders.extend(found);
        } else {
            folders.push(root.join(member));
        }
    }
    Ok(folders)
}

/// Every dependency table of a manifest, with a label for messages.
fn dependency_tables(manifest: &Table) -> Vec<(String, &Table)> {
    let mut tables = Vec::new();
    for section in SECTIONS {
        if let Some(table) = manifest.get(section).and_then(Value::as_table) {
            tables.push((section.to_owned(), table));
        }
    }
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for (target, value) in targets {
            for section in SECTIONS {
                if let Some(table) = value.get(section).and_then(Value::as_table) {
                    tables.push((format!("target {target} {section}"), table));
                }
            }
        }
    }
    tables
}

/// Runs the check, printing each problem on its own line. Returns whether
/// everything passed.
pub fn check(root: &Path) -> Result<bool> {
    let manifest = read_table(&root.join("Cargo.toml"))?;
    let folders = member_folders(root, &manifest)?;
    let (mut problems, vendored) = vendored_problems(&manifest);
    let mut checked = 0;
    for folder in &folders {
        let member = read_table(&folder.join("Cargo.toml"))?;
        let name = member
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .with_context(|| format!("{} has no package name", folder.display()))?;
        checked += 1;
        let Some((_, allowed)) = ALLOWED.iter().find(|(crate_name, _)| *crate_name == name) else {
            problems.push(format!(
                "{name} is not in the allowed table: add it to xtask/src/deps.rs and docs/dev/architecture.md"
            ));
            continue;
        };
        for (section, table) in dependency_tables(&member) {
            for (dependency, spec) in table {
                let inherited = spec
                    .as_table()
                    .and_then(|t| t.get("workspace"))
                    .and_then(Value::as_bool)
                    == Some(true);
                if !inherited {
                    problems.push(format!(
                        "{name} declares {dependency} in [{section}] without workspace = true"
                    ));
                }
                if vendored.contains(dependency) && !VENDORED_USERS.contains(&name) {
                    problems.push(format!(
                        "{name} depends on the vendored {dependency} in [{section}]; only {} may",
                        VENDORED_USERS.join(", ")
                    ));
                }
                if dependency == "xtask" || dependency == "diro" {
                    problems.push(format!(
                        "{name} depends on {dependency}, which nothing may depend on"
                    ));
                    continue;
                }
                let internal = ALLOWED
                    .iter()
                    .any(|(crate_name, _)| *crate_name == dependency);
                if internal && !allowed.contains(&dependency.as_str()) {
                    problems.push(format!(
                        "{name} depends on {dependency} in [{section}], which points the wrong way"
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        println!(
            "deps: Pass. {checked} crates checked; every dependency is inherited and points the allowed way; the {} vendored crates stay with {}.",
            vendored.len(),
            VENDORED_USERS.join(", ")
        );
        Ok(true)
    } else {
        println!("deps: Fail. {} problems:", problems.len());
        for problem in &problems {
            println!("{problem}");
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_no_cycles_and_names_itself_once() {
        for (name, allowed) in ALLOWED {
            assert!(!allowed.contains(&name), "{name} allows itself");
            assert_eq!(
                ALLOWED.iter().filter(|(n, _)| *n == name).count(),
                1,
                "{name}"
            );
            for dependency in allowed {
                let (_, theirs) = ALLOWED.iter().find(|(n, _)| n == dependency).unwrap();
                assert!(
                    !theirs.contains(&name),
                    "{name} and {dependency} allow each other"
                );
            }
        }
    }

    #[test]
    fn the_vendored_tree_is_excluded_patched_from_and_used_by_the_gui_alone() {
        let good: Table = r#"
            [workspace]
            members = ["crates/*"]
            exclude = ["plugins", "third_party/xilem", "third_party/accesskit_windows"]
            [workspace.dependencies]
            diroxide-core = { path = "crates/diroxide-core" }
            masonry = { path = "third_party/xilem/masonry", default-features = false }
            serde = "1"
            [patch.crates-io]
            parley = { path = "third_party/xilem/parley" }
            accesskit_windows = { path = "third_party/accesskit_windows" }
        "#
        .parse()
        .unwrap();
        let (problems, vendored) = vendored_problems(&good);
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(vendored, ["masonry"]);
        let bad: Table = r#"
            [workspace]
            exclude = ["third_party/xilem"]
            [workspace.dependencies]
            stray = { path = "../elsewhere" }
            [patch.crates-io]
            parley = { path = "vendor/parley" }
            winit = "0.30"
        "#
        .parse()
        .unwrap();
        let (problems, _) = vendored_problems(&bad);
        assert_eq!(
            problems,
            [
                "the root Cargo.toml does not exclude third_party/accesskit_windows: add it to [workspace] exclude",
                "stray is declared with the path ../elsewhere, outside crates/ and the vendored folders",
                "the patch for parley points at vendor/parley, outside the vendored folders",
                "the patch for winit has no path: patches come from the vendored folders",
            ]
        );
    }

    #[test]
    fn this_workspace_passes() {
        assert!(check(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap());
    }
}
