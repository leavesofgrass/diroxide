//! git's commands (plan section 5.16; R29 2.4): what each acts on, read
//! from the command's context (the active panel's folder, the marked
//! files, or the rows of one of the plugin's lists), and the lists' rows.
//!
//! Every command finds the repository by its `.git`, checks git, and
//! passes the trust scan (asking once when the owner has not answered)
//! before it runs anything. A command says its result in its own sentence
//! (`ui/announce`), so the host says no "done" of its own. A refusal it
//! expects (git missing, no repository, not trusted, nothing staged, a
//! pull that is not a fast-forward, a sign-in git cannot ask for) is said
//! the same way, the cause first at warning importance and the next step
//! after it, rather than as an error the host words "Could not run ...
//! (code ...)"; Escape answers cancelled, and only a broken call to the
//! host is an error.
//!
//! A list's row carries only a short id the plugin made; what the row
//! stands for (a path, a commit, a stash) stays here, since the host
//! cleans a row's words for the screen and a path with two spaces in a
//! row would not survive that.

mod branch;
mod commit;
mod diff;
mod history;
mod remote;
mod tree;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use diroxide_plugins::protocol::Params;
use serde_json::{Map, Value, json};

use crate::error::GitError;
use crate::folder::{self, local_path};
use crate::host::{Host, Progress};
use crate::lines;
use crate::plugin::Plugin;
use crate::repo::Repo;

/// The commands this build answers (plugin-local ids).
pub const IDS: &[&str] = &[
    "status",
    "diff",
    "diff_head",
    "stage",
    "unstage",
    "stage_all",
    "ignore",
    "discard",
    "commit",
    "log",
    "file_log",
    "commit_files",
    "blame",
    "branches",
    "switch",
    "branch_new",
    "branch_rename",
    "stashes",
    "stash_push",
    "stash_pop",
    "stash_drop",
    "stash_show",
    "fetch",
    "pull",
    "push",
    "push_lease",
];

/// The lists this build opens.
pub const LISTERS: &[&str] = &["status", "log", "commit", "branches", "stashes"];

/// How many opened lists keep their rows: a list shown in both panels,
/// and the ones before, still answer.
pub const KEPT_LISTS: usize = 32;

/// What one row of a list stands for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowData {
    /// The repository's top.
    pub root: PathBuf,
    /// A path from the top.
    pub path: Option<String>,
    /// A rename's original path.
    pub from: Option<String>,
    /// A commit's full hash.
    pub hash: Option<String>,
    /// A stash's number and full hash.
    pub stash: Option<(u32, String)>,
    /// A branch's name.
    pub branch: Option<String>,
    /// The state in words.
    pub word: String,
}

/// One row: its words, its visible columns, and what it stands for.
#[derive(Clone, Debug, Default)]
pub struct Row {
    /// The row's words, meaning first.
    pub name: String,
    /// The values of the list's columns, by column id.
    pub columns: Vec<(&'static str, String)>,
    /// What it stands for.
    pub data: RowData,
}

/// What a list shows, to build it again on a refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// The repository's changed files.
    Status {
        /// The repository's top.
        root: PathBuf,
    },
    /// The commits that changed a folder or a file.
    Log {
        /// The repository's top.
        root: PathBuf,
        /// The path from the top, empty for the whole folder.
        path: String,
        /// The words for what was logged.
        what: String,
    },
    /// What one commit changed.
    Commit {
        /// The repository's top.
        root: PathBuf,
        /// The commit's full hash.
        hash: String,
    },
    /// The local branches.
    Branches {
        /// The repository's top.
        root: PathBuf,
    },
    /// The stashes.
    Stashes {
        /// The repository's top.
        root: PathBuf,
    },
}

struct Opened {
    generation: u64,
    lister: String,
    view: View,
    rows: Vec<RowData>,
}

/// The rows of the lists the plugin opened, by a generation and a number,
/// so a row's id reads "7.12".
#[derive(Default)]
pub struct Listers {
    next: AtomicU64,
    opened: Mutex<VecDeque<Opened>>,
}

impl std::fmt::Debug for Listers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listers").finish_non_exhaustive()
    }
}

impl Listers {
    /// Keeps a list's rows and gives them back as the host's rows, each
    /// with its id.
    pub fn register(&self, lister: &str, view: View, rows: Vec<Row>) -> Vec<Value> {
        let generation = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let wire = rows
            .iter()
            .enumerate()
            .map(|(n, row)| {
                let mut extra = Map::new();
                extra.insert("id".into(), json!(format!("{generation}.{n}")));
                for (key, value) in &row.columns {
                    extra.insert((*key).to_owned(), json!(value));
                }
                json!({"name": row.name, "kind": "item", "extra": extra})
            })
            .collect();
        let mut opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        if opened.len() >= KEPT_LISTS {
            opened.pop_front();
        }
        opened.push_back(Opened {
            generation,
            lister: lister.to_owned(),
            view,
            rows: rows.into_iter().map(|r| r.data).collect(),
        });
        wire
    }

