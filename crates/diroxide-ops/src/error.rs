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
    /// The trash call returned, but the post-check could not confirm the
    /// item is in the trash; the line says what was found.
    #[error("{0}")]
    TrashNotConfirmed(String),
    /// A safety rule refused the job before anything was listed: a drive
    /// root, a mount point, a protected folder. The sentence says which:
    /// "Refused: this is a drive root, D:\".
    #[error("{0}")]
    Refused(String),
    /// A folder would be copied or moved into itself or a folder inside
    /// it; the name is the folder's.
    #[error("{0} cannot go inside itself")]
    IntoItself(String),
    /// A new name the place refuses, with the reason in words: "Name needs
    /// care: ends in a dot or space".
    #[error("{0}")]
    NameRefused(String),
    /// Undo was asked for with nothing to undo. Undo is planned from the
    /// undo log ([`Engine::plan_undo`](crate::Engine::plan_undo)), never
    /// from a request.
    #[error("there is nothing to undo")]
    NothingToUndo,
    /// Links were asked for where the place cannot make them (a FAT or
    /// exFAT drive, an archive, a read-only place); the text is the
    /// capability's words: "Cannot link here: this place holds no links".
    #[error("{0}")]
    CannotLink(String),
}

impl Explain for OpsError {
    fn next_step(&self) -> String {
        match self {
            OpsError::Vfs(inner) => inner.next_step(),
            OpsError::NothingSelected => "Select something first, or move to an entry.".to_owned(),
            OpsError::NoDestination => "Open the destination in the other panel.".to_owned(),
            OpsError::NoName => "Type a name.".to_owned(),
            OpsError::NotBuilt(_) | OpsError::NothingToUndo => String::new(),
            OpsError::TrashNotConfirmed(_) => {
                "Look for it in the trash, and check the drive.".to_owned()
            }
            OpsError::Refused(_) => "Choose the entries inside it instead.".to_owned(),
            OpsError::IntoItself(_) => "Choose a destination outside it.".to_owned(),
            OpsError::NameRefused(_) => "Type another name.".to_owned(),
            OpsError::CannotLink(_) => {
                "Make the link in a folder on a drive that holds links.".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn refusals_read_as_sentences() {
        let refused = OpsError::Refused("Refused: this is a drive root, D:\\".into());
        assert!(within_cells(
            &refused.to_string(),
            "Refused: this is a drive root"
        ));
        assert_eq!(refused.next_step(), "Choose the entries inside it instead.");
        assert_eq!(
            OpsError::IntoItself("photos".into()).to_string(),
            "photos cannot go inside itself"
        );
        assert_eq!(
            OpsError::NothingToUndo.to_string(),
            "there is nothing to undo"
        );
    }
}
