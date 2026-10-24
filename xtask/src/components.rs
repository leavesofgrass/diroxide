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
//!
//! Prebuilt plugin programs (R25 commit 87) come in through
//! `--assets FOLDER`: a pack member `{ asset = "TARGET/diro-sftp.exe" }`
//! is read from that folder, which the release workflow fills with each
//! platform's build of the plugin. A file whose asset is not there is
//! left out with a line, and a component left with no file is left out of
//! the list, so the list committed without assets stays current and
//! `regen --check` passes; the release regenerates it with the assets
//! before it builds `diro`, which then pins every plugin file.

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

/// One part of a file's pack: `from`, a folder or one file git tracks,
/// from the root; or `asset`, a prebuilt file under the assets folder
/// (`--assets`). It goes into the zip under `as` (empty for the top). An
/// `optional` folder that is not there yet is left out with a line; an
/// asset that is not given leaves its whole file out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackFrom {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    asset: Option<String>,
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

/// Where a member goes in the zip: `rel` under `into`, or at the top.
fn inner_name(into: &str, rel: &str) -> String {
    if into.is_empty() {
        rel.to_owned()
    } else {
        format!("{}/{rel}", into.trim_end_matches('/'))
    }
}

/// The members of one file's pack, read from the repository and the
/// assets folder; `None` when an asset it needs is not given, so the file
/// is left out (with a line in `notes`).
fn pack_members(
    root: &Path,
    assets: Option<&Path>,
    id: &str,
    file: &FileDefinition,
    notes: &mut Vec<String>,
) -> Result<Option<Vec<(String, Vec<u8>)>>> {
    let mut members = Vec::new();
    for part in &file.pack {
        match (&part.from, &part.asset) {
            (Some(from), None) => {
                let tracked = tracked_files(root, from)?;
                if tracked.is_empty() {
                    if part.optional {
                        notes.push(format!("{id}: {from} has no files yet; packed without it."));
                        continue;
                    }
                    bail!("{id}: {from} has no files git tracks");
                }
                let single = from.trim_end_matches('/');
                let prefix = format!("{single}/");
                for path in tracked {
                    // A `from` that names one file packs it by its name.
                    let rel = match path.strip_prefix(&prefix) {
                        Some(rel) => rel.to_owned(),
                        None if path == single => {
                            path.rsplit('/').next().unwrap_or(&path).to_owned()
                        }
                        None => path.clone(),
                    };
                    let bytes = fs::read(root.join(&path))
                        .with_context(|| format!("{path} could not be read"))?;
                    members.push((inner_name(&part.into, &rel), bytes));
                }
            }
            (None, Some(asset)) => {
                if !diroxide_components::pin::is_plain_path(asset) {
                    bail!("{id}: the asset {asset} is not plain names joined by slashes");
                }
                let found = assets
                    .map(|folder| folder.join(asset))
                    .filter(|path| path.is_file());
                let Some(path) = found else {
                    notes.push(format!(
                        "{id}: no build given for {}; {} left out.",
                        file.target, file.name
                    ));
                    return Ok(None);
                };
                let bytes = fs::read(&path)
                    .with_context(|| format!("{} could not be read", path.display()))?;
                let name = asset.rsplit('/').next().unwrap_or(asset);
                members.push((inner_name(&part.into, name), bytes));
            }
            _ => bail!("{id}: each pack part names a from or an asset, not both"),
        }
    }
    Ok(Some(members))
}

/// Builds every defined component from the repository, with no prebuilt
/// assets: the list as it is committed.
pub fn build(root: &Path) -> Result<Built> {
    build_with(root, None)
}

