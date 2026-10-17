//! `cargo xtask deps --check`: the dependency rules of plan section 5.1.
//!
//! 1. Every member's dependencies (normal, dev, build, and per target) use
//!    `workspace = true`, so each crate is declared once, in the root.
//! 2. Every internal dependency points the allowed way, by [`ALLOWED`].
//! 3. Every member is in [`ALLOWED`]: a new crate is added only when a
//!    wave's brief says so, here and in docs/dev/architecture.md.
//! 4. Nothing depends on `xtask` or on the binary crate `diro`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use toml::{Table, Value};

/// Each crate and the workspace crates it may depend on. Keep in step with
/// docs/dev/architecture.md.
pub const ALLOWED: [(&str, &[&str]); 17] = [
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
            "diroxide-config",
            "diroxide-app",
            "diroxide-theme",
        ],
    ),
];

const SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

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
    let mut problems = Vec::new();
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
            "deps: Pass. {checked} crates checked; every dependency is inherited and points the allowed way."
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
    fn this_workspace_passes() {
        assert!(check(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap());
    }
}
