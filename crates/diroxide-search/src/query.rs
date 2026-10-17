//! What to look for and where: a [`SearchQuery`] is a filter in the one
//! filter language (`diroxide_core::filter`) and a [`Scope`], the part
//! the expression never holds (where to start, hidden entries, ignore
//! files, links, other drives, archives, depth).

use diroxide_core::filter::{Filter, FilterError, FilterOptions};
use diroxide_core::{CaseRule, Place};

/// The folder names a search never enters unless the scope says
/// otherwise: file system snapshots, which hold a copy of everything
/// around them.
pub const SNAPSHOT_FOLDERS: [&str; 2] = [".zfs", ".snapshots"];

/// The most rows a results list shows by default; the count keeps
/// counting past it ("Showing the first 100,000. Narrow the search.").
pub const DEFAULT_ROW_CAP: u64 = 100_000;

/// How names are matched (Phase 0). A search now reads names through the
/// filter language; this turns the two Phase 0 forms into its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamePattern {
    /// A glob: `*` any run of characters, `?` one character; several globs
    /// separated by commas.
    Glob(String),
    /// A regular expression in Rust's `regex` syntax.
    Regex(String),
}

impl NamePattern {
    /// The pattern in the filter language: a glob list as typed, each glob
    /// quoted so a word of the language reads as a name
    /// (`name matches "*.txt", "*.md"`), or `name regex "..."`.
    pub fn to_filter_text(&self) -> String {
        match self {
            NamePattern::Glob(globs) => {
                let parts: Vec<String> = globs
                    .split(',')
                    .map(str::trim)
                    .filter(|glob| !glob.is_empty())
                    .map(quoted)
                    .collect();
                if parts.is_empty() {
                    "name matches \"*\"".to_owned()
                } else {
                    format!("name matches {}", parts.join(", "))
                }
            }
            NamePattern::Regex(regex) => format!("name regex {}", quoted(regex)),
        }
    }
}

/// Contents to look for (Phase 0's shape), as the search dialog's
/// "contains" field gives them. A search reads contents through the
/// filter language's `content` test; this turns the pattern into its
/// text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentPattern {
    /// The regular expression.
    pub regex: String,
    /// Whether case matters.
    pub case_sensitive: bool,
    /// Stop reading a file after this many bytes.
    pub max_bytes: Option<u64>,
}

impl ContentPattern {
    /// A pattern for plain text, its special characters escaped.
    pub fn literal(text: &str, case_sensitive: bool) -> Self {
        Self {
            regex: regex::escape(text),
            case_sensitive,
            max_bytes: None,
        }
    }

    /// The pattern in the filter language: `case content regex "..."` or
    /// `anycase content regex "..."`, with `first N B` when a byte limit
    /// is set.
    pub fn to_filter_text(&self) -> String {
        let case = if self.case_sensitive {
            "case"
        } else {
            "anycase"
        };
        let mut text = format!("{case} content regex {}", quoted(&self.regex));
        if let Some(limit) = self.max_bytes {
            text.push_str(&format!(" first {limit} B"));
        }
        text
    }
}

/// Double quotes around text, a quote inside doubled: the filter
/// language's quoting, in which a backslash is never an escape.
pub fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// How content hits become rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RowShape {
    /// One row per file, with its first matching line and a count of the
    /// others: what "act on the results" means (copy these files).
    #[default]
    PerFile,
    /// One row per matching line, as ripgrep prints them.
    PerLine,
}

/// What a content search does with a file whose data is not on this
/// computer (a cloud placeholder): reading it would download it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OnlineOnly {
    /// Skip it and count it in the summary (the default).
    #[default]
    Skip,
    /// Read it, which downloads it. The app asks first.
    Read,
}

/// Where a search looks and how it walks: everything that is not in the
/// expression. The defaults are the plan's: hidden entries skipped (the
/// panel's setting decides), ignore files not read, links not followed,
/// one drive, archives not entered, any depth, snapshots skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Scope {
    /// Where to start: the panel's folder, or each marked entry.
    pub places: Vec<Place>,
    /// Whether hidden entries are searched (and hidden folders entered).
    pub hidden: bool,
    /// Whether `.gitignore` and `.ignore` files inside the search are
    /// read, and what they list skipped.
    pub ignore_files: bool,
    /// Whether links to folders are followed; never out of the place the
    /// search started in.
    pub follow_links: bool,
    /// Whether the search stays on the drive it started on.
    pub one_file_system: bool,
    /// Whether archives are searched inside.
    pub archives: bool,
    /// How many folders deep to go; direct children are 1.
    pub max_depth: Option<u32>,
    /// How many threads walk; `None` lets the search choose (two on a
    /// network drive).
    pub threads: Option<usize>,
    /// What a content search does with online-only files.
    pub online_only: OnlineOnly,
    /// Folder names never entered, snapshots by default.
    pub skip_names: Vec<String>,
    /// Places never entered: the entries labeled "Skip in searches".
    pub skip_places: Vec<Place>,
    /// The most hits delivered; the count keeps counting past it.
    pub row_cap: Option<u64>,
}

