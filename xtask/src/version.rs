//! `cargo xtask version --check`: before a tag, every place that carries
//! the program's version says the version in `[workspace.package]` of the
//! root manifest.
//!
//! The places are the set the alpha 4 release commit changed by hand
//! (`history/alpha4-readiness.md` in the planning repository, step 4):
//!
//! - the root manifest's path dependencies on the workspace's own crates,
//!   any member crate that writes a version or a path dependency of its
//!   own, and the root lock's entries for those crates;
//! - each plugin workspace under `plugins/`: its manifest's version and
//!   path dependencies, its lock's entries for diroxide's crates, and its
//!   `plugin.toml` (`version` and `min_host`);
//! - the components definitions and the generated components list: the
//!   list's `diro` line, and each plugin component's version, `min_diro`
//!   and asset names;
//! - the README's status line, and the guides that quote the release in
//!   their sample lines ([`DOC_PAGES`]);
//! - the changelog's newest released section.
//!
//! Places that lag on purpose are named in [`LAGGING`] with their reason,
//! and are not compared. The check prints one line, meaning first:
//! "version: Pass. ..." or "version: Fail. ..." with one line per place
//! that disagrees.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use toml::{Table, Value};

/// The guides whose sample lines quote the release: every pre-release
/// version written in them (such as `0.1.0-alpha.4`) must be the
/// workspace's, and each must quote it at least once.
pub const DOC_PAGES: [&str; 5] = [
    "docs/install.md",
    "docs/components.md",
    "docs/remote-places.md",
    "docs/writing-a-plugin.md",
    "docs/hooks.md",
];

/// The places that legitimately keep another version, each with its
/// reason in words. None of them is compared.
pub const LAGGING: [(&str, &str); 3] = [
    (
        "the changelog's Unreleased section",
        "it names the next alpha before the release commit gives it a version heading",
    ),
    (
        "the examples pack, diro-examples",
        "a data component with its own version (0.1.1) and minimum host; any newer host \
         satisfies it, so a release does not bump it (answer 125)",
    ),
    (
        "the helper components",
        "each carries the version of the program it pins, not diroxide's",
    ),
];

/// The outcome of one inspection: the version, how many places were
/// compared, and one line per place that disagrees.
#[derive(Debug, Default)]
pub struct Report {
    /// The version in `[workspace.package]`.
    pub version: String,
    /// How many places were compared with it.
    pub checked: usize,
    /// One line per disagreeing place: the file, the key, what it says.
    pub problems: Vec<String>,
}

impl Report {
    fn compare(&mut self, place: &str, key: &str, found: &str) {
        self.checked += 1;
        if found != self.version {
            self.problems.push(format!("{place}, {key}: says {found}."));
        }
    }

    fn problem(&mut self, line: String) {
        self.checked += 1;
        self.problems.push(line);
    }
}

/// Runs the check on the workspace at `root` and prints its line.
pub fn check(root: &Path) -> Result<bool> {
    let report = inspect(root)?;
    for line in lines(&report) {
        println!("{line}");
    }
    Ok(report.problems.is_empty())
}

/// The lines the check prints: the verdict first, then one line per place
/// that disagrees.
pub fn lines(report: &Report) -> Vec<String> {
    if report.problems.is_empty() {
        return vec![format!(
            "version: Pass. {} places say {}; {} keep their own on purpose: {}.",
            report.checked,
            report.version,
            LAGGING.len(),
            LAGGING
                .iter()
                .map(|(place, _)| *place)
                .collect::<Vec<_>>()
                .join(", ")
        )];
    }
    let count = report.problems.len();
    let (noun, verb) = if count == 1 {
        ("place", "disagrees")
    } else {
        ("places", "disagree")
    };
    let mut out = vec![format!(
        "version: Fail. {count} {noun} {verb} with {}:",
        report.version
    )];
    out.extend(report.problems.iter().map(|problem| format!("- {problem}")));
    out
}

