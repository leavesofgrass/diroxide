//! Every line the plugin says or hands the host, in one place, so each is
//! tested at 40 cells with its meaning first (the owner reads one line of
//! a 40-cell Braille display at a time). Words before symbols; the host
//! adds the plugin's name, "(Git)", to every line it says, so no line here
//! names the plugin. A cause is lowercase words for the host's "Could not
//! VERB OBJECT: CAUSE." line; a next step is a sentence of its own.

use diroxide_core::format::{count, human_size};

use crate::git::status::Branch;
use crate::git::version::{MINIMUM, Version};
use crate::repo::Operation;

/// The longest the title line's words should be: the app adds " (Git)"
/// and keeps 40 (the host-pieces lane's finding).
pub const TITLE_CELLS: usize = 33;

// ---- git itself ----

/// The title line, and a command's cause, when the host finds no git.
pub const GIT_NOT_FOUND: &str = "git not found";

/// The next step when git is missing or too old.
pub const GET_GIT: &str = "Install git from git-scm.com or your package manager, then try again.";

/// "git is too old: 2.20, needs 2.30" (32).
pub fn too_old(version: Version) -> String {
    format!("git is too old: {version}, needs {MINIMUM}")
}

/// A command's cause outside any repository.
pub const NOT_A_REPOSITORY: &str = "this folder is not in a git repository";

/// The title line, the column and a command's cause when git refuses a
/// repository for its owner (33).
pub const DUBIOUS: &str = "git refuses it: dubious ownership";

/// The next step for it.
pub const SAFE_DIRECTORY_HINT: &str =
    "If it is yours, run git config --global --add safe.directory with its path, in a terminal.";

// ---- the trust scan (answer 123) ----

/// The title line of a repository whose configuration runs a program and
/// that the owner has not trusted (38).
pub const NOT_TRUSTED: &str = "Not trusted: its config runs a program";

/// The Git column's cell in such a repository.
pub const NOT_CHECKED: &str = "not checked";

/// The trust question's title; the host adds the plugin's name.
pub const TRUST_TITLE: &str = "Trust this repository?";

/// The trust question's lines: what was found, then what a yes means.
pub fn trust_lines(root: &str, hits: &[String]) -> Vec<String> {
    let mut lines = vec![NOT_TRUSTED.to_owned()];
    lines.extend(hits.iter().take(5).cloned());
    if hits.len() > 5 {
        lines.push(format!("and {} more", hits.len() - 5));
    }
    lines.push(format!("Repository: {root}"));
    lines.push("Say yes only if you know where it came from.".to_owned());
    lines.push("Status still never runs its file monitor.".to_owned());
    lines
}

/// The next step after a no.
pub const TRUST_HINT: &str = "Use Git: trust this repository, when you know where it came from.";

/// What the trust command says of a configuration that runs nothing.
pub const NOTHING_TO_TRUST: &str = "Trusted: its config runs no program";

/// What it says after a yes.
pub const TRUSTED: &str = "Trusted: git status runs here now";

/// What it says after a no.
pub const NOT_TRUSTED_KEPT: &str = "Not trusted: git status stays off here";

// ---- the title line ----

/// What the title line is made from.
#[derive(Clone, Debug, Default)]
pub struct TitleFacts {
    /// The branch headers.
    pub branch: Branch,
    /// An operation in progress.
    pub operation: Option<Operation>,
    /// How many paths changed in this folder and below.
    pub here: usize,
    /// How many of those are in conflict.
    pub conflicts: usize,
    /// Whether untracked files were counted (a slow status leaves them
    /// out).
    pub untracked_counted: bool,
}

