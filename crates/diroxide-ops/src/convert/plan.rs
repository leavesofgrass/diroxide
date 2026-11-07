//! The conversion's plan (R28 5.3, step 3), computed before anything is
//! touched and shown in a list that starts on No: refusals first (an
//! output that would take its source's own name, two sources making one
//! output, a file over the recipe's `max-input`, a file only online, a
//! format the tool cannot read), then skips (an output that exists under
//! `skip`, or is up to date under `newer`), then the work ("song.flac to
//! song.mp3"), the tool, the input's size and a free-space warning.
//!
//! **A conversion never overwrites a file, attended or not.** The
//! conflict policies are keep both (the default, "report (2).pdf"), skip,
//! newer (skip an output that is up to date, else keep both) and ask
//! (only when a person is there: skip, keep both, a new name or cancel);
//! overwrite is refused before the plan is made.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use diroxide_core::format::{count, human_size};
use diroxide_core::job::ErrorPolicy;
use diroxide_core::names::spell_out;
use diroxide_core::{ConflictPolicy, Entry, EntryKind, JobId, JobOrigin, JobState, Message, Place};
use diroxide_vfs::names::{is_unfinished, unused_name};
use diroxide_vfs::{OpContext, Provider, ToolRequest, VfsError, facts};

use super::recipe::Recipe;
use super::staging::is_staging_name;
use super::tools;
use super::typing::{FileKind, HEAD_BYTES, Typing, magick_format};
use crate::engine::{Engine, JobControl};
use crate::progress::JobProgress;

/// Why a conversion could not be planned at all. Each reads after "Could
/// not convert: ".
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// The request asked to overwrite.
    #[error("conversions never replace a file; choose keep both, skip or newer")]
    Overwrite,
    /// Nothing was selected.
    #[error("nothing is selected")]
    NothingSelected,
    /// The recipe's tool is not installed.
    #[error("{name} is not installed")]
    ToolMissing {
        /// The tool's name as said.
        name: String,
    },
    /// The installed FFmpeg lacks the encoder the recipe needs.
    #[error("this {tool} has no {encoder} encoder")]
    NeedsEncoder {
        /// The tool's name as said.
        tool: String,
        /// The encoder.
        encoder: String,
    },
    /// The destination could not be used.
    #[error("the target folder: {0}")]
    Destination(VfsError),
    /// The plan was cancelled.
    #[error("it was cancelled")]
    Cancelled,
}

/// A conversion job as the app or the command line asks for it.
#[derive(Clone, Debug)]
pub struct ConvertRequest {
    /// The job's id.
    pub job: JobId,
    /// The files and folders to convert.
    pub sources: Vec<Place>,
    /// The folder the outputs go to: the other panel's, by the orthodox
    /// rule.
    pub destination: Place,
    /// The recipe.
    pub recipe: Recipe,
    /// What to do when an output's name is taken: keep both by default.
    pub conflicts: ConflictPolicy,
    /// What to do when a file fails: continue and list by default, so the
    /// report says every failure at the end.
    pub errors: ErrorPolicy,
    /// Who asked: a hot folder's job is unattended and never asks.
    pub origin: JobOrigin,
    /// Whether folders are converted with everything below them, the tree
    /// mirrored at the destination; else one level.
    pub recursive: bool,
    /// The state folder's spool, on this computer: where a source that is
    /// not local is copied first and an output for a place that is not
    /// local is made (R28 5.3, step 2). Without it such files are
    /// refused.
    pub spool: Option<PathBuf>,
}

impl ConvertRequest {
    /// A request with the defaults: keep both, continue and list, from the
    /// user, one level, no spool.
    pub fn new(job: JobId, sources: Vec<Place>, destination: Place, recipe: Recipe) -> Self {
        Self {
            job,
            sources,
            destination,
            recipe,
            conflicts: ConflictPolicy::KeepBoth,
            errors: ErrorPolicy::ContinueAndList,
            origin: JobOrigin::User,
            recursive: false,
            spool: None,
        }
    }

