//! `cargo xtask bench`: the bench of plan section 4, principle 8, on
//! trees it writes under the target folder and removes after the run.
//! Nothing it does touches a real folder, the trash or the network, and
//! nothing plays sound.
//!
//! The scenarios, each run `runs` times (three) with the median kept:
//!
//! - `list`: list a folder of 100,000 entries through the local provider.
//! - `stat`: read each of those entries' details by its place.
//! - `list-stat`: list the folder, then read each entry's details, as a
//!   panel that fills its columns late does (R23).
//! - `copy`: copy a 1 GB file through the job engine, planned and run.
//! - `copy-system`: the same file with the system's own copy call, for
//!   reference.
//! - `slow-place`: copy from W1b's slow place on the fake provider, every
//!   call slow and one call stalled, and check that the stall is said.
//! - `archive-zip`, `archive-tar`, `archive-7z`: list 100,000 members of a
//!   zip, a tar and a 7z through the archive provider, opened fresh each
//!   run, from the open to the last entry (alpha.2).
//! - `unpack`: unpack a zip of one 200 MB file through the unpack runner,
//!   planned and run, the rate said.
//! - `viewer`: open a 256 MB text log in the viewer and draw its first
//!   screen, in screen-reader mode.
//! - `search-name` and `search-content`: search the 1 GB tree of
//!   `bench/search-tree.toml` (its small form with `--quick`) by name and
//!   by contents through the search engine (alpha.3, W3s).
//!
//! The archive scenarios call the archive provider and its runners
//! directly (P2b, W2g's request 1): the archives are packed by diroxide's
//! own pack runner, and each timed run opens a new provider, so no run
//! reads another's cached index.
//!
//! The budgets are for an optimized build: `cargo run --release -p xtask
//! -- bench`. The medians go to `target/bench/result.json` and are said
//! in words.
//! `--save-baseline` writes them into `bench/baseline.json` for this
//! platform. The ratchet compares each median with the baseline and says
//! the difference; it is report-only in Wave 1, and `--gate` makes a
//! median outside the band in either direction fail, from Wave 2 on.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use diroxide_archive::{
    Allowances, ArchiveFormat, ArchiveProvider, JobEnv, Limits, PackRequest, UnpackRequest,
    plan_unpack, run_unpack,
};
use diroxide_core::format::human_size;
use diroxide_core::{
    ConflictAnswer, ErrorAction, JobId, JobKind, JobRequest, JobState, Place, SystemClock,
    Timestamp, UtcOffset,
};
use diroxide_ops::{
    Conflict, Engine, EngineOptions, ItemError, JobControl, JobProgress, JobSink, Latency, Plan,
    SlowPlace,
};
use diroxide_search::{FnSink, Scope, SearchControl, SearchHit, SearchQuery, quoted, run_search};
use diroxide_vfs::{OpContext, Provider, Registry};
use serde::{Deserialize, Serialize};

use crate::sample;

/// One size of the bench, from `bench/params.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Sizes {
    /// Entries in the listed folder.
    pub list_entries: usize,
    /// Bytes of the copied file.
    pub copy_bytes: u64,
    /// Files in the slow place.
    pub slow_files: usize,
    /// Bytes of each of them.
    pub slow_file_bytes: usize,
    /// The slow place's delay on every call, in milliseconds.
    pub slow_per_call_ms: u64,
    /// The call of the run, counted after the plan, that stalls.
    pub slow_stall_after_calls: u32,
    /// How long that call stalls, in milliseconds.
    pub slow_stall_ms: u64,
    /// The engine's stall detector, in milliseconds.
    pub stall_after_ms: u64,
    /// Members of the listed zip, tar and 7z.
    pub archive_entries: usize,
    /// Bytes of the one file in the unpacked zip.
    pub unpack_bytes: u64,
    /// Bytes of the log the viewer opens.
    pub viewer_bytes: u64,
}

/// The starting budgets, in milliseconds.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    /// Listing, on Windows.
    pub list_windows: u64,
    /// Listing, elsewhere.
    pub list_other: u64,
    /// Reading every entry's details, on Windows.
    pub stat_windows: u64,
    /// Reading every entry's details, elsewhere.
    pub stat_other: u64,
    /// A name search of the search tree, on Windows.
    pub search_name_windows: u64,
    /// A name search of the search tree, elsewhere.
    pub search_name_other: u64,
    /// A content search of the search tree, on Windows.
    pub search_content_windows: u64,
    /// A content search of the search tree, elsewhere.
    pub search_content_other: u64,
}

/// `bench/params.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// Runs of each scenario.
    pub runs: usize,
    /// The ratchet's band, in percent.
    pub tolerance_percent: u64,
    /// The starting budgets.
    pub budgets_ms: Budgets,
    /// The full sizes.
    pub full: Sizes,
    /// The sizes of `--quick`.
    pub quick: Sizes,
}

impl Params {
    /// Reads `bench/params.toml` under the workspace root.
    pub fn read(root: &Path) -> Result<Self> {
        let file = root.join("bench").join("params.toml");
        let text = fs::read_to_string(&file)
            .with_context(|| format!("{} could not be read", file.display()))?;
        toml::from_str(&text).with_context(|| format!("{} is not valid", file.display()))
    }
}

