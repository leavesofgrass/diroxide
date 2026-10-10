//! Errors of the job engine.

use diroxide_core::Explain;
use diroxide_vfs::VfsError;
use thiserror::Error;

/// Why a job could not be planned. Failures while running are reported per
/// entry in the [`JobReport`](crate::JobReport) instead.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OpsError {
    /// A provider failed.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// The job has nothing to act on.
    #[error("nothing is selected")]
    NothingSelected,
    /// The job needs a destination and has none.
    #[error("there is no destination")]
    NoDestination,
    /// The job needs a new name and has none.
    #[error("there is no new name")]
    NoName,
    /// The job's kind is not built yet.
    #[error("{0} jobs are not built yet")]
    NotBuilt(String),
}

impl Explain for OpsError {
    fn next_step(&self) -> String {
        match self {
            OpsError::Vfs(inner) => inner.next_step(),
            OpsError::NothingSelected => "Select something first, or move to an entry.".to_owned(),
            OpsError::NoDestination => "Open the destination in the other panel.".to_owned(),
            OpsError::NoName => "Type a name.".to_owned(),
            OpsError::NotBuilt(_) => String::new(),
        }
    }
}