    /// The same request answering name clashes this way.
    pub fn conflicts(mut self, policy: ConflictPolicy) -> Self {
        self.conflicts = policy;
        self
    }

    /// The same request with this error policy.
    pub fn on_errors(mut self, policy: ErrorPolicy) -> Self {
        self.errors = policy;
        self
    }

    /// The same request from this origin.
    pub fn from(mut self, origin: JobOrigin) -> Self {
        self.origin = origin;
        self
    }

    /// The same request taking folders with everything below them.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    /// The same request with the state folder's spool.
    pub fn spool(mut self, folder: impl Into<PathBuf>) -> Self {
        self.spool = Some(folder.into());
        self
    }
}

/// One file the plan converts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertItem {
    /// The file.
    pub source: Place,
    /// What the planner saw there.
    pub entry: Entry,
    /// The format diroxide found, for `{typed_input}`: `PNG`.
    pub format: Option<String>,
    /// Where the output goes, with the name the plan chose.
    pub output: Place,
    /// Whether the name was taken when planned and the job asks about it
    /// at the commit (the ask policy, with a person there).
    pub ask: bool,
    /// Whether the source is copied into the spool first: it is not on
    /// this computer, or its name is one the tool would misread.
    pub through_spool: bool,
}

/// A conversion's plan.
#[derive(Clone, Debug)]
pub struct ConvertPlan {
    /// The job's id.
    pub job: JobId,
    /// The recipe.
    pub recipe: Recipe,
    /// The tool's name as said: "ImageMagick".
    pub tool_name: String,
    /// The destination folder.
    pub destination: Place,
    /// The conflict policy, overwrite never.
    pub conflicts: ConflictPolicy,
    /// The error policy.
    pub errors: ErrorPolicy,
    /// Who asked.
    pub origin: JobOrigin,
    /// The spool folder, when the request gave one.
    pub spool: Option<PathBuf>,
    /// Files refused, each with its reason in words.
    pub refused: Vec<(Place, String)>,
    /// Files skipped, each with its reason.
    pub skipped: Vec<(Place, String)>,
    /// Files inside the selected folders that the recipe does not take.
    pub left_out: u64,
    /// The work, in order.
    pub work: Vec<ConvertItem>,
    /// Folders to make at the destination for a recursive run, parents
    /// first.
    pub folders: Vec<Place>,
    /// The input's bytes.
    pub bytes: u64,
    /// Free space at the destination, when the place says.
    pub free: Option<u64>,
}

/// What a recipe's group calls its files in the progress line.
fn noun_of(recipe: &Recipe) -> (&'static str, &'static str) {
    match recipe.from_group.as_deref() {
        Some("images") => ("image", "images"),
        Some("music") => ("audio file", "audio files"),
        Some("video") => ("video", "videos"),
        Some("documents") => ("document", "documents"),
        _ if recipe.tool == "magick" => ("image", "images"),
        _ => ("file", "files"),
    }
}

/// A place's name with hidden characters spelled out.
pub(crate) fn said(place: &Place) -> String {
    place
        .name()
        .map_or_else(|| place.to_string(), |name| spell_out(name).into_owned())
}

impl ConvertPlan {
    /// Whether there is nothing to convert.
    pub fn is_empty(&self) -> bool {
        self.work.is_empty()
    }

    /// The noun for one and for many of the recipe's files: "image",
    /// "images".
    pub fn noun(&self) -> (&'static str, &'static str) {
        noun_of(&self.recipe)
    }

    /// The run's progress in words, at the progress interval:
    /// "Converting 12 of 40 images" while it runs, "Converting song.flac:
    /// 40 percent" for one long file FFmpeg reports on, "Converting paused
    /// at 12 of 40", "Converting waits for an answer", "Converting
    /// finished".
    pub fn progress_line(&self, progress: &JobProgress) -> String {
        let total = progress.items_total;
        let at = (progress.items_done + 1).min(total.max(1));
        let (one, many) = self.noun();
        let noun = if total == 1 { one } else { many };
        match progress.state {
            JobState::Paused => format!("Converting paused at {at} of {total}"),
            JobState::WaitingForAnswer => "Converting waits for an answer".to_owned(),
            JobState::Finished => "Converting finished".to_owned(),
            _ => match &progress.current {
                Some(current) if total == 1 && current.ends_with(" percent") => {
                    format!("Converting {current}")
                }
                _ => format!("Converting {at} of {total} {noun}"),
            },
        }
    }