/// One scenario's result.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Outcome {
    /// The scenario's id: `list`, `stat`, `list-stat`, `copy`,
    /// `copy-system`, `slow-place`.
    pub id: String,
    /// What it did, in words.
    pub label: String,
    /// Each run, in milliseconds.
    pub runs_ms: Vec<f64>,
    /// The median run, in milliseconds.
    pub median_ms: f64,
    /// The budget, when there is one, in milliseconds.
    pub budget_ms: Option<u64>,
    /// One more fact in words: a rate, whether the stall was said.
    pub note: String,
}

/// A whole run, as written to `result.json` and kept per platform in
/// `bench/baseline.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Run {
    /// `windows`, `linux` or `macos`.
    pub platform: String,
    /// The day of the run, from the clock, in UTC.
    pub recorded_utc: String,
    /// Whether the sizes were the quick ones.
    pub quick: bool,
    /// Whether the bench was built with optimizations (`--release`); the
    /// budgets and the baselines are for an optimized build.
    #[serde(default)]
    pub optimized: bool,
    /// The scenarios.
    pub scenarios: Vec<Outcome>,
}

/// Whether this bench was built with optimizations.
pub const OPTIMIZED: bool = !cfg!(debug_assertions);

/// `bench/baseline.json`: one run per platform.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Baseline {
    /// What the file is.
    pub about: String,
    /// The baseline run of each platform.
    pub platforms: BTreeMap<String, Run>,
}

/// What `cargo xtask bench` was asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The quick sizes.
    pub quick: bool,
    /// Only this scenario.
    pub only: Option<String>,
    /// Write the result as this platform's baseline.
    pub save_baseline: bool,
    /// Fail when a median leaves the band.
    pub gate: bool,
}

/// The scenario ids, in the order they run.
pub const SCENARIOS: [&str; 13] = [
    "list",
    "stat",
    "list-stat",
    "copy",
    "copy-system",
    "slow-place",
    "archive-zip",
    "archive-tar",
    "archive-7z",
    "unpack",
    "viewer",
    "search-name",
    "search-content",
];

/// The archive listings: their id, the archive's name and its kind in
/// words.
const ARCHIVE_LISTINGS: [(&str, &str, &str); 3] = [
    ("archive-zip", "list.zip", "zip"),
    ("archive-tar", "list.tar", "tar"),
    ("archive-7z", "list.7z", "7z"),
];

const USAGE: &str = "Usage: cargo xtask bench [--quick] [--only ID] [--save-baseline] [--gate]";

/// Reads the arguments after `bench`.
pub fn parse(args: &[&str]) -> Result<Options> {
    let mut options = Options::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match *arg {
            "--quick" => options.quick = true,
            "--save-baseline" => options.save_baseline = true,
            "--gate" => options.gate = true,
            "--only" => {
                let Some(id) = rest.next() else {
                    bail!("--only needs a scenario: {}. {USAGE}", SCENARIOS.join(", "));
                };
                if !SCENARIOS.contains(id) {
                    bail!(
                        "{id} is not a scenario; the scenarios are {}",
                        SCENARIOS.join(", ")
                    );
                }
                options.only = Some((*id).to_owned());
            }
            other => bail!("{other} is not an option of bench. {USAGE}"),
        }
    }
    if options.save_baseline && (options.quick || options.only.is_some()) {
        bail!("a baseline is saved only from a full run of every scenario");
    }
    Ok(options)
}

/// The target folder: `CARGO_TARGET_DIR`, or `target` under the root.
pub(crate) fn target_folder(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| root.join("target"))
}

