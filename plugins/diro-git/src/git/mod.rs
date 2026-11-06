//! git through the host's `tools/run` (R29 2.2 and 2.4): an argument list,
//! never a shell; the host finds the program by the tools registry and the
//! owner's `tools.toml`, runs it with no window, kills its tree on cancel,
//! and hands back its output in base64.
//!
//! Every call carries `-c core.quotepath=false`, so names come back as
//! their own bytes, and `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0` and
//! `GIT_EDITOR=:`, so git never asks a question or opens an editor. A
//! call the plugin makes on its own (the title line, the column) is
//! **automatic**: it adds `--no-optional-locks`, `GIT_OPTIONAL_LOCKS=0`
//! and `-c core.fsmonitor=false`, which outranks the repository's own
//! configuration (spike GS4), so a hostile `core.fsmonitor` never runs. A
//! call that takes paths uses `--literal-pathspecs`, so a name with `*` or
//! `[` is that name and nothing else.

pub mod progress;
pub mod version;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use diroxide_plugins::base64;
use diroxide_plugins::protocol::{ErrorKind, Params, methods};
use serde_json::{Value, json};

use crate::error::GitError;
use crate::rpc::{Cancel, OutputSink, Peer};
use version::Version;

/// How long an automatic call may take: the title line keeps its old
/// words meanwhile (R29 2.3).
pub const AUTOMATIC_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a command's call may take by default.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a call that waits on a remote or a repository's hooks may
/// take: the host's own longest.
pub const LONG_TIMEOUT: Duration = Duration::from_secs(3600);

/// The most output a call may make, both streams: the host's answer
/// carries it in base64 on one line, which must stay under the host's
/// 8 MiB line (the disk plugin's lesson).
pub const MAX_OUTPUT: u64 = 5 * 1024 * 1024;

/// Where a run's progress records go, one rewrite of a line at a time.
pub type ProgressSink = Arc<dyn Fn(&str) + Send + Sync>;

/// One run of git.
#[derive(Clone, Default)]
pub struct Run {
    /// git's arguments after the plugin's own options.
    pub args: Vec<String>,
    /// The folder git runs in, as a place the host reads.
    pub cwd: Option<String>,
    /// Text on standard input: a commit message, never a secret.
    pub stdin: Option<String>,
    /// How long it may take.
    pub timeout: Option<Duration>,
    /// Whether the plugin runs it on its own (see the module's words).
    pub automatic: bool,
    /// Whether the arguments carry paths after `--`.
    pub paths: bool,
    /// Where standard error's records go while it runs, cut at each
    /// carriage return and line feed: git's progress lines.
    pub progress: Option<ProgressSink>,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run")
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("automatic", &self.automatic)
            .finish_non_exhaustive()
    }
}

impl Run {
    /// A run of git with these arguments, in `cwd`.
    pub fn new<S: AsRef<str>>(cwd: &Path, args: &[S]) -> Self {
        Self {
            args: args.iter().map(|a| a.as_ref().to_owned()).collect(),
            cwd: Some(cwd.display().to_string()),
            ..Self::default()
        }
    }

    /// The same run, made automatic.
    pub fn automatic(mut self) -> Self {
        self.automatic = true;
        self
    }

    /// The same run, its arguments ending in paths after `--`.
    pub fn with_paths<S: AsRef<str>>(mut self, paths: &[S]) -> Self {
        self.paths = true;
        self.args.push("--".to_owned());
        self.args
            .extend(paths.iter().map(|p| p.as_ref().to_owned()));
        self
    }

    /// The same run, with text on standard input.
    pub fn with_stdin(mut self, text: impl Into<String>) -> Self {
        self.stdin = Some(text.into());
        self
    }

    /// The same run, with another time limit.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The same run, its progress records handed to `sink`.
    pub fn with_progress(mut self, sink: ProgressSink) -> Self {
        self.progress = Some(sink);
        self
    }

    /// Every argument git gets, the plugin's own options first.
    pub fn command_line(&self) -> Vec<String> {
        let mut line = Vec::new();
        if self.automatic {
            line.push("--no-optional-locks".to_owned());
            line.push("-c".to_owned());
            line.push("core.fsmonitor=false".to_owned());
        }
        if self.paths {
            line.push("--literal-pathspecs".to_owned());
        }
        line.push("-c".to_owned());
        line.push("core.quotepath=false".to_owned());
        line.extend(self.args.iter().cloned());
        line
    }