    /// The question before the run, starting on No: "Convert 40 files to
    /// JPEG? y or n".
    pub fn question(&self) -> Message {
        let files = count(self.work.len() as u64, "file", "files");
        Message::question(&format!("Convert {files} to {}", self.recipe.makes_words()))
    }

    /// The plan in one line, the count first: "Convert plan: 40 files, 2
    /// skipped", with the refused files after the skipped.
    pub fn summary(&self) -> String {
        let mut parts = vec![count(self.work.len() as u64, "file", "files")];
        if !self.skipped.is_empty() {
            parts.push(format!("{} skipped", self.skipped.len()));
        }
        if !self.refused.is_empty() {
            parts.push(format!("{} refused", self.refused.len()));
        }
        format!("Convert plan: {}", parts.join(", "))
    }

    /// The plan's rows, each meaning first: the refusals ("Refused:
    /// a.heic, larger than 500 MB"), the skips ("Skipped song.flac:
    /// output exists"), what was left out, the work ("song.flac to
    /// song.mp3"), the tool, the input's size, and a warning when the
    /// destination has less room than the input takes.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (place, reason) in &self.refused {
            out.push(format!("Refused: {}, {reason}", said(place)));
        }
        for (place, reason) in &self.skipped {
            out.push(format!("Skipped {}: {reason}", said(place)));
        }
        if self.left_out > 0 {
            out.push(format!(
                "Left out: {} it does not take",
                count(self.left_out, "file", "files")
            ));
        }
        for item in &self.work {
            out.push(format!("{} to {}", said(&item.source), said(&item.output)));
        }
        out.push(format!("Tool: {}", self.tool_name));
        out.push(format!(
            "Input: {}, {}",
            count(self.work.len() as u64, "file", "files"),
            human_size(self.bytes)
        ));
        if let Some(free) = self.free
            && free < self.bytes
        {
            out.push(format!(
                "Low space: {} free, input {}",
                human_size(free),
                human_size(self.bytes)
            ));
        }
        out
    }
}

/// What the planner learned once per plan from the tool.
struct Probe {
    /// The formats ImageMagick reads, when the recipe's tool is it.
    magick_reads: Option<BTreeSet<String>>,
}

/// One planning pass.
struct Planner<'a> {
    engine: &'a Engine,
    request: &'a ConvertRequest,
    typing: &'a dyn Typing,
    control: &'a JobControl,
    ctx: OpContext,
    probe: Probe,
    plan: ConvertPlan,
    /// Lowercased names taken in each destination folder, by the folder's
    /// text: what was there, and what this plan claims.
    taken: HashMap<String, BTreeSet<String>>,
    /// The planned base output (folder and name, lowercased) and the
    /// source that claims it.
    claimed: HashMap<String, Place>,
    attended: bool,
}

/// A name's stem: everything before its last dot, or the whole name when
/// the dot is its first character or it has none.
pub(crate) fn stem_of(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// A name's extension, lowercase, without the dot.
pub(crate) fn extension_of(name: &str) -> Option<String> {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            Some(extension.to_ascii_lowercase())
        }
        _ => None,
    }
}

/// Whether the tool reads a name as something other than a file even
/// behind `FORMAT:` and a full path: ImageMagick takes a name ending in
/// `]` as a frame or size (spike V1 read `a.png[10x10+0+0]` as a crop of
/// `a.png`). Such a source goes through the spool under a plain name.
fn misread_by(tool: &str, name: &str) -> bool {
    tool == "magick" && name.ends_with(']')
}

