//! A component: what it is, what needs it, its pinned files, and its
//! state on this computer (status, verify, remove).
//!
//! The record, [`Component::status_in`], [`Component::verify_in`] and the
//! remove rule (only the component's own files, a file the owner added
//! stays) are textweaver's design (`component.rs`, the owner's own code,
//! relicensed under answer 3). diroxide adds the kind, the version and the
//! license note, files per target, and a remove that is planned first:
//! [`Component::removal_in`] lists every path it will remove, and
//! [`Removal::run`] removes those paths and no others (the listing rule of
//! ADR-0008).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use diroxide_core::format::{count, human_size};

use crate::error::ComponentError;
use crate::pin::{FilePin, hash_file, is_plain_name, is_plain_path};
use crate::receipt::{self, Receipt};

/// The suffix of the staging folder beside a component's folder:
/// `<folder>.partial`.
pub const STAGING_SUFFIX: &str = ".partial";

/// The suffix of a file still arriving: `<name>.part`.
pub const PART_SUFFIX: &str = ".part";

/// The suffix of the note beside a `.part` file that says where its bytes
/// came from: `<name>.part.from`.
pub const SIDECAR_SUFFIX: &str = ".part.from";

/// The lock file in the staging folder.
pub const LOCK_NAME: &str = "download.lock";

/// What sort of component it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A plugin executable: the host finds its `plugin.toml` at the top of
    /// its folder. Installing never enables it.
    Plugin,
    /// A helper program the tools registry finds through its `exec` path.
    Helper,
    /// Files only: the example plugins, a keymap preset, a lexicon.
    Data,
}

impl Kind {
    /// Every kind, in the list's order.
    pub const ALL: [Kind; 3] = [Kind::Plugin, Kind::Helper, Kind::Data];

    /// The word the list uses.
    pub const fn word(self) -> &'static str {
        match self {
            Kind::Plugin => "plugin",
            Kind::Helper => "helper",
            Kind::Data => "data",
        }
    }

    /// The kind a list's word names.
    pub fn from_word(word: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.word() == word)
    }
}

/// What installing does with the checked files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// The checked files are the component.
    Place,
    /// Each archive among the files is unpacked into the folder, and a
    /// receipt takes its place.
    Unpack,
}

impl Action {
    /// The word the list uses.
    pub const fn word(self) -> &'static str {
        match self {
            Action::Place => "place",
            Action::Unpack => "unpack",
        }
    }
}

/// Which list a component came from: the ask-first question names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The public list embedded in this build.
    Public,
    /// The list of the owner's own `components_source`.
    OwnSource,
}

/// An optional component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// A plain name: `diro-examples`, `diro-sftp`.
    pub id: String,
    /// What it is, meaning first: "Example plugins", "SFTP places plugin".
    pub title: String,
    /// Plugin, helper or data.
    pub kind: Kind,
    /// Its version: `0.1.0`.
    pub version: String,
    /// An SPDX expression: `Apache-2.0 OR MIT`.
    pub license: String,
    /// Words said after the license, such as "unconfirmed"; often empty.
    pub license_note: String,
    /// Who made it, for the notices.
    pub credit: String,
    /// The features that need it, as keys the app names in words
    /// (`examples`, `sftp`, `rar`).
    pub features: Vec<String>,
    /// Programs it needs, by the tools registry's names (`python3`).
    pub needs_tools: Vec<String>,
    /// Components it needs, by id.
    pub needs_components: Vec<String>,
    /// For a plugin, the protocol major it speaks.
    pub protocol: Option<u32>,
    /// The oldest diroxide it works with.
    pub min_diro: Option<String>,
    /// What installing does.
    pub action: Action,
    /// Its folder under the components folder: plain names joined by `/`;
    /// the id by default.
    pub folder: String,
    /// Bytes its archives unpack to: the unpack ceiling and part of the
    /// free-space check. Zero for a component that unpacks nothing.
    pub unpacked_size: u64,
    /// Its files, for every computer it has a build for.
    pub files: Vec<FilePin>,
    /// Which list it came from.
    pub origin: Origin,
}

