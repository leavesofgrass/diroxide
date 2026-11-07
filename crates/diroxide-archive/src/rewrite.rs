//! Editing inside a zip (alpha 4, R30 commit 82, R29 6.2, the owner's
//! answer 114): Delete, Rename and Make folder inside a zip, by
//! composition. Nothing in an archive is ever changed in place. A rewrite
//! is a pack job whose members are the old archive's other members, read
//! by their records (never by place, so a deep member keeps its path and
//! a name a place cannot hold is kept too), written under the pack
//! runner's temporary name beside the archive, tested, and only then put
//! in its place, the old archive to the trash, so Undo is the trash.
//!
//! What a rewrite keeps: every member's path, bytes, time and mode, the
//! members' order, links as links, and the password (one prompt for the
//! job, used to read the old members and to lock the new ones with
//! AES-256). What it loses, said in the plan's rows before the question:
//! the zip's comments and extra fields, the compression choices (every
//! file is compressed again at the normal level), older copies of a
//! repeated name, names the reader refused, and special entries.
//!
//! Zip only in this alpha: 7z and tar are refused in words ([`ZIP_ONLY`]),
//! because a solid 7z or a compressed tar must be recompressed whole. The
//! code is the same for them; [`REWRITTEN`] is the one line that would
//! enable them.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;

use diroxide_core::format::{count, human_size};
use diroxide_core::names::spell_out;
use diroxide_core::{
    ConflictPolicy, EntryKind, JobId, JobKind, Message, Place, Secret, SecretKind, SecretRequest,
    Timestamp,
};
use diroxide_ops::JobReport;
use diroxide_vfs::{OpContext, SecretSource, VfsError};

use crate::error::ArchiveError;
use crate::format::ArchiveFormat;
use crate::guard::Allowances;
use crate::index::{ContainerKey, Index, Locator, ROOT, RecordFlags, RecordKind};
use crate::job::JobEnv;
use crate::names::normalize_member_name;
use crate::pack::{PACK_TEMP_PREFIX, PACK_TEMP_SUFFIX, PackItem, PackPlan};
use crate::provider::{ArchiveProvider, Opened};
use crate::request::{CompressionLevel, StorePaths};
use crate::run_pack::run_pack;
use crate::secrets::JobSecrets;
use crate::unpack::{Room, drive_of};
use crate::write::{Contents, ItemKind, WriteItem};

/// The formats a rewrite may change: zip only (the owner's answer 114).
/// Adding 7z or the tars here is the one line that enables them; their
/// cost is time, a solid block or a compressed tar recompressed whole.
pub const REWRITTEN: &[ArchiveFormat] = &[ArchiveFormat::Zip];

/// The refusal for any other format.
pub const ZIP_ONLY: &str = "Zip only: 7z and tar are not rewritten";

/// The refusal for an archive inside an archive.
pub const NESTED_REFUSAL: &str = "Nested archives are not rewritten";

/// The refusal for a split archive (`backup.zip.001`).
pub const SPLIT_REFUSAL: &str = "Split archives are not rewritten";

/// The fidelity note every rewrite carries: what a zip keeps outside its
/// members' bytes.
pub const COMMENTS_LOST: &str = "Zip comments and extra fields are lost";

/// The fidelity note on compression.
pub const COMPRESSION_REDONE: &str = "Compression choices are not kept";

/// The note of a locked archive: one prompt, for reading and writing.
pub const PASSWORD_ONCE: &str = "Password: asked once, kept for the job";

/// The note of an archive with ZipCrypto members.
pub const ZIPCRYPTO_UPGRADED: &str = "ZipCrypto members become AES-256";

/// The row when the old archive goes to the trash.
pub const OLD_TO_TRASH: &str = "Old copy: to the trash, Undo restores";

/// The row when the place has no trash.
pub const OLD_NOT_KEPT: &str = "Old copy: not kept, no trash here";

