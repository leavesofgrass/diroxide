//! Which repository a folder is in, found without running git: the folder
//! and those above it, at most 64 (the host's own marker walk), are looked
//! at for `.git`, a folder or, in a linked worktree or a submodule, a file
//! naming the real one. Reading these names runs nothing, so it may come
//! before the trust scan.
//!
//! The repository's bookkeeping files are only looked at, never read for
//! their contents, except `HEAD`'s neighbours that say an operation is in
//! progress and the `.git` file's one line.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// How many folders, the first one included, the walk up looks at.
pub const MAX_DEPTH: usize = 64;

/// A working tree and its repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    /// The working tree's top folder.
    pub root: PathBuf,
    /// The repository folder for this working tree: `root/.git`, or the
    /// folder a `.git` file names.
    pub git_dir: PathBuf,
    /// The folder shared by every worktree, where `config` lives.
    pub common_dir: PathBuf,
}

/// When a file last changed and its size: what the caches compare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Stamp {
    /// Its time, when it exists.
    pub modified: Option<SystemTime>,
    /// Its size.
    pub len: u64,
}

impl Stamp {
    /// The stamp of a file, or the empty stamp when it is not there.
    pub fn of(path: &Path) -> Self {
        match fs::metadata(path) {
            Ok(meta) => Self {
                modified: meta.modified().ok(),
                len: meta.len(),
            },
            Err(_) => Self::default(),
        }
    }
}

/// An operation git has in progress, said before the branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// A merge stopped for conflicts or a message.
    Merge,
    /// A rebase.
    Rebase,
    /// A cherry-pick.
    CherryPick,
    /// A revert.
    Revert,
    /// A bisect.
    Bisect,
}

impl Operation {
    /// Its words: "Merge in progress".
    pub fn words(self) -> &'static str {
        match self {
            Operation::Merge => "Merge in progress",
            Operation::Rebase => "Rebase in progress",
            Operation::CherryPick => "Cherry-pick in progress",
            Operation::Revert => "Revert in progress",
            Operation::Bisect => "Bisect in progress",
        }
    }
}

/// Reads a `.git` file's "gitdir: PATH" line, relative to the folder it
/// is in.
fn gitdir_of_file(file: &Path, folder: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(file).ok()?;
    let line = text.lines().next()?.trim();
    let target = line.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        folder.join(target)
    })
}

/// The shared folder of a worktree's repository folder: what its
/// `commondir` file names, or itself.
fn common_of(git_dir: &Path) -> PathBuf {
    match fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) if !text.trim().is_empty() => {
            let target = Path::new(text.trim());
            if target.is_absolute() {
                target.to_path_buf()
            } else {
                git_dir.join(target)
            }
        }
        _ => git_dir.to_path_buf(),
    }
}

/// The repository a local folder is in, or `None`.
pub fn find(folder: &Path) -> Option<Repo> {
    find_below(folder, &[])
}

/// The ceiling folders git is told not to climb into
/// (`GIT_CEILING_DIRECTORIES`), read from this process's environment.
pub fn ceilings_from_environment() -> Vec<PathBuf> {
    std::env::var_os("GIT_CEILING_DIRECTORIES")
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default()
}

/// The repository a local folder is in, never looking in a ceiling folder
/// or above it (the folder itself is always looked in), as git does with
/// `GIT_CEILING_DIRECTORIES`.
pub fn find_below(folder: &Path, ceilings: &[PathBuf]) -> Option<Repo> {
    for at in folder.ancestors().take(MAX_DEPTH) {
        if at != folder && ceilings.iter().any(|c| c == at) {
            return None;
        }
        let marker = at.join(".git");
        let Ok(meta) = fs::symlink_metadata(&marker) else {
            continue;
        };
        let git_dir = if meta.is_dir() {
            marker
        } else if meta.is_file() {
            gitdir_of_file(&marker, at)?
        } else {
            continue;
        };
        return Some(Repo {
            root: at.to_path_buf(),
            common_dir: common_of(&git_dir),
            git_dir,
        });
    }
    None
}