/// Whether a component is installed, from the files' sizes (quick; the
/// hashes are [`Component::verify_in`]'s).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every file is there with its pinned size, or unpacked by a receipt
    /// whose files are all there.
    Installed,
    /// None of its files are there.
    NotInstalled,
    /// Some files are missing (their names).
    Partial(Vec<String>),
    /// A file has the wrong size, or a file an archive wrote is gone (its
    /// name).
    Damaged(String),
    /// It has no files for this computer.
    NoBuild,
}

impl Status {
    /// The state in words, for a row: "installed", "not installed",
    /// "partly installed", "damaged", "no build for this computer".
    pub fn words(&self) -> &'static str {
        match self {
            Status::Installed => "installed",
            Status::NotInstalled => "not installed",
            Status::Partial(_) => "partly installed",
            Status::Damaged(_) => "damaged",
            Status::NoBuild => "no build for this computer",
        }
    }

    /// Whether it is ready to use.
    pub fn is_installed(&self) -> bool {
        matches!(self, Status::Installed)
    }
}

/// One file, checked by size and hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    /// Size and hash match.
    Good,
    /// Not there.
    Missing,
    /// There, with the wrong size (the bytes found).
    WrongSize(u64),
    /// The right size, the wrong hash.
    WrongHash,
}

impl FileState {
    /// The state in words: "good", "missing", "wrong size", "does not
    /// match".
    pub fn words(self) -> &'static str {
        match self {
            FileState::Good => "good",
            FileState::Missing => "missing",
            FileState::WrongSize(_) => "wrong size",
            FileState::WrongHash => "does not match",
        }
    }
}

/// A file checked by [`Component::verify_in`]: its path under the
/// component's folder and its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// Its path under the folder, plain names joined by `/`.
    pub path: String,
    /// What the check found.
    pub state: FileState,
}

impl Component {
    /// A component of `kind` with one or more files and the defaults for
    /// the rest: what tests and the list reader start from.
    pub fn new(id: impl Into<String>, title: impl Into<String>, files: Vec<FilePin>) -> Self {
        let id = id.into();
        Self {
            folder: id.clone(),
            id,
            title: title.into(),
            kind: Kind::Data,
            version: "0.1.0".to_owned(),
            license: "Apache-2.0 OR MIT".to_owned(),
            license_note: String::new(),
            credit: String::new(),
            features: Vec::new(),
            needs_tools: Vec::new(),
            needs_components: Vec::new(),
            protocol: None,
            min_diro: None,
            action: Action::Place,
            unpacked_size: 0,
            files,
            origin: Origin::Public,
        }
    }

    /// The release tag its files are under, in a source and in the
    /// project's releases: `<id>-<version>`.
    pub fn release(&self) -> String {
        format!("{}-{}", self.id, self.version)
    }

    /// Its files for the computer `target` names.
    pub fn files_for(&self, target: &str) -> Vec<&FilePin> {
        self.files.iter().filter(|f| f.applies_to(target)).collect()
    }

    /// Bytes to download for the computer `target` names.
    pub fn size_for(&self, target: &str) -> u64 {
        self.files_for(target).iter().map(|f| f.size).sum()
    }

    /// The size for a person: "4.2 MB".
    pub fn size_text(&self, target: &str) -> String {
        human_size(self.size_for(target))
    }

    /// The pinned file named `name`, for any computer.
    pub fn file(&self, name: &str) -> Option<&FilePin> {
        self.files.iter().find(|f| f.name == name)
    }

    /// Its folder under `components_dir`.
    pub fn dir_in(&self, components_dir: &Path) -> PathBuf {
        self.folder
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(components_dir.to_path_buf(), |path, part| path.join(part))
    }

    /// The staging folder beside its folder: `<folder>.partial`.
    pub fn staging_in(&self, components_dir: &Path) -> PathBuf {
        let dir = self.dir_in(components_dir);
        let leaf = self.folder.rsplit('/').next().unwrap_or(&self.id);
        dir.with_file_name(format!("{leaf}{STAGING_SUFFIX}"))
    }