fn upstream_words(branch: &Branch) -> Option<String> {
    // A detached head, or a branch with no commit yet, has no upstream to
    // speak of.
    branch.head.as_ref()?;
    branch.oid.as_ref()?;
    if branch.upstream.is_none() {
        return Some("no upstream".to_owned());
    }
    let ahead = branch.ahead.unwrap_or(0);
    let behind = branch.behind.unwrap_or(0);
    Some(match (ahead, behind) {
        (0, 0) => "up to date".to_owned(),
        (a, 0) => format!("{a} ahead"),
        (0, b) => format!("{b} behind"),
        (a, b) => format!("{a} ahead, {b} behind"),
    })
}

fn head_words(branch: &Branch) -> String {
    match (&branch.head, &branch.oid) {
        (Some(name), Some(_)) => name.clone(),
        (Some(name), None) => format!("{name}, no commits yet"),
        (None, Some(oid)) => format!("Detached at {}", oid.chars().take(7).collect::<String>()),
        (None, None) => "Detached".to_owned(),
    }
}

/// The title line's words, the branch and its state first: "main, 2
/// ahead, 3 changed here" (29), "main, no upstream, 1 changed here" (33),
/// "Detached at a1b2c3d, 3 changed" (30), "Merge in progress, 2
/// conflicts" (30). "here" is this folder and below; when the words pass
/// [`TITLE_CELLS`], "here" goes first.
pub fn title(facts: &TitleFacts) -> String {
    if let Some(op) = facts.operation
        && facts.conflicts > 0
    {
        return format!(
            "{}, {}",
            op.words(),
            count(facts.conflicts as u64, "conflict", "conflicts")
        );
    }
    let head = match facts.operation {
        Some(op) => op.words().to_owned(),
        None => head_words(&facts.branch),
    };
    let mut parts = vec![head];
    if facts.operation.is_none()
        && let Some(upstream) = upstream_words(&facts.branch)
    {
        parts.push(upstream);
    }
    if facts.conflicts > 0 {
        parts.push(count(facts.conflicts as u64, "conflict", "conflicts"));
    }
    let changes = |here: bool| -> String {
        let place = if here { " here" } else { "" };
        match (facts.here, facts.untracked_counted) {
            (0, true) => format!("no changes{place}"),
            (n, true) => format!("{n} changed{place}"),
            (n, false) => format!("{n} changed, untracked not counted"),
        }
    };
    let long = [parts.clone(), vec![changes(true)]].concat().join(", ");
    if long.chars().count() <= TITLE_CELLS {
        return long;
    }
    [parts, vec![changes(false)]].concat().join(", ")
}

// ---- the Git column ----

/// A tracked file with no change.
pub const CLEAN: &str = "Clean";

/// A path git ignores.
pub const IGNORED: &str = "Ignored";

/// A place outside any repository.
pub const NO_REPOSITORY: &str = "Not in a repository";

/// A folder whose files changed: "3 changed inside" (16).
pub fn changed_inside(n: usize) -> String {
    format!("{n} changed inside")
}

// ---- rows ----

/// A changed file's row, the state first: "Modified: hello.txt" (19),
/// "Renamed: b.txt, from a.txt".
pub fn change_row(word: &str, path: &str, from: Option<&str>) -> String {
    match from {
        Some(from) => format!("{word}: {path}, from {from}"),
        None => format!("{word}: {path}"),
    }
}

/// The status list's title, the count first: "3 changes, main, 2
/// ahead".
pub fn status_title(changes: usize, branch: &Branch) -> String {
    let mut title = count(changes as u64, "change", "changes");
    if title == "no changes" {
        title = "No changes".to_owned();
    }
    title.push_str(", ");
    title.push_str(&head_words(branch));
    if let Some(upstream) = upstream_words(branch) {
        title.push_str(", ");
        title.push_str(&upstream);
    }
    title
}

/// The log list's title: "Log of src: 42 commits".
pub fn log_title(what: &str, commits: usize) -> String {
    format!(
        "Log of {what}: {}",
        count(commits as u64, "commit", "commits")
    )
}

