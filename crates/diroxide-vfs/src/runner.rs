//! Running a helper program from the job engine (alpha.7, R30 commit 7):
//! the one door through which a conversion, the hot-folder runner, a
//! signature check or the 7z reader starts ImageMagick, FFmpeg, pandoc,
//! gpg or 7-Zip. The job engine may depend only on core and vfs, so the
//! trait lives here and travels in every [`OpContext`](crate::OpContext);
//! the tools registry implements it over its one limited runner, and the
//! app and the command line put that runner in the context they make.
//!
//! The rules every runner keeps, whoever implements it:
//!
//! - **An argument list, never a shell string.** A name with a space or
//!   a dollar sign arrives whole.
//! - **Limits always.** A run has a time limit and an output cap; past
//!   either, the program and every child it started are stopped, and the
//!   error says which limit and what it was.
//! - **Cancel stops the tree.** The context's token is checked while the
//!   program runs, not only before it starts.
//! - **A failed exit is not an error here.** [`ToolOutput::status`]
//!   carries it, and the caller says what the program said.
//! - **Not installed is a sentence.** "ImageMagick is not installed" is
//!   the cause the house style puts after "Could not convert a.heic:".

use std::ffi::OsString;
use std::fmt;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::error::VfsError;

/// The limits a run is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RunLimits {
    /// How long the program may run before it and its children are
    /// stopped.
    pub timeout: Duration,
    /// How many bytes of output, both streams together, are kept before
    /// the program is stopped.
    pub max_output_bytes: usize,
}

impl RunLimits {
    /// The plugin host's defaults: a minute and 16 MiB.
    pub const DEFAULT: RunLimits = RunLimits {
        timeout: Duration::from_secs(60),
        max_output_bytes: 16 * 1024 * 1024,
    };

    /// Limits of a time and an output cap.
    pub const fn new(timeout: Duration, max_output_bytes: usize) -> Self {
        Self {
            timeout,
            max_output_bytes,
        }
    }
}

impl Default for RunLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Where a program's standard input comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ToolInput {
    /// Nothing: standard input is closed, so a program that asks a
    /// question gets end of file and never waits.
    #[default]
    Closed,
    /// These bytes, then end of file (ExifTool's `-@ -` list).
    Bytes(Vec<u8>),
    /// A file's bytes, read as the program asks for them.
    File(PathBuf),
}

/// How streamed output is cut into records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Split {
    /// At each line feed; a carriage return before it is dropped.
    #[default]
    #[serde(rename = "lines")]
    Lines,
    /// At each line feed and each lone carriage return, so a progress
    /// line a program rewrites in place (git's `clone --progress`,
    /// FFmpeg's status) arrives as each rewrite.
    #[serde(rename = "cr")]
    CarriageReturn,
}

impl Split {
    /// The word on the wire and in a manifest: `lines` or `cr`.
    pub const fn word(self) -> &'static str {
        match self {
            Split::Lines => "lines",
            Split::CarriageReturn => "cr",
        }
    }

    /// Cuts a piece of output into its records, keeping an unfinished
    /// last one in `rest` for the next piece: the rule every streaming
    /// runner applies, so the job engine, the plugin host and the tests
    /// cut the same way.
    pub fn records(self, rest: &mut String, piece: &str) -> Vec<String> {
        rest.push_str(piece);
        let mut out = Vec::new();
        loop {
            let at = match self {
                Split::Lines => rest.find('\n'),
                Split::CarriageReturn => rest.find(['\n', '\r']),
            };
            let Some(at) = at else { break };
            let mut record: String = rest.drain(..=at).collect();
            record.pop();
            if self == Split::Lines && record.ends_with('\r') {
                record.pop();
            }
            if self == Split::CarriageReturn && record.is_empty() {
                continue;
            }
            out.push(record);
        }
        out
    }
}

/// Which stream a streamed record came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// One run of a helper program.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolRequest {
    /// The tool's id in the tools registry: `magick`, `ffmpeg`, `7z`.
    pub tool: String,
    /// The arguments, each one whole.
    pub args: Vec<OsString>,
    /// Standard input.
    pub input: ToolInput,
    /// The limits.
    pub limits: RunLimits,
    /// The working folder, when it matters; otherwise the runner's.
    pub folder: Option<PathBuf>,
    /// How [`ToolRunner::run_streaming`] cuts the output.
    pub split: Split,
}

