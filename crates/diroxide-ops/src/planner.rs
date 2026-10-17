//! The planner: turns a request into a [`Plan`] without touching anything
//! (plan section 5.4; research/file-operations.md, "Changes to the briefs",
//! W1b; research/shell-techniques.md, the removal rules).
//!
//! - Every source is read through its provider and walked with the
//!   boundary rule ([`crate::walk`]); every path the job will touch is a
//!   step, and the entry seen there is kept for the identity re-check.
//! - The anchors are refused before any listing: a drive or file-system
//!   root, a mount point, and (for a permanent delete) a protected folder;
//!   the trash or a move of a protected folder asks for its name typed.
//! - The preflight asks before the start: not enough room, a file larger
//!   than the destination's largest, a name the destination refuses, items
//!   the trash cannot take.
//! - A copy or move of a folder into itself is refused; the same file is
//!   left out with "Same file: NAME. Nothing to do."
//! - A large permanent delete asks for its count typed.

use std::collections::HashMap;
use std::sync::Arc;

use diroxide_core::{
    ConflictPolicy, Entry, EntryKind, FrozenSelection, JobKind, JobRequest, JobState, Place,
};
use diroxide_vfs::names::{NameRules, check_name_with, unused_name};
use diroxide_vfs::{
    LinkKind, LinkRequest, OpContext, Provider, TrashFacts, TrashGate, TrashRules, VfsError,
    trash_gate,
};

use crate::engine::{Engine, JobControl, JobSink, Quiet};
use crate::error::OpsError;
use crate::plan::{Plan, Preflight, Step};
use crate::progress::JobProgress;
use crate::undo::UndoRecord;
use crate::walk::{Found, walk};

/// Why an anchor is being checked: what the job would do to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Removal {
    /// Delete for good: no override.
    Permanent,
    /// To the trash, or moved away: a protected folder asks for its name.
    Recoverable,
}

/// One planning pass.
struct Planner<'a> {
    engine: &'a Engine,
    request: &'a JobRequest,
    control: &'a JobControl,
    sink: &'a dyn JobSink,
    ctx: OpContext,
    plan: Plan,
    counted: u64,
}

impl<'a> Planner<'a> {
    fn new(
        engine: &'a Engine,
        request: &'a JobRequest,
        control: &'a JobControl,
        sink: &'a dyn JobSink,
    ) -> Self {
        Self {
            engine,
            request,
            control,
            sink,
            ctx: OpContext::new().with_cancel(control.cancel.clone()),
            plan: Plan::new(request.id, request.kind, Vec::new(), request.conflicts),
            counted: 0,
        }
    }

    fn provider(&self, place: &Place) -> Result<Arc<dyn Provider>, OpsError> {
        Ok(self.engine.registry().resolve(place)?)
    }

    fn stat(&self, place: &Place) -> Result<Entry, OpsError> {
        Ok(self.provider(place)?.stat(place, &self.ctx)?)
    }

    fn destination(&self) -> Result<Place, OpsError> {
        self.request
            .destination
            .clone()
            .ok_or(OpsError::NoDestination)
    }

    fn name_of(place: &Place) -> Result<String, OpsError> {
        place
            .name()
            .map(str::to_owned)
            .ok_or(OpsError::NothingSelected)
    }

    /// Says the count so far while counting, every few hundred entries,
    /// and stops when the job is cancelled.
    fn counting(&mut self, total: u64) -> Result<(), VfsError> {
        self.control.cancel.check()?;
        if total.is_multiple_of(500) {
            let mut progress = JobProgress::start(self.request.kind, 0, None);
            progress.items_done = self.counted + total;
            self.sink.progress(&progress);
        }
        Ok(())
    }

    fn walk(
        &mut self,
        provider: &dyn Provider,
        root: &Place,
        entry: Entry,
    ) -> Result<Vec<Found>, OpsError> {
        let max_depth = self.engine.options().max_depth;
        let ctx = self.ctx.clone();
        let mut seen = 0;
        let walked = walk(provider, root, entry, &ctx, max_depth, &mut |total| {
            seen = total;
            self.counting(total)
        })?;
        self.counted += seen;
        self.plan.facts.left_out.extend(walked.left_out);
        Ok(walked.found)
    }

    /// A folder that must exist and be a folder: the destination.
    fn folder(&self, place: &Place) -> Result<Entry, OpsError> {
        let entry = self.stat(place)?;
        if entry.kind == EntryKind::Folder {
            Ok(entry)
        } else {
            Err(VfsError::NotAFolder.into())
        }
    }

    /// Refuses a place no tree operation may take as its root: a drive or
    /// file-system root, a mount point, a protected folder or one that
    /// holds one. A protected folder asks for its name typed when the
    /// job can be undone.
    fn anchor(&mut self, place: &Place, removal: Removal) -> Result<(), OpsError> {
        let spelled = diroxide_core::names::spell_out(&place.to_string()).into_owned();
        if place.is_root() {
            return Err(OpsError::Refused(if place.to_string() == "/" {
                "Refused: this is the root folder, /".to_owned()
            } else {
                format!("Refused: this is a drive root, {spelled}")
            }));
        }
        let provider = self.provider(place)?;
        if let Ok(volume) = provider.volume(place, &self.ctx)
            && volume.root.is_within(place)
            && place.is_within(&volume.root)
        {
            return Err(OpsError::Refused(format!(
                "Refused: another drive is mounted here, {spelled}"
            )));
        }
        let home = std::env::home_dir()
            .and_then(|path| path.to_str().map(str::to_owned))
            .and_then(|text| Place::parse(&text).ok());
        let protected: Vec<Place> = self
            .engine
            .options()
            .protected
            .iter()
            .chain(home.iter())
            .cloned()
            .collect();
        for guarded in &protected {
            // The place is the protected folder itself, or holds it.
            if !guarded.is_within(place) {
                continue;
            }
            let exact = place.is_within(guarded);
            if removal == Removal::Permanent || !exact {
                return Err(OpsError::Refused(if !exact {
                    format!("Refused: it holds a protected folder, {spelled}")
                } else if home.as_ref() == Some(guarded) {
                    "Refused: this is your home folder".to_owned()
                } else {
                    format!("Refused: this folder is protected, {spelled}")
                }));
            }
            self.plan.facts.typed = Some(Self::name_of(place)?);
        }
        Ok(())
    }