/// A commit's file list's title: "Commit a1b2c3d: 3 files".
pub fn commit_title(short: &str, files: usize) -> String {
    format!("Commit {short}: {}", count(files as u64, "file", "files"))
}

/// The branch list's title: "Branches: 3, on main".
pub fn branches_title(n: usize, current: Option<&str>) -> String {
    match current {
        Some(name) => format!("Branches: {n}, on {name}"),
        None => format!("Branches: {n}"),
    }
}

/// The stash list's title: "Stashes: 2".
pub fn stashes_title(n: usize) -> String {
    format!("Stashes: {n}")
}

// ---- stage, unstage, ignore ----

fn files(paths: &[String]) -> String {
    match paths {
        [one] => one.clone(),
        _ => count(paths.len() as u64, "file", "files"),
    }
}

/// "Staged hello.txt", "Staged 2 files".
pub fn staged(paths: &[String]) -> String {
    format!("Staged {}", files(paths))
}

/// "Staged all changes here: 5 files".
pub fn staged_all(n: usize) -> String {
    format!(
        "Staged all changes here: {}",
        count(n as u64, "file", "files")
    )
}

/// "Unstaged hello.txt", "Unstaged 2 files".
pub fn unstaged(paths: &[String]) -> String {
    format!("Unstaged {}", files(paths))
}

/// "Added 2 patterns to .gitignore".
pub fn ignored(n: usize) -> String {
    format!(
        "Added {} to .gitignore",
        count(n as u64, "pattern", "patterns")
    )
}

/// What Add to .gitignore says when every pattern is there already.
pub const ALREADY_IGNORED: &str = "Already in .gitignore: nothing added";

/// What a command on changed files says when none of its files changed.
pub const NOTHING_CHANGED: &str = "Nothing changed in the marked files";

// ---- commit ----

/// The commit form's title.
pub const COMMIT_TITLE: &str = "Commit";

/// The commit form's summary: "2 files staged on main" (22), then each
/// file in words.
pub fn commit_summary(staged: &[String], branch: &str) -> String {
    let mut summary = format!(
        "{} staged on {branch}.",
        count(staged.len() as u64, "file", "files")
    );
    for row in staged.iter().take(10) {
        summary.push(' ');
        summary.push_str(row);
        summary.push('.');
    }
    if staged.len() > 10 {
        summary.push_str(&format!(" And {} more.", staged.len() - 10));
    }
    summary
}

/// "Committed 1 file on main: a1b2c3d" (33).
pub fn committed(files: usize, branch: &str, short: &str) -> String {
    format!(
        "Committed {} on {branch}: {short}",
        count(files as u64, "file", "files")
    )
}

/// "Amended the last commit on main: a1b2c3d".
pub fn amended(branch: &str, short: &str) -> String {
    format!("Amended the last commit on {branch}: {short}")
}

/// Commit with nothing staged (33).
pub const NOTHING_STAGED: &str = "Nothing staged: stage files first";

/// Commit with an empty message (39).
pub const EMPTY_MESSAGE: &str = "Nothing to commit: the message is empty";

/// git knows no author (38).
pub const NO_IDENTITY: &str = "Author identity unknown: set user.name";

/// The next step for it.
pub const SET_IDENTITY: &str =
    "In a terminal, run git config --global user.name and user.email, then try again.";

/// Said while a commit runs the repository's hooks (37).
pub const RUNNING_HOOKS: &str = "Running the repository's commit hooks";

/// "Resolve conflicts first: 2 files".
pub fn conflicts_first(n: usize) -> String {
    format!(
        "Resolve conflicts first: {}",
        count(n as u64, "file", "files")
    )
}

/// A hook or git stopped the commit: the first line of what it said.
pub fn commit_stopped(reason: &str) -> String {
    format!("Commit stopped: {reason}")
}

// ---- diff, log, blame ----

/// A side's title: the commit before the one shown.
pub fn before(short: &str) -> String {
    format!("before {short}")
}