/// Builds every defined component from the repository and the prebuilt
/// files under `assets` (`<assets>/<target>/<program>`).
pub fn build_with(root: &Path, assets: Option<&Path>) -> Result<Built> {
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
            let Some(members) = pack_members(root, assets, &d.id, f, &mut notes)? else {
                continue;
            };
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
        if pins.is_empty() {
            notes.push(format!(
                "{}: no file was built; left out of the list.",
                d.id
            ));
            continue;
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

/// `cargo xtask components [--check] [--assets FOLDER]`: writes the built
/// files as a folder source and the list; with `--check`, only says
/// whether the list is current. `--assets` adds the prebuilt plugin
/// programs in FOLDER, as the release does.
pub fn run(root: &Path, args: &[&str]) -> Result<bool> {
    let mut check = false;
    let mut assets = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match *arg {
            "--check" => check = true,
            "--assets" => {
                let folder = rest.next().context("--assets needs a folder")?;
                assets = Some(PathBuf::from(folder));
            }
            other => {
                bail!("cargo xtask components takes --check and --assets FOLDER, not {other}")
            }
        }
    }
    let built = build_with(root, assets.as_deref())?;
    for note in &built.notes {
        println!("components: {note}");
    }
    let text = built.manifest.to_toml();
    let on_disk = fs::read_to_string(root.join(MANIFEST)).unwrap_or_default();
    if check {
        if on_disk == text {
            println!("components: Pass. {MANIFEST} is current.");
            Ok(true)
        } else {
            println!(
                "components: Fail. {MANIFEST} is out of date. Run cargo xtask components and commit it."
            );
            Ok(false)
        }
    } else {
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

    /// A folder of prebuilt plugin programs, as the release workflow lays
    /// them out: `<target>/diro-sftp`, with `.exe` on Windows.
    fn fake_assets(folder: &Path, targets: &[&str]) {
        for target in targets {
            let program = if target.contains("windows") {
                "diro-sftp.exe"
            } else {
                "diro-sftp"
            };
            fs::create_dir_all(folder.join(target)).unwrap();
            fs::write(
                folder.join(target).join(program),
                format!("a {target} build"),
            )
            .unwrap();
        }
    }

    const TARGETS: [&str; 3] = [
        "x86_64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
        "universal-apple-darwin",
    ];

    #[test]
    fn plugin_programs_come_in_as_assets_with_their_pins_and_sizes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let without = build(root).unwrap();
        assert!(without.manifest.get("diro-sftp").is_none());
        assert!(
            without
                .notes
                .iter()
                .any(|n| n == "diro-sftp: no file was built; left out of the list."),
            "{:?}",
            without.notes
        );
        let tmp = tempfile::tempdir().unwrap();
        fake_assets(tmp.path(), &TARGETS);
        let built = build_with(root, Some(tmp.path())).unwrap();
        let sftp = built
            .manifest
            .get("diro-sftp")
            .expect("the plugin is listed");
        assert_eq!(sftp.kind, Kind::Plugin);
        assert_eq!(sftp.files.len(), 3);
        for (pin, target) in sftp.files.iter().zip(TARGETS) {
            assert_eq!(pin.target, target);
            let file = built
                .files
                .iter()
                .find(|f| f.name == pin.name)
                .expect("the built file");
            assert!(pin.matches_bytes(&file.bytes), "{}", pin.name);
            assert_eq!(file.release, "diro-sftp-0.1.0-alpha.1");
            let mut zip = zip::ZipArchive::new(Cursor::new(file.bytes.clone())).unwrap();
            let names: Vec<String> = (0..zip.len())
                .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
                .collect();
            let program = pin.exec.clone().unwrap();
            assert_eq!(names, vec![program.clone(), "plugin.toml".to_owned()]);
            let entry = zip.by_name(&program).unwrap();
            assert_eq!(entry.unix_mode(), Some(0o100755), "{program}");
        }
        // The examples pack is the same with or without the assets.
        assert_eq!(
            built.manifest.get("diro-examples"),
            without.manifest.get("diro-examples")
        );
        // One build given: the others are left out, and said.
        let one = tempfile::tempdir().unwrap();
        fake_assets(one.path(), &TARGETS[1..2]);
        let partial = build_with(root, Some(one.path())).unwrap();
        assert_eq!(partial.manifest.get("diro-sftp").unwrap().files.len(), 1);
        assert!(
            partial
                .notes
                .iter()
                .any(|n| n.starts_with("diro-sftp: no build given for x86_64-pc-windows-msvc"))
        );
    }

    #[test]
    fn an_asset_path_that_is_not_plain_is_refused() {
        let file = FileDefinition {
            name: "x.zip".to_owned(),
            target: "any".to_owned(),
            exec: None,
            strip: 0,
            url: None,
            pack: vec![PackFrom {
                from: None,
                asset: Some("../outside/diro-sftp".to_owned()),
                into: String::new(),
                optional: false,
            }],
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let error = pack_members(root, Some(tmp.path()), "x", &file, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("not plain names"), "{error}");
    }

    /// The end-to-end test of the plugin as a component (R25 commit 87):
    /// built with prebuilt programs, served from a folder source, installed
    /// for this computer and verified; a computer with no build hears the
    /// cargo install line.
    #[test]
    fn the_plugin_component_installs_from_a_folder_source() {
        use std::sync::Arc;

        use diroxide_components::messages;
        use diroxide_components::{
            Consent, Downloader, FileState, Sources, StandardFetcher, Status, host_target,
        };
        use diroxide_vfs::OpContext;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let assets = tmp.path().join("assets");
        fake_assets(&assets, &TARGETS);
        let built = build_with(root, Some(&assets)).unwrap();
        let source = tmp.path().join("source");
        for file in &built.files {
            fs::create_dir_all(source.join(&file.release)).unwrap();
            fs::write(source.join(&file.release).join(&file.name), &file.bytes).unwrap();
        }
        let sftp = built.manifest.get("diro-sftp").unwrap();
        let components = tmp.path().join("state").join("components");
        let target = host_target();
        if sftp.files_for(target).is_empty() {
            assert_eq!(sftp.status_in(&components, target), Status::NoBuild);
            let lines = messages::no_build(sftp);
            assert_eq!(lines[0], "No build for this computer.");
            assert!(lines[2].starts_with("cargo install --locked --git "));
            return;
        }
        let sources = Sources::offline().with_source(&source.display().to_string());
        let downloader = Downloader::new(
            components.clone(),
            sources,
            Arc::new(StandardFetcher::local_only()),
        );
        let outcome = downloader
            .download(sftp, &Consent::yes(sftp), &OpContext::new(), &mut |_| {})
            .unwrap();
        assert_eq!(outcome.fetched.len(), 1, "one file for this computer");
        assert_eq!(sftp.status_in(&components, target), Status::Installed);
        let checked = sftp.verify_in(&components, target);
        assert!(checked.iter().any(|c| c.path == "plugin.toml"));
        assert!(
            checked.iter().all(|c| c.state == FileState::Good),
            "{checked:?}"
        );
        let program = sftp.files_for(target)[0].exec.clone().unwrap();
        let installed = sftp.dir_in(&components).join(&program);
        assert!(installed.is_file(), "{}", installed.display());
        // Another computer has no build here: the cargo install line.
        assert_eq!(
            sftp.status_in(&components, "aarch64-unknown-linux-musl"),
            Status::NoBuild
        );
        assert_eq!(
            messages::no_build(sftp)[1],
            "Build it from source with cargo:"
        );
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
