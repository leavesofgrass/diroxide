//! The process helper: one place where diroxide starts a program, for
//! plugins, script commands and tools alike (research/alpha4-plan.md,
//! section 5.3).
//!
//! A [`ProcessSpec`] names the program (a resolved path, never a shell
//! line), its arguments, its working folder, its environment, where each
//! standard stream goes, and how the child is tied to the host
//! ([`Tie`]). [`ProcessSpec::spawn`] starts it and returns a
//! [`ProcessHandle`] that writes, reads, waits and kills the whole tree.
//!
//! - **A tree** ([`Tie::Tree`], the default): on Windows the child starts
//!   with no console window and inside a job object, so killing it ends
//!   everything it started; elsewhere it leads a process group of its own,
//!   and the group is killed. Dropping the handle kills the tree.
//! - **Detached** ([`Tie::Detached`]): a program the host launches and
//!   never waits for (`tools/launch`, the `launch` mode): its own session
//!   or process group, outside every job the host makes, no console
//!   window, reaped by a thread of its own.
//! - **Terminal** ([`Tie::Terminal`]): the `terminal` modes, which share
//!   the host's console and foreground group so the owner's own Ctrl+C
//!   reaches them.
//!
//! The unsafe system calls (job objects, process groups) live in the
//! `process-wrap` crate, so this crate stays free of `unsafe`.
//!
//! Known limit (spike S5, Friday, October 23, 2026): `process-wrap`
//! 10.0.1 creates its job object without the kill-on-close limit, so on
//! Windows a host killed hard leaves the tree running, as a process group
//! does on Unix. A plugin that exits when its input ends (the protocol
//! asks it to) still goes, because the host's end of the pipe closes.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_core::{Explain, catalog};
use diroxide_vfs::limits::{PROGRAM_FOLDER_MAX_UNITS, utf16_units};
use process_wrap::std::{ChildWrapper, CommandWrap};
use thiserror::Error;

/// Windows' flag for a child that gets no console window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Windows' flag for a child that leads a new process group.
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Windows' flag for a child that leaves the host's own job, when that job
/// allows it.
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// The environment variable that counts how deep diroxide runs inside
/// itself: 1 in a child of diroxide, one more in each child below it.
pub const LEVEL_VARIABLE: &str = "DIRO_LEVEL";

/// The prefix of diroxide's own environment variables; inherited ones are
/// removed before a child starts, except [`LEVEL_VARIABLE`], which is
/// counted up.
pub const DIRO_PREFIX: &str = "DIRO_";

/// Where a child's standard stream goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stream {
    /// Nowhere: the null device.
    Null,
    /// A pipe the host reads or writes.
    Piped,
    /// The host's own stream (terminal modes and test helpers).
    Inherit,
    /// A file, appended to (standard error into a log).
    File(PathBuf),
}

/// How a child is tied to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tie {
    /// Its own job object or process group, no console window, killed as
    /// a tree, and killed when its handle is dropped.
    Tree,
    /// A detached launch: its own session or group, outside every job the
    /// host makes, no console window, never waited for.
    Detached,
    /// Shares the host's console and foreground group: the terminal
    /// modes. Killed alone.
    Terminal,
}

/// Why a program could not be started or stopped.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ProcessError {
    /// The program could not be started.
    #[error("{program} could not be started: {message}")]
    Start {
        /// The program's file name.
        program: String,
        /// The system's words.
        message: String,
    },
    /// The working folder is deeper than Windows starts a program in.
    #[error("{}", catalog::PROGRAM_TOO_DEEP)]
    TooDeep,
    /// Waiting for or stopping the program failed.
    #[error("{0}")]
    Io(String),
}

impl Explain for ProcessError {
    fn next_step(&self) -> String {
        match self {
            ProcessError::TooDeep => "Run it from a shorter folder.".to_owned(),
            ProcessError::Start { .. } | ProcessError::Io(_) => String::new(),
        }
    }
}

/// Everything about a program to start. Built step by step, then
/// [`ProcessSpec::spawn`]ed; it can be spawned more than once.
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    set: Vec<(OsString, OsString)>,
    remove: Vec<OsString>,
    diro: bool,
    stdin: Stream,
    stdout: Stream,
    stderr: Stream,
    tie: Tie,
}

