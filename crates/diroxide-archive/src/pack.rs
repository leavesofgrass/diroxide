//! The pack plan (plan commits 32 to 34, research/archives.md 8.2): what
//! goes in, under which names, how it is compressed and locked, where the
//! new archive lands, all decided before a byte is read.
//!
//! The rows are a short list of labeled settings, each with its meaning
//! in its first 40 cells; a row that does not apply says why instead of
//! vanishing ("Solid: not for zip"). The question starts on No: "Pack 12
//! items into reports.zip? y or n".

use std::collections::HashSet;

use diroxide_core::format::{count, human_size};
use diroxide_core::names::spell_out;
use diroxide_core::pattern::{Glob, Pattern};
use diroxide_core::{
    ConflictPolicy, Entry, EntryKind, JobId, JobKind, JobRequest, Message, NameCase, Place,
};
use diroxide_vfs::{OpContext, Registry, VfsError};

use crate::error::ArchiveError;
use crate::format::ArchiveFormat;
use crate::request::{CompressionLevel, PackRequest, StorePaths};
use crate::unpack::{Room, drive_of};
use crate::write::{ItemKind, WriteItem};

/// The deepest a pack walks into a folder.
const MAX_DEPTH: usize = 256;

/// The start of every temporary archive's name: the new archive is
/// written as `.diroxide-` and the job's number and `.tmp` beside its
/// destination, tested, then renamed into place; the next start sweeps
/// what a crash left by this name only ([`is_pack_temp`]).
pub const PACK_TEMP_PREFIX: &str = ".diroxide-";

/// The end of every temporary archive's name.
pub const PACK_TEMP_SUFFIX: &str = ".tmp";

/// Whether a name is a pack runner's temporary archive.
pub fn is_pack_temp(name: &str) -> bool {
    name.strip_prefix(PACK_TEMP_PREFIX)
        .and_then(|rest| rest.strip_suffix(PACK_TEMP_SUFFIX))
        .is_some_and(|middle| {
            !middle.is_empty()
                && middle
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// One member of the new archive and the entry it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackItem {
    /// The entry packed.
    pub source: Place,
    /// Its path in the archive.
    pub name: String,
    /// What it is: a file, a folder or a link.
    pub kind: EntryKind,
    /// Its size, as listed.
    pub size: u64,
}

/// What a pack will do, decided before it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackPlan {
    /// The job.
    pub job: JobId,
    /// The items chosen, as the user chose them.
    pub sources: Vec<Place>,
    /// The new archive.
    pub destination: Place,
    /// The temporary archive beside it, renamed into place once tested.
    pub temp: Place,
    /// The format.
    pub format: ArchiveFormat,
    /// How hard to compress.
    pub level: CompressionLevel,
    /// A solid 7z.
    pub solid: bool,
    /// The largest solid block, in bytes.
    pub block_bytes: u64,
    /// Whether a password locks it (the password itself travels beside
    /// the plan, never in it).
    pub locked: bool,
    /// Whether a 7z hides its names too.
    pub encrypt_names: bool,
    /// Which part of each path is stored.
    pub store_paths: StorePaths,
    /// Whether tar keeps owner names.
    pub keep_owner: bool,
    /// Threads for the encoders that use them.
    pub threads: u32,
    /// Whether the new archive is read back and checked.
    pub test_after: bool,
    /// What to do when the archive's name is taken.
    pub conflicts: ConflictPolicy,
    /// The exclude patterns asked for, as typed.
    pub exclude: Vec<String>,
    /// The members, in the order they are written.
    pub items: Vec<PackItem>,
    /// What was left out, with the reason in words.
    pub left_out: Vec<(Place, String)>,
    /// The counts: files, folders, links, and the bytes read.
    pub files: u64,
    /// Folders stored.
    pub folders: u64,
    /// Links stored.
    pub links: u64,
    /// The bytes the files hold.
    pub bytes: u64,
    /// The room on the destination's drive.
    pub room: Room,
    pub(crate) write_items: Vec<WriteItem>,
}

/// The format's short name, as the rows say it: "zip", "tar.gz".
pub fn format_word(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::SevenZ => "7z",
        ArchiveFormat::Tar => "tar",
        ArchiveFormat::TarGz => "tar.gz",
        ArchiveFormat::TarXz => "tar.xz",
        ArchiveFormat::Xz => "xz",
    }
}