/// Reads every place under `root` and compares it with the workspace's
/// version.
pub fn inspect(root: &Path) -> Result<Report> {
    let manifest = read_table(root, "Cargo.toml")?;
    let Some(version) = manifest
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(Value::as_str)
    else {
        bail!("the root manifest has no version in [workspace.package]")
    };
    let mut report = Report {
        version: version.to_string(),
        ..Report::default()
    };

    // The root manifest's path dependencies on the workspace's crates.
    if let Some(deps) = manifest
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(Value::as_table)
    {
        path_dependencies(&mut report, "Cargo.toml", deps);
    }

    // Member crates that write a version or a path dependency themselves.
    for member in member_manifests(root)? {
        let table = read_table(root, &member)?;
        crate_manifest(&mut report, &member, &table);
    }
    lock(&mut report, root, "Cargo.lock")?;

    // The plugin workspaces.
    let plugins = plugin_folders(root)?;
    for name in &plugins {
        let base = format!("plugins/{name}");
        let cargo = format!("{base}/Cargo.toml");
        let table = read_table(root, &cargo)?;
        crate_manifest(&mut report, &cargo, &table);
        lock(&mut report, root, &format!("{base}/Cargo.lock"))?;
        let plugin_toml = format!("{base}/plugin.toml");
        if root.join(&plugin_toml).is_file() {
            let table = read_table(root, &plugin_toml)?;
            let plugin = table.get("plugin").and_then(Value::as_table);
            for key in ["version", "min_host"] {
                match plugin.and_then(|p| p.get(key)).and_then(Value::as_str) {
                    Some(found) => report.compare(&plugin_toml, key, found),
                    None => report.problem(format!("{plugin_toml}, {key}: missing.")),
                }
            }
        }
    }

    // The components definitions and the generated list.
    for file in [crate::components::DEFINITIONS, crate::components::MANIFEST] {
        if !root.join(file).is_file() {
            continue;
        }
        let table = read_table(root, file)?;
        if file == crate::components::MANIFEST {
            match table.get("diro").and_then(Value::as_str) {
                Some(found) => report.compare(file, "diro", found),
                None => report.problem(format!("{file}, diro: missing.")),
            }
        }
        components(&mut report, file, &table, &plugins);
    }

    // The README's status line.
    readme(&mut report, root)?;

    // The guides' sample lines.
    for page in DOC_PAGES {
        let path = root.join(page);
        if !path.is_file() {
            report.problem(format!("{page}: missing."));
            continue;
        }
        let text = read(root, page)?;
        let found = prerelease_versions(&text);
        if found.is_empty() {
            report.problem(format!("{page}: names no version."));
        }
        for (line, token) in found {
            report.compare(page, &format!("line {line}"), &token);
        }
    }

    changelog(&mut report, root)?;
    Ok(report)
}

/// Reads `relative` under `root` as text.
fn read(root: &Path, relative: &str) -> Result<String> {
    fs::read_to_string(root.join(relative)).with_context(|| format!("reading {relative}"))
}

/// Reads `relative` under `root` as a TOML table.
fn read_table(root: &Path, relative: &str) -> Result<Table> {
    read(root, relative)?
        .parse::<Table>()
        .with_context(|| format!("parsing {relative}"))
}

/// Whether a dependency or lock name is one of diroxide's own crates or
/// plugins: `diro`, `diroxide-*`, `diro-*`, or the xtask crate.
fn is_ours(name: &str) -> bool {
    name.starts_with("diro") || name == "xtask"
}

/// Compares each path dependency on one of diroxide's crates that writes a
/// version.
fn path_dependencies(report: &mut Report, place: &str, deps: &Table) {
    for (name, spec) in deps {
        let Some(spec) = spec.as_table() else {
            continue;
        };
        if !is_ours(name) || !spec.contains_key("path") {
            continue;
        }
        if let Some(found) = spec.get("version").and_then(Value::as_str) {
            report.compare(place, &format!("{name}'s version"), found);
        }
    }
}