/// Where a rewrite's members come from: the old archive's records, by
/// number, and the identity of the archive the plan was made from, so a
/// run against a changed archive stops before it writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FromIndex {
    /// The archive rewritten.
    pub(crate) archive: Place,
    /// Its identity when the plan was made: place, size and time.
    pub(crate) key: ContainerKey,
    /// Each item's record in the old archive, by the item's number; `None`
    /// for a folder the rewrite makes.
    pub(crate) records: Vec<Option<u32>>,
}

/// The members' bytes of a rewrite, read by record from the old archive
/// through the limiter chain: the zip's own CRC is checked at the end of
/// each member, so a damaged member stops the job before the old archive
/// is touched.
pub(crate) struct IndexContents<'a> {
    archives: &'a ArchiveProvider,
    opened: Arc<Opened>,
    records: &'a [Option<u32>],
    ctx: &'a OpContext,
}

impl<'a> IndexContents<'a> {
    /// Opens the old archive for a run, and refuses one that changed since
    /// the plan was made: "it changed since it was listed".
    pub(crate) fn open(
        archives: &'a ArchiveProvider,
        source: &'a FromIndex,
        ctx: &'a OpContext,
    ) -> Result<Self, VfsError> {
        let opened = archives
            .open_archive(&source.archive, ctx, Allowances::all())
            .map_err(ArchiveError::into_vfs)?;
        if opened.key() != &source.key {
            return Err(VfsError::Changed);
        }
        Ok(Self {
            archives,
            opened,
            records: &source.records,
            ctx,
        })
    }
}

impl Contents for IndexContents<'_> {
    fn open(&self, n: usize) -> Result<Box<dyn Read + Send + '_>, VfsError> {
        let id = self
            .records
            .get(n)
            .copied()
            .flatten()
            .ok_or(VfsError::NotFound)?;
        self.archives
            .read_record(&self.opened, id, self.ctx, Allowances::all(), None)
            .map_err(ArchiveError::into_vfs)
    }
}

/// What a rewrite does, in the words of its question.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Change {
    /// Members removed, by their chosen paths.
    Remove(Vec<String>),
    /// Members renamed: old path, new path.
    Rename(Vec<(String, String)>),
    /// Folders made.
    Folders(Vec<String>),
    /// More than one kind at once: how many paths were named in all.
    Mixed(u64),
}

/// A rewrite of a zip, decided before it starts: the pack plan that runs
/// it, and what the question, the cost line and the rows say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewritePlan {
    /// The pack plan the runner writes: the kept members, renamed where
    /// asked, and the new folders, in the old archive's order, written
    /// under a temporary name beside the archive and tested before it
    /// replaces the old one.
    pub pack: PackPlan,
    /// The archive changed.
    pub archive: Place,
    /// The archive's size: what is written again.
    pub archive_bytes: u64,
    /// Members of the old archive left out, the chosen ones and all they
    /// hold.
    pub removed_members: u64,
    /// What the rewrite loses or changes beyond the request, in words,
    /// each within 40 cells.
    pub notes: Vec<String>,
    /// Whether members are locked, so the job asks the password once.
    pub locked: bool,
    /// Whether the old archive goes to the trash, so Undo restores it.
    pub keeps_old: bool,
    change: Change,
    first_locked: Option<u32>,
}

/// The path's last part: "notes.txt" of "week1/notes.txt".
fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The place of a member, for the report; the archive's root when a place
/// cannot hold its name.
fn member_place(archive: &Place, path: &str) -> Place {
    let root = archive.clone().into_archive();
    let mut place = root.clone();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        match place.join(part) {
            Ok(next) => place = next,
            Err(_) => return root,
        }
    }
    place
}

/// A member path as the request gives it, without slashes at its ends;
/// `None` when nothing is left.
fn trimmed(path: &str) -> Option<&str> {
    let path = path.trim_matches('/');
    (!path.is_empty()).then_some(path)
}

/// What becomes of one record of the old archive.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fate {
    /// Left out: chosen for removal, or inside a folder that is.
    Removed,
    /// Kept under its own path.
    Kept,
    /// Kept under a new path.
    Moved(String),
}

