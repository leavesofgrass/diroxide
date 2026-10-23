//! The conformance suite on its own subjects: the fake plugin in its
//! correct and broken modes, a manifest-only plugin, a broken manifest,
//! and a Python plugin written with the standard library alone (when
//! Python is on this computer). The report's text is golden.

use std::path::{Path, PathBuf};

use diroxide_plugins::conformance::{Group, Verdict, run, run_cli};

const EXE: &str = env!("CARGO_BIN_EXE_diroxide-fake-plugin");

/// A plugin folder whose manifest runs the fake in `mode`.
fn fake(folder: &Path, mode: &str, extra: &str) -> PathBuf {
    let dir = folder.join(mode);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            "[plugin]\nid = \"fake\"\nname = \"Fake plugin\"\nversion = \"0.1.0\"\ndescription = \"Answers as its mode says.\"\nprotocol = 1\n\n[run]\ncommand = ['{EXE}', '{mode}']\nstartup_timeout = \"2s\"\n{extra}\n[[command]]\nid = \"echo\"\nname = \"Echo\"\ndescription = \"Answers at once.\"\n"
        ),
    )
    .unwrap();
    dir
}

#[test]
fn a_correct_plugin_passes_every_group() {
    let root = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    let code = run_cli(&fake(root.path(), "echo", ""), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text,
        "Pass: Manifest\nPass: Accessibility\nPass: Lifecycle\nPass: Framing\nPass: Methods\nPass: Commands\nPass: Cancel\nPass: Settings\nSkipped: Providers: none declared\nSkipped: Hooks: none declared\nPass: Services\n  Note: it called no services\nPass: Output hygiene\nConformance: 10 passed, 0 failed, 2 skipped (Fake plugin)\n"
    );
    assert_eq!(code, 0);
}

#[test]
fn a_broken_plugin_fails_and_names_the_check() {
    let root = tempfile::tempdir().unwrap();
    let report = run(&fake(root.path(), "bad", ""));
    assert!(!report.passed());
    assert_eq!(
        report.verdict(Group::Lifecycle),
        Some(&Verdict::Fail(vec![
            "a request before initialize was not answered -32002".to_owned()
        ]))
    );
    assert_eq!(
        report.verdict(Group::Methods),
        Some(&Verdict::Fail(vec![
            "an unknown request was not answered -32601".to_owned()
        ]))
    );
    let text = report.text();
    assert!(text.contains("Fail: Output hygiene: "), "{text}");
    assert!(
        text.ends_with("Conformance: 6 passed, 4 failed, 2 skipped (Fake plugin)\n"),
        "{text}"
    );
}

#[test]
fn a_silent_plugin_fails_at_the_start() {
    let root = tempfile::tempdir().unwrap();
    let report = run(&fake(root.path(), "silent", ""));
    assert_eq!(
        report.verdict(Group::Lifecycle),
        Some(&Verdict::Fail(vec![
            "a request before initialize was not answered".to_owned(),
            "initialize was not answered in 2 seconds".to_owned()
        ]))
    );
    assert_eq!(
        report.verdict(Group::Commands),
        Some(&Verdict::Skipped("the plugin did not start".to_owned()))
    );
}

#[test]
fn hooks_and_providers_are_checked_when_declared() {
    let root = tempfile::tempdir().unwrap();
    let report = run(&fake(
        root.path(),
        "provider",
        "\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\n\n[[hook]]\nevent = \"before_delete\"\n\n[[hook]]\nevent = \"after_copy\"\n",
    ));
    assert_eq!(
        report.verdict(Group::Providers),
        Some(&Verdict::Pass(Vec::new()))
    );
    assert_eq!(
        report.verdict(Group::Hooks),
        Some(&Verdict::Pass(Vec::new()))
    );
    assert!(report.passed(), "{}", report.text());
}

#[test]
fn a_manifest_only_plugin_and_a_broken_manifest() {
    let root = tempfile::tempdir().unwrap();
    let recipe = root.path().join("recipe");
    std::fs::create_dir_all(&recipe).unwrap();
    std::fs::write(
        recipe.join("plugin.toml"),
        "[plugin]\nid = \"recipe\"\nname = \"Recipe\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[[command]]\nid = \"count\"\nname = \"Count\"\nargv = [\"wc\", \"-l\", \"{file}\"]\n",
    )
    .unwrap();
    let report = run(&recipe);
    assert!(report.passed(), "{}", report.text());
    assert_eq!(
        report.verdict(Group::Lifecycle),
        Some(&Verdict::Skipped(
            "a manifest-only plugin runs no process".to_owned()
        ))
    );
    let broken = root.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("plugin.toml"), "[plugin]\nid = \"Bad Id\"\n").unwrap();
    let report = run(&broken);
    assert!(!report.passed());
    assert!(
        report.text().starts_with("Fail: Manifest: "),
        "{}",
        report.text()
    );
    let mut out = Vec::new();
    assert_eq!(run_cli(&broken, &mut out), 1);
}

/// The Python subject, a plugin with the standard library alone; skipped
/// where no Python is found.
#[test]
fn a_python_plugin_with_the_standard_library_passes() {
    let python = if cfg!(windows) { "py" } else { "python3" };
    if diroxide_tools::which(python, std::env::var_os("PATH").as_deref()).is_none() {
        println!("no {python} on this computer: skipped");
        return;
    }
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/plugins/conformance/python-echo");
    let report = run(&folder);
    assert!(report.passed(), "{}", report.text());
    assert_eq!(report.plugin, "Python echo");
}
