//! `cargo xtask components [--check]`: builds the components this
//! repository makes, computes each file's size and SHA-256, and writes the
//! public list `crates/diroxide-components/components.toml`, which the
//! `diro` binary embeds (R22 section 6.2, question 2's default: the list
//! is generated from the real assets, never typed, and committed).
//!
//! The definitions are `packaging/components/components.toml`: each
//! component's words (title, kind, version, license, features, action)
//! and, per file, the folders the xtask packs into it. A pack is a zip
//! written the same way every time: the members are the files git
//! tracks under each folder, sorted, stored without compression, dated
//! 1980-01-01, marked as Unix files with mode 644 (755 for the file a
//! component names as its program). So the same commit gives the same
//! bytes on every computer, and `cargo xtask regen --check` fails when
//! the list is stale.
//!
//! The built files are also written as a ready folder source,
//! `<target>/components/source/<id>-<version>/<file>`, for the owner's
//! check (`diro components download diro-examples --source FOLDER`) and
//! for the release's assets.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use diroxide_components::manifest::{self, Manifest};
use diroxide_components::{Action, Component, FilePin, Kind, Origin, is_plain_name};
use serde::Deserialize;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipWriter};

/// The definitions, from the root.
pub const DEFINITIONS: &str = "packaging/components/components.toml";

/// The generated list, from the root.
pub const MANIFEST: &str = "crates/diroxide-components/components.toml";

/// What wrote the list, as it says.
pub const GENERATED_BY: &str = "cargo xtask components";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definitions {
    format: u32,
    #[serde(default)]
    component: Vec<Definition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    id: String,
    title: String,
    kind: String,
    version: String,
    license: String,
    #[serde(default)]
    license_note: String,
    #[serde(default)]
    credit: String,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    needs_tools: Vec<String>,
    #[serde(default)]
    needs_components: Vec<String>,
    protocol: Option<u32>,
    min_diro: Option<String>,
    action: String,
    folder: Option<String>,
    file: Vec<FileDefinition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDefinition {
    name: String,
    #[serde(default = "any")]
    target: String,
    exec: Option<String>,
    #[serde(default)]
    strip: u32,
    url: Option<String>,
    pack: Vec<PackFrom>,
}

fn any() -> String {
    "any".to_owned()
}

/// One folder packed into a file: `from`, from the root, goes into the
/// zip under `as` (empty for the top). An `optional` folder that is not
/// there yet is left out with a line.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackFrom {
    from: String,
    #[serde(rename = "as", default)]
    into: String,
    #[serde(default)]
    optional: bool,
}

