//! `diro watch --task NAME [--once] [--dry-run]` and `diro watch
//! --list`: a task's hot folder run by the engine, `--once` the catch-up
//! scan and the queue, then the end, so a scheduler gets catch-up for
//! free; `--dry-run` writes nothing, not even the ledger. `--rule` is
//! another name for `--task`. Exit codes, the family's: 0 all done, 1 the
//! task could not run, 2 wrong use, 3 partly done (alpha.7, R28 6.3;
//! the corrected plan 5.17; R30 commit 69, lane AW). `diro watch PATH`
//! without a task stays the say-each-file mode of `watch.rs`.
//!
//! - `--list`: one line a task, the name first, as the Watched folders
//!   list says it ("Scans, on, 3 done today, 1 failed"); then each
//!   task that cannot be used, its problem first.
//! - `--dry-run`: one line a file in `in`, what the task would do or why
//!   not, then "Dry run: 3 would convert, 1 skipped".
//! - `--once`: one line a file ("Done: scan-004.pdf, 3 seconds"), then
//!   the summary ("Scans: 3 done, 1 failed"); progress words go to
//!   standard error.
//! - With neither, the task runs until Ctrl+C, its polite summaries at
//!   most every ten seconds.
//!
//! A task the owner has not approved, as it is now, asks in a terminal
//! (starting on No) and, with nobody there, says so and does not run.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diroxide_app::watched::{AppTaskActions, load_tasks, runner_for, store_of, today_tally};
use diroxide_config::Paths;
use diroxide_core::Clock;
use diroxide_filetype::TypeRegistry;
use diroxide_ops::Engine;
use diroxide_ops::convert::CartaRunner;
use diroxide_ops::hot::lines::list_row;
use diroxide_ops::hot::{CatchUpChoice, HotNote, StartError, TaskRunner, run_once, spawn_task};
use diroxide_tools::ToolRegistry;
use diroxide_vfs::ToolRunner;

use super::archive::{Asker, NOT_ASKED};
use crate::CliError;

/// The summaries' pace: at most one line this often.
const PACE: Duration = Duration::from_secs(10);

/// `diro watch --task`'s options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchTaskArgs {
    /// The task's name, its file's stem in the tasks folder.
    pub task: Option<String>,
    /// Catch up, run the queue, then end.
    pub once: bool,
    /// Say what would happen and touch nothing.
    pub dry_run: bool,
    /// List the tasks, one a line, the name first.
    pub list: bool,
}

/// Whether `watch`'s arguments ask for a task rather than the
/// say-each-file mode: the route the command line takes.
pub fn wants_task(args: &[String]) -> bool {
    args.iter().any(|a| {
        let name = a.split_once('=').map_or(a.as_str(), |(n, _)| n);
        matches!(name, "--task" | "--rule" | "--list")
    })
}

/// Reads the arguments after `watch` when they name a task.
pub fn parse(args: &[String]) -> Result<WatchTaskArgs, CliError> {
    let mut parsed = WatchTaskArgs::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--task" | "--rule" => {
                let value = inline
                    .or_else(|| rest.next().cloned())
                    .ok_or_else(|| CliError::Missing(name.to_owned()))?;
                if parsed.task.replace(value).is_some() {
                    return Err(CliError::TooMany(name.to_owned()));
                }
            }
            "--once" => parsed.once = true,
            "--dry-run" => parsed.dry_run = true,
            "--list" => parsed.list = true,
            option if option.starts_with('-') => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            other => return Err(CliError::TooMany(format!("watch {other}"))),
        }
    }
    if parsed.task.is_none() && !parsed.list {
        return Err(CliError::Missing("watch --task".to_owned()));
    }
    Ok(parsed)
}

/// What a task runs with: the config and state folders, the helper
/// programs, the tools registry for versions, the file types, the clock,
/// where carta runs, whom to ask, and how to wait.
pub struct TaskEnv<'a> {
    /// The config and state folders.
    pub paths: Paths,
    /// The helper programs.
    pub tools: Arc<dyn ToolRunner>,
    /// The tools registry, for the versions an approval names; none in
    /// tests, so no version command runs.
    pub versions: Option<ToolRegistry>,
    /// The file types.
    pub types: Arc<TypeRegistry>,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// Where carta runs: a `diro` child in the program.
    pub carta: CartaRunner,
    /// Whom to ask the approval and catch-up questions.
    pub asker: &'a dyn Asker,
    /// The wait while files settle: the real one, or a step of a fixed
    /// clock in tests.
    pub sleep: &'a mut dyn FnMut(Duration),
    /// Set by Ctrl+C: the running task ends.
    pub stop: Arc<AtomicBool>,
}

