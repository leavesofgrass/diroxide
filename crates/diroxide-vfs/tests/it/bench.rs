//! The 100,000-entry listing, timed (research/alpha1-plan.md spike five;
//! R8's starting budgets: 800 ms on Windows, 300 ms on Linux, under 100 MB
//! held). No gate yet: W1e's bench takes the number. Run with
//! `cargo nextest run -p diroxide-vfs --run-ignored only --no-capture`.

use std::fs::File;
use std::mem::size_of;
use std::time::{Duration, Instant};

use diroxide_core::Entry;
use diroxide_vfs::{LocalProvider, OpContext, Provider};

use crate::local::place_of;
use crate::scratch;

const ENTRIES: usize = 100_000;

/// The heap an entry holds, roughly: the struct and its strings.
fn held(entry: &Entry) -> usize {
    size_of::<Entry>()
        + entry.name.capacity()
        + entry.owner.as_ref().map_or(0, String::capacity)
        + entry.group.as_ref().map_or(0, String::capacity)
}

#[test]
#[ignore = "a measurement of 100,000 files: run on request"]
fn listing_100000_entries() {
    let folder = scratch();
    let made = Instant::now();
    for n in 0..ENTRIES {
        File::create(folder.path().join(format!("entry-{n:06}.txt"))).unwrap();
    }
    let made = made.elapsed();
    let provider = LocalProvider::new();
    let place = place_of(&folder);
    let ctx = OpContext::new();
    // One listing to warm the system's caches, then the best of three.
    let warm = provider.list(&place, &ctx).unwrap();
    assert_eq!(warm.len(), ENTRIES);
    let mut best = Duration::MAX;
    let mut entries = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        entries = provider.list(&place, &ctx).unwrap();
        best = best.min(started.elapsed());
    }
    assert_eq!(entries.len(), ENTRIES);
    let bytes: usize = entries.iter().map(held).sum();
    eprintln!(
        "Listing 100,000 entries: {} ms, best of 3 warm runs; about {} MB held; made in {} s",
        best.as_millis(),
        bytes / 1_000_000,
        made.as_secs()
    );
}
