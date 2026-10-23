//! Running a script command (R3 sections 5.3 to 5.6, R22 sections 2.6
//! and 5): the process through the process helper (no shell, no console
//! window, a tree killed whole), the context on standard input, the
//! directives followed while it runs, the output where its mode says, a
//! cancel or a timeout in words, and the run folder removed at the end.
//!
//! - **Cancel** makes the cancel file appear (`DIRO_CANCEL_FILE`), which
//!   a script may poll; after the grace period (three seconds) the whole
//!   tree is killed. There is no console break: a child with no console
//!   window cannot receive one (R22 section 5.4).
//! - **Timeout** is the header's `timeout`, from the start; it ends the
//!   run the same way, and is said as "Count lines timed out after 30
//!   seconds."
//! - **Modes.** `output-pane` hands each output line to the sink as it
//!   comes; `run`, `output-viewer` keep it for the end; `output-column`
//!   runs once per entry and keeps each entry's first line; `lister`
//!   reads places (NUL or line separated) up to `max_rows`, then stops
//!   the program; `terminal` and `terminal-wait` share the owner's
//!   terminal; `launch` starts the program detached and answers at once.
//! - **Output** is cleaned of escape sequences and control characters and
//!   cut at 1 MiB; standard error is kept for the log, and its last line
//!   goes under a failure ("Last error: cannot read a.txt.").

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::{Message, Place};
use diroxide_tools::process::ProcessSpec;
use diroxide_tools::{ToolRegistry, ToolStatus};
use serde_json::{Map, Value};

use crate::lines;
use crate::script::command::{Program, ScriptCommand};
use crate::script::context::{
    Picked, PrepareError, Prepared, Refusal, RunContext, StateFolders, effective, prepare,
    process_spec,
};
use crate::script::directives::{Directive, DirectiveReader, parse_directive, stdout_directive};
use crate::script::interpreter::{Lookup, Missing, is_app_alias};
use crate::script::recipe::{ListForm, Per, RunMode, StdinForm};
use crate::script::words::{self, Ending, ending_lines};
use crate::services::clean_text;

/// How long a script has between the cancel file and the kill.
pub const CANCEL_GRACE: Duration = Duration::from_secs(3);

/// The most output kept, in bytes; the rest is read and dropped.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// The longest output line kept, in characters.
pub const MAX_OUTPUT_LINE_CHARS: usize = 2000;

/// How much standard error is kept for the log, in bytes.
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// How often the runner looks at the program, the directives and the
/// cancel flag.
const POLL: Duration = Duration::from_millis(25);

/// How long the runner waits for the output to close after the program
/// ended (a child it left behind may hold it open).
const DRAIN: Duration = Duration::from_secs(2);

/// The owner's cancel, shared with the runner.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Not cancelled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the run.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether the run was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Where a run's live effects go: the app's job, or a recorder in tests
/// and `diro plugin run`.
pub trait RunSink {
    /// A directive that acts at once: announce, warn, error, progress,
    /// refresh.
    fn act(&mut self, directive: &Directive);

    /// A line of output, cleaned, as it arrives (`output-pane`).
    fn output(&mut self, _line: &str) {}

    /// A line for the command's log: an ignored directive, a refusal.
    fn log(&mut self, _line: &str) {}
}

/// A sink that keeps everything, in order.
#[derive(Debug, Default)]
pub struct Recorder {
    /// The directives that acted at once, with when they came.
    pub acted: Vec<(Instant, Directive)>,
    /// The output lines handed over live.
    pub output: Vec<String>,
    /// The log lines.
    pub log: Vec<String>,
}

impl RunSink for Recorder {
    fn act(&mut self, directive: &Directive) {
        self.acted.push((Instant::now(), directive.clone()));
    }

    fn output(&mut self, line: &str) {
        self.output.push(line.to_owned());
    }

    fn log(&mut self, line: &str) {
        self.log.push(line.to_owned());
    }
}

/// What a run needs from the host besides its context.
#[derive(Clone, Debug)]
pub struct RunEnv {
    /// Where interpreters are found.
    pub lookup: Lookup,
    /// The folder of runs and the command's data folder.
    pub folders: StateFolders,
    /// The tools, with the owner's `tools.toml`.
    pub registry: ToolRegistry,
    /// Between the cancel file and the kill.
    pub grace: Duration,
    /// The owner's values of the plugin's settings, for `{setting.KEY}`.
    pub settings: Map<String, Value>,
}

