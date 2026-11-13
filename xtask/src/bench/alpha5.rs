//! The alpha 5 scenarios (lane B5, R34's commits 41 and 44): the batch
//! rename's preview, a compare of two trees, duplicates, the flat view, a
//! tab switch, and one batched `column/format` request to a process
//! plugin, the number question 128 waits on.
//!
//! Each calls the engine as its lane built it: the rename plan of
//! `diroxide_ops::rename_rules` (BR, RA), the compare of
//! `diroxide_ops::sync` (CS, SA), the flat view of `diroxide_search`
//! (FC, LA); duplicates and tabs go through the app itself, since the
//! duplicate search's hash is the metadata crate's BLAKE3, which only the
//! app reaches (DU, DA), and tabs live in the app (TB). The plugin is the
//! tree's own `diroxide-fake-plugin`, a correct process plugin in its
//! `echo` mode, built from the tree; nothing is downloaded.
//!
//! Every tree is written in the run's work folder and goes with it.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use diroxide_core::{Effect, Key, Place, Side};
use diroxide_ops::{Engine, JobControl};
use diroxide_vfs::{OpContext, Registry};

use super::{
    OPTIMIZED, Outcome, Setting, grouped, measure, median, millis, outcome, place_of,
    scenario_folder, target_folder, time_words,
};

/// The time every file of the compare's trees carries, on both sides, so
/// only the differences the tree means are different.
fn tree_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_780_000_000)
}

/// What the compare's trees hold, counted as they are written.
#[derive(Clone, Debug, Default)]
pub struct Trees {
    /// The tree on the left: every file.
    pub here: PathBuf,
    /// The tree on the right: most of them, some changed, some more.
    pub there: PathBuf,
    /// Files in the left tree.
    pub files: u64,
    /// Files only in the left tree.
    pub only_here: u64,
    /// Files only in the right tree.
    pub only_there: u64,
    /// Files in both whose sizes differ.
    pub different: u64,
}

/// Writes the compare's two trees, `compare_files` files on the left in
/// `compare_folders` folders: one in a hundred left out on the right, one
/// in a hundred one byte longer there, and one in two hundred only there.
/// Every file is empty but the changed ones, and every file carries the
/// same time.
pub fn compare_trees(setting: &Setting<'_>) -> Result<Trees> {
    let sizes = setting.sizes;
    let folder = scenario_folder(setting, "compare")?;
    let here = folder.join("here");
    let there = folder.join("there");
    let folders = sizes.compare_folders.max(1);
    let per_folder = sizes.compare_files.div_ceil(folders);
    println!(
        "bench: writing two trees of {} files in {} folders to compare.",
        grouped(sizes.compare_files as u64),
        grouped(folders as u64)
    );
    let started = Instant::now();
    let mut trees = Trees {
        here: here.clone(),
        there: there.clone(),
        ..Trees::default()
    };
    for f in 0..folders {
        let name = format!("folder-{f:04}");
        let (left, right) = (here.join(&name), there.join(&name));
        fs::create_dir_all(&left)
            .with_context(|| format!("{} could not be made", left.display()))?;
        fs::create_dir_all(&right)
            .with_context(|| format!("{} could not be made", right.display()))?;
        let first = f * per_folder;
        let last = (first + per_folder).min(sizes.compare_files);
        for i in first..last {
            let name = format!("file-{i:06}.txt");
            trees.files += 1;
            match i % 100 {
                7 => {
                    tree_file(&left.join(&name), b"")?;
                    trees.only_here += 1;
                }
                13 => {
                    tree_file(&left.join(&name), b"a")?;
                    tree_file(&right.join(&name), b"ab")?;
                    trees.different += 1;
                }
                _ => {
                    tree_file(&left.join(&name), b"")?;
                    tree_file(&right.join(&name), b"")?;
                }
            }
            if i % 200 == 0 {
                tree_file(&right.join(format!("extra-{i:06}.txt")), b"")?;
                trees.only_there += 1;
            }
        }
    }
    println!(
        "bench: wrote them in {}.",
        time_words(millis(started.elapsed()))
    );
    Ok(trees)
}

