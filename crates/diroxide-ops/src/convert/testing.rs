//! The conversion tests' fake tools and scratch folders: no real program
//! runs, nothing plays sound, and every file is made under the build's
//! own folder.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diroxide_core::{JobId, Place};
use diroxide_vfs::{
    CancelToken, Registry, ToolOutput, ToolRequest, ToolRunError, ToolRunner, ToolStream,
};

use super::book::read_toml;
use super::recipe::{Recipe, RecipeSource};
use crate::engine::Engine;

/// A scratch folder under the build's folder, never the system's temp.
pub(crate) fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("convert-unit-tmp");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("convert-")
        .tempdir_in(&base)
        .unwrap()
}

/// A local place for a path.
pub(crate) fn place(path: &Path) -> Place {
    Place::parse(&path.to_string_lossy()).unwrap()
}

/// What the fake does when a recipe runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// Writes the output: "made from " and the input's bytes.
    Write,
    /// Writes half an output, then ends with this code and these words on
    /// standard error.
    Fail(i32, &'static str),
    /// Ends well and writes nothing.
    Nothing,
    /// Writes half an output, then waits until cancelled.
    Hang,
}

/// A hook run while the fake works, with its output's path.
type During = Box<dyn Fn(&Path) + Send + Sync>;

/// A tools registry stand-in: the tools it has, what each run does, and
/// every run's arguments.
pub(crate) struct FakeTools {
    has: BTreeSet<String>,
    pub act: Mutex<Act>,
    pub runs: Mutex<Vec<(String, Vec<OsString>)>>,
    pub encoders: String,
    pub formats: String,
    during: Mutex<Option<During>>,
}

impl FakeTools {
    /// A runner that has these tools, writing every output.
    pub(crate) fn with(tools: &[&str]) -> Self {
        Self {
            has: tools.iter().map(|t| (*t).to_owned()).collect(),
            act: Mutex::new(Act::Write),
            runs: Mutex::new(Vec::new()),
            encoders: " ------\n A....D libmp3lame  MP3\n A....D libopus  Opus\n A....D flac  FLAC\n V....D libvpx-vp9  VP9\n".to_owned(),
            formats: "     HEIC  HEIC      r--   High Efficiency Image Format\n      PNG* PNG       rw-   Portable Network Graphics\n     JPEG* JPEG      rw-   JPEG\n     WEBP* WEBP      rw+   WebP\n".to_owned(),
            during: Mutex::new(None),
        }
    }

    /// The same runner doing this.
    pub(crate) fn acting(self, act: Act) -> Self {
        *self.act.lock().unwrap() = act;
        self
    }

    /// The same runner calling `hook` while it works.
    pub(crate) fn during(self, hook: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        *self.during.lock().unwrap() = Some(Box::new(hook));
        self
    }

    /// The arguments of the recipe runs, without the probes.
    pub(crate) fn recipe_runs(&self) -> Vec<(String, Vec<String>)> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, args)| {
                tool != "ffprobe"
                    && !args.iter().any(|a| a == "-encoders")
                    && !args.windows(2).any(|w| w[0] == "-list" && w[1] == "format")
            })
            .map(|(tool, args)| {
                (
                    tool.clone(),
                    args.iter()
                        .map(|a| a.to_string_lossy().into_owned())
                        .collect(),
                )
            })
            .collect()
    }

    fn answer(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        self.runs
            .lock()
            .unwrap()
            .push((request.tool.clone(), request.args.clone()));
        if !self.has.contains(&request.tool) {
            return Err(ToolRunError::NotInstalled {
                tool: request.tool.clone(),
                name: request.tool.clone(),
            });
        }
        let args: Vec<String> = request
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if args.iter().any(|a| a == "-encoders") {
            return Ok(ToolOutput::new(
                Some(0),
                self.encoders.clone().into(),
                Vec::new(),
            ));
        }
        if args.windows(2).any(|w| w[0] == "-list" && w[1] == "format") {
            return Ok(ToolOutput::new(
                Some(0),
                self.formats.clone().into(),
                Vec::new(),
            ));
        }
        if request.tool == "ffprobe" {
            return Ok(ToolOutput::new(Some(0), b"4.000000\n".to_vec(), Vec::new()));
        }
        let output = args
            .iter()
            .find(|a| a.contains(".diro-part-"))
            .map(PathBuf::from)
            .ok_or_else(|| ToolRunError::NotStarted {
                name: request.tool.clone(),
                message: "no output named".to_owned(),
            })?;
        let input = args.iter().find_map(|a| {
            let bare = a
                .split_once(':')
                .filter(|(prefix, _)| prefix.len() > 1)
                .map_or(a.as_str(), |(_, rest)| rest);
            let path = Path::new(bare);
            (path.is_file() && !a.contains(".diro-part-")).then(|| path.to_path_buf())
        });
        let bytes = input
            .as_ref()
            .map(fs::read)
            .transpose()
            .unwrap_or_default()
            .unwrap_or_default();
        if let Some(hook) = self.during.lock().unwrap().as_ref() {
            hook(&output);
        }
        let act = self.act.lock().unwrap().clone();
        match act {
            Act::Write => {
                let mut made = b"made from ".to_vec();
                made.extend_from_slice(&bytes);
                fs::write(&output, made).unwrap();
                if request.tool == "ffmpeg" {
                    for line in ["out_time_us=1000000", "out_time_us=2000000", "progress=end"] {
                        let _ = record(ToolStream::Stdout, line);
                    }
                }
                Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new()))
            }
            Act::Fail(code, words) => {
                fs::write(&output, b"half").unwrap();
                let _ = record(ToolStream::Stderr, words);
                Ok(ToolOutput::new(Some(code), Vec::new(), Vec::new()))
            }
            Act::Nothing => Ok(ToolOutput::new(Some(0), Vec::new(), Vec::new())),
            Act::Hang => {
                fs::write(&output, b"half").unwrap();
                while !cancel.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(ToolRunError::Cancelled)
            }
        }
    }
}

impl ToolRunner for FakeTools {
    fn has(&self, tool: &str) -> bool {
        self.has.contains(tool)
    }

    fn name_of(&self, tool: &str) -> String {
        match tool {
            "magick" => "ImageMagick".to_owned(),
            "ffmpeg" => "FFmpeg".to_owned(),
            other => other.to_owned(),
        }
    }

    fn run(&self, request: &ToolRequest, cancel: &CancelToken) -> Result<ToolOutput, ToolRunError> {
        self.answer(request, cancel, &mut |_, _| ControlFlow::Continue(()))
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        let mut output = self.answer(request, cancel, record)?;
        output.stdout.clear();
        output.stderr.clear();
        Ok(output)
    }
}

/// An engine over the real local provider and these fake tools.
pub(crate) fn engine(tools: Arc<FakeTools>) -> Engine {
    Engine::new(Arc::new(Registry::with_local())).with_tools(tools)
}

/// One recipe from TOML text, built in.
pub(crate) fn recipe(text: &str) -> Recipe {
    let (mut recipes, problems) = read_toml(text, &RecipeSource::BuiltIn);
    assert!(problems.is_empty(), "{problems:?}");
    recipes.remove(0)
}

/// The job id the tests use.
pub(crate) const JOB: JobId = JobId::new(7);
