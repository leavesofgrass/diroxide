//! Why a compare did not finish. A compare that does not finish leaves
//! "Not compared", never a result that looks whole.

use diroxide_core::Explain;
use diroxide_vfs::VfsError;
use thiserror::Error;

/// Why a compare did not finish.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DiffError {
    /// The compare was cancelled.
    #[error("it was cancelled")]
    Cancelled,
    /// A file could not be read or looked at.
    #[error(transparent)]
    Read(VfsError),
    /// A place to compare is a folder, a broken link or something else
    /// that is not a file; the place is kept for the caller to name.
    #[error("it is not a file")]
    NotAFile(String),
}

impl From<VfsError> for DiffError {
    /// A provider's error; its cancel is the compare's.
    fn from(error: VfsError) -> Self {
        match error {
            VfsError::Cancelled => DiffError::Cancelled,
            other => DiffError::Read(other),
        }
    }
}

impl Explain for DiffError {
    fn next_step(&self) -> String {
        match self {
            DiffError::Cancelled => String::new(),
            DiffError::Read(inner) => inner.next_step(),
            DiffError::NotAFile(_) => "Choose two files to compare.".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_providers_cancel_is_a_cancel() {
        assert_eq!(DiffError::from(VfsError::Cancelled), DiffError::Cancelled);
        let denied = DiffError::from(VfsError::PermissionDenied);
        assert_eq!(denied.to_string(), "permission was denied");
    }
}