    /// Whether two entries are the same entry: the same volume and inode
    /// where both are known, else the same place.
    fn same_entry(a: &Entry, b: &Entry, a_place: &Place, b_place: &Place) -> bool {
        match (a.volume.zip(a.inode), b.volume.zip(b.inode)) {
            (Some(x), Some(y)) => x == y,
            _ => a_place.is_within(b_place) && b_place.is_within(a_place),
        }
    }

    /// Leaves out a source whose name the place could not read as
    /// Unicode: every operation on it is refused, with its words.
    fn unreadable(&mut self, source: &Place, entry: &Entry) -> bool {
        if entry.name_not_unicode {
            self.plan.facts.left_out.push((
                source.clone(),
                "the name is not valid Unicode; rename it with another program first".to_owned(),
            ));
        }
        entry.name_not_unicode
    }

    fn same_file_line(name: &str) -> String {
        format!(
            "Same file: {}. Nothing to do.",
            diroxide_core::names::spell_out(name)
        )
    }

    /// The target of a found entry, under the target root.
    fn target_of(target_root: &Place, found: &Found) -> Result<Place, OpsError> {
        let mut place = target_root.clone();
        for name in &found.path {
            place = place.join(name).map_err(VfsError::from)?;
        }
        Ok(place)
    }

    /// The questions about names the destination refuses.
    fn check_target_names(&mut self, rules: &NameRules, targets: &[Place]) {
        for target in targets {
            let Some(name) = target.name() else {
                continue;
            };
            if let Some(issue) = check_name_with(name, &[], rules)
                .into_iter()
                .find(|issue| issue.refused(rules))
            {
                self.plan.facts.questions.push(Preflight::NameRefused {
                    place: target.clone(),
                    line: issue.line(),
                });
            }
        }
    }

    /// The questions about room: free space against the bytes written,
    /// and each file against the largest file the destination takes.
    fn check_room(
        &mut self,
        provider: &dyn Provider,
        folder: &Place,
        needed: u64,
        files: &[(Place, u64)],
    ) {
        let Ok(space) = provider.space(folder, &self.ctx) else {
            return;
        };
        if let Some(free) = space.free_to_user
            && needed > free
        {
            let drive = provider
                .volume(folder, &self.ctx)
                .map_or_else(|_| folder.clone(), |volume| volume.root);
            self.plan.facts.questions.push(Preflight::NotEnoughRoom {
                drive,
                needed,
                free,
            });
        }
        if let Some(max) = space.max_file_size {
            for (place, size) in files {
                if *size > max {
                    self.plan.facts.questions.push(Preflight::TooLarge {
                        place: place.clone(),
                        size: *size,
                        max,
                    });
                }
            }
        }
    }

    /// Copy and duplicate: every entry under each source becomes a copy
    /// step, folders before their contents.
    fn copy(&mut self, duplicate: bool) -> Result<(), OpsError> {
        let folder = if duplicate {
            None
        } else {
            Some(self.destination()?)
        };
        let mut written = 0u64;
        let mut files = Vec::new();
        let mut targets = Vec::new();
        let mut clone_only = true;
        let mut destination = None;
        let request = self.request;
        for source in &request.sources {
            let name = Self::name_of(source)?;
            let source_provider = self.provider(source)?;
            let entry = source_provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            let dest_folder = match &folder {
                Some(folder) => folder.clone(),
                None => source.parent().ok_or(OpsError::NothingSelected)?,
            };
            let dest_provider = self.provider(&dest_folder)?;
            let dest_entry = self.folder(&dest_folder)?;
            let target_name = if duplicate {
                match (&request.new_name, request.sources.len()) {
                    (Some(new_name), 1) => new_name.clone(),
                    _ => {
                        let siblings: Vec<String> = dest_provider
                            .list(&dest_folder, &self.ctx)?
                            .into_iter()
                            .map(|e| e.name)
                            .collect();
                        let siblings: Vec<&str> = siblings.iter().map(String::as_str).collect();
                        unused_name(&name, &siblings).ok_or_else(|| {
                            OpsError::NameRefused(format!("No free name for a copy of {name}"))
                        })?
                    }
                }
            } else {
                name.clone()
            };
            let target = dest_folder.join(&target_name).map_err(VfsError::from)?;
            if entry.kind == EntryKind::Folder && target.is_within(source) {
                return Err(OpsError::IntoItself(name));
            }
            if Self::same_target(&*dest_provider, source, &entry, &target, &self.ctx) {
                self.plan
                    .facts
                    .left_out
                    .push((source.clone(), Self::same_file_line(&name)));
                continue;
            }
            let same_volume = entry.volume.is_some() && entry.volume == dest_entry.volume;
            let clones = Arc::ptr_eq(&source_provider, &dest_provider)
                && same_volume
                && dest_provider.capabilities(&dest_folder).clone_copy;
            clone_only &= clones;
            destination.get_or_insert_with(|| dest_folder.clone());
            let found = self.walk(&*source_provider, source, entry)?;
            for item in found {
                let to = Self::target_of(&target, &item)?;
                if item.entry.kind != EntryKind::Folder && item.entry.kind != EntryKind::Link {
                    let size = item.entry.size.unwrap_or(0);
                    written += size;
                    files.push((item.place.clone(), size));
                }
                targets.push(to.clone());
                self.plan.facts.count(&item.place, &item.entry);
                self.plan.expected.insert(item.place.clone(), item.entry);
                self.plan.steps.push(Step::Copy {
                    from: item.place,
                    to,
                });
            }
        }
        self.plan.facts.counted = true;
        self.plan.facts.destination = destination.clone();
        if let Some(dest_folder) = destination {
            let dest_provider = self.provider(&dest_folder)?;
            let rules = NameRules::from_capabilities(&dest_provider.capabilities(&dest_folder));
            self.check_target_names(&rules, &targets);
            if !clone_only {
                self.check_room(&*dest_provider, &dest_folder, written, &files);
            }
        }
        Ok(())
    }

    /// Whether a target is the source itself: the same place, or the
    /// same entry by identity (a hard link, a case alias).
    fn same_target(
        provider: &dyn Provider,
        source: &Place,
        entry: &Entry,
        target: &Place,
        ctx: &OpContext,
    ) -> bool {
        if source.is_within(target) && target.is_within(source) {
            return true;
        }
        match provider.stat(target, ctx) {
            Ok(existing) => {
                entry.inode.is_some() && Self::same_entry(entry, &existing, source, target)
            }
            Err(_) => false,
        }
    }

