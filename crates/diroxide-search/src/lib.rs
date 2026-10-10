//! Search, dirsize and duplicates.
//!
//! Search walks a place and reports hits as they are found into a results
//! list that every command can act on. Names match a glob or a regex;
//! contents match a regex. Dirsize walks with the same walker. Duplicates
//! go by size, then a fast hash, then a full comparison.
//!
//! Phase 0: a naive walk through the provider, glob names with `*` and `?`,
//! no content search, no duplicates. Wave 2 (W2b) owns this crate and
//! brings in ripgrep's crates (`ignore`, `globset`, `grep-regex`,
//! `grep-searcher`) and `blake3`.

use diroxide_core::format::{count, human_size};
use diroxide_core::{Entry, Explain, ListModel, ListRow, Place, Timestamp};
use diroxide_vfs::{OpContext, Registry, VfsError};
use thiserror::Error;

/// How names are matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamePattern {
    /// A glob: `*` any run of characters, `?` one character; several globs
    /// separated by commas.
    Glob(String),
    /// A regular expression in Rust's `regex` syntax.
    Regex(String),
}

/// How contents are matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentPattern {
    /// The regular expression.
    pub regex: String,
    /// Whether case matters.
    pub case_sensitive: bool,
    /// Stop reading a file after this many bytes.
    pub max_bytes: Option<u64>,
}

/// What to look for and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    /// Where to start.
    pub root: Place,
    /// Names to match; `None` matches every name.
    pub name: Option<NamePattern>,
    /// Contents to match.
    pub content: Option<ContentPattern>,
    /// The smallest size, in bytes.
    pub min_size: Option<u64>,
    /// The largest size, in bytes.
    pub max_size: Option<u64>,
    /// Modified at or after.
    pub modified_after: Option<Timestamp>,
    /// Modified before.
    pub modified_before: Option<Timestamp>,
    /// How many folders deep to go; `None` for no limit.
    pub max_depth: Option<usize>,
    /// Include hidden entries.
    pub include_hidden: bool,
    /// Follow links into folders (never out of the root).
    pub follow_links: bool,
    /// Skip what `.gitignore` files ignore.
    pub respect_ignore_files: bool,
}

impl SearchQuery {
    /// A query for names matching a glob under a root, with the defaults:
    /// any depth, hidden entries skipped, links not followed.
    pub fn names(root: Place, glob: &str) -> Self {
        Self {
            root,
            name: Some(NamePattern::Glob(glob.to_owned())),
            content: None,
            min_size: None,
            max_size: None,
            modified_after: None,
            modified_before: None,
            max_depth: None,
            include_hidden: false,
            follow_links: false,
            respect_ignore_files: false,
        }
    }
}

/// A matching line in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentMatch {
    /// The line number, from one.
    pub line_number: u64,
    /// The line's text.
    pub text: String,
}

/// One result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// Where it is.
    pub place: Place,
    /// What it is.
    pub entry: Entry,
    /// The first matching line, for a content search.
    pub line: Option<ContentMatch>,
}

/// How a search went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchSummary {
    /// Hits found.
    pub hits: u64,
    /// Folders read.
    pub folders: u64,
    /// Folders that could not be read.
    pub unreadable: u64,
    /// Whether it was cancelled.
    pub cancelled: bool,
}

impl SearchSummary {
    /// The summary in words: "12 found in 40 folders, 1 folder unreadable".
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} found in {}",
            self.hits,
            count(self.folders, "folder", "folders")
        );
        if self.unreadable > 0 {
            text.push_str(&format!(
                ", {} unreadable",
                count(self.unreadable, "folder", "folders")
            ));
        }
        if self.cancelled {
            text.push_str(", cancelled");
        }
        text
    }
}

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
}

impl Explain for SearchError {
    fn next_step(&self) -> String {
        match self {
            SearchError::Vfs(inner) => inner.next_step(),
            SearchError::NotBuilt(_) => String::new(),
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

/// Searches, sending each hit to `found` as it is found. Phase 0: names
/// only, through the provider, depth first.
pub fn search(
    query: &SearchQuery,
    registry: &Registry,
    ctx: &OpContext,
    found: &mut dyn FnMut(SearchHit),
) -> Result<SearchSummary, SearchError> {
    if query.content.is_some() {
        return Err(SearchError::NotBuilt("content search".to_owned()));
    }
    if matches!(query.name, Some(NamePattern::Regex(_))) {
        return Err(SearchError::NotBuilt("regex name search".to_owned()));
    }
    let provider = registry.resolve(&query.root)?;
    let mut summary = SearchSummary::default();
    let mut pending = vec![(query.root.clone(), 0usize)];
    let mut first = true;
    while let Some((folder, depth)) = pending.pop() {
        if ctx.cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let entries = match provider.list(&folder, ctx) {
            Ok(entries) => entries,
            Err(error) if first => return Err(error.into()),
            Err(_) => {
                summary.unreadable += 1;
                continue;
            }
        };
        first = false;
        summary.folders += 1;
        for entry in entries {
            if entry.hidden && !query.include_hidden {
                continue;
            }
            let Ok(place) = folder.join(&entry.name) else {
                continue;
            };
            let name_ok = match &query.name {
                Some(NamePattern::Glob(globs)) => glob_matches(globs, &entry.name),
                _ => true,
            };
            let size_ok = query
                .min_size
                .is_none_or(|min| entry.size.unwrap_or(0) >= min)
                && query
                    .max_size
                    .is_none_or(|max| entry.size.unwrap_or(0) <= max);
            if entry.kind == diroxide_core::EntryKind::Folder
                && query.max_depth.is_none_or(|max| depth < max)
            {
                pending.push((place.clone(), depth + 1));
            }
            if name_ok && size_ok {
                summary.hits += 1;
                found(SearchHit {
                    place,
                    entry,
                    line: None,
                });
            }
        }
    }
    Ok(summary)
}

/// Adds up the size of a folder. Phase 0: through the provider, without
/// following links.
pub fn dirsize(
    place: &Place,
    registry: &Registry,
    ctx: &OpContext,
) -> Result<DirSize, SearchError> {
    let mut size = DirSize::default();
    let query = SearchQuery {
        include_hidden: true,
        ..SearchQuery::names(place.clone(), "*")
    };
    search(&query, registry, ctx, &mut |hit| match hit.entry.kind {
        diroxide_core::EntryKind::Folder => size.folders += 1,
        _ => {
            size.files += 1;
            size.bytes += hit.entry.size.unwrap_or(0);
        }
    })?;
    Ok(size)
}

/// The results as a list: each row the name, then the folder it is in.
pub fn results_list(title: &str, hits: &[SearchHit]) -> ListModel {
    let rows = hits
        .iter()
        .map(|hit| {
            let folder = hit
                .place
                .parent()
                .map(|p| p.to_string())
                .unwrap_or_default();
            ListRow::new(hit.place.to_string(), hit.entry.display_name())
                .with_details([format!("in {folder}")])
        })
        .collect();
    let mut list = ListModel::new(title).with_rows(rows);
    list.set_empty_text("Nothing found");
    list
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn content_search_is_not_built_yet() {
        let mut query = SearchQuery::names(Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap(), "*");
        query.content = Some(ContentPattern {
            regex: "x".into(),
            case_sensitive: false,
            max_bytes: None,
        });
        let err = search(
            &query,
            &Registry::with_local(),
            &OpContext::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "content search is not built yet");
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
        };
        assert_eq!(
            summary.describe(),
            "12 found in 40 folders, 1 folder unreadable"
        );
    }
}
