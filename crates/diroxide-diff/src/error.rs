//! Why a compare did not finish. A compare that does not finish leaves
//! "Not compared", never a result that looks whole.

use diroxide_core::Explain;
use thiserror::Error;

/// Why a compare did not finish.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DiffError {
    /// The compare was cancelled.
    #[error("it was cancelled")]
    Cancelled,
}

impl Explain for DiffError {
    fn next_step(&self) -> String {
        match self {
            DiffError::Cancelled => String::new(),
        }
    }
}