/// The median of some runs (the lower middle for an even count).
pub fn median(runs: &[f64]) -> f64 {
    let mut sorted = runs.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted
        .get(sorted.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or(0.0)
}

/// A count with thousands separators: "100,000".
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A time in words: "4.2 ms", "412 ms", "5.08 s", "12.5 s".
pub fn time_words(ms: f64) -> String {
    if ms < 10.0 {
        format!("{ms:.1} ms")
    } else if ms < 999.5 {
        format!("{ms:.0} ms")
    } else if ms < 9_995.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// The day from the clock, in UTC: "2026-10-16".
fn today_utc() -> String {
    let c = Timestamp::from_system_time(SystemTime::now()).to_civil(UtcOffset::UTC);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
}

/// A duration in milliseconds, to a tenth: finer than that is noise, and
/// the JSON files stay readable.
fn millis(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() * 10_000.0).round() / 10.0
}

/// Runs a step `runs` times and keeps each time.
fn measure(runs: usize, mut step: impl FnMut(usize) -> Result<Duration>) -> Result<Vec<f64>> {
    (0..runs.max(1)).map(|run| step(run).map(millis)).collect()
}

/// A sink for the bench's jobs: it watches for the stall, skips every
/// conflict and every error, and the job's report says what failed.
#[derive(Debug, Default)]
struct Watch {
    stalled: AtomicBool,
}

impl JobSink for Watch {
    fn progress(&self, progress: &JobProgress) {
        if progress.state == JobState::Stalled {
            self.stalled.store(true, Ordering::Relaxed);
        }
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

fn place_of(path: &Path) -> Result<Place> {
    let text = path
        .to_str()
        .with_context(|| format!("{} is not valid Unicode", path.display()))?;
    Place::parse(text).with_context(|| format!("{text} could not be read as a place"))
}

/// Plans a copy of `sources` into `target`.
fn plan_copy(engine: &Engine, sources: Vec<Place>, target: Place, job: u64) -> Result<Plan> {
    let request = JobRequest::new(JobId::new(job), JobKind::Copy, sources).to(target);
    engine
        .plan(&request)
        .map_err(|e| anyhow::anyhow!("the copy could not be planned: {e}"))
}

/// Runs a planned copy and checks that every source was done. Returns
/// whether the stall was said.
fn run_copy(engine: &Engine, plan: &Plan, count: usize) -> Result<bool> {
    let watch = Watch::default();
    let report = engine.run(plan, &JobControl::new(), &watch);
    if report.done.len() != count || !report.failed.is_empty() || report.cancelled {
        bail!("the copy did not finish: {}", report.summary());
    }
    Ok(watch.stalled.load(Ordering::Relaxed))
}

/// Everything a run needs from outside.
pub struct Setting<'a> {
    /// The sizes.
    pub sizes: &'a Sizes,
    /// The parameters.
    pub params: &'a Params,
    /// The folder to write the trees in (removed with its contents after
    /// the run: it is the run's own).
    pub work: &'a Path,
    /// Only this scenario, or all.
    pub only: Option<&'a str>,
}

fn wanted(setting: &Setting<'_>, ids: &[&str]) -> bool {
    setting.only.is_none_or(|only| ids.contains(&only))
}

/// Runs the scenarios and returns their outcomes, saying what it does as
/// it goes.
pub fn run_scenarios(setting: &Setting<'_>) -> Result<Vec<Outcome>> {
    let sizes = setting.sizes;
    let runs = setting.params.runs;
    let budgets = &setting.params.budgets_ms;
    let windows = cfg!(windows);
    let mut outcomes = Vec::new();
    let registry = Registry::with_local();

    if wanted(setting, &["list", "stat", "list-stat"]) {
        let folder = setting.work.join("list");
        fs::create_dir(&folder)
            .with_context(|| format!("{} could not be made", folder.display()))?;
        let entries = grouped(sizes.list_entries as u64);
        println!("bench: writing {entries} empty files for the listing.");
        let started = Instant::now();
        let names = sample::flat_folder(&folder, sizes.list_entries)?;
        println!(
            "bench: wrote them in {}.",
            time_words(millis(started.elapsed()))
        );
        let place = place_of(&folder)?;
        let provider = registry
            .resolve(&place)
            .map_err(|e| anyhow::anyhow!("no provider for the bench folder: {e}"))?;
        let ctx = OpContext::new();
        let list = |provider: &Arc<dyn Provider>| -> Result<Vec<diroxide_core::Entry>> {
            let entries = provider
                .list(&place, &ctx)
                .map_err(|e| anyhow::anyhow!("the folder could not be listed: {e}"))?;
            if entries.len() != sizes.list_entries {
                bail!(
                    "the listing found {} entries, not {}",
                    entries.len(),
                    sizes.list_entries
                );
            }
            Ok(entries)
        };
        let stat = |provider: &Arc<dyn Provider>, name: &str| -> Result<()> {
            let entry_place = place
                .join(name)
                .map_err(|e| anyhow::anyhow!("{name} could not be joined: {e}"))?;
            provider
                .stat(&entry_place, &ctx)
                .map_err(|e| anyhow::anyhow!("{name} could not be read: {e}"))?;
            Ok(())
        };
        if wanted(setting, &["list"]) {
            let times = measure(runs, |_| {
                let started = Instant::now();
                list(&provider)?;
                Ok(started.elapsed())
            })?;
            outcomes.push(outcome(
                "list",
                format!("List {entries} entries"),
                times,
                Some(if windows {
                    budgets.list_windows
                } else {
                    budgets.list_other
                }),
                "warm, through the local provider".to_owned(),
            ));
        }
        if wanted(setting, &["stat"]) {
            let times = measure(runs, |_| {
                let started = Instant::now();
                for name in &names {
                    stat(&provider, name)?;
                }
                Ok(started.elapsed())
            })?;
            outcomes.push(outcome(
                "stat",
                format!("Stat {entries} entries"),
                times,
                Some(if windows {
                    budgets.stat_windows
                } else {
                    budgets.stat_other
                }),
                "each by its place, without a listing".to_owned(),
            ));
        }
        if wanted(setting, &["list-stat"]) {
            let times = measure(runs, |_| {
                let started = Instant::now();
                for entry in list(&provider)? {
                    stat(&provider, &entry.name)?;
                }
                Ok(started.elapsed())
            })?;
            outcomes.push(outcome(
                "list-stat",
                format!("List, then stat each of {entries}"),
                times,
                None,
                "as a panel that fills its columns late".to_owned(),
            ));
        }
    }

    if wanted(setting, &["copy", "copy-system"]) {
        let folder = setting.work.join("copy");
        fs::create_dir(&folder)
            .with_context(|| format!("{} could not be made", folder.display()))?;
        let source = folder.join("source.bin");
        let size = human_size(sizes.copy_bytes);
        println!("bench: writing a file of {size} to copy.");
        sample::big_file(&source, sizes.copy_bytes, None)?;
        let rate = |times: &[f64]| {
            let seconds = median(times) / 1000.0;
            if seconds > 0.0 {
                format!(
                    "{} a second",
                    human_size((sizes.copy_bytes as f64 / seconds) as u64)
                )
            } else {
                String::new()
            }
        };
        if wanted(setting, &["copy"]) {
            let engine = Engine::new(Arc::new(Registry::with_local()));
            let times = measure(runs, |run| {
                let target = folder.join(format!("engine-{run}"));
                fs::create_dir(&target)
                    .with_context(|| format!("{} could not be made", target.display()))?;
                let (from, to) = (place_of(&source)?, place_of(&target)?);
                let started = Instant::now();
                let plan = plan_copy(&engine, vec![from], to, run as u64 + 1)?;
                run_copy(&engine, &plan, 1)?;
                let elapsed = started.elapsed();
                remove_copy(&target)?;
                Ok(elapsed)
            })?;
            let note = rate(&times);
            outcomes.push(outcome(
                "copy",
                format!("Copy {size} through the engine"),
                times,
                None,
                note,
            ));
        }
        if wanted(setting, &["copy-system"]) {
            let times = measure(runs, |run| {
                let target = folder.join(format!("system-{run}"));
                fs::create_dir(&target)
                    .with_context(|| format!("{} could not be made", target.display()))?;
                let started = Instant::now();
                fs::copy(&source, target.join("source.bin"))
                    .with_context(|| format!("{} could not be copied", source.display()))?;
                let elapsed = started.elapsed();
                remove_copy(&target)?;
                Ok(elapsed)
            })?;
            let note = rate(&times);
            outcomes.push(outcome(
                "copy-system",
                format!("Copy {size} with the system call"),
                times,
                None,
                note,
            ));
        }
        fs::remove_file(&source)
            .with_context(|| format!("{} could not be removed", source.display()))?;
    }

    if wanted(setting, &["slow-place"]) {
        let root = Place::parse(if windows { "Z:\\slow" } else { "/slow" })
            .map_err(|e| anyhow::anyhow!("the slow place could not be named: {e}"))?;
        let mut said = Vec::new();
        let times = measure(runs, |run| {
            let slow = SlowPlace::new(
                &root,
                sizes.slow_files,
                sizes.slow_file_bytes,
                Latency {
                    per_call: Duration::from_millis(sizes.slow_per_call_ms),
                    stall_after_calls: None,
                    stall_for: Duration::from_millis(sizes.slow_stall_ms),
                },
            )
            .map_err(|e| anyhow::anyhow!("the slow place could not be built: {e}"))?;
            let mut registry = Registry::new();
            registry.set_local(Arc::new(slow.fake.clone()));
            let mut options = EngineOptions::default();
            options.stall_after = Duration::from_millis(sizes.stall_after_ms);
            let engine = Engine::new(Arc::new(registry)).with_options(options);
            let count = slow.sources.len();
            let started = Instant::now();
            let plan = plan_copy(&engine, slow.sources, slow.target, run as u64 + 1)?;
            // The stall comes that many calls into the run, after the
            // plan, where the job is there to say it.
            let calls = slow.fake.calls();
            slow.fake.switches_mut().latency.stall_after_calls =
                Some(calls + sizes.slow_stall_after_calls);
            said.push(run_copy(&engine, &plan, count)?);
            Ok(started.elapsed())
        })?;
        let heard = said.iter().filter(|s| **s).count();
        outcomes.push(outcome(
            "slow-place",
            format!("Copy {} files from a slow place", sizes.slow_files),
            times,
            None,
            format!("stall said in {heard} of {} runs", said.len()),
        ));
    }

    if wanted(
        setting,
        &["archive-zip", "archive-tar", "archive-7z", "unpack"],
    ) {
        outcomes.extend(archive_scenarios(setting)?);
    }

    if wanted(setting, &["viewer"]) {
        let folder = setting.work.join("viewer");
        fs::create_dir(&folder)
            .with_context(|| format!("{} could not be made", folder.display()))?;
        let log = folder.join("big.log");
        let size = human_size(sizes.viewer_bytes);
        println!("bench: writing a text log of {size} for the viewer.");
        log_file(&log, sizes.viewer_bytes)?;
        let place = place_of(&log)?;
        let times = measure(runs, |_| {
            let started = Instant::now();
            let (mut viewer, _said) = diroxide_app::viewer::Viewer::open(
                &registry,
                &place,
                Some(sizes.viewer_bytes),
                diroxide_app::viewer::ViewerOptions::default(),
            )
            .map_err(|e| anyhow::anyhow!("the viewer could not open the log: {e}"))?;
            viewer.set_size(80, 24);
            let rows = viewer.rows();
            let elapsed = started.elapsed();
            if rows.len() < 24 || !rows[0].text.starts_with("2026-10-16") {
                bail!("the viewer's first screen has {} rows", rows.len());
            }
            Ok(elapsed)
        })?;
        fs::remove_file(&log).with_context(|| format!("{} could not be removed", log.display()))?;
        outcomes.push(outcome(
            "viewer",
            format!("View the first screen of a {size} log"),
            times,
            None,
            "24 rows, screen-reader mode".to_owned(),
        ));
    }

    if wanted(setting, &["search-name", "search-content"]) {
        outcomes.extend(search_scenarios(setting)?);
    }
    Ok(outcomes)
}

/// The search scenarios (alpha.3, W3s): the search tree written once
/// (the small one with `--quick`), then searched by name and by contents
/// through the search engine itself (`run_search`, as the app runs it),
/// each run checked against the tree's known count of hits.
fn search_scenarios(setting: &Setting<'_>) -> Result<Vec<Outcome>> {
    let mut outcomes = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("the workspace root could not be found")?;
    let quick = setting.sizes == &setting.params.quick;
    let tree_params = sample::tree_params(root, quick)?;
    let tree = setting.work.join("search-tree");
    println!(
        "bench: writing the search tree, {} files in {} folders.",
        grouped(tree_params.files),
        grouped(tree_params.folders)
    );
    let started = Instant::now();
    let facts = sample::write_search_tree(&tree, &tree_params)?;
    println!(
        "bench: wrote {} in {}.",
        human_size(facts.bytes),
        time_words(millis(started.elapsed()))
    );
    let place = place_of(&tree)?;
    let registry = Registry::with_local();
    let clock = SystemClock::utc();
    let budgets = &setting.params.budgets_ms;
    let windows = cfg!(windows);
    let scenarios = [
        (
            "search-name",
            format!("name {}*", tree_params.hit_word),
            format!("Search {} files by name", grouped(facts.files)),
            if windows {
                budgets.search_name_windows
            } else {
                budgets.search_name_other
            },
        ),
        (
            "search-content",
            format!("content {}", quoted(&tree_params.hit_word)),
            format!("Search {} of files by contents", human_size(facts.bytes)),
            if windows {
                budgets.search_content_windows
            } else {
                budgets.search_content_other
            },
        ),
    ];
    for (id, expression, label, budget) in scenarios {
        if !wanted(setting, &[id]) {
            continue;
        }
        let query = SearchQuery::parse(place.clone(), &expression)
            .map_err(|e| anyhow::anyhow!("{expression} does not read: {}", e.line()))?
            .with_scope(Scope::new(place.clone()).with_row_cap(None));
        let mut found = 0;
        let times = measure(setting.params.runs, |_| {
            let started = Instant::now();
            let summary = run_search(
                &query,
                &registry,
                &clock,
                &SearchControl::new(),
                &mut FnSink(|_: SearchHit| {}),
            )
            .map_err(|e| anyhow::anyhow!("the search could not run: {e}"))?;
            let elapsed = started.elapsed();
            found = summary.hits;
            if found != facts.hit_files {
                bail!(
                    "the search found {found}, and the tree holds {} hits",
                    facts.hit_files
                );
            }
            Ok(elapsed)
        })?;
        outcomes.push(outcome(
            id,
            label,
            times,
            Some(budget),
            format!("{} found, through the search engine", grouped(found)),
        ));
    }
    Ok(outcomes)
}

/// Builds `diro` with this bench's own profile (release for an optimized
/// bench) and returns its path in the target folder.
pub fn diro_binary(root: &Path) -> Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .args(["build", "--quiet", "--package", "diro", "--bin", "diro"]);
    if OPTIMIZED {
        command.arg("--release");
    }
    let status = command
        .status()
        .context("cargo could not be started to build diro")?;
    if !status.success() {
        bail!("diro could not be built: cargo ended with {status}");
    }
    let profile = if OPTIMIZED { "release" } else { "debug" };
    let path = target_folder(root)
        .join(profile)
        .join(format!("diro{}", std::env::consts::EXE_SUFFIX));
    if !path.is_file() {
        bail!("diro was built, but {} is not there", path.display());
    }
    Ok(path)
}

/// Runs `diro` with these arguments in `folder`, standard input given,
/// and returns what it wrote; fails unless it ends with 0.
pub fn run_diro(diro: &Path, folder: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut child = Command::new(diro)
        .current_dir(folder)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{} could not be started", diro.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input)
            .context("diro's standard input could not be written")?;
    }
    let output = child
        .wait_with_output()
        .context("diro could not be waited for")?;
    if !output.status.success() {
        bail!(
            "diro {} ended with {}: {}{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// The workspace's root: the parent of this crate's folder (the demo's
/// test builds `diro` there).
#[cfg(test)]
pub fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("the workspace root could not be found")
}

/// The local provider and the archive provider, connected, at the
/// default limits: a new pair for each timed run, so no run reads an index
/// another run cached.
pub fn archive_registry() -> (Arc<Registry>, Arc<ArchiveProvider>) {
    ArchiveProvider::install(Registry::with_local(), Limits::default())
}

/// Packs `items` into `archive` with diroxide's own pack runner, in the
/// format its name says, with `password` when one is given (a zip is then
/// AES-256).
pub fn pack_items(archive: &Path, items: &[PathBuf], password: Option<&str>) -> Result<()> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("{} has no plain name", archive.display()))?;
    let format = ArchiveFormat::from_name(name)
        .with_context(|| format!("{name} is not a name of an archive diroxide writes"))?;
    let sources = items
        .iter()
        .map(|item| place_of(item))
        .collect::<Result<Vec<_>>>()?;
    let mut request = PackRequest::new(sources, place_of(archive)?, format);
    request.password = password.map(|p| diroxide_core::Secret::new(p.to_owned()));
    let (registry, _) = archive_registry();
    diroxide_archive::pack(&request, &registry, &OpContext::new())
        .map_err(|e| anyhow::anyhow!("{name} could not be packed: {e}"))
}