impl RunEnv {
    /// The system's interpreters and tools, with these folders.
    pub fn system(folders: StateFolders) -> Self {
        Self {
            lookup: Lookup::system(),
            folders,
            registry: ToolRegistry::builtin(),
            grace: CANCEL_GRACE,
            settings: Map::new(),
        }
    }

    /// Where a tool is: the tools registry's answer for a tool it knows
    /// (the owner's `tools.toml` first), else the one PATH rule; never an
    /// app alias.
    pub fn find_tool(&self, name: &str) -> Option<PathBuf> {
        if Path::new(name).is_absolute() {
            return Path::new(name).is_file().then(|| PathBuf::from(name));
        }
        match self.registry.find(name) {
            Ok(ToolStatus::Found(path)) => Some(path),
            Ok(ToolStatus::NotFound | ToolStatus::Disabled) => None,
            Err(_) => self
                .lookup
                .which_all(name)
                .into_iter()
                .find(|p| !is_app_alias(p)),
        }
    }
}

/// Why a command did not run at all.
#[derive(Debug)]
pub enum RunError {
    /// The context does not fit the command.
    Refused(Refusal),
    /// No program runs the script.
    Interpreter(Missing),
    /// A tool the command needs is missing.
    NeedsTool(String),
    /// The command is not made for this system.
    OtherSystem,
    /// A placeholder could not be filled.
    Placeholder(String),
    /// The run folder could not be written.
    Io(String),
}

impl RunError {
    /// The sentence for a command named `title`, meaning first.
    pub fn sentence(&self, title: &str) -> String {
        match self {
            RunError::Refused(refusal) => refusal.sentence(title),
            RunError::Interpreter(missing) => missing.sentence(),
            RunError::NeedsTool(tool) => words::needs_tool(tool),
            RunError::OtherSystem => words::other_system(title),
            RunError::Placeholder(why) => lines::could_not_run(title, why),
            RunError::Io(cause) => lines::could_not_run(title, cause),
        }
    }
}

impl From<PrepareError> for RunError {
    fn from(error: PrepareError) -> Self {
        match error {
            PrepareError::Refused(refusal) => RunError::Refused(refusal),
            PrepareError::Io(e) => RunError::Io(e.to_string()),
        }
    }
}

/// What a run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunReport {
    /// How it ended.
    pub ending: Ending,
    /// The verdict lines; empty after a lister or a launch that went well.
    pub said: Vec<Message>,
    /// The directives that wait for the end, in order: apply them now.
    pub deferred: Vec<Directive>,
    /// The output lines, cleaned.
    pub output: Vec<String>,
    /// Whether the output was cut at the cap.
    pub output_cut: bool,
    /// For a lister: the places, as text.
    pub places: Vec<String>,
    /// For a lister: whether `max_rows` stopped the reading.
    pub capped: bool,
    /// For `output-column`: each entry's first output line.
    pub column: Vec<(Place, String)>,
    /// For `launch`: the program started.
    pub launched: Option<String>,
    /// For `terminal-wait`: the frontend waits for Enter.
    pub wait_for_enter: bool,
    /// How many times the program ran (per entry, or in batches).
    pub runs: usize,
    /// The last lines of standard error, for the log.
    pub stderr: Vec<String>,
}

impl RunReport {
    fn new() -> Self {
        Self {
            ending: Ending::Exited(0),
            said: Vec::new(),
            deferred: Vec::new(),
            output: Vec::new(),
            output_cut: false,
            places: Vec::new(),
            capped: false,
            column: Vec::new(),
            launched: None,
            wait_for_enter: false,
            runs: 0,
            stderr: Vec::new(),
        }
    }
}

/// One start of the program: its path and its whole argument list.
pub type Invocation = (PathBuf, Vec<OsString>);

