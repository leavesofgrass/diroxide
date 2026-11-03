//! The GUI's budgets (R27's commit 35), measured by
//! `cargo xtask bench --gui` and recorded beside the scenarios, never
//! gated:
//!
//! - `binary-size`: the size of the `diro` program the bench built (the
//!   release profile for an optimized bench, which is the one to record;
//!   the packages' `dist` profile is smaller still).
//! - `gui-start`: from the start of `diro --gui --background` to the
//!   window's first frame, which the window says in its `--log` file
//!   ("graphics: drawing with ...") at the first step after the frame is
//!   drawn; the median of the bench's runs. Each run is one real window,
//!   off screen, never activated and with no taskbar button, started with
//!   no console, on a copy of the program in the bench's work folder with
//!   `--portable`, so its settings and state are the run's own; the
//!   program is ended once the frame is there.
//! - `gui-working-set`: the program's peak working set a second after its
//!   first frame, the median of the runs (Windows: `PeakWorkingSet64`
//!   through PowerShell; Linux: `VmHWM`; macOS: the resident size from
//!   `ps`).
//! - `cold-build`, with `--cold-build` only: `cargo build --release -p
//!   diro` into a target folder of its own, with no compiler cache. The
//!   folder must not exist yet, so the build is cold; it is left for the
//!   person to remove with `.claude/tools/safe-remove.ps1`, since the
//!   bench removes only its own work folder.
//!
//! Where there is no display (Linux with neither `WAYLAND_DISPLAY` nor
//! `DISPLAY`), the two window budgets are skipped, and said.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::bench::{diro_binary, median, target_folder, time_words};

/// One GUI budget's measure.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Budget {
    /// Its id: `binary-size`, `gui-start`, `gui-working-set` or
    /// `cold-build`.
    pub id: String,
    /// What it measures, in words.
    pub label: String,
    /// The measure, the median of the runs, in [`Budget::unit`].
    pub value: f64,
    /// `ms` or `MB`.
    pub unit: String,
    /// Each run's measure.
    pub runs: Vec<f64>,
    /// One more fact in words.
    pub note: String,
}

/// The line the window writes in its `--log` file at its first frame.
pub const FIRST_FRAME: &str = "graphics: drawing with ";

/// How long a start may take before the run fails.
const START_LIMIT: Duration = Duration::from_secs(60);

/// A size in megabytes (10^6 bytes), to a tenth.
fn megabytes(bytes: u64) -> f64 {
    (bytes as f64 / 100_000.0).round() / 10.0
}

/// A measure in words: a time (minutes and seconds from a minute on,
/// as a cold build takes), or a size in megabytes.
pub fn value_words(value: f64, unit: &str) -> String {
    match unit {
        "ms" if value >= 60_000.0 => {
            let seconds = (value / 1000.0).round() as u64;
            format!("{} min {} s", seconds / 60, seconds % 60)
        }
        "ms" => time_words(value),
        _ => format!("{value:.1} {unit}"),
    }
}