/// A path's place below the top, with `/` between names: "src/lib.rs",
/// or "" for the top itself; `None` when it is not inside.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let mut names = Vec::new();
    for part in rest.components() {
        match part {
            Component::Normal(name) => names.push(name.to_str()?.to_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(names.join("/"))
}

impl Repo {
    /// A path's place below this working tree's top.
    pub fn relative(&self, path: &Path) -> Option<String> {
        relative(&self.root, path)
    }

    /// The stamps the status cache compares: the index, `HEAD`, and the
    /// folder asked about.
    pub fn status_stamps(&self, folder: &Path) -> [Stamp; 3] {
        [
            Stamp::of(&self.git_dir.join("index")),
            Stamp::of(&self.git_dir.join("HEAD")),
            Stamp::of(folder),
        ]
    }

    /// The stamps the scan cache compares: the configuration files.
    pub fn config_stamps(&self) -> [Stamp; 2] {
        [
            Stamp::of(&self.common_dir.join("config")),
            Stamp::of(&self.git_dir.join("config.worktree")),
        ]
    }

    /// The operation in progress, if any.
    pub fn operation(&self) -> Option<Operation> {
        let has = |name: &str| self.git_dir.join(name).exists();
        if has("rebase-merge") || has("rebase-apply") {
            Some(Operation::Rebase)
        } else if has("MERGE_HEAD") {
            Some(Operation::Merge)
        } else if has("CHERRY_PICK_HEAD") {
            Some(Operation::CherryPick)
        } else if has("REVERT_HEAD") {
            Some(Operation::Revert)
        } else if has("BISECT_LOG") {
            Some(Operation::Bisect)
        } else {
            None
        }
    }

    /// Whether a commit here runs hooks of the repository's own, in its
    /// hooks folder (`core.hooksPath` elsewhere is the trust scan's).
    pub fn has_commit_hooks(&self) -> bool {
        [
            "pre-commit",
            "prepare-commit-msg",
            "commit-msg",
            "post-commit",
        ]
        .iter()
        .any(|hook| self.common_dir.join("hooks").join(hook).is_file())
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
            .prefix("repo-")
            .tempdir_in(base)
            .unwrap()
    }

    #[test]
    fn a_folder_finds_its_repository_by_walking_up() {
        let tmp = scratch();
        let root = tmp.path().join("work");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("src").join("deep")).unwrap();
        let repo = find(&root.join("src").join("deep")).unwrap();
        assert_eq!(
            find_below(&root.join("src"), &[root.join("src")])
                .unwrap()
                .root,
            root
        );
        assert_eq!(
            find_below(&root.join("src").join("deep"), &[root.join("src")]),
            None
        );
        assert_eq!(repo.root, root);
        assert_eq!(repo.git_dir, root.join(".git"));
        assert_eq!(repo.common_dir, root.join(".git"));
        assert_eq!(
            repo.relative(&root.join("src").join("deep")).as_deref(),
            Some("src/deep")
        );
        assert_eq!(repo.relative(&root).as_deref(), Some(""));
        assert_eq!(repo.relative(tmp.path()), None);
        assert_eq!(repo.operation(), None);
        fs::write(root.join(".git").join("MERGE_HEAD"), "x").unwrap();
        assert_eq!(repo.operation(), Some(Operation::Merge));
        assert!(!repo.has_commit_hooks());
    }

    #[test]
    fn a_linked_worktree_s_git_file_names_its_repository() {
        let tmp = scratch();
        let main_git = tmp.path().join("main").join(".git");
        let wt_git = main_git.join("worktrees").join("wt");
        fs::create_dir_all(&wt_git).unwrap();
        fs::write(wt_git.join("commondir"), "../..").unwrap();
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        let repo = find(&wt).unwrap();
        assert_eq!(repo.root, wt);
        assert_eq!(repo.git_dir, wt_git);
        assert_eq!(repo.common_dir, wt_git.join("../.."));
        // A .git file that names nothing is no repository.
        let bad = tmp.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join(".git"), "nonsense").unwrap();
        assert_eq!(find(&bad), None);
    }
}