    /// Checks every name in it: the id, the folder, each file's name and
    /// `exec` path; each hash, each size above zero; no two files of one
    /// computer with names that differ only by case. The list reader runs
    /// this on every component before any of it is used. The words say
    /// what is wrong, the name first.
    pub fn check(&self) -> Result<(), String> {
        if !is_plain_name(&self.id) {
            return Err(format!("the id {:?} is not a plain name", self.id));
        }
        if !is_plain_path(&self.folder) {
            return Err(format!(
                "{}: the folder {:?} is not plain names",
                self.id, self.folder
            ));
        }
        if self.title.trim().is_empty() || self.title.chars().any(char::is_control) {
            return Err(format!(
                "{}: the title is empty or has control characters",
                self.id
            ));
        }
        if self.files.is_empty() {
            return Err(format!("{}: it has no files", self.id));
        }
        let mut seen = BTreeSet::new();
        for file in &self.files {
            if !is_plain_name(&file.name)
                || file.name.ends_with(PART_SUFFIX)
                || file.name.ends_with(receipt::SUFFIX)
                || file.name == LOCK_NAME
            {
                return Err(format!(
                    "{}: the file name {:?} is not a plain name",
                    self.id, file.name
                ));
            }
            if file.size == 0 {
                return Err(format!("{}: {} has no size", self.id, file.name));
            }
            if !file.hash_is_well_formed() {
                return Err(format!(
                    "{}: {} has no 64-digit SHA-256",
                    self.id, file.name
                ));
            }
            if !is_plain_name(&file.target) {
                return Err(format!("{}: {} has an unknown target", self.id, file.name));
            }
            if let Some(exec) = &file.exec
                && !is_plain_path(exec)
            {
                return Err(format!(
                    "{}: the exec path {exec:?} is not plain names",
                    self.id
                ));
            }
            if let Some(url) = &file.url
                && !is_https(url)
            {
                return Err(format!(
                    "{}: {} has an address that is not https",
                    self.id, file.name
                ));
            }
            if !seen.insert((file.target.clone(), file.name.to_ascii_lowercase())) {
                return Err(format!(
                    "{}: two files named {} for one computer, by case",
                    self.id, file.name
                ));
            }
        }
        Ok(())
    }

    /// Whether it is installed in its folder under `components_dir`, for
    /// the computer `target` names, from sizes.
    pub fn status_in(&self, components_dir: &Path, target: &str) -> Status {
        let files = self.files_for(target);
        if files.is_empty() {
            return Status::NoBuild;
        }
        let dir = self.dir_in(components_dir);
        let mut missing = Vec::new();
        for file in &files {
            if let Some(receipt) = Receipt::matching(&dir, file) {
                for written in &receipt.files {
                    let ok = receipt::listed(&dir, &written.path).is_some_and(|p| {
                        fs::symlink_metadata(p)
                            .is_ok_and(|m| m.is_file() && m.len() == written.size)
                    });
                    if !ok {
                        return Status::Damaged(written.path.clone());
                    }
                }
                continue;
            }
            match fs::symlink_metadata(dir.join(&file.name)) {
                Ok(m) if m.is_file() && m.len() == file.size => {}
                Ok(_) => return Status::Damaged(file.name.clone()),
                Err(_) => missing.push(file.name.clone()),
            }
        }
        if missing.is_empty() {
            Status::Installed
        } else if missing.len() == files.len() {
            Status::NotInstalled
        } else {
            Status::Partial(missing)
        }
    }

    /// Checks each file by size and hash: each placed file against its pin,
    /// each file an archive wrote against its receipt.
    pub fn verify_in(&self, components_dir: &Path, target: &str) -> Vec<Checked> {
        let dir = self.dir_in(components_dir);
        let mut checked = Vec::new();
        for file in self.files_for(target) {
            if let Some(receipt) = Receipt::matching(&dir, file) {
                for written in &receipt.files {
                    let state = match receipt::listed(&dir, &written.path) {
                        Some(path) => check_one(&path, written.size, &written.sha256),
                        None => FileState::Missing,
                    };
                    checked.push(Checked {
                        path: written.path.clone(),
                        state,
                    });
                }
                continue;
            }
            checked.push(Checked {
                path: file.name.clone(),
                state: check_one(&dir.join(&file.name), file.size, &file.sha256),
            });
        }
        checked
    }