/// "No changes in hello.txt".
pub fn no_changes(name: &str) -> String {
    format!("No changes in {name}")
}

/// "Binary file changed: logo.png, 120 KB to 130 KB".
pub fn binary(name: &str, old: u64, new: u64) -> String {
    format!(
        "Binary file changed: {name}, {} to {}",
        human_size(old),
        human_size(new)
    )
}

/// "Too large to compare: big.log, 12.0 MB".
pub fn too_large(name: &str, size: u64) -> String {
    format!("Too large to compare: {name}, {}", human_size(size))
}

/// The blame question's title.
pub const BLAME_TITLE: &str = "Who changed which line?";

/// The blame question's line.
pub const BLAME_LINE: &str = "Line number:";

/// What a log of a path with no commits says.
pub fn no_commits(what: &str) -> String {
    format!("No commits yet for {what}")
}

// ---- branches and stashes ----

/// "Switched to topic".
pub fn switched(name: &str) -> String {
    format!("Switched to {name}")
}

/// git refused to switch: "Cannot switch: hello.txt has local changes".
pub fn cannot_switch_changes(paths: &[String]) -> String {
    match paths {
        [one] => format!("Cannot switch: {one} has local changes"),
        _ => format!(
            "Cannot switch: {} have local changes",
            count(paths.len() as u64, "file", "files")
        ),
    }
}

/// The next step after it.
pub const COMMIT_OR_STASH: &str = "Commit or stash your changes first, then switch.";

/// "Created branch topic and switched to it".
pub fn created_branch(name: &str) -> String {
    format!("Created branch {name} and switched to it")
}

/// "Renamed branch topic to feature".
pub fn renamed_branch(old: &str, new: &str) -> String {
    format!("Renamed branch {old} to {new}")
}

/// The new-branch question.
pub const NEW_BRANCH_TITLE: &str = "New branch";

/// Its line.
pub const BRANCH_NAME: &str = "Branch name:";

/// The rename question.
pub const RENAME_BRANCH_TITLE: &str = "Rename branch";

/// What a bad branch name says.
pub const BAD_BRANCH_NAME: &str = "Not a branch name: use letters, digits, - and /";

/// The stash question.
pub const STASH_TITLE: &str = "Stash changes";

/// Its line.
pub const STASH_MESSAGE: &str = "Message, or empty:";

/// "Stashed changes as stash 0".
pub const STASHED: &str = "Stashed changes as stash 0";

/// Stash with nothing to stash.
pub const NOTHING_TO_STASH: &str = "Nothing to stash: no local changes";

/// "Popped stash 2".
pub fn popped(n: u32) -> String {
    format!("Popped stash {n}")
}

/// "Dropped stash 2".
pub fn dropped(n: u32) -> String {
    format!("Dropped stash {n}")
}

/// The drop question's lines: what is dropped, then that it is for good.
pub fn drop_lines(row: &str) -> Vec<String> {
    vec![row.to_owned(), "It cannot be undone.".to_owned()]
}

/// The drop question's title: "Drop stash 2?".
pub fn drop_title(n: u32) -> String {
    format!("Drop stash {n}?")
}

/// The stash list changed since it was shown.
pub const STASH_MOVED: &str = "The stashes changed: open the list again";

/// No stash at all.
pub const NO_STASHES: &str = "No stashes here";

// ---- discard ----

/// The discard question's title: "Discard changes to 3 files?" (27).
pub fn discard_title(n: usize) -> String {
    format!("Discard changes to {}?", count(n as u64, "file", "files"))
}

/// The discard question's lines: each file, then where the changes go.
pub fn discard_lines(rows: &[String], skipped_untracked: usize) -> Vec<String> {
    let mut lines: Vec<String> = rows.iter().take(20).cloned().collect();
    if rows.len() > 20 {
        lines.push(format!("and {} more", rows.len() - 20));
    }
    if skipped_untracked > 0 {
        lines.push(format!(
            "{} left as they are",
            count(
                skipped_untracked as u64,
                "untracked file",
                "untracked files"
            )
        ));
    }
    lines.push("Staged changes stay staged.".to_owned());
    lines.push("The changes are kept in a stash.".to_owned());
    lines
}