/// Runs a command with a context. The questions (`confirm`, `prompt`) are
/// the caller's, asked before; their answer is in the context.
pub fn run(
    command: &ScriptCommand,
    context: &RunContext,
    env: &RunEnv,
    sink: &mut dyn RunSink,
    cancel: &Cancel,
) -> Result<RunReport, RunError> {
    if !command.exists_on(env.lookup.os) {
        return Err(RunError::OtherSystem);
    }
    let mut tools = BTreeMap::new();
    for tool in &command.needs {
        let found = env
            .find_tool(tool)
            .ok_or_else(|| RunError::NeedsTool(tool.clone()))?;
        tools.insert(tool.clone(), found);
    }
    let interpreter = match &command.program {
        Program::Script(path) => Some(
            env.lookup
                .choose(path, &command.recipe.interpreter)
                .map_err(RunError::Interpreter)?,
        ),
        Program::Argv(_) => None,
    };
    let forward = interpreter.as_ref().is_some_and(|i| i.forward_slashes);
    let contexts: Vec<RunContext> = if command.recipe.per == Per::Entry {
        let chosen = effective(command, context).map_err(RunError::Refused)?;
        chosen
            .entries
            .into_iter()
            .map(|entry| one_entry(context, entry))
            .collect()
    } else {
        vec![context.clone()]
    };
    let mut report = RunReport::new();
    let mut last_error = None;
    for one in &contexts {
        if cancel.is_cancelled() {
            report.ending = Ending::Cancelled;
            break;
        }
        let prepared = prepare(command, one, &env.folders, forward, &tools)?;
        let invocations = match &interpreter {
            Some(interpreter) => match &command.program {
                Program::Script(path) => vec![interpreter.command_for(path)],
                Program::Argv(_) => Vec::new(),
            },
            None => match argv_invocations(command, one, &prepared, env) {
                Ok(list) => list,
                Err(e) => {
                    let _ = prepared.folder.remove();
                    return Err(e);
                }
            },
        };
        for (program, args) in invocations {
            if cancel.is_cancelled() {
                report.ending = Ending::Cancelled;
                break;
            }
            let spec = process_spec(&program, &args, &prepared, command);
            let outcome = run_process(&spec, &prepared, command, env, sink, cancel, &mut report);
            report.runs += 1;
            if command.recipe.mode == RunMode::OutputColumn
                && let Some(entry) = one.cursor.as_ref()
            {
                let first = outcome.first_line.clone().unwrap_or_default();
                report.column.push((entry.place.clone(), first));
            }
            if outcome.stderr_last.is_some() {
                last_error = outcome.stderr_last.clone();
            }
            let stop = !matches!(outcome.ending, Ending::Exited(0));
            if stop || report.ending == Ending::Exited(0) {
                report.ending = outcome.ending;
            }
            if matches!(report.ending, Ending::Cancelled | Ending::TimedOut(_)) {
                break;
            }
        }
        if let Err(e) = prepared.folder.remove() {
            sink.log(&format!("the run folder was not removed: {e}"));
        }
        if matches!(report.ending, Ending::Cancelled | Ending::TimedOut(_)) {
            break;
        }
    }
    let quiet = report.ending.is_done()
        && (report.launched.is_some() || command.recipe.mode == RunMode::Lister);
    if !quiet {
        report.said = ending_lines(
            &command.title,
            &command.source,
            &report.ending,
            &command.recipe.exit_words,
            last_error.as_deref(),
        );
    }
    if let Some(program) = &report.launched
        && report.ending.is_done()
    {
        report.said = vec![Message::status(words::started(program))];
    }
    Ok(report)
}

/// A per-entry run's context: the one entry is the cursor and the
/// selection.
fn one_entry(context: &RunContext, entry: Picked) -> RunContext {
    let mut one = context.clone();
    one.cursor = Some(entry.clone());
    one.marked = vec![entry];
    one
}

/// The starts of a manifest-only command: its `argv` with the program
/// found by the tools rule. Placeholders arrive with the next commit.
fn argv_invocations(
    command: &ScriptCommand,
    _context: &RunContext,
    _prepared: &Prepared,
    env: &RunEnv,
) -> Result<Vec<Invocation>, RunError> {
    let Program::Argv(argv) = &command.program else {
        return Ok(Vec::new());
    };
    let Some((program, args)) = argv.split_first() else {
        return Err(RunError::Placeholder("its argv is empty".to_owned()));
    };
    let path = env
        .find_tool(program)
        .ok_or_else(|| RunError::NeedsTool(program.clone()))?;
    Ok(vec![(path, args.iter().map(OsString::from).collect())])
}

/// What one start of the program left.
struct Outcome {
    ending: Ending,
    first_line: Option<String>,
    stderr_last: Option<String>,
}

/// A piece of a stream, from its reader thread.
enum Chunk {
    Record(Vec<u8>),
    End,
}

