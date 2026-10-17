//! The search spike of alpha.3 (research/alpha3-plan.md, "Search speed on
//! a 1 GB tree" and "The index-free design on network shares"): times
//! the walkers and the search on a tree written by
//! `cargo xtask search-tree PATH`, compares them with `rg` when it is on
//! PATH, and lists a slow place at 1, 2, 4 and 8 threads.
//!
//! ```text
//! cargo run --release -p diroxide-search --example spike -- TREE [--delay-ms N] [--runs N] [--json FILE]
//! ```
//!
//! The slow place is the local provider with a delay before every call:
//! a stand-in for a network share, which this machine may not reach. It
//! says so on its lines. The spike reads the tree and writes only the
//! JSON file it is given; it never changes the tree.

use std::collections::VecDeque;
use std::error::Error;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use diroxide_core::{Entry, EntryKind, Place, SystemClock};
use diroxide_search::{
    SearchControl, SearchHit, SearchProgress, SearchQuery, SearchSink, results_list_under,
    run_search,
};
use diroxide_vfs::{
    AttributeChange, CancelToken, Capabilities, FileWriter, LocalProvider, OpContext, Provider,
    Registry, VfsError, VolumeInfo, WriteMode,
};

type Outcome<T> = Result<T, Box<dyn Error>>;

/// The local provider with a delay before every call: a stand-in for a
/// network share.
#[derive(Debug)]
struct SlowPlace {
    inner: LocalProvider,
    delay: Duration,
    calls: AtomicU64,
}

impl SlowPlace {
    fn wait(&self) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(self.delay);
    }
}

impl Provider for SlowPlace {
    fn scheme(&self) -> &str {
        self.inner.scheme()
    }

    fn capabilities(&self, place: &Place) -> Capabilities {
        self.inner.capabilities(place)
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.wait();
        self.inner.list(place, ctx)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.wait();
        self.inner.stat(place, ctx)
    }

    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.wait();
        self.inner.read(place, ctx)
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.inner.write(place, mode, ctx)
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.create_dir(place, ctx)
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.rename(from, to, ctx)
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.inner.remove(place, ctx)
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.inner.set_attributes(place, change, ctx)
    }

    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        self.inner.volume(place, ctx)
    }
}

/// One measured line: the scenario, its median in milliseconds, and what
/// it saw.
struct Line {
    id: String,
    median_ms: f64,
    note: String,
}

fn median(mut runs: Vec<f64>) -> f64 {
    runs.sort_by(f64::total_cmp);
    match runs.len() {
        0 => 0.0,
        n if n % 2 == 1 => runs[n / 2],
        n => (runs[n / 2 - 1] + runs[n / 2]) / 2.0,
    }
}

fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

fn rate(items: u64, ms: f64) -> u64 {
    if ms <= 0.0 {
        0
    } else {
        (items as f64 * 1000.0 / ms) as u64
    }
}

/// The `ignore` crate's parallel walk with every standard filter off:
/// the ceiling of a local walk, counting entries.
fn walk_ignore(tree: &Path) -> u64 {
    let count = Arc::new(AtomicU64::new(0));
    let walker = ignore::WalkBuilder::new(tree)
        .standard_filters(false)
        .build_parallel();
    walker.run(|| {
        let count = Arc::clone(&count);
        Box::new(move |entry| {
            if entry.is_ok() {
                count.fetch_add(1, Ordering::Relaxed);
            }
            ignore::WalkState::Continue
        })
    });
    count.load(Ordering::Relaxed).saturating_sub(1)
}

/// A walk of folders through a provider with `threads` workers sharing
/// one queue: what a network place's walk costs at each thread count.
/// Returns the entries seen and, when `stop_after` is given, how long the
/// workers took to stop after the cancel.
fn walk_provider(
    provider: &dyn Provider,
    root: &Place,
    threads: usize,
    stop_after: Option<Duration>,
) -> (u64, Option<Duration>) {
    struct Queue {
        folders: VecDeque<Place>,
        busy: usize,
    }
    let queue = Mutex::new(Queue {
        folders: VecDeque::from([root.clone()]),
        busy: 0,
    });
    let ready = Condvar::new();
    let seen = AtomicU64::new(0);
    let cancel = CancelToken::new();
    let ctx = OpContext::new().with_cancel(cancel.clone());
    let finished = AtomicUsize::new(0);
    let cancelled_at = Mutex::new(None::<Instant>);
    let stopped = Mutex::new(None::<Duration>);
    std::thread::scope(|scope| {
        if let Some(after) = stop_after {
            let (cancel, ready, cancelled_at) = (&cancel, &ready, &cancelled_at);
            scope.spawn(move || {
                std::thread::sleep(after);
                *cancelled_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
                cancel.cancel();
                ready.notify_all();
            });
        }
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let folder = {
                        let mut q = queue.lock().unwrap_or_else(PoisonError::into_inner);
                        loop {
                            if cancel.is_cancelled() {
                                break None;
                            }
                            if let Some(folder) = q.folders.pop_front() {
                                q.busy += 1;
                                break Some(folder);
                            }
                            if q.busy == 0 {
                                break None;
                            }
                            q = ready
                                .wait_timeout(q, Duration::from_millis(20))
                                .unwrap_or_else(PoisonError::into_inner)
                                .0;
                        }
                    };
                    let Some(folder) = folder else { break };
                    let entries = provider.list(&folder, &ctx).unwrap_or_default();
                    seen.fetch_add(entries.len() as u64, Ordering::Relaxed);
                    let mut q = queue.lock().unwrap_or_else(PoisonError::into_inner);
                    for entry in entries {
                        if entry.kind == EntryKind::Folder
                            && let Ok(place) = folder.join(&entry.name)
                        {
                            q.folders.push_back(place);
                        }
                    }
                    q.busy -= 1;
                    ready.notify_all();
                }
                if finished.fetch_add(1, Ordering::SeqCst) + 1 == threads {
                    let at = *cancelled_at.lock().unwrap_or_else(PoisonError::into_inner);
                    *stopped.lock().unwrap_or_else(PoisonError::into_inner) =
                        at.map(|at| at.elapsed());
                }
            });
        }
    });
    let stopped = *stopped.lock().unwrap_or_else(PoisonError::into_inner);
    (seen.load(Ordering::Relaxed), stopped)
}