/// Whether a window can open here: Windows and macOS always; elsewhere a
/// Wayland or X11 display.
fn display_present() -> bool {
    cfg!(any(windows, target_os = "macos"))
        || ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// A command with no console window of its own on Windows.
fn quiet(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// The process's peak working set in bytes, or `None` where it cannot be
/// read.
fn peak_working_set(pid: u32) -> Option<u64> {
    if cfg!(windows) {
        let out = quiet("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else if cfg!(target_os = "linux") {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map(|kb| kb * 1024)
    } else {
        let out = quiet("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .ok()
            .map(|kb| kb * 1024)
    }
}

/// One start of the window: the time to its first frame and its peak
/// working set then.
fn one_start(program: &Path, folder: &Path, log: &Path) -> Result<(Duration, Option<u64>)> {
    let started = Instant::now();
    let mut child = quiet(program)
        .current_dir(folder)
        .arg("--gui")
        .arg("--background")
        .arg("--portable")
        .arg("--log")
        .arg(log)
        .arg(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("{} could not be started", program.display()))?;
    let first = loop {
        if fs::read_to_string(log)
            .is_ok_and(|text| text.lines().any(|l| l.starts_with(FIRST_FRAME)))
        {
            break Some(started.elapsed());
        }
        if child.try_wait().ok().flatten().is_some() || started.elapsed() > START_LIMIT {
            break None;
        }
        sleep(Duration::from_millis(5));
    };
    let working_set = first.and_then(|_| {
        sleep(Duration::from_secs(1));
        peak_working_set(child.id())
    });
    // The run's own window: ended once it is measured.
    let _ = child.kill();
    let _ = child.wait();
    match first {
        Some(elapsed) => Ok((elapsed, working_set)),
        None => bail!(
            "the window drew no first frame within {} seconds; its log: {}",
            START_LIMIT.as_secs(),
            fs::read_to_string(log).unwrap_or_default().trim()
        ),
    }
}

/// Measures the GUI's budgets, saying what it does as it goes. `work` is
/// the bench's own work folder, removed after the run.
pub fn measure(root: &Path, work: &Path, runs: usize, cold_build: bool) -> Result<Vec<Budget>> {
    let mut budgets = Vec::new();
    println!("bench: the GUI's budgets, recorded and never gated.");
    let diro = diro_binary(root)?;
    let size = fs::metadata(&diro)
        .with_context(|| format!("{} could not be read", diro.display()))?
        .len();
    let profile = if crate::bench::OPTIMIZED {
        "release"
    } else {
        "debug"
    };
    budgets.push(Budget {
        id: "binary-size".to_owned(),
        label: "Binary size".to_owned(),
        value: megabytes(size),
        unit: "MB".to_owned(),
        runs: vec![megabytes(size)],
        note: format!("the {profile} build of diro, {size} bytes"),
    });

    if display_present() {
        let gui = work.join("gui");
        let folder = gui.join("sample");
        fs::create_dir_all(folder.join("Backup"))
            .with_context(|| format!("{} could not be made", folder.display()))?;
        for name in ["notes.txt", "report.pdf", "budget.xlsx"] {
            fs::write(folder.join(name), "diroxide\n")
                .with_context(|| format!("{name} could not be written"))?;
        }
        let program = gui.join(diro.file_name().context("the program has no name")?);
        fs::copy(&diro, &program)
            .with_context(|| format!("{} could not be copied", diro.display()))?;
        println!(
            "bench: {} starts of diro --gui --background, each one window off screen, never activated.",
            runs.max(1)
        );
        let mut starts = Vec::new();
        let mut sets = Vec::new();
        // A log of its own for each run, so no run reads another's.
        let log = |run: usize| gui.join(format!("start-{run}.log"));
        for run in 0..runs.max(1) {
            let (elapsed, set) = one_start(&program, &folder, &log(run))?;
            starts.push((elapsed.as_secs_f64() * 10_000.0).round() / 10.0);
            if let Some(set) = set {
                sets.push(megabytes(set));
            }
        }
        let road = fs::read_to_string(log(0))
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|l| l.strip_prefix(FIRST_FRAME))
                    .and_then(|rest| rest.split(';').next())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "a road the log did not name".to_owned());
        budgets.push(Budget {
            id: "gui-start".to_owned(),
            label: "Start to the first frame".to_owned(),
            value: median(&starts),
            unit: "ms".to_owned(),
            runs: starts,
            note: format!("started with --gui --background, drawing with {road}"),
        });
        if sets.is_empty() {
            println!("bench: the working set could not be read on this system.");
        } else {
            budgets.push(Budget {
                id: "gui-working-set".to_owned(),
                label: "Working set at the first frame".to_owned(),
                value: median(&sets),
                unit: "MB".to_owned(),
                runs: sets,
                note: "the peak, a second after the first frame".to_owned(),
            });
        }
    } else {
        println!(
            "bench: no display (no WAYLAND_DISPLAY or DISPLAY): the start and the working set are skipped."
        );
    }

    if cold_build {
        budgets.push(cold(root)?);
    }
    Ok(budgets)
}

/// The cold build: `cargo build --release -p diro` into a target folder
/// that does not exist yet, with no compiler cache.
fn cold(root: &Path) -> Result<Budget> {
    let folder: PathBuf = target_folder(root).join("bench").join("cold-build");
    if folder.exists() {
        bail!(
            "{} exists, so a build there is not cold; remove it with .claude/tools/safe-remove.ps1 after a dry run, then run the bench again",
            folder.display()
        );
    }
    println!(
        "bench: a cold release build of diro in {}, minutes of work.",
        folder.display()
    );
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let started = Instant::now();
    let status = quiet(cargo)
        .current_dir(root)
        .args([
            "build",
            "--quiet",
            "--release",
            "--package",
            "diro",
            "--bin",
            "diro",
        ])
        .env("CARGO_TARGET_DIR", &folder)
        .env("RUSTC_WRAPPER", "")
        .env("CARGO_INCREMENTAL", "0")
        .status()
        .context("cargo could not be started for the cold build")?;
    let elapsed = started.elapsed();
    if !status.success() {
        bail!("the cold build failed: cargo ended with {status}");
    }
    let ms = (elapsed.as_secs_f64() * 10_000.0).round() / 10.0;
    Ok(Budget {
        id: "cold-build".to_owned(),
        label: "Cold release build of diro".to_owned(),
        value: ms,
        unit: "ms".to_owned(),
        runs: vec![ms],
        note: format!(
            "no compiler cache; the build is left in {}, to be removed with safe-remove.ps1",
            folder.display()
        ),
    })
}

/// The lines that say one budget: the measure first, then the runs and
/// the note, then the baseline, which is said and never gates.
pub fn describe(budget: &Budget, baseline: Option<&Budget>) -> Vec<String> {
    let mut lines = vec![format!(
        "{}: {}.",
        budget.label,
        value_words(budget.value, &budget.unit)
    )];
    let mut second = String::new();
    if budget.runs.len() > 1 {
        let runs: Vec<String> = budget
            .runs
            .iter()
            .map(|v| value_words(*v, &budget.unit))
            .collect();
        second = format!("Runs: {}.", runs.join(", "));
    }
    if !budget.note.is_empty() {
        let mut chars = budget.note.chars();
        if let Some(c) = chars.next() {
            if !second.is_empty() {
                second.push(' ');
            }
            second.extend(c.to_uppercase());
            second.push_str(chars.as_str());
            second.push('.');
        }
    }
    if !second.is_empty() {
        lines.push(second);
    }
    if let Some(base) = baseline.filter(|b| b.value > 0.0 && b.unit == budget.unit) {
        let change = (budget.value - base.value) / base.value * 100.0;
        let rounded = change.abs().round();
        let word = if budget.unit == "ms" {
            if change > 0.0 { "slower" } else { "faster" }
        } else if change > 0.0 {
            "larger"
        } else {
            "smaller"
        };
        let words = if rounded < 1.0 {
            "the same".to_owned()
        } else {
            format!("{rounded:.0} percent {word}")
        };
        lines.push(format!(
            "Baseline {}: {words}; recorded, not gated.",
            value_words(base.value, &base.unit)
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(id: &str, value: f64, unit: &str, runs: Vec<f64>, note: &str) -> Budget {
        Budget {
            id: id.to_owned(),
            label: match id {
                "gui-start" => "Start to the first frame",
                _ => "Binary size",
            }
            .to_owned(),
            value,
            unit: unit.to_owned(),
            runs,
            note: note.to_owned(),
        }
    }

    #[test]
    fn sizes_read_in_megabytes() {
        assert_eq!(megabytes(38_249_984), 38.2);
        assert_eq!(value_words(38.2, "MB"), "38.2 MB");
        assert_eq!(value_words(412.0, "ms"), "412 ms");
        assert_eq!(value_words(305_800.0, "ms"), "5 min 6 s");
    }

    #[test]
    fn a_budget_says_its_measure_first_and_its_baseline_without_a_gate() {
        let now = budget(
            "gui-start",
            412.0,
            "ms",
            vec![430.0, 412.0, 401.0],
            "started with --gui --background",
        );
        let base = budget("gui-start", 400.0, "ms", vec![400.0], "");
        let lines = describe(&now, Some(&base));
        assert_eq!(lines[0], "Start to the first frame: 412 ms.");
        assert_eq!(
            lines[1],
            "Runs: 430 ms, 412 ms, 401 ms. Started with --gui --background."
        );
        assert_eq!(
            lines[2],
            "Baseline 400 ms: 3 percent slower; recorded, not gated."
        );
        let size = budget("binary-size", 40.0, "MB", vec![40.0], "the release build");
        let base = budget("binary-size", 50.0, "MB", vec![50.0], "");
        let lines = describe(&size, Some(&base));
        assert_eq!(lines[0], "Binary size: 40.0 MB.");
        assert_eq!(lines[1], "The release build.");
        assert_eq!(
            lines[2],
            "Baseline 50.0 MB: 20 percent smaller; recorded, not gated."
        );
        // Each first line puts its meaning first, inside a Braille line.
        for line in [&describe(&now, None)[0], &describe(&size, None)[0]] {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }
}
