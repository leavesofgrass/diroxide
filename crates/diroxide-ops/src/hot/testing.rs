//! The hot-folder tests' scratch folders, fake tools and fake frontend:
//! every folder under the build's own folder, never the system's
//! temporary folder or the owner's folders; no real program runs and
//! nothing plays sound.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diroxide_core::{Entry, FixedClock, Place, Timestamp, UtcOffset};
use diroxide_vfs::{
    CancelToken, Registry, ToolOutput, ToolRequest, ToolRunError, ToolRunner, ToolStream,
};

use super::actions::{ActionResult, ActionRun, TaskActions};
use super::approval::Resolved;
use super::refuse::Surroundings;
use super::runner::{HotNote, RunnerOptions, TaskRunner};
use super::store::TaskStore;
use super::task::Task;
use crate::convert::book::read_toml;
use crate::convert::recipe::{Recipe, RecipeSource};
use crate::engine::Engine;
use crate::tracker::later_by;

/// A scratch folder under the build's folder, removed when dropped.
pub(crate) fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe.parent().unwrap().parent().unwrap().join("hot-unit-tmp");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("hot-")
        .tempdir_in(&base)
        .unwrap()
}

/// A local place for a path.
pub(crate) fn place(path: &Path) -> Place {
    Place::parse(&path.to_string_lossy()).unwrap()
}

/// What the fake tool does with an input, chosen by the input's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// Writes "made from " and the input's bytes.
    Write,
    /// Writes half an output, then ends with code 1 and these words on
    /// standard error.
    Fail(&'static str),
    /// Says the time limit passed.
    TimedOut,
    /// Writes half an output, then waits until cancelled.
    Hang,
    /// Appends to the input while it "converts" (a producer still
    /// writing), then writes the output.
    Grow,
    /// Replaces the input with another file of the same size and a new
    /// identity while it converts.
    Replace,
}

/// A tools registry stand-in with one tool, `pdftotext`.
pub(crate) struct FakeTools {
    pub has: Mutex<bool>,
    pub acts: Mutex<HashMap<String, Act>>,
    pub runs: Mutex<Vec<String>>,
    /// The version `pdftotext` reports through the runner, when set.
    pub version: Mutex<Option<String>>,
}

impl FakeTools {
    pub(crate) fn new() -> Self {
        Self {
            has: Mutex::new(true),
            acts: Mutex::new(HashMap::new()),
            runs: Mutex::new(Vec::new()),
            version: Mutex::new(None),
        }
    }

    /// The fake does `act` for the input named `name`.
    pub(crate) fn on(&self, name: &str, act: Act) {
        self.acts.lock().unwrap().insert(name.to_owned(), act);
    }

    /// The inputs' names, one per run.
    pub(crate) fn ran(&self) -> Vec<String> {
        self.runs.lock().unwrap().clone()
    }

    fn answer(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolRunError> {
        if !*self.has.lock().unwrap() {
            return Err(ToolRunError::NotInstalled {
                tool: request.tool.clone(),
                name: request.tool.clone(),
            });
        }
        let args: Vec<String> = request
            .args
            .iter()
            .map(|a: &OsString| a.to_string_lossy().into_owned())
            .collect();
        let output = PathBuf::from(
            args.iter()
                .find(|a| a.contains(".diro-part-"))
                .cloned()
                .unwrap_or_default(),
        );
        let input = args
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file() && !p.to_string_lossy().contains(".diro-part-"))
            .unwrap_or_default();
        let name = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.runs.lock().unwrap().push(name.clone());
        let act = self
            .acts
            .lock()
            .unwrap()
            .get(&name)
            .cloned()
            .unwrap_or(Act::Write);
        let bytes = fs::read(&input).unwrap_or_default();
        let mut made = b"made from ".to_vec();
        made.extend_from_slice(&bytes);
        match act {
            Act::Write => {
                fs::write(&output, made).unwrap();
                Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new()))
            }
            Act::Fail(words) => {
                fs::write(&output, b"half").unwrap();
                Ok(ToolOutput::new(
                    Some(1),
                    Vec::new(),
                    words.as_bytes().to_vec(),
                ))
            }
            Act::TimedOut => {
                fs::write(&output, b"half").unwrap();
                Err(ToolRunError::TimedOut {
                    name: "pdftotext".to_owned(),
                    seconds: 600,
                })
            }
            Act::Hang => {
                fs::write(&output, b"half").unwrap();
                while !cancel.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(ToolRunError::Cancelled)
            }
            Act::Grow => {
                let mut longer = bytes.clone();
                longer.extend_from_slice(b" and more");
                fs::write(&input, longer).unwrap();
                fs::write(&output, made).unwrap();
                Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new()))
            }
            Act::Replace => {
                let other = input.with_file_name(format!("{name}.swap"));
                fs::write(&other, &bytes).unwrap();
                fs::remove_file(&input).unwrap();
                fs::rename(&other, &input).unwrap();
                fs::write(&output, made).unwrap();
                Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new()))
            }
        }
    }
}

