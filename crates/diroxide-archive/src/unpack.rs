//! The unpack plan (plan commit 29, research/archives.md 8.3 and 7):
//! which members, where they land, the folder rule, flattening, the
//! destination's name rules, what is left out and why, and the room it
//! needs, all decided from the index before a byte is read or written.
//!
//! The rows say the plan in words, each with its meaning in its first 40
//! cells: "Unpack 10 members into course", "Where: D:\out\course",
//! "Writes 8 files, 48 KB. 12 GB free on D:", or the refusal "Needs 40
//! GB, only 12 GB free on D:.", which the runner holds to: nothing is
//! written when the room is short. The question starts on No: "Unpack 10
//! members into course? y or n".
//!
//! Names follow the destination's rules: a component that needs care
//! (a hidden or control character, a character the drive reserves, a
//! trailing dot or space on Windows, a Windows device name, a name that
//! would read back as an archive place, one too long) is renamed by one
//! rule, [`destination_name`], and counted: "Renamed 3 names: unsafe
//! characters." Links are left out unless the safe ones are asked for,
//! devices and other special entries always, each with its line.

use std::collections::HashSet;

use diroxide_core::format::{count, human_size};
use diroxide_core::names::{classify, spell_out, split_extension};
use diroxide_core::{ConflictPolicy, JobId, JobKind, JobRequest, Message, Place};
use diroxide_vfs::names::{DEVICE_NAMES, NameRules, RESERVED_CHARACTERS};
use diroxide_vfs::{OpContext, Registry, VfsError};

use crate::error::ArchiveError;
use crate::format::ArchiveFormat;
use crate::guard::{Allowances, Question, QuestionKind, refuse};
use crate::index::{Index, ROOT, RecordFlags, RecordKind, link_leaves};
use crate::limits::limit_words;
use crate::names::{NameFlags, has_archive_marker};
use crate::provider::ArchiveProvider;
use crate::request::{FolderRule, UnpackRequest};

/// A name made safe for the destination by one rule, and whether it
/// changed: hidden and control characters, the stand-in for bytes that
/// were not text, and (on a place with Windows's rules) reserved
/// characters become `_`; a `!` after an archive extension becomes `_`
/// so the name never reads back as a place inside an archive; a leading
/// space, and on Windows a trailing dot or space, become `_`; a Windows
/// device name gains `_` after its stem; a name too long is cut, its
/// extension kept; an empty or dot name becomes `_`.
pub fn destination_name(name: &str, rules: &NameRules) -> (String, bool) {
    let mut out: String = name
        .chars()
        .map(|c| {
            let reserved = c == '/' || (rules.windows && RESERVED_CHARACTERS.contains(&c));
            if classify(c).is_hidden() || c == '\u{FFFD}' || reserved || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect();
    while has_archive_marker(&out) {
        let Some(at) = marker_at(&out) else { break };
        out.replace_range(at..at + 1, "_");
    }
    if out.starts_with(' ') {
        out.replace_range(0..1, "_");
    }
    if rules.windows {
        let kept = out.trim_end_matches(['.', ' ']).len();
        let trailing = out.len() - kept;
        if trailing > 0 {
            out.truncate(kept);
            out.push_str(&"_".repeat(trailing));
        }
        let stem = out
            .split('.')
            .next()
            .unwrap_or(&out)
            .trim_end()
            .to_lowercase();
        if DEVICE_NAMES.contains(&stem.as_str()) {
            let at = out.find('.').unwrap_or(out.len());
            out.insert(at, '_');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "_".repeat(out.len().max(1));
    }
    if let Some(max) = rules.max_length {
        out = cut_to(&out, max, rules.length_in_utf16);
    }
    let changed = out != name;
    (out, changed)
}

/// Where the `!` of an archive marker is: the first `!` that follows a
/// known archive extension.
fn marker_at(name: &str) -> Option<usize> {
    name.match_indices('!')
        .map(|(at, _)| at)
        .find(|&at| diroxide_core::place::opens_as_archive(&name[..at]))
}

/// A name cut to `max` (bytes, or UTF-16 units), its extension kept when
/// there is room.
fn cut_to(name: &str, max: usize, utf16: bool) -> String {
    let length = |text: &str| {
        if utf16 {
            text.encode_utf16().count()
        } else {
            text.len()
        }
    };
    if length(name) <= max {
        return name.to_owned();
    }
    let (stem, extension) = split_extension(name);
    let extension = if length(extension) * 2 < max {
        extension
    } else {
        ""
    };
    let room = max - length(extension);
    let mut kept = String::new();
    for c in stem.chars() {
        if length(&kept) + length(c.encode_utf8(&mut [0; 4])) > room {
            break;
        }
        kept.push(c);
    }
    format!("{kept}{extension}")
}

/// The line for a member left out, its meaning first: "Skipped ../x: name
/// leaves the archive.", or with a long name, "Skipped, link not
/// followed: a-very-long-name.txt."
pub fn skipped_line(name: &str, reason: &str) -> String {
    let name = diroxide_core::names::display_name(name);
    if name.chars().count() > 20 {
        format!("Skipped, {reason}: {name}.")
    } else {
        format!("Skipped {name}: {reason}.")
    }
}

/// One member the plan unpacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedMember {
    /// Its record in the archive's index.
    pub id: u32,
    /// Its path in the archive, made safe.
    pub path: String,
    /// Where it lands.
    pub target: Place,
    /// What it is.
    pub kind: RecordKind,
    /// The size it claims.
    pub size: u64,
    /// Whether a name on its way was renamed by the destination's rules.
    pub renamed: bool,
    /// A link's target, for a link made (restore safe links).
    pub link_target: Option<String>,
    /// Whether the link points at a folder.
    pub link_to_folder: bool,
    /// When it last changed, in seconds since the epoch.
    pub modified: Option<i64>,
    /// Its Unix mode bits, as stored.
    pub mode: Option<u32>,
    /// Whether it is locked behind a password.
    pub encrypted: bool,
}

/// A member left out, with the reason in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// Its path in the archive.
    pub path: String,
    /// Why: "link not followed".
    pub reason: String,
}

impl LeftOut {
    /// The line: "Skipped latest: link not followed."
    pub fn line(&self) -> String {
        skipped_line(&self.path, &self.reason)
    }
}

/// The room a job needs on its destination's drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Room {
    /// The bytes the job writes.
    pub needed: u64,
    /// The bytes free, when the drive says.
    pub free: Option<u64>,
    /// The drive in words: "D:", or empty when it has no letter.
    pub drive: String,
}

