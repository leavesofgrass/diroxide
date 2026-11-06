//! The git plugin through the host's own checks: its manifest, the
//! conformance suite against the built program, and the column and the
//! title line asked of it by the real host.

// Test helpers may unwrap: a failure is the test's failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_core::Place;
use diroxide_plugins::conformance::{Group, Verdict, run};
use diroxide_plugins::host::QUIT_BUDGET;
use diroxide_plugins::{Host, Manifest, PluginInfo};

/// The built plugin program.
fn plugin_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diro-git"))
}

/// The plugin's own manifest with its program replaced by `exe`, in a
/// folder of its own under `root`.
fn plugin_folder(root: &Path, exe: &Path) -> PathBuf {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin.toml")).unwrap();
    let command = format!("['{}']", exe.display());
    let manifest = manifest
        .replace("[\"./diro-git.exe\"]", &command)
        .replace("[\"./diro-git\"]", &command);
    let folder = root.join("git");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("plugin.toml"), manifest).unwrap();
    folder
}

/// A folder for the test under the build's target folder, never the
/// system's temporary folder.
fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("git-it-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("git-")
        .tempdir_in(base)
        .unwrap()
}

#[test]
fn the_manifest_reads_and_keeps_the_40_cell_rule() {
    let text = include_str!("../../plugin.toml");
    let report = Manifest::check(text);
    assert_eq!(report.text().trim(), "Pass.", "{}", report.text());
    let manifest = Manifest::from_toml(text).unwrap();
    assert_eq!(manifest.id, "git");
    assert_eq!(manifest.name, "Git");
    assert_eq!(manifest.columns[0].id, "git");
    assert_eq!(manifest.place_info[0].marker.as_deref(), Some(".git"));
    assert_eq!(manifest.permissions.tools, ["git"]);
}

#[test]
fn the_plugin_passes_the_conformance_suite() {
    let root = scratch();
    let report = run(&plugin_folder(root.path(), &plugin_exe()));
    assert!(report.passed(), "{}", report.text());
    for group in [
        Group::Manifest,
        Group::Accessibility,
        Group::Lifecycle,
        Group::Framing,
        Group::Methods,
    ] {
        assert!(
            matches!(report.verdict(group), Some(Verdict::Pass(_))),
            "{group:?}: {}",
            report.text()
        );
    }
}

/// The skeleton through the real host: the Git column is offered, a page
/// of cells is all "pending", and a folder gets no description yet.
#[test]
fn the_host_offers_the_column_and_asks_for_the_title_line() {
    let root = scratch();
    let folder = plugin_folder(root.path(), &plugin_exe());
    let manifest =
        Manifest::from_toml(&std::fs::read_to_string(folder.join("plugin.toml")).unwrap()).unwrap();
    let mut host = Host::new(&diroxide_config::PluginSettings::default())
        .with_state_dir(root.path().join("state"));
    host.add(PluginInfo {
        manifest,
        folder,
        enabled: true,
    });
    let host = Arc::new(host);
    let columns = host.plugin_columns();
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].key(), "git.git");
    let repo = Place::parse(&root.path().display().to_string()).unwrap();
    let places = [
        repo.join("hello.txt").unwrap(),
        repo.join("notes.txt").unwrap(),
    ];
    let cells = host.column_page(&columns[0], &repo, &places);
    assert!(cells.iter().all(|c| c.text == "pending"), "{cells:?}");
    assert_eq!(host.describe_place("git", &repo).unwrap(), None);
    host.shutdown_all(QUIT_BUDGET);
}