/// "Discarded 3 files, kept in stash 0" (34).
pub fn discarded(n: usize, stash: u32) -> String {
    format!(
        "Discarded {}, kept in stash {stash}",
        count(n as u64, "file", "files")
    )
}

/// The stash message a discard leaves, with the time from the clock.
pub fn discard_message(time: &str) -> String {
    format!("diroxide discard {time}")
}

/// "Brought back 2 files from stash 0".
pub fn brought_back(n: usize, stash: u32) -> String {
    format!(
        "Brought back {} from stash {stash}",
        count(n as u64, "file", "files")
    )
}

/// A discard undone over a file changed since: "Not brought back:
/// hello.txt changed since".
pub fn changed_since_discard(paths: &[String]) -> String {
    match paths {
        [one] => format!("Not brought back: {one} changed since"),
        _ => format!(
            "Not brought back: {} changed since",
            count(paths.len() as u64, "file", "files")
        ),
    }
}

// ---- remotes ----

/// Pull that is not a fast-forward (32).
pub const NOT_FAST_FORWARD: &str = "Pull stopped: not a fast-forward";

/// Its next step.
pub const MERGE_IN_TERMINAL: &str =
    "Your branch and its upstream both have new commits; merge or rebase them in a terminal.";

/// A push the remote refused (33).
pub const PUSH_REFUSED: &str = "Push refused: remote has new work";

/// Its next step.
pub const PULL_FIRST: &str =
    "Pull first; or, to replace what is there, use Git: replace remote history.";

/// git needed a password or a key it could not ask for (35).
pub const SIGN_IN: &str = "Sign-in needed: git cannot ask here";

/// Its next step.
pub const SIGN_IN_HINT: &str =
    "Load your key with ssh-add, or sign in with your credential manager, then try again.";

/// A branch with no upstream to pull from or replace.
pub const NO_UPSTREAM: &str = "This branch has no upstream";

/// A repository with no remote at all.
pub const NO_REMOTE: &str = "This repository has no remote";

/// Nothing to pull.
pub const UP_TO_DATE: &str = "Already up to date";

/// Nothing to push.
pub const NOTHING_TO_PUSH: &str = "Nothing to push: up to date";

/// "Fetched from origin".
pub fn fetched(remote: &str) -> String {
    format!("Fetched from {remote}")
}

/// "Fetched: main is 3 behind origin/main".
pub fn fetched_behind(branch: &str, behind: u32, upstream: &str) -> String {
    format!("Fetched: {branch} is {behind} behind {upstream}")
}

/// "Pulled 3 commits into main".
pub fn pulled(n: u32, branch: &str) -> String {
    format!(
        "Pulled {} into {branch}",
        count(u64::from(n), "commit", "commits")
    )
}

/// The push question: "Push 2 commits to origin/main?" (30).
pub fn push_question(n: u32, upstream: &str) -> String {
    format!(
        "Push {} to {upstream}?",
        count(u64::from(n), "commit", "commits")
    )
}

/// The push question for a branch with no upstream yet.
pub fn push_new_question(branch: &str, remote: &str) -> String {
    format!("Push {branch} to {remote}?")
}

/// "Pushed 2 commits to origin/main".
pub fn pushed(n: u32, upstream: &str) -> String {
    format!(
        "Pushed {} to {upstream}",
        count(u64::from(n), "commit", "commits")
    )
}

/// "Pushed main to origin".
pub fn pushed_new(branch: &str, remote: &str) -> String {
    format!("Pushed {branch} to {remote}")
}

/// The lease push's first question: "Replace history of origin/main?"
/// (31).
pub fn replace_question(upstream: &str) -> String {
    format!("Replace history of {upstream}?")
}