impl Room {
    /// Whether there is room, or nothing says there is not.
    pub fn enough(&self) -> bool {
        self.free.is_none_or(|free| self.needed <= free)
    }

    fn on(&self) -> String {
        if self.drive.is_empty() {
            "here".to_owned()
        } else {
            format!("on {}", self.drive)
        }
    }

    /// The refusal when the room is short: "Needs 40 GB, only 12 GB free
    /// on D:."
    pub fn refusal(&self) -> Option<String> {
        let free = self.free?;
        (self.needed > free).then(|| {
            format!(
                "Needs {}, only {} free {}.",
                human_size(self.needed),
                human_size(free),
                self.on()
            )
        })
    }

    /// The free space in words: "12 GB free on D:".
    pub fn free_words(&self) -> Option<String> {
        self.free
            .map(|free| format!("{} free {}", human_size(free), self.on()))
    }
}

/// The drive of a place in words: "D:" for a Windows path, empty
/// otherwise.
pub(crate) fn drive_of(place: &Place) -> String {
    let path = match place.archive() {
        Some((container, _)) => container.local_path(),
        None => place.local_path(),
    };
    match path {
        Some(path) if path.as_bytes().get(1) == Some(&b':') => path[..2].to_uppercase(),
        _ => String::new(),
    }
}

/// What the plan counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnpackFacts {
    /// Files to write.
    pub files: u64,
    /// Folders to make.
    pub folders: u64,
    /// Links to make.
    pub links: u64,
    /// The bytes the files claim.
    pub bytes: u64,
    /// Members locked behind a password.
    pub locked: u64,
    /// Items at the top, before the folder rule.
    pub tops: u64,
}