impl<'a> Planner<'a> {
    fn provider(&self, place: &Place) -> Result<Arc<dyn Provider>, VfsError> {
        self.engine.registry().resolve(place)
    }

    /// Lists a destination folder's names once, lowercased.
    fn names_in(&mut self, folder: &Place) -> &mut BTreeSet<String> {
        let key = folder.to_string();
        if !self.taken.contains_key(&key) {
            let names = self
                .provider(folder)
                .and_then(|p| p.list(folder, &self.ctx))
                .map(|entries| entries.iter().map(|e| e.name.to_lowercase()).collect())
                .unwrap_or_default();
            self.taken.insert(key.clone(), names);
        }
        self.taken.entry(key).or_default()
    }

    /// The type of a file from its name and first bytes.
    fn kind_of(&self, place: &Place, entry: &Entry) -> FileKind {
        let mut head = Vec::with_capacity(HEAD_BYTES);
        if let Ok(provider) = self.provider(place)
            && let Ok(reader) = provider.read(place, &self.ctx)
        {
            use std::io::Read;
            let _ = reader.take(HEAD_BYTES as u64).read_to_end(&mut head);
        }
        self.typing.kind_of(&entry.name, &head)
    }

    /// Adds a file the request named, or one found in a selected folder
    /// (`chosen` false), with the folder its output goes to.
    fn file(&mut self, place: &Place, entry: Entry, folder: &Place, chosen: bool) {
        let recipe = &self.request.recipe;
        let extension = extension_of(&entry.name);
        let kind = self.kind_of(place, &entry);
        if !recipe.takes(extension.as_deref(), kind.type_id.as_deref(), &kind.groups) {
            if chosen {
                let what = kind.type_id.as_deref().unwrap_or("this kind of file");
                self.plan.refused.push((
                    place.clone(),
                    format!("{} does not take {what}", recipe.name),
                ));
            } else {
                self.plan.left_out += 1;
            }
            return;
        }
        if let (Some(limit), Some(size)) = (recipe.max_input, entry.size)
            && size > limit
        {
            self.plan.refused.push((
                place.clone(),
                format!("larger than {}, its limit", human_size(limit)),
            ));
            return;
        }
        if facts::is_online_only(&entry) {
            self.plan.refused.push((
                place.clone(),
                "only online; reading it would download it".to_owned(),
            ));
            return;
        }
        let format = if recipe.tool == "magick" {
            let format = magick_format(&kind, extension.as_deref());
            if let (Some(reads), Some(format)) = (&self.probe.magick_reads, &format)
                && !reads.contains(format)
            {
                self.plan.refused.push((
                    place.clone(),
                    format!("{} cannot open {format}", self.plan.tool_name),
                ));
                return;
            }
            format
        } else {
            None
        };
        let not_local = place.local_path().is_none();
        let through_spool = not_local || misread_by(&recipe.tool, &entry.name);
        if (through_spool || self.request.destination.local_path().is_none())
            && self.request.spool.is_none()
        {
            self.plan.refused.push((
                place.clone(),
                "not on this computer, and no spool folder".to_owned(),
            ));
            return;
        }
        let base = format!("{}.{}", stem_of(&entry.name), recipe.to);
        let Ok(base_place) = folder.join(&base) else {
            self.plan
                .refused
                .push((place.clone(), "its output name is not valid".to_owned()));
            return;
        };
        if base_place == *place
            || (base_place.parent() == place.parent()
                && place.name().is_some_and(|n| n.eq_ignore_ascii_case(&base)))
        {
            self.plan.refused.push((
                place.clone(),
                "its output would take its own name".to_owned(),
            ));
            return;
        }
        let claim = base_place.to_string().to_lowercase();
        if let Some(first) = self.claimed.get(&claim) {
            let line = format!("{} makes the same {}", said(first), spell_out(&base));
            self.plan.refused.push((place.clone(), line));
            return;
        }
        self.claimed.insert(claim, place.clone());
        let existing = self.names_in(folder).contains(&base.to_lowercase());
        let mut ask = false;
        let name = if !existing {
            base
        } else {
            match self.request.conflicts {
                ConflictPolicy::Skip => {
                    self.plan
                        .skipped
                        .push((place.clone(), "output exists".to_owned()));
                    return;
                }
                ConflictPolicy::Newer => {
                    let old = self
                        .provider(&base_place)
                        .and_then(|p| p.stat(&base_place, &self.ctx))
                        .ok();
                    let up_to_date = matches!(
                        (old.as_ref().and_then(|o| o.modified), entry.modified),
                        (Some(made), Some(source)) if made >= source
                    );
                    if up_to_date {
                        self.plan
                            .skipped
                            .push((place.clone(), "output is up to date".to_owned()));
                        return;
                    }
                    self.keep_both(folder, &base)
                }
                ConflictPolicy::Ask | ConflictPolicy::Rename if self.attended => {
                    ask = true;
                    base
                }
                _ => self.keep_both(folder, &base),
            }
        };
        self.names_in(folder).insert(name.to_lowercase());
        let Ok(output) = folder.join(&name) else {
            self.plan
                .refused
                .push((place.clone(), "its output name is not valid".to_owned()));
            return;
        };
        self.plan.bytes += entry.size.unwrap_or(0);
        self.plan.work.push(ConvertItem {
            source: place.clone(),
            entry,
            format,
            output,
            ask,
            through_spool,
        });
    }