/// One file of the compare's trees, with the trees' one time.
fn tree_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        File::create(path).with_context(|| format!("{} could not be made", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("{} could not be written", path.display()))?;
    file.set_modified(tree_time())
        .with_context(|| format!("the time of {} could not be set", path.display()))
}

/// The batch rename's preview of many names (R34's commit 41): the
/// folder's facts read, the plan made, its count line and every row's
/// line, as the app's preview opens on the marked files. Ten of the new
/// names are taken already, so the plan has problems to put first.
pub fn rename_scenario(setting: &Setting<'_>) -> Result<Outcome> {
    use diroxide_ops::rename_rules::{
        CaseForm, Part, Recipe, RenameSource, Rule, plan_rename, read_context,
    };
    let count = setting.sizes.rename_files;
    let folder = scenario_folder(setting, "rename")?;
    println!(
        "bench: writing {} empty files to rename.",
        grouped(count as u64)
    );
    let every = (count / 10).max(1);
    let mut clashes = 0;
    for n in 1..=count {
        let path = folder.join(format!("IMG_{n:05}.JPG"));
        File::create(&path).with_context(|| format!("{} could not be made", path.display()))?;
        if n % every == 0 {
            let taken = folder.join(format!("trip-{n:05}.jpg"));
            File::create(&taken)
                .with_context(|| format!("{} could not be made", taken.display()))?;
            clashes += 1;
        }
    }
    let place = place_of(&folder)?;
    let registry = Arc::new(Registry::with_local());
    let provider = registry
        .resolve(&place)
        .map_err(|e| anyhow::anyhow!("no provider for the bench folder: {e}"))?;
    let sources: Vec<RenameSource> = provider
        .list(&place, &OpContext::new())
        .map_err(|e| anyhow::anyhow!("the folder could not be listed: {e}"))?
        .into_iter()
        .filter(|entry| entry.name.starts_with("IMG_"))
        .map(|entry| {
            let at = place
                .join(&entry.name)
                .map_err(|e| anyhow::anyhow!("{} could not be joined: {e}", entry.name))?;
            Ok(RenameSource::new(at, entry))
        })
        .collect::<Result<_>>()?;
    if sources.len() != count {
        bail!(
            "the folder lists {} files to rename, not {count}",
            sources.len()
        );
    }
    let recipe = Recipe::of(vec![
        Rule::replace("IMG_", "trip-"),
        Rule::case(CaseForm::Lower).on(Part::Extension),
    ]);
    let engine = Engine::new(Arc::clone(&registry));
    let mut plans = Vec::new();
    let mut count_line = String::new();
    let times = measure(setting.params.runs, |_| {
        let started = Instant::now();
        let context = read_context(&engine, &sources, &JobControl::new());
        let planning = Instant::now();
        let plan = plan_rename(&sources, &recipe, &context)
            .map_err(|e| anyhow::anyhow!("the recipe was refused: {e}"))?;
        plans.push(millis(planning.elapsed()));
        count_line = plan.count_line();
        let preview: Vec<String> = plan.problems_first().iter().map(|row| row.line()).collect();
        let elapsed = started.elapsed();
        if preview.len() != count || plan.problems() != clashes {
            bail!(
                "the preview has {} rows and {} problems, not {count} and {clashes}",
                preview.len(),
                plan.problems()
            );
        }
        Ok(elapsed)
    })?;
    Ok(outcome(
        "rename-preview",
        format!("Preview renaming {} files", grouped(count as u64)),
        times,
        setting.params.budgets_ms.budget_for("rename-preview"),
        format!(
            "the plan alone {}, made again after each change; {count_line}",
            time_words(median(&plans))
        ),
    ))
}