    /// Move: a rename where source and target are on one drive of one
    /// provider; across drives or providers, or onto a folder that is
    /// there (a merge), entry by entry: copy each folder, move each file,
    /// then remove each emptied folder, deepest first.
    fn moving(&mut self) -> Result<(), OpsError> {
        let folder = self.destination()?;
        let dest_provider = self.provider(&folder)?;
        let dest_entry = self.folder(&folder)?;
        let mut whole_count = true;
        let mut written = 0u64;
        let mut files = Vec::new();
        let mut targets = Vec::new();
        let mut crosses = false;
        let request = self.request;
        for source in &request.sources {
            self.anchor(source, Removal::Recoverable)?;
            let name = Self::name_of(source)?;
            let source_provider = self.provider(source)?;
            let entry = source_provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            let target = folder.join(&name).map_err(VfsError::from)?;
            if entry.kind == EntryKind::Folder && target.is_within(source) {
                return Err(OpsError::IntoItself(name));
            }
            if Self::same_target(&*dest_provider, source, &entry, &target, &self.ctx) {
                self.plan
                    .facts
                    .left_out
                    .push((source.clone(), Self::same_file_line(&name)));
                continue;
            }
            let same_provider = Arc::ptr_eq(&source_provider, &dest_provider);
            let same_volume = match (entry.volume, dest_entry.volume) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            };
            let merge = entry.kind == EntryKind::Folder
                && dest_provider
                    .stat(&target, &self.ctx)
                    .is_ok_and(|existing| existing.kind == EntryKind::Folder);
            targets.push(target.clone());
            if same_provider && same_volume && !merge {
                if entry.kind == EntryKind::Folder {
                    whole_count = false;
                }
                self.plan.facts.count(source, &entry);
                self.plan.expected.insert(source.clone(), entry);
                self.plan.steps.push(Step::Move {
                    from: source.clone(),
                    to: target,
                });
                continue;
            }
            crosses |= !(same_provider && same_volume);
            let found = self.walk(&*source_provider, source, entry)?;
            let mut emptied = Vec::new();
            for item in found {
                let to = Self::target_of(&target, &item)?;
                self.plan.facts.count(&item.place, &item.entry);
                self.plan
                    .expected
                    .insert(item.place.clone(), item.entry.clone());
                if item.entry.kind == EntryKind::Folder {
                    emptied.push((item.depth(), item.place.clone()));
                    self.plan.steps.push(Step::Copy {
                        from: item.place,
                        to,
                    });
                } else {
                    if item.entry.kind != EntryKind::Link {
                        let size = item.entry.size.unwrap_or(0);
                        written += size;
                        files.push((item.place.clone(), size));
                    }
                    self.plan.steps.push(Step::Move {
                        from: item.place,
                        to,
                    });
                }
            }
            emptied.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
            self.plan.steps.extend(
                emptied
                    .into_iter()
                    .map(|(_, place)| Step::RemoveEmptyFolder { place }),
            );
        }
        self.plan.facts.counted = whole_count;
        self.plan.facts.destination = Some(folder.clone());
        let rules = NameRules::from_capabilities(&dest_provider.capabilities(&folder));
        self.check_target_names(&rules, &targets);
        if crosses {
            self.check_room(&*dest_provider, &folder, written, &files);
        }
        Ok(())
    }

    /// To the trash: one step per selected item, the trash unit; folders
    /// are walked to count what is inside and to find the longest path
    /// the gate must allow. Items the gate refuses are left out, and the
    /// question offers the listed permanent delete as its own job.
    fn trash(&mut self) -> Result<(), OpsError> {
        let mut refused: Vec<(TrashGate, Place)> = Vec::new();
        let request = self.request;
        for source in &request.sources {
            self.anchor(source, Removal::Recoverable)?;
            Self::name_of(source)?;
            let provider = self.provider(source)?;
            let entry = provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            let mut gate = provider.trash_gate(source, &self.ctx)?;
            let found = self.walk(&*provider, source, entry.clone())?;
            for item in &found {
                self.plan.facts.count(&item.place, &item.entry);
            }
            if gate.allows() && entry.kind == EntryKind::Folder {
                let longest = found
                    .iter()
                    .map(|item| diroxide_vfs::limits::utf16_units(&item.place.to_string()))
                    .max()
                    .unwrap_or(0);
                let volume = provider
                    .volume(source, &self.ctx)
                    .map_or(diroxide_vfs::VolumeKind::Fixed, |v| v.kind);
                let inner = trash_gate(
                    &TrashFacts {
                        volume,
                        longest_path_units: longest,
                        network_path: source.remote().is_some(),
                        // Only the inner paths' length is checked here; the
                        // provider's own gate said whether it copies.
                        copies_across_drives: false,
                    },
                    TrashRules::native(),
                );
                if matches!(inner, TrashGate::TooLong { .. }) {
                    gate = inner;
                }
            }
            if gate.allows() {
                self.plan.expected.insert(source.clone(), entry);
                self.plan.steps.push(Step::Trash {
                    place: source.clone(),
                });
            } else {
                self.plan.facts.left_out.push((source.clone(), gate.why()));
                refused.push((gate, source.clone()));
            }
        }
        self.plan.facts.counted = true;
        if let Some((gate, _)) = refused.first() {
            let gate = *gate;
            self.plan.facts.questions.push(Preflight::NoTrash {
                gate,
                places: refused.into_iter().map(|(_, place)| place).collect(),
            });
        }
        Ok(())
    }

    /// Delete for good: every path listed, files and links first, then
    /// folders deepest first, each with a call that never recurses, so a
    /// folder with anything unlisted inside stays.
    fn delete(&mut self) -> Result<(), OpsError> {
        let request = self.request;
        for source in &request.sources {
            self.anchor(source, Removal::Permanent)?;
            Self::name_of(source)?;
            let provider = self.provider(source)?;
            let entry = provider.stat(source, &self.ctx)?;
            if self.unreadable(source, &entry) {
                continue;
            }
            let found = self.walk(&*provider, source, entry)?;
            let mut folders = Vec::new();
            for item in found {
                self.plan.facts.count(&item.place, &item.entry);
                self.plan
                    .expected
                    .insert(item.place.clone(), item.entry.clone());
                if item.entry.kind == EntryKind::Folder {
                    folders.push((item.depth(), item.place));
                } else {
                    self.plan.steps.push(Step::Remove { place: item.place });
                }
            }
            folders.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
            self.plan
                .steps
                .extend(folders.into_iter().map(|(_, place)| Step::Remove { place }));
        }
        self.plan.facts.counted = true;
        let options = self.engine.options();
        let entries = self.plan.facts.entries();
        let items = request.sources.len() as u64;
        if entries > options.typed_count_over
            || (items > 0 && entries > items.saturating_mul(options.typed_count_ratio))
        {
            self.plan.facts.typed = Some(entries.to_string());
        }
        Ok(())
    }

    /// Checks one new name against the folder's rules; a refused name
    /// stops the plan.
    fn check_new_name(&self, folder: &Place, name: &str) -> Result<(), OpsError> {
        let provider = self.provider(folder)?;
        let rules = NameRules::from_capabilities(&provider.capabilities(folder));
        if let Some(issue) = check_name_with(name, &[], &rules)
            .into_iter()
            .find(|issue| issue.refused(&rules))
        {
            return Err(OpsError::NameRefused(issue.line()));
        }
        Ok(())
    }

    /// Rename, one or many: each pair checked, then ordered so a name is
    /// free before it is taken. A chain runs from its free end; a cycle
    /// (a swap) sends one member through a temporary name, and runs whole,
    /// with no cancel inside it.
    fn rename(&mut self) -> Result<(), OpsError> {
        let pairs: Vec<(Place, String)> = if self.request.renames.is_empty() {
            let name = self.request.new_name.clone().ok_or(OpsError::NoName)?;
            let source = self
                .request
                .sources
                .first()
                .ok_or(OpsError::NothingSelected)?;
            vec![(source.clone(), name)]
        } else {
            self.request.renames.clone()
        };
        let mut folders: Vec<(Place, Vec<Row>)> = Vec::new();
        for (from, name) in pairs {
            let folder = from.parent().ok_or(OpsError::NothingSelected)?;
            self.check_new_name(&folder, &name)?;
            let to = folder.join(&name).map_err(|_| {
                OpsError::NameRefused(format!("A name cannot hold a slash: {name}"))
            })?;
            let entry = self.stat(&from)?;
            if self.unreadable(&from, &entry) {
                continue;
            }
            if from.to_string() == to.to_string() {
                self.plan
                    .facts
                    .left_out
                    .push((from.clone(), "unchanged: the name is the same".to_owned()));
                continue;
            }
            let insensitive = !self.provider(&folder)?.capabilities(&folder).case_sensitive;
            let fold = |text: &str| {
                if insensitive {
                    text.to_lowercase()
                } else {
                    text.to_owned()
                }
            };
            let row = Row {
                old_key: fold(from.name().unwrap_or_default()),
                new_key: fold(&name),
                from: from.clone(),
                to,
            };
            self.plan.facts.count(&from, &entry);
            self.plan.expected.insert(from, entry);
            match folders.iter_mut().find(|(f, _)| *f == folder) {
                Some((_, rows)) => rows.push(row),
                None => folders.push((folder, vec![row])),
            }
        }
        let job = self.request.id.get();
        let mut hop = 0;
        for (folder, rows) in folders {
            for (i, row) in rows.iter().enumerate() {
                if rows[..i].iter().any(|other| other.new_key == row.new_key) {
                    return Err(OpsError::NameRefused(format!(
                        "Two items would both be named {}",
                        row.to.name().unwrap_or_default()
                    )));
                }
            }
            let siblings: Vec<String> = self
                .provider(&folder)?
                .list(&folder, &self.ctx)
                .map(|entries| entries.into_iter().map(|e| e.name).collect())
                .unwrap_or_default();
            for group in order_renames(&rows) {
                let start = self.plan.steps.len();
                let cycle = group.cycle;
                for (position, index) in group.rows.iter().enumerate() {
                    let row = &rows[*index];
                    if cycle && position == 0 {
                        let via = loop {
                            hop += 1;
                            let name = format!("diro-tmp-{job}-{hop}");
                            if !siblings.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                                break folder.join(&name).map_err(VfsError::from)?;
                            }
                        };
                        self.plan.steps.push(Step::RenameHop {
                            from: row.from.clone(),
                            via,
                            to: row.to.clone(),
                        });
                    } else {
                        self.plan.steps.push(Step::Rename {
                            from: row.from.clone(),
                            to: row.to.clone(),
                        });
                    }
                }
                if cycle {
                    self.plan.facts.whole.push((start, self.plan.steps.len()));
                }
            }
        }
        Ok(())
    }

    /// Make folder: the name may hold separators, one step per level.
    fn make_folder(&mut self) -> Result<(), OpsError> {
        let base = self.destination()?;
        let name = self.request.new_name.clone().ok_or(OpsError::NoName)?;
        let windows_path = base.local_path().is_some_and(|path| !path.starts_with('/'));
        let parts: Vec<&str> = name
            .split(|c| c == '/' || (windows_path && c == '\\'))
            .collect();
        let mut place = base;
        for part in parts {
            if part.is_empty() || part == "." || part == ".." {
                return Err(OpsError::NameRefused(
                    "A folder name cannot be empty, or a dot".to_owned(),
                ));
            }
            self.check_new_name(&place, part)?;
            place = place.join(part).map_err(VfsError::from)?;
            if let Ok(existing) = self.stat(&place)
                && existing.kind != EntryKind::Folder
            {
                return Err(OpsError::NameRefused(format!(
                    "Cannot make {part}: a file has that name"
                )));
            }
            self.plan.steps.push(Step::MakeFolder {
                place: place.clone(),
            });
        }
        self.plan.facts.destination = place.parent();
        Ok(())
    }

    /// Links: one link in the destination for each source, pointing at
    /// the source's full place.
    fn link(&mut self, kind: LinkKind) -> Result<(), OpsError> {
        let folder = self.destination()?;
        self.folder(&folder)?;
        let capabilities = self.provider(&folder)?.capabilities(&folder);
        if !capabilities.can_link() {
            return Err(OpsError::CannotLink(
                capabilities.links_in_words().to_owned(),
            ));
        }
        let request = self.request;
        for source in &request.sources {
            let name = match (&request.new_name, request.sources.len()) {
                (Some(name), 1) => name.clone(),
                _ => Self::name_of(source)?,
            };
            self.check_new_name(&folder, &name)?;
            let entry = self.stat(source)?;
            let place = folder.join(&name).map_err(VfsError::from)?;
            self.plan.steps.push(Step::Link {
                place,
                request: LinkRequest {
                    kind,
                    target: source.to_string(),
                    target_is_folder: entry.kind == EntryKind::Folder,
                },
            });
        }
        self.plan.facts.destination = Some(folder);
        Ok(())
    }

    fn finish(mut self) -> Plan {
        self.plan.facts.origin = self.request.origin.clone();
        self.plan.facts.items = self.request.sources.len().max(self.request.renames.len()) as u64;
        self.plan
    }
}