/// Runs `diro watch --task` on this computer.
pub fn run(args: &WatchTaskArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let paths = crate::open::paths(false)
        .ok_or_else(|| CliError::Failed("Could not run the task: no config folder.".to_owned()))?;
    let config = diroxide_config::load(&paths);
    let types = TypeRegistry::builtin()
        .with_config(&config.filetypes)
        .with_retro(config.settings.filetypes.retro);
    let registry = super::archive::tools_here();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let _ = ctrlc::set_handler(move || stopping.store(true, Ordering::SeqCst));
    let asker = super::archive::TerminalAsker::if_present(true);
    let mut sleep = |wait: Duration| std::thread::sleep(wait.min(Duration::from_secs(1)));
    let mut env = TaskEnv {
        paths,
        tools: Arc::new(registry.clone()),
        versions: Some(registry),
        types: Arc::new(types),
        clock: Arc::new(diroxide_app::startup::local_clock()),
        carta: std::env::current_exe()
            .map(CartaRunner::child)
            .unwrap_or_default(),
        asker: asker.as_ref(),
        sleep: &mut sleep,
        stop,
    };
    run_with(args, out, &mut env)
}

/// Writes one line, or fails in words.
fn line(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    writeln!(out, "{text}").map_err(|e| CliError::Failed(e.to_string()))
}

/// `--list`: one line a task, then the problems.
fn list(out: &mut dyn Write, env: &TaskEnv<'_>) -> Result<i32, CliError> {
    let (tasks, problems) = load_tasks(&env.paths);
    if tasks.is_empty() && problems.is_empty() {
        let folder = env.paths.config_dir.join(Paths::TASKS);
        line(
            out,
            &format!("No tasks. Each is a file in {}", folder.display()),
        )?;
        return Ok(0);
    }
    for (task, _) in &tasks {
        let state = store_of(&env.paths, &task.name)
            .map(|store| store.state())
            .unwrap_or_default();
        let (done, failed) = today_tally(&env.paths, &task.name, env.clock.as_ref());
        line(
            out,
            &list_row(
                &task.title(),
                state.on,
                state.paused.is_some(),
                done,
                failed,
            ),
        )?;
    }
    for problem in &problems {
        line(out, problem)?;
    }
    Ok(0)
}

/// The task's runner, or why there is none.
fn runner(name: &str, env: &TaskEnv<'_>) -> Result<TaskRunner, String> {
    let (tasks, problems) = load_tasks(&env.paths);
    let Some((task, _)) = tasks.into_iter().find(|(t, _)| t.name == name) else {
        let prefix = format!("Task {name}:");
        return Err(problems
            .into_iter()
            .find(|p| p.starts_with(&prefix))
            .unwrap_or_else(|| format!("Could not run {name}: no task is named {name}.")));
    };
    let (registry, _) = super::archive::archive_registry();
    let engine = Engine::new(registry).with_tools(Arc::clone(&env.tools));
    let actions = Arc::new(AppTaskActions::new(
        Some(&env.paths),
        Arc::clone(&env.types),
        env.versions.clone(),
        Arc::clone(&env.clock),
    ));
    runner_for(
        task,
        engine,
        actions,
        &env.paths,
        Arc::clone(&env.clock),
        env.carta.clone(),
        PACE,
    )
}

/// Asks the approval question when the task needs it: yes records it;
/// with nobody there it is said and answered No.
fn approved(runner: &TaskRunner, env: &TaskEnv<'_>, out: &mut dyn Write) -> Result<bool, CliError> {
    let (approval, key) = match runner.approval() {
        Ok(found) => found,
        Err(error) => {
            line(out, &error.to_string())?;
            return Ok(false);
        }
    };
    let Some(question) = approval.question(runner.task()) else {
        return Ok(true);
    };
    line(out, &diroxide_ops::hot::sentence(runner.task()))?;
    if !env.asker.present() {
        line(out, &question)?;
        line(out, NOT_ASKED)?;
        return Ok(false);
    }
    if !env.asker.confirm(&question) {
        line(out, "Not run: the task is not approved.")?;
        return Ok(false);
    }
    runner
        .store()
        .approve(&key, env.clock.now())
        .map_err(|e| CliError::Failed(format!("Could not approve {}: {e}.", runner.title())))?;
    Ok(true)
}

