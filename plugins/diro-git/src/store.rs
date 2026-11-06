//! The owner's trust answers (answer 123), kept by the repository's path
//! and the digest of its configuration, in `trust.json` in the plugin's
//! state folder: a changed configuration has another digest, so it asks
//! again. Nothing else is kept, and nothing about the owner.
//!
//! The file is written whole to a temporary name beside it, then renamed
//! over it, so a crash never leaves half a file; a file that cannot be
//! read counts as no answers (the question comes again, starting on No).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

/// The file's name in the state folder.
pub const FILE: &str = "trust.json";

/// One answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// The repository's top folder.
    pub repository: String,
    /// The SHA-256 of its own configuration when the owner answered.
    pub digest: String,
    /// Yes or no.
    pub trusted: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    answers: Vec<Answer>,
}

/// The answers, read once and written on each change.
#[derive(Debug, Default)]
pub struct TrustStore {
    folder: Mutex<Option<PathBuf>>,
    answers: Mutex<Option<Vec<Answer>>>,
}

impl TrustStore {
    /// Where the answers live: the plugin's state folder, from
    /// `initialize`. Until it is set, answers last for the session only.
    pub fn set_folder(&self, folder: PathBuf) {
        *self.folder.lock().unwrap_or_else(PoisonError::into_inner) = Some(folder);
        *self.answers.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn path(&self) -> Option<PathBuf> {
        self.folder
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|f| f.join(FILE))
    }

    fn with_answers<T>(&self, f: impl FnOnce(&mut Vec<Answer>) -> T) -> T {
        let mut answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        if answers.is_none() {
            let read = self
                .path()
                .and_then(|p| fs::read_to_string(p).ok())
                .and_then(|t| serde_json::from_str::<File>(&t).ok())
                .map(|f| f.answers)
                .unwrap_or_default();
            *answers = Some(read);
        }
        f(answers.get_or_insert_with(Vec::new))
    }

    /// The owner's answer for this repository and configuration, if any.
    pub fn answer(&self, repository: &Path, digest: &str) -> Option<bool> {
        let repository = repository.display().to_string();
        self.with_answers(|answers| {
            answers
                .iter()
                .find(|a| a.repository == repository && a.digest == digest)
                .map(|a| a.trusted)
        })
    }

    /// Keeps an answer, replacing the repository's earlier ones, and
    /// writes the file; an error says why it could not be written (the
    /// answer still holds for the session).
    pub fn record(&self, repository: &Path, digest: &str, trusted: bool) -> std::io::Result<()> {
        let repository = repository.display().to_string();
        let all = self.with_answers(|answers| {
            answers.retain(|a| a.repository != repository);
            answers.push(Answer {
                repository,
                digest: digest.to_owned(),
                trusted,
            });
            answers.clone()
        });
        let Some(path) = self.path() else {
            return Ok(());
        };
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let text =
            serde_json::to_string_pretty(&File { answers: all }).map_err(std::io::Error::other)?;
        let temporary = path.with_file_name("trust.json.part");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        let base = std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("git-unit-tmp");
        fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("store-")
            .tempdir_in(base)
            .unwrap()
    }

    #[test]
    fn an_answer_is_kept_by_path_and_digest_across_sessions() {
        let tmp = scratch();
        let store = TrustStore::default();
        store.set_folder(tmp.path().join("state"));
        let repo = Path::new("D:/work/repo");
        assert_eq!(store.answer(repo, "aa"), None);
        store.record(repo, "aa", false).unwrap();
        assert_eq!(store.answer(repo, "aa"), Some(false));
        store.record(repo, "aa", true).unwrap();
        assert_eq!(store.answer(repo, "aa"), Some(true));
        // A changed configuration asks again.
        assert_eq!(store.answer(repo, "bb"), None);
        let again = TrustStore::default();
        again.set_folder(tmp.path().join("state"));
        assert_eq!(again.answer(repo, "aa"), Some(true));
        assert_eq!(again.answer(Path::new("D:/other"), "aa"), None);
    }

    #[test]
    fn a_broken_file_counts_as_no_answers_and_no_folder_keeps_the_session() {
        let tmp = scratch();
        fs::create_dir_all(tmp.path().join("state")).unwrap();
        fs::write(tmp.path().join("state").join(FILE), "not json").unwrap();
        let store = TrustStore::default();
        store.set_folder(tmp.path().join("state"));
        assert_eq!(store.answer(Path::new("r"), "aa"), None);
        let session = TrustStore::default();
        session.record(Path::new("r"), "aa", true).unwrap();
        assert_eq!(session.answer(Path::new("r"), "aa"), Some(true));
    }
}