/// One rename of a batch, with its names folded by the folder's rule.
struct Row {
    from: Place,
    to: Place,
    old_key: String,
    new_key: String,
}

/// A group of renames in their order: a chain, or a cycle whose first
/// member goes through a temporary name.
struct Group {
    rows: Vec<usize>,
    cycle: bool,
}

/// Orders a folder's renames: a row waits for the row whose old name it
/// takes. Chains run from their free end; what is left are cycles.
fn order_renames(rows: &[Row]) -> Vec<Group> {
    let old: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (row.old_key.as_str(), i))
        .collect();
    let waits_for: Vec<Option<usize>> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| old.get(row.new_key.as_str()).copied().filter(|&j| j != i))
        .collect();
    let mut done = vec![false; rows.len()];
    let mut groups = Vec::new();
    loop {
        let mut progressed = false;
        for i in 0..rows.len() {
            if !done[i] && waits_for[i].is_none_or(|j| done[j]) {
                done[i] = true;
                progressed = true;
                groups.push(Group {
                    rows: vec![i],
                    cycle: false,
                });
            }
        }
        if !progressed {
            break;
        }
    }
    for start in 0..rows.len() {
        if done[start] {
            continue;
        }
        // A cycle: the start leaves through a temporary name; then each
        // row waiting on the last one moved, until the cycle closes.
        let mut members = vec![start];
        done[start] = true;
        let mut last = start;
        while let Some(next) = (0..rows.len()).find(|&k| !done[k] && waits_for[k] == Some(last)) {
            done[next] = true;
            members.push(next);
            last = next;
        }
        groups.push(Group {
            rows: members,
            cycle: true,
        });
    }
    groups
}