/// Runs `diro watch --task` with these surroundings. See the module
/// documentation.
pub fn run_with(
    args: &WatchTaskArgs,
    out: &mut dyn Write,
    env: &mut TaskEnv<'_>,
) -> Result<i32, CliError> {
    if args.list {
        return list(out, env);
    }
    let Some(name) = args.task.clone() else {
        return Err(CliError::Missing("watch --task".to_owned()));
    };
    let runner = match runner(&name, env) {
        Ok(runner) => runner,
        Err(why) => {
            line(out, &why)?;
            return Ok(1);
        }
    };
    if args.dry_run {
        return match runner.preview() {
            Ok(dry) => {
                for row in &dry.lines {
                    line(out, row)?;
                }
                line(out, &dry.summary)?;
                Ok(0)
            }
            Err(why) => {
                line(
                    out,
                    &format!("Could not preview {}: {why}.", runner.title()),
                )?;
                Ok(1)
            }
        };
    }
    if !approved(&runner, env, out)? {
        return Ok(1);
    }
    if args.once {
        let clock = Arc::clone(&env.clock);
        let report = run_once(runner, clock.as_ref(), env.sleep);
        for row in &report.lines {
            line(out, row)?;
        }
        line(out, &report.summary())?;
        return Ok(report.exit_code());
    }
    watch(runner, out, env)
}

/// The task running until Ctrl+C: its polite lines as they come, then
/// the totals.
fn watch(runner: TaskRunner, out: &mut dyn Write, env: &mut TaskEnv<'_>) -> Result<i32, CliError> {
    let title = runner.title().to_owned();
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    let sink = move |note: HotNote| {
        let said = match note {
            HotNote::Say(line) | HotNote::Paused(line) => Some(line),
            HotNote::Resumed => Some("Resumed".to_owned()),
            _ => None,
        };
        if let Some(said) = said {
            let _ = sender.send(said);
        }
    };
    let mut runner = Some(runner);
    let run = loop {
        let Some(this) = runner.take() else {
            return Ok(1);
        };
        let store = this.store().clone();
        match spawn_task(this, Arc::clone(&env.clock), Box::new(sink.clone())) {
            Ok(run) => break run,
            Err(StartError::AskCatchUp { question, .. }) => {
                let choice = if env.asker.present() && env.asker.confirm(&question) {
                    CatchUpChoice::All
                } else {
                    if !env.asker.present() {
                        line(out, &question)?;
                        line(out, NOT_ASKED)?;
                    }
                    CatchUpChoice::None
                };
                store
                    .remember_catch_up(choice)
                    .map_err(|e| CliError::Failed(format!("Could not start {title}: {e}.")))?;
                runner = Some(self::runner(&store_name(&store), env).map_err(CliError::Failed)?);
            }
            Err(error) => {
                line(out, &error.to_string())?;
                return Ok(1);
            }
        }
    };
    line(out, &format!("Watching {title}. Ctrl+C stops."))?;
    while !env.stop.load(Ordering::SeqCst) && !run.has_ended() {
        while let Ok(said) = receiver.try_recv() {
            line(out, &said)?;
        }
        (env.sleep)(Duration::from_millis(200));
    }
    run.stop();
    drop(run);
    while let Ok(said) = receiver.try_recv() {
        line(out, &said)?;
    }
    line(out, &format!("Stopped watching {title}."))?;
    Ok(0)
}