/// A compare of two trees (R34's commit 41), by size and time, as Compare
/// folders runs it: every row read, the count line that adds up, and the
/// synchronize plan the result offers next.
pub fn compare_scenario(setting: &Setting<'_>, trees: &Trees) -> Result<Outcome> {
    use diroxide_ops::sync::{CompareCriteria, CompareProgress, SyncMode, compare_dirs, plan_sync};
    let registry = Registry::with_local();
    let (here, there) = (place_of(&trees.here)?, place_of(&trees.there)?);
    let criteria = CompareCriteria::default();
    let sink = |_: &CompareProgress| {};
    let mut plans = Vec::new();
    let mut count_line = String::new();
    let times = measure(setting.params.runs, |_| {
        let started = Instant::now();
        let result = compare_dirs(
            &registry,
            &here,
            &there,
            &criteria,
            &JobControl::new(),
            &sink,
        )
        .map_err(|e| anyhow::anyhow!("the trees could not be compared: {e}"))?;
        count_line = result.count_line();
        let elapsed = started.elapsed();
        let counts = result.counts();
        let found = (counts.only_left, counts.only_right, counts.different);
        let meant = (trees.only_here, trees.only_there, trees.different);
        if found != meant {
            bail!("the compare found {found:?} only here, only there and different, not {meant:?}");
        }
        let planning = Instant::now();
        let plan = plan_sync(&result, SyncMode::UpdateThere);
        let _ = plan.summary();
        plans.push(millis(planning.elapsed()));
        Ok(elapsed)
    })?;
    Ok(outcome(
        "compare-trees",
        format!(
            "Compare two trees of {} files",
            grouped(setting.sizes.compare_files as u64)
        ),
        times,
        setting.params.budgets_ms.budget_for("compare-trees"),
        format!(
            "by size and time; the plan to update there {} more; {count_line}",
            time_words(median(&plans))
        ),
    ))
}

/// The flat view of a tree (R34's commit 41): every file below the
/// compare's left tree as one list, as the View menu's Flat view runs it
/// with the panel's options, then made into the lister's rows.
pub fn flat_scenario(setting: &Setting<'_>, trees: &Trees) -> Result<Outcome> {
    use diroxide_search::{FlatOptions, FnSink, SearchControl, SearchHit, flat_list, flat_view};
    let registry = Registry::with_local();
    let root = place_of(&trees.here)?;
    let options = FlatOptions::for_panel(false);
    let mut line = String::new();
    let times = measure(setting.params.runs, |_| {
        let started = Instant::now();
        let mut hits = Vec::new();
        let summary = {
            let mut sink = FnSink(|hit: SearchHit| hits.push(hit));
            flat_view(&root, &options, &registry, &SearchControl::new(), &mut sink)
                .map_err(|e| anyhow::anyhow!("the flat view could not run: {e}"))?
        };
        let list = flat_list(&root, &hits);
        let elapsed = started.elapsed();
        if summary.files != trees.files || list.rows().len() as u64 != trees.files {
            bail!(
                "the flat view found {} files and {} rows, not {}",
                summary.files,
                list.rows().len(),
                trees.files
            );
        }
        line = summary.count_line();
        Ok(elapsed)
    })?;
    Ok(outcome(
        "flat-view",
        format!("Flat view of {} files", grouped(trees.files)),
        times,
        setting.params.budgets_ms.budget_for("flat-view"),
        format!("through the search engine, rows made; {line}"),
    ))
}

/// What the app said in these effects.
fn said(effects: Vec<Effect>) -> Vec<String> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Announce(message) => Some(message.text),
            _ => None,
        })
        .collect()
}

/// An app on two folders, started and idle, with no question about
/// components and no settle time on its dialogs.
fn started_app(left: &Path, right: &Path) -> Result<(diroxide_app::App, diroxide_app::Runtime)> {
    use diroxide_app::{App, AppOptions, Runtime};
    let mut options = AppOptions::new(place_of(left)?, place_of(right)?);
    options.settings.components_first_run_shown = true;
    options.settings.dialog_settle_ms = 0;
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local().with_components_offline();
    let effects = app.start();
    runtime.run_until_idle(&mut app, effects);
    Ok((app, runtime))
}

