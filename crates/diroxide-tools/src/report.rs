//! The tools report: what the Settings Tools page and `diro doctor` say
//! about each helper program (alpha.7, R28 1.3; the owner's answer 124).
//! One fact a line, the tool's name first, plain words:
//! "ImageMagick: 7.1.2-35, on PATH", "pandoc: not found", then the next
//! step, and "Tools: 7 of 17 found". No secret, no environment and no
//! home folder is ever in a line.
//!
//! A tool is looked for in one order ([`LOOKUP_ORDER`]): its path in
//! `tools.toml`, then an installed component, then PATH, then the folders
//! its installers use; the line says where it was found ([`FoundBy`]).
//! The contract step of alpha 4 (P4) fixes these types;
//! [`ToolRegistry::report`](crate::ToolRegistry::report) is naive (no
//! versions read, `tools.toml` and PATH only) until the registry's lane
//! (commits 17 to 20) fills it.

/// One step of the lookup, in the order of answer 124 (textweaver's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookupStep {
    /// The tool's `path` in `tools.toml`.
    ToolsToml,
    /// An installed helper component.
    Component,
    /// The PATH, through the one rule (absolute folders only).
    Path,
    /// The folders the tool's installers use: Program Files, Git for
    /// Windows, Homebrew.
    InstallFolders,
}

/// The lookup's order: `tools.toml`, a component, PATH, the known install
/// folders (answer 124).
pub const LOOKUP_ORDER: [LookupStep; 4] = [
    LookupStep::ToolsToml,
    LookupStep::Component,
    LookupStep::Path,
    LookupStep::InstallFolders,
];

impl LookupStep {
    /// The step in words, for the Settings page's help: "tools.toml".
    pub const fn words(self) -> &'static str {
        match self {
            LookupStep::ToolsToml => "tools.toml",
            LookupStep::Component => "an installed component",
            LookupStep::Path => "PATH",
            LookupStep::InstallFolders => "the folders its installers use",
        }
    }
}

/// Where a tool was found, as its line says it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FoundBy {
    /// Its path in `tools.toml`.
    ToolsToml,
    /// An installed component.
    Component,
    /// On PATH.
    Path,
    /// In a folder its installer uses, named for the line: "Program
    /// Files", "Git for Windows".
    InstallFolder(String),
    /// Built into diro (carta, the converter in process).
    BuiltIn,
}

impl FoundBy {
    /// The words after the version: "from tools.toml", "on PATH", "in
    /// Program Files", "built in".
    pub fn words(&self) -> String {
        match self {
            FoundBy::ToolsToml => "from tools.toml".to_owned(),
            FoundBy::Component => "from components".to_owned(),
            FoundBy::Path => "on PATH".to_owned(),
            FoundBy::InstallFolder(folder) => format!("in {folder}"),
            FoundBy::BuiltIn => "built in".to_owned(),
        }
    }
}

/// What kind of tool an entry of the table is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// A program, looked for in [`LOOKUP_ORDER`] and run with an argument
    /// list.
    #[default]
    Program,
    /// Compiled into diro and run in process, always found while built
    /// in: carta, so a recipe always names a tool and Settings and `diro
    /// doctor` list it.
    BuiltIn,
}

/// A tool's state, for its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolState {
    /// Found: its version's first line when read, and where.
    Found {
        /// The version, cleaned to one short line, when it was read.
        version: Option<String>,
        /// Where it was found.
        by: FoundBy,
    },
    /// Not found anywhere the lookup looks.
    NotFound {
        /// The next step: a package command for this system, or where to
        /// get it.
        install: Option<String>,
    },
    /// Turned off in `tools.toml`.
    TurnedOff,
}

/// One tool's line in the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolReport {
    /// Its id: `magick`.
    pub id: String,
    /// Its name as said: "ImageMagick".
    pub name: String,
    /// Its state.
    pub state: ToolState,
}

impl ToolReport {
    /// Whether it can be used.
    pub fn found(&self) -> bool {
        matches!(self.state, ToolState::Found { .. })
    }

    /// Its line, the name first: "ImageMagick: 7.1.2-35, on PATH",
    /// "ImageMagick: version not read, on PATH", "pandoc: not found",
    /// "FFmpeg: turned off in tools.toml".
    pub fn line(&self) -> String {
        match &self.state {
            ToolState::Found { version, by } => format!(
                "{}: {}, {}",
                self.name,
                version.as_deref().unwrap_or("version not read"),
                by.words()
            ),
            ToolState::NotFound { .. } => format!("{}: not found", self.name),
            ToolState::TurnedOff => format!("{}: turned off in tools.toml", self.name),
        }
    }

    /// The next step for a tool not found, as its own line: "Install:
    /// winget install JohnMacFarlane.Pandoc".
    pub fn install_line(&self) -> Option<String> {
        match &self.state {
            ToolState::NotFound {
                install: Some(install),
            } => Some(format!("Install: {install}")),
            _ => None,
        }
    }
}

/// Every tool's line, and the count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolsReport {
    /// The tools, in the table's order.
    pub tools: Vec<ToolReport>,
}

impl ToolsReport {
    /// The count, meaning first: "Tools: 7 of 17 found".
    pub fn summary(&self) -> String {
        let found = self.tools.iter().filter(|t| t.found()).count();
        format!("Tools: {found} of {} found", self.tools.len())
    }

    /// The report as lines: the count first, then each tool's line, each
    /// missing tool's next step after its line.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.summary()];
        for tool in &self.tools {
            lines.push(tool.line());
            lines.extend(tool.install_line());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, state: ToolState) -> ToolReport {
        ToolReport {
            id: name.to_lowercase(),
            name: name.to_owned(),
            state,
        }
    }

    /// R30's lines, at 40 cells with the name first.
    #[test]
    fn each_line_says_the_tool_first_in_40_cells() {
        let report = ToolsReport {
            tools: vec![
                tool(
                    "ImageMagick",
                    ToolState::Found {
                        version: Some("7.1.2-35".to_owned()),
                        by: FoundBy::Path,
                    },
                ),
                tool(
                    "pandoc",
                    ToolState::NotFound {
                        install: Some("winget install JohnMacFarlane.Pandoc".to_owned()),
                    },
                ),
                tool(
                    "carta",
                    ToolState::Found {
                        version: Some("0.0.13".to_owned()),
                        by: FoundBy::BuiltIn,
                    },
                ),
                tool("FFmpeg", ToolState::TurnedOff),
            ],
        };
        let lines = report.lines();
        assert_eq!(
            lines,
            [
                "Tools: 2 of 4 found",
                "ImageMagick: 7.1.2-35, on PATH",
                "pandoc: not found",
                "Install: winget install JohnMacFarlane.Pandoc",
                "carta: 0.0.13, built in",
                "FFmpeg: turned off in tools.toml",
            ]
        );
        for line in &lines[..3] {
            assert!(line.chars().count() <= 40, "{line}");
        }
        assert!(diroxide_core::format::within_cells(&lines[3], "Install:"));
        assert_eq!(
            FoundBy::InstallFolder("Program Files".to_owned()).words(),
            "in Program Files"
        );
        assert_eq!(
            LOOKUP_ORDER.map(LookupStep::words),
            [
                "tools.toml",
                "an installed component",
                "PATH",
                "the folders its installers use"
            ]
        );
    }
}
