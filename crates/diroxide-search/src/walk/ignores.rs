//! Ignore files for the provider walk: `.ignore` and `.gitignore` inside
//! the search, read through the provider and matched with the `ignore`
//! crate's own gitignore matcher, with the precedence its parallel walker
//! uses: for each kind of file the nearest folder with a match decides,
//! and `.ignore` wins over `.gitignore`. Files above the start, git's
//! exclude file and the global ignore file are not read, by either
//! walker, and `.gitignore` applies outside a git repository too.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use diroxide_core::{Entry, Place};
use diroxide_vfs::{OpContext, Provider};
use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// The ignore files' names, in the order they win.
pub(crate) const IGNORE_FILES: [&str; 2] = [".ignore", ".gitignore"];

/// The most of an ignore file read: 1 MiB.
const MOST: u64 = 1024 * 1024;

/// The ignore files of one folder, and its parent's.
#[derive(Debug)]
pub(crate) struct IgnoreLevel {
    parent: Option<Arc<IgnoreLevel>>,
    /// How many names below the start this folder is.
    depth: usize,
    /// `.ignore`, then `.gitignore`.
    matchers: [Option<Gitignore>; 2],
}

impl IgnoreLevel {
    /// The level of a folder whose listing is `entries`: its ignore files
    /// read through `provider`, or the parent's level when it has none.
    pub(crate) fn read(
        parent: Option<Arc<IgnoreLevel>>,
        depth: usize,
        folder: &Place,
        entries: &[Entry],
        provider: &dyn Provider,
        ctx: &OpContext,
    ) -> Option<Arc<IgnoreLevel>> {
        let mut matchers = [None, None];
        for (slot, name) in matchers.iter_mut().zip(IGNORE_FILES) {
            if !entries.iter().any(|e| e.name == name) {
                continue;
            }
            let Ok(place) = folder.join(name) else {
                continue;
            };
            let Ok(reader) = provider.read(&place, ctx) else {
                continue;
            };
            let mut bytes = Vec::new();
            if reader.take(MOST).read_to_end(&mut bytes).is_err() {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let mut builder = GitignoreBuilder::new("");
            for line in text.lines() {
                // A line the matcher cannot read is left out, as ripgrep
                // leaves it out with a warning.
                let _ = builder.add_line(None, line);
            }
            if let Ok(matcher) = builder.build()
                && !matcher.is_empty()
            {
                *slot = Some(matcher);
            }
        }
        if matchers.iter().all(Option::is_none) {
            return parent;
        }
        Some(Arc::new(IgnoreLevel {
            parent,
            depth,
            matchers,
        }))
    }
}

/// Whether an entry is ignored: `names` is its path below the start, one
/// name each.
pub(crate) fn is_ignored(level: Option<&IgnoreLevel>, names: &[String], is_dir: bool) -> bool {
    let mut found = [Match::None, Match::None];
    let mut current = level;
    while let Some(level) = current {
        let below: PathBuf = names.iter().skip(level.depth).collect();
        for (kind, matcher) in found.iter_mut().zip(&level.matchers) {
            if kind.is_none()
                && let Some(matcher) = matcher
            {
                *kind = matcher.matched(&below, is_dir).map(|_| ());
            }
        }
        current = level.parent.as_deref();
    }
    let [ignore, git] = found;
    ignore.or(git).is_ignore()
}