    /// What a row id stands for.
    pub fn find(&self, id: &str) -> Option<RowData> {
        let (generation, n) = id.split_once('.')?;
        let generation: u64 = generation.parse().ok()?;
        let n: usize = n.parse().ok()?;
        let opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        opened
            .iter()
            .find(|o| o.generation == generation)
            .and_then(|o| o.rows.get(n).cloned())
    }

    /// What the newest list of an id shows.
    pub fn latest(&self, lister: &str) -> Option<View> {
        let opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        opened
            .iter()
            .rev()
            .find(|o| o.lister == lister)
            .map(|o| o.view.clone())
    }

    /// A list left its panel. Its rows stay among the kept ones, since
    /// the same list may still show in the other panel; the oldest go as
    /// new ones come.
    pub fn closed(&self, lister: &str) {
        let _ = lister;
    }
}

/// What a command acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A row of one of the plugin's lists.
    Row(RowData),
    /// A file or folder in a panel.
    File {
        /// Its path.
        path: PathBuf,
        /// Whether it is a folder.
        folder: bool,
    },
}

/// A command's request: the plugin, the host and the context.
pub struct Ctx<'a> {
    /// The plugin.
    pub plugin: &'a Plugin,
    /// The way to the host.
    pub host: Host<'a>,
    /// The job's progress.
    pub progress: Progress,
    params: &'a Params,
}

impl Ctx<'_> {
    fn context(&self) -> Option<&Map<String, Value>> {
        self.params.get("context").and_then(Value::as_object)
    }

    /// The active panel's folder, on this computer.
    pub fn folder(&self) -> Result<PathBuf, GitError> {
        self.context()
            .and_then(|c| c.get("active"))
            .and_then(|a| a.get("place"))
            .and_then(Value::as_str)
            .and_then(local_path)
            .ok_or_else(|| GitError::Bad("the folder is not on this computer".to_owned()))
    }

    /// The repository of the active panel's folder, with git checked and
    /// the trust scan passed.
    pub fn repo(&self) -> Result<Repo, GitError> {
        let repo = self
            .plugin
            .find_repo(&self.folder()?)
            .ok_or(GitError::NotARepository)?;
        self.ready(&repo)?;
        Ok(repo)
    }

    /// git found and new enough, and the repository trusted.
    pub fn ready(&self, repo: &Repo) -> Result<(), GitError> {
        folder::git_ready(self.plugin, self.host)?;
        folder::ensure_trusted(self.plugin, self.host, repo)
    }

    /// What the command acts on: the list's rows, or the marked files
    /// (the cursor's when nothing is marked).
    pub fn targets(&self) -> Result<Vec<Target>, GitError> {
        let Some(context) = self.context() else {
            return Ok(Vec::new());
        };
        let mut entries: Vec<&Value> = context
            .get("selection")
            .and_then(Value::as_array)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        if entries.is_empty()
            && let Some(cursor) = context.get("cursor").filter(|c| !c.is_null())
        {
            entries.push(cursor);
        }
        let mut targets = Vec::new();
        for entry in entries {
            if let Some(id) = entry
                .get("extra")
                .and_then(|e| e.get("id"))
                .and_then(Value::as_str)
            {
                let row = self
                    .plugin
                    .listers
                    .find(id)
                    .ok_or_else(|| GitError::refused(LIST_CHANGED, None))?;
                targets.push(Target::Row(row));
                continue;
            }
            let Some(path) = entry
                .get("place")
                .and_then(Value::as_str)
                .and_then(local_path)
            else {
                continue;
            };
            let folder = entry.get("kind").and_then(Value::as_str) == Some("folder");
            targets.push(Target::File { path, folder });
        }
        Ok(targets)
    }

    /// The repository the targets are in and their paths from its top;
    /// targets in another repository than the first are refused.
    pub fn target_paths(&self) -> Result<(Repo, Vec<String>, Vec<Target>), GitError> {
        let targets = self.targets()?;
        let mut repo: Option<Repo> = None;
        let mut paths = Vec::new();
        for target in &targets {
            let (found, path) = match target {
                Target::Row(row) => (
                    self.plugin
                        .find_repo(&row.root)
                        .filter(|r| r.root == row.root),
                    row.path.clone(),
                ),
                Target::File { path, .. } => {
                    let found = self.plugin.find_repo(path.parent().unwrap_or(path));
                    let relative = found.as_ref().and_then(|r| r.relative(path));
                    (found, relative)
                }
            };
            let found = found.ok_or(GitError::NotARepository)?;
            if let Some(known) = &repo {
                if known.root != found.root {
                    return Err(GitError::refused(TWO_REPOSITORIES, None));
                }
            } else {
                repo = Some(found);
            }
            if let Some(path) = path.filter(|p| !p.is_empty()) {
                paths.push(path);
            }
        }
        let repo = match repo {
            Some(repo) => repo,
            None => self
                .plugin
                .find_repo(&self.folder()?)
                .ok_or(GitError::NotARepository)?,
        };
        self.ready(&repo)?;
        Ok((repo, paths, targets))
    }

    /// Says a command's result, and drops the repository's kept statuses.
    pub fn done(&self, repo: Option<&Repo>, sentence: &str, count: usize) -> Value {
        if let Some(repo) = repo {
            self.plugin.statuses.drop_repository(&repo.root);
        }
        self.host.announce(sentence, "info");
        json!({"done": count, "skipped": 0, "failed": []})
    }

    /// Says why nothing was done, at warning importance.
    pub fn nothing(&self, sentence: &str) -> Value {
        self.host.announce(sentence, "warning");
        json!({"done": 0, "skipped": 0, "failed": []})
    }

    /// Opens one of the plugin's lists over the active panel.
    pub fn open(
        &self,
        lister: &str,
        title: &str,
        view: View,
        rows: Vec<Row>,
    ) -> Result<Value, GitError> {
        let count = rows.len();
        let wire = self.plugin.listers.register(lister, view, rows);
        self.host.open_lister(lister, title, wire)?;
        Ok(json!({"done": count, "skipped": 0, "failed": []}))
    }

    /// Whether the command was run from one of the plugin's lists, and
    /// which.
    pub fn lister(&self) -> Option<&str> {
        self.params.get("lister").and_then(Value::as_str)
    }
}