/// Reads a stream into records ended by `separator`, sending each; past
/// `limit` bytes it keeps reading and drops, so the program never blocks.
fn read_records<R: Read + Send + 'static>(
    stream: R,
    separator: u8,
    limit: usize,
) -> (Receiver<Chunk>, Arc<AtomicBool>) {
    let (sender, receiver) = mpsc::channel();
    let cut = Arc::new(AtomicBool::new(false));
    let cut_flag = Arc::clone(&cut);
    let spawned = thread::Builder::new()
        .name("script-output".to_owned())
        .spawn(move || {
            let mut stream = stream;
            let mut buffer = [0u8; 16 * 1024];
            let mut record = Vec::new();
            let mut kept = 0usize;
            loop {
                let n = match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for &byte in &buffer[..n] {
                    if kept >= limit {
                        cut_flag.store(true, Ordering::SeqCst);
                        continue;
                    }
                    kept += 1;
                    if byte == separator {
                        if sender
                            .send(Chunk::Record(std::mem::take(&mut record)))
                            .is_err()
                        {
                            return;
                        }
                    } else {
                        record.push(byte);
                    }
                }
            }
            if !record.is_empty() {
                let _ = sender.send(Chunk::Record(record));
            }
            let _ = sender.send(Chunk::End);
        });
    if spawned.is_err() {
        cut.store(true, Ordering::SeqCst);
    }
    (receiver, cut)
}

/// Reads standard error, keeping its last bytes.
fn read_tail<R: Read + Send + 'static>(stream: R) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("script-stderr".to_owned())
        .spawn(move || {
            let mut stream = stream;
            let mut kept: Vec<u8> = Vec::new();
            let mut buffer = [0u8; 8 * 1024];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        kept.extend_from_slice(&buffer[..n]);
                        if kept.len() > MAX_STDERR_BYTES * 2 {
                            let cut = kept.len() - MAX_STDERR_BYTES;
                            kept.drain(..cut);
                        }
                    }
                }
            }
            let _ = sender.send(kept);
        });
    receiver
}