    /// The variables git gets beside diroxide's environment: the ones the
    /// host lets a plugin set.
    pub fn environment(&self) -> Value {
        let mut env = json!({
            "LC_ALL": "C",
            "LANG": "C",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_EDITOR": ":",
            "GIT_PAGER": "cat",
        });
        if self.automatic {
            env["GIT_OPTIONAL_LOCKS"] = json!("0");
        }
        env
    }

    /// The `tools/run` request's parameters.
    pub fn params(&self) -> Params {
        let timeout = self.timeout.unwrap_or(if self.automatic {
            AUTOMATIC_TIMEOUT
        } else {
            COMMAND_TIMEOUT
        });
        let mut params = json!({
            "tool": "git",
            "args": self.command_line(),
            "encoding": "base64",
            "env": self.environment(),
            "timeout_ms": u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            "max_output_bytes": MAX_OUTPUT,
        });
        if let Some(cwd) = &self.cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(stdin) = &self.stdin {
            params["stdin"] = json!(stdin);
        }
        if self.progress.is_some() {
            params["stream"] = json!(["stderr"]);
            params["split"] = json!("cr");
        }
        match params {
            Value::Object(map) => map,
            _ => Params::new(),
        }
    }
}

/// What a run of git gave back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// Its exit code; `None` when it was stopped.
    pub code: Option<i32>,
    /// Its standard output, raw.
    pub stdout: Vec<u8>,
    /// Its standard error, as text (what was streamed is not repeated).
    pub stderr: String,
    /// Whether the output passed its cap and git was stopped.
    pub truncated: bool,
}

impl Output {
    /// Whether git ended with code 0.
    pub fn ok(&self) -> bool {
        self.code == Some(0) && !self.truncated
    }

    /// Standard output as text, lossy where it is not UTF-8.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// git's reason in its own words: the first line of standard error
    /// that says something, without "fatal: ", "error: " or "hint: ".
    pub fn reason(&self) -> String {
        reason_of(&self.stderr)
    }
}

/// git's reason in its error output: its first "fatal:" line, else its
/// first "error:" line, else the first line that is neither a hint nor a
/// progress line, without its prefix.
pub fn reason_of(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .collect();
    let chosen = lines
        .iter()
        .find(|l| l.starts_with("fatal: "))
        .or_else(|| lines.iter().find(|l| l.starts_with("error: ")))
        .or_else(|| lines.iter().find(|l| progress::read(l).is_none()));
    match chosen {
        Some(line) => ["fatal: ", "error: ", "warning: "]
            .iter()
            .fold(*line, |l, p| l.strip_prefix(p).unwrap_or(l))
            .trim_end_matches(['.', ':'])
            .to_owned(),
        None => "git gave no reason".to_owned(),
    }
}

/// How long the plugin waits for the host's answer beyond the run's own
/// limit.
const ANSWER_MARGIN: Duration = Duration::from_secs(30);

/// Runs git through the host. A missing git is [`GitError::NotFound`]; a
/// git that ran and failed is an [`Output`] with its code, for the caller
/// to word.
pub fn run(peer: &Peer, cancel: Option<&Cancel>, run: &Run) -> Result<Output, GitError> {
    let params = run.params();
    let wait = run
        .timeout
        .unwrap_or(COMMAND_TIMEOUT)
        .max(AUTOMATIC_TIMEOUT)
        .saturating_add(ANSWER_MARGIN);
    // A streamed standard error is not repeated in the answer, so its
    // records are kept here too: git's reason for a refusal comes last.
    let streamed = Arc::new(Mutex::new(String::new()));
    let answer = match &run.progress {
        Some(progress) => {
            let progress = Arc::clone(progress);
            let kept = Arc::clone(&streamed);
            let sink: OutputSink = Arc::new(move |stream: &str, chunk: &str| {
                if stream != "stderr" {
                    return;
                }
                let bytes = base64::decode(chunk).unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes);
                let text = text.trim_matches(['\r', '\n']);
                if text.is_empty() {
                    return;
                }
                keep_tail(
                    &mut kept.lock().unwrap_or_else(PoisonError::into_inner),
                    text,
                );
                progress(text);
            });
            peer.call_streaming(methods::TOOLS_RUN, params, cancel, wait, sink)
        }
        None => peer.call(methods::TOOLS_RUN, params, cancel, wait),
    };
    let answer = answer.map_err(|error| {
        if error.code == ErrorKind::NotFound.code() {
            GitError::NotFound
        } else {
            GitError::from(error)
        }
    })?;
    let mut out = read_output(&answer);
    let streamed = std::mem::take(&mut *streamed.lock().unwrap_or_else(PoisonError::into_inner));
    if !streamed.is_empty() {
        out.stderr = format!("{streamed}{}", out.stderr);
    }
    Ok(out)
}

