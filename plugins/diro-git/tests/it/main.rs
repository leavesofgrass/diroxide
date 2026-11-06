//! The git plugin through the host's own checks (its manifest, the
//! conformance suite against the built program, the column and the title
//! line asked by the real host), and through a fake host that runs real
//! git with a placeholder identity for every command (`support.rs`).

// Test helpers may unwrap: a failure is the test's failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod branches;
mod commit;
mod remote;
mod status;
mod support;
mod tree;
mod trust;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_core::Place;
use diroxide_plugins::conformance::{Group, Verdict, run};
use diroxide_plugins::host::QUIT_BUDGET;
use diroxide_plugins::tools::ToolBox;
use diroxide_plugins::{Host, Manifest, PluginInfo};

use support::scratch;

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
    // Every command the build answers is declared, and the reverse.
    let declared: Vec<&str> = manifest.commands.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(declared, diro_git::commands::IDS);
    let lists: Vec<&str> = manifest.listers.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(lists, diro_git::commands::LISTERS);
    for command in &manifest.commands {
        assert!(command.name.starts_with("Git: "), "{}", command.name);
        assert!(command.key.is_none(), "{} has a key", command.id);
        assert_eq!(command.needs, ["git"]);
    }
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
        Group::Commands,
        Group::Cancel,
    ] {
        assert!(
            matches!(report.verdict(group), Some(Verdict::Pass(_))),
            "{group:?}: {}",
            report.text()
        );
    }
}

/// Through the real host, with a PATH that holds no git (so no git of
/// the owner's ever runs here): the Git column is offered, a page of
/// cells says git is missing, and so does the title line.
#[test]
fn the_real_host_offers_the_column_and_the_title_line_says_git_is_missing() {
    let root = scratch();
    let folder = plugin_folder(root.path(), &plugin_exe());
    let manifest =
        Manifest::from_toml(&std::fs::read_to_string(folder.join("plugin.toml")).unwrap()).unwrap();
    let empty = root.path().join("no-tools");
    std::fs::create_dir_all(&empty).unwrap();
    let mut host = Host::new(&diroxide_config::PluginSettings::default())
        .with_state_dir(root.path().join("state"))
        .with_tools(ToolBox::with_path(Some(empty.into_os_string())));
    host.add(PluginInfo {
        manifest,
        folder,
        enabled: true,
    });
    let host = Arc::new(host);
    let columns = host.plugin_columns();
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].key(), "git.git");
    let repo_path = root.path().join("repo");
    std::fs::create_dir_all(repo_path.join(".git")).unwrap();
    let repo = Place::parse(&repo_path.display().to_string()).unwrap();
    let places = [
        repo.join("hello.txt").unwrap(),
        repo.join("notes.txt").unwrap(),
    ];
    let cells = host.column_page(&columns[0], &repo, &places);
    assert!(cells.iter().all(|c| c.text == "git not found"), "{cells:?}");
    assert_eq!(
        host.describe_place("git", &repo).unwrap().as_deref(),
        Some("git not found")
    );
    host.shutdown_all(QUIT_BUDGET);
}