/// Presses a key in the app and runs until it is idle; what it said.
fn press(
    app: &mut diroxide_app::App,
    runtime: &mut diroxide_app::Runtime,
    key: &str,
) -> Vec<String> {
    let Ok(key) = Key::parse(key) else {
        return Vec::new();
    };
    let effects = app.handle(diroxide_app::AppEvent::Key(key));
    said(runtime.run_until_idle(app, effects))
}

/// Puts the focus of the open list on the first row whose words start so.
fn focus_row(app: &mut diroxide_app::App, start: &str) -> Result<()> {
    let at = match app.dialog() {
        Some((_, diroxide_app::Dialog::Choose { list, .. })) => list
            .rows()
            .iter()
            .position(|row| row.label.starts_with(start)),
        _ => None,
    };
    match at {
        Some(index) if app.focus_dialog_row(index) => Ok(()),
        _ => bail!("no row starts {start} in the open list"),
    }
}

/// The bytes of one file of the duplicates' folder: its number at the
/// start, then a pattern of that number, so no two numbers are alike.
fn dup_bytes(number: u64, size: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; size];
    for (k, byte) in bytes.iter_mut().enumerate() {
        *byte = ((k as u64)
            .wrapping_mul(131)
            .wrapping_add(number.wrapping_mul(7919))
            % 251) as u8;
    }
    let stamp = number.to_le_bytes();
    bytes[..stamp.len().min(size)].copy_from_slice(&stamp[..stamp.len().min(size)]);
    bytes
}

/// Duplicates over many files (R34's commit 41), through the app as
/// Tools, Find duplicates runs it, with its BLAKE3: from Enter on "Find
/// now" until the groups open. The files sit in ten folders; each ten in
/// a row share a size, the tenth a copy of the ninth, and one ten in
/// fifty is 256 KiB, past the quick check, so the full hash has work.
pub fn duplicates_scenario(setting: &Setting<'_>) -> Result<Outcome> {
    use diroxide_app::commands::ids;
    let count = setting.sizes.dup_files;
    let folder = scenario_folder(setting, "dups")?;
    let other = scenario_folder(setting, "dups-other")?;
    println!(
        "bench: writing {} files with copies among them.",
        grouped(count as u64)
    );
    let mut bytes = 0u64;
    let mut groups = 0u64;
    for n in 0..count as u64 {
        let ten = n / 10;
        let size = if ten % 50 == 0 {
            256 * 1024
        } else {
            4096 * (1 + (ten % 8) as usize)
        };
        let number = if n % 10 == 9 {
            groups += 1;
            n - 1
        } else {
            n
        };
        let sub = folder.join(format!("part-{:02}", n % 10));
        if n < 10 {
            fs::create_dir(&sub).with_context(|| format!("{} could not be made", sub.display()))?;
        }
        let path = sub.join(format!("photo-{n:05}.bin"));
        fs::write(&path, dup_bytes(number, size))
            .with_context(|| format!("{} could not be written", path.display()))?;
        bytes += size as u64;
    }
    println!("bench: wrote {}.", diroxide_core::format::human_size(bytes));
    let want = format!("{} group", grouped(groups));
    let mut found = String::new();
    let times = measure(setting.params.runs, |_| {
        let (mut app, mut runtime) = started_app(&folder, &other)?;
        let effects = app.dispatch(&diroxide_core::Command::new(ids::FIND_DUPLICATES));
        runtime.run_until_idle(&mut app, effects);
        focus_row(&mut app, "This folder: ")?;
        press(&mut app, &mut runtime, "enter");
        if app.dialog_purpose() != Some("DupsOptions") {
            bail!("the duplicates' options did not open");
        }
        focus_row(&mut app, "Find now")?;
        let started = Instant::now();
        let heard = press(&mut app, &mut runtime, "enter");
        let elapsed = started.elapsed();
        if app.dialog_purpose() != Some("DupsGroups") {
            bail!("the groups did not open: {heard:?}");
        }
        let Some(line) = heard.iter().find(|line| line.starts_with(&want)) else {
            bail!("no line starts {want}: {heard:?}");
        };
        found = line.clone();
        Ok(elapsed)
    })?;
    Ok(outcome(
        "duplicates",
        format!("Find duplicates among {} files", grouped(count as u64)),
        times,
        setting.params.budgets_ms.budget_for("duplicates"),
        format!("through the app with its BLAKE3; {found}"),
    ))
}

