//! The external tools registry: one table of programs diroxide can use
//! when they are present and never requires, each found through one PATH
//! rule (then `tools.toml` overrides), and run with an argument list: no
//! shell, no console window on Windows, output captured, cancellable.
//!
//! Phase 0: the table, the PATH rule and a plain run are here; version
//! reading, cancellation while running and the Settings page data come in
//! Wave 5 (W5a), which owns this crate.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use diroxide_config::ToolsConfig;
use diroxide_core::Explain;
use diroxide_vfs::OpContext;
use thiserror::Error;

/// One external tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolSpec {
    /// The id used in recipes, actions and `tools.toml`: `magick`.
    pub id: &'static str,
    /// The name said in Settings: "ImageMagick".
    pub name: &'static str,
    /// Program names to look for, in order.
    pub programs: &'static [&'static str],
    /// Arguments that make it print its version.
    pub version_args: &'static [&'static str],
    /// What diroxide uses it for, in words.
    pub purpose: &'static str,
    /// Where to get it, in words.
    pub install_hint: &'static str,
}

/// The tools diroxide knows about.
pub const BUILTIN: [ToolSpec; 7] = [
    ToolSpec {
        id: "magick",
        name: "ImageMagick",
        programs: &["magick", "convert"],
        version_args: &["-version"],
        purpose: "image details and conversions",
        install_hint: "Install ImageMagick from imagemagick.org or your package manager.",
    },
    ToolSpec {
        id: "ffmpeg",
        name: "FFmpeg",
        programs: &["ffmpeg"],
        version_args: &["-version"],
        purpose: "audio and video conversions",
        install_hint: "Install FFmpeg from ffmpeg.org or your package manager.",
    },
    ToolSpec {
        id: "ffprobe",
        name: "FFprobe",
        programs: &["ffprobe"],
        version_args: &["-version"],
        purpose: "audio and video details",
        install_hint: "FFprobe comes with FFmpeg.",
    },
    ToolSpec {
        id: "pandoc",
        name: "pandoc",
        programs: &["pandoc"],
        version_args: &["--version"],
        purpose: "document previews and conversions",
        install_hint: "Install pandoc from pandoc.org or your package manager.",
    },
    ToolSpec {
        id: "7z",
        name: "7-Zip",
        programs: &["7z", "7za", "7zz"],
        version_args: &[],
        purpose: "cross-checking archives",
        install_hint: "Install 7-Zip from 7-zip.org, or p7zip from your package manager.",
    },
    ToolSpec {
        id: "pdftotext",
        name: "pdftotext",
        programs: &["pdftotext"],
        version_args: &["-v"],
        purpose: "PDF text previews",
        install_hint: "Install Poppler's utilities from your package manager.",
    },
    ToolSpec {
        id: "open",
        name: "the system opener",
        programs: if cfg!(windows) {
            &["explorer"]
        } else if cfg!(target_os = "macos") {
            &["open"]
        } else {
            &["xdg-open"]
        },
        version_args: &[],
        purpose: "opening files in their default programs",
        install_hint: "Install xdg-utils from your package manager.",
    },
];

/// Whether a tool can be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    /// Found, at this path.
    Found(PathBuf),
    /// Not on PATH and not set in `tools.toml`.
    NotFound,
    /// Turned off in `tools.toml`.
    Disabled,
}

/// What a tool printed and how it ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolOutput {
    /// The exit code, or `None` when it was stopped by a signal.
    pub code: Option<i32>,
    /// Standard output.
    pub stdout: Vec<u8>,
    /// Standard error.
    pub stderr: Vec<u8>,
}

impl ToolOutput {
    /// Whether it exited with code zero.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Why a tool could not be used.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ToolError {
    /// No tool has this id.
    #[error("no tool is called {0}")]
    Unknown(String),
    /// The tool is not installed.
    #[error("{0} is not installed")]
    NotFound(String),
    /// The tool is turned off.
    #[error("{0} is turned off in tools.toml")]
    Disabled(String),
    /// The run was cancelled.
    #[error("it was cancelled")]
    Cancelled,
    /// The tool could not be started.
    #[error("{tool} could not be started: {message}")]
    Start {
        /// The tool's name.
        tool: String,
        /// The system's words.
        message: String,
    },
}

impl Explain for ToolError {
    fn next_step(&self) -> String {
        match self {
            ToolError::NotFound(_) => "Install it, or set its path in tools.toml.".to_owned(),
            ToolError::Disabled(_) => "Turn it on in tools.toml.".to_owned(),
            ToolError::Unknown(_) | ToolError::Cancelled | ToolError::Start { .. } => String::new(),
        }
    }
}

/// The tools, with the user's overrides.
#[derive(Clone, Debug)]
pub struct ToolRegistry {
    specs: Vec<ToolSpec>,
    overrides: ToolsConfig,
    path_var: Option<OsString>,
}

impl ToolRegistry {
    /// The built-in tools, looked for on the process's PATH.
    pub fn builtin() -> Self {
        Self {
            specs: BUILTIN.to_vec(),
            overrides: ToolsConfig::default(),
            path_var: env::var_os("PATH"),
        }
    }