/// Plans a rewrite of `archive`, a zip: the members at `removals` (and
/// everything inside a folder among them) left out, each `(from, to)` of
/// `renames` given its new path (a folder's members follow it), and each
/// of `new_folders` made. Paths are member paths as the panel has them,
/// `/` between names. Nothing is read but the index, and nothing is
/// written. Refused in words: any other format, a nested or split
/// archive, a path not in the archive, a new name taken or not safe, a
/// member that cannot be read, too little room for a second copy.
pub fn plan_rewrite(
    archives: &ArchiveProvider,
    archive: &Place,
    removals: &[String],
    renames: &[(String, String)],
    new_folders: &[String],
    job: JobId,
    ctx: &OpContext,
) -> Result<RewritePlan, ArchiveError> {
    let name = archive
        .name()
        .map_or_else(|| archive.to_string(), str::to_owned);
    let spoken = spell_out(&name).into_owned();
    let refuse = ArchiveError::Refused;
    if archive.archive().is_some() {
        return Err(refuse(NESTED_REFUSAL.to_owned()));
    }
    if diroxide_core::names::split_part(&name).is_some() {
        return Err(refuse(SPLIT_REFUSAL.to_owned()));
    }
    let opened = archives.open_archive(archive, ctx, Allowances::all())?;
    if opened.parts().is_some() {
        return Err(refuse(SPLIT_REFUSAL.to_owned()));
    }
    if !REWRITTEN.contains(&opened.format()) {
        return Err(refuse(ZIP_ONLY.to_owned()));
    }
    let index = opened.index();
    let change = change_of(removals, renames, new_folders);
    if change == Change::Mixed(0) {
        return Err(refuse(format!("Nothing to change in {spoken}")));
    }
    let find = |path: &str| -> Result<u32, ArchiveError> {
        trimmed(path)
            .and_then(|path| index.find(path))
            .filter(|&id| id != ROOT)
            .ok_or_else(|| {
                refuse(format!(
                    "{} is not in {spoken}",
                    spell_out(path.trim_matches('/'))
                ))
            })
    };
    let removed: HashSet<u32> = removals
        .iter()
        .map(|path| find(path))
        .collect::<Result<_, _>>()?;
    let mut renamed: HashMap<u32, String> = HashMap::new();
    for (from, to) in renames {
        let id = find(from)?;
        let to = safe_path(to, &spoken)?;
        renamed.insert(id, to);
    }
    let fates = fates(index, &removed, &renamed);
    let mut finals: HashMap<String, String> = HashMap::new();
    let mut folders: HashSet<String> = HashSet::new();
    for (id, fate) in fates.iter().enumerate() {
        let path = match fate {
            Fate::Removed => continue,
            Fate::Kept => index.path(id as u32),
            Fate::Moved(path) => path.clone(),
        };
        if index
            .record(id as u32)
            .is_some_and(|r| r.kind == RecordKind::Folder)
        {
            folders.insert(path.to_lowercase());
        }
        if let Some(clash) = finals.insert(path.to_lowercase(), path.clone()) {
            // The name said is the new one, whichever came first.
            let taken = if matches!(fate, Fate::Moved(_)) {
                path
            } else {
                clash
            };
            return Err(refuse(format!(
                "{} is in {spoken} already",
                spell_out(&taken)
            )));
        }
    }
    let mut made = Vec::new();
    for path in new_folders {
        let path = safe_path(path, &spoken)?;
        if finals.contains_key(&path.to_lowercase()) {
            return Err(refuse(format!(
                "{} is in {spoken} already",
                spell_out(&path)
            )));
        }
        finals.insert(path.to_lowercase(), path.clone());
        folders.insert(path.to_lowercase());
        made.push(path);
    }
    // Every renamed member and every new folder needs its folder, kept or
    // made; the archive's root always is.
    for path in renamed.values().chain(made.iter()) {
        if let Some((folder, _)) = path.rsplit_once('/') {
            let lower = folder.to_lowercase();
            if !folders.contains(&lower) && !implied(&finals, &lower, &path.to_lowercase()) {
                return Err(refuse(format!("{} is not in {spoken}", spell_out(folder))));
            }
        }
    }
    let registry = archives.registry()?;
    let provider = registry.resolve(archive)?;
    let folder = archive.parent().ok_or(VfsError::NotFound)?;
    if !provider.capabilities(&folder).writable {
        return Err(VfsError::PermissionDenied.into());
    }
    let archive_bytes = opened.key().size;
    let free = provider
        .space(&folder, ctx)
        .ok()
        .and_then(|space| space.free_to_user);
    let room = Room {
        needed: archive_bytes,
        free,
        drive: drive_of(&folder),
    };
    if let Some(line) = room.refusal() {
        return Err(refuse(line));
    }
    let built = build_items(index, &fates, archive, &made, &spoken)?;
    let mut notes = vec![COMMENTS_LOST.to_owned(), COMPRESSION_REDONE.to_owned()];
    if built.locked.is_some() {
        notes.push(PASSWORD_ONCE.to_owned());
    }
    if built.weak {
        notes.push(ZIPCRYPTO_UPGRADED.to_owned());
    }
    notes.extend(built.dropped_lines());
    let keeps_old = provider.capabilities(archive).trash;
    let count_of = |kind: EntryKind| built.items.iter().filter(|i| i.kind == kind).count() as u64;
    let bytes = built.items.iter().map(|i| i.size).sum();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8)) as u32;
    let pack = PackPlan {
        job,
        sources: vec![archive.clone()],
        destination: archive.clone(),
        temp: folder
            .join(&format!(
                "{PACK_TEMP_PREFIX}{}{PACK_TEMP_SUFFIX}",
                job.get()
            ))
            .map_err(VfsError::from)?,
        format: ArchiveFormat::Zip,
        level: CompressionLevel::Normal,
        solid: false,
        block_bytes: crate::limits::GIB,
        locked: built.locked.is_some(),
        encrypt_names: false,
        store_paths: StorePaths::Relative,
        keep_owner: false,
        threads,
        test_after: true,
        conflicts: ConflictPolicy::Overwrite,
        exclude: Vec::new(),
        files: count_of(EntryKind::File),
        folders: count_of(EntryKind::Folder),
        links: count_of(EntryKind::Link),
        bytes,
        items: built.items,
        left_out: Vec::new(),
        room,
        write_items: built.write_items,
        rewrite: Some(FromIndex {
            archive: archive.clone(),
            key: opened.key().clone(),
            records: built.records,
        }),
    };
    Ok(RewritePlan {
        pack,
        archive: archive.clone(),
        archive_bytes,
        removed_members: built.removed,
        notes,
        locked: built.locked.is_some(),
        keeps_old,
        change,
        first_locked: built.locked,
    })
}