/// The format and level row: "Format: zip, level normal (6 of 9)", or
/// "Format: tar, no compression".
pub fn format_row(format: ArchiveFormat, level: CompressionLevel) -> String {
    match level.number_for(format) {
        Some(number) => format!(
            "Format: {}, level {} ({number} of 9)",
            format_word(format),
            level.word()
        ),
        None => format!("Format: {}, no compression", format_word(format)),
    }
}

/// Whether a format can carry a password.
pub fn takes_password(format: ArchiveFormat) -> bool {
    matches!(format, ArchiveFormat::Zip | ArchiveFormat::SevenZ)
}

/// The excludes, compiled: a pattern with a `/` inside matches the stored
/// path, any other matches the name (with `/` at an end for folders).
struct Excludes {
    names: Vec<Pattern>,
    paths: Vec<Glob>,
}

impl Excludes {
    fn new(patterns: &[String]) -> Result<Self, VfsError> {
        let mut names = Vec::new();
        let mut paths = Vec::new();
        for text in patterns {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let inner = trimmed.trim_matches('/');
            let refused = |line: &str| VfsError::UnsupportedFeature {
                what: format!("the exclude pattern {trimmed}: {}", line.to_lowercase()),
            };
            if inner.contains('/') {
                paths.push(Glob::for_path(inner, NameCase::Ignore).map_err(|e| refused(e.line()))?);
            } else {
                names.push(Pattern::parse(trimmed).map_err(|e| refused(e.line()))?);
            }
        }
        Ok(Self { names, paths })
    }

    fn leaves_out(&self, name: &str, path: &str, folder: bool) -> bool {
        self.names.iter().any(|p| p.matches(name, folder))
            || self.paths.iter().any(|g| g.is_match(path))
    }
}

/// The path stored for an entry: relative to the folder its source is in,
/// the whole path without its drive or root, or the name alone.
fn stored_path(store: StorePaths, source: &Place, relative: &str, leaf: &str) -> String {
    match store {
        StorePaths::Relative => relative.to_owned(),
        StorePaths::NamesOnly => leaf.to_owned(),
        StorePaths::Full => {
            let base = source
                .parent()
                .and_then(|p| p.local_path().map(str::to_owned));
            let base = base.unwrap_or_default().replace('\\', "/");
            let base = match base.as_bytes().get(1) {
                Some(b':') => base[2..].to_owned(),
                _ => base,
            };
            let base = base.trim_matches('/');
            if base.is_empty() {
                relative.to_owned()
            } else {
                format!("{base}/{relative}")
            }
        }
    }
}

