//! The `unpack` action, and installing from a zip file or a folder.
//!
//! An archive was checked against its pin before it got here, so what it
//! holds is what the publisher made; its members still have to be plain
//! (R22 sections 6.1, 6.2 and 7.3). The archive is read through
//! `diroxide-archive` (zip, 7z, tar and its gzip and xz forms, all pure
//! Rust, with that crate's limits and name rules), and each member is
//! left out with its reason when it:
//!
//! - climbs out with `..`, or had a leading slash, a drive or a device
//!   prefix removed, or backslashes read as folders (the archive crate's
//!   name flags), or holds control, hidden or private-use characters;
//! - has a part that is not a plain name: a Windows device name (`CON`,
//!   `aux.txt`), a colon (an alternate data stream), a trailing dot;
//! - is a link or a special file (links are never made);
//! - differs only by case from a member already written;
//! - lies outside the folders `strip` removes.
//!
//! The ceiling is the component's `unpacked_size` and a small slack: an
//! archive that expands past it, or holds more than [`MAX_MEMBERS`]
//! members, is stopped, and every file it wrote is removed by the list
//! kept while writing. Nothing is written outside the component's folder,
//! and nothing through a link already in it. A receipt
//! ([`crate::receipt`]) takes the archive's place, with a hash per file.
//!
//! [`install_from`] does what a download does from a zip file or a folder
//! on this computer, for a computer without a network: each pinned file
//! is found by its name, or by its size and hash, and checked; the rest is
//! refused with the reason; then the same finish moves the files into
//! place and unpacks them.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use diroxide_archive::{Allowances, ArchiveFormat, ArchiveProvider, Limits, RecordKind};
use diroxide_core::Place;
use diroxide_core::names::spell_out;
use diroxide_vfs::{OpContext, VfsError};
use sha2::{Digest, Sha256};

use crate::component::{Action, Component, PART_SUFFIX};
use crate::download::{Claim, Outcome, finish_with, set_executable};
use crate::error::ComponentError;
use crate::pin::{FilePin, hash_file, is_plain_name};
use crate::receipt::{Receipt, ReceiptFile};

/// The most members one archive may unpack.
pub const MAX_MEMBERS: usize = 50_000;

/// The least slack over `unpacked_size` an unpack allows.
pub const MIN_SLACK: u64 = 64 << 10;

/// The reason given for a file in a zip or a folder to install from that
/// is none of the component's files: it is not used, and its being there
/// does not make the install partial.
pub const NOT_ITS_FILE: &str = "not one of the component's files";

/// What unpacking left out: each member, with the reason in words.
pub type LeftOut = Vec<(String, String)>;

/// Whether a pinned file is an archive the unpack action opens.
pub fn is_archive(name: &str) -> bool {
    ArchiveFormat::from_name(name).is_some()
}

/// The ceiling for a component: its `unpacked_size` and one percent, at
/// least [`MIN_SLACK`].
pub fn ceiling(component: &Component) -> u64 {
    let size = component.unpacked_size;
    size.saturating_add((size / 100).max(MIN_SLACK))
}

/// Unpacks each of `component`'s archives found in `dest` into `dest`,
/// writes its receipt, and removes the archive. A file that is not an
/// archive stays as it is. Returns the members left out, with the reason.
pub fn unpack_in(
    component: &Component,
    dest: &Path,
    ctx: &OpContext,
) -> Result<LeftOut, ComponentError> {
    unpack_with(component, dest, ctx, MAX_MEMBERS)
}