/// What an unpack will do, decided before it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpackPlan {
    /// The job.
    pub job: JobId,
    /// The archive.
    pub archive: Place,
    /// Its file name.
    pub archive_name: String,
    /// Its format.
    pub format: ArchiveFormat,
    /// The folder chosen (the other panel's, by default).
    pub chosen: Place,
    /// Where the members land: the chosen folder, or the folder made in
    /// it for the archive.
    pub destination: Place,
    /// The folder made for the archive, by the folder rule.
    pub made_folder: Option<String>,
    /// The folder rule asked for.
    pub folder_rule: FolderRule,
    /// Names only, without folders.
    pub flatten: bool,
    /// Whether safe relative links are made.
    pub restore_links: bool,
    /// What to do when a name is taken.
    pub conflicts: ConflictPolicy,
    /// The members, in the order their data lies in the archive.
    pub members: Vec<PlannedMember>,
    /// What is left out, with the reasons.
    pub left_out: Vec<LeftOut>,
    /// The index's own refusals: "Skipped ../x: name leaves the archive."
    pub refused_lines: Vec<String>,
    /// How many names the index refused in all.
    pub refused: u64,
    /// How many members were renamed by the destination's rules.
    pub renamed: u64,
    /// The counts.
    pub facts: UnpackFacts,
    /// The room on the destination's drive.
    pub room: Room,
    /// The questions to ask before the start (a total over the limit);
    /// each answer yes goes in the run's allowances.
    pub questions: Vec<Question>,
}

/// The archive's name without its extension (and without a part number):
/// the folder the folder rule makes. "course.zip" gives "course",
/// "logs.tar.xz" "logs", "backup.7z.001" "backup".
pub fn archive_stem(name: &str) -> String {
    let base = diroxide_core::names::split_part(name).map_or(name, |part| part.base);
    for extension in [
        ".tar.gz", ".tar.xz", ".tgz", ".txz", ".zip", ".7z", ".tar", ".xz",
    ] {
        if base.len() > extension.len()
            && let Some(tail) = base.get(base.len() - extension.len()..)
            && tail.eq_ignore_ascii_case(extension)
        {
            return base[..base.len() - extension.len()].to_owned();
        }
    }
    format!("{base} files")
}

/// The policy in words, for the plan's row.
fn policy_words(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Ask => "ask",
        ConflictPolicy::Skip => "skip it",
        ConflictPolicy::Overwrite => "replace it",
        ConflictPolicy::KeepBoth => "keep both",
        ConflictPolicy::Rename => "rename the new one",
        ConflictPolicy::Newer => "replace it if newer",
    }
}

impl UnpackPlan {
    /// The members unpacked: files, folders and links.
    pub fn count(&self) -> u64 {
        self.members.len() as u64
    }

    /// What the members land in, by name: the folder made, or the chosen
    /// folder's name.
    fn landing_name(&self) -> String {
        match &self.made_folder {
            Some(folder) => folder.clone(),
            None => self
                .destination
                .name()
                .map_or_else(|| self.destination.to_string(), str::to_owned),
        }
    }

    /// The plan in one line, meaning first: "Unpack 10 members into
    /// course".
    pub fn summary(&self) -> String {
        format!(
            "Unpack {} into {}",
            count(self.count(), "member", "members"),
            spell_out(&self.landing_name())
        )
    }

    /// The question before the start, starting on No: "Unpack 10 members
    /// into course? y or n".
    pub fn question(&self) -> Message {
        Message::question(&self.summary())
    }

    /// The refusal the run holds to, when there is one: "Needs 40 GB,
    /// only 12 GB free on D:."
    pub fn refusal(&self) -> Option<Message> {
        self.room.refusal().map(Message::error)
    }

    /// The plan's rows, each a line with its meaning first.
    pub fn rows(&self) -> Vec<String> {
        let mut rows = vec![self.summary()];
        rows.push(format!(
            "Where: {}",
            spell_out(&self.destination.to_string())
        ));
        rows.push(match (&self.made_folder, self.folder_rule) {
            (Some(folder), FolderRule::Always) => {
                format!("Folder: {}, always made", spell_out(folder))
            }
            (Some(folder), _) => format!(
                "Folder: {}, {} at the top",
                spell_out(folder),
                count(self.facts.tops, "item", "items")
            ),
            (None, FolderRule::Never) => "Folder: none, as set".to_owned(),
            (None, _) => "Folder: none, one item at the top".to_owned(),
        });
        rows.push(if self.flatten {
            "Paths: names only, folders left out".to_owned()
        } else {
            "Paths: kept".to_owned()
        });
        rows.push(match self.room.refusal() {
            Some(refusal) => refusal,
            None => {
                let writes = format!(
                    "Writes {}, {}",
                    count(self.facts.files, "file", "files"),
                    human_size(self.facts.bytes)
                );
                match self.room.free_words() {
                    Some(free) => format!("{writes}. {free}"),
                    None => format!("{writes}."),
                }
            }
        });
        rows.push(format!(
            "When a name is taken: {}",
            policy_words(self.conflicts)
        ));
        if self.facts.locked > 0 {
            rows.push(format!(
                "Password: asked for {}",
                count(self.facts.locked, "locked member", "locked members")
            ));
        }
        if self.renamed > 0 {
            rows.push(renamed_line(self.renamed));
        }
        for left in &self.left_out {
            rows.push(left.line());
        }
        rows.extend(self.refused_lines.iter().cloned());
        let shown = self.refused_lines.len() as u64;
        if self.refused > shown {
            rows.push(format!(
                "And {} refused.",
                count(self.refused - shown, "more name", "more names")
            ));
        }
        rows
    }