/// Plans a pack: checks the format's limits (one file for xz, no password
/// for tar), walks every source through its provider without following
/// links, applies the excludes, leaves out the archive itself and what
/// cannot be packed, and checks the destination. Nothing is read but the
/// listings, and nothing is written.
pub fn plan_pack(
    registry: &Registry,
    request: &PackRequest,
    job: JobId,
    ctx: &OpContext,
) -> Result<PackPlan, ArchiveError> {
    let format = request.format;
    if !matches!(
        format,
        ArchiveFormat::Zip
            | ArchiveFormat::SevenZ
            | ArchiveFormat::Tar
            | ArchiveFormat::TarGz
            | ArchiveFormat::TarXz
            | ArchiveFormat::Xz
    ) {
        return Err(VfsError::unsupported("archive", "pack to this format").into());
    }
    if request.destination.archive().is_some() {
        return Err(crate::words::read_only_error().into());
    }
    let archive_name = request
        .destination
        .name()
        .map_or_else(|| request.destination.to_string(), str::to_owned);
    let refuse = |why: &str| ArchiveError::Refused(format!("Could not pack {archive_name}: {why}"));
    let locked = request.password.as_ref().is_some_and(|p| !p.is_empty());
    if locked && !takes_password(format) {
        return Err(refuse(&format!(
            "{} keeps no password. Use 7z or zip.",
            format_word(format)
        )));
    }
    let folder = request.destination.parent().ok_or(VfsError::NotFound)?;
    if request.destination.name().is_none() {
        return Err(VfsError::NotFound.into());
    }
    let provider = registry.resolve(&request.destination)?;
    if !provider.capabilities(&folder).writable {
        return Err(VfsError::PermissionDenied.into());
    }
    let excludes = Excludes::new(&request.exclude)?;
    let mut plan_items = Vec::new();
    let mut write_items = Vec::new();
    let mut left_out = Vec::new();
    let mut names = HashSet::new();
    for source in &request.sources {
        ctx.check()?;
        let source_provider = registry.resolve(source)?;
        let entry = source_provider.stat(source, ctx)?;
        let leaf = source.name().unwrap_or("item").to_owned();
        let mut stack = vec![(source.clone(), entry, leaf.clone(), 0usize)];
        while let Some((place, entry, relative, depth)) = stack.pop() {
            ctx.check()?;
            if place == request.destination {
                left_out.push((place, "the archive being made".to_owned()));
                continue;
            }
            let entry_name = place.name().unwrap_or(&leaf).to_owned();
            if is_pack_temp(&entry_name) || diroxide_vfs::names::is_unfinished(&entry_name) {
                continue;
            }
            let folder_entry = entry.kind == EntryKind::Folder && entry.link.is_none();
            if excludes.leaves_out(&entry_name, &relative, folder_entry) {
                left_out.push((place, "left out by a pattern".to_owned()));
                continue;
            }
            let stored = stored_path(request.store_paths, source, &relative, &entry_name);
            let kind = match entry.kind {
                EntryKind::Folder if entry.link.is_some() => EntryKind::Link,
                EntryKind::File | EntryKind::Archive => EntryKind::File,
                other => other,
            };
            match kind {
                EntryKind::Folder => {
                    if depth >= MAX_DEPTH {
                        left_out.push((place, "too deep to pack".to_owned()));
                        continue;
                    }
                    let mut children = source_provider.list(&place, ctx)?;
                    // Written in name order, so the archive's order is the
                    // same on every platform.
                    children.sort_by(|a, b| b.name.cmp(&a.name));
                    for child in children {
                        let Ok(child_place) = place.join(&child.name) else {
                            continue;
                        };
                        let child_relative = format!("{relative}/{}", child.name);
                        stack.push((child_place, child, child_relative, depth + 1));
                    }
                    if request.store_paths == StorePaths::NamesOnly {
                        continue;
                    }
                }
                EntryKind::Link if entry.link.is_none() => {
                    left_out.push((place, "a link whose target cannot be read".to_owned()));
                    continue;
                }
                EntryKind::Special => {
                    left_out.push((place, "device or special entry, not packed".to_owned()));
                    continue;
                }
                _ => {}
            }
            if format == ArchiveFormat::Xz && kind != EntryKind::File {
                return Err(refuse("xz holds one file. Use tar.xz."));
            }
            if !names.insert(stored.to_lowercase()) {
                left_out.push((place, format!("{stored} is in the archive already")));
                continue;
            }
            let item_kind = match kind {
                EntryKind::Folder => ItemKind::Folder,
                EntryKind::Link => ItemKind::Link(
                    entry
                        .link
                        .as_ref()
                        .map(|l| l.target.clone())
                        .unwrap_or_default(),
                ),
                _ => ItemKind::File,
            };
            let size = if kind == EntryKind::File {
                entry.size.unwrap_or(0)
            } else {
                0
            };
            write_items.push(write_item(
                &stored,
                item_kind,
                size,
                &entry,
                request.keep_owner,
            ));
            plan_items.push(PackItem {
                source: place,
                name: stored,
                kind,
                size,
            });
        }
    }
    if format == ArchiveFormat::Xz && plan_items.len() != 1 {
        return Err(refuse("xz holds one file. Use tar.xz."));
    }
    let count_of = |kind: EntryKind| plan_items.iter().filter(|i| i.kind == kind).count() as u64;
    let bytes = plan_items.iter().map(|i| i.size).sum();
    let free = provider
        .space(&folder, ctx)
        .ok()
        .and_then(|space| space.free_to_user);
    let threads = request.threads.map_or_else(
        || std::thread::available_parallelism().map_or(1, |n| n.get().min(8)),
        |n| n.clamp(1, 64),
    ) as u32;
    Ok(PackPlan {
        job,
        sources: request.sources.clone(),
        temp: folder
            .join(&format!(
                "{PACK_TEMP_PREFIX}{}{PACK_TEMP_SUFFIX}",
                job.get()
            ))
            .map_err(VfsError::from)?,
        destination: request.destination.clone(),
        format,
        level: request.level,
        solid: request.solid && format == ArchiveFormat::SevenZ,
        block_bytes: request
            .solid_block_bytes
            .unwrap_or(crate::limits::GIB)
            .max(1 << 16),
        locked,
        encrypt_names: locked && request.encrypt_names && format == ArchiveFormat::SevenZ,
        store_paths: request.store_paths,
        keep_owner: request.keep_owner,
        threads,
        test_after: request.test_after,
        conflicts: request.conflicts,
        exclude: request.exclude.clone(),
        files: count_of(EntryKind::File),
        folders: count_of(EntryKind::Folder),
        links: count_of(EntryKind::Link),
        bytes,
        items: plan_items,
        left_out,
        room: Room {
            needed: bytes,
            free,
            drive: drive_of(&folder),
        },
        write_items,
    })
}