/// The kind of change, from the three lists.
fn change_of(removals: &[String], renames: &[(String, String)], new_folders: &[String]) -> Change {
    let kinds = [
        !removals.is_empty(),
        !renames.is_empty(),
        !new_folders.is_empty(),
    ];
    match kinds {
        [true, false, false] => Change::Remove(removals.to_vec()),
        [false, true, false] => Change::Rename(renames.to_vec()),
        [false, false, true] => Change::Folders(new_folders.to_vec()),
        _ => Change::Mixed((removals.len() + renames.len() + new_folders.len()) as u64),
    }
}

/// A new path checked: a safe member name, written as it will be stored.
fn safe_path(path: &str, spoken: &str) -> Result<String, ArchiveError> {
    let wanted = path.trim_matches('/');
    match normalize_member_name(wanted) {
        Some(safe) if safe == wanted => Ok(safe),
        _ => Err(ArchiveError::Refused(format!(
            "{} is not a safe name in {spoken}",
            spell_out(wanted)
        ))),
    }
}

/// Whether some final path other than `except` lies inside `folder`, so
/// the folder exists by implication, as a zip may hold `a/b.txt` with no
/// `a/` entry. All three are lowercase.
fn implied(finals: &HashMap<String, String>, folder: &str, except: &str) -> bool {
    let prefix = format!("{folder}/");
    finals
        .keys()
        .any(|path| path != except && path.starts_with(&prefix))
}