    /// The same registry with `tools.toml` overrides.
    pub fn with_overrides(mut self, overrides: ToolsConfig) -> Self {
        self.overrides = overrides;
        self
    }

    /// The same registry looking on another PATH, for tests.
    pub fn with_path(mut self, path_var: Option<OsString>) -> Self {
        self.path_var = path_var;
        self
    }

    /// Every tool, in table order.
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// A tool by id.
    pub fn spec(&self, id: &str) -> Option<&ToolSpec> {
        self.specs.iter().find(|spec| spec.id == id)
    }

    /// Whether and where a tool is.
    pub fn find(&self, id: &str) -> Result<ToolStatus, ToolError> {
        let spec = self
            .spec(id)
            .ok_or_else(|| ToolError::Unknown(id.to_owned()))?;
        if let Some(own) = self.overrides.tools.get(id) {
            if own.disabled {
                return Ok(ToolStatus::Disabled);
            }
            if let Some(path) = &own.path {
                let path = PathBuf::from(path);
                return Ok(if path.is_file() {
                    ToolStatus::Found(path)
                } else {
                    ToolStatus::NotFound
                });
            }
        }
        Ok(spec
            .programs
            .iter()
            .find_map(|program| which(program, self.path_var.as_deref()))
            .map_or(ToolStatus::NotFound, ToolStatus::Found))
    }

    /// The tool's state in words, its name first: "pandoc, found at
    /// /usr/bin/pandoc", or "FFmpeg, not found. Install FFmpeg from ...".
    pub fn describe(&self, id: &str) -> String {
        let Some(spec) = self.spec(id) else {
            return format!("{id}, unknown tool");
        };
        match self.find(id) {
            Ok(ToolStatus::Found(path)) => format!("{}, found at {}", spec.name, path.display()),
            Ok(ToolStatus::Disabled) => format!("{}, turned off", spec.name),
            Ok(ToolStatus::NotFound) | Err(_) => {
                format!("{}, not found. {}", spec.name, spec.install_hint)
            }
        }
    }

    /// Runs a tool with an argument list, no shell, no console window,
    /// standard input closed, output captured. Phase 0 checks for
    /// cancellation only before starting.
    pub fn run(
        &self,
        id: &str,
        args: &[OsString],
        ctx: &OpContext,
    ) -> Result<ToolOutput, ToolError> {
        let spec = self
            .spec(id)
            .ok_or_else(|| ToolError::Unknown(id.to_owned()))?;
        let path = match self.find(id)? {
            ToolStatus::Found(path) => path,
            ToolStatus::NotFound => return Err(ToolError::NotFound(spec.name.to_owned())),
            ToolStatus::Disabled => return Err(ToolError::Disabled(spec.name.to_owned())),
        };
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let mut command = Command::new(&path);
        command.args(args).stdin(Stdio::null());
        hide_console(&mut command);
        let output = command.output().map_err(|e| ToolError::Start {
            tool: spec.name.to_owned(),
            message: e.to_string(),
        })?;
        Ok(ToolOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

/// The one PATH lookup rule: each folder on `path_var` in order, never the
/// current folder; on Windows a name without an extension also tries the
/// extensions in `PATHEXT` (or `.exe`, `.com`, `.cmd` and `.bat`).
pub fn which(program: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    let has_extension = Path::new(program).extension().is_some();
    let extensions: Vec<String> = if cfg!(windows) && !has_extension {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.COM;.CMD;.BAT".to_owned())
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        Vec::new()
    };
    for folder in env::split_paths(path_var) {
        if !folder.is_absolute() {
            continue;
        }
        let candidate = folder.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        for ext in &extensions {
            let candidate = folder.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_config::ToolOverride;

    fn here() -> OsString {
        OsString::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn which_looks_only_on_the_given_path() {
        let found = which("Cargo.toml", Some(&here())).unwrap();
        assert!(found.ends_with("Cargo.toml"));
        assert!(which("no-such-program", Some(&here())).is_none());
        assert!(which("Cargo.toml", None).is_none());
        assert!(which("Cargo.toml", Some(OsStr::new("relative-folder"))).is_none());
    }

    #[test]
    fn a_missing_tool_says_how_to_get_it() {
        let tools = ToolRegistry::builtin().with_path(None);
        assert_eq!(tools.find("pandoc"), Ok(ToolStatus::NotFound));
        assert_eq!(
            tools.describe("pandoc"),
            "pandoc, not found. Install pandoc from pandoc.org or your package manager."
        );
        let err = tools.run("pandoc", &[], &OpContext::new()).unwrap_err();
        assert_eq!(err.to_string(), "pandoc is not installed");
        assert_eq!(
            tools.find("nothing"),
            Err(ToolError::Unknown("nothing".into()))
        );
    }

    #[test]
    fn overrides_turn_tools_off() {
        let mut overrides = ToolsConfig::default();
        overrides.tools.insert(
            "ffmpeg".to_owned(),
            ToolOverride {
                path: None,
                disabled: true,
            },
        );
        let tools = ToolRegistry::builtin().with_overrides(overrides);
        assert_eq!(tools.find("ffmpeg"), Ok(ToolStatus::Disabled));
        assert_eq!(tools.describe("ffmpeg"), "FFmpeg, turned off");
    }
}