/// One built file: the release folder it goes under, its name, its bytes.
pub struct BuiltFile {
    /// `<id>-<version>`.
    pub release: String,
    /// The file's name.
    pub name: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// The list and the files it pins.
pub struct Built {
    /// The list, as it is embedded.
    pub manifest: Manifest,
    /// The files, in the list's order.
    pub files: Vec<BuiltFile>,
    /// What was left out, in words.
    pub notes: Vec<String>,
}

/// The files git tracks under `folder` (from the root), as paths with `/`.
fn tracked_files(root: &Path, folder: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--"])
        .arg(folder)
        .output()
        .context("git could not be run to list the files to pack")?;
    if !output.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut files: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    files.sort();
    Ok(files)
}

/// A zip of `members` (path in the zip, bytes), written the same way
/// every time: sorted, stored, dated 1980-01-01, Unix mode 644, or 755
/// for `exec`.
pub fn pack_bytes(mut members: Vec<(String, Vec<u8>)>, exec: Option<&str>) -> Result<Vec<u8>> {
    members.sort_by(|a, b| a.0.cmp(&b.0));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in &members {
        if !path.split('/').all(is_plain_name) {
            bail!("{path} is not plain names joined by slashes; rename it before it is packed");
        }
        let mode = if Some(path.as_str()) == exec {
            0o755
        } else {
            0o644
        };
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(DateTime::default())
            .system(System::Unix)
            .unix_permissions(mode);
        zip.start_file(path.as_str(), options)
            .with_context(|| format!("{path} could not be added"))?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

/// The members of one file's pack, read from the repository.
fn pack_members(
    root: &Path,
    id: &str,
    file: &FileDefinition,
    notes: &mut Vec<String>,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut members = Vec::new();
    for from in &file.pack {
        let tracked = tracked_files(root, &from.from)?;
        if tracked.is_empty() {
            if from.optional {
                notes.push(format!(
                    "{id}: {} has no files yet; packed without it.",
                    from.from
                ));
                continue;
            }
            bail!("{id}: {} has no files git tracks", from.from);
        }
        let prefix = format!("{}/", from.from.trim_end_matches('/'));
        for path in tracked {
            let rel = path.strip_prefix(&prefix).unwrap_or(&path);
            let inner = if from.into.is_empty() {
                rel.to_owned()
            } else {
                format!("{}/{rel}", from.into.trim_end_matches('/'))
            };
            let bytes =
                fs::read(root.join(&path)).with_context(|| format!("{path} could not be read"))?;
            members.push((inner, bytes));
        }
    }
    Ok(members)
}

/// Builds every defined component from the repository.
pub fn build(root: &Path) -> Result<Built> {
    let text = fs::read_to_string(root.join(DEFINITIONS))
        .with_context(|| format!("{DEFINITIONS} could not be read"))?;
    let definitions: Definitions =
        toml::from_str(&text).with_context(|| format!("{DEFINITIONS} is not as expected"))?;
    if definitions.format != manifest::FORMAT {
        bail!("{DEFINITIONS} has format {}, not 1", definitions.format);
    }
    let mut list = Manifest::new(env!("CARGO_PKG_VERSION"), GENERATED_BY);
    let mut files = Vec::new();
    let mut notes = Vec::new();
    for d in definitions.component {
        let kind =
            Kind::from_word(&d.kind).with_context(|| format!("{}: kind {}", d.id, d.kind))?;
        let action = match d.action.as_str() {
            "place" => Action::Place,
            "unpack" => Action::Unpack,
            other => bail!("{}: the action {other} is not one the list knows", d.id),
        };
        let release = format!("{}-{}", d.id, d.version);
        let mut pins = Vec::new();
        let mut unpacked_size = 0u64;
        for f in &d.file {
            let members = pack_members(root, &d.id, f, &mut notes)?;
            unpacked_size += members.iter().map(|(_, b)| b.len() as u64).sum::<u64>();
            let bytes = pack_bytes(members, f.exec.as_deref())?;
            let mut pin = FilePin::of_bytes(f.name.clone(), &bytes);
            pin.target = f.target.clone();
            pin.exec = f.exec.clone();
            pin.strip = f.strip;
            pin.url = f.url.clone();
            pins.push(pin);
            files.push(BuiltFile {
                release: release.clone(),
                name: f.name.clone(),
                bytes,
            });
        }
        let mut component = Component::new(d.id.clone(), d.title, pins);
        component.kind = kind;
        component.version = d.version;
        component.license = d.license;
        component.license_note = d.license_note;
        component.credit = d.credit;
        component.features = d.features;
        component.needs_tools = d.needs_tools;
        component.needs_components = d.needs_components;
        component.protocol = d.protocol;
        component.min_diro = d.min_diro;
        component.action = action;
        component.folder = d.folder.unwrap_or(d.id);
        component.unpacked_size = if action == Action::Unpack {
            unpacked_size
        } else {
            0
        };
        component.origin = Origin::Public;
        list.components.push(component);
    }
    // The list must read back whole, as the program will read it.
    let read = manifest::parse(&list.to_toml(), Origin::Public)?;
    if let Some(refusal) = read.refused.first() {
        bail!("{}", refusal.line());
    }
    Ok(Built {
        manifest: list,
        files,
        notes,
    })
}

/// The generated list as a page for `cargo xtask regen`: its path and
/// text.
pub fn page(root: &Path) -> Result<(String, String)> {
    Ok((MANIFEST.to_owned(), build(root)?.manifest.to_toml()))
}

/// Where the built files go: `<target>/components/source`.
fn source_folder(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("components")
        .join("source")
}

/// `cargo xtask components`: writes the built files as a folder source and
/// the list; with `--check`, only says whether the list is current.
pub fn run(root: &Path, args: &[&str]) -> Result<bool> {
    let built = build(root)?;
    for note in &built.notes {
        println!("components: {note}");
    }
    let text = built.manifest.to_toml();
    let on_disk = fs::read_to_string(root.join(MANIFEST)).unwrap_or_default();
    match args {
        ["--check"] => {
            if on_disk == text {
                println!("components: Pass. {MANIFEST} is current.");
                Ok(true)
            } else {
                println!(
                    "components: Fail. {MANIFEST} is out of date. Run cargo xtask components and commit it."
                );
                Ok(false)
            }
        }
        [] => {
            let source = source_folder(root);
            for file in &built.files {
                let folder = source.join(&file.release);
                fs::create_dir_all(&folder)
                    .with_context(|| format!("{} could not be made", folder.display()))?;
                let path = folder.join(&file.name);
                fs::write(&path, &file.bytes)
                    .with_context(|| format!("{} could not be written", path.display()))?;
                println!(
                    "components: {}, {} bytes, in {}",
                    file.name,
                    file.bytes.len(),
                    folder.display()
                );
            }
            if on_disk == text {
                println!("components: {MANIFEST} is current.");
            } else {
                fs::write(root.join(MANIFEST), &text)
                    .with_context(|| format!("{MANIFEST} could not be written"))?;
                println!("components: wrote {MANIFEST}.");
            }
            Ok(true)
        }
        _ => bail!("cargo xtask components takes nothing or --check"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_is_the_same_bytes_every_time() {
        let members = vec![
            ("plugins/b.toml".to_owned(), b"b".to_vec()),
            ("README.md".to_owned(), b"readme".to_vec()),
        ];
        let mut reversed = members.clone();
        reversed.reverse();
        let one = pack_bytes(members, None).unwrap();
        let two = pack_bytes(reversed, None).unwrap();
        assert_eq!(one, two);
        let mut zip = zip::ZipArchive::new(Cursor::new(one)).unwrap();
        assert_eq!(zip.len(), 2);
        let first = zip.by_index(0).unwrap();
        assert_eq!(first.name().unwrap(), "README.md");
        assert_eq!(first.unix_mode(), Some(0o100644));
    }

    #[test]
    fn a_program_gets_its_mode_and_odd_names_are_refused() {
        let bytes = pack_bytes(
            vec![("bin/tool".to_owned(), b"#!".to_vec())],
            Some("bin/tool"),
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(zip.by_index(0).unwrap().unix_mode(), Some(0o100755));
        let error = pack_bytes(vec![("a b.txt".to_owned(), Vec::new())], None).unwrap_err();
        assert!(error.to_string().contains("a b.txt is not plain names"));
    }

    /// The end-to-end test of the whole path with no network (R22 section
    /// 6.4): the examples pack, built as the release builds it, served
    /// from a folder source named the way the owner's `components_source`
    /// names one, downloaded against the embedded list, verified, removed.
    #[test]
    fn the_examples_pack_downloads_verifies_and_removes_from_a_folder_source() {
        use std::sync::Arc;

        use diroxide_components::{
            Consent, Downloader, FileState, Sources, StandardFetcher, Status, host_target,
        };
        use diroxide_vfs::OpContext;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let built = build(root).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        for file in &built.files {
            fs::create_dir_all(source.join(&file.release)).unwrap();
            fs::write(source.join(&file.release).join(&file.name), &file.bytes).unwrap();
        }
        let list = Manifest::embedded();
        let pack = list
            .get("diro-examples")
            .expect("the examples pack is listed");
        let sources = Sources::offline().with_source(&source.display().to_string());
        let components = tmp.path().join("state").join("components");
        let downloader = Downloader::new(
            components.clone(),
            sources,
            Arc::new(StandardFetcher::local_only()),
        );
        let mut events = Vec::new();
        let outcome = downloader
            .download(pack, &Consent::yes(pack), &OpContext::new(), &mut |e| {
                events.push(e.clone());
            })
            .unwrap();
        assert_eq!(outcome.fetched, ["diro-examples-0.1.0.zip"]);
        assert!(outcome.left_out.is_empty(), "{:?}", outcome.left_out);
        let target = host_target();
        assert_eq!(pack.status_in(&components, target), Status::Installed);
        let checked = pack.verify_in(&components, target);
        assert!(checked.iter().any(|c| c.path == "README.md"));
        assert!(
            checked.iter().all(|c| c.state == FileState::Good),
            "{checked:?}"
        );
        let removal = pack.removal_in(&components);
        assert!(removal.kept.is_empty());
        removal.run().unwrap();
        assert!(!pack.dir_in(&components).exists());
        assert_eq!(pack.status_in(&components, target), Status::NotInstalled);
    }

    #[test]
    fn the_embedded_list_is_current() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let (path, text) = page(root).unwrap();
        let on_disk = fs::read_to_string(root.join(&path)).unwrap();
        assert_eq!(
            on_disk, text,
            "run cargo xtask components and commit {path}"
        );
        assert_eq!(on_disk, manifest::EMBEDDED);
    }
}