/// What becomes of every record, in record order: a folder's record comes
/// before its members', so a removal or a rename reaches everything
/// inside it.
fn fates(index: &Index, removed: &HashSet<u32>, renamed: &HashMap<u32, String>) -> Vec<Fate> {
    let mut fates: Vec<Fate> = Vec::with_capacity(index.len());
    for id in 0..index.len() as u32 {
        let parent = index.record(id).map_or(ROOT, |r| r.parent);
        let inherited = if parent == ROOT {
            Fate::Kept
        } else {
            fates.get(parent as usize).cloned().unwrap_or(Fate::Kept)
        };
        let fate = if inherited == Fate::Removed || removed.contains(&id) {
            Fate::Removed
        } else if let Some(to) = renamed.get(&id) {
            Fate::Moved(to.clone())
        } else if let Fate::Moved(folder) = inherited {
            Fate::Moved(format!("{folder}/{}", index.name(id)))
        } else {
            Fate::Kept
        };
        fates.push(fate);
    }
    fates
}

/// The items of a rewrite and what was dropped on the way.
#[derive(Default)]
struct Built {
    items: Vec<PackItem>,
    write_items: Vec<WriteItem>,
    records: Vec<Option<u32>>,
    locked: Option<u32>,
    weak: bool,
    removed: u64,
    older_copies: u64,
    shadowed: u64,
    specials: u64,
    refused: u64,
}

impl Built {
    /// The notes for what a rewrite drops, each within 40 cells.
    fn dropped_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.older_copies > 0 {
            lines.push(format!(
                "Older copies of {} are dropped",
                count(self.older_copies, "name", "names")
            ));
        }
        if self.shadowed > 0 {
            lines.push(format!(
                "{} under a folder's name dropped",
                count(self.shadowed, "file", "files")
            ));
        }
        if self.specials > 0 {
            lines.push(format!(
                "{} dropped",
                count(self.specials, "special entry", "special entries")
            ));
        }
        if self.refused > 0 {
            lines.push(format!(
                "{} dropped",
                count(self.refused, "unsafe name", "unsafe names")
            ));
        }
        lines
    }
}

