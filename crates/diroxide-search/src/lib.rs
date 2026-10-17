//! Search, dirsize and duplicates.
//!
//! A search is a [`SearchQuery`]: an expression in the one filter language
//! (`diroxide_core::filter`, the same text the panel filter and select by
//! pattern take) and a [`Scope`], the part the expression never holds
//! (where to start, hidden entries, ignore files, links, other drives,
//! archives, depth). [`run_search`] walks the scope, decides each entry
//! with the filter's evaluator, cheap facts first and file contents last,
//! and hands each hit to a [`SearchSink`] as it is found, in arrival
//! order. A [`SearchControl`] stops, holds and resumes it from any thread.
//! The [`SearchSummary`] says in words what was found and everything that
//! was skipped, so nothing is filtered silently.
//!
//! Dirsize walks with filters off. Duplicates go by size, then a fast
//! hash, then a full comparison (alpha.8).

mod engine;
mod hit;
mod local_entry;
mod query;
mod sink;
mod summary;
mod walk;

use diroxide_core::format::{count, human_size};
use diroxide_core::{EntryKind, Explain, ListModel, Place, SystemClock};
use diroxide_vfs::{OpContext, Registry, VfsError};
use thiserror::Error;

pub use engine::run_search;
pub use hit::{ContentMatch, SearchHit};
pub use query::{
    ContentPattern, DEFAULT_ROW_CAP, NamePattern, OnlineOnly, RowShape, SNAPSHOT_FOLDERS, Scope,
    SearchQuery, quoted,
};
pub use sink::{FnSink, SearchControl, SearchProgress, SearchSink};
pub use summary::{SearchSummary, Skips};

/// The total size of a folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirSize {
    /// Bytes in files.
    pub bytes: u64,
    /// Files counted.
    pub files: u64,
    /// Folders counted, not counting the root.
    pub folders: u64,
}

impl DirSize {
    /// In words: "2.1 MB in 40 files and 3 folders".
    pub fn describe(&self) -> String {
        format!(
            "{} in {} and {}",
            human_size(self.bytes),
            count(self.files, "file", "files"),
            count(self.folders, "folder", "folders")
        )
    }
}

/// Why a search could not run.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SearchError {
    /// A provider failed at the root.
    #[error(transparent)]
    Vfs(#[from] VfsError),
    /// This kind of search is not built yet.
    #[error("{0} is not built yet")]
    NotBuilt(String),
    /// The scope names no place to search.
    #[error("there is no place to search")]
    NoPlaces,
}

impl Explain for SearchError {
    fn next_step(&self) -> String {
        match self {
            SearchError::Vfs(inner) => inner.next_step(),
            SearchError::NotBuilt(_) => String::new(),
            SearchError::NoPlaces => "Choose a folder to search in.".to_owned(),
        }
    }
}

/// Whether a name matches a comma-separated list of globs, ignoring case.
pub fn glob_matches(globs: &str, name: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    globs
        .split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .any(|glob| {
            let glob: Vec<char> = glob.to_lowercase().chars().collect();
            wildcard(&glob, &name)
        })
}

fn wildcard(glob: &[char], name: &[char]) -> bool {
    let (mut g, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if g < glob.len() && (glob[g] == '?' || glob[g] == name[n]) {
            g += 1;
            n += 1;
        } else if g < glob.len() && glob[g] == '*' {
            star = Some((g, n));
            g += 1;
        } else if let Some((star_g, star_n)) = star {
            g = star_g + 1;
            n = star_n + 1;
            star = Some((star_g, star_n + 1));
        } else {
            return false;
        }
    }
    glob[g..].iter().all(|&c| c == '*')
}

/// Searches, sending each hit to `found` as it is found (Phase 0's form):
/// the system clock in UTC reads the date words, and `ctx`'s cancel token
/// stops it.
pub fn search(
    query: &SearchQuery,
    registry: &Registry,
    ctx: &OpContext,
    found: &mut dyn FnMut(SearchHit),
) -> Result<SearchSummary, SearchError> {
    let control = SearchControl::new().with_cancel(ctx.cancel.clone());
    run_search(
        query,
        registry,
        &SystemClock::utc(),
        &control,
        &mut FnSink(found),
    )
}