pub(crate) fn unpack_with(
    component: &Component,
    dest: &Path,
    ctx: &OpContext,
    max_members: usize,
) -> Result<LeftOut, ComponentError> {
    let mut left_out = Vec::new();
    let mut total = 0u64;
    let ceiling = ceiling(component);
    for pin in &component.files {
        let archive = dest.join(&pin.name);
        if !is_archive(&pin.name) || !fs::symlink_metadata(&archive).is_ok_and(|m| m.is_file()) {
            continue;
        }
        let mut written: Vec<ReceiptFile> = Vec::new();
        let result = unpack_one(
            &archive,
            pin,
            dest,
            ctx,
            &Budget {
                ceiling,
                max_members,
            },
            &mut total,
            &mut written,
            &mut left_out,
        );
        if let Err(error) = result {
            // Nothing half unpacked is left behind: what this archive wrote
            // is removed by the list kept while writing.
            for file in &written {
                if let Some(path) = crate::receipt::listed(dest, &file.path) {
                    let _ = fs::remove_file(path);
                }
            }
            remove_empty_folders(dest, &written);
            return Err(error);
        }
        Receipt {
            archive_sha256: pin.sha256.clone(),
            files: written,
        }
        .write(dest, pin)?;
        fs::remove_file(&archive).map_err(|e| ComponentError::io(&archive, e))?;
    }
    Ok(left_out)
}

struct Budget {
    ceiling: u64,
    max_members: usize,
}

fn unpack_error(archive: &Path, why: impl std::fmt::Display) -> ComponentError {
    let name = archive
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    ComponentError::Unpack(format!("{name}: {why}"))
}

#[allow(clippy::too_many_arguments)]
fn unpack_one(
    archive: &Path,
    pin: &FilePin,
    dest: &Path,
    ctx: &OpContext,
    budget: &Budget,
    total: &mut u64,
    written: &mut Vec<ReceiptFile>,
    left_out: &mut LeftOut,
) -> Result<(), ComponentError> {
    let place = Place::parse(&archive.to_string_lossy()).map_err(|e| unpack_error(archive, e))?;
    let file = File::open(archive).map_err(|e| ComponentError::io(archive, e))?;
    // The archive crate's own ceiling is a backstop a little above ours,
    // so the sentence that stops an archive is this module's.
    let limits = Limits {
        max_total_bytes: budget.ceiling.saturating_add(1 << 20),
        max_entries: budget.max_members,
        ..Limits::default()
    };
    let provider = ArchiveProvider::new(limits);
    let opened = provider
        .open_bytes(Box::new(file), &place, ctx, Allowances::default())
        .map_err(|e| unpack_error(archive, e))?;
    let summary = opened.index().summary().clone();
    if summary.members + summary.folders > budget.max_members as u64 {
        return Err(unpack_error(archive, "it holds too many files"));
    }
    for line in &summary.refused_lines {
        left_out.push((String::new(), line.trim_end_matches('.').to_owned()));
    }
    let more = summary
        .refused
        .saturating_sub(summary.refused_lines.len() as u64);
    if more > 0 {
        left_out.push((String::new(), format!("{more} more names were refused")));
    }
    let mut seen = BTreeSet::new();
    let mut stop: Option<ComponentError> = None;
    let mut visitor = |index: &diroxide_archive::Index,
                       id: u32,
                       reader: &mut dyn Read|
     -> Result<bool, VfsError> {
        let Some(record) = index.record(id).copied() else {
            return Ok(true);
        };
        // Shown to the owner: hidden characters spelled out.
        let raw = spell_out(&index.path(id)).into_owned();
        match record.kind {
            RecordKind::Folder => return Ok(true),
            RecordKind::Link | RecordKind::Special => {
                left_out.push((raw, "a link or a special file, not a file".to_owned()));
                return Ok(true);
            }
            RecordKind::File => {}
        }
        if !record.name_flags.is_empty() {
            left_out.push((raw, record.name_flags.words().join(", ")));
            return Ok(true);
        }
        let parts: Vec<&str> = raw.split('/').collect();
        let strip = pin.strip as usize;
        if parts.len() <= strip {
            left_out.push((
                raw.clone(),
                "outside the folder that is stripped".to_owned(),
            ));
            return Ok(true);
        }
        let parts = &parts[strip..];
        if let Some(bad) = parts.iter().find(|p| !is_plain_name(p)) {
            let why = if bad.contains(':') {
                "has a colon, which would make a hidden stream".to_owned()
            } else {
                format!("{bad} is not a plain name")
            };
            left_out.push((raw.clone(), why));
            return Ok(true);
        }
        let rel = parts.join("/");
        if !seen.insert(rel.to_ascii_lowercase()) {
            left_out.push((raw, "differs only by case from another file".to_owned()));
            return Ok(true);
        }
        match write_member(reader, dest, &rel, budget.ceiling, total) {
            Ok(file) => {
                written.push(file);
                Ok(true)
            }
            Err(Skip::LeftOut(why)) => {
                left_out.push((raw, why));
                Ok(true)
            }
            Err(Skip::Stop(error)) => {
                stop = Some(error);
                Ok(false)
            }
        }
    };
    let visited = provider.visit_opened(&opened, ctx, Allowances::default(), None, &mut visitor);
    if let Some(error) = stop {
        return Err(error);
    }
    visited.map_err(|e| unpack_error(archive, e))?;
    for file in written.iter() {
        if file_is_exec(pin, &file.path)
            && let Some(path) = crate::receipt::listed(dest, &file.path)
        {
            set_executable(&path);
        }
    }
    Ok(())
}

