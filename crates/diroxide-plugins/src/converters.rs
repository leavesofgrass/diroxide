//! Plugins' converters in the recipe book (lane AW's request 4, CL's 6):
//! a manifest's `[[converter]]` tables made into the recipe model's
//! [`ConverterParts`], and only for a plugin the owner enabled whose files
//! are unchanged since (the consent check): a plugin changed since it was
//! enabled offers no converter until it is enabled again.
//!
//! The app (or `diro convert`) passes the parts to
//! `RecipeBook::add_converter`; this module reads files only to hash them
//! for the consent check.

use diroxide_ops::convert::{ConverterOutput, ConverterParts};
use toml::{Table, Value};

use crate::host::PluginInfo;
use crate::manifest::{ConverterSpec, Manifest};
use crate::script::consent::{ConsentState, ConsentStore, Hasher, plugin_fingerprint};

/// The strings of an array field of a table, others left out.
fn strings(table: &Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// One string field of a table, empty when it is missing.
fn string(table: &Table, key: &str) -> String {
    table
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// One converter as the recipe model needs it; `timeout` is the
/// manifest's `[timeouts] convert`, written as seconds ("90s").
pub fn converter_parts_of(
    plugin: &str,
    spec: &ConverterSpec,
    timeout: Option<u64>,
) -> ConverterParts {
    ConverterParts {
        plugin: plugin.to_owned(),
        id: spec.id.clone(),
        name: spec.name.clone(),
        extensions: strings(&spec.from, "extensions"),
        types: strings(&spec.from, "types"),
        outputs: spec
            .to
            .iter()
            .map(|output| ConverterOutput {
                id: string(output, "id"),
                name: string(output, "name"),
                extension: string(output, "extension"),
            })
            .collect(),
        needs: spec.needs.clone(),
        argv: spec.argv.clone(),
        timeout: timeout.map(|seconds| format!("{seconds}s")),
    }
}

/// Every converter a manifest declares, in its order.
pub fn converter_parts(manifest: &Manifest) -> Vec<ConverterParts> {
    let timeout = manifest
        .timeouts
        .convert
        .map(|limit| limit.as_secs().max(1));
    manifest
        .converters
        .iter()
        .map(|spec| converter_parts_of(&manifest.id, spec, timeout))
        .collect()
}

/// The converters of the plugins the owner enabled, each plugin's files
/// unchanged since its consent (`consent` holds the records, `hash` is
/// SHA-256): what the recipe book may offer. A plugin not enabled, or
/// changed since, offers none.
pub fn enabled_converters(
    plugins: &[PluginInfo],
    consent: &ConsentStore,
    hash: Hasher,
) -> Vec<ConverterParts> {
    plugins
        .iter()
        .filter(|plugin| plugin.enabled && !plugin.manifest.converters.is_empty())
        .filter(|plugin| {
            plugin_fingerprint(&plugin.manifest, &plugin.folder, hash)
                .is_ok_and(|now| consent.state(&plugin.manifest.id, &now) == ConsentState::Enabled)
        })
        .flat_map(|plugin| converter_parts(&plugin.manifest))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use diroxide_ops::convert::RecipeBook;

    use super::*;
    use crate::manifest::Manifest;
    use crate::script::consent::Kind;

    const MANIFEST: &str = r#"[plugin]
id = "pandoc-extra"
name = "Pandoc extra"
version = "0.1.0"
description = "Converts Org files to Word."
license = "MIT"
protocol = 1

[timeouts]
convert = "90s"

[[converter]]
id = "org-word"
name = "Org to Word"
from = { extensions = ["org"] }
to = [{ id = "docx", name = "Word document", extension = "docx" }]
needs = ["pandoc"]
argv = ["pandoc", "{input}", "-o", "{output}"]
"#;

    fn fake_hash(bytes: &[u8]) -> String {
        format!(
            "{:016x}",
            bytes
                .iter()
                .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(*b)))
        )
    }

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("converters-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("conv-")
            .tempdir_in(base)
            .unwrap()
    }

    fn installed(folder: &Path, text: &str, enabled: bool) -> PluginInfo {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join(Manifest::FILE), text).unwrap();
        PluginInfo {
            manifest: Manifest::from_toml(text).unwrap(),
            folder: folder.to_path_buf(),
            enabled,
        }
    }

    #[test]
    fn a_manifests_converter_becomes_a_recipe_in_the_book() {
        let manifest = Manifest::from_toml(MANIFEST).unwrap();
        let parts = converter_parts(&manifest);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].plugin, "pandoc-extra");
        assert_eq!(parts[0].extensions, ["org"]);
        assert_eq!(parts[0].outputs[0].extension, "docx");
        assert_eq!(parts[0].timeout.as_deref(), Some("90s"));
        let mut book = RecipeBook::new();
        book.add_converter(&parts[0]);
        assert!(book.problems().is_empty(), "{:?}", book.problems());
        assert!(book.get("pandoc-extra-org-word-docx").is_some());
    }

    #[test]
    fn only_an_enabled_unchanged_plugin_offers_its_converters() {
        let dir = scratch();
        let state = dir.path().join("state");
        let folder = dir.path().join("pandoc-extra");
        let plugin = installed(&folder, MANIFEST, true);
        let mut consent = ConsentStore::open(&state);
        // Not enabled yet: nothing offered.
        assert!(enabled_converters(std::slice::from_ref(&plugin), &consent, fake_hash).is_empty());
        let now = plugin_fingerprint(&plugin.manifest, &folder, fake_hash).unwrap();
        consent.enable("pandoc-extra", Kind::Plugin, now).unwrap();
        let offered = enabled_converters(std::slice::from_ref(&plugin), &consent, fake_hash);
        assert_eq!(offered.len(), 1);
        // Turned off in the settings: none.
        let off = PluginInfo {
            enabled: false,
            ..plugin.clone()
        };
        assert!(enabled_converters(&[off], &consent, fake_hash).is_empty());
        // Changed since its consent: none until it is enabled again.
        let changed = installed(&folder, &MANIFEST.replace("90s", "95s"), true);
        assert!(enabled_converters(&[changed], &consent, fake_hash).is_empty());
    }
}
