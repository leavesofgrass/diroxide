//! The walkers: two ways through a scope, one stream of candidates.
//!
//! - **Local places** go through `ignore`'s parallel walker with its
//!   standard filters off ([`local`]): hidden entries, ignore files,
//!   snapshots and marked places are diroxide's decisions, made here and
//!   counted, never the crate's silent defaults. Links are never followed
//!   by it.
//! - **Every other place, and any search that follows links or enters
//!   archives,** goes through the provider walk ([`provider`]): folders
//!   listed through `Provider::list` by a few threads sharing one queue,
//!   with the boundary rule (a link is never followed out of the place
//!   the search started in), loops caught, other drives, snapshots and
//!   deep folders skipped and counted.
//!
//! Both hand each entry to a [`Visitor`] as a [`Candidate`]: the entry,
//! its depth (direct children are 1) and where it is. A folder is handed
//! over before its contents. On the same local tree, with links not
//! followed, both give the same set of entries and the same counts.

pub(crate) mod ignores;
pub(crate) mod local;
pub(crate) mod provider;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{Provider, Registry, VfsError, VolumeKind};

use crate::sink::SearchControl;
use crate::summary::Skips;

/// The deepest a walk goes when the scope sets no limit: deeper folders
/// are a reported skip, never a stack overflow (the walk helper's limit).
pub(crate) const DEEPEST: u32 = diroxide_vfs::walk::DEFAULT_MAX_DEPTH as u32;

/// Where a candidate is.
#[derive(Clone, Debug)]
pub(crate) enum Location {
    /// A local path from the parallel walker; its place is made when it
    /// is needed (a hit, or a path test).
    Local(PathBuf),
    /// A place from the provider walk, with the provider that serves it.
    Place(Place, Arc<dyn Provider>),
}

/// One entry the walk met, for the evaluator.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    /// What it is. Its size and times are missing when the walk was told
    /// the filter does not need them.
    pub(crate) entry: Entry,
    /// Folders below the start: direct children are 1, a start that is a
    /// file is 0.
    pub(crate) depth: u32,
    /// Where it is.
    pub(crate) location: Location,
}

impl Candidate {
    /// The whole path as text, for a `path` test and for rows.
    pub(crate) fn path_text(&self) -> Cow<'_, str> {
        match &self.location {
            Location::Local(path) => path.to_string_lossy(),
            Location::Place(place, _) => Cow::Owned(place.to_string()),
        }
    }

    /// The candidate's place, made from the start's place and the names
    /// below it, so a name the place cannot hold is caught: `None` then.
    pub(crate) fn place(&self, root: &Root) -> Option<Place> {
        match &self.location {
            Location::Place(place, _) => Some(place.clone()),
            Location::Local(path) => {
                let base = root.path.as_deref()?;
                let rest = path.strip_prefix(base).ok()?;
                let mut place = root.place.clone();
                for part in rest.components() {
                    place = place.join(part.as_os_str().to_str()?).ok()?;
                }
                Some(place)
            }
        }
    }

    /// The local path, when the candidate has one.
    pub(crate) fn local_path(&self) -> Option<&Path> {
        match &self.location {
            Location::Local(path) => Some(path),
            Location::Place(..) => None,
        }
    }
}

/// A start of the walk: its place and, for a local place, its path.
#[derive(Clone, Debug)]
pub(crate) struct Root {
    /// The place.
    pub(crate) place: Place,
    /// Its path on this computer, for a local place.
    pub(crate) path: Option<PathBuf>,
}

impl Root {
    /// A start at a place.
    pub(crate) fn new(place: Place) -> Self {
        let path = place.local_path().map(PathBuf::from);
        Self { place, path }
    }
}

/// Takes the walk's candidates, one visitor per thread.
pub(crate) trait Visitor: Send {
    /// One candidate. `Break` stops the whole walk.
    fn visit(&mut self, candidate: Candidate) -> ControlFlow<()>;
}