/// The archive scenarios: a zip, a tar and a 7z of empty members, packed
/// by the pack runner and listed through the archive provider; a zip of
/// one large file unpacked by the unpack runner.
fn archive_scenarios(setting: &Setting<'_>) -> Result<Vec<Outcome>> {
    let sizes = setting.sizes;
    let runs = setting.params.runs;
    let mut outcomes = Vec::new();
    let folder = setting.work.join("archive");
    let members = folder.join("members");
    fs::create_dir_all(&members)
        .with_context(|| format!("{} could not be made", members.display()))?;
    let listings: Vec<_> = ARCHIVE_LISTINGS
        .iter()
        .filter(|(id, _, _)| wanted(setting, &[id]))
        .collect();
    if !listings.is_empty() {
        let entries = grouped(sizes.archive_entries as u64);
        println!("bench: writing {entries} empty files to pack.");
        let names = sample::flat_folder(&members, sizes.archive_entries)?;
        let items: Vec<PathBuf> = names.iter().map(|name| members.join(name)).collect();
        for (id, archive, kind) in listings {
            println!("bench: packing them into a {kind}.");
            let path = folder.join(archive);
            pack_items(&path, &items, None)?;
            let root = place_of(&path)?.into_archive();
            let times = measure(runs, |_| {
                let (registry, _) = archive_registry();
                let started = Instant::now();
                let listed = registry
                    .resolve(&root)
                    .and_then(|provider| provider.list(&root, &OpContext::new()))
                    .map_err(|e| anyhow::anyhow!("the {kind} could not be listed: {e}"))?;
                let elapsed = started.elapsed();
                if listed.len() != sizes.archive_entries {
                    bail!("the {kind} listed {} members, not {entries}", listed.len());
                }
                Ok(elapsed)
            })?;
            fs::remove_file(&path)
                .with_context(|| format!("{} could not be removed", path.display()))?;
            outcomes.push(outcome(
                id,
                format!("List {entries} entries in a {kind}"),
                times,
                None,
                "through the archive provider, from the open to the last entry".to_owned(),
            ));
        }
        for name in &names {
            let path = members.join(name);
            fs::remove_file(&path)
                .with_context(|| format!("{} could not be removed", path.display()))?;
        }
    }
    if wanted(setting, &["unpack"]) {
        let source = members.join("unpack.bin");
        let size = human_size(sizes.unpack_bytes);
        println!("bench: writing a file of {size} and packing it into a zip.");
        sample::big_file(&source, sizes.unpack_bytes, None)?;
        let zip = folder.join("unpack.zip");
        pack_items(&zip, std::slice::from_ref(&source), None)?;
        fs::remove_file(&source)
            .with_context(|| format!("{} could not be removed", source.display()))?;
        let zip_place = place_of(&zip)?;
        let times = measure(runs, |run| {
            let target = folder.join(format!("unpack-{run}"));
            fs::create_dir(&target)
                .with_context(|| format!("{} could not be made", target.display()))?;
            let (registry, archives) = archive_registry();
            let engine = Engine::new(Arc::clone(&registry));
            let control = JobControl::new();
            let sink = Watch::default();
            let job = JobId::new(run as u64 + 1);
            let request = UnpackRequest::new(zip_place.clone(), place_of(&target)?);
            let allow = Allowances::default();
            let started = Instant::now();
            let plan = plan_unpack(
                &archives,
                &registry,
                &request,
                job,
                &OpContext::new(),
                allow,
            )
            .map_err(|e| anyhow::anyhow!("the unpack could not be planned: {e}"))?;
            let env = JobEnv::new(&engine, &archives, job, &control, &sink);
            let outcome = run_unpack(&plan, None, allow, &env);
            let elapsed = started.elapsed();
            if !outcome.report.failed.is_empty() || outcome.report.cancelled {
                bail!("the unpack did not finish: {}", outcome.lines.join(" "));
            }
            let unpacked = target.join("unpack.bin");
            let length = fs::metadata(&unpacked)
                .with_context(|| format!("{} was not unpacked", unpacked.display()))?
                .len();
            if length != sizes.unpack_bytes {
                bail!(
                    "the unpacked file has {length} bytes, not {}",
                    sizes.unpack_bytes
                );
            }
            fs::remove_file(&unpacked)
                .with_context(|| format!("{} could not be removed", unpacked.display()))?;
            fs::remove_dir(&target)
                .with_context(|| format!("{} could not be removed", target.display()))?;
            Ok(elapsed)
        })?;
        fs::remove_file(&zip).with_context(|| format!("{} could not be removed", zip.display()))?;
        let seconds = median(&times) / 1000.0;
        let note = if seconds > 0.0 {
            format!(
                "{} a second, through the unpack runner",
                human_size((sizes.unpack_bytes as f64 / seconds) as u64)
            )
        } else {
            String::new()
        };
        outcomes.push(outcome(
            "unpack",
            format!("Unpack a zip of {size}"),
            times,
            None,
            note,
        ));
    }
    Ok(outcomes)
}