/// How much of a streamed standard error is kept: its end, where git's
/// reason is.
const STREAMED_KEPT: usize = 64 * 1024;

/// Adds a record to the kept end of a streamed standard error.
fn keep_tail(kept: &mut String, record: &str) {
    kept.push_str(record);
    kept.push('\n');
    if kept.len() > STREAMED_KEPT {
        let mut cut = kept.len() - STREAMED_KEPT;
        while !kept.is_char_boundary(cut) {
            cut += 1;
        }
        kept.drain(..cut);
    }
}

/// Reads `tools/run`'s answer.
pub fn read_output(answer: &Value) -> Output {
    let raw = answer.get("encoding").and_then(Value::as_str) == Some("base64");
    let bytes = |key: &str| -> Vec<u8> {
        let text = answer.get(key).and_then(Value::as_str).unwrap_or("");
        if raw {
            base64::decode(text).unwrap_or_default()
        } else {
            text.as_bytes().to_vec()
        }
    };
    Output {
        code: answer
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok()),
        stdout: bytes("stdout"),
        stderr: String::from_utf8_lossy(&bytes("stderr")).into_owned(),
        truncated: answer
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// Reads git's version through the host: `git --version`.
pub fn version(peer: &Peer, cancel: Option<&Cancel>) -> Result<Version, GitError> {
    let out = run(
        peer,
        cancel,
        &Run {
            args: vec!["--version".to_owned()],
            timeout: Some(AUTOMATIC_TIMEOUT),
            ..Run::default()
        },
    )?;
    Version::parse(&out.text())
        .or_else(|| Version::parse(&out.stderr))
        .ok_or_else(|| GitError::Bad("git did not say its version".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_automatic_call_carries_the_safety_options_first() {
        let run = Run::new(Path::new("repo"), &["status", "--porcelain=v2", "-z"])
            .automatic()
            .with_paths(&["."]);
        assert_eq!(
            run.command_line(),
            [
                "--no-optional-locks",
                "-c",
                "core.fsmonitor=false",
                "--literal-pathspecs",
                "-c",
                "core.quotepath=false",
                "status",
                "--porcelain=v2",
                "-z",
                "--",
                "."
            ]
        );
        let env = run.environment();
        assert_eq!(env["GIT_OPTIONAL_LOCKS"], "0");
        assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
        assert_eq!(env["LC_ALL"], "C");
        let params = run.params();
        assert_eq!(params["tool"], "git");
        assert_eq!(params["encoding"], "base64");
        assert_eq!(params["cwd"], "repo");
        assert!(params.get("stream").is_none());
    }

    #[test]
    fn a_command_s_call_runs_as_in_a_terminal_but_never_asks() {
        let run = Run::new(Path::new("repo"), &["commit", "-F", "-"]).with_stdin("Fix the parser");
        assert_eq!(
            run.command_line(),
            ["-c", "core.quotepath=false", "commit", "-F", "-"]
        );
        let env = run.environment();
        assert!(env.get("GIT_OPTIONAL_LOCKS").is_none());
        assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
        assert_eq!(env["GIT_EDITOR"], ":");
        assert_eq!(run.params()["stdin"], "Fix the parser");
        let streamed = run.with_progress(Arc::new(|_: &str| {}));
        assert_eq!(streamed.params()["split"], "cr");
        assert_eq!(streamed.params()["stream"], json!(["stderr"]));
    }

    #[test]
    fn the_answer_reads_in_base64_and_git_s_reason_reads_plainly() {
        let out = read_output(&json!({
            "exit_code": 128,
            "stdout": base64::encode(b"a\0b\xff"),
            "stderr": base64::encode(b"hint: try this\nfatal: not a git repository (or any of the parent directories): .git\n"),
            "encoding": "base64",
            "truncated": false,
        }));
        assert_eq!(out.stdout, b"a\0b\xff");
        assert!(!out.ok());
        assert_eq!(
            out.reason(),
            "not a git repository (or any of the parent directories): .git"
        );
        assert_eq!(reason_of(""), "git gave no reason");
        assert_eq!(
            reason_of(
                "Enumerating objects: 5, done.\nTo file:///x\n ! [rejected] main\nerror: failed to push some refs to 'x'\n"
            ),
            "failed to push some refs to 'x'"
        );
        assert_eq!(
            reason_of("Receiving objects: 100%\nwarning: something odd\n"),
            "something odd"
        );
        assert_eq!(
            reason_of("error: pathspec 'x' did not match any file(s) known to git."),
            "pathspec 'x' did not match any file(s) known to git"
        );
    }
}