fn file_is_exec(pin: &FilePin, rel: &str) -> bool {
    pin.exec.as_deref() == Some(rel)
}

enum Skip {
    LeftOut(String),
    Stop(ComponentError),
}

/// Writes one member under `dest` at `rel` (plain parts), never through a
/// link, counting bytes against the ceiling; its hash is kept for the
/// receipt.
fn write_member(
    reader: &mut dyn Read,
    dest: &Path,
    rel: &str,
    ceiling: u64,
    total: &mut u64,
) -> Result<ReceiptFile, Skip> {
    let mut path = dest.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for (at, part) in parts.iter().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(Skip::LeftOut("a link is in the way".to_owned()));
            }
            Ok(m) if at + 1 < parts.len() && !m.is_dir() => {
                return Err(Skip::LeftOut("a file is where its folder goes".to_owned()));
            }
            Ok(m) if at + 1 == parts.len() && m.is_dir() => {
                return Err(Skip::LeftOut("a folder is where it goes".to_owned()));
            }
            Ok(_) => {}
            Err(_) if at + 1 < parts.len() => {
                fs::create_dir(&path).map_err(|e| Skip::Stop(ComponentError::io(&path, e)))?;
            }
            Err(_) => {}
        }
    }
    let mut out = File::create(&path).map_err(|e| Skip::Stop(ComponentError::io(&path, e)))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    let stop = |e: ComponentError| Skip::Stop(e);
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                drop(out);
                let _ = fs::remove_file(&path);
                return Err(stop(ComponentError::Unpack(format!(
                    "{rel} could not be read: {e}"
                ))));
            }
        };
        *total += n as u64;
        if *total > ceiling {
            drop(out);
            let _ = fs::remove_file(&path);
            return Err(stop(ComponentError::Unpack(
                "it unpacks to more than its list says".to_owned(),
            )));
        }
        size += n as u64;
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])
            .map_err(|e| stop(ComponentError::io(&path, e)))?;
    }
    out.flush()
        .map_err(|e| stop(ComponentError::io(&path, e)))?;
    let digest = hasher.finalize();
    Ok(ReceiptFile {
        path: rel.to_owned(),
        size,
        sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
    })
}

/// Removes the folders under `dest` that the written files' paths made, if
/// they are empty now, deepest first.
fn remove_empty_folders(dest: &Path, written: &[ReceiptFile]) {
    let mut folders = BTreeSet::new();
    for file in written {
        if let Some(path) = crate::receipt::listed(dest, &file.path) {
            let mut up = path.parent();
            while let Some(folder) = up.filter(|p| *p != dest && p.starts_with(dest)) {
                folders.insert(folder.to_path_buf());
                up = folder.parent();
            }
        }
    }
    let mut folders: Vec<PathBuf> = folders.into_iter().collect();
    folders.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for folder in folders {
        let _ = fs::remove_dir(folder);
    }
}