/// One line of the bench's and the demo's logs, in the shape of a
/// server's log, about 60 bytes, starting with the date.
pub fn log_line(n: u64) -> String {
    let level = match n % 7 {
        0 => "WARN",
        3 => "DEBUG",
        _ => "INFO",
    };
    format!(
        "2026-10-16 14:{:02}:{:02} {level} request {n:07} served in {} ms\n",
        (n / 60) % 60,
        n % 60,
        n % 97 + 3
    )
}

/// Writes a text log of whole lines, at least `bytes` bytes.
pub fn log_file(path: &Path, bytes: u64) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("{} could not be made", path.display()))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    let mut written = 0u64;
    let mut n = 1u64;
    while written < bytes {
        let line = log_line(n);
        out.write_all(line.as_bytes())
            .with_context(|| format!("{} could not be written", path.display()))?;
        written += line.len() as u64;
        n += 1;
    }
    out.flush()
        .with_context(|| format!("{} could not be written", path.display()))
}

/// Removes the one file a copy run made, then its folder: only what the
/// run itself wrote.
fn remove_copy(target: &Path) -> Result<()> {
    let copy = target.join("source.bin");
    fs::remove_file(&copy).with_context(|| format!("{} could not be removed", copy.display()))?;
    fs::remove_dir(target).with_context(|| format!("{} could not be removed", target.display()))
}