impl Scope {
    /// The default scope at one place.
    pub fn new(place: Place) -> Self {
        Self::of(vec![place])
    }

    /// The default scope over several places, such as the marked entries.
    pub fn of(places: Vec<Place>) -> Self {
        Self {
            places,
            hidden: false,
            ignore_files: false,
            follow_links: false,
            one_file_system: true,
            archives: false,
            max_depth: None,
            threads: None,
            online_only: OnlineOnly::Skip,
            skip_names: SNAPSHOT_FOLDERS.iter().map(|s| (*s).to_owned()).collect(),
            skip_places: Vec::new(),
            row_cap: Some(DEFAULT_ROW_CAP),
        }
    }

    /// The same scope, searching hidden entries or not.
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// The same scope, reading ignore files or not.
    pub fn with_ignore_files(mut self, read: bool) -> Self {
        self.ignore_files = read;
        self
    }

    /// The same scope, following links to folders or not.
    pub fn with_follow_links(mut self, follow: bool) -> Self {
        self.follow_links = follow;
        self
    }

    /// The same scope, staying on one drive or not.
    pub fn with_one_file_system(mut self, stay: bool) -> Self {
        self.one_file_system = stay;
        self
    }

    /// The same scope, searching inside archives or not.
    pub fn with_archives(mut self, archives: bool) -> Self {
        self.archives = archives;
        self
    }

    /// The same scope with a depth limit; direct children are 1.
    pub fn with_max_depth(mut self, depth: Option<u32>) -> Self {
        self.max_depth = depth;
        self
    }

    /// The same scope with a thread count; `None` lets the search choose.
    pub fn with_threads(mut self, threads: Option<usize>) -> Self {
        self.threads = threads;
        self
    }

    /// The same scope, reading online-only files or skipping them.
    pub fn with_online_only(mut self, online_only: OnlineOnly) -> Self {
        self.online_only = online_only;
        self
    }

    /// The same scope with other folder names never entered.
    pub fn with_skip_names(mut self, names: Vec<String>) -> Self {
        self.skip_names = names;
        self
    }

    /// The same scope with places never entered.
    pub fn with_skip_places(mut self, places: Vec<Place>) -> Self {
        self.skip_places = places;
        self
    }

    /// The same scope with another row cap; `None` delivers every hit.
    pub fn with_row_cap(mut self, cap: Option<u64>) -> Self {
        self.row_cap = cap;
        self
    }
}

/// What to look for and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    /// The expression, in the filter language; `None` finds every entry.
    pub filter: Option<Filter>,
    /// Where to look and how to walk.
    pub scope: Scope,
    /// One row per file or per matching line, for content tests.
    pub rows: RowShape,
}

impl SearchQuery {
    /// A query for names matching a glob under a root, with the defaults:
    /// any depth, hidden entries skipped, links not followed. Case is
    /// ignored, as in Phase 0. A glob list that cannot be read finds
    /// nothing rather than everything.
    pub fn names(root: Place, glob: &str) -> Self {
        let text = NamePattern::Glob(glob.to_owned()).to_filter_text();
        let options = FilterOptions {
            case: CaseRule::Ignore,
            ..FilterOptions::default()
        };
        let filter = Filter::parse_with(&text, &options)
            .or_else(|_| Filter::parse_with("name is \"\"", &options))
            .ok();
        Self {
            filter,
            scope: Scope::new(root),
            rows: RowShape::PerFile,
        }
    }

    /// A query for an expression in the filter language under a root,
    /// with the default scope. An empty expression finds every entry.
    pub fn parse(root: Place, expression: &str) -> Result<Self, FilterError> {
        Self::parse_with(root, expression, &FilterOptions::default())
    }

    /// [`SearchQuery::parse`] with the owner's filter settings.
    pub fn parse_with(
        root: Place,
        expression: &str,
        options: &FilterOptions,
    ) -> Result<Self, FilterError> {
        let filter = if expression.trim().is_empty() {
            None
        } else {
            Some(Filter::parse_with(expression, options)?)
        };
        Ok(Self {
            filter,
            scope: Scope::new(root),
            rows: RowShape::PerFile,
        })
    }

    /// Every entry under a root: no filter.
    pub fn everything(root: Place) -> Self {
        Self {
            filter: None,
            scope: Scope::new(root),
            rows: RowShape::PerFile,
        }
    }