/// Its lines.
pub fn replace_lines(branch: &str, upstream: &str) -> Vec<String> {
    vec![
        format!("{upstream} becomes what {branch} is here."),
        format!("Commits only on {upstream} are removed there."),
        "Others who have them must repair their copies.".to_owned(),
    ]
}

/// The second question: "Really replace origin/main?" (27).
pub fn really_replace(upstream: &str) -> String {
    format!("Really replace {upstream}?")
}

/// "Replaced the history of origin/main".
pub fn replaced(upstream: &str) -> String {
    format!("Replaced the history of {upstream}")
}

/// The lease refused: someone pushed since the last fetch.
pub const LEASE_REFUSED: &str = "Not replaced: the remote changed since";

/// Its next step.
pub const FETCH_FIRST: &str = "Fetch, look at what is there, then decide again.";

// ---- clone and init ----

/// The clone form's title.
pub const CLONE_TITLE: &str = "Clone a repository";

/// "Cloned into repo".
pub fn cloned(name: &str) -> String {
    format!("Cloned into {name}")
}

/// A clone into a folder that is there already.
pub fn folder_exists(name: &str) -> String {
    format!("A folder named {name} is here already")
}

/// A clone that stopped: "Clone stopped: repository not found".
pub fn clone_stopped(reason: &str) -> String {
    format!("Clone stopped: {reason}")
}

/// The init form's title.
pub const INIT_TITLE: &str = "Make this folder a repository";

/// "Made a repository on main: 12 files committed".
pub fn made_repository(branch: &str, committed: Option<usize>) -> String {
    match committed {
        Some(n) => format!(
            "Made a repository on {branch}: {} committed",
            count(n as u64, "file", "files")
        ),
        None => format!("Made a repository on {branch}"),
    }
}

/// Init in a folder that is one already.
pub const ALREADY_A_REPOSITORY: &str = "This folder is a repository already";

/// Init inside another repository's working tree.
pub fn inside_another(root: &str) -> String {
    format!("This folder is inside the repository at {root}")
}