fn outcome(
    id: &str,
    label: String,
    runs_ms: Vec<f64>,
    budget_ms: Option<u64>,
    note: String,
) -> Outcome {
    let median_ms = median(&runs_ms);
    Outcome {
        id: id.to_owned(),
        label,
        runs_ms,
        median_ms,
        budget_ms,
        note,
    }
}

/// How a median compares with its baseline, in words, and whether it is
/// inside the band.
pub fn against_baseline(median_ms: f64, baseline_ms: f64, tolerance: u64) -> (String, bool) {
    if baseline_ms <= 0.0 {
        return ("no baseline to compare".to_owned(), true);
    }
    let change = (median_ms - baseline_ms) / baseline_ms * 100.0;
    // Judged on the whole percent that is said, so the words and the
    // verdict agree.
    let rounded = change.abs().round();
    let inside = rounded <= tolerance as f64;
    let words = if rounded < 1.0 {
        format!("Baseline {}: the same", time_words(baseline_ms))
    } else if change > 0.0 {
        format!(
            "Baseline {}: {rounded:.0} percent slower",
            time_words(baseline_ms)
        )
    } else {
        format!(
            "Baseline {}: {rounded:.0} percent faster",
            time_words(baseline_ms)
        )
    };
    let band = if inside {
        format!("inside the {tolerance} percent band")
    } else {
        format!("outside the {tolerance} percent band")
    };
    (format!("{words}, {band}."), inside)
}

