//! The host's conformance suite on the plugin (R25 commit 47's test), and
//! the plugin's manifest through the host's own checks.

use diroxide_plugins::Manifest;
use diroxide_plugins::conformance::{Group, Verdict, run};

use crate::{plugin_exe, plugin_folder};

#[test]
fn the_manifest_reads_and_keeps_the_40_cell_rule() {
    let text = include_str!("../../plugin.toml");
    let report = Manifest::check(text);
    assert_eq!(report.text().trim(), "Pass.", "{}", report.text());
    let manifest = Manifest::from_toml(text).unwrap();
    assert_eq!(manifest.id, "sftp");
    assert_eq!(manifest.name, "SFTP places");
    assert_eq!(manifest.providers[0].scheme, "sftp");
}

#[test]
fn the_plugin_passes_the_conformance_suite() {
    let root = tempfile::tempdir().unwrap();
    let report = run(&plugin_folder(root.path(), &plugin_exe()));
    assert!(report.passed(), "{}", report.text());
    for group in [
        Group::Manifest,
        Group::Accessibility,
        Group::Lifecycle,
        Group::Framing,
        Group::Methods,
        Group::Settings,
        Group::Providers,
    ] {
        assert!(
            matches!(report.verdict(group), Some(Verdict::Pass(_))),
            "{group:?}: {}",
            report.text()
        );
    }
}