impl Engine {
    /// Computes what a job will do, touching nothing: every path walked,
    /// counted and listed. Undo is planned from the undo log with
    /// [`Engine::plan_undo`], never from a request.
    pub fn plan(&self, request: &JobRequest) -> Result<Plan, OpsError> {
        self.plan_with(request, &JobControl::new(), &Quiet)
    }

    /// [`Engine::plan`], saying the count while it walks ("Copying:
    /// counting", with the count in `items_done`) and stopping when the
    /// control is cancelled.
    pub fn plan_with(
        &self,
        request: &JobRequest,
        control: &JobControl,
        sink: &dyn JobSink,
    ) -> Result<Plan, OpsError> {
        match request.kind {
            JobKind::Copy | JobKind::Move | JobKind::Link if request.destination.is_none() => {
                return Err(OpsError::NoDestination);
            }
            JobKind::MakeFolder if request.destination.is_none() => {
                return Err(OpsError::NoDestination);
            }
            JobKind::MakeFolder if request.new_name.is_none() => return Err(OpsError::NoName),
            JobKind::Rename if request.renames.is_empty() && request.new_name.is_none() => {
                return Err(OpsError::NoName);
            }
            JobKind::Undo => return Err(OpsError::NothingToUndo),
            JobKind::Copy
            | JobKind::Move
            | JobKind::Duplicate
            | JobKind::Trash
            | JobKind::DeletePermanently
            | JobKind::Rename
            | JobKind::MakeFolder
            | JobKind::Link => {}
            other => return Err(OpsError::NotBuilt(other.verb().to_owned())),
        }
        if request.sources.is_empty()
            && request.renames.is_empty()
            && request.kind != JobKind::MakeFolder
        {
            return Err(OpsError::NothingSelected);
        }
        let mut planner = Planner::new(self, request, control, sink);
        match request.kind {
            JobKind::Copy => planner.copy(false)?,
            JobKind::Duplicate => planner.copy(true)?,
            JobKind::Move => planner.moving()?,
            JobKind::Trash => planner.trash()?,
            JobKind::DeletePermanently => planner.delete()?,
            JobKind::Rename => planner.rename()?,
            JobKind::MakeFolder => planner.make_folder()?,
            _ => planner.link(LinkKind::Symbolic)?,
        }
        let mut progress = JobProgress::start(request.kind, 0, None);
        progress.items_done = planner.counted;
        progress.state = JobState::Counting;
        sink.progress(&progress);
        Ok(planner.finish())
    }

    /// Plans a job on a selection frozen at the moment of confirmation:
    /// the sources are the frozen ids in `folder`, by id, so a list
    /// refreshed since can never change what the plan acts on; and when
    /// the focused row is not part of the action, the plan's first line
    /// says so ("Not the focused row: report.pdf").
    pub fn plan_frozen(
        &self,
        request: &JobRequest,
        folder: &Place,
        selection: &FrozenSelection,
    ) -> Result<Plan, OpsError> {
        if selection.ids.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let mut frozen = request.clone();
        frozen.sources = selection
            .ids
            .iter()
            .map(|id| folder.join(id).map_err(|e| OpsError::Vfs(e.into())))
            .collect::<Result<_, _>>()?;
        let mut plan = self.plan(&frozen)?;
        plan.note = selection.focus_note();
        Ok(plan)
    }

    /// Plans links of a chosen kind (symbolic, junction or hard) for a
    /// link request; [`Engine::plan`] makes symbolic links.
    pub fn plan_links(&self, request: &JobRequest, kind: LinkKind) -> Result<Plan, OpsError> {
        if request.destination.is_none() {
            return Err(OpsError::NoDestination);
        }
        if request.sources.is_empty() {
            return Err(OpsError::NothingSelected);
        }
        let quiet = Quiet;
        let control = JobControl::new();
        let mut planner = Planner::new(self, request, &control, &quiet);
        planner.plan.kind = JobKind::Link;
        planner.link(kind)?;
        Ok(planner.finish())
    }

    /// Plans the undo of a job's record: what it made is trashed if
    /// unchanged and its folders removed if empty, its moves are moved
    /// back, and what it trashed is restored. Each step is checked at undo
    /// time against what the job left, and skipped with its reason when
    /// the entry changed since.
    pub fn plan_undo(
        &self,
        record: &UndoRecord,
        job: diroxide_core::JobId,
    ) -> Result<Plan, OpsError> {
        if record.is_empty() {
            return Err(OpsError::NothingToUndo);
        }
        let ctx = OpContext::new();
        let folders: Vec<Place> = record
            .created
            .iter()
            .filter(|place| {
                self.registry()
                    .resolve(place)
                    .and_then(|provider| provider.stat(place, &ctx))
                    .is_ok_and(|entry| entry.kind == EntryKind::Folder)
            })
            .cloned()
            .collect();
        let mut plan = record.undo_plan(job, &folders);
        plan.conflicts = ConflictPolicy::Skip;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeProvider, FakeSwitches};
    use crate::options::EngineOptions;
    use crate::testing::{Script, engine, p};
    use diroxide_core::format::within_cells;
    use diroxide_core::{JobId, ListModel, ListRow};