/// The lines that say one outcome: the median and the budget first, then
/// the runs, the note and the baseline.
pub fn describe(
    outcome: &Outcome,
    baseline: Option<&Outcome>,
    tolerance: u64,
) -> (Vec<String>, bool) {
    let mut first = format!("{}: {}", outcome.label, time_words(outcome.median_ms));
    if let Some(budget) = outcome.budget_ms {
        if outcome.median_ms <= budget as f64 {
            first.push_str(&format!(", within the {budget} ms budget"));
        } else {
            first.push_str(&format!(", over the {budget} ms budget"));
        }
    }
    first.push('.');
    let runs: Vec<String> = outcome.runs_ms.iter().map(|ms| time_words(*ms)).collect();
    let mut second = format!("Runs: {}.", runs.join(", "));
    if !outcome.note.is_empty() {
        let mut chars = outcome.note.chars();
        if let Some(c) = chars.next() {
            second.push(' ');
            second.extend(c.to_uppercase());
            second.push_str(chars.as_str());
            second.push('.');
        }
    }
    let mut lines = vec![first, second];
    let mut inside = true;
    if let Some(base) = baseline {
        let (words, within) = against_baseline(outcome.median_ms, base.median_ms, tolerance);
        lines.push(words);
        inside = within;
    }
    (lines, inside)
}