    /// What Remove would take away from `components_dir`, listed before
    /// anything is touched: the component's own files (each pinned file
    /// for any computer, its receipt and every file the receipt lists), the
    /// download's leftovers in the staging folder (`.part` files, their
    /// notes, staged files, the lock), and then the folders that leaves
    /// empty. Every other file in the folder is the owner's and stays.
    pub fn removal_in(&self, components_dir: &Path) -> Removal {
        let dir = self.dir_in(components_dir);
        let staging = self.staging_in(components_dir);
        let mut ours = BTreeSet::new();
        let mut folders = BTreeSet::new();
        for file in &self.files {
            if let Some(receipt) = Receipt::read(&dir, file) {
                for written in &receipt.files {
                    if let Some(path) = receipt::listed(&dir, &written.path) {
                        let mut up = path.parent();
                        while let Some(folder) = up.filter(|p| *p != dir && p.starts_with(&dir)) {
                            folders.insert(folder.to_path_buf());
                            up = folder.parent();
                        }
                        ours.insert(path);
                    }
                }
                ours.insert(receipt::path_for(&dir, file));
            }
            ours.insert(dir.join(&file.name));
            ours.insert(staging.join(&file.name));
            ours.insert(staging.join(format!("{}{PART_SUFFIX}", file.name)));
            ours.insert(staging.join(format!("{}{SIDECAR_SUFFIX}", file.name)));
        }
        ours.insert(staging.join(LOCK_NAME));
        let files: Vec<PathBuf> = ours
            .into_iter()
            .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
            .collect();
        let listed: BTreeSet<&PathBuf> = files.iter().collect();
        let mut kept = Vec::new();
        collect_files(&dir, &mut kept, 0);
        collect_files(&staging, &mut kept, 0);
        kept.retain(|p| !listed.contains(p));
        let mut folders: Vec<PathBuf> = folders.into_iter().collect();
        folders.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        folders.push(dir);
        folders.push(staging);
        folders.retain(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()));
        Removal {
            files,
            folders,
            kept,
        }
    }
}

/// Whether `url` is an absolute `https` address with a host.
pub fn is_https(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty() && !rest.starts_with('/') && !rest.contains(char::is_whitespace)
    })
}

fn check_one(path: &Path, size: u64, sha256: &str) -> FileState {
    match fs::symlink_metadata(path) {
        Err(_) => FileState::Missing,
        Ok(m) if !m.is_file() => FileState::Missing,
        Ok(m) if m.len() != size => FileState::WrongSize(m.len()),
        Ok(_) => match hash_file(path) {
            Ok(hash) if hash == sha256 => FileState::Good,
            _ => FileState::WrongHash,
        },
    }
}

/// Every regular file under `dir`, links not followed, at most 16 levels
/// deep and 100,000 files.
fn collect_files(dir: &Path, into: &mut Vec<PathBuf>, depth: usize) {
    if depth > 16 || into.len() > 100_000 {
        return;
    }
    let Ok(items) = fs::read_dir(dir) else {
        return;
    };
    for item in items.flatten() {
        let path = item.path();
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => collect_files(&path, into, depth + 1),
            Ok(_) => into.push(path),
            Err(_) => {}
        }
    }
}

/// What Remove takes away, listed before anything is touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// The component's own files that are there now, each a full path.
    pub files: Vec<PathBuf>,
    /// The folders removed afterwards if they are empty, deepest first.
    pub folders: Vec<PathBuf>,
    /// Files in its folders that are not the component's: they stay.
    pub kept: Vec<PathBuf>,
}

impl Removal {
    /// Whether there is nothing to remove.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The listing, one full path a line, in the order removed.
    pub fn lines(&self) -> Vec<String> {
        self.files.iter().map(|p| p.display().to_string()).collect()
    }

    /// The count in words, meaning first: "Removes 3 files. Yours stay."
    /// when the owner's files share the folder, else "Removes 3 files.".
    pub fn summary(&self) -> String {
        let removes = format!(
            "Removes {}.",
            count(self.files.len() as u64, "file", "files")
        );
        if self.kept.is_empty() {
            removes
        } else {
            format!("{removes} Yours stay.")
        }
    }

