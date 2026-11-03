//! The slow tests' gate (R27's commit 37, `docs/dev/building.md`, "Slow
//! tests"). A test that takes ten seconds or more in the full run runs
//! only when `DIRO_SLOW` is `1`, which CI's jobs, `scripts/dev-check.sh`
//! and `docker/sftp/run-tests.sh` set. A builder's quick run leaves it
//! unset: each slow test then writes [`SKIPPED`] and passes at once, and
//! nextest shows that line, since `.config/nextest.toml` names the test in
//! its slow override.
//!
//! Each crate with slow tests keeps its own copy of the gate, a function
//! named `slow_test_skipped`. This module is xtask's copy, compiled for
//! its tests only, with the tests that keep the copies and the override in
//! step: every gated test is named in the override, every name there is
//! gated, and every copy writes the same line.

use std::ffi::OsStr;

/// The variable that runs the slow tests.
pub const VARIABLE: &str = "DIRO_SLOW";

/// The line a skipped slow test writes, which nextest shows.
pub const SKIPPED: &str = "skipped: slow, set DIRO_SLOW=1";

/// Whether the slow tests run, from the variable's value: only `1` runs
/// them; unset, empty, `0` or anything else skips them.
pub fn wanted(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// xtask's gate, the first line of each of its slow tests: true when the
/// test is skipped, after it says so.
pub fn slow_test_skipped() -> bool {
    if wanted(std::env::var_os(VARIABLE).as_deref()) {
        return false;
    }
    println!("{SKIPPED}");
    true
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    fn root() -> PathBuf {
        crate::bench::workspace_root().unwrap()
    }

    /// The Rust files below `folder`, in a steady order.
    fn rust_files(folder: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(folder) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The main workspace's Rust files that may hold tests: the crates and
    /// xtask (the vendored crates and the fuzz targets hold none of ours).
    fn workspace_files() -> Vec<PathBuf> {
        let root = root();
        let mut files = Vec::new();
        rust_files(&root.join("crates"), &mut files);
        rust_files(&root.join("xtask").join("src"), &mut files);
        files
    }

    /// A function's name, when this line declares one.
    fn declared(line: &str) -> Option<&str> {
        let rest = line.trim_start();
        let rest = rest.strip_prefix("pub ").unwrap_or(rest);
        let rest = rest.strip_prefix("pub(crate) ").unwrap_or(rest);
        let rest = rest.strip_prefix("fn ")?;
        let end = rest.find(['(', '<'])?;
        Some(&rest[..end])
    }

    /// Whether this line is a gate's call, as every slow test starts:
    /// `if crate::slow_test_skipped() {`, with `return;` on the next line.
    fn calls_the_gate(line: &str) -> bool {
        let line = line.trim();
        line.starts_with("if ") && line.ends_with(concat!("slow_test_skipped", "() {"))
    }

    /// The tests that call a gate, by their function's name, with the file
    /// they are in: the function declared last before the call.
    fn gated(files: &[PathBuf]) -> Vec<(String, PathBuf)> {
        let mut found = Vec::new();
        for file in files {
            let text = fs::read_to_string(file).unwrap();
            let mut current: Option<&str> = None;
            for line in text.lines() {
                if let Some(name) = declared(line) {
                    current = Some(name);
                    continue;
                }
                if calls_the_gate(line) {
                    let name = current.unwrap_or_else(|| {
                        panic!("a gate outside a function in {}", file.display())
                    });
                    found.push((name.to_owned(), file.clone()));
                }
            }
        }
        found
    }

    /// The tests named in the slow override of `.config/nextest.toml`,
    /// by their function's name (the last part of each `test(=...)`).
    fn named_in_the_override() -> Vec<String> {
        let file = root().join(".config").join("nextest.toml");
        let config: toml::Value = toml::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        let overrides = config["profile"]["default"]["overrides"]
            .as_array()
            .unwrap();
        let slow: Vec<&toml::Value> = overrides
            .iter()
            .filter(|o| o.get("success-output").and_then(toml::Value::as_str) == Some("immediate"))
            .collect();
        assert_eq!(slow.len(), 1, "one override shows the slow tests' line");
        let filter = slow[0]["filter"].as_str().unwrap();
        let mut names = Vec::new();
        let mut rest = filter;
        while let Some(start) = rest.find("test(=") {
            rest = &rest[start + "test(=".len()..];
            let end = rest.find(')').unwrap();
            let path = rest[..end].trim();
            names.push(path.rsplit("::").next().unwrap().to_owned());
            rest = &rest[end..];
        }
        names
    }

    #[test]
    fn only_one_runs_the_slow_tests() {
        assert!(wanted(Some(OsStr::new("1"))));
        for value in ["", "0", "yes", "true", " 1"] {
            assert!(!wanted(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(!wanted(None));
    }

    #[test]
    fn the_line_puts_its_meaning_first() {
        assert!(SKIPPED.starts_with("skipped: slow"));
        assert!(SKIPPED.chars().count() <= 40, "{SKIPPED}");
        assert!(SKIPPED.ends_with(&format!("{VARIABLE}=1")));
    }

    #[test]
    fn every_gated_test_is_named_in_the_override_and_no_other() {
        let gated = gated(&workspace_files());
        assert!(!gated.is_empty(), "the gate is called somewhere");
        let named = named_in_the_override();
        for (name, file) in &gated {
            assert!(
                named.contains(name),
                "{name} in {} is gated but not named in the slow override of .config/nextest.toml, so its line would not show",
                file.display()
            );
        }
        for name in &named {
            assert!(
                gated.iter().any(|(gated, _)| gated == name),
                "{name} is named in the slow override but calls no gate"
            );
        }
        let mut unique = named.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), named.len(), "each slow test is named once");
    }

    #[test]
    fn every_copy_of_the_gate_reads_the_same_variable_and_says_the_same_line() {
        let root = root();
        let mut files = workspace_files();
        rust_files(&root.join("plugins"), &mut files);
        let mut copies = 0;
        for file in &files {
            let text = fs::read_to_string(file).unwrap();
            if !text.contains("fn slow_test_skipped()") {
                continue;
            }
            copies += 1;
            if file.ends_with(Path::new("xtask").join("src").join("slow.rs")) {
                continue;
            }
            assert!(
                text.contains(&format!("\"{SKIPPED}\"")),
                "{} says another line",
                file.display()
            );
            assert!(
                text.contains(&format!("var_os(\"{VARIABLE}\")")) && text.contains("v == \"1\""),
                "{} reads the variable another way",
                file.display()
            );
        }
        // xtask's, the preview crate's, the archive crate's, the plugin
        // crate's and the SFTP plugin's server tests'.
        assert_eq!(copies, 5, "one copy in each crate with slow tests");
    }

    #[test]
    fn a_call_is_told_from_a_mention() {
        assert!(calls_the_gate("    if crate::slow_test_skipped() {"));
        assert!(calls_the_gate("if slow_test_skipped() {"));
        assert!(!calls_the_gate("pub fn slow_test_skipped() -> bool {"));
        assert!(!calls_the_gate(
            r#"    if code.contains("slow_test_skipped()") {"#
        ));
        assert!(!calls_the_gate(
            "// if crate::slow_test_skipped() { return; }"
        ));
    }
}