/// Follows one start of the program to its end.
fn run_process(
    spec: &ProcessSpec,
    prepared: &Prepared,
    command: &ScriptCommand,
    env: &RunEnv,
    sink: &mut dyn RunSink,
    cancel: &Cancel,
    report: &mut RunReport,
) -> Outcome {
    let recipe = &command.recipe;
    let mode = recipe.mode;
    let mut outcome = Outcome {
        ending: Ending::Exited(0),
        first_line: None,
        stderr_last: None,
    };
    if mode == RunMode::Launch {
        match spec.launch() {
            Ok(_) => {
                report.launched = Some(
                    spec.program()
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            Err(e) => outcome.ending = Ending::NotStarted(e.to_string()),
        }
        return outcome;
    }
    let mut child = match spec.spawn() {
        Ok(child) => child,
        Err(e) => {
            outcome.ending = Ending::NotStarted(e.to_string());
            return outcome;
        }
    };
    if mode == RunMode::TerminalWait {
        report.wait_for_enter = true;
    }
    if recipe.stdin == StdinForm::Json
        && let Some(mut stdin) = child.take_stdin()
    {
        let json = std::fs::read(prepared.folder.context_file()).unwrap_or_default();
        let _ = thread::Builder::new()
            .name("script-stdin".to_owned())
            .spawn(move || {
                // A script that never reads its input, or ends early, is
                // not an error: a broken pipe is ignored.
                let _ = stdin.write_all(&json);
                let _ = stdin.write_all(b"\n");
            });
    }
    let separator = if mode == RunMode::Lister && recipe.list == ListForm::Nul {
        0
    } else {
        b'\n'
    };
    let stdout = child
        .take_stdout()
        .map(|out| read_records(out, separator, MAX_OUTPUT_BYTES));
    let stderr = child.take_stderr().map(read_tail);
    let mut directives = DirectiveReader::new();
    let directives_file = prepared.folder.directives_file();
    let started = Instant::now();
    let mut ending_at: Option<Instant> = None;
    let mut cancelled = false;
    let mut timed_out = false;
    let mut killed = false;
    let mut output_open = stdout.is_some();
    let mut capped = false;
    let status = loop {
        if let Some((receiver, _)) = &stdout {
            while let Ok(chunk) = receiver.try_recv() {
                match chunk {
                    Chunk::Record(bytes) => {
                        handle_record(&bytes, command, sink, report, &mut outcome, &mut capped);
                    }
                    Chunk::End => output_open = false,
                }
            }
        }
        if let Ok(lines) = directives.poll(&directives_file) {
            for line in lines {
                handle_directive(&line, sink, report);
            }
        }
        if capped && !killed {
            let _ = child.start_kill();
            killed = true;
        }
        if ending_at.is_none() {
            if cancel.is_cancelled() {
                cancelled = true;
                ending_at = Some(Instant::now());
            } else if let Some(limit) = recipe.timeout
                && started.elapsed() >= limit
            {
                timed_out = true;
                ending_at = Some(Instant::now());
            }
            if ending_at.is_some() {
                let _ = prepared.folder.cancel();
            }
        }
        if let Some(at) = ending_at
            && at.elapsed() >= env.grace
            && !killed
        {
            let _ = child.start_kill();
            killed = true;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        thread::sleep(POLL);
    };
    // The rest of the output, unless something the program left behind
    // holds it open.
    if let Some((receiver, cut)) = &stdout {
        let deadline = Instant::now() + DRAIN;
        while output_open {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(Chunk::Record(bytes)) => {
                    handle_record(&bytes, command, sink, report, &mut outcome, &mut capped);
                }
                Ok(Chunk::End) | Err(RecvTimeoutError::Disconnected) => output_open = false,
                Err(RecvTimeoutError::Timeout) => {
                    let _ = child.start_kill();
                    break;
                }
            }
        }
        if cut.load(Ordering::SeqCst) && mode != RunMode::Lister {
            report.output_cut = true;
        }
    }
    if let Ok(lines) = directives.finish(&directives_file) {
        for line in lines {
            handle_directive(&line, sink, report);
        }
    }
    if let Some(receiver) = stderr
        && let Ok(bytes) = receiver.recv_timeout(DRAIN)
    {
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = text
            .lines()
            .map(|l| clean_text(l, MAX_OUTPUT_LINE_CHARS))
            .filter(|l| !l.is_empty())
            .collect();
        outcome.stderr_last = lines.last().cloned();
        let keep = lines.len().saturating_sub(20);
        report.stderr.extend(lines.into_iter().skip(keep));
    }
    if capped {
        report.capped = true;
    }
    outcome.ending = if cancelled {
        Ending::Cancelled
    } else if timed_out {
        Ending::TimedOut(recipe.timeout.unwrap_or_default())
    } else if capped {
        Ending::Exited(0)
    } else {
        match status.map(|s| s.code()) {
            Some(Some(code)) => Ending::Exited(code),
            Some(None) => Ending::Signal,
            None => Ending::NotStarted("its end could not be read".to_owned()),
        }
    };
    outcome
}

/// One record of standard output: a place for a lister, else a line (or
/// an opted-in directive).
fn handle_record(
    bytes: &[u8],
    command: &ScriptCommand,
    sink: &mut dyn RunSink,
    report: &mut RunReport,
    outcome: &mut Outcome,
    capped: &mut bool,
) {
    let recipe = &command.recipe;
    if recipe.mode == RunMode::Lister {
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        if bytes.is_empty() {
            return;
        }
        if report.places.len() >= recipe.max_rows as usize {
            *capped = true;
            return;
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => report.places.push(text.to_owned()),
            Err(_) => sink.log("a place that is not valid text was left out"),
        }
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_suffix('\r').unwrap_or(&text);
    if let Some(parsed) = stdout_directive(text, recipe.stdout_directives) {
        match parsed {
            Ok(directive) => route(directive, sink, report),
            Err(e) => sink.log(&e.to_string()),
        }
        return;
    }
    let line = clean_text(text, MAX_OUTPUT_LINE_CHARS);
    if outcome.first_line.is_none() && !line.is_empty() {
        outcome.first_line = Some(line.clone());
    }
    if recipe.mode == RunMode::OutputPane {
        sink.output(&line);
    }
    report.output.push(line);
}

/// One line of the directives file.
fn handle_directive(line: &str, sink: &mut dyn RunSink, report: &mut RunReport) {
    if line.trim().is_empty() {
        return;
    }
    match parse_directive(line) {
        Ok(directive) => route(directive, sink, report),
        Err(e) => sink.log(&e.to_string()),
    }
}

/// A directive acts now, or waits for the end.
fn route(directive: Directive, sink: &mut dyn RunSink, report: &mut RunReport) {
    if directive.is_deferred() {
        report.deferred.push(directive);
    } else {
        sink.act(&directive);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::script::header::parse_header;
    use diroxide_core::EntryKind;
    use std::fs;

    /// A scratch scripts folder and state folder, with the system's
    /// interpreters; `None` when this computer has no bash (the test then
    /// says so and passes).
    pub(crate) struct Bench {
        pub root: tempfile::TempDir,
        pub env: RunEnv,
    }

    impl Bench {
        pub(crate) fn new() -> Option<Self> {
            let root = tempfile::tempdir().unwrap();
            let mut env = RunEnv::system(StateFolders {
                runs: root.path().join("state").join("runs"),
                data: root.path().join("state").join("data"),
            });
            env.grace = Duration::from_millis(500);
            if env.lookup.bash().is_err() {
                eprintln!("bash was not found; this test is skipped here");
                return None;
            }
            fs::create_dir_all(root.path().join("scripts")).unwrap();
            fs::create_dir_all(root.path().join("work")).unwrap();
            Some(Self { root, env })
        }

        /// Writes a bash script with `\n` line ends.
        pub(crate) fn script(&self, name: &str, text: &str) -> ScriptCommand {
            let path = self.root.path().join("scripts").join(name);
            fs::write(&path, text.replace("\r\n", "\n")).unwrap();
            ScriptCommand::from_script(&path, &parse_header(text).header)
        }

        pub(crate) fn work(&self) -> Place {
            Place::parse(&self.root.path().join("work").to_string_lossy()).unwrap()
        }

        /// A context in the work folder, the cursor on a file there.
        pub(crate) fn context(&self) -> RunContext {
            let work = self.work();
            let file = self.root.path().join("work").join("a.txt");
            fs::write(&file, b"hello\n").unwrap();
            let mut context = RunContext::new(work.clone(), work);
            context.cursor = Some(Picked::new(
                Place::parse(&file.to_string_lossy()).unwrap(),
                EntryKind::File,
            ));
            context
        }

        pub(crate) fn run(
            &self,
            command: &ScriptCommand,
            context: &RunContext,
        ) -> (RunReport, Recorder) {
            let mut sink = Recorder::default();
            let report = run(command, context, &self.env, &mut sink, &Cancel::new()).unwrap();
            (report, sink)
        }
    }

    fn said(report: &RunReport) -> Vec<String> {
        report.said.iter().map(|m| m.text.clone()).collect()
    }

    const HEAD: &str = "#!/usr/bin/env bash\n# diro: name = \"Probe\"\n";

    #[test]
    fn exit_codes_end_in_words() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "codes.sh",
            &format!("{HEAD}echo 'cannot read a.txt' >&2\nexit \"${{DIRO_INPUT:-0}}\"\n"),
        );
        let expect = [
            ("0", "Probe done."),
            ("1", "Could not run Probe: code 1, nothing changed."),
            ("2", "Could not run Probe: code 2, wrong use."),
            ("3", "Probe partly done: code 3."),
            ("126", "Could not run Probe: it is not executable."),
            ("127", "Could not run Probe: a program was not found."),
        ];
        for (code, words) in expect {
            let mut context = bench.context();
            context.input = Some(code.to_owned());
            let (report, _) = bench.run(&command, &context);
            assert_eq!(report.ending, Ending::Exited(code.parse().unwrap()));
            assert_eq!(said(&report)[0], words);
            if code != "0" {
                assert_eq!(said(&report)[1], "Last error: cannot read a.txt.");
            }
        }
        // The run folders are gone.
        let runs = bench.root.path().join("state").join("runs");
        assert_eq!(fs::read_dir(runs).unwrap().count(), 0);
    }

    #[test]
    fn words_act_at_once_and_panel_moves_wait() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "directives.sh",
            &format!(
                "{HEAD}d=\"${{DIRO_DIRECTIVES:?}}\"\nprintf 'announce Selected: 1\\n' >> \"$d\"\nprintf 'cd %s\\n' \"$DIRO_ACTIVE\" >> \"$d\"\nprintf 'dance\\n' >> \"$d\"\nsleep 1\nprintf 'select %s\\n' \"$DIRO_CURSOR\" >> \"$d\"\nprintf 'progress 1 of 2\\n' >> \"$d\"\n"
            ),
        );
        let (report, sink) = bench.run(&command, &bench.context());
        let ended = Instant::now();
        assert_eq!(report.ending, Ending::Exited(0));
        let acted: Vec<&Directive> = sink.acted.iter().map(|(_, d)| d).collect();
        assert_eq!(acted[0], &Directive::Announce("Selected: 1".into()));
        assert!(matches!(
            acted[1],
            Directive::Progress {
                done: 1,
                total: 2,
                ..
            }
        ));
        // The announcement came while the script still slept.
        assert!(ended.duration_since(sink.acted[0].0) >= Duration::from_millis(500));
        assert_eq!(report.deferred.len(), 2);
        assert!(matches!(report.deferred[0], Directive::Cd(_)));
        assert!(matches!(report.deferred[1], Directive::Select(_)));
        assert_eq!(sink.log, ["unknown directive dance, ignored"]);
        let message = acted[0].message(&command.source).unwrap();
        assert_eq!(message.text, "Selected: 1. By Probe.");
    }

    #[test]
    fn a_printed_name_never_steers_the_panel() {
        let Some(bench) = Bench::new() else { return };
        let spoof =
            format!("{HEAD}echo 'diro:cd /elsewhere'\necho 'diro:announce Moved to the trash'\n");
        let command = bench.script("spoof.sh", &spoof);
        let (report, sink) = bench.run(&command, &bench.context());
        assert!(report.deferred.is_empty() && sink.acted.is_empty());
        assert_eq!(
            report.output,
            ["diro:cd /elsewhere", "diro:announce Moved to the trash"]
        );
        let opted = bench.script(
            "opted.sh",
            &format!(
                "{HEAD}# diro: stdout_directives = true\necho 'diro:cd /elsewhere'\necho plain\n"
            ),
        );
        let (report, _) = bench.run(&opted, &bench.context());
        assert_eq!(report.deferred, [Directive::Cd("/elsewhere".into())]);
        assert_eq!(report.output, ["plain"]);
    }

    #[test]
    fn the_context_arrives_on_standard_input() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "stdin.sh",
            &format!(
                "{HEAD}IFS= read -r line\ncase \"$line\" in '{{\"protocol\":1'*) echo json ;; *) echo other ;; esac\n"
            ),
        );
        let (report, _) = bench.run(&command, &bench.context());
        assert_eq!(report.output, ["json"]);
    }

    #[test]
    fn output_modes_put_the_output_where_they_say() {
        let Some(bench) = Bench::new() else { return };
        let body = "printf 'one\\n\\033[31mtwo\\033[0m\\n'\n";
        let pane = bench.script("pane.sh", &format!("{HEAD}{body}"));
        let (report, sink) = bench.run(&pane, &bench.context());
        assert_eq!(sink.output, ["one", "two"]);
        assert_eq!(report.output, ["one", "two"]);
        let viewer = bench.script(
            "viewer.sh",
            &format!("{HEAD}# diro: mode = \"output-viewer\"\n{body}"),
        );
        let (report, sink) = bench.run(&viewer, &bench.context());
        assert!(sink.output.is_empty());
        assert_eq!(report.output, ["one", "two"]);
        let quiet = bench.script("quiet.sh", &format!("{HEAD}# diro: mode = \"run\"\n{body}"));
        let (report, sink) = bench.run(&quiet, &bench.context());
        assert!(sink.output.is_empty());
        assert_eq!(report.output.len(), 2);
    }

    #[test]
    fn a_column_runs_once_per_entry() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "column.sh",
            &format!("{HEAD}# diro: mode = \"output-column\"\nprintf '%s\\n' \"${{DIRO_CURSOR_NAME:?}}\" extra\n"),
        );
        let mut context = bench.context();
        let work = bench.root.path().join("work");
        let mut marked = Vec::new();
        for name in ["b.txt", "c.txt"] {
            fs::write(work.join(name), b"x").unwrap();
            marked.push(Picked::from_disk(
                Place::parse(&work.join(name).to_string_lossy()).unwrap(),
            ));
        }
        context.marked = marked;
        let (report, _) = bench.run(&command, &context);
        assert_eq!(report.runs, 2);
        let values: Vec<&str> = report.column.iter().map(|(_, v)| v.as_str()).collect();
        assert_eq!(values, ["b.txt", "c.txt"]);
    }

    #[test]
    fn a_lister_reads_places_and_stops_at_its_cap() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "lister.sh",
            &format!(
                "{HEAD}# diro: mode = \"lister\"\n# diro: list = \"nul\"\n# diro: max_rows = 3\n# diro: selection = \"none\"\nfor n in 1 2 3 4 5; do printf '/r/line\\nbreak %s\\0' \"$n\"; done\nsleep 20\n"
            ),
        );
        let started = Instant::now();
        let (report, _) = bench.run(&command, &bench.context());
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the cap stops the program"
        );
        assert!(report.capped);
        assert_eq!(report.places.len(), 3);
        assert_eq!(report.places[0], "/r/line\nbreak 1");
        assert!(report.said.is_empty());
        assert_eq!(report.ending, Ending::Exited(0));
    }

    #[test]
    fn cancel_makes_the_cancel_file_appear_then_kills() {
        let Some(bench) = Bench::new() else { return };
        let polite = bench.script(
            "polite.sh",
            &format!("{HEAD}while [ ! -e \"${{DIRO_CANCEL_FILE:?}}\" ]; do sleep 0.1; done\necho saw it\nexit 0\n"),
        );
        let cancel = Cancel::new();
        let flag = cancel.clone();
        let waker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(700));
            flag.cancel();
        });
        let mut sink = Recorder::default();
        let report = run(&polite, &bench.context(), &bench.env, &mut sink, &cancel).unwrap();
        waker.join().unwrap();
        assert_eq!(report.ending, Ending::Cancelled);
        assert_eq!(said(&report), ["Probe: cancelled."]);
        assert_eq!(report.output, ["saw it"]);
        let deaf = bench.script("deaf.sh", &format!("{HEAD}sleep 30\n"));
        let cancel = Cancel::new();
        let flag = cancel.clone();
        let waker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            flag.cancel();
        });
        let started = Instant::now();
        let report = run(&deaf, &bench.context(), &bench.env, &mut sink, &cancel).unwrap();
        waker.join().unwrap();
        assert_eq!(report.ending, Ending::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_timeout_ends_the_run_in_words() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "slow.sh",
            &format!("{HEAD}# diro: timeout = \"1s\"\nsleep 30\n"),
        );
        let started = Instant::now();
        let (report, _) = bench.run(&command, &bench.context());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(report.ending, Ending::TimedOut(Duration::from_secs(1)));
        assert_eq!(said(&report), ["Probe timed out after 1 second."]);
    }

    #[test]
    fn terminal_wait_and_launch_say_what_the_frontend_does() {
        let Some(bench) = Bench::new() else { return };
        let wait = bench.script(
            "wait.sh",
            &format!("{HEAD}# diro: mode = \"terminal-wait\"\nexit 0\n"),
        );
        let (report, _) = bench.run(&wait, &bench.context());
        assert!(report.wait_for_enter);
        assert_eq!(report.ending, Ending::Exited(0));
        let launch = bench.script(
            "launch.sh",
            &format!("{HEAD}# diro: mode = \"launch\"\nexit 0\n"),
        );
        let (report, _) = bench.run(&launch, &bench.context());
        assert_eq!(report.launched.as_deref(), Some("bash"));
        assert_eq!(said(&report), ["Started bash."]);
    }

    #[test]
    fn a_missing_tool_is_refused_before_anything_runs() {
        let Some(bench) = Bench::new() else { return };
        let command = bench.script(
            "needs.sh",
            &format!("{HEAD}# diro: needs = [\"no-such-tool-here\"]\nexit 0\n"),
        );
        let mut sink = Recorder::default();
        let error = run(
            &command,
            &bench.context(),
            &bench.env,
            &mut sink,
            &Cancel::new(),
        )
        .unwrap_err();
        assert_eq!(
            error.sentence("Probe"),
            "Needs no-such-tool-here, not found."
        );
        let other = bench.script(
            "other.sh",
            &format!(
                "{HEAD}# diro: os = [\"{}\"]\nexit 0\n",
                if cfg!(windows) { "linux" } else { "windows" }
            ),
        );
        let error = run(
            &other,
            &bench.context(),
            &bench.env,
            &mut sink,
            &Cancel::new(),
        )
        .unwrap_err();
        assert_eq!(error.sentence("Probe"), "Probe is not for this system.");
    }
}
