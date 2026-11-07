//! The encryption plugin through the host's own checks (its manifest, the
//! conformance suite against the built program), and through a fake host
//! that answers its forms and questions (`support.rs`), with keys and
//! passphrases made at test time.

// Test helpers may unwrap: a failure is the test's failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod decrypt;
mod encrypt;
mod fuzz;
mod secrets;
mod support;

use std::path::{Path, PathBuf};

use diroxide_plugins::Manifest;
use diroxide_plugins::conformance::{Group, Verdict, run};

use support::scratch;

/// The built plugin program.
fn plugin_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diro-crypt"))
}

/// The plugin's own manifest with its program replaced by `exe`, in a
/// folder of its own under `root`.
fn plugin_folder(root: &Path, exe: &Path) -> PathBuf {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin.toml")).unwrap();
    let command = format!("['{}']", exe.display());
    let manifest = manifest
        .replace("[\"./diro-crypt.exe\"]", &command)
        .replace("[\"./diro-crypt\"]", &command);
    let folder = root.join("crypt");
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
    assert_eq!(manifest.id, "crypt");
    assert_eq!(manifest.name, "Encryption");
    // Every command the build answers is declared, and the reverse.
    let declared: Vec<&str> = manifest.commands.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(declared, diro_crypt::commands::IDS);
    let names: Vec<&str> = manifest.commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Encrypt with a passphrase",
            "Encrypt for a recipient",
            "Decrypt"
        ]
    );
    for command in &manifest.commands {
        assert!(command.key.is_none(), "{} has a key", command.id);
        assert!(command.needs.is_empty(), "{} needs a tool", command.id);
    }
    // It asks the host for no files, panels, tools or programs.
    assert!(manifest.permissions.tools.is_empty());
    assert!(manifest.permissions.launch.is_empty());
    assert!(manifest.permissions.commands.is_empty());
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