    /// The job request the app keeps in its jobs list and journal for this
    /// unpack: the archive, the destination, the policy. It holds no
    /// password.
    pub fn job_request(&self) -> JobRequest {
        let mut request = JobRequest::new(self.job, JobKind::Unpack, vec![self.archive.clone()])
            .to(self.chosen.clone());
        request.conflicts = self.conflicts;
        request
    }
}

/// "Renamed 3 names: unsafe characters."
pub fn renamed_line(renamed: u64) -> String {
    format!(
        "Renamed {}: unsafe characters.",
        count(renamed, "name", "names")
    )
}

/// The records an unpack takes, each with its path relative to where the
/// selection starts: everything, or the members asked for (a folder with
/// all it holds), relative to the folder they are in.
fn selection(
    index: &Index,
    request: &UnpackRequest,
    left_out: &mut Vec<LeftOut>,
) -> Vec<(u32, String)> {
    if request.members.is_empty() {
        return (0..index.len() as u32)
            .map(|id| (id, index.path(id)))
            .collect();
    }
    let mut chosen = Vec::new();
    let mut seen = HashSet::new();
    for asked in &request.members {
        let asked = asked.trim_matches('/');
        let Some(id) = index.find(asked).filter(|&id| id != ROOT) else {
            left_out.push(LeftOut {
                path: asked.to_owned(),
                reason: "not in the archive".to_owned(),
            });
            continue;
        };
        let base = index.path(id);
        let base = base
            .rsplit_once('/')
            .map_or("", |(folder, _)| folder)
            .to_owned();
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            if !seen.insert(at) {
                continue;
            }
            let path = index.path(at);
            let relative = if base.is_empty() {
                path
            } else {
                path.strip_prefix(&format!("{base}/"))
                    .map_or(path.clone(), str::to_owned)
            };
            chosen.push((at, relative));
            stack.extend(index.children(at).iter().rev());
        }
    }
    chosen
}