/// What the writer needs to know of one entry.
fn write_item(name: &str, kind: ItemKind, size: u64, entry: &Entry, keep_owner: bool) -> WriteItem {
    let mode = match entry.attributes {
        Some(diroxide_core::Attributes::Unix(mode)) => Some(mode & 0o7777),
        _ => None,
    };
    let owner = if keep_owner {
        entry
            .owner
            .clone()
            .map(|owner| (owner, entry.group.clone().unwrap_or_default()))
    } else {
        None
    };
    WriteItem {
        name: name.to_owned(),
        kind,
        size,
        modified: entry.modified,
        mode,
        owner,
    }
}

impl PackPlan {
    /// The archive's name.
    pub fn archive_name(&self) -> String {
        self.destination
            .name()
            .map_or_else(|| self.destination.to_string(), str::to_owned)
    }

    /// The plan in one line, meaning first: "Pack 12 items into
    /// reports.zip".
    pub fn summary(&self) -> String {
        format!(
            "Pack {} into {}",
            count(self.sources.len() as u64, "item", "items"),
            spell_out(&self.archive_name())
        )
    }

    /// The question before the start, starting on No: "Pack 12 items into
    /// reports.zip? y or n".
    pub fn question(&self) -> Message {
        Message::question(&self.summary())
    }

    /// The rows, with the destination said as it is.
    pub fn rows(&self) -> Vec<String> {
        self.rows_for(false)
    }