/// The task's name, as its store keeps it.
fn store_name(store: &diroxide_ops::hot::TaskStore) -> String {
    store.name().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{FixedClock, Timestamp, UtcOffset};

    /// Answers every question with the one answer it was made with.
    struct Answers(bool);

    impl Asker for Answers {
        fn present(&self) -> bool {
            true
        }
        fn confirm(&self, _question: &str) -> bool {
            self.0
        }
        fn secret(&self, _prompt: &str) -> Option<diroxide_core::Secret> {
            None
        }
        fn tell(&self, _line: &str) {}
    }

    /// A home with `in` holding two Markdown files, and a task that
    /// converts each to Org with carta, in process, moving the originals
    /// to `processed`. Everything is in the test's own folder.
    fn home() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::Builder::new().prefix("task-").tempdir().unwrap();
        let home = dir.path().join("notes");
        for folder in ["in", "out", "processed", "failed"] {
            std::fs::create_dir_all(home.join(folder)).unwrap();
        }
        std::fs::write(home.join("in").join("a.md"), "# A\n\nOne.\n").unwrap();
        std::fs::write(home.join("in").join("b.md"), "# B\n\nTwo.\n").unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let tasks = paths.config_dir.join(Paths::TASKS);
        std::fs::create_dir_all(&tasks).unwrap();
        let task = format!(
            "name = \"notes\"\nkind = \"convert\"\nrecipe = \"markdown-to-org\"\n\n[trigger]\nkind = \"folder\"\nhome = '{}'\nmatch = [\"*.md\"]\nsettle = \"2s\"\n",
            home.display()
        );
        std::fs::write(tasks.join("notes.toml"), task).unwrap();
        (dir, paths)
    }

    fn run_in(paths: &Paths, args: &WatchTaskArgs, yes: bool) -> (i32, String) {
        let clock = Arc::new(FixedClock::new(
            Timestamp::from_civil(2026, 11, 7, 9, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        ));
        let ticking = Arc::clone(&clock);
        let mut sleep = move |wait: Duration| {
            ticking.advance(i64::try_from(wait.as_secs().max(1)).unwrap_or(1));
        };
        let asker = Answers(yes);
        let mut env = TaskEnv {
            paths: paths.clone(),
            tools: Arc::new(diroxide_vfs::NoTools),
            versions: None,
            types: Arc::new(TypeRegistry::builtin()),
            clock,
            carta: CartaRunner::InProcess,
            asker: &asker,
            sleep: &mut sleep,
            stop: Arc::new(AtomicBool::new(true)),
        };
        let mut out = Vec::new();
        let code = run_with(args, &mut out, &mut env).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_options_are_read() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(wants_task(&args(&["--task", "scans"])));
        assert!(!wants_task(&args(&["D:\\scans"])));
        assert_eq!(
            parse(&args(&["--rule=scans", "--once"])),
            Ok(WatchTaskArgs {
                task: Some("scans".into()),
                once: true,
                ..WatchTaskArgs::default()
            })
        );
        assert_eq!(
            parse(&args(&["--once"])),
            Err(CliError::Missing("watch --task".into()))
        );
    }

    #[test]
    fn list_and_dry_run_say_the_task_and_touch_nothing() {
        let (dir, paths) = home();
        let (code, text) = run_in(
            &paths,
            &WatchTaskArgs {
                list: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(code, 0);
        assert_eq!(text, "Notes, off\n");
        let args = WatchTaskArgs {
            task: Some("notes".into()),
            dry_run: true,
            ..Default::default()
        };
        let (code, text) = run_in(&paths, &args, false);
        assert_eq!(code, 0, "{text}");
        assert_eq!(
            text,
            "Would convert a.md\nWould convert b.md\nDry run: 2 would convert, 0 skipped\n"
        );
        assert!(dir.path().join("notes").join("in").join("a.md").exists());
        for row in text.lines() {
            assert!(row.chars().count() <= 40, "{row}");
        }
    }

    #[test]
    fn once_asks_the_approval_converts_and_says_the_family_s_code() {
        let (dir, paths) = home();
        let args = WatchTaskArgs {
            task: Some("notes".into()),
            once: true,
            ..Default::default()
        };
        // No: nothing runs, exit 1.
        let (code, text) = run_in(&paths, &args, false);
        assert_eq!(code, 1, "{text}");
        assert!(text.starts_with("Notes. Watches "), "{text}");
        assert!(
            text.ends_with("Not run: the task is not approved.\n"),
            "{text}"
        );
        let home = dir.path().join("notes");
        assert!(home.join("in").join("a.md").exists());
        // Yes: both converted, the originals moved, exit 0.
        let (code, text) = run_in(&paths, &args, true);
        assert_eq!(code, 0, "{text}");
        assert!(text.ends_with("Notes: 2 done\n"), "{text}");
        assert!(home.join("out").join("a.org").exists());
        assert!(home.join("processed").join("b.md").exists());
        assert!(!home.join("in").join("a.md").exists());
        // Approved now: the list says the tally of today.
        let (_, text) = run_in(
            &paths,
            &WatchTaskArgs {
                list: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(text, "Notes, off, 2 done today\n");
    }

    #[test]
    fn an_unknown_task_could_not_run() {
        let (_dir, paths) = home();
        let args = WatchTaskArgs {
            task: Some("nope".into()),
            once: true,
            ..Default::default()
        };
        let (code, text) = run_in(&paths, &args, true);
        assert_eq!(code, 1);
        assert_eq!(text, "Could not run nope: no task is named nope.\n");
    }
}