/// Plans an unpack: opens the archive (its index, from the cache when
/// unchanged; a locked 7z's names ask for the password through `ctx`, so
/// pass the job's [`crate::JobSecrets`] context to ask once per job),
/// chooses the members, applies the folder rule and the destination's
/// name rules, leaves out what must not be made, and checks the room.
/// Nothing is read but the index, and nothing is written.
pub fn plan_unpack(
    archives: &ArchiveProvider,
    registry: &Registry,
    request: &UnpackRequest,
    job: JobId,
    ctx: &OpContext,
    allow: Allowances,
) -> Result<UnpackPlan, ArchiveError> {
    let opened = archives.open_archive(&request.source, ctx, allow)?;
    let index = opened.index();
    let provider = registry.resolve(&request.destination)?;
    let capabilities = provider.capabilities(&request.destination);
    let rules = NameRules::from_capabilities(&capabilities);
    let mut left_out = Vec::new();
    let chosen = selection(index, request, &mut left_out);
    let order: Vec<u32> = opened.archive_order();
    let rank: std::collections::HashMap<u32, usize> =
        order.iter().enumerate().map(|(n, &id)| (id, n)).collect();
    let mut chosen = chosen;
    chosen.sort_by_key(|(id, _)| rank.get(id).copied().unwrap_or(usize::MAX));
    let tops: HashSet<String> = chosen
        .iter()
        .filter(|(id, _)| {
            index.record(*id).is_some_and(|r| {
                !(r.kind == RecordKind::Special
                    || (r.kind == RecordKind::Link && !request.restore_links))
            })
        })
        .map(|(_, path)| path.split('/').next().unwrap_or("").to_owned())
        .collect();
    let tops = tops.len() as u64;
    let made_folder = match request.folder_rule {
        FolderRule::Always => Some(archive_stem(opened.name())),
        FolderRule::WhenSeveral if tops > 1 => Some(archive_stem(opened.name())),
        _ => None,
    };
    let destination = match &made_folder {
        Some(folder) => {
            let (safe, _) = destination_name(folder, &rules);
            request.destination.join(&safe).map_err(VfsError::from)?
        }
        None => request.destination.clone(),
    };
    let mut facts = UnpackFacts {
        tops,
        ..UnpackFacts::default()
    };
    let mut members = Vec::new();
    let mut renamed = 0u64;
    let links_here = capabilities.can_link();
    for (id, relative) in chosen {
        let Some(record) = index.record(id).copied() else {
            continue;
        };
        let leave = |left_out: &mut Vec<LeftOut>, reason: &str| {
            left_out.push(LeftOut {
                path: relative.clone(),
                reason: reason.to_owned(),
            });
        };
        let mut link_target = None;
        let mut link_to_folder = false;
        match record.kind {
            RecordKind::Special => {
                leave(&mut left_out, "device or special entry, not unpacked");
                continue;
            }
            RecordKind::Link => {
                let target = index.link_target(id).unwrap_or("").to_owned();
                if !request.restore_links {
                    leave(&mut left_out, "link not followed");
                    continue;
                }
                if target.is_empty()
                    || record.flags.contains(RecordFlags::LINK_LEAVES)
                    || link_leaves(&relative, &target)
                {
                    leave(&mut left_out, "link leaves the archive");
                    continue;
                }
                if !links_here {
                    leave(&mut left_out, "this drive holds no links");
                    continue;
                }
                link_to_folder = link_points_to_folder(index, id, &target);
                link_target = Some(target);
            }
            RecordKind::Folder => {
                if record.flags.contains(RecordFlags::FILE_CLASH) {
                    leave(&mut left_out, "file and folder share the name, folder kept");
                }
                if record.flags.contains(RecordFlags::LINK_CLASH) {
                    leave(&mut left_out, "link and folder share the name, folder kept");
                }
                if request.flatten {
                    continue;
                }
            }
            RecordKind::File => {
                if record.flags.contains(RecordFlags::UNREADABLE) {
                    let why = index
                        .note(id)
                        .map_or("cannot be read here".to_owned(), |note| {
                            format!("cannot be read here: {}", note.trim_end_matches('.'))
                        });
                    leave(&mut left_out, &why);
                    continue;
                }
                if record.flags.contains(RecordFlags::MISMATCH) {
                    leave(&mut left_out, "its two headers disagree");
                    continue;
                }
            }
        }
        let path = if request.flatten && record.kind != RecordKind::Folder {
            relative.rsplit('/').next().unwrap_or(&relative).to_owned()
        } else {
            relative.clone()
        };
        let mut target = destination.clone();
        let mut changed = record.name_flags.contains(NameFlags::DRIVE)
            || record.name_flags.contains(NameFlags::LEADING_SLASH)
            || record.name_flags.contains(NameFlags::DEVICE_PREFIX);
        let mut made = true;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let (safe, renamed_part) = destination_name(part, &rules);
            changed |= renamed_part;
            match target.join(&safe) {
                Ok(next) => target = next,
                Err(_) => {
                    made = false;
                    break;
                }
            }
        }
        if !made || target == destination {
            leave(&mut left_out, "name cannot be made here");
            continue;
        }
        if changed {
            renamed += 1;
        }
        let encrypted = record.flags.contains(RecordFlags::ENCRYPTED);
        match record.kind {
            RecordKind::Folder => facts.folders += 1,
            RecordKind::Link => facts.links += 1,
            _ => {
                facts.files += 1;
                facts.bytes = facts.bytes.saturating_add(record.size);
            }
        }
        facts.locked += u64::from(encrypted);
        members.push(PlannedMember {
            id,
            path: relative,
            target,
            kind: record.kind,
            size: record.size,
            renamed: changed,
            link_target,
            link_to_folder,
            modified: record.modified(),
            mode: record.mode(),
            encrypted,
        });
    }
    let limits = archives.limits();
    if facts.bytes > limits.max_total_bytes {
        return Err(refuse("bytes unpacked", limit_words(limits.max_total_bytes)).into());
    }
    let mut questions = Vec::new();
    if facts.bytes > limits.ask_total_bytes && !allow.total {
        questions.push(Question {
            kind: QuestionKind::Total,
            text: format!(
                "Expands to {} from {}. Unpack",
                human_size(facts.bytes),
                human_size(opened.key().size)
            ),
        });
    }
    let free = provider
        .space(&request.destination, ctx)
        .ok()
        .and_then(|space| space.free_to_user);
    let summary = index.summary();
    Ok(UnpackPlan {
        job,
        archive: request.source.clone(),
        archive_name: opened.name().to_owned(),
        format: opened.format(),
        chosen: request.destination.clone(),
        destination,
        made_folder,
        folder_rule: request.folder_rule,
        flatten: request.flatten,
        restore_links: request.restore_links,
        conflicts: request.conflicts,
        members,
        left_out,
        refused_lines: if request.members.is_empty() {
            summary.refused_lines.clone()
        } else {
            Vec::new()
        },
        refused: if request.members.is_empty() {
            summary.refused
        } else {
            0
        },
        renamed,
        facts,
        room: Room {
            needed: facts.bytes,
            free,
            drive: drive_of(&request.destination),
        },
        questions,
    })
}