impl ToolRunner for FakeTools {
    fn has(&self, tool: &str) -> bool {
        tool == "pdftotext" && *self.has.lock().unwrap()
    }

    fn version(&self, tool: &str) -> Option<String> {
        if tool == "pdftotext" {
            self.version.lock().unwrap().clone()
        } else {
            None
        }
    }

    fn run(&self, request: &ToolRequest, cancel: &CancelToken) -> Result<ToolOutput, ToolRunError> {
        self.answer(request, cancel)
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        let mut output = self.answer(request, cancel)?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        for line in stderr.lines() {
            let _ = record(ToolStream::Stderr, line);
        }
        output.stdout.clear();
        output.stderr.clear();
        Ok(output)
    }
}

/// The recipe the tests convert with: PDF to text through `pdftotext`.
pub(crate) fn recipe() -> Recipe {
    let (mut recipes, problems) = read_toml(
        "[[recipe]]\nid = \"pdf-to-text\"\nname = \"PDF to text\"\nfrom = [\"pdf\"]\nto = \"txt\"\ntool = \"pdftotext\"\nargs = [\"{input}\", \"{output}\"]\n",
        &RecipeSource::BuiltIn,
    );
    assert!(problems.is_empty(), "{problems:?}");
    recipes.remove(0)
}

/// A frontend stand-in: the test recipe, a filter that takes names
/// holding "keep", and script commands that do what they are told.
#[derive(Default)]
pub(crate) struct FakeActions {
    pub script: Mutex<Option<ActionResult>>,
    pub scripts: Mutex<Vec<ActionRun>>,
}

impl TaskActions for FakeActions {
    fn recipe(&self, id: &str) -> Option<Recipe> {
        (id == "pdf-to-text").then(recipe)
    }

    fn tool_version(&self, tool: &str) -> Option<String> {
        (tool == "pdftotext").then(|| "25.03.0".to_owned())
    }

    fn passes(&self, filter: &str, place: &Place, _entry: &Entry) -> Result<bool, String> {
        if filter == "unreadable" {
            return Err("it is not a filter".to_owned());
        }
        Ok(place.name().is_some_and(|n| n.contains(filter)))
    }

    fn resolve_script(&self, id: &str) -> Result<Resolved, String> {
        Ok(Resolved::Script {
            id: id.to_owned(),
            digest: "ab12".to_owned(),
        })
    }

    fn run_script(&self, run: &ActionRun, _cancel: &CancelToken) -> ActionResult {
        self.scripts.lock().unwrap().push(run.clone());
        self.script
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(ActionResult::Made(Vec::new()))
    }
}

/// A test's world: a scratch folder holding the task's home and the state
/// folder, the fake tools, a fixed clock.
pub(crate) struct World {
    pub dir: tempfile::TempDir,
    pub tools: Arc<FakeTools>,
    pub actions: Arc<FakeActions>,
    pub clock: Arc<FixedClock>,
}

impl World {
    pub(crate) fn new() -> Self {
        let dir = scratch();
        for folder in ["in", "out", "processed", "failed"] {
            fs::create_dir_all(dir.path().join("scans").join(folder)).unwrap();
        }
        Self {
            dir,
            tools: Arc::new(FakeTools::new()),
            actions: Arc::new(FakeActions::default()),
            // Friday, November 6, 2026, 8:41:07 PM UTC.
            clock: Arc::new(FixedClock::new(
                Timestamp::from_unix(1_793_997_667, 0),
                UtcOffset::UTC,
            )),
        }
    }