    /// The rows, each a labeled setting with its meaning first; with
    /// `other_panel`, the destination row says "the other panel" first, as
    /// the orthodox target rule picks it.
    pub fn rows_for(&self, other_panel: bool) -> Vec<String> {
        let folder = self
            .destination
            .parent()
            .map_or_else(String::new, |p| spell_out(&p.to_string()).into_owned());
        let mut rows = vec![
            self.summary(),
            format_row(self.format, self.level),
            if other_panel {
                format!("Where: the other panel, {folder}")
            } else {
                format!("Where: {folder}")
            },
            self.password_row(),
            self.solid_row(),
            self.reads_row(),
            match self.store_paths {
                StorePaths::Relative => "Paths: relative to their folder".to_owned(),
                StorePaths::Full => "Paths: full, without the drive".to_owned(),
                StorePaths::NamesOnly => "Paths: names only".to_owned(),
            },
        ];
        if matches!(
            self.format,
            ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarXz
        ) {
            rows.push(if self.keep_owner {
                "Owner names: kept".to_owned()
            } else {
                "Owner names: left out".to_owned()
            });
        }
        if !self.exclude.is_empty() {
            rows.push(format!("Leaves out: {}", self.exclude.join(", ")));
        }
        rows.push(if self.test_after {
            "Test after packing: on".to_owned()
        } else {
            "Test after packing: off".to_owned()
        });
        for (place, reason) in &self.left_out {
            let name = place
                .name()
                .map_or_else(|| place.to_string(), str::to_owned);
            rows.push(crate::unpack::skipped_line(&name, reason));
        }
        rows
    }

    fn password_row(&self) -> String {
        match (self.format, self.locked) {
            (ArchiveFormat::Zip, false) => "Password: none (zip names stay visible)".to_owned(),
            (ArchiveFormat::Zip, true) => "Password: AES-256 (zip names visible)".to_owned(),
            (ArchiveFormat::SevenZ, false) => "Password: none".to_owned(),
            (ArchiveFormat::SevenZ, true) if self.encrypt_names => {
                "Password: AES-256, names hidden".to_owned()
            }
            (ArchiveFormat::SevenZ, true) => "Password: AES-256, names visible".to_owned(),
            (format, _) => format!("Password: not for {}; use 7z", format_word(format)),
        }
    }

    fn solid_row(&self) -> String {
        match self.format {
            ArchiveFormat::SevenZ if self.solid => format!(
                "Solid: on, blocks up to {}",
                crate::limits::limit_words(self.block_bytes)
            ),
            ArchiveFormat::SevenZ => "Solid: off".to_owned(),
            format => format!("Solid: not for {}", format_word(format)),
        }
    }

    fn reads_row(&self) -> String {
        let reads = format!(
            "Reads {}, {}",
            count(self.files, "file", "files"),
            human_size(self.bytes)
        );
        match self.room.free_words() {
            Some(free) => format!("{reads}. {free}"),
            None => format!("{reads}."),
        }
    }

    /// The job request the app keeps in its jobs list and journal for this
    /// pack: the sources, the folder, the archive's name and the policy.
    /// It holds no password.
    pub fn job_request(&self) -> JobRequest {
        let mut request = JobRequest::new(self.job, JobKind::Pack, self.sources.clone())
            .named(self.archive_name());
        if let Some(folder) = self.destination.parent() {
            request = request.to(folder);
        }
        request.conflicts = self.conflicts;
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_name_the_format_and_level_in_words() {
        assert_eq!(
            format_row(ArchiveFormat::Zip, CompressionLevel::Normal),
            "Format: zip, level normal (6 of 9)"
        );
        assert_eq!(
            format_row(ArchiveFormat::Tar, CompressionLevel::Normal),
            "Format: tar, no compression"
        );
        assert_eq!(
            format_row(ArchiveFormat::SevenZ, CompressionLevel::Maximum),
            "Format: 7z, level maximum (8 of 9)"
        );
    }

    #[test]
    fn temporary_names_are_plain_and_known() {
        assert!(is_pack_temp(".diroxide-41.tmp"));
        assert!(!is_pack_temp(".diroxide-.tmp"));
        assert!(!is_pack_temp(".diroxide-../x.tmp"));
        assert!(!is_pack_temp("notes.tmp"));
    }
}