/// A crate manifest: its package version when it writes one (not
/// `version.workspace = true`), and its path dependencies in every
/// dependency table, target-specific ones included.
fn crate_manifest(report: &mut Report, place: &str, table: &Table) {
    if let Some(found) = table
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(Value::as_str)
    {
        report.compare(place, "package version", found);
    }
    let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
    for kind in kinds {
        if let Some(deps) = table.get(kind).and_then(Value::as_table) {
            path_dependencies(report, place, deps);
        }
    }
    if let Some(targets) = table.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            for kind in kinds {
                if let Some(deps) = target.get(kind).and_then(Value::as_table) {
                    path_dependencies(report, place, deps);
                }
            }
        }
    }
}

/// A lock file: each local package (no `source`) of diroxide's own.
fn lock(report: &mut Report, root: &Path, relative: &str) -> Result<()> {
    if !root.join(relative).is_file() {
        return Ok(());
    }
    let table = read_table(root, relative)?;
    let Some(packages) = table.get("package").and_then(Value::as_array) else {
        return Ok(());
    };
    for package in packages {
        let Some(package) = package.as_table() else {
            continue;
        };
        if package.contains_key("source") {
            continue;
        }
        let name = package.get("name").and_then(Value::as_str).unwrap_or("");
        if !is_ours(name) {
            continue;
        }
        if let Some(found) = package.get("version").and_then(Value::as_str) {
            report.compare(relative, &format!("{name}'s entry"), found);
        }
    }
    Ok(())
}

/// The member manifests: `crates/*/Cargo.toml` and `xtask/Cargo.toml`, in
/// name order.
fn member_manifests(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let crates = root.join("crates");
    if crates.is_dir() {
        for name in sorted_folders(&crates)? {
            if crates.join(&name).join("Cargo.toml").is_file() {
                out.push(format!("crates/{name}/Cargo.toml"));
            }
        }
    }
    if root.join("xtask/Cargo.toml").is_file() {
        out.push("xtask/Cargo.toml".to_string());
    }
    Ok(out)
}

/// The plugin workspaces: each folder under `plugins/` with a manifest,
/// in name order.
fn plugin_folders(root: &Path) -> Result<Vec<String>> {
    let plugins = root.join("plugins");
    if !plugins.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for name in sorted_folders(&plugins)? {
        if plugins.join(&name).join("Cargo.toml").is_file() {
            out.push(name);
        }
    }
    Ok(out)
}