    /// Removes the listed files, by reference to this listing and nothing
    /// else: each must still be a regular file (a link or a folder that
    /// took its place is left alone), then each listed folder that is now
    /// empty. Returns how many files were removed.
    pub fn run(&self) -> Result<usize, ComponentError> {
        let mut removed = 0;
        for path in &self.files {
            match fs::symlink_metadata(path) {
                Ok(m) if m.is_file() => {
                    fs::remove_file(path).map_err(|e| ComponentError::io(path, e))?;
                    removed += 1;
                }
                _ => {}
            }
        }
        for folder in &self.folders {
            let empty = fs::symlink_metadata(folder).is_ok_and(|m| m.is_dir())
                && fs::read_dir(folder).is_ok_and(|mut d| d.next().is_none());
            if empty {
                fs::remove_dir(folder).map_err(|e| ComponentError::io(folder, e))?;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A component with one file, `a.bin`, holding `abc`.
    pub(crate) fn sample() -> Component {
        let mut c = Component::new(
            "sample",
            "Sample data",
            vec![FilePin::of_bytes("a.bin", b"abc")],
        );
        c.folder = "things/sample".into();
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pin::{UNIVERSAL_APPLE, sha256_hex};
    use crate::receipt::ReceiptFile;
    use tests_support::sample;

    #[test]
    fn status_verify_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let c = sample();
        let t = "x86_64-unknown-linux-gnu";
        let dir = c.dir_in(root);
        assert_eq!(dir, root.join("things").join("sample"));
        assert_eq!(
            c.staging_in(root),
            root.join("things").join("sample.partial")
        );
        assert_eq!(c.status_in(root, t), Status::NotInstalled);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.bin"), b"ab").unwrap();
        assert_eq!(c.status_in(root, t), Status::Damaged("a.bin".into()));
        assert_eq!(c.verify_in(root, t)[0].state, FileState::WrongSize(2));
        fs::write(dir.join("a.bin"), b"abd").unwrap();
        assert_eq!(c.status_in(root, t), Status::Installed);
        assert_eq!(c.verify_in(root, t)[0].state, FileState::WrongHash);
        fs::write(dir.join("a.bin"), b"abc").unwrap();
        assert_eq!(c.verify_in(root, t)[0].state, FileState::Good);
        // A file the owner added stays, and so does the folder holding it.
        fs::write(dir.join("mine.txt"), b"keep").unwrap();
        let removal = c.removal_in(root);
        assert_eq!(removal.files, vec![dir.join("a.bin")]);
        assert_eq!(removal.kept, vec![dir.join("mine.txt")]);
        assert_eq!(removal.summary(), "Removes 1 file. Yours stay.");
        assert_eq!(
            removal.lines(),
            vec![dir.join("a.bin").display().to_string()]
        );
        assert_eq!(removal.run().unwrap(), 1);
        assert!(dir.join("mine.txt").is_file());
        assert!(dir.is_dir());
        assert_eq!(c.status_in(root, t), Status::NotInstalled);
    }

    #[test]
    fn remove_takes_the_leftovers_and_the_empty_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let c = sample();
        let dir = c.dir_in(root);
        let staging = c.staging_in(root);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(dir.join("a.bin"), b"abc").unwrap();
        fs::write(staging.join("a.bin.part"), b"a").unwrap();
        fs::write(staging.join("a.bin.part.from"), b"address x").unwrap();
        fs::write(staging.join(LOCK_NAME), b"").unwrap();
        let removal = c.removal_in(root);
        assert_eq!(removal.files.len(), 4);
        assert!(removal.kept.is_empty());
        assert_eq!(removal.summary(), "Removes 4 files.");
        assert_eq!(removal.run().unwrap(), 4);
        assert!(!dir.exists());
        assert!(!staging.exists());
        assert!(
            root.join("things").is_dir(),
            "only the component's folders go"
        );
    }

    #[test]
    fn a_receipt_counts_and_remove_takes_exactly_what_it_lists() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let mut c = sample();
        c.action = Action::Unpack;
        c.files = vec![FilePin::of_bytes("pack.zip", b"zipbytes")];
        let t = "aarch64-apple-darwin";
        let dir = c.dir_in(root);
        fs::create_dir_all(dir.join("plugins").join("deep")).unwrap();
        fs::write(dir.join("plugins").join("deep").join("x.toml"), b"xx").unwrap();
        fs::write(dir.join("readme.txt"), b"r").unwrap();
        let receipt = Receipt {
            archive_sha256: c.files[0].sha256.clone(),
            files: vec![
                ReceiptFile {
                    path: "plugins/deep/x.toml".into(),
                    size: 2,
                    sha256: sha256_hex(b"xx"),
                },
                ReceiptFile {
                    path: "readme.txt".into(),
                    size: 1,
                    sha256: sha256_hex(b"r"),
                },
            ],
        };
        receipt.write(&dir, &c.files[0]).unwrap();
        assert_eq!(c.status_in(root, t), Status::Installed);
        assert!(
            c.verify_in(root, t)
                .iter()
                .all(|f| f.state == FileState::Good)
        );
        fs::write(dir.join("readme.txt"), b"R").unwrap();
        let bad = c.verify_in(root, t);
        assert_eq!(bad[1].state, FileState::WrongHash);
        fs::write(dir.join("plugins").join("mine.toml"), b"m").unwrap();
        let removal = c.removal_in(root);
        assert_eq!(removal.files.len(), 3, "{removal:?}");
        assert_eq!(removal.kept, vec![dir.join("plugins").join("mine.toml")]);
        removal.run().unwrap();
        assert!(dir.join("plugins").join("mine.toml").is_file());
        assert!(!dir.join("plugins").join("deep").exists());
        assert!(!dir.join("readme.txt").exists());
        assert_eq!(c.status_in(root, t), Status::NotInstalled);
        assert!(c.removal_in(root).is_empty());
    }

    #[test]
    fn files_are_chosen_for_this_computer() {
        let mut c = sample();
        let mut mac = FilePin::of_bytes("tool", b"mac");
        mac.target = UNIVERSAL_APPLE.into();
        let mut linux = FilePin::of_bytes("tool", b"linux!");
        linux.target = "x86_64-unknown-linux-gnu".into();
        c.files = vec![mac, linux];
        assert!(c.check().is_ok());
        assert_eq!(c.size_for("aarch64-apple-darwin"), 3);
        assert_eq!(c.size_for("x86_64-unknown-linux-gnu"), 6);
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            c.status_in(tmp.path(), "x86_64-pc-windows-msvc"),
            Status::NoBuild
        );
        assert_eq!(Status::NoBuild.words(), "no build for this computer");
        assert_eq!(c.release(), "sample-0.1.0");
    }