impl ProcessSpec {
    /// A program by its path, with no arguments, standard input and output
    /// piped, standard error to the null device, tied as a tree.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            set: Vec::new(),
            remove: Vec::new(),
            diro: false,
            stdin: Stream::Piped,
            stdout: Stream::Piped,
            stderr: Stream::Null,
            tie: Tie::Tree,
        }
    }

    /// Adds one argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds arguments.
    pub fn args<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// The working folder (a full path).
    pub fn cwd(mut self, folder: impl Into<PathBuf>) -> Self {
        self.cwd = Some(folder.into());
        self
    }

    /// Sets an environment variable over the inherited environment.
    pub fn env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.set.push((name.into(), value.into()));
        self
    }

    /// Removes an inherited environment variable.
    pub fn env_remove(mut self, name: impl Into<OsString>) -> Self {
        self.remove.push(name.into());
        self
    }

    /// The environment every child of diroxide gets: inherited, then
    /// `NO_COLOR=1`, `PYTHONUTF8=1` and `PYTHONIOENCODING=utf-8`, every
    /// inherited `DIRO_*` variable removed, and `DIRO_LEVEL` counted up.
    /// Variables set with [`ProcessSpec::env`] come after, so the host's
    /// own `DIRO_*` values win.
    pub fn diroxide_environment(mut self) -> Self {
        self.diro = true;
        self
    }

    /// Where standard input comes from.
    pub fn stdin(mut self, stream: Stream) -> Self {
        self.stdin = stream;
        self
    }

    /// Where standard output goes.
    pub fn stdout(mut self, stream: Stream) -> Self {
        self.stdout = stream;
        self
    }

    /// Where standard error goes.
    pub fn stderr(mut self, stream: Stream) -> Self {
        self.stderr = stream;
        self
    }

    /// How the child is tied to the host.
    pub fn tie(mut self, tie: Tie) -> Self {
        self.tie = tie;
        self
    }

    /// The program.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The arguments.
    pub fn arguments(&self) -> &[OsString] {
        &self.args
    }

    /// The tie.
    pub fn tied(&self) -> Tie {
        self.tie
    }

    /// The Windows creation flags this spec starts a child with (the job
    /// object adds a temporary suspension of its own); 0 elsewhere. A test
    /// hook as much as a rule: a tree or a detached child never gets a
    /// console window.
    pub fn creation_flags(&self) -> u32 {
        if !cfg!(windows) {
            return 0;
        }
        match self.tie {
            Tie::Tree => CREATE_NO_WINDOW,
            Tie::Detached => CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP,
            Tie::Terminal => 0,
        }
    }

    /// The environment changes, in order: what is removed, then what is
    /// set. Read from the host's environment now.
    pub fn environment_changes(&self) -> (Vec<OsString>, Vec<(OsString, OsString)>) {
        let mut removed = Vec::new();
        let mut set = Vec::new();
        if self.diro {
            for (name, _) in env::vars_os() {
                let text = name.to_string_lossy();
                if text.starts_with(DIRO_PREFIX) && text != LEVEL_VARIABLE {
                    removed.push(name);
                }
            }
            set.push(("NO_COLOR".into(), "1".into()));
            set.push(("PYTHONUTF8".into(), "1".into()));
            set.push(("PYTHONIOENCODING".into(), "utf-8".into()));
            let level = next_level(env::var_os(LEVEL_VARIABLE).as_deref());
            set.push((LEVEL_VARIABLE.into(), level.to_string().into()));
        }
        removed.extend(self.remove.iter().cloned());
        set.extend(self.set.iter().cloned());
        (removed, set)
    }

    fn command(&self) -> Result<Command, ProcessError> {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(cwd) = &self.cwd {
            if cfg!(windows) && utf16_units(&cwd.to_string_lossy()) > PROGRAM_FOLDER_MAX_UNITS {
                return Err(ProcessError::TooDeep);
            }
            command.current_dir(cwd);
        }
        let (removed, set) = self.environment_changes();
        for name in removed {
            command.env_remove(name);
        }
        for (name, value) in set {
            command.env(name, value);
        }
        command.stdin(self.stdio(&self.stdin)?);
        command.stdout(self.stdio(&self.stdout)?);
        command.stderr(self.stdio(&self.stderr)?);
        Ok(command)
    }

    fn stdio(&self, stream: &Stream) -> Result<Stdio, ProcessError> {
        Ok(match stream {
            Stream::Null => Stdio::null(),
            Stream::Piped => Stdio::piped(),
            Stream::Inherit => Stdio::inherit(),
            Stream::File(path) => OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map(Stdio::from)
                .map_err(|e| self.start_error(&e))?,
        })
    }

    fn start_error(&self, error: &io::Error) -> ProcessError {
        ProcessError::Start {
            program: self
                .program
                .file_name()
                .unwrap_or(self.program.as_os_str())
                .to_string_lossy()
                .into_owned(),
            message: error.to_string(),
        }
    }

    /// Starts the program.
    pub fn spawn(&self) -> Result<ProcessHandle, ProcessError> {
        let mut wrap = CommandWrap::from(self.command()?);
        platform::wrap(&mut wrap, self.tie, self.creation_flags());
        let child = wrap.spawn().map_err(|e| self.start_error(&e))?;
        Ok(ProcessHandle {
            id: child.id(),
            child,
            tie: self.tie,
            status: None,
        })
    }

    /// Starts the program detached and returns its process id at once; a
    /// thread waits for it so it never lingers as a zombie. On Windows the
    /// child leaves the host's own job when that job allows it.
    pub fn launch(&self) -> Result<u32, ProcessError> {
        let spec = self.clone().tie(Tie::Detached);
        let mut handle = match platform::breakaway(&spec) {
            Some(Ok(handle)) => handle,
            Some(Err(_)) | None => spec.spawn()?,
        };
        let id = handle.id();
        let reaper = thread::Builder::new()
            .name("launch-reaper".to_owned())
            .spawn(move || {
                let _ = handle.wait();
            });
        if let Err(e) = reaper {
            return Err(ProcessError::Io(e.to_string()));
        }
        Ok(id)
    }
}