/// How a walk goes: the scope, read for one start.
#[derive(Clone, Copy)]
pub(crate) struct WalkPlan<'a> {
    /// Where it starts.
    pub(crate) root: &'a Root,
    /// Whether hidden entries are walked.
    pub(crate) hidden: bool,
    /// Whether ignore files are read.
    pub(crate) ignore_files: bool,
    /// Whether links to folders inside the start are followed.
    pub(crate) follow_links: bool,
    /// Whether the walk stays on the start's drive.
    pub(crate) one_file_system: bool,
    /// Whether archives are entered.
    pub(crate) archives: bool,
    /// The deepest level handed over; direct children are 1.
    pub(crate) max_depth: Option<u32>,
    /// Threads; 0 lets the walker choose.
    pub(crate) threads: usize,
    /// Folder names never entered.
    pub(crate) skip_names: &'a [String],
    /// Places never entered.
    pub(crate) skip_places: &'a [Place],
    /// Whether candidates need their sizes and times.
    pub(crate) metadata: bool,
    /// Cancel and pause.
    pub(crate) control: &'a SearchControl,
}

impl WalkPlan<'_> {
    /// The deepest level handed over, with the walk's own limit when the
    /// scope sets none.
    pub(crate) fn depth_limit(&self) -> u32 {
        self.max_depth.unwrap_or(DEEPEST).min(DEEPEST)
    }

    /// Whether a folder at this depth is entered: below the limit.
    pub(crate) fn enters(&self, depth: u32) -> bool {
        depth < self.depth_limit()
    }

    /// Whether a folder at this depth was left unentered because of the
    /// walk's own limit, not the owner's.
    pub(crate) fn too_deep(&self, depth: u32) -> bool {
        depth >= DEEPEST && self.max_depth.is_none_or(|m| m >= DEEPEST)
    }
}

/// What a walk saw, besides its candidates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WalkStats {
    /// Folders read, the start included.
    pub(crate) folders: u64,
    /// Candidates handed over.
    pub(crate) entries: u64,
    /// Folders that could not be read.
    pub(crate) unreadable: u64,
    /// What was not walked, and why.
    pub(crate) skips: Skips,
    /// Whether it was cancelled or stopped by a visitor.
    pub(crate) stopped: bool,
}

impl WalkStats {
    /// Adds another thread's counts.
    pub(crate) fn add(&mut self, other: &WalkStats) {
        self.folders += other.folders;
        self.entries += other.entries;
        self.unreadable += other.unreadable;
        self.skips.add(&other.skips);
        self.stopped |= other.stopped;
    }
}

/// Which walker a start takes: the local walker for a local place when
/// links are not followed and archives not entered, the provider walk
/// otherwise.
pub(crate) fn uses_local_walker(plan: &WalkPlan<'_>) -> bool {
    plan.root.path.is_some() && !plan.follow_links && !plan.archives
}

/// The threads a walk uses when the scope does not say: the local walker
/// chooses its own (0); the provider walk takes two on a network drive
/// (a share must not be flooded), one inside an archive, four elsewhere.
pub(crate) fn default_threads(local: bool, volume: Option<VolumeKind>, archive: bool) -> usize {
    if local {
        0
    } else if archive {
        1
    } else if volume == Some(VolumeKind::Network) {
        2
    } else {
        4
    }
}

/// Walks one start with the walker that fits it.
pub(crate) fn run<V, F>(
    plan: &WalkPlan<'_>,
    registry: &Registry,
    make: &F,
) -> Result<WalkStats, VfsError>
where
    F: Fn() -> V + Sync,
    V: Visitor,
{
    match (&plan.root.path, uses_local_walker(plan)) {
        (Some(path), true) => Ok(local::walk(plan, path, make)),
        _ => {
            let provider = registry.resolve(&plan.root.place)?;
            Ok(provider::walk(plan, provider, registry, make))
        }
    }
}