/// Adds up the size of a folder. Phase 0: through the provider, without
/// following links.
pub fn dirsize(
    place: &Place,
    registry: &Registry,
    ctx: &OpContext,
) -> Result<DirSize, SearchError> {
    let mut size = DirSize::default();
    let query = SearchQuery::everything(place.clone()).with_scope(
        Scope::new(place.clone())
            .with_hidden(true)
            .with_row_cap(None),
    );
    search(&query, registry, ctx, &mut |hit| match hit.entry.kind {
        EntryKind::Folder => size.folders += 1,
        _ => {
            size.files += 1;
            size.bytes += hit.entry.size.unwrap_or(0);
        }
    })?;
    Ok(size)
}

/// The results as a list: each row the name, then what the hit is, then
/// the folder it is in, in full.
pub fn results_list(title: &str, hits: &[SearchHit]) -> ListModel {
    results_list_under(title, None, hits)
}

/// The results as a list, each row's folder said from the search's start
/// ("notes.txt, in Documents/q3, 4 KB") when `root` is given.
pub fn results_list_under(title: &str, root: Option<&Place>, hits: &[SearchHit]) -> ListModel {
    let rows = hits.iter().map(|hit| hit.row(root)).collect();
    let mut list = ListModel::new(title).with_rows(rows);
    list.set_empty_text("Nothing found");
    list
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers for the unit tests that write.

    use std::path::Path;

    /// A folder for a unit test that writes, under the build's own target
    /// folder (the test binary lives in `target/<profile>/deps`), never a
    /// user's folder or the system's temporary folder.
    pub(crate) fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().expect("the test's own path");
        let base = exe
            .parent()
            .and_then(Path::parent)
            .expect("the target folder")
            .join("search-unit-tmp");
        std::fs::create_dir_all(&base).expect("a scratch folder");
        tempfile::Builder::new()
            .prefix("search-unit-")
            .tempdir_in(base)
            .expect("a scratch folder")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::filter::Unknown;

    #[test]
    fn globs_match_like_a_shell_ignoring_case() {
        assert!(glob_matches("*.rs", "lib.RS"));
        assert!(glob_matches("*.txt, *.rs", "main.rs"));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("*a*b*", "xxaxxbxx"));
        assert!(!glob_matches("*.rs", "lib.rs.bak"));
    }

    #[test]
    fn a_name_search_walks_this_crate() {
        let registry = Registry::with_local();
        let root = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let mut hits = Vec::new();
        let summary = search(
            &SearchQuery::names(root, "*.rs"),
            &registry,
            &OpContext::new(),
            &mut |hit| hits.push(hit),
        )
        .unwrap();
        assert!(hits.iter().any(|h| h.entry.name == "lib.rs"));
        assert!(summary.folders >= 2);
        let list = results_list("Search results", &hits);
        assert!(list.announce_focus().contains(".rs, in "));
    }

    #[test]
    fn content_is_left_undecided_until_it_is_read() {
        let root = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let query = SearchQuery::names(root, "lib.rs")
            .with_content(&ContentPattern::literal("x", false))
            .unwrap();
        let mut hits = 0;
        let summary = search(
            &query,
            &Registry::with_local(),
            &OpContext::new(),
            &mut |_| hits += 1,
        )
        .unwrap();
        assert_eq!(hits, 0);
        assert!(
            summary
                .unknown
                .iter()
                .any(|(why, _)| *why == Unknown::ContentNotRead),
            "{summary:?}"
        );
    }

    #[test]
    fn summaries_in_words() {
        let size = DirSize {
            bytes: 2_100_000,
            files: 40,
            folders: 3,
        };
        assert_eq!(size.describe(), "2.1 MB in 40 files and 3 folders");
        let summary = SearchSummary {
            hits: 12,
            folders: 40,
            unreadable: 1,
            cancelled: false,
            ..SearchSummary::default()
        };
        assert_eq!(
            summary.describe(),
            "12 found in 40 folders, 1 folder unreadable"
        );
    }

    #[test]
    fn a_search_needs_a_place() {
        let query = SearchQuery::everything(Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap())
            .with_scope(Scope::of(Vec::new()));
        let error = search(
            &query,
            &Registry::with_local(),
            &OpContext::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(error, SearchError::NoPlaces);
        assert_eq!(error.next_step(), "Choose a folder to search in.");
    }
}