/// `DIRO_LEVEL` for a child: one more than the host's, or 1.
pub fn next_level(inherited: Option<&OsStr>) -> u32 {
    inherited
        .and_then(OsStr::to_str)
        .and_then(|text| text.trim().parse::<u32>().ok())
        .map_or(1, |level| level.saturating_add(1))
}

/// A started program: its standard streams, waiting, and killing the
/// whole tree. Dropping the handle of a tree kills the tree.
#[derive(Debug)]
pub struct ProcessHandle {
    child: Box<dyn ChildWrapper>,
    id: u32,
    tie: Tie,
    status: Option<ExitStatus>,
}

impl ProcessHandle {
    /// The process id.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// The tie it was started with.
    pub fn tie(&self) -> Tie {
        self.tie
    }

    /// Takes the pipe to its standard input, once.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin().take()
    }

    /// Takes the pipe from its standard output, once.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout().take()
    }

    /// Takes the pipe from its standard error, once.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr().take()
    }

    /// How it ended, if it has.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, ProcessError> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let status = self
            .child
            .try_wait()
            .map_err(|e| ProcessError::Io(e.to_string()))?;
        self.status = status;
        Ok(status)
    }

    /// Waits for it to end.
    pub fn wait(&mut self) -> Result<ExitStatus, ProcessError> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let status = self
            .child
            .wait()
            .map_err(|e| ProcessError::Io(e.to_string()))?;
        self.status = Some(status);
        Ok(status)
    }

    /// Waits up to `timeout` for it to end; `None` when it is still
    /// running.
    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ExitStatus>, ProcessError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            thread::sleep((deadline - now).min(Duration::from_millis(10)));
        }
    }

    /// Starts killing it, and for a tree everything it started, without
    /// waiting.
    pub fn start_kill(&mut self) -> Result<(), ProcessError> {
        if self.status.is_some() && self.tie != Tie::Tree {
            return Ok(());
        }
        match self.child.start_kill() {
            Ok(()) => Ok(()),
            // Already gone: nothing to kill.
            Err(_) if self.try_wait().ok().flatten().is_some() => Ok(()),
            Err(e) => Err(ProcessError::Io(e.to_string())),
        }
    }

    /// Kills it, and for a tree everything it started, then reaps it.
    pub fn kill_tree(&mut self) -> Result<ExitStatus, ProcessError> {
        self.start_kill()?;
        self.wait()
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if self.tie == Tie::Tree {
            // The job or the group may still hold children after the
            // leader ended, so a tree is always ended; the reap is not
            // waited for.
            let _ = self.child.start_kill();
            let _ = self.child.try_wait();
        }
    }
}