/// The pack items of a rewrite, in the old archive's order, the new
/// folders last.
fn build_items(
    index: &Index,
    fates: &[Fate],
    archive: &Place,
    made: &[String],
    spoken: &str,
) -> Result<Built, ArchiveError> {
    let mut built = Built {
        refused: index.summary().refused,
        ..Built::default()
    };
    for (id, fate) in fates.iter().enumerate() {
        let id = id as u32;
        let Some(record) = index.record(id).copied() else {
            continue;
        };
        if *fate == Fate::Removed {
            if !record.flags.contains(RecordFlags::SYNTHESIZED) {
                built.removed += 1;
            }
            continue;
        }
        let path = match fate {
            Fate::Moved(path) => path.clone(),
            _ => index.path(id),
        };
        if record.flags.contains(RecordFlags::DUPLICATE) {
            built.older_copies += 1;
        }
        if record.flags.contains(RecordFlags::FILE_CLASH)
            || record.flags.contains(RecordFlags::LINK_CLASH)
        {
            built.shadowed += 1;
        }
        let modified = record.modified().map(|secs| Timestamp::from_unix(secs, 0));
        let mode = record.mode().map(|m| m & 0o7777);
        let (kind, item_kind) = match record.kind {
            // A folder only a member's path implies stays implied.
            RecordKind::Folder if record.flags.contains(RecordFlags::SYNTHESIZED) => continue,
            RecordKind::Folder => (EntryKind::Folder, ItemKind::Folder),
            RecordKind::Link => {
                let target = index.link_target(id).unwrap_or_default().to_owned();
                (EntryKind::Link, ItemKind::Link(target))
            }
            RecordKind::Special => {
                built.specials += 1;
                continue;
            }
            RecordKind::File => {
                if record.locator == Locator::None || record.flags.contains(RecordFlags::UNREADABLE)
                {
                    return Err(ArchiveError::Refused(format!(
                        "{} cannot be read, so {spoken} stays as it is",
                        spell_out(&path)
                    )));
                }
                if record.flags.contains(RecordFlags::ENCRYPTED) && built.locked.is_none() {
                    built.locked = Some(id);
                }
                built.weak |= record.flags.contains(RecordFlags::WEAK_CRYPTO);
                (EntryKind::File, ItemKind::File)
            }
        };
        let size = if kind == EntryKind::File {
            record.size
        } else {
            0
        };
        built.write_items.push(WriteItem {
            name: path.clone(),
            kind: item_kind,
            size,
            modified,
            mode,
            owner: None,
        });
        built.items.push(PackItem {
            source: member_place(archive, &index.path(id)),
            name: path,
            kind,
            size,
        });
        built.records.push(Some(id));
    }
    let now = Timestamp::from_system_time(std::time::SystemTime::now());
    for path in made {
        built.write_items.push(WriteItem {
            name: path.clone(),
            kind: ItemKind::Folder,
            size: 0,
            modified: Some(now),
            mode: None,
            owner: None,
        });
        built.items.push(PackItem {
            source: member_place(archive, path),
            name: path.clone(),
            kind: EntryKind::Folder,
            size: 0,
        });
        built.records.push(None);
    }
    Ok(built)
}

impl RewritePlan {
    /// The archive's name.
    pub fn archive_name(&self) -> String {
        self.archive
            .name()
            .map_or_else(|| self.archive.to_string(), str::to_owned)
    }

    fn spoken(&self) -> String {
        spell_out(&self.archive_name()).into_owned()
    }

    /// The change in one line, the verb first: "Remove 2 members from
    /// course.zip", "Rename notes.txt in course.zip", "Make folder week3
    /// in course.zip", "Change 4 members of course.zip".
    pub fn summary(&self) -> String {
        let archive = self.spoken();
        match &self.change {
            Change::Remove(paths) if paths.len() == 1 => {
                format!("Remove {} from {archive}", spell_out(leaf(&paths[0])))
            }
            Change::Remove(paths) => format!(
                "Remove {} from {archive}",
                count(paths.len() as u64, "member", "members")
            ),
            Change::Rename(pairs) if pairs.len() == 1 => {
                format!("Rename {} in {archive}", spell_out(leaf(&pairs[0].0)))
            }
            Change::Rename(pairs) => format!(
                "Rename {} in {archive}",
                count(pairs.len() as u64, "member", "members")
            ),
            Change::Folders(paths) if paths.len() == 1 => {
                format!("Make folder {} in {archive}", spell_out(leaf(&paths[0])))
            }
            Change::Folders(paths) => format!(
                "Make {} in {archive}",
                count(paths.len() as u64, "folder", "folders")
            ),
            Change::Mixed(n) => format!("Change {} of {archive}", count(*n, "member", "members")),
        }
    }

    /// The question before the start, which starts on No: "Remove 2
    /// members from course.zip? y or n".
    pub fn question(&self) -> Message {
        Message::question(&self.summary())
    }

    /// The cost, said with the question: "Rewrites all of course.zip: 1.2
    /// GB".
    pub fn cost_line(&self) -> String {
        format!(
            "Rewrites all of {}: {}",
            self.spoken(),
            human_size(self.archive_bytes)
        )
    }