/// What a row's id says when its list is gone.
pub const LIST_CHANGED: &str = "the list changed; open it again";

/// Targets in two repositories at once.
pub const TWO_REPOSITORIES: &str = "the marked files are in two repositories";

/// `command/run`: runs one command.
pub fn run(plugin: &Plugin, host: Host<'_>, params: &Params) -> Result<Value, GitError> {
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let command = command.rsplit('.').next().unwrap_or(command).to_owned();
    let token = params.get("progress_token").cloned();
    let ctx = Ctx {
        plugin,
        host,
        progress: Progress::new(host.peer, token),
        params,
    };
    let result = dispatch(&ctx, &command);
    ctx.progress.end(None);
    match result {
        Err(
            error @ (GitError::NotFound
            | GitError::TooOld(_)
            | GitError::NotARepository
            | GitError::NotTrusted
            | GitError::Refused { .. }),
        ) => {
            let rpc = error.to_rpc();
            host.announce(&capitalized(&rpc.message), "warning");
            if let Some(hint) = rpc.hint() {
                host.announce(hint, "info");
            }
            Ok(json!({"done": 0, "skipped": 0, "failed": []}))
        }
        other => other,
    }
}

/// A cause made a sentence: its first letter capitalized.
pub fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn dispatch(ctx: &Ctx<'_>, command: &str) -> Result<Value, GitError> {
    match command {
        "status" => tree::status(ctx),
        "stage" => tree::stage(ctx),
        "unstage" => tree::unstage(ctx),
        "stage_all" => tree::stage_all(ctx),
        "ignore" => tree::ignore(ctx),
        "discard" => tree::discard(ctx),
        "diff" => diff::diff(ctx, false),
        "diff_head" => diff::diff(ctx, true),
        "commit" => commit::commit(ctx),
        "log" => history::log(ctx, false),
        "file_log" => history::log(ctx, true),
        "commit_files" => history::commit_files(ctx),
        "blame" => history::blame(ctx),
        "branches" => branch::branches(ctx),
        "switch" => branch::switch(ctx),
        "branch_new" => branch::new_branch(ctx),
        "branch_rename" => branch::rename(ctx),
        "stashes" => branch::stashes(ctx),
        "stash_push" => branch::stash_push(ctx),
        "stash_pop" => branch::stash_pop(ctx),
        "stash_drop" => branch::stash_drop(ctx),
        "stash_show" => branch::stash_show(ctx),
        "fetch" => remote::fetch(ctx),
        "pull" => remote::pull(ctx),
        "push" => remote::push(ctx),
        "push_lease" => remote::push_lease(ctx),
        other => Err(GitError::Host(diroxide_plugins::protocol::RpcError::new(
            diroxide_plugins::protocol::ErrorKind::MethodNotFound,
            format!("no command {other}"),
        ))),
    }
}