#[cfg(windows)]
mod platform {
    use process_wrap::std::{CommandWrap, CreationFlags, JobObject};

    use super::{CREATE_BREAKAWAY_FROM_JOB, ProcessError, ProcessHandle, ProcessSpec, Tie};

    fn flags(value: u32) -> CreationFlags {
        let mut flags = CreationFlags(Default::default());
        flags.0.0 = value;
        flags
    }

    pub(super) fn wrap(wrap: &mut CommandWrap, tie: Tie, creation_flags: u32) {
        match tie {
            Tie::Tree => {
                wrap.wrap(flags(creation_flags));
                wrap.wrap(JobObject);
            }
            Tie::Detached => {
                wrap.wrap(flags(creation_flags));
            }
            Tie::Terminal => {}
        }
    }

    /// A detached start that leaves the host's own job; `None` when the
    /// spec is not detached.
    pub(super) fn breakaway(spec: &ProcessSpec) -> Option<Result<ProcessHandle, ProcessError>> {
        if spec.tie != Tie::Detached {
            return None;
        }
        let command = match spec.command() {
            Ok(command) => command,
            Err(e) => return Some(Err(e)),
        };
        let mut wrapped = CommandWrap::from(command);
        wrapped.wrap(flags(spec.creation_flags() | CREATE_BREAKAWAY_FROM_JOB));
        Some(
            wrapped
                .spawn()
                .map(|child| ProcessHandle {
                    id: child.id(),
                    child,
                    tie: Tie::Detached,
                    status: None,
                })
                .map_err(|e| spec.start_error(&e)),
        )
    }
}

#[cfg(unix)]
mod platform {
    use process_wrap::std::{CommandWrap, ProcessGroup, ProcessSession};

    use super::{ProcessError, ProcessHandle, ProcessSpec, Tie};

    pub(super) fn wrap(wrap: &mut CommandWrap, tie: Tie, _creation_flags: u32) {
        match tie {
            Tie::Tree => {
                wrap.wrap(ProcessGroup::leader());
            }
            Tie::Detached => {
                wrap.wrap(ProcessSession);
            }
            Tie::Terminal => {}
        }
    }

    pub(super) fn breakaway(_spec: &ProcessSpec) -> Option<Result<ProcessHandle, ProcessError>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::mpsc;

    use super::*;

    const HELPER: &str = "DIROXIDE_PROCESS_HELPER";
    const WAIT: Duration = Duration::from_secs(30);

