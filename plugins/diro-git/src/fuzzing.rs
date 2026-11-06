//! The bodies of the fuzz targets (R30 commit 106), here so the plugin's
//! own tests replay them on their seeds, and the fuzz crate under
//! `plugins/diro-git/fuzz` stays a thin wrapper (`docs/dev/fuzzing.md`'s
//! shape). Each reads what git or a hostile repository could hand the
//! plugin and must never panic, loop, or allocate by a number it read.

use crate::git::{blame, log, progress, refs, status, trust};
use crate::lines;

/// The porcelain v2 status reader, and the words of every change it
/// reads.
pub fn status_v2(data: &[u8]) {
    if let Ok(status) = status::parse(data) {
        for change in &status.changes {
            let _ = change.word();
        }
        let _ = lines::title(&lines::TitleFacts {
            branch: status.branch.clone(),
            here: status.changes.len(),
            conflicts: status.conflicts(),
            untracked_counted: true,
            ..lines::TitleFacts::default()
        });
        let _ = status.under("a").count();
    }
}

/// The log record reader, and the commit-files reader.
pub fn log_record(data: &[u8]) {
    for commit in log::parse_log(data) {
        let _ = commit.row();
    }
    for change in log::parse_name_status(data) {
        let _ = change.row();
    }
}

/// The trust scan over a configuration listing.
pub fn config_scan(data: &[u8]) {
    let scan = trust::scan(data);
    for hit in &scan.hits {
        let _ = hit.words();
    }
}

/// The other readers of git's words: blame, branches, stashes, progress.
pub fn other_readers(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    if let Some(blame) = blame::parse(&text) {
        let _ = blame.words(1);
    }
    for branch in refs::parse_branches(&text) {
        let _ = branch.row();
    }
    for stash in refs::parse_stashes(data) {
        let _ = stash.row();
    }
    for line in text.split(['\r', '\n']) {
        if let Some(step) = progress::read(line) {
            let _ = progress::words("Cloning", &step);
        }
    }
}