    /// The keep-both name in a folder: "photo (2).jpg".
    fn keep_both(&mut self, folder: &Place, base: &str) -> String {
        let taken: Vec<String> = self.names_in(folder).iter().cloned().collect();
        let refs: Vec<&str> = taken.iter().map(String::as_str).collect();
        unused_name(base, &refs).unwrap_or_else(|| base.to_owned())
    }

    /// Adds a folder's files, and with `recursive` its folders' files,
    /// mirrored under `target`.
    fn folder(&mut self, place: &Place, target: &Place, depth: usize) -> Result<(), ConvertError> {
        self.control
            .cancel
            .check()
            .map_err(|_| ConvertError::Cancelled)?;
        let entries = match self.provider(place).and_then(|p| p.list(place, &self.ctx)) {
            Ok(entries) => entries,
            Err(VfsError::Cancelled) => return Err(ConvertError::Cancelled),
            Err(error) => {
                self.plan.refused.push((place.clone(), error.to_string()));
                return Ok(());
            }
        };
        let mut entries = entries;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            if entry.name.starts_with('.') || entry.hidden || is_unfinished(&entry.name) {
                continue;
            }
            if is_staging_name(&entry.name) {
                continue;
            }
            let Ok(child) = place.join(&entry.name) else {
                continue;
            };
            match entry.kind {
                EntryKind::Folder if self.request.recursive => {
                    if depth >= self.engine.options().max_depth {
                        self.plan
                            .refused
                            .push((child, "deeper than the walk goes".to_owned()));
                        continue;
                    }
                    let Ok(mirrored) = target.join(&entry.name) else {
                        continue;
                    };
                    self.plan.folders.push(mirrored.clone());
                    self.folder(&child, &mirrored, depth + 1)?;
                }
                EntryKind::Folder | EntryKind::Link => {}
                _ => self.file(&child, entry, target, false),
            }
        }
        Ok(())
    }

    /// Reads the tool's lists the plan needs, once.
    fn probe(&mut self) -> Result<(), ConvertError> {
        let recipe = &self.request.recipe;
        let tools = self.engine.tools();
        let cancel = &self.control.cancel;
        if let Some(encoder) = &recipe.needs_encoder {
            let request = ToolRequest::new(recipe.tool.clone(), ["-hide_banner", "-encoders"]);
            let listing = tools
                .run(&request, cancel)
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
                .unwrap_or_default();
            if !tools::ffmpeg_encoders(&listing).contains(encoder) {
                return Err(ConvertError::NeedsEncoder {
                    tool: self.plan.tool_name.clone(),
                    encoder: encoder.clone(),
                });
            }
        }
        if recipe.tool == "magick" {
            let request = ToolRequest::new("magick", ["-list", "format"]);
            self.probe.magick_reads = tools
                .run(&request, cancel)
                .ok()
                .filter(|out| out.succeeded())
                .map(|out| tools::magick_readable(&String::from_utf8_lossy(&out.stdout)))
                .filter(|reads| !reads.is_empty());
        }
        Ok(())
    }
}

