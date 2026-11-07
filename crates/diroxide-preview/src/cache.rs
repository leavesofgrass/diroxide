//! The previews' cache (alpha 4, R30 commit 40; R28 part 2.2's caps):
//! in memory only (decision 6: nothing is cached on disk in this alpha),
//! at most [`MAX_ENTRIES`] previews or [`MAX_BYTES`] of their words, the
//! least recently used dropped first. A preview is keyed by its place,
//! its size and modified time as listed, the previewer that made it and
//! that previewer's version, and the most lines asked for; so a changed
//! file, or a previewer that now says something else, is never answered
//! from the cache. A file with neither a size nor a time is not cached,
//! and neither is a preview that says a tool is missing, which the owner
//! may install.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use diroxide_core::{Place, Timestamp};

use crate::Preview;

/// The most previews kept.
pub const MAX_ENTRIES: usize = 256;

/// The most bytes of words kept, all previews together (32 MiB).
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// What a cached preview is found by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// The entry's place.
    pub place: Place,
    /// Its size as listed.
    pub size: Option<u64>,
    /// Its modified time as listed.
    pub modified: Option<Timestamp>,
    /// The previewer that made the preview.
    pub previewer: String,
    /// That previewer's version.
    pub version: u32,
    /// The most lines asked for.
    pub max_lines: usize,
}

#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<CacheKey, (Preview, usize, u64)>,
    bytes: usize,
    tick: u64,
}

/// The cache. See the module documentation. Shared between threads.
#[derive(Debug)]
pub struct PreviewCache {
    inner: Mutex<Inner>,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self::new(MAX_ENTRIES, MAX_BYTES)
    }
}

/// About how many bytes a preview's words take.
pub fn weight(preview: &Preview) -> usize {
    let lines: usize = preview.lines.iter().map(|l| l.len() + 24).sum();
    let details: usize = preview
        .details
        .iter()
        .map(|r| r.key.len() + r.label.len() + r.value.len() + 80)
        .sum();
    let document: usize = preview.document.as_ref().map_or(0, |view| {
        view.rows().iter().map(|r| r.text.len() + 64).sum::<usize>()
            + view
                .tables()
                .iter()
                .flat_map(|t| &t.cells)
                .flatten()
                .map(|c| c.len() + 24)
                .sum::<usize>()
    });
    let runs: usize = preview.runs.iter().map(|r| r.len() * 16 + 24).sum();
    preview.summary.len() + lines + details + document + runs + 128
}

impl PreviewCache {
    /// A cache of at most `max_entries` previews and `max_bytes` of
    /// words.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            max_entries: max_entries.max(1),
            max_bytes,
        }
    }

    /// Whether a preview of an entry with this key may be kept.
    pub fn keeps(key: &CacheKey, preview: &Preview) -> bool {
        (key.size.is_some() || key.modified.is_some())
            && !preview.summary.starts_with("No preview:")
    }

    /// The preview kept under a key, marked as used.
    pub fn get(&self, key: &CacheKey) -> Option<Preview> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.tick += 1;
        let tick = inner.tick;
        let (preview, _, used) = inner.entries.get_mut(key)?;
        *used = tick;
        Some(preview.clone())
    }

    /// Keeps a preview under a key, dropping the least recently used
    /// past the caps. A preview heavier than the whole cap is not kept.
    pub fn put(&self, key: CacheKey, preview: Preview) {
        if !Self::keeps(&key, &preview) {
            return;
        }
        let cost = weight(&preview);
        if cost > self.max_bytes {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.tick += 1;
        let tick = inner.tick;
        if let Some((_, old, _)) = inner.entries.insert(key, (preview, cost, tick)) {
            inner.bytes -= old;
        }
        inner.bytes += cost;
        while inner.entries.len() > self.max_entries || inner.bytes > self.max_bytes {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((_, cost, _)) = inner.entries.remove(&oldest) {
                inner.bytes -= cost;
            }
        }
    }

    /// How many previews are kept, and their weight.
    pub fn usage(&self) -> (usize, usize) {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        (inner.entries.len(), inner.bytes)
    }

    /// Drops every preview of a place, as when its file changed under a
    /// watch.
    pub fn forget(&self, place: &Place) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let gone: Vec<CacheKey> = inner
            .entries
            .keys()
            .filter(|k| &k.place == place)
            .cloned()
            .collect();
        for key in gone {
            if let Some((_, cost, _)) = inner.entries.remove(&key) {
                inner.bytes -= cost;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PreviewKind;

    fn key(n: u64) -> CacheKey {
        CacheKey {
            // A place every system parses: beside the crate.
            place: Place::parse(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .join(&format!("{n}.txt"))
                .unwrap(),
            size: Some(n),
            modified: None,
            previewer: "text".to_owned(),
            version: 1,
            max_lines: 10,
        }
    }

    fn preview(words: &str) -> Preview {
        let mut p = Preview::new(PreviewKind::Text, "text file, 4 bytes");
        p.lines = vec![words.to_owned()];
        p
    }

    #[test]
    fn the_least_recently_used_goes_first_past_the_entries_cap() {
        let cache = PreviewCache::new(3, MAX_BYTES);
        for n in 0..3 {
            cache.put(key(n), preview("x"));
        }
        assert!(cache.get(&key(0)).is_some(), "used, so kept");
        cache.put(key(3), preview("y"));
        assert_eq!(cache.usage().0, 3);
        assert!(cache.get(&key(1)).is_none(), "the oldest unused went");
        assert!(cache.get(&key(0)).is_some() && cache.get(&key(3)).is_some());
    }

    #[test]
    fn the_bytes_cap_holds_and_a_changed_key_misses() {
        let one = weight(&preview(&"z".repeat(1_000)));
        let cache = PreviewCache::new(MAX_ENTRIES, one * 2);
        for n in 0..5 {
            cache.put(key(n), preview(&"z".repeat(1_000)));
        }
        assert!(cache.usage().1 <= one * 2);
        assert_eq!(cache.usage().0, 2);
        let mut changed = key(4);
        changed.size = Some(99);
        assert!(cache.get(&changed).is_none());
        changed = key(4);
        changed.version = 2;
        assert!(cache.get(&changed).is_none());
        cache.forget(&key(4).place);
        assert!(cache.get(&key(4)).is_none());
    }

    #[test]
    fn an_unkeyed_entry_and_a_missing_tool_are_never_kept() {
        let cache = PreviewCache::default();
        let mut unkeyed = key(1);
        unkeyed.size = None;
        cache.put(unkeyed.clone(), preview("x"));
        assert!(cache.get(&unkeyed).is_none());
        cache.put(
            key(2),
            Preview::new(PreviewKind::Media, "No preview: ffprobe not found"),
        );
        assert!(cache.get(&key(2)).is_none());
        assert_eq!(cache.usage(), (0, 0));
    }
}