/// A tab switch at many entries (R34's commit 41): the left panel's first
/// tab on the listing's folder, its second on an empty one; Previous tab
/// shows the first again and lists its folder again, timed until the app
/// is idle. The note says how soon the switch itself was back.
pub fn tab_switch_scenario(setting: &Setting<'_>) -> Result<Outcome> {
    use diroxide_app::commands::ids;
    let count = setting.sizes.list_entries;
    let big = setting.work.join("list");
    if !big.is_dir() {
        fs::create_dir(&big).with_context(|| format!("{} could not be made", big.display()))?;
        println!(
            "bench: writing {} empty files for the tab switch.",
            grouped(count as u64)
        );
        super::sample::flat_folder(&big, count)?;
    }
    let small = scenario_folder(setting, "tabs-other")?;
    let (mut app, mut runtime) = started_app(&big, &small)?;
    let command = |id| diroxide_core::Command::new(id);
    let effects = app.dispatch(&command(ids::TAB_NEW));
    runtime.run_until_idle(&mut app, effects);
    let effects = app.open_place(Side::Left, place_of(&small)?);
    runtime.run_until_idle(&mut app, effects);
    if app.tab_count(Side::Left) != 2 || app.tab_index(Side::Left) != 1 {
        bail!("the second tab was not made");
    }
    let mut shown = Vec::new();
    let times = measure(setting.params.runs, |_| {
        let started = Instant::now();
        let effects = app.dispatch(&command(ids::TAB_PREVIOUS));
        shown.push(millis(started.elapsed()));
        runtime.run_until_idle(&mut app, effects);
        let elapsed = started.elapsed();
        let rows = app.panel(Side::Left).list().len();
        if app.tab_index(Side::Left) != 0 || rows < count {
            bail!("the first tab shows {rows} rows, not {count}");
        }
        let effects = app.dispatch(&command(ids::TAB_NEXT));
        runtime.run_until_idle(&mut app, effects);
        Ok(elapsed)
    })?;
    Ok(outcome(
        "tab-switch",
        format!("Switch to a tab of {} entries", grouped(count as u64)),
        times,
        setting.params.budgets_ms.budget_for("tab-switch"),
        format!(
            "shown in {}, then listed again until idle",
            time_words(median(&shown))
        ),
    ))
}

/// Builds the tree's own process plugin, `diroxide-fake-plugin`, with
/// this bench's profile, and returns its path in the target folder.
fn fake_plugin_binary(root: &Path) -> Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Process::new(cargo);
    command.current_dir(root).args([
        "build",
        "--quiet",
        "--package",
        "diroxide-plugins",
        "--features",
        "fake-plugin",
        "--bin",
        "diroxide-fake-plugin",
    ]);
    if OPTIMIZED {
        command.arg("--release");
    }
    let status = command
        .status()
        .context("cargo could not be started to build the fake plugin")?;
    if !status.success() {
        bail!("the fake plugin could not be built: cargo ended with {status}");
    }
    let profile = if OPTIMIZED { "release" } else { "debug" };
    let path = target_folder(root).join(profile).join(format!(
        "diroxide-fake-plugin{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !path.is_file() {
        bail!(
            "the fake plugin was built, but {} is not there",
            path.display()
        );
    }
    Ok(path)
}