/// The names of the folders in `folder`, sorted.
fn sorted_folders(folder: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(folder).with_context(|| format!("listing {}", folder.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(name) = entry.file_name().to_str()
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// A components file: each component whose id is a plugin folder's name
/// carries the version, as its version, its `min_diro` and in every
/// asset's name. The examples pack and the helpers are in [`LAGGING`].
fn components(report: &mut Report, place: &str, table: &Table, plugins: &[String]) {
    let Some(list) = table.get("component").and_then(Value::as_array) else {
        return;
    };
    for component in list {
        let Some(id) = component.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !plugins.iter().any(|p| p == id) {
            continue;
        }
        for key in ["version", "min_diro"] {
            match component.get(key).and_then(Value::as_str) {
                Some(found) => report.compare(place, &format!("{id}'s {key}"), found),
                None => report.problem(format!("{place}, {id}'s {key}: missing.")),
            }
        }
        let want = format!("{id}-{}-", report.version);
        let files = component.get("file").and_then(Value::as_array);
        for file in files.into_iter().flatten() {
            let Some(name) = file.get("name").and_then(Value::as_str) else {
                continue;
            };
            report.checked += 1;
            if !name.starts_with(&want) {
                report
                    .problems
                    .push(format!("{place}, asset {name}: not named {want}..."));
            }
        }
    }
}

/// The README's status line: every version on it is the workspace's, and
/// for a pre-release such as `0.1.0-alpha.4` it says "alpha 4".
fn readme(report: &mut Report, root: &Path) -> Result<()> {
    let place = "README.md";
    if !root.join(place).is_file() {
        report.problem(format!("{place}: missing."));
        return Ok(());
    }
    let text = read(root, place)?;
    let Some((number, line)) = text
        .lines()
        .enumerate()
        .find(|(_, line)| line.starts_with("Status:"))
    else {
        report.problem(format!("{place}: no status line."));
        return Ok(());
    };
    let key = format!("status line {}", number + 1);
    let found = prerelease_versions(line);
    if found.is_empty() {
        report.problem(format!("{place}, {key}: names no version."));
    }
    for (_, token) in found {
        report.compare(place, &key, &token);
    }
    if let Some(words) = release_words(&report.version) {
        report.checked += 1;
        if !line.contains(&words) {
            report
                .problems
                .push(format!("{place}, {key}: does not say {words}."));
        }
    }
    Ok(())
}

/// The changelog's newest released section is the workspace's version.
/// An `## Unreleased` heading above it is in [`LAGGING`].
fn changelog(report: &mut Report, root: &Path) -> Result<()> {
    let place = "CHANGELOG.md";
    if !root.join(place).is_file() {
        report.problem(format!("{place}: missing."));
        return Ok(());
    }
    let text = read(root, place)?;
    let newest = text
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(str::trim)
        .find(|heading| !heading.starts_with("Unreleased"));
    match newest {
        Some(heading) => report.compare(place, "newest released section", heading),
        None => report.problem(format!("{place}: no released section.")),
    }
    Ok(())
}

/// "alpha 4" for `0.1.0-alpha.4`: the pre-release's words as the README
/// says them, or `None` for a version with no pre-release.
fn release_words(version: &str) -> Option<String> {
    let (_, pre) = version.split_once('-')?;
    let (word, number) = pre.split_once('.')?;
    Some(format!("{word} {number}"))
}

/// Every pre-release version written in `text`, such as `0.1.0-alpha.4`,
/// with its line number (from 1): three numbers, a hyphen, a word and a
/// number. A plain `0.1.1` is not one.
pub fn prerelease_versions(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            let boundary =
                start == 0 || !(bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.');
            if boundary
                && bytes[start].is_ascii_digit()
                && let Some(end) = prerelease_at(bytes, start)
            {
                out.push((index + 1, line[start..end].to_string()));
                start = end;
                continue;
            }
            start += 1;
        }
    }
    out
}

/// The end of a pre-release version starting at `start`, if one does.
fn prerelease_at(bytes: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    for part in 0..3 {
        let digits = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        at += digits;
        let separator = if part < 2 { b'.' } else { b'-' };
        if bytes.get(at) != Some(&separator) {
            return None;
        }
        at += 1;
    }
    let letters = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    if letters == 0 {
        return None;
    }
    at += letters;
    if bytes.get(at) != Some(&b'.') {
        return None;
    }
    at += 1;
    let digits = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    Some(at + digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "0.1.0-alpha.4";

    /// Writes `text` at `relative` under `root`, making its folders.
    fn put(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A small tree in which every place says `V`.
    fn agreeing_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(
            root,
            "Cargo.toml",
            &format!(
                "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"{V}\"\n\n\
                 [workspace.dependencies]\ndiroxide-core = {{ path = \"crates/diroxide-core\", version = \"{V}\" }}\n\
                 serde = {{ version = \"1.0.228\" }}\nmasonry = {{ path = \"third_party/xilem/masonry\", version = \"0.4.0\" }}\n"
            ),
        );
        put(
            root,
            "crates/diroxide-core/Cargo.toml",
            "[package]\nname = \"diroxide-core\"\nversion.workspace = true\n",
        );
        put(
            root,
            "Cargo.lock",
            &format!(
                "version = 4\n\n[[package]]\nname = \"diroxide-core\"\nversion = \"{V}\"\n\n\
                 [[package]]\nname = \"masonry\"\nversion = \"0.4.0\"\n\n\
                 [[package]]\nname = \"serde\"\nversion = \"1.0.228\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            ),
        );
        put(
            root,
            "plugins/diro-git/Cargo.toml",
            &format!(
                "[workspace]\n\n[package]\nname = \"diro-git\"\nversion = \"{V}\"\n\n\
                 [dependencies]\ndiroxide-core = {{ path = \"../../crates/diroxide-core\", version = \"{V}\" }}\n"
            ),
        );
        put(
            root,
            "plugins/diro-git/Cargo.lock",
            &format!(
                "version = 4\n\n[[package]]\nname = \"diro-git\"\nversion = \"{V}\"\n\n\
                 [[package]]\nname = \"diroxide-core\"\nversion = \"{V}\"\n"
            ),
        );
        put(
            root,
            "plugins/diro-git/plugin.toml",
            &format!("[plugin]\nid = \"git\"\nversion = \"{V}\"\nmin_host = \"{V}\"\n"),
        );
        put(
            root,
            crate::components::DEFINITIONS,
            &format!(
                "format = 1\n\n[[component]]\nid = \"diro-examples\"\nversion = \"0.1.1\"\nmin_diro = \"0.1.0-alpha.3\"\n\n\
                 [[component.file]]\nname = \"diro-examples-0.1.1.zip\"\n\n\
                 [[component]]\nid = \"diro-git\"\nversion = \"{V}\"\nmin_diro = \"{V}\"\n\n\
                 [[component.file]]\nname = \"diro-git-{V}-x86_64-pc-windows-msvc.zip\"\n\n\
                 [[component]]\nid = \"sevenzip\"\nkind = \"helper\"\nversion = \"25.01\"\n"
            ),
        );
        put(
            root,
            crate::components::MANIFEST,
            &format!(
                "format = 1\ndiro = \"{V}\"\n\n[[component]]\nid = \"diro-examples\"\nversion = \"0.1.1\"\nmin_diro = \"0.1.0-alpha.3\"\n"
            ),
        );
        put(
            root,
            "README.md",
            &format!(
                "# diroxide\n\nStatus: alpha. This is alpha 4, version {V}: a file manager.\n"
            ),
        );
        for page in DOC_PAGES {
            put(
                root,
                page,
                &format!(
                    "# A guide\n\nThe first line is `diro {V}`.\nThe pack is diro-examples-0.1.1.\n"
                ),
            );
        }
        put(
            root,
            "CHANGELOG.md",
            &format!(
                "# Changelog\n\n## Unreleased: alpha 5\n\nSoon.\n\n## {V}\n\nDone.\n\n## 0.1.0-alpha.3\n"
            ),
        );
        dir
    }

    #[test]
    fn a_tree_that_agrees_passes_and_lagging_places_are_not_compared() {
        let dir = agreeing_tree();
        let report = inspect(dir.path()).unwrap();
        assert_eq!(report.version, V);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        // Root dep 1, lock 1, plugin manifest 2, plugin lock 2, plugin.toml
        // 2, definitions 3, list 1, README 2, five guides 5, changelog 1.
        assert_eq!(report.checked, 20);
    }

    #[test]
    fn a_plugin_toml_left_behind_is_named_by_file_and_key() {
        let dir = agreeing_tree();
        put(
            dir.path(),
            "plugins/diro-git/plugin.toml",
            &format!("[plugin]\nid = \"git\"\nversion = \"{V}\"\nmin_host = \"0.1.0-alpha.3\"\n"),
        );
        let report = inspect(dir.path()).unwrap();
        assert_eq!(
            report.problems,
            ["plugins/diro-git/plugin.toml, min_host: says 0.1.0-alpha.3."]
        );
    }

    #[test]
    fn a_path_dependency_and_a_lock_entry_left_behind_fail() {
        let dir = agreeing_tree();
        let root = dir.path();
        let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let stale = format!("crates/diroxide-core\", version = \"{V}\"");
        put(
            root,
            "Cargo.toml",
            &manifest.replace(
                &stale,
                "crates/diroxide-core\", version = \"0.1.0-alpha.3\"",
            ),
        );
        let lock = fs::read_to_string(root.join("plugins/diro-git/Cargo.lock")).unwrap();
        put(
            root,
            "plugins/diro-git/Cargo.lock",
            &lock.replacen(
                &format!("version = \"{V}\""),
                "version = \"0.1.0-alpha.3\"",
                1,
            ),
        );
        let report = inspect(root).unwrap();
        assert_eq!(
            report.problems,
            [
                "Cargo.toml, diroxide-core's version: says 0.1.0-alpha.3.",
                "plugins/diro-git/Cargo.lock, diro-git's entry: says 0.1.0-alpha.3.",
            ]
        );
    }

    #[test]
    fn a_component_asset_left_behind_fails_and_the_examples_pack_does_not() {
        let dir = agreeing_tree();
        let root = dir.path();
        let text = fs::read_to_string(root.join(crate::components::DEFINITIONS)).unwrap();
        put(
            root,
            crate::components::DEFINITIONS,
            &text.replace(&format!("diro-git-{V}-x86"), "diro-git-0.1.0-alpha.3-x86"),
        );
        let report = inspect(root).unwrap();
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert!(report.problems[0].starts_with(
            "packaging/components/components.toml, asset diro-git-0.1.0-alpha.3-x86_64"
        ));
    }

    #[test]
    fn the_readme_and_a_guide_left_behind_fail_by_line() {
        let dir = agreeing_tree();
        let root = dir.path();
        put(
            root,
            "README.md",
            "# diroxide\n\nStatus: alpha. This is alpha 3, version 0.1.0-alpha.3: a file manager.\n",
        );
        put(
            root,
            "docs/install.md",
            "# Install\n\nSuch as `diro-0.1.0-alpha.3-windows-x86_64.exe`.\n",
        );
        let report = inspect(root).unwrap();
        assert_eq!(
            report.problems,
            [
                "README.md, status line 3: says 0.1.0-alpha.3.",
                "README.md, status line 3: does not say alpha 4.",
                "docs/install.md, line 3: says 0.1.0-alpha.3.",
            ]
        );
    }

    #[test]
    fn the_changelog_skips_unreleased_and_reads_the_newest_released_section() {
        let dir = agreeing_tree();
        put(
            dir.path(),
            "CHANGELOG.md",
            "# Changelog\n\n## Unreleased, to be 0.1.0-alpha.4\n\n## 0.1.0-alpha.3\n",
        );
        let report = inspect(dir.path()).unwrap();
        assert_eq!(
            report.problems,
            ["CHANGELOG.md, newest released section: says 0.1.0-alpha.3."]
        );
    }

    #[test]
    fn a_missing_workspace_version_is_an_error_not_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "Cargo.toml", "[workspace]\nmembers = []\n");
        assert!(inspect(dir.path()).is_err());
    }

    #[test]
    fn prerelease_versions_are_found_and_plain_ones_are_not() {
        let text = "diro-sftp-0.1.0-alpha.4-x86_64.zip and 0.1.1\n`diro 0.1.0-beta.12`.";
        assert_eq!(
            prerelease_versions(text),
            [
                (1, "0.1.0-alpha.4".to_string()),
                (2, "0.1.0-beta.12".to_string())
            ]
        );
        assert_eq!(release_words(V).as_deref(), Some("alpha 4"));
        assert_eq!(release_words("1.0.0"), None);
    }

    #[test]
    fn the_output_lines_put_the_verdict_first() {
        let dir = agreeing_tree();
        let report = inspect(dir.path()).unwrap();
        let pass = lines(&report);
        assert_eq!(pass.len(), 1);
        // "version: Pass. 20 places say 0.1.0-alp" is the first 40 cells.
        assert!(pass[0].starts_with("version: Pass. 20 places say 0.1.0-alpha.4; 3 keep"));
        assert!(pass[0].ends_with("the helper components."), "{}", pass[0]);
        for (place, reason) in LAGGING {
            assert!(!place.is_empty() && !reason.is_empty());
        }

        let report = Report {
            version: V.to_string(),
            checked: 20,
            problems: vec!["docs/hooks.md, line 9: says 0.1.0-alpha.3.".to_string()],
        };
        let fail = lines(&report);
        // "version: Fail. 1 place disagrees with" is 37 cells.
        let head = "version: Fail. 1 place disagrees with";
        assert_eq!(head.chars().count(), 37);
        assert_eq!(fail[0], format!("{head} {V}:"));
        assert_eq!(fail[1], "- docs/hooks.md, line 9: says 0.1.0-alpha.3.");
    }
}
