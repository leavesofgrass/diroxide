//! The errors of the archive provider's own calls: a question a limit
//! asks first, or a provider error. The `Provider` methods give the plain
//! [`VfsError`]; the runners and the app's archive dialogs call the
//! provider's own methods, which keep the question so it can be asked.

use diroxide_core::{Explain, Message};
use diroxide_vfs::VfsError;
use thiserror::Error;

use crate::guard::{LimitHit, Question};

/// Why an archive call stopped.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ArchiveError {
    /// A limit asks first: ask, then call again with the answer in the
    /// allowances.
    #[error("{}", .0.message().text)]
    Ask(Question),
    /// It failed or was refused.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// Refused before anything started, said whole: "Could not pack
    /// docs.xz: xz holds one file. Use tar.xz."
    #[error("{0}")]
    Refused(String),
}

impl ArchiveError {
    /// The question, if this is one.
    pub fn question(&self) -> Option<&Question> {
        match self {
            ArchiveError::Ask(question) => Some(question),
            ArchiveError::Vfs(_) | ArchiveError::Refused(_) => None,
        }
    }

    /// The error a caller that cannot ask gives.
    pub fn into_vfs(self) -> VfsError {
        match self {
            ArchiveError::Ask(question) => question.as_error(),
            ArchiveError::Vfs(error) => error,
            ArchiveError::Refused(sentence) => VfsError::Unsafe { why: sentence },
        }
    }

    /// The sentence to say: the question ending "y or n", or "Could not
    /// VERB OBJECT: CAUSE. NEXT STEP."
    pub fn message(&self, verb: &str, object: &str) -> Message {
        match self {
            ArchiveError::Ask(question) => question.message(),
            ArchiveError::Vfs(error) => Message::from_error(verb, object, error),
            ArchiveError::Refused(sentence) => Message::error(sentence.clone()),
        }
    }
}

impl From<LimitHit> for ArchiveError {
    fn from(hit: LimitHit) -> Self {
        match hit {
            LimitHit::Ask(question) => ArchiveError::Ask(question),
            LimitHit::Refuse(error) => ArchiveError::Vfs(error),
        }
    }
}

impl From<ArchiveError> for VfsError {
    fn from(error: ArchiveError) -> Self {
        error.into_vfs()
    }
}

impl Explain for ArchiveError {
    fn next_step(&self) -> String {
        match self {
            ArchiveError::Ask(_) | ArchiveError::Refused(_) => String::new(),
            ArchiveError::Vfs(error) => error.next_step(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::QuestionKind;

    #[test]
    fn a_question_and_a_refusal_say_themselves() {
        let ask = ArchiveError::Ask(Question {
            kind: QuestionKind::MemberSize,
            text: "big.bin is 4.3 GB. Read it".to_owned(),
        });
        assert_eq!(
            ask.message("read", "big.bin").text,
            "big.bin is 4.3 GB. Read it? y or n"
        );
        let refused = ArchiveError::from(VfsError::PasswordRequired);
        assert_eq!(
            refused.message("open", "a.7z").text,
            "Could not open a.7z: a password is needed. Enter the password and try again."
        );
        assert!(matches!(ask.into_vfs(), VfsError::LimitReached { .. }));
    }
}
