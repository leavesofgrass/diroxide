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
    /// A split was asked for with no part size, or a size of zero.
    #[error("there is no part size")]
    NoPartSize,
    /// A split would make more than 99,999 parts; the sentence names the
    /// size to use instead: "Too many parts: 600,000. Use parts of 6.1 MB
    /// or more."
    #[error("{0}")]
    TooManyParts(String),
    /// A set of parts cannot be joined, said before anything is written:
    /// "Part 004 is missing. Nothing was joined."
    #[error("{0}")]
    CannotJoin(String),
    /// A permissions or times job was asked for with nothing to change.
    #[error("there is no change to make")]
    NoChange,
    /// A link was selected for a permissions change: links have none of
    /// their own. The text is the link's name.
    #[error("links have no permissions of their own: {0}")]
    LinkSelected(String),
    /// A plugin's `before_*` hook stopped the job before its question:
    /// the line says why and ends with the plugin's name ("Delete
    /// stopped: keep.txt is protected (Folder guard)"), or says the hook
    /// did not answer in time and its manifest says to stop. Said as it
    /// is, never inside "Could not ...".
    #[error("{0}")]
    HookStopped(String),
    /// The trash cannot be listed here (macOS), or the engine has no trash
    /// to list: the trash browser says
    /// [`TRASH_NOT_LISTED`](diroxide_vfs::TRASH_NOT_LISTED) whole.
    #[error("the trash cannot be listed on this system")]
    NoTrashList,
    /// Empty the trash was asked for, and the trash holds nothing.
    #[error("the trash is empty")]
    TrashEmpty,
}

impl OpsError {
    /// The error as a sentence of its own, where it reads better that way
    /// than inside "Could not VERB OBJECT: CAUSE.": "The trash cannot be
    /// listed on this system."; "The trash is empty."; a provider's own
    /// ([`VfsError::sentence`]). `None` for the rest.
    pub fn sentence(&self) -> Option<String> {
        match self {
            OpsError::NoTrashList => Some(diroxide_vfs::TRASH_NOT_LISTED.to_owned()),
            OpsError::TrashEmpty => Some("The trash is empty.".to_owned()),
            OpsError::Vfs(inner) => inner.sentence(),
            _ => None,
        }
    }
}

impl Explain for OpsError {
    fn next_step(&self) -> String {
        match self {
            OpsError::Vfs(inner) => inner.next_step(),
            OpsError::NothingSelected => "Select something first, or move to an entry.".to_owned(),
            OpsError::NoDestination => "Open the destination in the other panel.".to_owned(),
            OpsError::NoName => "Type a name.".to_owned(),
            OpsError::NotBuilt(_)
            | OpsError::NothingToUndo
            | OpsError::HookStopped(_)
            | OpsError::NoTrashList
            | OpsError::TrashEmpty => String::new(),
            OpsError::TrashNotConfirmed(_) => {
                "Look for it in the trash, and check the drive.".to_owned()
            }
            OpsError::Refused(_) => "Choose the entries inside it instead.".to_owned(),
            OpsError::IntoItself(_) => "Choose a destination outside it.".to_owned(),
            OpsError::NameRefused(_) => "Type another name.".to_owned(),
            OpsError::CannotLink(_) => {
                "Make the link in a folder on a drive that holds links.".to_owned()
            }
            OpsError::NoPartSize => "Choose a part size.".to_owned(),
            OpsError::TooManyParts(_) => "Choose a larger part size.".to_owned(),
            OpsError::CannotJoin(_) => "Check the parts, then join again.".to_owned(),
            OpsError::NoChange => "Choose what to change.".to_owned(),
            OpsError::LinkSelected(_) => "Change the target instead.".to_owned(),
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
        assert_eq!(
            OpsError::NoTrashList.sentence().as_deref(),
            Some("The trash cannot be listed on this system.")
        );
        assert_eq!(
            OpsError::TrashEmpty.sentence().as_deref(),
            Some("The trash is empty.")
        );
        assert_eq!(OpsError::NothingSelected.sentence(), None);
    }
}
