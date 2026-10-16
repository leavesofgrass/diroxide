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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use diroxide_core::format::human_size;
use diroxide_core::{
    ConflictAnswer, ErrorAction, JobId, JobKind, JobRequest, JobState, Place, Timestamp, UtcOffset,
};
use diroxide_ops::{
    Conflict, Engine, EngineOptions, ItemError, JobControl, JobProgress, JobSink, Latency, Plan,
    SlowPlace,
};
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
pub const SCENARIOS: [&str; 6] = [
    "list",
    "stat",
    "list-stat",
    "copy",
    "copy-system",
    "slow-place",
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
fn target_folder(root: &Path) -> PathBuf {
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

/// A time in words: "4.2 ms", "412 ms", "12.5 s".
pub fn time_words(ms: f64) -> String {
    if ms < 10.0 {
        format!("{ms:.1} ms")
    } else if ms < 10_000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// The day from the clock, in UTC: "2026-10-16".
fn today_utc() -> String {
    let c = Timestamp::from_system_time(SystemTime::now()).to_civil(UtcOffset::UTC);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
}

fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
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
    Ok(outcomes)
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
    let inside = change.abs() <= tolerance as f64;
    let rounded = change.abs().round();
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