/// A sink that notes when the first hit arrived and counts the rest.
struct Clocked {
    started: Instant,
    first: Option<Duration>,
    hits: u64,
}

impl SearchSink for Clocked {
    fn found(&mut self, _hit: SearchHit) -> std::ops::ControlFlow<()> {
        self.first.get_or_insert_with(|| self.started.elapsed());
        self.hits += 1;
        std::ops::ControlFlow::Continue(())
    }

    fn progress(&mut self, _progress: &SearchProgress) {}
}

/// A search through diroxide's own engine: its time, its hits and when the
/// first hit arrived.
fn search_once(registry: &Registry, root: &Place, expression: &str) -> Outcome<(f64, u64, f64)> {
    let query = SearchQuery::parse(root.clone(), expression)?;
    let started = Instant::now();
    let mut sink = Clocked {
        started,
        first: None,
        hits: 0,
    };
    run_search(
        &query,
        registry,
        &SystemClock::utc(),
        &SearchControl::new(),
        &mut sink,
    )?;
    let total = millis(started.elapsed());
    let first = sink.first.map_or(total, millis);
    Ok((total, sink.hits, first))
}

/// `rg` with every filter that hides files lifted (`-uu`), timed, or
/// `None` when it is not on PATH.
fn rg(tree: &Path, args: &[&str]) -> Option<(f64, u64)> {
    let started = Instant::now();
    let output = Command::new("rg")
        .args(args)
        .arg(tree)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let ms = millis(started.elapsed());
    let lines = output.stdout.iter().filter(|b| **b == b'\n').count() as u64;
    Some((ms, lines))
}

fn place_of(path: &Path) -> Outcome<Place> {
    Ok(Place::parse(&path.to_string_lossy())?)
}

struct Args {
    tree: PathBuf,
    delay: Duration,
    runs: usize,
    json: Option<PathBuf>,
}

fn args() -> Outcome<Args> {
    let mut list = std::env::args().skip(1);
    let mut tree = None;
    let mut delay = Duration::from_millis(5);
    let mut runs = 3;
    let mut json = None;
    while let Some(arg) = list.next() {
        match arg.as_str() {
            "--delay-ms" => {
                delay = Duration::from_millis(list.next().ok_or("--delay-ms needs N")?.parse()?);
            }
            "--runs" => runs = list.next().ok_or("--runs needs N")?.parse()?,
            "--json" => json = Some(PathBuf::from(list.next().ok_or("--json needs a file")?)),
            other if tree.is_none() => tree = Some(PathBuf::from(other)),
            other => return Err(format!("{other} is not an argument of the spike").into()),
        }
    }
    let tree = tree.ok_or(
        "Usage: spike TREE [--delay-ms N] [--runs N] [--json FILE]; write TREE with cargo xtask search-tree",
    )?;
    Ok(Args {
        tree,
        delay,
        runs: runs.max(1),
        json,
    })
}