/// The Lua gate's number (R34's commit 44, question 128): one batched
/// `column/format` request for many rows to a process plugin, timed from
/// the ask to the answer, the plugin already running. The note says the
/// cold start (the process, the handshake and the first batch) and the
/// app's own way, batches of at most 500 through the host's column call.
pub fn plugin_column_scenario(setting: &Setting<'_>) -> Result<Outcome> {
    use diroxide_plugins::protocol::methods::COLUMN_FORMAT;
    use diroxide_plugins::{Host, Manifest, PluginInfo};
    let rows = setting.sizes.plugin_rows;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("the workspace root could not be found")?;
    println!("bench: building the tree's own process plugin.");
    let exe = fake_plugin_binary(root)?;
    let exe_text = exe
        .to_str()
        .with_context(|| format!("{} is not valid Unicode", exe.display()))?;
    let folder = scenario_folder(setting, "plugin")?;
    let text = format!(
        "[plugin]\nid = \"bench\"\nname = \"Bench plugin\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[run]\ncommand = ['{exe_text}', 'echo']\nstartup_timeout = \"10s\"\n\n[[command]]\nid = \"echo\"\nname = \"Echo\"\n\n[[column]]\nid = \"state\"\nheader = \"State\"\nkind = \"text\"\n"
    );
    let report = Manifest::check(&text);
    let problems = report.text();
    let manifest = report
        .manifest
        .with_context(|| format!("the bench plugin's manifest does not read: {problems}"))?;
    let settings = diroxide_app::config::Settings::default();
    let mut host = Host::new(&settings.plugins)
        .with_state_dir(folder.join("state"))
        .with_registry(Arc::new(Registry::with_local()));
    host.add(PluginInfo {
        manifest,
        folder: folder.clone(),
        enabled: true,
    });
    let base = place_of(&folder)?;
    let places: Vec<Place> = (1..=rows)
        .map(|n| {
            base.join(&format!("entry-{n:06}.txt"))
                .map_err(|e| anyhow::anyhow!("a place could not be joined: {e}"))
        })
        .collect::<Result<_>>()?;
    let texts: Vec<String> = places.iter().map(ToString::to_string).collect();
    let params =
        serde_json::json!({"column": "state", "folder": base.to_string(), "places": texts});
    // The fake answers every place but the last, which stays pending.
    let answered = |answer: &serde_json::Value| {
        answer
            .get("values")
            .and_then(serde_json::Value::as_object)
            .map_or(0, serde_json::Map::len)
    };
    let cold_start = Instant::now();
    let first = host
        .call("bench", COLUMN_FORMAT, params.clone())
        .map_err(|e| anyhow::anyhow!("the plugin did not answer: {e}"))?;
    let cold = millis(cold_start.elapsed());
    if answered(&first) + 1 != rows {
        bail!("the plugin answered {} of {rows} rows", answered(&first));
    }
    let times = measure(setting.params.runs, |_| {
        let started = Instant::now();
        let answer = host
            .call("bench", COLUMN_FORMAT, params.clone())
            .map_err(|e| anyhow::anyhow!("the plugin did not answer: {e}"))?;
        let elapsed = started.elapsed();
        if answered(&answer) + 1 != rows {
            bail!("the plugin answered {} of {rows} rows", answered(&answer));
        }
        Ok(elapsed)
    })?;
    let batch = diroxide_plugins::listers::MAX_COLUMN_BATCH;
    let batched = measure(setting.params.runs, |_| {
        let started = Instant::now();
        let mut cells = 0;
        for chunk in places.chunks(batch) {
            cells += host
                .column_values("bench", "state", &base, chunk)
                .map_err(|e| anyhow::anyhow!("the plugin did not answer: {e}"))?
                .len();
        }
        let elapsed = started.elapsed();
        if cells != rows {
            bail!("the column calls gave {cells} cells, not {rows}");
        }
        Ok(elapsed)
    })?;
    host.shutdown_all(diroxide_plugins::host::QUIT_BUDGET);
    Ok(outcome(
        "plugin-column",
        format!("One column request for {} rows", grouped(rows as u64)),
        times,
        setting.params.budgets_ms.budget_for("plugin-column"),
        format!(
            "to a running process plugin; cold start {}; in batches of {batch} {}",
            time_words(cold),
            time_words(median(&batched))
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_bytes_differ_by_number_and_repeat_for_one() {
        assert_eq!(dup_bytes(3, 64), dup_bytes(3, 64));
        assert_ne!(dup_bytes(3, 64), dup_bytes(4, 64));
        assert_eq!(dup_bytes(5, 4).len(), 4);
        assert_eq!(&dup_bytes(258, 4096)[..8], &258u64.to_le_bytes());
    }
}