    #[test]
    fn names_are_checked() {
        let good = sample();
        assert!(good.check().is_ok());
        type Change = Box<dyn Fn(&mut Component)>;
        let cases: Vec<(&str, Change)> = vec![
            ("not a plain name", Box::new(|c| c.id = "a b".into())),
            (
                "not plain names",
                Box::new(|c| c.folder = "things/../up".into()),
            ),
            ("not plain names", Box::new(|c| c.folder = "/abs".into())),
            (
                "not a plain name",
                Box::new(|c| c.files[0].name = "x.part".into()),
            ),
            (
                "not a plain name",
                Box::new(|c| c.files[0].name = "CON".into()),
            ),
            ("no size", Box::new(|c| c.files[0].size = 0)),
            ("SHA-256", Box::new(|c| c.files[0].sha256 = "00".into())),
            (
                "not https",
                Box::new(|c| c.files[0].url = Some("http://x/a".into())),
            ),
            (
                "not plain names",
                Box::new(|c| c.files[0].exec = Some("../sh".into())),
            ),
            ("no files", Box::new(|c| c.files.clear())),
            ("title", Box::new(|c| c.title = " ".into())),
            (
                "by case",
                Box::new(|c| {
                    let mut twin = c.files[0].clone();
                    twin.name = "A.BIN".into();
                    c.files.push(twin);
                }),
            ),
        ];
        for (words, change) in cases {
            let mut c = sample();
            change(&mut c);
            let error = c.check().unwrap_err();
            assert!(error.contains(words), "{error} should say {words}");
        }
        assert!(is_https("https://example.invalid/a"));
        assert!(!is_https("https://"));
        assert!(!is_https("https:///x"));
    }
}