    /// The rows of the question's dialog, each with its meaning in its
    /// first 40 cells: the change, the cost, how many members go in all
    /// when a folder was among them, what the rewrite loses, and where the
    /// old copy goes.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = vec![self.summary(), self.cost_line()];
        let chosen = match &self.change {
            Change::Remove(paths) => paths.len() as u64,
            _ => 0,
        };
        if chosen > 0 && self.removed_members > chosen {
            rows.push(format!(
                "Removes {} in all",
                count(self.removed_members, "member", "members")
            ));
        }
        rows.extend(self.notes.iter().cloned());
        rows.push(if self.keeps_old {
            OLD_TO_TRASH.to_owned()
        } else {
            OLD_NOT_KEPT.to_owned()
        });
        rows
    }

    /// The line after a rewrite that sent the old archive to the trash:
    /// "Old copy of course.zip is in the trash".
    pub fn trashed_line(&self) -> String {
        format!("Old copy of {} is in the trash", self.spoken())
    }

    /// The line after a rewrite that replaced the old archive without the
    /// trash: "Old copy of course.zip was not kept".
    pub fn not_kept_line(&self) -> String {
        format!("Old copy of {} was not kept", self.spoken())
    }

    /// The first line after a rewrite that finished, in the past tense:
    /// "Removed 2 members from course.zip".
    pub fn done_line(&self) -> String {
        let summary = self.summary();
        for (verb, done) in [
            ("Remove ", "Removed "),
            ("Rename ", "Renamed "),
            ("Make ", "Made "),
            ("Change ", "Changed "),
        ] {
            if let Some(rest) = summary.strip_prefix(verb) {
                return format!("{done}{rest}");
            }
        }
        summary
    }
}

/// What a rewrite did: the engine's report (its undo record holds the
/// new archive, and the old one in the trash, so Undo restores it), the
/// lines to say, and whether the archive was changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteOutcome {
    /// The report.
    pub report: JobReport,
    /// The result first, then the check and the trash: "Removed 2
    /// members from course.zip", "Checked 998 of 998 entries, all
    /// good.", "Old copy of course.zip is in the trash". On a failure,
    /// "Could not replace course.zip: another program has it open" or
    /// "Could not change course.zip: CAUSE", then "course.zip is
    /// unchanged".
    pub lines: Vec<String>,
    /// Whether the new archive took the old one's place.
    pub changed: bool,
    /// Whether the old archive is in the trash.
    pub old_in_trash: bool,
}

/// Runs a rewrite: the password asked once when members are locked (and
/// tried on the first locked member, three tries at most), then the pack
/// runner over the old archive's records, then the index forgotten so the
/// panel reads the new archive. A cancel or a failure at any step leaves
/// the old archive as it was. `password` is one already typed.
pub fn run_rewrite(
    plan: &RewritePlan,
    password: Option<Secret>,
    env: &JobEnv<'_>,
) -> RewriteOutcome {
    let spoken = plan.spoken();
    let mut password = password;
    if let Some(id) = plan.first_locked {
        match unlock(plan, id, password.take(), env) {
            Ok(secret) => password = Some(secret),
            Err(error) => {
                let mut report = JobReport::new(env.job, JobKind::Pack);
                if error == VfsError::Cancelled {
                    report.cancelled = true;
                } else {
                    report
                        .failed
                        .push((plan.archive.clone(), error.to_string()));
                }
                let lines = failure_lines(&spoken, &report);
                return RewriteOutcome {
                    report,
                    lines,
                    changed: false,
                    old_in_trash: false,
                };
            }
        }
    }
    let mut outcome = run_pack(&plan.pack, password.as_ref(), env);
    env.archives.forget(&plan.archive);
    let changed = outcome.archive.is_some();
    let cancelled = VfsError::Cancelled.to_string();
    if !changed
        && (env.control.cancel.is_cancelled()
            || outcome.report.failed.iter().any(|(_, c)| *c == cancelled))
    {
        // A cancel met while a member was read is the job's cancel.
        outcome.report.cancelled = true;
    }
    let old_in_trash = changed && outcome.report.undo.trashed.contains(&plan.archive);
    let lines = if changed {
        let mut lines = vec![plan.done_line()];
        lines.extend(outcome.lines.iter().skip(1).cloned());
        lines.push(if old_in_trash {
            plan.trashed_line()
        } else {
            // No trash at the place, or the engine's option to keep the
            // old file is off: said, so nobody counts on Undo.
            plan.not_kept_line()
        });
        lines
    } else {
        failure_lines(&spoken, &outcome.report)
    };
    RewriteOutcome {
        report: outcome.report,
        lines,
        changed,
        old_in_trash,
    }
}