    /// The same query with this scope.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// The same query with this row shape.
    pub fn with_rows(mut self, rows: RowShape) -> Self {
        self.rows = rows;
        self
    }

    /// The same query with a "contains" pattern added to the expression
    /// (the search dialog's second field): `(EXPRESSION) and content ...`.
    pub fn with_content(self, content: &ContentPattern) -> Result<Self, FilterError> {
        let options = self.options();
        let text = match &self.filter {
            Some(filter) => format!("({}) and {}", filter.text(), content.to_filter_text()),
            None => content.to_filter_text(),
        };
        Ok(Self {
            filter: Some(Filter::parse_with(&text, &options)?),
            ..self
        })
    }

    /// The filter settings the expression was read with.
    pub fn options(&self) -> FilterOptions {
        self.filter
            .as_ref()
            .map(|f| *f.options())
            .unwrap_or_default()
    }

    /// Whether a test reads files.
    pub fn reads_content(&self) -> bool {
        self.filter.as_ref().is_some_and(Filter::reads_content)
    }

    /// Whether hidden entries are searched: the scope says so, or the
    /// expression asks for them with a `hidden` test.
    pub fn searches_hidden(&self) -> bool {
        self.scope.hidden || self.filter.as_ref().is_some_and(Filter::shows_hidden)
    }

    /// How deep the walk goes: the scope's limit, or a depth test the
    /// whole expression depends on, whichever is less.
    pub fn depth_limit(&self) -> Option<u32> {
        let from_filter = self
            .filter
            .as_ref()
            .and_then(Filter::max_depth)
            .map(|d| u32::try_from(d).unwrap_or(u32::MAX));
        match (self.scope.max_depth, from_filter) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The expression as said, for the first line of a results list and
    /// `diro search --explain`: "name matches *.txt", or "every entry".
    pub fn sentence(&self) -> String {
        self.filter
            .as_ref()
            .map_or_else(|| "every entry".to_owned(), Filter::sentence)
    }

    /// The expression as typed, or `*` for every entry.
    pub fn typed(&self) -> &str {
        self.filter.as_ref().map_or("*", Filter::text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Place {
        Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap()
    }

    #[test]
    fn phase_0_patterns_read_in_the_filter_language() {
        assert_eq!(
            NamePattern::Glob("*.txt, *.md".into()).to_filter_text(),
            "name matches \"*.txt\", \"*.md\""
        );
        assert_eq!(
            NamePattern::Regex("^IMG_\\d{4}".into()).to_filter_text(),
            "name regex \"^IMG_\\d{4}\""
        );
        for pattern in [
            NamePattern::Glob("*.rs,and,\"odd\"".into()),
            NamePattern::Regex("a\"b".into()),
        ] {
            assert!(
                Filter::parse(&pattern.to_filter_text()).is_ok(),
                "{pattern:?}"
            );
        }
        let content = ContentPattern {
            max_bytes: Some(65_536),
            ..ContentPattern::literal("total (due)", false)
        };
        let filter = Filter::parse(&content.to_filter_text()).unwrap();
        assert!(filter.reads_content());
        assert!(filter.sentence().contains("total"), "{}", filter.sentence());
    }

    #[test]
    fn the_scope_defaults_are_the_plans() {
        let scope = Scope::new(root());
        assert!(!scope.hidden && !scope.ignore_files && !scope.follow_links);
        assert!(scope.one_file_system && !scope.archives);
        assert_eq!(scope.max_depth, None);
        assert_eq!(scope.online_only, OnlineOnly::Skip);
        assert_eq!(scope.row_cap, Some(100_000));
        assert!(scope.skip_names.iter().any(|n| n == ".snapshots"));
    }

    #[test]
    fn queries_read_and_combine() {
        let query = SearchQuery::parse(root(), "hidden and kind file").unwrap();
        assert!(query.searches_hidden());
        let query = SearchQuery::parse(root(), "depth at most 2 and name *.rs")
            .unwrap()
            .with_scope(Scope::new(root()).with_max_depth(Some(5)));
        assert_eq!(query.depth_limit(), Some(2));
        let query = SearchQuery::parse(root(), "").unwrap();
        assert_eq!(query.sentence(), "every entry");
        assert_eq!(query.typed(), "*");
        let query = SearchQuery::parse(root(), "ext txt or ext md")
            .unwrap()
            .with_content(&ContentPattern::literal("total due", false))
            .unwrap();
        assert!(query.reads_content());
        assert!(SearchQuery::parse(root(), "size over").is_err());
        let names = SearchQuery::names(root(), "*.RS");
        assert_eq!(names.options().case, CaseRule::Ignore);
    }
}