/// What a folder name the plain-name rule refuses says.
pub const PLAIN_NAME: &str = "Use letters, digits, -, _ and . in the folder name";

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::{BRAILLE_CELLS, within_cells};

    fn fits(line: &str) {
        assert!(
            line.chars().count() <= BRAILLE_CELLS,
            "{line} is {} cells",
            line.chars().count()
        );
    }

    fn branch(head: Option<&str>, upstream: Option<&str>, ahead: u32, behind: u32) -> Branch {
        Branch {
            oid: Some("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into()),
            head: head.map(str::to_owned),
            upstream: upstream.map(str::to_owned),
            ahead: upstream.map(|_| ahead),
            behind: upstream.map(|_| behind),
        }
    }

    fn facts(branch: Branch, here: usize) -> TitleFacts {
        TitleFacts {
            branch,
            here,
            untracked_counted: true,
            ..TitleFacts::default()
        }
    }

    #[test]
    fn the_title_words_put_the_branch_first_within_33_cells() {
        let cases = [
            (
                facts(branch(Some("main"), Some("origin/main"), 2, 0), 3),
                "main, 2 ahead, 3 changed here",
            ),
            (
                facts(branch(Some("main"), None, 0, 0), 1),
                "main, no upstream, 1 changed here",
            ),
            (
                facts(branch(None, None, 0, 0), 3),
                "Detached at a1b2c3d, 3 changed",
            ),
            (
                facts(branch(Some("main"), Some("origin/main"), 0, 0), 0),
                "main, up to date, no changes here",
            ),
            (
                facts(branch(Some("main"), Some("origin/main"), 2, 1), 3),
                "main, 2 ahead, 1 behind, 3 changed",
            ),
            (
                TitleFacts {
                    operation: Some(Operation::Merge),
                    conflicts: 2,
                    ..facts(branch(Some("main"), None, 0, 0), 4)
                },
                "Merge in progress, 2 conflicts",
            ),
            (
                TitleFacts {
                    operation: Some(Operation::Rebase),
                    ..facts(branch(None, None, 0, 0), 1)
                },
                "Rebase in progress, 1 changed",
            ),
        ];
        for (facts, words) in cases {
            let line = title(&facts);
            assert_eq!(line, words);
            fits(&line);
        }
        let mut initial = branch(Some("main"), None, 0, 0);
        initial.oid = None;
        let line = title(&facts(initial, 2));
        assert_eq!(line, "main, no commits yet, 2 changed");
        assert!(within_cells(&line, "no commits yet"));
        // The four the brief names stay within the 33 the app keeps.
        for words in [
            "main, 2 ahead, 3 changed here",
            "main, no upstream, 1 changed here",
            "Detached at a1b2c3d, 3 changed",
            "Merge in progress, 2 conflicts",
        ] {
            assert!(words.chars().count() <= TITLE_CELLS);
        }
    }

    #[test]
    fn every_fixed_line_fits_40_cells_with_its_meaning_first() {
        for line in [
            GIT_NOT_FOUND,
            DUBIOUS,
            NOT_TRUSTED,
            TRUST_TITLE,
            NOTHING_TO_TRUST,
            TRUSTED,
            NOT_TRUSTED_KEPT,
            CLEAN,
            IGNORED,
            NO_REPOSITORY,
            ALREADY_IGNORED,
            NOTHING_CHANGED,
            NOTHING_STAGED,
            EMPTY_MESSAGE,
            NO_IDENTITY,
            RUNNING_HOOKS,
            BLAME_TITLE,
            NEW_BRANCH_TITLE,
            RENAME_BRANCH_TITLE,
            STASH_TITLE,
            STASHED,
            NOTHING_TO_STASH,
            STASH_MOVED,
            NO_STASHES,
            NOT_FAST_FORWARD,
            PUSH_REFUSED,
            SIGN_IN,
            NO_UPSTREAM,
            NO_REMOTE,
            UP_TO_DATE,
            NOTHING_TO_PUSH,
            LEASE_REFUSED,
            CLONE_TITLE,
            INIT_TITLE,
            ALREADY_A_REPOSITORY,
        ] {
            fits(line);
        }
        assert_eq!(NOT_TRUSTED.chars().count(), 38);
        assert_eq!(SIGN_IN.chars().count(), 35);
        assert_eq!(PUSH_REFUSED.chars().count(), 33);
        assert_eq!(NOT_FAST_FORWARD.chars().count(), 32);
    }

    #[test]
    fn the_built_lines_fit_with_their_numbers_first() {
        let v = Version {
            major: 2,
            minor: 20,
            patch: 1,
        };
        let line = too_old(v);
        assert_eq!(line, "git is too old: 2.20, needs 2.30");
        assert_eq!(line.chars().count(), 32);
        let built = [
            (
                committed(1, "main", "a1b2c3d"),
                "Committed 1 file on main: a1b2c3d",
            ),
            (discarded(3, 0), "Discarded 3 files, kept in stash 0"),
            (discard_title(3), "Discard changes to 3 files?"),
            (
                replace_question("origin/main"),
                "Replace history of origin/main?",
            ),
            (really_replace("origin/main"), "Really replace origin/main?"),
            (
                push_question(2, "origin/main"),
                "Push 2 commits to origin/main?",
            ),
            (pushed(2, "origin/main"), "Pushed 2 commits to origin/main"),
            (pulled(3, "main"), "Pulled 3 commits into main"),
            (
                change_row("Modified", "hello.txt", None),
                "Modified: hello.txt",
            ),
            (changed_inside(3), "3 changed inside"),
            (staged(&["hello.txt".into()]), "Staged hello.txt"),
            (staged(&["a".into(), "b".into()]), "Staged 2 files"),
            (unstaged(&["a".into(), "b".into()]), "Unstaged 2 files"),
            (ignored(2), "Added 2 patterns to .gitignore"),
            (switched("topic"), "Switched to topic"),
            (
                cannot_switch_changes(&["hello.txt".into()]),
                "Cannot switch: hello.txt has local changes",
            ),
            (
                created_branch("topic"),
                "Created branch topic and switched to it",
            ),
            (popped(2), "Popped stash 2"),
            (dropped(2), "Dropped stash 2"),
            (drop_title(2), "Drop stash 2?"),
            (conflicts_first(2), "Resolve conflicts first: 2 files"),
            (no_changes("hello.txt"), "No changes in hello.txt"),
            (
                binary("logo.png", 120_000, 130_000),
                "Binary file changed: logo.png, 120 KB to 130 KB",
            ),
            (
                too_large("big.log", 12_000_000),
                "Too large to compare: big.log, 12.0 MB",
            ),
            (cloned("repo"), "Cloned into repo"),
            (
                made_repository("main", Some(12)),
                "Made a repository on main: 12 files committed",
            ),
            (log_title("src", 42), "Log of src: 42 commits"),
            (commit_title("a1b2c3d", 3), "Commit a1b2c3d: 3 files"),
            (branches_title(3, Some("main")), "Branches: 3, on main"),
            (stashes_title(2), "Stashes: 2"),
            (
                fetched_behind("main", 3, "origin/main"),
                "Fetched: main is 3 behind origin/main",
            ),
            (
                replaced("origin/main"),
                "Replaced the history of origin/main",
            ),
            (staged_all(5), "Staged all changes here: 5 files"),
            (brought_back(2, 0), "Brought back 2 files from stash 0"),
            (
                changed_since_discard(&["hello.txt".into()]),
                "Not brought back: hello.txt changed since",
            ),
        ];
        for (line, words) in built {
            assert_eq!(line, words);
        }
        for line in [
            committed(1, "main", "a1b2c3d"),
            discarded(3, 0),
            discard_title(3),
            replace_question("origin/main"),
            really_replace("origin/main"),
            push_question(2, "origin/main"),
            pushed(2, "origin/main"),
            pulled(3, "main"),
            changed_inside(3),
            ignored(2),
            created_branch("topic"),
            conflicts_first(2),
            fetched_behind("main", 3, "origin/main"),
            replaced("origin/main"),
            staged_all(5),
            log_title("src", 42),
            commit_title("a1b2c3d", 3),
        ] {
            fits(&line);
        }
        // Long names push the end along; the meaning stays first.
        assert!(within_cells(
            &binary("a-very-long-file-name-for-a-picture.png", 1, 2),
            "Binary file changed"
        ));
        assert!(within_cells(
            &made_repository("main", Some(12)),
            "Made a repository on main"
        ));
        let summary = commit_summary(
            &["Modified: hello.txt".into(), "Added: notes.txt".into()],
            "main",
        );
        assert_eq!(
            summary,
            "2 files staged on main. Modified: hello.txt. Added: notes.txt."
        );
        assert!(within_cells(&summary, "2 files staged on main"));
        let lines = trust_lines("D:\\work\\repo", &["core.fsmonitor runs ./watch.sh".into()]);
        assert_eq!(lines[0], NOT_TRUSTED);
        assert!(
            lines
                .iter()
                .all(|l| within_cells(l, l.split(' ').next().unwrap()))
        );
        let lines = discard_lines(&["Modified: a.txt".into()], 2);
        assert_eq!(lines[1], "2 untracked files left as they are");
        assert_eq!(
            status_title(3, &branch(Some("main"), Some("origin/main"), 2, 0)),
            "3 changes, main, 2 ahead"
        );
        assert_eq!(
            status_title(0, &branch(Some("main"), None, 0, 0)),
            "No changes, main, no upstream"
        );
    }
}