/// `lister/rows`: a list's rows again, from its newest view, from
/// `offset`.
pub fn rows(plugin: &Plugin, host: Host<'_>, params: &Params) -> Result<Value, GitError> {
    let lister = params
        .get("lister")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let offset = params
        .get("offset")
        .and_then(Value::as_u64)
        .and_then(|o| usize::try_from(o).ok())
        .unwrap_or(0);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|o| usize::try_from(o).ok())
        .unwrap_or(usize::MAX);
    let Some(view) = plugin.listers.latest(lister) else {
        return Ok(json!({"rows": []}));
    };
    folder::git_ready(plugin, host)?;
    let rows = build(plugin, host, &view)?;
    let wire = plugin.listers.register(lister, view, rows);
    let page: Vec<Value> = wire.into_iter().skip(offset).take(limit).collect();
    Ok(json!({"rows": page}))
}

/// A view's rows, built again.
fn build(plugin: &Plugin, host: Host<'_>, view: &View) -> Result<Vec<Row>, GitError> {
    let repo = |root: &Path| -> Result<Repo, GitError> {
        let repo = plugin.find_repo(root).ok_or(GitError::NotARepository)?;
        folder::ensure_trusted(plugin, host, &repo)?;
        Ok(repo)
    };
    match view {
        View::Status { root } => Ok(tree::status_rows(host, &repo(root)?)?.1),
        View::Log { root, path, .. } => history::log_rows(host, &repo(root)?, path),
        View::Commit { root, hash } => history::commit_rows(host, &repo(root)?, hash),
        View::Branches { root } => branch::branch_rows(host, &repo(root)?),
        View::Stashes { root } => branch::stash_rows(host, &repo(root)?),
    }
}

/// A path's words for a sentence: its last name.
pub fn short_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// A place for the host, from a path: its text.
pub fn place_of(path: &Path) -> String {
    path.display().to_string()
}

/// Whether a sentence from git says it could not get a password or a key.
pub fn needs_sign_in(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "authentication failed",
        "permission denied (publickey",
        "host key verification failed",
        "invalid username or password",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// The lines for a command whose cause is git's own words.
pub fn git_refused(verb: &str, reason: &str) -> GitError {
    if needs_sign_in(reason) {
        return GitError::refused(lines::SIGN_IN, Some(lines::SIGN_IN_HINT));
    }
    GitError::refused(format!("{verb}: {reason}"), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_id_finds_what_it_stands_for_and_old_lists_drop_out() {
        let listers = Listers::default();
        let row = |path: &str| Row {
            name: format!("Modified: {path}"),
            columns: vec![("path", path.to_owned())],
            data: RowData {
                path: Some(path.to_owned()),
                ..RowData::default()
            },
        };
        let wire = listers.register(
            "status",
            View::Status {
                root: PathBuf::from("r"),
            },
            vec![row("a  b.txt"), row("c.txt")],
        );
        let id = wire[0]["extra"]["id"].as_str().unwrap().to_owned();
        assert_eq!(wire[0]["extra"]["path"], "a  b.txt");
        assert_eq!(listers.find(&id).unwrap().path.as_deref(), Some("a  b.txt"));
        assert!(listers.find("99.0").is_none());
        assert!(listers.find("junk").is_none());
        for _ in 0..KEPT_LISTS {
            listers.register(
                "other",
                View::Status {
                    root: PathBuf::from("other"),
                },
                Vec::new(),
            );
        }
        assert!(listers.find(&id).is_none());
        assert_eq!(
            listers.latest("other"),
            Some(View::Status {
                root: PathBuf::from("other")
            })
        );
        assert_eq!(listers.latest("status"), None);
    }

    #[test]
    fn sign_in_failures_are_told_apart() {
        assert!(needs_sign_in(
            "fatal: could not read Username for 'https://example.invalid': terminal prompts disabled"
        ));
        assert!(needs_sign_in(
            "git@example.invalid: Permission denied (publickey)."
        ));
        assert!(!needs_sign_in("fatal: repository not found"));
        assert_eq!(
            git_refused("Could not fetch", "terminal prompts disabled").to_string(),
            lines::SIGN_IN
        );
        assert_eq!(short_name("src/lib.rs"), "lib.rs");
        assert_eq!(short_name("u/"), "u");
    }
}