impl ToolRequest {
    /// A run of a tool with these arguments, standard input closed, the
    /// default limits and line records.
    pub fn new<I, A>(tool: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        Self {
            tool: tool.into(),
            args: args.into_iter().map(Into::into).collect(),
            input: ToolInput::Closed,
            limits: RunLimits::DEFAULT,
            folder: None,
            split: Split::Lines,
        }
    }

    /// The same run with these limits.
    pub fn with_limits(mut self, limits: RunLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The same run with this standard input.
    pub fn with_input(mut self, input: ToolInput) -> Self {
        self.input = input;
        self
    }

    /// The same run in this working folder.
    pub fn in_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// The same run cutting streamed output this way.
    pub fn split_by(mut self, split: Split) -> Self {
        self.split = split;
        self
    }
}

/// What a finished run gave back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolOutput {
    /// The exit code, `None` when the program was ended by a signal.
    pub status: Option<i32>,
    /// Standard output, at most the output cap. Empty for a streaming run,
    /// whose records went to the caller.
    pub stdout: Vec<u8>,
    /// Standard error, likewise.
    pub stderr: Vec<u8>,
}

impl ToolOutput {
    /// An output with an exit code and the two streams.
    pub fn new(status: Option<i32>, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            status,
            stdout,
            stderr,
        }
    }

    /// Whether the program ended with exit code 0.
    pub fn succeeded(&self) -> bool {
        self.status == Some(0)
    }
}

/// Why a run gave no output. Each says its cause in words for the house
/// style's "Could not VERB OBJECT: CAUSE.", the program's own name first.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ToolRunError {
    /// The program is not installed, or `tools.toml` turns it off.
    #[error("{name} is not installed")]
    NotInstalled {
        /// The tool's id: `magick`.
        tool: String,
        /// Its name as said: "ImageMagick".
        name: String,
    },
    /// The program could not be started.
    #[error("{name} could not be started: {message}")]
    NotStarted {
        /// Its name as said.
        name: String,
        /// The system's words, in lowercase.
        message: String,
    },
    /// The program ran past its time limit and was stopped, with every
    /// child it started.
    #[error("{name} took longer than {} and was stopped", seconds_words(.seconds))]
    TimedOut {
        /// Its name as said.
        name: String,
        /// The limit, in whole seconds.
        seconds: u64,
    },
    /// The program wrote more than its output cap and was stopped.
    #[error("{name} wrote more than {limit} and was stopped")]
    TooMuchOutput {
        /// Its name as said.
        name: String,
        /// The cap in words: "16 MB".
        limit: String,
    },
    /// The run was cancelled; the program was stopped.
    #[error("it was cancelled")]
    Cancelled,
}

impl From<ToolRunError> for VfsError {
    fn from(error: ToolRunError) -> Self {
        match error {
            ToolRunError::Cancelled => VfsError::Cancelled,
            ToolRunError::TooMuchOutput { name, limit } => VfsError::LimitReached {
                what: format!("{name}'s output"),
                limit,
            },
            ToolRunError::NotInstalled { .. } => VfsError::Io {
                kind: std::io::ErrorKind::NotFound,
                message: error.to_string(),
            },
            ToolRunError::TimedOut { .. } => VfsError::Io {
                kind: std::io::ErrorKind::TimedOut,
                message: error.to_string(),
            },
            ToolRunError::NotStarted { .. } => VfsError::Io {
                kind: std::io::ErrorKind::Other,
                message: error.to_string(),
            },
        }
    }
}

/// Runs helper programs for the job engine. See the module documentation
/// for the rules every implementation keeps.
pub trait ToolRunner: Send + Sync {
    /// Whether the tool is found, by its registry id, without running it:
    /// what a plan checks before it offers a recipe.
    fn has(&self, tool: &str) -> bool;

    /// The tool's name as said ("ImageMagick"), or the id when this runner
    /// does not know it.
    fn name_of(&self, tool: &str) -> String {
        tool.to_owned()
    }