impl Engine {
    /// Plans a conversion, touching nothing: every source read (a folder
    /// one level, or its tree with `recursive`), typed through `typing`,
    /// and sorted into refusals, skips and work, each output's name
    /// chosen by the conflict policy. Fails at once for overwrite, a
    /// missing tool or a missing encoder.
    pub fn plan_convert(
        &self,
        request: &ConvertRequest,
        typing: &dyn Typing,
        control: &JobControl,
    ) -> Result<ConvertPlan, ConvertError> {
        if request.conflicts == ConflictPolicy::Overwrite {
            return Err(ConvertError::Overwrite);
        }
        if request.sources.is_empty() {
            return Err(ConvertError::NothingSelected);
        }
        let recipe = &request.recipe;
        let built_in = recipe.tool == "carta";
        let tool_name = if built_in {
            "carta, built in".to_owned()
        } else {
            self.tools().name_of(&recipe.tool)
        };
        if !built_in && !self.tools().has(&recipe.tool) {
            return Err(ConvertError::ToolMissing { name: tool_name });
        }
        let ctx = self.context(&control.cancel);
        let destination = self
            .registry()
            .resolve(&request.destination)
            .and_then(|p| p.stat(&request.destination, &ctx))
            .map_err(ConvertError::Destination)?;
        if destination.kind != EntryKind::Folder {
            return Err(ConvertError::Destination(VfsError::NotAFolder));
        }
        let attended = matches!(request.origin, JobOrigin::User);
        let mut planner = Planner {
            engine: self,
            request,
            typing,
            control,
            ctx,
            probe: Probe { magick_reads: None },
            plan: ConvertPlan {
                job: request.job,
                recipe: recipe.clone(),
                tool_name,
                destination: request.destination.clone(),
                conflicts: request.conflicts,
                errors: request.errors,
                origin: request.origin.clone(),
                spool: request.spool.clone(),
                refused: Vec::new(),
                skipped: Vec::new(),
                left_out: 0,
                work: Vec::new(),
                folders: Vec::new(),
                bytes: 0,
                free: None,
            },
            taken: HashMap::new(),
            claimed: HashMap::new(),
            attended,
        };
        planner.probe()?;
        let target = request.destination.clone();
        for source in &request.sources {
            control
                .cancel
                .check()
                .map_err(|_| ConvertError::Cancelled)?;
            let entry = match planner
                .provider(source)
                .and_then(|p| p.stat(source, &planner.ctx))
            {
                Ok(entry) => entry,
                Err(VfsError::Cancelled) => return Err(ConvertError::Cancelled),
                Err(error) => {
                    planner
                        .plan
                        .refused
                        .push((source.clone(), error.to_string()));
                    continue;
                }
            };
            match entry.kind {
                EntryKind::Folder => {
                    let mirrored = if request.recursive {
                        match target.join(&entry.name) {
                            Ok(mirrored) => {
                                planner.plan.folders.push(mirrored.clone());
                                mirrored
                            }
                            Err(_) => target.clone(),
                        }
                    } else {
                        target.clone()
                    };
                    planner.folder(source, &mirrored, 1)?;
                }
                EntryKind::Link => planner
                    .plan
                    .refused
                    .push((source.clone(), "a link; links are not followed".to_owned())),
                _ => planner.file(source, entry, &target, true),
            }
        }
        planner.plan.free = self
            .registry()
            .resolve(&request.destination)
            .and_then(|p| p.space(&request.destination, &planner.ctx))
            .ok()
            .and_then(|space| space.free_to_user);
        Ok(planner.plan)
    }
}