    fn request(kind: JobKind, sources: &[&str]) -> JobRequest {
        JobRequest::new(JobId::new(7), kind, sources.iter().map(|s| p(s)).collect())
    }

    fn tree() -> FakeProvider {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/photos/one.jpg"), b"1111");
        fake.add_file(&p("/a/photos/trip/two.jpg"), b"22");
        fake.add_file(&p("/a/notes.txt"), b"notes");
        fake.add_folder(&p("/b"));
        fake
    }

    fn steps(plan: &Plan) -> Vec<String> {
        plan.steps.iter().map(Step::describe).collect()
    }

    #[test]
    fn a_copy_walks_every_entry_folders_first() {
        let fake = tree();
        let plan = engine(&fake)
            .plan(&request(JobKind::Copy, &["/a/photos", "/a/notes.txt"]).to(p("/b")))
            .unwrap();
        let from: Vec<Place> = plan.steps.iter().map(|s| s.subject().clone()).collect();
        assert_eq!(
            from,
            vec![
                p("/a/photos"),
                p("/a/photos/one.jpg"),
                p("/a/photos/trip"),
                p("/a/photos/trip/two.jpg"),
                p("/a/notes.txt")
            ]
        );
        assert_eq!(
            plan.steps[3],
            Step::Copy {
                from: p("/a/photos/trip/two.jpg"),
                to: p("/b/photos/trip/two.jpg")
            }
        );
        assert_eq!((plan.facts.files, plan.facts.folders), (3, 2));
        assert_eq!(plan.facts.bytes, 11);
        assert_eq!(plan.facts.items, 2);
        assert_eq!(plan.expected.len(), 5);
        assert!(plan.summary().starts_with("Copy 5 items to "));
        assert!(plan.summary().ends_with(", 11 bytes"));
        assert!(plan.facts.questions.is_empty());
    }