/// Whether a link's stored target, from where the link sits, is a folder
/// in the archive.
fn link_points_to_folder(index: &Index, id: u32, target: &str) -> bool {
    let path = index.path(id);
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    index
        .find(&parts.join("/"))
        .and_then(|found| index.record(found))
        .is_some_and(|record| record.kind == RecordKind::Folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: NameRules = NameRules {
        windows: true,
        max_length: Some(255),
        length_in_utf16: true,
    };
    const UNIX: NameRules = NameRules {
        windows: false,
        max_length: Some(255),
        length_in_utf16: false,
    };

    #[test]
    fn names_are_made_safe_by_one_rule() {
        let table: [(&str, NameRules, &str); 12] = [
            ("notes.txt", WINDOWS, "notes.txt"),
            ("a\u{1}b.txt", UNIX, "a_b.txt"),
            ("bad\u{FFFD}name", UNIX, "bad_name"),
            ("photo\u{202E}gpj.exe", UNIX, "photo_gpj.exe"),
            ("a:b.txt", WINDOWS, "a_b.txt"),
            ("a:b.txt", UNIX, "a:b.txt"),
            ("ends.", WINDOWS, "ends_"),
            ("ends. ", WINDOWS, "ends__"),
            ("con.txt", WINDOWS, "con_.txt"),
            ("NUL", WINDOWS, "NUL_"),
            ("inner.zip!x.txt", UNIX, "inner.zip_x.txt"),
            (" lead", UNIX, "_lead"),
        ];
        for (name, rules, safe) in table {
            let (made, changed) = destination_name(name, &rules);
            assert_eq!(made, safe, "{name:?}");
            assert_eq!(changed, name != safe, "{name:?}");
        }
        let long = format!("{}.txt", "x".repeat(300));
        let (cut, changed) = destination_name(&long, &WINDOWS);
        assert!(
            changed && cut.ends_with(".txt") && cut.len() == 255,
            "{cut}"
        );
    }

    #[test]
    fn the_stem_is_the_archive_name_without_its_extension() {
        assert_eq!(archive_stem("course.zip"), "course");
        assert_eq!(archive_stem("logs.TAR.XZ"), "logs");
        assert_eq!(archive_stem("backup.7z.001"), "backup");
        assert_eq!(archive_stem("notes.txt.xz"), "notes.txt");
        assert_eq!(archive_stem("odd"), "odd files");
    }

    #[test]
    fn the_room_line_says_the_need_first() {
        let room = Room {
            needed: 40_000_000_000,
            free: Some(12_000_000_000),
            drive: "D:".to_owned(),
        };
        assert_eq!(
            room.refusal().unwrap(),
            "Needs 40.0 GB, only 12.0 GB free on D:."
        );
        assert!(!room.enough());
        let fine = Room {
            needed: 1,
            free: None,
            drive: String::new(),
        };
        assert!(fine.enough() && fine.refusal().is_none());
        assert_eq!(
            skipped_line("../x", "name leaves the archive"),
            "Skipped ../x: name leaves the archive."
        );
        assert_eq!(renamed_line(3), "Renamed 3 names: unsafe characters.");
    }
}