/// The lines of a rewrite that changed nothing, the cause first: a lock
/// by another program in its own words, a cancel, or the first failure.
fn failure_lines(spoken: &str, report: &JobReport) -> Vec<String> {
    let unchanged = format!("{spoken} is unchanged");
    if report.cancelled {
        return vec![format!("Cancelled: {unchanged}")];
    }
    let in_use = VfsError::InUse.to_string();
    let first = match report.failed.first() {
        Some((_, cause)) if *cause == in_use => {
            format!("Could not replace {spoken}: another program has it open")
        }
        Some((_, cause)) => format!("Could not change {spoken}: {cause}"),
        None => match report.skipped.first() {
            Some((_, reason)) => format!("Could not change {spoken}: {reason}"),
            None => format!("Could not change {spoken}"),
        },
    };
    vec![first, unchanged]
}

/// Asks the password of a locked zip once for the whole rewrite and tries
/// it on the first locked member: three tries at most, as every job's
/// passwords are. The same password then reads every old member and locks
/// every new one.
fn unlock(
    plan: &RewritePlan,
    id: u32,
    password: Option<Secret>,
    env: &JobEnv<'_>,
) -> Result<Secret, VfsError> {
    let secrets = JobSecrets::knowing(env.secrets.clone(), password);
    let ctx = env.context(secrets.clone());
    let opened = env
        .archives
        .open_archive(&plan.archive, &ctx, Allowances::all())
        .map_err(ArchiveError::into_vfs)?;
    secrets
        .with_tries(&ctx, |ctx| {
            let mut reader = env
                .archives
                .read_record(&opened, id, ctx, Allowances::all(), None)?;
            let mut first = [0u8; 1];
            reader
                .read(&mut first)
                .map_err(|e| ArchiveError::Vfs(crate::guard::from_io(e)))?;
            Ok(())
        })
        .map_err(ArchiveError::into_vfs)?;
    let request = SecretRequest::new(SecretKind::ArchivePassword, plan.archive_name());
    secrets
        .ask(&request, &ctx.cancel)
        .ok_or(VfsError::PasswordRequired)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixed_lines_fit_the_display() {
        for line in [
            ZIP_ONLY,
            NESTED_REFUSAL,
            SPLIT_REFUSAL,
            COMMENTS_LOST,
            COMPRESSION_REDONE,
            PASSWORD_ONCE,
            ZIPCRYPTO_UPGRADED,
            OLD_TO_TRASH,
            OLD_NOT_KEPT,
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert_eq!(ZIP_ONLY.chars().count(), 38);
    }

    #[test]
    fn a_change_is_named_by_its_kind() {
        let one = |s: &str| vec![s.to_owned()];
        assert_eq!(change_of(&one("a"), &[], &[]), Change::Remove(one("a")));
        assert_eq!(change_of(&[], &[], &one("b")), Change::Folders(one("b")));
        assert_eq!(change_of(&one("a"), &[], &one("b")), Change::Mixed(2));
        assert_eq!(change_of(&[], &[], &[]), Change::Mixed(0));
        assert_eq!(leaf("week1/notes.txt"), "notes.txt");
        assert_eq!(trimmed("/week1/"), Some("week1"));
        assert_eq!(trimmed("/"), None);
    }

    #[test]
    fn new_paths_must_be_safe() {
        assert_eq!(safe_path("week3/", "a.zip").unwrap(), "week3");
        let line = safe_path("../x", "a.zip").unwrap_err().to_string();
        assert_eq!(line, "../x is not a safe name in a.zip");
        assert!(safe_path("a/../b", "a.zip").is_err());
    }
}
