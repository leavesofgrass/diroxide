//! Compare folders and synchronize, their types in the app (alpha 5,
//! lane SA): the compare, the plan view and the run, over the engine
//! half in `diroxide_ops::sync`.
//!
//! [`SyncAsk`] travels as [`Request::Sync`](crate::Request::Sync) to the
//! runtime, which answers in `runtime/syncing.rs`, and [`SyncNote`] comes
//! back as [`AppEvent::Syncing`](crate::AppEvent::Syncing), handled in
//! `app/syncing.rs`. The compare runs as a `JobKind::Compare` job, so
//! Escape on the panel and the Jobs list cancel it; the run is one
//! `JobKind::Synchronize` job with one undo record, its report the
//! engine's own (`AppEvent::JobFinished`).

use std::sync::Arc;
use std::time::Duration;

use diroxide_core::{JobId, Message, Place, RequestId};
use diroxide_diff::{FileComparison, FileOptions};
use diroxide_ops::sync::{CompareBy, CompareCriteria, Comparison, SyncPlan, TOLERANCE};

/// What a folder compare looks at, as the owner chose it: the engine's
/// [`CompareCriteria`] without its filter, so it can be held by a dialog
/// and compared with the settings again before a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompareOptions {
    /// What decides "the same".
    pub by: CompareBy,
    /// Whether folders on both sides are compared inside too.
    pub subfolders: bool,
    /// Whether hidden entries take part.
    pub hidden: bool,
    /// Whether two times one hour apart count as the same.
    pub ignore_one_hour: bool,
    /// Two times this many seconds apart, or closer, count as the same.
    pub tolerance_seconds: u64,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            by: CompareBy::SizeAndTime,
            subfolders: true,
            hidden: true,
            ignore_one_hour: false,
            tolerance_seconds: TOLERANCE.as_secs(),
        }
    }
}

impl CompareOptions {
    /// The options from the settings (`[compare]`): the tolerance and the
    /// hour shift; the rest at the engine's defaults.
    pub fn from_settings(settings: &diroxide_config::Settings) -> Self {
        Self {
            tolerance_seconds: settings.compare.folder_tolerance_seconds,
            ignore_one_hour: settings.compare.folder_hour_shift,
            ..Self::default()
        }
    }

    /// The engine's criteria for these options.
    pub fn criteria(&self) -> CompareCriteria {
        CompareCriteria {
            by: self.by,
            subfolders: self.subfolders,
            hidden: self.hidden,
            ignore_one_hour: self.ignore_one_hour,
            tolerance: Duration::from_secs(self.tolerance_seconds),
            ..CompareCriteria::default()
        }
    }
}

/// A compare's result, held by the dialogs that show it. Two are equal
/// when they are the same result, not when their rows match.
#[derive(Clone, Debug)]
pub struct HeldComparison(pub Arc<Comparison>);

impl PartialEq for HeldComparison {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HeldComparison {}

/// What the app asks the runtime for a compare or a synchronize.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncAsk {
    /// Compare two folders as a job: the place here (this panel) first.
    Compare {
        /// The job; Cancel trips it.
        job: JobId,
        /// This panel's folder.
        here: Place,
        /// The other panel's folder.
        there: Place,
        /// What the compare looks at.
        options: CompareOptions,
        /// The shortest time between two progress lines, in
        /// milliseconds: the app's progress interval.
        every_ms: u64,
        /// Whether the owner asked for Synchronize, so the modes follow
        /// the result at once.
        then_plan: bool,
    },
    /// Compare two files of a compare's Different row.
    Files {
        /// The request the answer names.
        request: RequestId,
        /// The file there (the other panel's), before.
        before: Place,
        /// The file here (this panel's), after.
        after: Place,
        /// The file compare's options.
        options: FileOptions,
    },
    /// Ask the `before_copy` and `before_delete` hooks about a plan,
    /// before the question.
    Hooks {
        /// The job the run will be.
        job: JobId,
        /// The plan.
        plan: Box<SyncPlan>,
    },
    /// Run a plan as one job, its places read again first.
    Run {
        /// The job.
        job: JobId,
        /// The plan, as the owner left it.
        plan: Box<SyncPlan>,
    },
}

/// What the runtime answers a [`SyncAsk`].
#[derive(Debug)]
pub enum SyncNote {
    /// Where a compare is, at the interval: "Comparing: 1,200 entries, in
    /// photos".
    Progress {
        /// The compare's job.
        job: JobId,
        /// The line to say.
        line: String,
    },
    /// A compare ended: its result, or why it could not run.
    Compared {
        /// The compare's job.
        job: JobId,
        /// This panel's folder.
        here: Place,
        /// The other panel's folder.
        there: Place,
        /// What it looked at.
        options: CompareOptions,
        /// Whether the modes follow at once.
        then_plan: bool,
        /// The result, or the line that says why not.
        result: Result<Box<Comparison>, String>,
    },
    /// Two files compared, for a Different row.
    Files {
        /// The request.
        request: RequestId,
        /// The file there.
        before: Place,
        /// The file here.
        after: Place,
        /// The options it was compared under.
        options: FileOptions,
        /// What was found, or what went wrong.
        result: Result<Box<FileComparison>, Message>,
    },
    /// The hooks were asked: the lines they had to say, or the line of the
    /// one that stopped the plan, its plugin's name last.
    Hooked {
        /// The job the run will be.
        job: JobId,
        /// The plan.
        plan: Box<SyncPlan>,
        /// What the hooks said, or why one stopped it.
        result: Result<Vec<String>, String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_follow_the_settings_and_give_the_engine_criteria() {
        let mut settings = diroxide_config::Settings::default();
        settings.compare.folder_tolerance_seconds = 3;
        settings.compare.folder_hour_shift = true;
        let options = CompareOptions::from_settings(&settings);
        assert_eq!(options.tolerance_seconds, 3);
        assert!(options.ignore_one_hour);
        let criteria = options.criteria();
        assert_eq!(criteria.tolerance, Duration::from_secs(3));
        assert!(criteria.ignore_one_hour && criteria.subfolders && criteria.hidden);
        assert_eq!(CompareOptions::default().tolerance_seconds, 2);
    }
}