    #[test]
    fn a_folder_cannot_go_inside_itself_and_the_same_file_is_left_out() {
        let fake = tree();
        let engine = engine(&fake);
        assert_eq!(
            engine.plan(&request(JobKind::Copy, &["/a/photos"]).to(p("/a/photos/trip"))),
            Err(OpsError::IntoItself("photos".into()))
        );
        assert_eq!(
            engine.plan(&request(JobKind::Move, &["/a/photos"]).to(p("/a/photos"))),
            Err(OpsError::IntoItself("photos".into()))
        );
        let same = engine
            .plan(&request(JobKind::Copy, &["/a/notes.txt"]).to(p("/a")))
            .unwrap();
        assert!(same.steps.is_empty());
        assert_eq!(
            same.facts.left_out[0].1,
            "Same file: notes.txt. Nothing to do."
        );
        fake.add_hard_link(&p("/b/notes.txt"), &p("/a/notes.txt"));
        let alias = engine
            .plan(&request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")))
            .unwrap();
        assert!(alias.steps.is_empty(), "a hard-link alias is the same file");
    }

    #[test]
    fn the_preflight_asks_about_room_size_and_names() {
        let fake = tree();
        fake.switches_mut().free_space = Some(5);
        fake.switches_mut().max_file_size = Some(4);
        fake.switches_mut().reserved_names = true;
        fake.add_file(&p("/a/aux.txt"), b"x");
        let plan = engine(&fake)
            .plan(&request(JobKind::Copy, &["/a/photos", "/a/notes.txt", "/a/aux.txt"]).to(p("/b")))
            .unwrap();
        let firsts: Vec<String> = plan
            .facts
            .questions
            .iter()
            .map(|q| q.lines()[0].clone())
            .collect();
        assert!(
            firsts
                .iter()
                .any(|l| l.starts_with("Name not allowed there: aux.txt")),
            "{firsts:?}"
        );
        assert!(
            firsts.iter().any(|l| l.starts_with("Not enough room on")),
            "{firsts:?}"
        );
        assert!(
            firsts.contains(&"Too large for this drive: notes.txt".to_owned()),
            "{firsts:?}"
        );
        for line in &firsts {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }

    #[test]
    fn a_clone_on_one_drive_needs_no_room_check() {
        let fake = tree();
        fake.switches_mut().free_space = Some(1);
        fake.switches_mut().clone_copy = true;
        let plan = engine(&fake)
            .plan(&request(JobKind::Copy, &["/a/notes.txt"]).to(p("/b")))
            .unwrap();
        assert!(plan.facts.questions.is_empty());
    }

    #[test]
    fn a_move_on_one_drive_is_one_rename_and_across_drives_entry_by_entry() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            second_volume: Some(p("/usb")),
            ..FakeSwitches::default()
        });
        fake.add_file(&p("/a/photos/one.jpg"), b"1");
        fake.add_file(&p("/a/photos/trip/two.jpg"), b"2");
        fake.add_folder(&p("/b"));
        fake.add_folder(&p("/usb"));
        let engine = engine(&fake);
        let same = engine
            .plan(&request(JobKind::Move, &["/a/photos"]).to(p("/b")))
            .unwrap();
        assert_eq!(
            same.steps,
            vec![Step::Move {
                from: p("/a/photos"),
                to: p("/b/photos")
            }]
        );
        assert!(!same.facts.counted);
        let across = engine
            .plan(&request(JobKind::Move, &["/a/photos"]).to(p("/usb")))
            .unwrap();
        let lines = steps(&across);
        assert!(lines[0].starts_with("copy "), "{lines:?}");
        assert!(lines[1].starts_with("move "), "{lines:?}");
        assert_eq!(
            &across.steps[across.steps.len() - 2..],
            &[
                Step::RemoveEmptyFolder {
                    place: p("/a/photos/trip")
                },
                Step::RemoveEmptyFolder {
                    place: p("/a/photos")
                }
            ]
        );
        assert!(across.facts.counted);
        fake.add_folder(&p("/b/photos"));
        let merge = engine
            .plan(&request(JobKind::Move, &["/a/photos"]).to(p("/b")))
            .unwrap();
        assert!(merge.steps.len() > 1, "a folder over a folder merges");
    }

    #[test]
    fn the_trash_takes_whole_items_and_refuses_what_it_cannot_hold() {
        let fake = tree();
        let engine = engine(&fake);
        let plan = engine
            .plan(&request(JobKind::Trash, &["/a/photos", "/a/notes.txt"]))
            .unwrap();
        assert_eq!(
            plan.steps,
            vec![
                Step::Trash {
                    place: p("/a/photos")
                },
                Step::Trash {
                    place: p("/a/notes.txt")
                }
            ]
        );
        assert_eq!(plan.facts.entries(), 5, "what is inside is counted");
        assert_eq!(plan.summary(), "Move 5 items to the trash, 11 bytes");
        fake.switches_mut().no_trash = true;
        fake.switches_mut().volume_kind = Some(diroxide_vfs::VolumeKind::Removable);
        let refused = engine
            .plan(&request(JobKind::Trash, &["/a/notes.txt"]))
            .unwrap();
        assert!(refused.steps.is_empty(), "never falls through");
        assert_eq!(
            refused.facts.questions[0].lines()[0],
            "This drive has no trash. Delete 1 item for good? y or n"
        );
        assert_eq!(
            refused.facts.left_out[0].1,
            "this removable drive has no trash"
        );
    }

    #[test]
    fn a_permanent_delete_lists_files_first_then_folders_deepest_first() {
        let fake = tree();
        let plan = engine(&fake)
            .plan(&request(JobKind::DeletePermanently, &["/a/photos"]))
            .unwrap();
        assert_eq!(
            plan.steps,
            vec![
                Step::Remove {
                    place: p("/a/photos/one.jpg")
                },
                Step::Remove {
                    place: p("/a/photos/trip/two.jpg")
                },
                Step::Remove {
                    place: p("/a/photos/trip")
                },
                Step::Remove {
                    place: p("/a/photos")
                },
            ]
        );
        assert_eq!(plan.question().text, "Delete 4 items for good? y or n");
        assert!(plan.facts.typed.is_none());
        let options = EngineOptions {
            typed_count_over: 3,
            ..EngineOptions::default()
        };
        let typed = engine(&fake)
            .with_options(options)
            .plan(&request(JobKind::DeletePermanently, &["/a/photos"]))
            .unwrap();
        assert_eq!(typed.facts.typed.as_deref(), Some("4"));
        assert!(within_cells(&typed.question().text, "Type 4 to go on"));
    }

    #[test]
    fn the_anchors_are_refused_before_any_listing() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            second_volume: Some(p("/mnt")),
            ..FakeSwitches::default()
        });
        fake.add_file(&p("/mnt/x"), b"1");
        fake.add_file(&p("/home/ada/notes.txt"), b"1");
        let options = EngineOptions {
            protected: vec![p("/home/ada")],
            ..EngineOptions::default()
        };
        let engine = engine(&fake).with_options(options);
        let root = Place::parse(if cfg!(windows) { "D:\\" } else { "/" }).unwrap();
        let refused = engine
            .plan(&JobRequest::new(
                JobId::new(1),
                JobKind::DeletePermanently,
                vec![root],
            ))
            .unwrap_err();
        assert!(
            refused.to_string().starts_with("Refused: this is"),
            "{refused}"
        );
        let mount = engine
            .plan(&request(JobKind::DeletePermanently, &["/mnt"]))
            .unwrap_err();
        assert!(
            mount
                .to_string()
                .starts_with("Refused: another drive is mounted here")
        );
        let home = engine
            .plan(&request(JobKind::DeletePermanently, &["/home/ada"]))
            .unwrap_err();
        assert!(
            home.to_string()
                .starts_with("Refused: this folder is protected")
        );
        let holder = engine
            .plan(&request(JobKind::Trash, &["/home"]))
            .unwrap_err();
        assert!(
            holder
                .to_string()
                .starts_with("Refused: it holds a protected folder")
        );
        let typed = engine
            .plan(&request(JobKind::Trash, &["/home/ada"]))
            .unwrap();
        assert_eq!(typed.facts.typed.as_deref(), Some("ada"));
        assert!(
            engine
                .plan(&request(
                    JobKind::DeletePermanently,
                    &["/home/ada/notes.txt"]
                ))
                .is_ok(),
            "what is inside a protected folder is not an anchor"
        );
    }

    #[test]
    fn a_swap_goes_through_a_temporary_name_and_runs_whole() {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/x.txt"), b"x");
        fake.add_file(&p("/a/y.txt"), b"y");
        fake.add_file(&p("/a/z.txt"), b"z");
        let swap = JobRequest::new(JobId::new(9), JobKind::Rename, Vec::new()).renaming(vec![
            (p("/a/x.txt"), "y.txt".into()),
            (p("/a/y.txt"), "x.txt".into()),
            (p("/a/z.txt"), "w.txt".into()),
        ]);
        let plan = engine(&fake).plan(&swap).unwrap();
        assert_eq!(
            plan.steps[0],
            Step::Rename {
                from: p("/a/z.txt"),
                to: p("/a/w.txt")
            },
            "a free name first"
        );
        assert_eq!(
            plan.steps[1],
            Step::RenameHop {
                from: p("/a/x.txt"),
                via: p("/a/diro-tmp-9-1"),
                to: p("/a/y.txt")
            }
        );
        assert_eq!(
            plan.steps[2],
            Step::Rename {
                from: p("/a/y.txt"),
                to: p("/a/x.txt")
            }
        );
        assert_eq!(plan.facts.whole, vec![(1, 3)]);
    }

    #[test]
    fn a_chain_runs_from_its_free_end() {
        let fake = FakeProvider::new();
        fake.add_file(&p("/a/1"), b"1");
        fake.add_file(&p("/a/2"), b"2");
        let chain = JobRequest::new(JobId::new(3), JobKind::Rename, Vec::new())
            .renaming(vec![(p("/a/1"), "2".into()), (p("/a/2"), "3".into())]);
        let plan = engine(&fake).plan(&chain).unwrap();
        assert_eq!(plan.steps[0].subject(), &p("/a/2"));
        assert_eq!(plan.steps[1].subject(), &p("/a/1"));
        assert!(plan.facts.whole.is_empty());
    }

    #[test]
    fn rename_refuses_bad_names_and_twins() {
        let fake = FakeProvider::with_switches(FakeSwitches {
            reserved_names: true,
            case_insensitive: true,
            ..FakeSwitches::default()
        });
        fake.add_file(&p("/a/x"), b"x");
        fake.add_file(&p("/a/y"), b"y");
        let engine = engine(&fake);
        let named = |name: &str| request(JobKind::Rename, &["/a/x"]).named(name);
        assert!(matches!(
            engine.plan(&named("notes.")),
            Err(OpsError::NameRefused(line)) if line == "Name needs care: ends in a dot or space"
        ));
        assert!(matches!(
            engine.plan(&named("con")),
            Err(OpsError::NameRefused(_))
        ));
        assert!(matches!(
            engine.plan(&named("..")),
            Err(OpsError::NameRefused(_))
        ));
        let twins = JobRequest::new(JobId::new(4), JobKind::Rename, Vec::new())
            .renaming(vec![(p("/a/x"), "Same".into()), (p("/a/y"), "same".into())]);
        assert!(matches!(engine.plan(&twins), Err(OpsError::NameRefused(_))));
        let case_only = engine.plan(&named("X")).unwrap();
        assert_eq!(
            case_only.steps,
            vec![Step::Rename {
                from: p("/a/x"),
                to: p("/a/X")
            }],
            "a case-only rename is direct"
        );
    }

    #[test]
    fn make_folder_makes_each_level() {
        let fake = tree();
        let engine = engine(&fake);
        let make = |name: &str| {
            JobRequest::new(JobId::new(5), JobKind::MakeFolder, Vec::new())
                .to(p("/b"))
                .named(name)
        };
        let plan = engine.plan(&make("x/y/z")).unwrap();
        assert_eq!(
            plan.steps,
            vec![
                Step::MakeFolder { place: p("/b/x") },
                Step::MakeFolder { place: p("/b/x/y") },
                Step::MakeFolder {
                    place: p("/b/x/y/z")
                },
            ]
        );
        assert_eq!(plan.summary(), "Make 3 folders");
        assert!(matches!(
            engine.plan(&make("x/../y")),
            Err(OpsError::NameRefused(_))
        ));
        assert!(matches!(
            engine.plan(&make("x//y")),
            Err(OpsError::NameRefused(_))
        ));
        fake.add_file(&p("/b/file"), b"1");
        assert!(matches!(
            engine.plan(&make("file/inner")),
            Err(OpsError::NameRefused(line)) if line == "Cannot make file: a file has that name"
        ));
    }

    #[test]
    fn a_frozen_selection_plans_by_id_and_says_when_focus_is_elsewhere() {
        let fake = tree();
        let mut list = ListModel::new("a").with_rows(vec![
            ListRow::new("notes.txt", "notes.txt"),
            ListRow::new("photos", "photos"),
        ]);
        list.toggle_selected();
        list.move_focus(diroxide_core::Move::Down);
        let frozen = list.freeze();
        // The list refreshes after the question opened: the plan stays.
        list.set_rows(vec![ListRow::new("photos", "photos")]);
        let plan = engine(&fake)
            .plan_frozen(&request(JobKind::Trash, &[]), &p("/a"), &frozen)
            .unwrap();
        assert_eq!(
            plan.steps,
            vec![Step::Trash {
                place: p("/a/notes.txt")
            }]
        );
        assert_eq!(plan.listing()[0], "Not the focused row: photos");
    }

    #[test]
    fn links_point_at_the_full_source() {
        let fake = tree();
        let plan = engine(&fake)
            .plan_links(
                &request(JobKind::Link, &["/a/photos"]).to(p("/b")),
                LinkKind::Junction,
            )
            .unwrap();
        assert_eq!(
            plan.steps,
            vec![Step::Link {
                place: p("/b/photos"),
                request: LinkRequest {
                    kind: LinkKind::Junction,
                    target: p("/a/photos").to_string(),
                    target_is_folder: true
                }
            }]
        );
    }

    #[test]
    fn a_link_job_makes_the_link_and_a_place_without_links_refuses_in_words() {
        let fake = tree();
        let engine = engine(&fake);
        let plan = engine
            .plan(&request(JobKind::Link, &["/a/notes.txt"]).to(p("/b")))
            .unwrap();
        assert_eq!(plan.summary(), format!("Make 1 link to {}", p("/b")));
        let report = engine.run(&plan, &JobControl::new(), &Script::default());
        assert_eq!(report.done, vec![p("/b/notes.txt")]);
        let made = fake.entry(&p("/b/notes.txt")).unwrap();
        assert_eq!(made.kind, EntryKind::Link);
        assert_eq!(
            made.link.map(|link| link.target),
            Some(p("/a/notes.txt").to_string())
        );
        assert_eq!(report.undo.created, vec![p("/b/notes.txt")]);

        let stick = tree();
        stick.switches_mut().no_links = true;
        let refused = crate::testing::engine(&stick)
            .plan(&request(JobKind::Link, &["/a/notes.txt"]).to(p("/b")))
            .unwrap_err();
        assert_eq!(
            refused,
            OpsError::CannotLink("Cannot link here: this place holds no links".into())
        );
        assert!(within_cells(&refused.to_string(), "Cannot link here"));
        assert!(
            crate::testing::engine(&stick)
                .plan_links(
                    &request(JobKind::Link, &["/a/photos"]).to(p("/b")),
                    LinkKind::Junction,
                )
                .is_err()
        );
        assert!(!stick.exists(&p("/b/notes.txt")), "nothing was made");
    }

    #[test]
    fn counting_is_said_and_can_be_cancelled() {
        let fake = FakeProvider::new();
        for n in 0..600 {
            fake.add_file(&p(&format!("/big/f{n}")), b"1");
        }
        fake.add_folder(&p("/b"));
        let engine = engine(&fake);
        let sink = Script::default();
        engine
            .plan_with(
                &request(JobKind::Copy, &["/big"]).to(p("/b")),
                &JobControl::new(),
                &sink,
            )
            .unwrap();
        assert!(
            sink.states
                .lock()
                .unwrap()
                .iter()
                .all(|s| *s == JobState::Counting)
        );
        assert_eq!(
            sink.said.lock().unwrap()[0],
            "Copying: counting, 500 so far"
        );
        let control = JobControl::new();
        control.cancel.cancel();
        assert_eq!(
            engine.plan_with(
                &request(JobKind::Copy, &["/big"]).to(p("/b")),
                &control,
                &sink
            ),
            Err(OpsError::Vfs(VfsError::Cancelled))
        );
    }
}