    /// Runs a program to its end under the request's limits, stopped with
    /// its children when `cancel` is set, and gives back its exit code and
    /// output.
    fn run(&self, request: &ToolRequest, cancel: &CancelToken) -> Result<ToolOutput, ToolRunError>;

    /// Runs a program, handing each record of its output (cut by
    /// [`ToolRequest::split`]) to `record` as it arrives; `record` returns
    /// `Break` to stop the program. The output's streams come back empty.
    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError>;
}

/// A runner that finds nothing: the default in an [`OpContext`](crate::OpContext),
/// for tests and for the operations that run no program. Every run says
/// the tool is not installed.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTools;

impl ToolRunner for NoTools {
    fn has(&self, _tool: &str) -> bool {
        false
    }

    fn run(
        &self,
        request: &ToolRequest,
        _cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolRunError> {
        Err(not_installed(&request.tool))
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        _cancel: &CancelToken,
        _record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        Err(not_installed(&request.tool))
    }
}

/// A whole number of seconds in words for a message: "1 second", "90
/// seconds".
pub fn seconds_words(seconds: &u64) -> String {
    if *seconds == 1 {
        "1 second".to_owned()
    } else {
        format!("{seconds} seconds")
    }
}

fn not_installed(tool: &str) -> ToolRunError {
    ToolRunError::NotInstalled {
        tool: tool.to_owned(),
        name: tool.to_owned(),
    }
}

impl fmt::Display for ToolStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ToolStream::Stdout => "standard output",
            ToolStream::Stderr => "standard error",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_drop_the_carriage_return_and_keep_the_unfinished_rest() {
        let mut rest = String::new();
        assert_eq!(
            Split::Lines.records(&mut rest, "one\r\ntwo\nthr"),
            ["one", "two"]
        );
        assert_eq!(rest, "thr");
        assert_eq!(Split::Lines.records(&mut rest, "ee\n"), ["three"]);
        assert!(rest.is_empty());
    }

    /// git's `clone --progress` rewrites one line with carriage returns;
    /// each rewrite is a record, and a CR LF ending makes no empty one.
    #[test]
    fn cr_records_each_rewrite_of_a_progress_line() {
        let mut rest = String::new();
        let records = Split::CarriageReturn.records(
            &mut rest,
            "Receiving objects:  10% (1/10)\rReceiving objects: 100% (10/10)\r\ndone.\n",
        );
        assert_eq!(
            records,
            [
                "Receiving objects:  10% (1/10)",
                "Receiving objects: 100% (10/10)",
                "done."
            ]
        );
        assert_eq!(
            serde_json::to_string(&Split::CarriageReturn).unwrap(),
            "\"cr\""
        );
        assert_eq!(
            serde_json::from_str::<Split>("\"lines\"").unwrap(),
            Split::Lines
        );
    }

    #[test]
    fn no_tools_says_the_tool_is_not_installed_in_words() {
        let request = ToolRequest::new("magick", ["a.png", "a.jpg"]);
        assert_eq!(request.args.len(), 2);
        let error = NoTools.run(&request, &CancelToken::new()).unwrap_err();
        assert_eq!(error.to_string(), "magick is not installed");
        let vfs: VfsError = error.into();
        assert_eq!(vfs.to_string(), "magick is not installed");
        assert!(!NoTools.has("magick"));
        let timed = ToolRunError::TimedOut {
            name: "FFmpeg".to_owned(),
            seconds: 600,
        };
        assert_eq!(
            timed.to_string(),
            "FFmpeg took longer than 600 seconds and was stopped"
        );
        let short = ToolRunError::TimedOut {
            name: "pandoc".to_owned(),
            seconds: 1,
        };
        assert_eq!(
            short.to_string(),
            "pandoc took longer than 1 second and was stopped"
        );
        let capped: VfsError = ToolRunError::TooMuchOutput {
            name: "ExifTool".to_owned(),
            limit: "16 MB".to_owned(),
        }
        .into();
        assert_eq!(
            capped.to_string(),
            "it passes the limit for ExifTool's output: 16 MB"
        );
    }
}