    /// The task's home.
    pub(crate) fn home(&self) -> PathBuf {
        self.dir.path().join("scans")
    }

    /// A folder of the task's home.
    pub(crate) fn folder(&self, name: &str) -> PathBuf {
        self.home().join(name)
    }

    /// The names in a folder of the task's home, sorted.
    pub(crate) fn names(&self, folder: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.folder(folder))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Puts a file in `in`.
    pub(crate) fn drop_in(&self, name: &str, bytes: &[u8]) {
        fs::write(self.folder("in").join(name), bytes).unwrap();
    }

    /// The task from TOML, its home filled in.
    pub(crate) fn task(&self, extra: &str) -> Task {
        let home = self.home().to_string_lossy().into_owned();
        let text = format!(
            "name = \"scans\"\nrecipe = \"pdf-to-text\"\n{extra}\n[trigger]\nhome = '{home}'\nmatch = [\"*.pdf\"]\n"
        );
        Task::from_toml(
            &text,
            "tasks/scans.toml",
            &Surroundings::none(diroxide_core::place::PathStyle::native()),
        )
        .unwrap()
    }

    /// The task's state folder.
    pub(crate) fn store(&self) -> TaskStore {
        TaskStore::new(&self.dir.path().join("state").join("hot"), "scans").unwrap()
    }

    /// A task from a whole task file's text, `{home}` standing for the
    /// task's home.
    pub(crate) fn task_from(&self, text: &str) -> Task {
        let text = text.replace("{home}", &self.home().to_string_lossy());
        Task::from_toml(
            &text,
            "tasks/scans.toml",
            &Surroundings::none(diroxide_core::place::PathStyle::native()),
        )
        .unwrap()
    }

    /// A runner over the real local provider and the fake tools, not yet
    /// approved, every number R16's but the free-space floor.
    pub(crate) fn bare_runner(&self, task: Task) -> TaskRunner {
        let engine = Engine::new(Arc::new(Registry::with_local())).with_tools(self.tools.clone());
        TaskRunner::new(task, engine, self.actions.clone(), self.store())
            .unwrap()
            .with_clock(self.clock.clone())
            .with_options(RunnerOptions {
                space_floor: 0,
                space_percent: 0,
                ..RunnerOptions::default()
            })
    }

    /// [`World::bare_runner`], the task approved as it is.
    pub(crate) fn runner(&self, task: Task) -> TaskRunner {
        let runner = self.bare_runner(task);
        let (_, key) = runner.approval().unwrap();
        runner.store().approve(&key, self.now()).unwrap();
        runner
    }

    pub(crate) fn now(&self) -> Timestamp {
        diroxide_core::Clock::now(&*self.clock)
    }

    /// Moves the clock on.
    pub(crate) fn pass(&self, by: Duration) {
        self.clock.set(later_by(self.now(), by));
    }

    /// Wakes the runner every half second for `seconds`, running the queue
    /// whenever it has work, and gathers what it said.
    pub(crate) fn run_for(&self, runner: &mut TaskRunner, seconds: u64) -> Vec<HotNote> {
        let mut notes = Vec::new();
        for _ in 0..seconds * 2 {
            notes.extend(self.settle_now(runner));
            self.pass(Duration::from_millis(500));
        }
        notes.extend(self.settle_now(runner));
        notes
    }

    /// Wakes the runner while it has work now.
    pub(crate) fn settle_now(&self, runner: &mut TaskRunner) -> Vec<HotNote> {
        let mut notes = runner.wake(self.now());
        let mut guard = 0;
        while runner.next_wake(self.now()) == Some(Duration::ZERO) && guard < 10_000 {
            notes.extend(runner.wake(self.now()));
            guard += 1;
        }
        notes
    }
}

/// The lines of the file notes, in order.
pub(crate) fn file_lines(notes: &[HotNote]) -> Vec<String> {
    notes
        .iter()
        .filter_map(|n| match n {
            HotNote::File(file) => Some(file.line.clone()),
            _ => None,
        })
        .collect()
}

/// The lines said, in order.
pub(crate) fn said(notes: &[HotNote]) -> Vec<String> {
    notes
        .iter()
        .filter_map(|n| match n {
            HotNote::Say(line) => Some(line.clone()),
            _ => None,
        })
        .collect()
}