/// Where each pinned file was found for an install from a file.
enum Found {
    /// A file on disk that matches.
    File(PathBuf),
    /// A member of an archive, extracted to the staging folder and checked.
    Staged,
}

/// Installs `component` from `from`, a zip file or a folder on this
/// computer, into its folder under `components_dir`, for the computer
/// `target` names. Each pinned file is found by name or by size and hash
/// and checked; anything else in the zip, or at the top of the folder, is
/// left out with the reason. When a pinned file is not there, nothing is
/// installed and the error names it.
pub fn install_from(
    component: &Component,
    from: &Path,
    components_dir: &Path,
    target: &str,
    ctx: &OpContext,
) -> Result<Outcome, ComponentError> {
    component.check().map_err(ComponentError::Manifest)?;
    let files = component.files_for(target);
    if files.is_empty() {
        return Err(ComponentError::NoBuild(component.id.clone()));
    }
    let meta = fs::metadata(from).map_err(|e| ComponentError::io(from, e))?;
    let dest = component.dir_in(components_dir);
    let staging = component.staging_in(components_dir);
    let mut claim = Claim::take(&component.id, &dest, staging.clone())?;
    let mut outcome = Outcome::default();
    let mut found: Vec<(&FilePin, Found)> = Vec::new();
    let mut used: BTreeSet<PathBuf> = BTreeSet::new();
    let mut missing = Vec::new();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if meta.is_dir() {
        for folder in [from.to_path_buf(), from.join(component.release())] {
            if let Ok(items) = fs::read_dir(&folder) {
                let mut here: Vec<PathBuf> = items.flatten().map(|i| i.path()).collect();
                here.sort();
                candidates.extend(
                    here.into_iter()
                        .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_file())),
                );
            }
        }
    } else {
        candidates.push(from.to_path_buf());
    }
    for pin in &files {
        if pin.matches_file(&dest.join(&pin.name)) || Receipt::matching(&dest, pin).is_some() {
            outcome.kept.push(pin.name.clone());
            continue;
        }
        let by_name = candidates
            .iter()
            .find(|p| p.file_name().is_some_and(|n| n == pin.name.as_str()) && pin.matches_file(p));
        let by_hash = || {
            candidates
                .iter()
                .find(|p| !used.contains(*p) && pin.size_matches(p) && pin.matches_file(p))
        };
        if let Some(path) = by_name.or_else(by_hash).cloned() {
            used.insert(path.clone());
            found.push((pin, Found::File(path)));
            continue;
        }
        missing.push(*pin);
    }
    // A zip that is not itself a pinned file is a container: its members
    // are searched for the pins still missing.
    if !meta.is_dir()
        && !missing.is_empty()
        && used.is_empty()
        && is_archive(&from.to_string_lossy())
    {
        claim.stage(&component.id)?;
        let left = extract_pins(from, &missing, &staging, ctx, &mut outcome.left_out)?;
        let staged: Vec<&FilePin> = missing
            .iter()
            .copied()
            .filter(|pin| !left.iter().any(|l| l.name == pin.name))
            .collect();
        for pin in staged {
            found.push((pin, Found::Staged));
        }
        missing = left;
    } else if !meta.is_dir() && used.is_empty() {
        outcome
            .left_out
            .push((display_name(from), NOT_ITS_FILE.to_owned()));
    }
    if meta.is_dir() {
        for path in candidates.iter().filter(|p| !used.contains(*p)).take(50) {
            outcome
                .left_out
                .push((display_name(path), NOT_ITS_FILE.to_owned()));
        }
    }
    if let Some(pin) = missing.first() {
        // Nothing is installed; only what this install staged goes.
        for (pin, how) in &found {
            if matches!(how, Found::Staged) {
                let _ = fs::remove_file(staging.join(&pin.name));
            }
        }
        return Err(ComponentError::Fetch {
            file: pin.name.clone(),
            reason: format!("it was not found in {}", display_name(from)),
        });
    }
    claim.stage(&component.id)?;
    for (pin, how) in &found {
        if let Found::File(path) = how {
            let part = staging.join(format!("{}{PART_SUFFIX}", pin.name));
            fs::copy(path, &part).map_err(|e| ComponentError::io(&part, e))?;
            if !pin.matches_file(&part) {
                let _ = fs::remove_file(&part);
                return Err(ComponentError::Hash {
                    file: pin.name.clone(),
                });
            }
            let staged = staging.join(&pin.name);
            fs::rename(&part, &staged).map_err(|e| ComponentError::io(&staged, e))?;
        }
        outcome.fetched.push(pin.name.clone());
    }
    let mut left = finish_with(component, &dest, &staging, &outcome.fetched, ctx)?;
    outcome.left_out.append(&mut left);
    drop(claim);
    Ok(outcome)
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Extracts the members of the archive at `from` that are pinned files of
/// `wanted` (by name, or by size and hash) into `staging`, each checked.
/// Returns the pins not found; members that are not pinned files are left
/// out with the reason.
fn extract_pins<'a>(
    from: &Path,
    wanted: &[&'a FilePin],
    staging: &Path,
    ctx: &OpContext,
    left_out: &mut LeftOut,
) -> Result<Vec<&'a FilePin>, ComponentError> {
    let place = Place::parse(&from.to_string_lossy()).map_err(|e| unpack_error(from, e))?;
    let file = File::open(from).map_err(|e| ComponentError::io(from, e))?;
    let limits = Limits {
        max_entries: MAX_MEMBERS,
        ..Limits::default()
    };
    let provider = ArchiveProvider::new(limits);
    let opened = provider
        .open_bytes(Box::new(file), &place, ctx, Allowances::default())
        .map_err(|e| unpack_error(from, e))?;
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut failure: Option<ComponentError> = None;
    let mut visitor = |index: &diroxide_archive::Index,
                       id: u32,
                       reader: &mut dyn Read|
     -> Result<bool, VfsError> {
        let Some(record) = index.record(id).copied() else {
            return Ok(true);
        };
        if record.kind != RecordKind::File {
            return Ok(true);
        }
        let path = spell_out(&index.path(id)).into_owned();
        let name = index.name(id).to_owned();
        let candidates: Vec<&&FilePin> = wanted
            .iter()
            .filter(|p| !found.contains(&p.name) && (p.name == name || p.size == record.size))
            .collect();
        if candidates.is_empty() {
            left_out.push((path, NOT_ITS_FILE.to_owned()));
            return Ok(true);
        }
        let part = staging.join(format!("extract{PART_SUFFIX}"));
        let copied = File::create(&part).and_then(|mut out| {
            std::io::copy(&mut reader.take(record.size.saturating_add(1)), &mut out)
        });
        if let Err(e) = copied {
            failure = Some(ComponentError::io(&part, e));
            return Ok(false);
        }
        let hash = hash_file(&part).unwrap_or_default();
        let size = fs::metadata(&part).map_or(0, |m| m.len());
        match candidates
            .iter()
            .find(|p| p.sha256 == hash && p.size == size)
        {
            Some(pin) => {
                let staged = staging.join(&pin.name);
                if let Err(e) = fs::rename(&part, &staged) {
                    failure = Some(ComponentError::io(&staged, e));
                    return Ok(false);
                }
                found.insert(pin.name.clone());
            }
            None => {
                let _ = fs::remove_file(&part);
                left_out.push((path, "does not match its pin".to_owned()));
            }
        }
        Ok(true)
    };
    let visited = provider.visit_opened(&opened, ctx, Allowances::default(), None, &mut visitor);
    if let Some(error) = failure {
        return Err(error);
    }
    visited.map_err(|e| unpack_error(from, e))?;
    Ok(wanted
        .iter()
        .copied()
        .filter(|p| !found.contains(&p.name))
        .collect())
}

/// Whether a component unpacks: its action, and at least one archive.
pub fn unpacks(component: &Component) -> bool {
    component.action == Action::Unpack && component.files.iter().any(|f| is_archive(&f.name))
}