    /// This test binary run again as a helper process in one mode.
    fn helper(mode: &str) -> ProcessSpec {
        let exe = env::current_exe().unwrap();
        ProcessSpec::new(exe)
            .args([
                "--exact",
                "process::tests::helper_process",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env(HELPER, mode)
    }

    /// The helper's behavior, chosen by the environment; an ordinary test
    /// run does nothing here.
    #[test]
    fn helper_process() {
        let Ok(mode) = env::var(HELPER) else {
            return;
        };
        let mut out = io::stdout();
        match mode.as_str() {
            // Starts a grandchild that shares this output, says ready,
            // and sleeps.
            "tree" => {
                let exe = env::current_exe().unwrap();
                let mut grandchild = Command::new(exe)
                    .args(["--exact", "process::tests::helper_process", "--nocapture"])
                    .env(HELPER, "sleep")
                    .stdin(Stdio::null())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                writeln!(out, "ready").unwrap();
                out.flush().unwrap();
                thread::sleep(Duration::from_secs(120));
                let _ = grandchild.wait();
            }
            "sleep" => thread::sleep(Duration::from_secs(120)),
            // Starts a tree whose output is this one's, and waits to be
            // killed.
            "host" => {
                let _tree = helper("tree")
                    .stdin(Stream::Null)
                    .stdout(Stream::Inherit)
                    .spawn()
                    .unwrap();
                thread::sleep(Duration::from_secs(120));
            }
            "env" => {
                for name in [
                    LEVEL_VARIABLE,
                    "DIRO_ACTIVE",
                    "DIRO_PLUGIN_ID",
                    "NO_COLOR",
                    "PYTHONUTF8",
                ] {
                    let value = env::var(name).unwrap_or_else(|_| "unset".to_owned());
                    writeln!(out, "{name}={value}").unwrap();
                }
                eprintln!("to the log");
                out.flush().unwrap();
            }
            "echo" => {
                let stdin = io::stdin();
                for line in stdin.lock().lines() {
                    writeln!(out, "{}", line.unwrap()).unwrap();
                    out.flush().unwrap();
                }
            }
            _ => {}
        }
        std::process::exit(0);
    }

    /// Reads lines from an output on a thread: each line, then `None` at
    /// the end of the stream.
    fn lines_of(output: impl Read + Send + 'static) -> mpsc::Receiver<Option<String>> {
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if sender.send(Some(line)).is_err() {
                    return;
                }
            }
            let _ = sender.send(None);
        });
        lines
    }

    /// Waits for a line that ends with `wanted` (the test harness may print
    /// its own words before the helper's on the same line).
    fn wait_for(lines: &mpsc::Receiver<Option<String>>, wanted: &str) {
        loop {
            match lines.recv_timeout(WAIT) {
                Ok(Some(line)) if line.ends_with(wanted) => return,
                Ok(Some(_)) => continue,
                other => panic!("waiting for {wanted}: {other:?}"),
            }
        }
    }

    fn wait_for_end(lines: &mpsc::Receiver<Option<String>>) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match lines.recv_timeout(left) {
                Ok(None) => return true,
                Ok(Some(_)) => continue,
                Err(_) => return false,
            }
        }
        false
    }

    #[test]
    fn killing_a_tree_ends_everything_it_started() {
        let mut child = helper("tree").stdin(Stream::Null).spawn().unwrap();
        let lines = lines_of(child.take_stdout().unwrap());
        wait_for(&lines, "ready");
        child.kill_tree().unwrap();
        // The output ends only when the child and its grandchild, which
        // shares it, are both gone.
        assert!(wait_for_end(&lines), "a process of the tree survived");
    }

    #[test]
    fn dropping_a_tree_handle_kills_the_tree() {
        let mut child = helper("tree").stdin(Stream::Null).spawn().unwrap();
        let lines = lines_of(child.take_stdout().unwrap());
        wait_for(&lines, "ready");
        drop(child);
        assert!(wait_for_end(&lines), "a process of the tree survived");
    }

    /// Spike S5, the hard half: a host killed outright. Ignored because it
    /// fails today on Windows: `process-wrap` 10.0.1 makes its job object
    /// without the kill-on-close limit, so the tree outlives the host (the
    /// contract change request in the W4a-1 report). On Unix a process
    /// group never dies with its parent; plugins exit when their input
    /// ends instead.
    #[test]
    #[ignore = "known limit: the job object has no kill-on-close (spike S5)"]
    fn a_tree_dies_with_a_host_killed_hard() {
        // The host is a helper that starts a tree and is then killed
        // alone, as Task Manager would.
        let exe = env::current_exe().unwrap();
        let mut host = Command::new(exe)
            .args(["--exact", "process::tests::helper_process", "--nocapture"])
            .env(HELPER, "host")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let lines = lines_of(host.stdout.take().unwrap());
        wait_for(&lines, "ready");
        host.kill().unwrap();
        let _ = host.wait();
        assert!(wait_for_end(&lines), "the tree outlived its host");
    }

    #[test]
    fn the_environment_is_diroxides() {
        let mut child = helper("env")
            .diroxide_environment()
            .env("DIRO_PLUGIN_ID", "line-counter")
            .stdin(Stream::Null)
            .stderr(Stream::Null)
            .spawn()
            .unwrap();
        let mut text = String::new();
        child
            .take_stdout()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(child.wait().unwrap().success());
        let expected_level = next_level(env::var_os(LEVEL_VARIABLE).as_deref());
        assert!(
            text.contains(&format!("DIRO_LEVEL={expected_level}")),
            "{text}"
        );
        assert!(text.contains("DIRO_PLUGIN_ID=line-counter"), "{text}");
        assert!(text.contains("NO_COLOR=1"), "{text}");
        assert!(text.contains("PYTHONUTF8=1"), "{text}");
    }

    #[test]
    fn inherited_diro_variables_are_removed_and_the_level_counts_up() {
        assert_eq!(next_level(None), 1);
        assert_eq!(next_level(Some(OsStr::new("2"))), 3);
        assert_eq!(next_level(Some(OsStr::new("junk"))), 1);
        assert_eq!(
            next_level(Some(OsStr::new(&u32::MAX.to_string()))),
            u32::MAX
        );
        let spec = ProcessSpec::new("x")
            .diroxide_environment()
            .env("DIRO_ACTIVE", "/home/ada");
        let (removed, set) = spec.environment_changes();
        assert!(removed.iter().all(|n| {
            let n = n.to_string_lossy();
            n.starts_with(DIRO_PREFIX) && n != LEVEL_VARIABLE
        }));
        let names: Vec<String> = set
            .iter()
            .map(|(n, _)| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "NO_COLOR",
                "PYTHONUTF8",
                "PYTHONIOENCODING",
                "DIRO_LEVEL",
                "DIRO_ACTIVE"
            ]
        );
    }

    #[test]
    fn standard_error_goes_to_a_log_file() {
        let folder = tempfile::tempdir().unwrap();
        let log = folder.path().join("plugin.log");
        let mut child = helper("env")
            .stdin(Stream::Null)
            .stdout(Stream::Null)
            .stderr(Stream::File(log.clone()))
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("to the log"), "{text}");
    }

    #[test]
    fn pipes_carry_lines_both_ways() {
        let mut child = helper("echo").spawn().unwrap();
        let mut stdin = child.take_stdin().unwrap();
        let lines = lines_of(child.take_stdout().unwrap());
        writeln!(stdin, "{{\"name\":\"caf\u{e9}\"}}").unwrap();
        stdin.flush().unwrap();
        wait_for(&lines, "{\"name\":\"caf\u{e9}\"}");
        drop(stdin);
        assert!(wait_for_end(&lines));
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_tree_never_gets_a_console_window() {
        let tree = ProcessSpec::new("x");
        let detached = ProcessSpec::new("x").tie(Tie::Detached);
        let terminal = ProcessSpec::new("x").tie(Tie::Terminal);
        if cfg!(windows) {
            assert_eq!(tree.creation_flags() & CREATE_NO_WINDOW, CREATE_NO_WINDOW);
            assert_eq!(
                detached.creation_flags() & CREATE_NO_WINDOW,
                CREATE_NO_WINDOW
            );
            assert_eq!(terminal.creation_flags(), 0);
        } else {
            assert_eq!(tree.creation_flags(), 0);
        }
    }

    #[test]
    fn a_detached_launch_runs_and_is_reaped() {
        let folder = tempfile::tempdir().unwrap();
        let out = folder.path().join("launched.txt");
        let id = helper("env")
            .diroxide_environment()
            .stdin(Stream::Null)
            .stdout(Stream::File(out.clone()))
            .launch()
            .unwrap();
        assert!(id > 0);
        // The launch answers at once; the program runs on its own.
        let deadline = Instant::now() + WAIT;
        while !std::fs::read_to_string(&out).is_ok_and(|t| t.contains("PYTHONUTF8=1")) {
            assert!(Instant::now() < deadline, "the launched program never ran");
            thread::sleep(Duration::from_millis(20));
        }
        // Let it end before the test does, so no handle outlives the test.
        thread::sleep(Duration::from_millis(300));
    }

    #[test]
    fn a_missing_program_says_so() {
        let err = ProcessSpec::new("no-such-program-anywhere")
            .spawn()
            .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("no-such-program-anywhere could not be started: "),
            "{err}"
        );
    }

    #[test]
    fn a_folder_too_deep_is_refused_on_windows() {
        let deep = PathBuf::from(format!("D:\\{}", "x".repeat(300)));
        let result = ProcessSpec::new("x").cwd(deep).spawn();
        if cfg!(windows) {
            let err = result.unwrap_err();
            assert_eq!(err, ProcessError::TooDeep);
            assert_eq!(
                err.to_string(),
                "Cannot start a program in a folder this deep"
            );
        } else {
            assert!(result.is_err());
        }
    }
}