/// `cargo xtask bench`.
pub fn run(root: &Path, args: &[&str]) -> Result<bool> {
    let options = parse(args)?;
    if options.save_baseline && !OPTIMIZED {
        bail!(
            "a baseline is saved only from an optimized build: cargo run --release -p xtask -- bench --save-baseline"
        );
    }
    let params = Params::read(root)?;
    let sizes = if options.quick {
        &params.quick
    } else {
        &params.full
    };
    let platform = std::env::consts::OS.to_owned();
    let bench_folder = target_folder(root).join("bench");
    fs::create_dir_all(&bench_folder)
        .with_context(|| format!("{} could not be made", bench_folder.display()))?;
    let work = tempfile::Builder::new()
        .prefix("work-")
        .tempdir_in(&bench_folder)
        .with_context(|| format!("no work folder could be made in {}", bench_folder.display()))?;
    println!(
        "bench: {platform}, {} runs of each scenario, the median first. Trees in {}.",
        params.runs,
        work.path().display()
    );
    if !OPTIMIZED {
        println!(
            "bench: this build has no optimizations; the budgets are for cargo run --release -p xtask -- bench."
        );
    }
    let outcomes = run_scenarios(&Setting {
        sizes,
        params: &params,
        work: work.path(),
        only: options.only.as_deref(),
    })?;
    println!("bench: removing the trees.");
    work.close().with_context(|| {
        format!(
            "the work folder in {} could not be removed",
            bench_folder.display()
        )
    })?;

    let baseline_file = root.join("bench").join("baseline.json");
    let mut baseline: Baseline = match fs::read_to_string(&baseline_file) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid", baseline_file.display()))?,
        Err(_) => Baseline::default(),
    };
    let base_run = baseline
        .platforms
        .get(&platform)
        .filter(|run| run.quick == options.quick && run.optimized == OPTIMIZED)
        .cloned();
    if base_run.is_none() {
        println!(
            "bench: no {platform} baseline for these sizes and this build yet; nothing to compare."
        );
    }
    let mut all_inside = true;
    for outcome in &outcomes {
        let base = base_run
            .as_ref()
            .and_then(|run| run.scenarios.iter().find(|s| s.id == outcome.id));
        let (lines, inside) = describe(outcome, base, params.tolerance_percent);
        all_inside &= inside;
        for line in lines {
            println!("{line}");
        }
    }
    let result = Run {
        platform: platform.clone(),
        recorded_utc: today_utc(),
        quick: options.quick,
        optimized: OPTIMIZED,
        scenarios: outcomes,
    };
    let result_file = bench_folder.join("result.json");
    fs::write(
        &result_file,
        format!("{}\n", serde_json::to_string_pretty(&result)?),
    )
    .with_context(|| format!("{} could not be written", result_file.display()))?;
    println!("bench: the medians are in {}.", result_file.display());
    if options.save_baseline {
        if baseline.about.is_empty() {
            baseline.about = "The bench's baseline per platform, written by cargo xtask bench --save-baseline (docs/dev/bench.md).".to_owned();
        }
        baseline.platforms.insert(platform.clone(), result);
        fs::write(
            &baseline_file,
            format!("{}\n", serde_json::to_string_pretty(&baseline)?),
        )
        .with_context(|| format!("{} could not be written", baseline_file.display()))?;
        println!("bench: saved as the {platform} baseline in bench/baseline.json.");
    }
    if options.gate && !all_inside {
        println!(
            "bench: Fail. A median left the {} percent band.",
            params.tolerance_percent
        );
        return Ok(false);
    }
    let verdict = if all_inside {
        "Pass"
    } else {
        "Pass, report only"
    };
    println!("bench: {verdict}.");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn the_parameters_read() {
        let params = Params::read(&root()).unwrap();
        assert_eq!(params.runs, 3);
        assert_eq!(params.full.list_entries, 100_000);
        assert_eq!(params.full.copy_bytes, 1_000_000_000);
        assert_eq!(params.budgets_ms.list_windows, 800);
        assert_eq!(params.budgets_ms.list_other, 300);
        assert_eq!(params.full.archive_entries, 100_000);
        assert_eq!(params.full.unpack_bytes, 200_000_000);
        assert_eq!(params.full.viewer_bytes, 256_000_000);
    }

    #[test]
    fn log_lines_start_with_the_date_and_read_well() {
        assert_eq!(
            log_line(1),
            "2026-10-16 14:00:01 INFO request 0000001 served in 4 ms\n"
        );
        assert!(log_line(7).contains(" WARN "));
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("big.log");
        log_file(&log, 10_000).unwrap();
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.len() >= 10_000 && text.len() < 10_100);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
        let options = parse(&["--quick", "--only", "copy", "--gate"]).unwrap();
        assert!(options.quick && options.gate);
        assert_eq!(options.only.as_deref(), Some("copy"));
        assert!(parse(&["--only", "dance"]).is_err());
        assert!(parse(&["--quick", "--save-baseline"]).is_err());
        assert!(parse(&["--fast"]).is_err());
    }

    #[test]
    fn medians_and_words() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0]), 1.0);
        assert_eq!(time_words(4.24), "4.2 ms");
        assert_eq!(time_words(412.4), "412 ms");
        assert_eq!(time_words(999.6), "1.00 s");
        assert_eq!(time_words(5078.0), "5.08 s");
        assert_eq!(time_words(12_500.0), "12.5 s");
        assert_eq!(grouped(100_000), "100,000");
        assert_eq!(grouped(999), "999");
    }

    #[test]
    fn the_ratchet_says_both_directions() {
        assert_eq!(
            against_baseline(440.0, 400.0, 20),
            (
                "Baseline 400 ms: 10 percent slower, inside the 20 percent band.".to_owned(),
                true
            )
        );
        assert_eq!(
            against_baseline(300.0, 400.0, 20),
            (
                "Baseline 400 ms: 25 percent faster, outside the 20 percent band.".to_owned(),
                false
            )
        );
        assert!(against_baseline(401.0, 400.0, 20).0.contains("the same"));
        // 20.4 percent is said as 20, and so is inside a 20 percent band.
        assert!(against_baseline(481.6, 400.0, 20).1);
        assert!(!against_baseline(482.0, 400.0, 20).1);
    }

    #[test]
    fn lines_say_the_median_and_budget_first() {
        let outcome = outcome(
            "list",
            "List 100,000 entries".to_owned(),
            vec![398.0, 455.0, 412.0],
            Some(800),
            "warm".to_owned(),
        );
        let (lines, inside) = describe(&outcome, None, 20);
        assert!(inside);
        assert_eq!(
            lines[0],
            "List 100,000 entries: 412 ms, within the 800 ms budget."
        );
        assert_eq!(lines[1], "Runs: 398 ms, 455 ms, 412 ms. Warm.");
        assert!(diroxide_core::format::within_cells(
            &lines[0],
            "List 100,000 entries: 412 ms"
        ));
    }

    #[test]
    fn a_quick_run_of_every_scenario_works_and_cleans_up() {
        let params = Params::read(&root()).unwrap();
        let work = tempfile::tempdir().unwrap();
        let outcomes = run_scenarios(&Setting {
            sizes: &params.quick,
            params: &params,
            work: work.path(),
            only: None,
        })
        .unwrap();
        let ids: Vec<&str> = outcomes.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, SCENARIOS);
        for outcome in &outcomes {
            assert_eq!(outcome.runs_ms.len(), params.runs);
            assert!(outcome.median_ms > 0.0, "{}", outcome.id);
        }
        let slow = outcomes.iter().find(|o| o.id == "slow-place").unwrap();
        assert_eq!(slow.note, "stall said in 3 of 3 runs");
        // The copies and the source are gone; only the listing's folder is
        // left, for the work folder's own removal.
        let copy = work.path().join("copy");
        assert_eq!(fs::read_dir(copy).unwrap().count(), 0);
    }
}