fn main() -> Outcome<()> {
    let args = args()?;
    let tree = args.tree.as_path();
    let root = place_of(tree)?;
    let registry = Registry::with_local();
    let mut lines = Vec::new();
    let optimized = !cfg!(debug_assertions);
    println!(
        "Search spike on {}: {} build, {} runs each, warm.",
        tree.display(),
        if optimized { "optimized" } else { "debug" },
        args.runs
    );
    // Warm the file system's cache once.
    let entries = walk_ignore(tree);

    let mut runs = Vec::new();
    for _ in 0..args.runs {
        let started = Instant::now();
        walk_ignore(tree);
        runs.push(millis(started.elapsed()));
    }
    let ms = median(runs);
    lines.push(Line {
        id: "walk-ignore".into(),
        median_ms: ms,
        note: format!(
            "{entries} entries, {} a second, ignore's parallel walk, filters off",
            rate(entries, ms)
        ),
    });

    let local = LocalProvider::new();
    let mut runs = Vec::new();
    let mut seen = 0;
    for _ in 0..args.runs {
        let started = Instant::now();
        seen = walk_provider(&local, &root, 1, None).0;
        runs.push(millis(started.elapsed()));
    }
    let ms = median(runs);
    lines.push(Line {
        id: "walk-provider".into(),
        median_ms: ms,
        note: format!(
            "{seen} entries, {} a second, the local provider, one thread",
            rate(seen, ms)
        ),
    });

    for (id, expression) in [
        ("search-name", "name *quokkaline*"),
        (
            "search-attributes",
            "ext txt and size over 1 KB and modified within 3 years",
        ),
    ] {
        let mut runs = Vec::new();
        let mut firsts = Vec::new();
        let mut hits = 0;
        for _ in 0..args.runs {
            let (ms, found, first) = search_once(&registry, &root, expression)?;
            runs.push(ms);
            firsts.push(first);
            hits = found;
        }
        lines.push(Line {
            id: id.into(),
            median_ms: median(runs),
            note: format!(
                "{hits} found for {expression}, first hit at {:.2} ms",
                median(firsts)
            ),
        });
    }

    for (id, rg_args) in [
        ("rg-files", vec!["--files", "-uu"]),
        ("rg-content", vec!["-uu", "-l", "-F", "quokkaline"]),
        (
            "rg-content-case",
            vec!["-uu", "-l", "-i", "-F", "MARMOTFIELD"],
        ),
    ] {
        let mut runs = Vec::new();
        let mut found = None;
        for _ in 0..args.runs {
            match rg(tree, &rg_args) {
                Some((ms, n)) => {
                    runs.push(ms);
                    found = Some(n);
                }
                None => break,
            }
        }
        lines.push(match found {
            Some(n) => Line {
                id: id.into(),
                median_ms: median(runs),
                note: format!("{n} lines from rg {}", rg_args.join(" ")),
            },
            None => Line {
                id: id.into(),
                median_ms: 0.0,
                note: "rg is not on PATH: not compared".into(),
            },
        });
    }

    let slow = SlowPlace {
        inner: LocalProvider::new(),
        delay: args.delay,
        calls: AtomicU64::new(0),
    };
    for threads in [1, 2, 4, 8] {
        slow.calls.store(0, Ordering::Relaxed);
        let started = Instant::now();
        let (seen, _) = walk_provider(&slow, &root, threads, None);
        let ms = millis(started.elapsed());
        lines.push(Line {
            id: format!("slow-place-{threads}"),
            median_ms: ms,
            note: format!(
                "{} items a second at {threads} threads, {} ms before every call, {} calls; a fake network place",
                rate(seen, ms),
                args.delay.as_millis(),
                slow.calls.load(Ordering::Relaxed)
            ),
        });
    }
    let (_, stopped) = walk_provider(&slow, &root, 2, Some(Duration::from_millis(300)));
    lines.push(Line {
        id: "slow-place-stop".into(),
        median_ms: stopped.map_or(0.0, millis),
        note: "from the cancel to the last thread stopped, 2 threads, a fake network place".into(),
    });

    for rows in [100_000usize, 1_000_000] {
        let hits: Vec<SearchHit> = (0..rows)
            .filter_map(|n| {
                let name = format!("file-{n:07}.txt");
                let place = root.join("sub").ok()?.join(&name).ok()?;
                Some(SearchHit::new(
                    place,
                    Entry::new(name, EntryKind::File).with_size(4_000),
                ))
            })
            .collect();
        let started = Instant::now();
        let list = results_list_under("Search results", Some(&root), &hits);
        let ms = millis(started.elapsed());
        lines.push(Line {
            id: format!("rows-{rows}"),
            median_ms: ms,
            note: format!("{} rows made into a results list", list.len()),
        });
    }

    let mut json = String::from("{\n  \"optimized\": ");
    json.push_str(if optimized { "true" } else { "false" });
    json.push_str(",\n  \"lines\": [\n");
    for (index, line) in lines.iter().enumerate() {
        println!("{}: {:.1} ms. {}.", line.id, line.median_ms, line.note);
        json.push_str(&format!(
            "    {{\"id\": \"{}\", \"median_ms\": {:.1}, \"note\": \"{}\"}}{}\n",
            line.id,
            line.median_ms,
            line.note.replace('\\', "\\\\").replace('"', "\\\""),
            if index + 1 == lines.len() { "" } else { "," }
        ));
    }
    json.push_str("  ]\n}\n");
    if let Some(path) = &args.json {
        std::fs::write(path, json)?;
        println!("Wrote {}.", path.display());
    }
    Ok(())
}
